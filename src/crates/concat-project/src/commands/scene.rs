// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! The scene commands: placing a scene on the timeline and keying its
//! camera.
//!
//! A scene lives in the document's top-level `scene` field - a raw JSON
//! value this crate round-trips but does not otherwise own (see
//! `concat-studio`'s `SCENE_KEY`, the same key by the same name). These
//! arms are the one place the value is parsed, edited and written back:
//! a parse failure here is a [`CommandError`], not a document that fails
//! to load, and the value an arm writes is the value it parsed, so
//! nothing the reader tolerated is dropped on the way through.

use std::sync::Arc;

use serde_json::{Map, Value};

use concat_scene::Scene;

use super::{Command, CommandError, IdMint, Outcome};
use crate::model::{Clip, ClipKind, Project};

/// The document key a scene lives under. `concat-studio` holds the same
/// name as `SCENE_KEY`; two constants rather than a shared one, because
/// the dependency runs the other way.
const SCENE_KEY: &str = "scene";
/// The multi-scene map: keyed by the scene's own id; read first.
const SCENES_KEY: &str = "scenes";

/// How close two camera-key frames must be to name the same key - the
/// tolerance [`concat_scene::CameraTrack::add_key`] upserts within.
const KEY_EPSILON: f64 = 1e-9;

pub(super) fn apply(
    project: &mut Project,
    mint: &mut IdMint,
    command: Command,
) -> Result<Outcome, CommandError> {
    match command {
        Command::AddSceneClip {
            track_id,
            above,
            start,
            scene,
            name,
        } => {
            let duration = (scene.duration_frames() / scene.fps()).max(super::MIN_CLIP_DURATION);
            // The track resolves first: a command that errs must leave the
            // project untouched, and embedding the scene before a
            // `TrackGone` refusal would strand it in the document with no
            // undo step to take it back. The embed then runs before the
            // timeline is borrowed, so the two mutable borrows of
            // `project` never overlap.
            {
                let timeline = project.active();
                match &track_id {
                    Some(id) if timeline.track(id).is_none() => {
                        return Err(CommandError::TrackGone);
                    }
                    _ => {}
                }
            }
            // The carried scene lands in the `scenes` map under its own
            // id — same id replaces (a standalone file re-inserted after
            // more drawing), a new id adds a second scene the map keeps.
            // The legacy single-`scene` mirror stays pointed at the first
            // entry so older readers keep opening the document.
            {
                let value = serde_json::to_value(&scene).map_err(|_| CommandError::SceneGone)?;
                let existing = read_scene_value(project, Some(&scene.id));
                if let Ok(previous) = existing
                    && previous == value
                {
                    // Nothing changes, so no undo step: `applied: true`
                    // here would push a snapshot whose restore is a
                    // no-op and clear the redo stack for nothing.
                    return Ok(Outcome {
                        created_id: None,
                        applied: false,
                    });
                }
                let map = project
                    .extra
                    .entry::<String>(SCENES_KEY.into())
                    .or_insert_with(|| Value::Object(Map::new()));
                if let Value::Object(entries) = map {
                    entries.insert(scene.id.clone(), value);
                }
                if project.extra.get(SCENE_KEY).is_none() {
                    if let Some(first) = read_first_scene_value(project) {
                        project.extra.insert(SCENE_KEY.into(), first);
                    }
                }
            }
            let timeline = project.active_mut();
            let track_id = match track_id {
                // Validated above against the same timeline.
                Some(id) => id,
                // Every lane above the video is taken: a new one at the
                // top for the scene to land on.
                None if above => super::clips::first_free_track_above(timeline, start, duration)
                    .unwrap_or_else(|| super::clips::fresh_track(timeline, mint)),
                None => super::clips::first_free_track(timeline, start, duration)
                    .unwrap_or_else(|| super::clips::fresh_track(timeline, mint)),
            };
            let start = timeline.nearest_free_start(&track_id, start, duration, &[]);
            let id = mint.next("c");
            let mut clip = Clip::blank(
                id.clone(),
                track_id,
                ClipKind::Scene,
                name.unwrap_or_else(|| scene.name.clone()),
                start,
                duration,
            );
            clip.scene_id = Some(scene.id.clone());
            timeline.clips.push(Arc::new(clip));
            Ok(Outcome {
                created_id: Some(id),
                applied: true,
            })
        }
        Command::SetSceneCameraKey {
            frame,
            x,
            y,
            zoom,
            rotation,
            easing,
        } => {
            let mut scene = read_scene(project, None)?;
            // The pose the overrides land on: the key already at the
            // frame when there is one - so keying zoom at a keyed frame
            // keeps the x the key holds, not one interpolated to it -
            // else the pose the track evaluates there.
            let base = match scene
                .camera
                .keys
                .iter()
                .find(|key| (key.frame - frame).abs() < KEY_EPSILON)
            {
                Some(key) => key.pose,
                None => scene.camera.pose_at(frame),
            };
            let pose = concat_camera::VirtualCamera {
                x: x.unwrap_or(base.x),
                y: y.unwrap_or(base.y),
                zoom: zoom.unwrap_or(base.zoom),
                rotation: rotation.unwrap_or(base.rotation),
                ..base
            };
            let easing = easing
                .or_else(|| {
                    scene
                        .camera
                        .keys
                        .iter()
                        .find(|key| (key.frame - frame).abs() < KEY_EPSILON)
                        .map(|key| key.easing)
                })
                .unwrap_or(concat_scene::Easing::LINEAR);
            scene.camera.add_key(frame, pose, easing);
            write_scene(project, &scene)
        }
        Command::RemoveSceneCameraKey { frame } => {
            let mut scene = read_scene(project, None)?;
            // The last key carries the viewport every renderer reads the
            // output frame from; taking it off would silently drop the
            // scene back to a 1920×1080 default. Remove all-but-one is the
            // caller clearing edits, and one key stays.
            if scene.camera.keys.len() <= 1 {
                return Ok(Outcome {
                    created_id: None,
                    applied: false,
                });
            }
            let before = scene.camera.keys.len();
            scene
                .camera
                .keys
                .retain(|key| (key.frame - frame).abs() >= KEY_EPSILON);
            if scene.camera.keys.len() == before {
                return Ok(Outcome {
                    created_id: None,
                    applied: false,
                });
            }
            write_scene(project, &scene)
        }
        _ => unreachable!("scene::apply routed a command that is not a scene command"),
    }
}

