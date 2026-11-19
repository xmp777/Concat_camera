// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 xmp777

//! The 2D scene pane: a `concat-scene` frame, its camera, and the frame's
//! number. Rendering goes through `concat_studio::preview_frame_sized`,
//! which is Concat's preview compositor - the pane adds no renderer.
//!
//! A request at a time is out (`busy`), and only the newest waiting frame is
//! kept (`pending`): dragging the camera or scrubbing the slider stays
//! responsive without a queue of stale frames.

use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use concat_scene::Scene;
use concat_studio::{load_scene, preview_frame_sized, save_scene};
use slint::SharedString;

use crate::host;
use crate::panes::Msg;
use crate::studio::Studio;

/// Preview size while editing: half of 1080p, quick on the CPU and enough to
/// judge a camera move. Export still uses the scene's own output frame.
const PREVIEW_WIDTH: u32 = 960;
const PREVIEW_HEIGHT: u32 = 540;
/// While a drag is in flight the frame is drawn at a quarter of the preview:
/// the pointer's moves stay ahead of the CPU compositor, and the full frame
/// is drawn once when the drag ends.
const DRAG_WIDTH: u32 = 480;
const DRAG_HEIGHT: u32 = 270;

/// Everything that can happen to the scene pane.
#[derive(Clone, Debug)]
pub enum SceneMsg {
    /// The Open button: pick a scene file.
    Open,
    /// A scene file was picked.
    Opened(PathBuf),
    /// The frame slider moved.
    Seek(f32),
    /// The picture was dragged, in fractions of the preview's width/height.
    Drag { dx: f32, dy: f32 },
    /// The pointer was lifted: draw the frame at full preview size again.
    DragEnded,
    /// The Save button.
    Save,
}

/// One open scene: its file, where its assets live, and the live document.
struct SceneState {
    path: PathBuf,
    dir: PathBuf,
    scene: RwLock<Scene>,
}

/// The scene pane's state.
pub struct ScenePane {
    /// The last frame drawn, already in Slint's hands.
    pub image: slint::Image,
    /// The frame the picture and slider show.
    pub frame: f32,
    pub max_frame: i32,
    pub status: SharedString,
    state: Option<Arc<SceneState>>,
    busy: bool,
    pending: Option<f32>,
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
            busy: false,
            pending: None,
            dragging: false,
        }
    }
}

impl ScenePane {
    /// Applies one message. The studio is the rest of the window; while this
    /// runs the studio's copy of the pane is a blank it must not read.
    pub fn update(&mut self, msg: SceneMsg, _studio: &mut Studio) {
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
            SceneMsg::Opened(path) => self.open(path),
            SceneMsg::Seek(frame) => {
                self.dragging = false;
                self.frame = frame.max(0.0);
                self.request(self.frame);
            }
            SceneMsg::Drag { dx, dy } => {
                self.dragging = true;
                self.drag(dx, dy);
            }
            SceneMsg::DragEnded => {
                if self.dragging {
                    self.dragging = false;
                    self.request(self.frame);
                }
            }
            SceneMsg::Save => self.save(),
        }
    }

    fn open(&mut self, path: PathBuf) {
        self.dragging = false;
        match load_scene(&path) {
            Ok(scene) => {
                self.max_frame = (scene.duration_frames().ceil() as i32 - 1).max(0);
                self.frame = 0.0;
                self.status = format!(
                    "{}  ·  {}×{}  ·  {} fps",
                    scene.name,
                    scene.world_width.round(),
                    scene.world_height.round(),
                    scene.fps().round()
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
                }));
                self.request(0.0);
            }
            Err(error) => self.status = error.into(),
        }
    }

    fn drag(&mut self, dx: f32, dy: f32) {
        let Some(state) = self.state.clone() else {
            return;
        };
        let mut scene = state.scene.write().expect("scene lock");
        let mut pose = scene.camera.pose_at(f64::from(self.frame));
        let zoom = pose.zoom.max(0.01);
        pose.x -= f64::from(dx) * f64::from(PREVIEW_WIDTH) / zoom;
        pose.y -= f64::from(dy) * f64::from(PREVIEW_HEIGHT) / zoom;
        scene
            .camera
            .add_key(f64::from(self.frame), pose, concat_scene::Easing::LINEAR);
        drop(scene);
        self.request(self.frame);
    }

    fn save(&mut self) {
        let Some(state) = self.state.clone() else {
            self.status = "Open a scene first.".into();
            return;
        };
        let scene = state.scene.read().expect("scene lock");
        self.status = match save_scene(&state.path, &scene) {
            Ok(()) => format!("Saved {}", state.path.display()).into(),
            Err(error) => error.into(),
        };
    }

    /// Ask for a frame, keeping at most one request waiting.
    fn request(&mut self, frame: f32) {
        if self.busy {
            self.pending = Some(frame);
            return;
        }
        let Some(state) = self.state.clone() else {
            return;
        };
        self.busy = true;
        let (width, height) = if self.dragging {
            (DRAG_WIDTH, DRAG_HEIGHT)
        } else {
            (PREVIEW_WIDTH, PREVIEW_HEIGHT)
        };
        let scene = state.scene.read().expect("scene lock").clone();
        let dir = state.dir.clone();
        host::spawn(
            move || preview_frame_sized(&scene, &dir, f64::from(frame), width, height),
            move |studio, _, _, result| {
                let pane = &mut studio.scene;
                pane.busy = false;
                match result {
                    Ok(picture) => {
                        let buffer =
                            slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(
                                &picture.pixels,
                                picture.width,
                                picture.height,
                            );
                        pane.image = slint::Image::from_rgba8(buffer);
                        pane.frame = frame;
                        pane.status = format!("frame {:.0} / {}", frame, pane.max_frame).into();
                    }
                    Err(error) => pane.status = error.into(),
                }
                if let Some(next) = pane.pending.take() {
                    pane.request(next);
                }
            },
        );
    }
}
