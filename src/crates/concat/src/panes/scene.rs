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
//!
//! Two sources, one truth: a scene either lives in the project document
//! (the top-level `scene` field - the pane follows it, edits go through
//! the command stack, and undo/redo and autosave just work) or is a
//! standalone file the pane opened (edits stay in the pane until Save).
//! A scene on the timeline plays as a clip: the frame the pane shows is
//! the playhead mapped into the clip's local time.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};

use concat_camera::VirtualCamera;
use concat_project::Project;
use concat_scene::Scene;
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
    /// The frame slider moved (fallback when the timeline has no scene clip).
    Seek(f32),
    /// The picture was dragged, in fractions of the displayed picture.
    Drag { dx: f32, dy: f32 },
    /// The pointer was lifted: draw the frame at full preview size again.
    DragEnded,
    /// The Save button (a standalone scene file).
    Save,
    /// Place the scene on the timeline at the playhead.
    AddToTimeline,
}

/// One in-flight request: its id.
struct InFlight {
    id: u64,
}

/// One open scene: where it lives, where its assets live, the live
/// document, and the previewer that reuses decoders and probed asset
/// sizes. `path` is `None` for the project's own scene, which the pane
/// follows rather than owns.
struct SceneState {
    path: Option<PathBuf>,
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
                self.drag(dx, dy, studio);
            }
            SceneMsg::DragEnded => {
                if self.dragging {
                    self.dragging = false;
                    // The project scene's drag was one gesture; the
                    // document's undo step closes here. A standalone
                    // scene edits no document, so it has no gesture.
                    if self.embedded() {
                        studio.end_scene_gesture();
                    }
                    self.request(self.frame, false);
                }
            }
            SceneMsg::Save => self.save(studio),
            SceneMsg::AddToTimeline => self.add_to_timeline(studio),
        }
    }

    /// Whether a scene is open. Drives the Save button.
    pub fn has_scene(&self) -> bool {
        self.state.is_some()
    }

    /// True when the open scene is the project's own document field -
    /// the pane follows the document, and its edits go through the
    /// command stack - rather than a standalone file.
    fn embedded(&self) -> bool {
        self.state.as_ref().map(|state| state.path.is_none()) == Some(true)
    }

    /// The open scene's id, for rules that compare it to the timeline's
    /// scene clips; None with no scene open.
    pub fn scene_id(&self) -> Option<String> {
        let state = self.state.as_ref()?;
        let scene = state.scene.read().unwrap_or_else(|p| p.into_inner());
        Some(scene.id.clone())
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
        match load_scene(&path) {
            Ok(scene) => {
                self.generation += 1;
                self.pending = None;
                self.in_flight = None;
                self.max_frame = (scene.duration_frames().ceil() as i32 - 1).max(0);
                // A standalone scene follows the timeline's whole length -
                // it is not a clip, so its frame is the playhead at the
                // scene's own rate.
                self.frame = (f64::from(playhead) * scene.fps())
                    .floor()
                    .clamp(0.0, f64::from(self.max_frame)) as f32;
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
                    path: Some(path.clone()),
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

    /// Takes the project's scene as the pane's own: the pane follows the
    /// document rather than owning a copy, so a camera edit made anywhere
    /// (here, undo, an agent) lands here. A standalone file the user
    /// opened is left alone; the frame is kept, clamped to the new
    /// length; nothing re-renders unless the scene actually changed.
    pub fn sync_from_project(&mut self, project: &Project, project_dir: &Path) {
        let parsed = project
            .extra
            .get(concat_studio::SCENE_KEY)
            .and_then(|value| serde_json::from_value::<Scene>(value.clone()).ok());
        // `Some(false)` is a standalone file - the pane's own, which the
        // document never overwrites; `Some(true)` is the embedded mode
        // this sync drives.
        let embedded = self.state.as_ref().map(|state| state.path.is_none());
        match (embedded, parsed) {
            // The user's own file: not the project's to overwrite.
            (Some(false), _) => {}
            // No scene anywhere, and none embedded before: nothing to do.
            (None, None) => {}
            // The scene is gone from the project (an undo of the embed):
            // the pane lets go of it.
            (Some(true), None) => {
                self.generation += 1;
                self.state = None;
                self.image = slint::Image::default();
                self.in_flight = None;
                self.pending = None;
                self.max_frame = 0;
                self.frame = 0.0;
                self.status = "Open a scene to preview it.".into();
            }
            (_, Some(scene)) => {
                let (changed, replaced) = match self.state.as_ref() {
                    Some(state) if state.path.is_none() => {
                        let changed =
                            *state.scene.read().unwrap_or_else(|p| p.into_inner()) != scene;
                        (changed, false)
                    }
                    _ => (true, true),
                };
                let frame = self.frame;
                if replaced {
                    // A new source: renders from the old one must not land.
                    self.generation += 1;
                    self.state = Some(Arc::new(SceneState {
                        path: None,
                        dir: project_dir.to_path_buf(),
                        scene: RwLock::new(scene.clone()),
                        previewer: Mutex::new(ScenePreviewer::new()),
                    }));
                } else {
                    *self
                        .state
                        .as_ref()
                        .expect("embedded, checked above")
                        .scene
                        .write()
                        .unwrap_or_else(|p| p.into_inner()) = scene.clone();
                }
                self.max_frame = (scene.duration_frames().ceil() as i32 - 1).max(0);
                self.frame = frame.clamp(0.0, self.max_frame as f32);
                if changed {
                    self.status = scene_status(&scene);
                    self.request(self.frame, false);
                }
            }
        }
    }

    fn drag(&mut self, dx: f32, dy: f32, studio: &mut Studio) {
        let Some(state) = self.state.clone() else {
            return;
        };
        let scene = state.scene.read().unwrap_or_else(|p| p.into_inner());
        let pose = scene.camera.pose_at(f64::from(self.frame));
        drop(scene);
        let (world_dx, world_dy) = drag_world_delta(&pose, dx, dy);
        if state.path.is_some() {
            // A standalone scene edits its own file's copy, as it did
            // before scenes lived in the project; Save writes it. The
            // document's scene - which the command stack reaches - is
            // someone else's, and a drag must not touch it.
            let mut scene = state.scene.write().unwrap_or_else(|p| p.into_inner());
            let mut moved = pose;
            moved.x -= world_dx;
            moved.y -= world_dy;
            let easing = scene
                .camera
                .keys
                .iter()
                .find(|key| (key.frame - f64::from(self.frame)).abs() < 1e-9)
                .map(|key| key.easing)
                .unwrap_or(concat_scene::Easing::LINEAR);
            scene.camera.add_key(f64::from(self.frame), moved, easing);
            drop(scene);
            self.request(self.frame, true);
            return;
        }
        // The project's scene: the camera edit goes through the command
        // stack, like every other edit - undoable, autosaved, mirrored
        // into the document. The whole drag folds into one undo step,
        // closed at DragEnded; the pane follows through the sync the
        // applied command already queued.
        studio.scene_gesture_gate();
        studio.apply_within(
            "scene-camera-drag",
            concat_project::Command::SetSceneCameraKey {
                frame: f64::from(self.frame),
                x: Some(pose.x - world_dx),
                y: Some(pose.y - world_dy),
                zoom: None,
                rotation: None,
                easing: None,
            },
        );
    }

    fn save(&mut self, studio: &Studio) {
        let Some(state) = self.state.clone() else {
            self.status = "Open a scene first.".into();
            return;
        };
        let Some(path) = state.path.clone() else {
            self.status = "The project's scene saves with the project.".into();
            return;
        };
        if same_manifest(&path, studio.project_folder.as_deref()) {
            self.status =
                "Refusing to save over the open project's manifest; use Save As on a copy.".into();
            return;
        }
        let scene = state.scene.read().unwrap_or_else(|p| p.into_inner());
        self.status = match save_scene(&path, &scene) {
            Ok(()) => format!("Saved {}", path.display()).into(),
            Err(error) => error.into(),
        };
    }

    /// Places the scene on the timeline at the playhead, as a scene clip:
    /// embedded when the project has none yet, matched when it holds this
    /// one. The scene's own length becomes the clip's.
    fn add_to_timeline(&mut self, studio: &mut Studio) {
        let Some(state) = self.state.clone() else {
            self.status = "Open a scene first.".into();
            return;
        };
        if studio.session.is_none() {
            self.status = "Open a project first.".into();
            return;
        }
        let scene = state.scene.read().unwrap_or_else(|p| p.into_inner()).clone();
        let start = f64::from(studio.playhead);
        studio.apply(concat_project::Command::AddSceneClip {
            track_id: None,
            above: false,
            start,
            scene,
            name: None,
        });
    }

    /// Follow the editor's timeline playhead. `scene_clips` are the
    /// timeline's scene clips as `(scene id, start, duration)`; the open
    /// scene's frame is its clip's local time, or, outside every clip,
    /// the nearest clip's boundary frame. A standalone scene (no clip of
    /// its own id) falls back to the playhead at its own rate, as before.
    pub fn follow_playhead(&mut self, seconds: f32, coarse: bool, scene_clips: &[(String, f64, f64)]) {
        let Some(state) = self.state.clone() else {
            return;
        };
        let scene = state.scene.read().unwrap_or_else(|p| p.into_inner());
        let fps = scene.fps();
        // The last frame is an index, one below the frame count the
        // duration names - the same convention the slider's maximum has.
        let max = (scene.duration_frames() - 1.0).max(0.0);
        let owned: Vec<(f64, f64)> = scene_clips
            .iter()
            .filter(|(id, _, _)| state.path.is_none() && *id == scene.id)
            .map(|(_, start, duration)| (*start, *duration))
            .collect();
        drop(scene);
        let frame = if owned.is_empty() {
            // Standalone: the whole timeline is the scene's clock, as
            // before the clip existed.
            (f64::from(seconds) * fps).floor().clamp(0.0, max)
        } else {
            scene_frame_at(&owned, f64::from(seconds), fps, max)
        } as f32;
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

/// The pane's opening status line: the scene's name over its output frame
/// and rate.
fn scene_status(scene: &Scene) -> SharedString {
    let (viewport_width, viewport_height) = scene
        .camera
        .keys
        .first()
        .map(|key| (key.pose.viewport_width, key.pose.viewport_height))
        .unwrap_or((1920, 1080));
    format!(
        "{}  ·  {}×{}  ·  {:.2} fps",
        scene.name,
        viewport_width,
        viewport_height,
        scene.fps()
    )
    .into()
}

/// The scene frame the timeline shows at `seconds`, over the scene clips
/// `(start, duration)` that play this scene: inside a clip, its local
/// time; outside every one, the nearest clip's boundary frame. Empty
/// `clips` falls back to the playhead as the scene's own clock.
fn scene_frame_at(clips: &[(f64, f64)], seconds: f64, fps: f64, max_frame: f64) -> f64 {
    if clips.is_empty() {
        return (seconds * fps).floor().clamp(0.0, max_frame);
    }
    let inside = clips.iter().find(|(start, duration)| {
        seconds >= *start && seconds < *start + *duration
    });
    if let Some((start, _)) = inside {
        return ((seconds - start) * fps).floor().clamp(0.0, max_frame);
    }
    // Nearest boundary, over all clips: the distance to a clip's span is
    // zero inside it and the gap to an edge outside.
    let nearest = clips
        .iter()
        .min_by(|(a_start, a_duration), (b_start, b_duration)| {
            let da = (seconds - a_start)
                .abs()
                .min((seconds - (a_start + a_duration)).abs());
            let db = (seconds - b_start)
                .abs()
                .min((seconds - (b_start + b_duration)).abs());
            da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
        });
    match nearest {
        Some((start, duration)) => {
            let head = (seconds - start).abs();
            let tail = (seconds - (start + duration)).abs();
            if head <= tail {
                0.0
            } else {
                (duration * fps).floor().clamp(0.0, max_frame)
            }
        }
        None => 0.0,
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

    /// The playhead inside a scene clip maps to the clip's local time;
    /// outside every clip it holds the nearest clip's boundary frame, so
    /// scrubbing the timeline never leaves the scene blank.
    #[test]
    fn the_playhead_maps_to_the_scene_clips_local_time() {
        let fps = 30.0;
        // The caller passes the last frame's index - one below the count
        // the scene's duration names - as `follow_playhead` does.
        let max = 89.0;
        // One clip from 2s to 5s.
        let clips = vec![(2.0, 3.0)];
        assert_eq!(scene_frame_at(&clips, 2.0, fps, max), 0.0);
        assert_eq!(scene_frame_at(&clips, 3.5, fps, max), 45.0);
        assert_eq!(scene_frame_at(&clips, 4.99, fps, max), 89.0);
        // Before it: the head frame. After it: the last frame index, one
        // below the count the duration names.
        assert_eq!(scene_frame_at(&clips, 0.0, fps, max), 0.0);
        assert_eq!(scene_frame_at(&clips, 10.0, fps, max), 89.0);
        // Two clips: nearest boundary wins. The first clip's tail frame is
        // its own length at the scene's rate (1s × 30fps), not the
        // scene's last frame - only a clip reaching the scene's end does.
        let clips = vec![(2.0, 1.0), (6.0, 1.0)];
        assert_eq!(scene_frame_at(&clips, 4.4, fps, max), 30.0, "closer to the first clip's tail");
        assert_eq!(scene_frame_at(&clips, 4.6, fps, max), 0.0, "closer to the second clip's head");
        // A trimmed clip maps only its own window.
        let clips = vec![(0.0, 1.5)];
        assert_eq!(scene_frame_at(&clips, 1.4, fps, max), 42.0);
        // No clip of this scene: the timeline is the scene's clock, as a
        // standalone scene always was - clamped to the last frame index.
        assert_eq!(scene_frame_at(&[], 5.0, fps, max), 89.0);
    }
}