/// The project's scene, parsed from the document. The raw value stays
/// whatever it was: an arm that errors out has touched nothing. Reads
/// from the `scenes` map first, falling back to the legacy single-`scene`
/// key so documents written before Phase 8 keep playing.
fn read_scene(project: &Project, scene_id: Option<&str>) -> Result<Scene, CommandError> {
    read_scene_value(project, scene_id)
        .and_then(|value| serde_json::from_value(value).map_err(|_| CommandError::SceneGone))
}

/// The raw JSON of the scene the command names: the `scenes` map keyed by
/// id when the document carries one, else the legacy `scene` field.
fn read_scene_value(project: &Project, scene_id: Option<&str>) -> Result<Value, CommandError> {
    let scenes = project.extra.get(SCENES_KEY);
    let legacy = project.extra.get(SCENE_KEY);
    let picked = match (scene_id, scenes, legacy) {
        (Some(id), Some(map), _) => map.get(id).cloned().ok_or(CommandError::SceneGone)?,
        (_, Some(map), legacy) => map
            .as_object()
            .and_then(|entries| entries.values().next())
            .cloned()
            .or_else(|| legacy.cloned())
            .ok_or(CommandError::SceneGone)?,
        (_, None, legacy) => legacy.cloned().ok_or(CommandError::SceneGone)?,
    };
    Ok(picked)
}

/// Writes the scene back and reports whether the document changed. The
/// comparison is on the JSON: a key written with the pose it already
/// held counts as "nothing happened", or the undo history would record
/// phantom edits. The scene lands in the `scenes` map (keyed by id) and,
/// while the legacy single-`scene` field is still the one older readers
/// open, there too - one write keeps both shapes true.
fn write_scene(project: &mut Project, scene: &Scene) -> Result<Outcome, CommandError> {
    let value = serde_json::to_value(scene).map_err(|_| CommandError::SceneGone)?;
    let mut changed = false;
    let scenes = project
        .extra
        .entry::<String>(SCENES_KEY.into())
        .or_insert_with(|| Value::Object(Map::new()));
    if let Value::Object(map) = scenes {
        changed |= map.get(scene.id.as_str()) != Some(&value);
        map.insert(scene.id.clone(), value);
    }
    // The legacy mirror stays the single scene the older readers (studio,
    // the pre-Phase-8 editor) open: the first of the map, which is the
    // same scene while a document carries one.
    if let Some(first) = read_first_scene_value(project) {
        let mirrored = project
            .extra
            .get(SCENE_KEY)
            .is_none_or(|previous| *previous != first);
        if mirrored {
            project.extra.insert(SCENE_KEY.into(), first);
            changed = true;
        }
    }
    Ok(Outcome {
        created_id: None,
        applied: changed,
    })
}

