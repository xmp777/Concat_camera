// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 xmp777

//! The 2D scene pane: a `concat-scene` frame, its camera, and the frame's
//! number. Rendering goes through `concat_studio::ScenePreviewer`, which is
//! Concat's preview compositor - the pane adds no renderer.
//!
//! One request is out at a time and only the newest waiting frame is kept.
//! Results carry the request id and scene generation, so a frame from a
//! previous scene or an older seek cannot rewind the slider or move the
//! camera at the wrong time.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};

use concat_camera::VirtualCamera;
use concat_scene::{Easing, Scene};
use concat_studio::{ScenePreviewer, load_scene, save_scene};
use slint::SharedString;

use crate::host;
use crate::panes::Msg;
use crate::studio::Studio;

/// Preview size while editing: half of 1080p, quick on the CPU and enough to
/// judge a camera move. Export still uses the scene's own output frame.
const PREVIEW_WIDTH: u32 = 960;
const PREVIEW_HEIGHT: u32 = 540;
/// While a drag or playback is in flight the frame is drawn at a quarter of
/// the preview; the full frame is drawn once the drag ends.
const COARSE_WIDTH: u32 = 480;
const COARSE_HEIGHT: u32 = 270;

/// Everything that can happen to the scene pane.
#[derive(Clone, Debug)]
pub enum SceneMsg {
    /// The Open button: pick a scene file.
    Open,
    /// A scene file was picked.
    Opened(PathBuf),
    /// The frame slider moved (fallback when the timeline is empty).
    Seek(f32),
    /// The picture was dragged, in fractions of the displayed picture.
    Drag { dx: f32, dy: f32 },
    /// The pointer was lifted: draw the frame at full preview size again.
    DragEnded,
    /// The Save button.
    Save,
}

/// One in-flight request: its id.
struct InFlight {
    id: u64,
}

/// One open scene: its file, where its assets live, the live document, and
/// the previewer that reuses decoders and probed asset sizes.
struct SceneState {
    path: PathBuf,
    dir: PathBuf,
    scene: RwLock<Scene>,
    previewer: Mutex<ScenePreviewer>,
}

/// The scene pane's state.
pub struct ScenePane {
    /// The last frame drawn, already in Slint's hands.
    pub image: slint::Image,
    /// The frame the user asked for; results never move it backwards.
    pub frame: f32,
    pub max_frame: i32,
    pub status: SharedString,
    state: Option<Arc<SceneState>>,
    in_flight: Option<InFlight>,
    pending: Option<(f32, bool)>,
    request_id: u64,
    generation: u64,
    /// A drag is in flight: draw smaller until it ends.
    dragging: bool,
}

impl Default for ScenePane {
    fn default() -> Self {
        Self {
            image: slint::Image::default(),
            frame: 0.0,
            max_frame: 0,
            status: "Open a scene to preview it.".into(),
            state: None,
            in_flight: None,
            pending: None,
            request_id: 0,
            generation: 0,
            dragging: false,
        }
    }
}

impl ScenePane {
    /// Applies one message. The studio is the rest of the window; while this
    /// runs the studio's copy of the pane is a blank it must not read.
    pub fn update(&mut self, msg: SceneMsg, studio: &mut Studio) {
        match msg {
            SceneMsg::Open => {
                host::pick_files_then(
                    "Open a 2D scene".to_owned(),
                    Some(("2D Scene".to_owned(), &["json"])),
                    |studio, _, _, paths| {
                        if let Some(path) = paths.into_iter().next() {
                            studio.handle(Msg::Scene(SceneMsg::Opened(path)));
                        }
                    },
                );
            }
            SceneMsg::Opened(path) => self.open(path, studio),
            SceneMsg::Seek(frame) => {
                self.dragging = false;
                self.frame = frame.clamp(0.0, self.max_frame as f32);
                self.request(self.frame, false);
            }
            SceneMsg::Drag { dx, dy } => {
                self.dragging = true;
                self.drag(dx, dy);
            }
            SceneMsg::DragEnded => {
                if self.dragging {
                    self.dragging = false;
                    self.request(self.frame, false);
                }
            }
            SceneMsg::Save => self.save(studio),
        }
    }

    /// Whether a scene is open. Drives the Save button.
    pub fn has_scene(&self) -> bool {
        self.state.is_some()
    }

    /// The open scene's duration in seconds, for the slider fallback.
    pub fn duration_seconds(&self) -> Option<f64> {
        let state = self.state.as_ref()?;
        let scene = state.scene.read().unwrap_or_else(|p| p.into_inner());
        Some(scene.duration_frames() / scene.fps())
    }

