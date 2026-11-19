// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 xmp777

#![allow(missing_docs)] // Slint's generated module has no doc comments.

//! Phase 4 spike: show a `concat-scene` frame in Slint and drag the camera.
//!
//! Rendering goes through [`concat_studio::preview_frame`], which is Concat's
//! own preview compositor - this window adds no renderer of its own. A worker
//! thread renders the newest requested frame; the UI thread only uploads the
//! finished RGBA pixels.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock, mpsc};
use std::thread;

use concat_scene::{Easing, Scene};
use concat_studio::{load_scene, preview_frame_sized, save_scene};

slint::include_modules!();

/// The size previews are composited at: half of 1080p is quick on the CPU and
/// large enough to judge a camera move.
const PREVIEW_WIDTH: u32 = 960;
const PREVIEW_HEIGHT: u32 = 540;

struct Request {
    frame: f64,
    generation: u64,
}

fn main() -> Result<(), slint::PlatformError> {
    let path = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("examples/camera_demo/scene.json"));
    let scene_dir = path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));

    let (scene, opened) = match load_scene(&path) {
        Ok(scene) => {
            let opened = format!(
                "{}  ·  {}×{}  ·  {} fps",
                scene.name,
                scene.world_width.round(),
                scene.world_height.round(),
                scene.fps().round()
            );
            (scene, opened)
        }
        Err(error) => {
            eprintln!("{error}");
            (
                Scene::new("scene", "Scene", 8000.0, 4500.0, 1920, 1080),
                error,
            )
        }
    };
    let last_frame = (scene.duration_frames().ceil() as i32 - 1).max(0);
    let scene = Arc::new(RwLock::new(scene));
    let generation = Arc::new(AtomicU64::new(0));

    let ui = MainWindow::new()?;
    ui.set_max_frame(last_frame);
    ui.set_status(opened.into());

    let (send, receive) = mpsc::channel::<Request>();
    {
        let scene = Arc::clone(&scene);
        let weak = ui.as_weak();
        let generation = Arc::clone(&generation);
        thread::spawn(move || {
            while let Ok(mut request) = receive.recv() {
                // Newest wins: drop frames queued while the last one drew.
                while let Ok(next) = receive.try_recv() {
                    request = next;
                }
                let scene = scene.read().expect("scene lock").clone();
                let picture = match preview_frame_sized(
                    &scene,
                    &scene_dir,
                    request.frame,
                    PREVIEW_WIDTH,
                    PREVIEW_HEIGHT,
                ) {
                    Ok(picture) => picture,
                    Err(error) => {
                        let error = error.clone();
                        let weak = weak.clone();
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(ui) = weak.upgrade() {
                                ui.set_status(error.into());
                            }
                        });
                        continue;
                    }
                };
                // A newer request already exists: its frame is the one to show.
                if generation.load(Ordering::SeqCst) != request.generation {
                    continue;
                }
                let weak = weak.clone();
                let frame = request.frame;
                let _ = slint::invoke_from_event_loop(move || {
                    let Some(ui) = weak.upgrade() else { return };
                    let buffer = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(
                        &picture.pixels,
                        picture.width,
                        picture.height,
                    );
                    ui.set_frame(slint::Image::from_rgba8(buffer));
                    ui.set_frame_number(frame as f32);
                    ui.set_status(
                        format!(
                            "frame {:.0} / {}  ·  drag to move the camera  ·  Save writes the scene",
                            frame, ui.get_max_frame()
                        )
                        .into(),
                    );
                });
            }
        });
    }

    let request = {
        let send = send.clone();
        let generation = Arc::clone(&generation);
        Arc::new(move |frame: f64| {
            let generation = generation.fetch_add(1, Ordering::SeqCst) + 1;
            let _ = send.send(Request { frame, generation });
        })
    };

    {
        let request = request.clone();
        ui.on_seek(move |frame| request(frame as f64));
    }
    {
        let scene = Arc::clone(&scene);
        let request = request.clone();
        let weak = ui.as_weak();
        ui.on_drag_camera(move |dx, dy| {
            let Some(ui) = weak.upgrade() else { return };
            let frame = ui.get_frame_number() as f64;
            let mut scene = scene.write().expect("scene lock");
            let mut pose = scene.camera.pose_at(frame);
            let zoom = pose.zoom.max(0.01);
            pose.x -= (dx as f64) * f64::from(PREVIEW_WIDTH) / zoom;
            pose.y -= (dy as f64) * f64::from(PREVIEW_HEIGHT) / zoom;
            scene.camera.add_key(frame, pose, Easing::LINEAR);
            drop(scene);
            request(frame);
        });
    }
    {
        let path = path.clone();
        let scene = Arc::clone(&scene);
        let weak = ui.as_weak();
        ui.on_save(move || {
            let result = save_scene(&path, &scene.read().expect("scene lock"));
            if let Some(ui) = weak.upgrade() {
                ui.set_status(
                    match result {
                        Ok(()) => format!("saved {}", path.display()),
                        Err(error) => error,
                    }
                    .into(),
                );
            }
        });
    }

    request(0.0);
    ui.run()
}