/// The first scene of the `scenes` map, for the legacy mirror.
fn read_first_scene_value(project: &Project) -> Option<Value> {
    let map = project.extra.get(SCENES_KEY)?;
    let value = map.as_object()?.values().next().cloned();
    value
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::DocumentSettings;
    use crate::editor::Editor;
    use serde_json::json;

    fn settings() -> DocumentSettings {
        DocumentSettings {
            name: "Test".to_owned(),
            width: 1920,
            height: 1080,
            rate_num: 30,
            rate_den: 1,
            canvas: [0, 0, 0, 255],
        }
    }

    fn editor_with_scene() -> (Editor, String, concat_scene::Scene) {
        let scene = Scene::new("sc1", "Test scene", 4000.0, 2250.0, 1920, 1080);
        let mut editor = Editor::new();
        let id = editor
            .apply(Command::AddSceneClip {
                track_id: None,
                above: false,
                start: 0.0,
                scene: scene.clone(),
                name: None,
            })
            .unwrap()
            .created_id
            .unwrap();
        (editor, id, scene)
    }

    fn scene_of(project: &Project) -> Scene {
        serde_json::from_value(project.extra.get(SCENE_KEY).cloned().unwrap()).unwrap()
    }

    fn key_zoom(frame: f64, zoom: Option<f64>) -> Command {
        Command::SetSceneCameraKey {
            frame,
            x: None,
            y: None,
            zoom,
            rotation: None,
            easing: None,
        }
    }

    /// A scene clip carries the scene into the document, names it after
    /// the scene, and runs for the scene's own length; the whole thing
    /// survives a save and a reopen.
    #[test]
    fn a_scene_clip_embeds_the_scene_places_itself_and_round_trips() {
        let (editor, id, scene) = editor_with_scene();
        let clip = editor.project().active().clip(&id).unwrap().clone();
        assert_eq!(clip.kind, ClipKind::Scene);
        assert_eq!(clip.scene_id.as_deref(), Some("sc1"));
        assert_eq!(clip.name, "Test scene");
        assert_eq!(clip.media_id, "");
        // 90 frames at 30fps: three seconds.
        assert!((clip.duration - 3.0).abs() < 1e-9);
        assert_eq!(scene_of(editor.project()).id, "sc1");

        let document = editor.to_document(&settings());
        let restored = Editor::from_document(&document).expect("loads");
        assert_eq!(restored.project(), editor.project());
        let clip = restored.project().active().clip(&id).unwrap().clone();
        assert_eq!(clip.scene_id.as_deref(), Some("sc1"));
        assert_eq!(scene_of(restored.project()), scene);
    }

    /// The scene command crosses the API as JSON, the way an agent would
    /// send it.
    #[test]
    fn an_add_scene_clip_command_parses_from_the_wire() {
        let command: Command = serde_json::from_value(json!({
            "op": "addSceneClip",
            "trackId": null,
            "above": true,
            "start": 2.0,
            "scene": {
                "id": "sc9", "name": "Wire scene",
                "world_width": 8000.0, "world_height": 4500.0,
                "rate_num": 30, "rate_den": 1,
                "duration_frames": 60.0,
                "camera": { "keys": [] }
            }
        }))
        .expect("the wire shape parses");
        let mut editor = Editor::new();
        let outcome = editor.apply(command).unwrap();
        assert!(outcome.applied);
        let clip = editor
            .project()
            .active()
            .clip(outcome.created_id.as_deref().unwrap())
            .unwrap();
        // 60 frames at 30fps: two seconds.
        assert!((clip.duration - 2.0).abs() < 1e-9);
        assert_eq!(clip.scene_id.as_deref(), Some("sc9"));
    }

    /// Keying the camera is one undoable edit; the pose a key holds is
    /// the evaluated one plus the overrides; and a key written with the
    /// pose it already holds is not a step in the history.
    #[test]
    fn keying_the_scene_camera_is_an_undoable_edit() {
        let (mut editor, _id, _scene) = editor_with_scene();
        assert!(
            editor
                .apply(Command::SetSceneCameraKey {
                    frame: 45.0,
                    x: None,
                    y: None,
                    zoom: Some(2.0),
                    rotation: None,
                    easing: None,
                })
                .unwrap()
                .applied
        );
        let scene = scene_of(editor.project());
        let key = scene
            .camera
            .keys
            .iter()
            .find(|key| (key.frame - 45.0).abs() < KEY_EPSILON)
            .expect("the key landed");
        assert!((key.pose.zoom - 2.0).abs() < 1e-9);
        // The x is what the track evaluated at frame 45 - the centre of
        // the world, where the opening key of a `Scene::new` sits - not a
        // default.
        assert!((key.pose.x - 2000.0).abs() < 1e-9);

        // Halfway between the opening key and this one, the zoom rides.
        let mid = scene_of(editor.project()).camera.pose_at(22.5);
        assert!(mid.zoom > 1.0 && mid.zoom < 2.0);

        assert!(editor.undo());
        assert!(scene_of(editor.project()).camera.keys.len() == 1);

        // The key again is an edit; the same key a second time is not -
        // the phantom edit the outcome contract guards against.
        assert!(editor.apply(key_zoom(45.0, Some(2.0))).unwrap().applied);
        assert!(
            !editor.apply(key_zoom(45.0, Some(2.0))).unwrap().applied,
            "the same pose again is not a step"
        );
    }

    /// A second scene joins the `scenes` map under its own id — the
    /// Phase 8 multi-scene container — and each clip keeps its own
    /// binding. The legacy mirror stays on the first entry.
    #[test]
    fn a_second_scene_coexists_in_the_scenes_map() {
        let (mut editor, first_id, scene) = editor_with_scene();
        let other = Scene {
            id: "sc2".into(),
            name: "第二幕".into(),
            ..scene.clone()
        };
        let second = editor
            .apply(Command::AddSceneClip {
                track_id: None,
                above: false,
                start: 3.0,
                scene: other.clone(),
                name: None,
            })
            .unwrap();
        assert_eq!(editor.project().active().clips.len(), 2);
        let clip = editor
            .project()
            .active()
            .clip(second.created_id.as_deref().unwrap())
            .unwrap()
            .clone();
        assert_eq!(clip.scene_id.as_deref(), Some("sc2"));
        assert_eq!(clip.duration, other.duration_frames() / other.fps());
        let project = editor.project();
        let scenes = project
            .extra
            .get(SCENES_KEY)
            .and_then(|value| value.as_object())
            .expect("scenes map");
        assert_eq!(scenes.len(), 2, "sc1 and sc2 both stored");
        // The legacy mirror still names the first scene.
        assert_eq!(scene_of(editor.project()).id, "sc1");
        let _ = first_id;
    }

    /// The same scene again is no clip and no step: the map entry is
    /// already identical, and a phantom undo step would only clear the
    /// redo stack for nothing. The fixture holds the first insert, so
    /// the apply below re-inserts the scene it carries.
    #[test]
    fn re_inserting_the_same_scene_changes_nothing() {
        let (mut editor, _id, scene) = editor_with_scene();
        let again = editor
            .apply(Command::AddSceneClip {
                track_id: None,
                above: false,
                start: 0.0,
                scene: scene.clone(),
                name: None,
            })
            .unwrap();
        assert!(!again.applied, "the same scene again is not a step");
        assert_eq!(again.created_id, None);
        assert_eq!(editor.project().active().clips.len(), 1);
        // One undo step only - the first insert. With the phantom step
        // the bug pushed, this first undo would restore an identical
        // document and the clip would survive it.
        assert!(editor.undo());
        assert_eq!(editor.project().active().clips.len(), 0);
        // And the redo stack survived the no-op apply.
        assert!(editor.redo());
        assert_eq!(editor.project().active().clips.len(), 1);
    }

    /// A camera key comes off at the frame it was put on, and taking one
    /// off where none is changes nothing.
    #[test]
    fn removing_a_camera_key_and_a_missing_one() {
        let (mut editor, _id, _scene) = editor_with_scene();
        editor
            .apply(Command::SetSceneCameraKey {
                frame: 10.0,
                x: None,
                y: Some(100.0),
                zoom: None,
                rotation: None,
                easing: None,
            })
            .unwrap();
        assert!(
            editor
                .apply(Command::RemoveSceneCameraKey { frame: 10.0 })
                .unwrap()
                .applied
        );
        assert_eq!(scene_of(editor.project()).camera.keys.len(), 1);
        assert!(
            !editor
                .apply(Command::RemoveSceneCameraKey { frame: 10.0 })
                .unwrap()
                .applied
        );
    }

    /// A scene command on a project without a scene errs with the
    /// sentence a user reads, rather than touching the document.
    #[test]
    fn a_scene_command_without_a_scene_is_refused() {
        let mut editor = Editor::new();
        let error = editor.apply(key_zoom(0.0, Some(2.0))).unwrap_err();
        assert!(matches!(error, CommandError::SceneGone));
    }
}