    fn open(&mut self, path: PathBuf, studio: &Studio) {
        self.dragging = false;
        if same_manifest(&path, studio.project_folder.as_deref()) {
            self.status =
                "This is the open project's manifest; open a standalone scene copy instead.".into();
            return;
        }
        let playhead = studio.playhead;
        let follow_timeline = studio.duration() > 0.0;
        match load_scene(&path) {
            Ok(scene) => {
                self.generation += 1;
                self.pending = None;
                self.in_flight = None;
                self.max_frame = (scene.duration_frames().ceil() as i32 - 1).max(0);
                self.frame = if follow_timeline {
                    (f64::from(playhead) * scene.fps())
                        .floor()
                        .clamp(0.0, f64::from(self.max_frame)) as f32
                } else {
                    0.0
                };
                let (viewport_width, viewport_height) = scene
                    .camera
                    .keys
                    .first()
                    .map(|key| (key.pose.viewport_width, key.pose.viewport_height))
                    .unwrap_or((1920, 1080));
                self.status = format!(
                    "{}  ·  {}×{}  ·  {:.2} fps",
                    scene.name,
                    viewport_width,
                    viewport_height,
                    scene.fps()
                )
                .into();
                let dir = path
                    .parent()
                    .map(Path::to_path_buf)
                    .unwrap_or_else(|| PathBuf::from("."));
                self.state = Some(Arc::new(SceneState {
                    path,
                    dir,
                    scene: RwLock::new(scene),
                    previewer: Mutex::new(ScenePreviewer::new()),
                }));
                self.request(self.frame, false);
            }
            Err(error) => {
                // A failed open must not leave the previous scene ready to
                // Save under the new file's name.
                self.generation += 1;
                self.state = None;
                self.image = slint::Image::default();
                self.in_flight = None;
                self.pending = None;
                self.max_frame = 0;
                self.frame = 0.0;
                self.status = error.into();
            }
        }
    }

    fn drag(&mut self, dx: f32, dy: f32) {
        let Some(state) = self.state.clone() else {
            return;
        };
        let mut scene = state.scene.write().unwrap_or_else(|p| p.into_inner());
        let mut pose = scene.camera.pose_at(f64::from(self.frame));
        let (world_dx, world_dy) = drag_world_delta(&pose, dx, dy);
        pose.x -= world_dx;
        pose.y -= world_dy;
        let easing = scene
            .camera
            .keys
            .iter()
            .find(|key| (key.frame - f64::from(self.frame)).abs() < 1e-9)
            .map(|key| key.easing)
            .unwrap_or(Easing::LINEAR);
        scene.camera.add_key(f64::from(self.frame), pose, easing);
        drop(scene);
        self.request(self.frame, true);
    }

    fn save(&mut self, studio: &Studio) {
        let Some(state) = self.state.clone() else {
            self.status = "Open a scene first.".into();
            return;
        };
        if same_manifest(&state.path, studio.project_folder.as_deref()) {
            self.status =
                "Refusing to save over the open project's manifest; use Save As on a copy.".into();
            return;
        }
        let scene = state.scene.read().unwrap_or_else(|p| p.into_inner());
        self.status = match save_scene(&state.path, &scene) {
            Ok(()) => format!("Saved {}", state.path.display()).into(),
            Err(error) => error.into(),
        };
    }

    /// Follow the editor's timeline playhead: the scene's frame is the
    /// playhead time at the scene's own rate, clamped to its duration.
    pub fn follow_playhead(&mut self, seconds: f32, coarse: bool) {
        let Some(state) = self.state.clone() else {
            return;
        };
        let fps = state.scene.read().unwrap_or_else(|p| p.into_inner()).fps();
        let frame = (f64::from(seconds) * fps)
            .floor()
            .clamp(0.0, f64::from(self.max_frame)) as f32;
        if (frame - self.frame).abs() > 0.5 {
            self.frame = frame;
            self.request(frame, coarse);
        }
    }

    /// Ask for a frame, keeping at most one request waiting.
    fn request(&mut self, frame: f32, coarse: bool) {
        let Some(state) = self.state.clone() else {
            return;
        };
        if self.in_flight.is_some() {
            self.pending = Some((frame, coarse));
            return;
        }
        self.request_id += 1;
        let id = self.request_id;
        let generation = self.generation;
        let scene = state.scene.read().unwrap_or_else(|p| p.into_inner());
        let (viewport_width, viewport_height) = scene
            .camera
            .keys
            .first()
            .map(|key| (key.pose.viewport_width, key.pose.viewport_height))
            .unwrap_or((1920, 1080));
        drop(scene);
        let (max_width, max_height) = if coarse || self.dragging {
            (COARSE_WIDTH, COARSE_HEIGHT)
        } else {
            (PREVIEW_WIDTH, PREVIEW_HEIGHT)
        };
        let (width, height) = fit_size(viewport_width, viewport_height, max_width, max_height);
        self.in_flight = Some(InFlight { id });

        let work_state = Arc::clone(&state);
        host::spawn_publishing(
            move || {
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let scene = work_state
                        .scene
                        .read()
                        .unwrap_or_else(|p| p.into_inner())
                        .clone();
                    let mut previewer = work_state
                        .previewer
                        .lock()
                        .unwrap_or_else(|p| p.into_inner());
                    previewer.preview_frame(
                        &scene,
                        &work_state.dir,
                        f64::from(frame),
                        width,
                        height,
                    )
                }))
                .unwrap_or_else(|_| Err("scene preview panicked".to_owned()))
            },
            move |studio, _, _, result| {
                let pane = &mut studio.scene;
                if pane.in_flight.as_ref().map(|request| request.id) == Some(id) {
                    pane.in_flight = None;
                }
                if pane.generation == generation && (pane.frame - frame).abs() < 0.5 {
                    match result {
                        Ok(picture) => {
                            let buffer =
                                slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(
                                    &picture.pixels,
                                    picture.width,
                                    picture.height,
                                );
                            pane.image = slint::Image::from_rgba8(buffer);
                            pane.status = format!("frame {:.0} / {}", frame, pane.max_frame).into();
                        }
                        Err(error) => pane.status = error.into(),
                    }
                }
                if let Some((next, coarse)) = pane.pending.take() {
                    pane.request(next, coarse);
                }
            },
            |studio, app, models| studio.publish_scene(app, models),
        );
    }
}

/// Fit a scene's output frame into a preview box without changing its aspect;
/// never upscale, so a 640×360 scene stays 640×360.
fn fit_size(
    viewport_width: u32,
    viewport_height: u32,
    max_width: u32,
    max_height: u32,
) -> (u32, u32) {
    let vw = viewport_width.max(1) as f64;
    let vh = viewport_height.max(1) as f64;
    let scale = (f64::from(max_width) / vw)
        .min(f64::from(max_height) / vh)
        .min(1.0);
    (
        (vw * scale).round().max(1.0) as u32,
        (vh * scale).round().max(1.0) as u32,
    )
}

/// The world-space delta a drag of `dx`/`dy` fractions of the displayed
/// picture asks for. `viewport_to_world` carries the camera's zoom and
/// rotation, so a full-width drag always moves one viewport's worth.
fn drag_world_delta(pose: &VirtualCamera, dx: f32, dy: f32) -> (f64, f64) {
    let (x0, y0) = pose.viewport_to_world(0.0, 0.0);
    let (x1, y1) = pose.viewport_to_world(
        f64::from(dx) * f64::from(pose.viewport_width),
        f64::from(dy) * f64::from(pose.viewport_height),
    );
    (x1 - x0, y1 - y0)
}

/// Whether `scene_path` is the manifest of the project the editor has open.
fn same_manifest(scene_path: &Path, project_folder: Option<&Path>) -> bool {
    let Some(folder) = project_folder else {
        return false;
    };
    let manifest = concat_host::projects::manifest_path(folder);
    match (
        std::fs::canonicalize(scene_path),
        std::fs::canonicalize(manifest),
    ) {
        (Ok(scene), Ok(manifest)) => scene == manifest,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_full_width_drag_moves_one_viewport() {
        let mut pose = VirtualCamera::new(1000, 500);
        pose.zoom = 2.0;
        let (dx, dy) = drag_world_delta(&pose, 1.0, 0.0);
        assert!((dx - 500.0).abs() < 1e-9);
        assert!(dy.abs() < 1e-9);
    }

    #[test]
    fn rotation_maps_a_horizontal_drag_to_the_world_y_axis() {
        let mut pose = VirtualCamera::new(1000, 500);
        pose.rotation = std::f64::consts::FRAC_PI_2;
        let (dx, dy) = drag_world_delta(&pose, 1.0, 0.0);
        assert!(dx.abs() < 1e-9);
        assert!((dy - 1000.0).abs() < 1e-9);
    }

    #[test]
    fn a_portrait_scene_is_previewed_portrait() {
        let (width, height) = fit_size(1080, 1920, 960, 540);
        assert!(width < height, "{width}×{height}");
        assert!(width <= 960 && height <= 540);
        assert_eq!(fit_size(640, 360, 960, 540), (640, 360));
        assert_eq!(fit_size(1920, 1080, 960, 540), (960, 540));
    }

    #[test]
    fn the_open_projects_manifest_is_refused() {
        let folder = std::env::temp_dir().join(format!("scene-pane-{}", std::process::id()));
        std::fs::create_dir_all(&folder).expect("scratch folder");
        let manifest = concat_host::projects::manifest_path(&folder);
        std::fs::write(&manifest, "{}").expect("manifest");
        assert!(same_manifest(&manifest, Some(&folder)));
        assert!(!same_manifest(
            &folder.join("standalone.json"),
            Some(&folder)
        ));
        assert!(!same_manifest(&manifest, None));
        let _ = std::fs::remove_dir_all(&folder);
    }
}
