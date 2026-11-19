// SPDX-License-Identifier: AGPL-3.0-or-later
// 2D Mini Studio / Concat_camera：场景 + 摄像机 → Concat 导出管线

//! 把 `concat-scene` 的 2D 场景转换为 `concat-export` 的导出请求，并调用
//! Concat 自己的 GPU 合成/编码管线输出 MP4。这里不实现任何解码、合成或编码。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use concat_export::{ExportRequest, PreviewFrameRequest, Reporter, render_on};
use concat_media::{ReaderPool, probe};
use concat_scene::{LayerPlan, Scene, SceneLayer};
use serde_json::{Value, json};

/// Concat 工程文档中存放 2D 场景的顶层字段。
pub const SCENE_KEY: &str = "scene";

/// 读取场景文件。
///
/// 兼容两种格式：
/// - 裸 `Scene` JSON（`examples/camera_demo/scene.json`）；
/// - Concat 工程文档，顶层 `scene` 字段保存场景，其余工程字段原样保留。
pub fn load_scene(path: &Path) -> Result<Scene, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|error| format!("无法读取场景 {}：{error}", path.display()))?;
    let value: Value = serde_json::from_str(&text)
        .map_err(|error| format!("场景 JSON 无效（{}）：{error}", path.display()))?;
    let scene = value.get(SCENE_KEY).cloned().unwrap_or(value);
    serde_json::from_value(scene).map_err(|error| format!("场景 JSON 无效：{error}"))
}

/// 把场景写回场景文件。
///
/// 已有 Concat 工程文档会保留其余字段，只替换 `scene`；裸场景或新文件会
/// 写成一份新的 Concat 工程文档（`concat`/`version`/`timelines` + `scene`）。
pub fn save_scene(path: &Path, scene: &Scene) -> Result<(), String> {
    let mut document = match std::fs::read_to_string(path) {
        Ok(text) => {
            let value: Value = serde_json::from_str(&text)
                .map_err(|error| format!("无法解析工程 {}：{error}", path.display()))?;
            if value.get("concat").is_some() || value.get("timelines").is_some() {
                value
            } else {
                project_document(scene)?
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => project_document(scene)?,
        Err(error) => {
            return Err(format!("无法读取工程 {}：{error}", path.display()));
        }
    };
    let object = document
        .as_object_mut()
        .ok_or_else(|| format!("工程 {} 的顶层不是对象", path.display()))?;
    object.insert(
        SCENE_KEY.to_owned(),
        serde_json::to_value(scene).map_err(|error| format!("无法序列化场景：{error}"))?,
    );
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("无法创建目录 {}：{error}", parent.display()))?;
    }
    let text = serde_json::to_string_pretty(&document)
        .map_err(|error| format!("无法序列化工程：{error}"))?;
    std::fs::write(path, format!("{text}\n"))
        .map_err(|error| format!("无法写入 {}：{error}", path.display()))
}

/// 新工程文档：合法 Concat 工程 + 场景扩展字段（调用方插入）。
fn project_document(scene: &Scene) -> Result<Value, String> {
    let (width, height) = viewport_of(scene);
    let settings = concat_project::DocumentSettings {
        name: scene.name.clone(),
        width,
        height,
        rate_num: scene.rate_num.max(1),
        rate_den: scene.rate_den.max(1),
        canvas: [
            (scene.background[0].clamp(0.0, 1.0) * 255.0).round() as u8,
            (scene.background[1].clamp(0.0, 1.0) * 255.0).round() as u8,
            (scene.background[2].clamp(0.0, 1.0) * 255.0).round() as u8,
            (scene.background[3].clamp(0.0, 1.0) * 255.0).round() as u8,
        ],
    };
    let project = concat_project::Project::with_video(settings.video());
    Ok(concat_project::to_document(&settings, &project))
}

pub struct RenderOptions {
    pub output: PathBuf,
    pub crf: u8,
    pub preset: String,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            output: PathBuf::from("scene.mp4"),
            crf: 20,
            preset: "medium".to_owned(),
        }
    }
}

/// 渲染整个场景为 MP4。
///
/// 图层按 z 序分配轨道；摄像机运动按帧采样烘焙为 clip 的 offset/scale/rotation 关键帧，
/// 由 Concat 的合成器按同一缓动插值。图形图层（无素材）当前跳过。
pub fn render_scene(
    scene: &Scene,
    scene_dir: &Path,
    options: &RenderOptions,
    progress: &mut dyn FnMut(i64, i64, &'static str),
) -> Result<(), String> {
    let cancel = AtomicBool::new(false);
    render_scene_with_cancel(scene, scene_dir, options, &cancel, progress)
}

/// Render with a caller-owned cancel flag, so an MCP client or a dialog can
/// stop a long scene export between frames.
pub fn render_scene_with_cancel(
    scene: &Scene,
    scene_dir: &Path,
    options: &RenderOptions,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(i64, i64, &'static str),
) -> Result<(), String> {
    let (width, height) = viewport_of(scene);
    let clips = scene_clips(scene, scene_dir, &mut HashMap::new())?;
    let canvas = canvas_of(scene);

    let request_value = json!({
        "output": options.output.to_string_lossy(),
        "width": width,
        "height": height,
        "rateNum": scene.rate_num,
        "rateDen": scene.rate_den,
        "crf": options.crf,
        "preset": options.preset,
        "codec": "h264",
        "tenBit": false,
        "rateMode": "vbr",
        "bitrateKbps": 0,
        "colorRange": "limited",
        "colorSpace": "sdr",
        "hdr": false,
        "effectTimeOffset": 0.0,
        "canvas": canvas,
        "clips": clips,
    });
    let request: ExportRequest =
        serde_json::from_value(request_value).map_err(|e| format!("导出请求无效：{e}"))?;

    if let Some(parent) = options.output.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("无法创建输出目录 {}：{e}", parent.display()))?;
    }

    let reporter = Reporter { progress, cancel };
    render_on(&request, None, reporter).map_err(|e| format!("导出失败：{e}"))?;
    Ok(())
}

/// 一帧预览：RGBA 像素与尺寸（相机已完成投影）。
pub struct PreviewFrame {
    pub pixels: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// 复用解码器与素材尺寸的场景预览器。
///
/// 编辑器拖动/滑杆会在同一场景上连续取帧；每次新建 `ReaderPool` 并重新
/// `probe` 所有素材会让固定开销盖过像素合成。这里保留池与素材尺寸缓存，
/// 每次只按当前帧构建一次静态图层，而不是烘焙整条相机轨道。
pub struct ScenePreviewer {
    pool: ReaderPool,
    probes: HashMap<PathBuf, (u32, u32)>,
}

impl Default for ScenePreviewer {
    fn default() -> Self {
        Self {
            pool: ReaderPool::new(64 * 1024 * 1024, 4),
            probes: HashMap::new(),
        }
    }
}

impl ScenePreviewer {
    pub fn new() -> Self {
        Self::default()
    }

    /// 渲染场景的当前帧；`width == 0 || height == 0` 时使用场景输出尺寸。
    pub fn preview_frame(
        &mut self,
        scene: &Scene,
        scene_dir: &Path,
        frame: f64,
        width: u32,
        height: u32,
    ) -> Result<PreviewFrame, String> {
        let (scene_width, scene_height) = viewport_of(scene);
        let (width, height) = if width == 0 || height == 0 {
            (scene_width, scene_height)
        } else {
            (width, height)
        };
        let clips = scene_clips_at(scene, scene_dir, frame, &mut self.probes)?;
        let request_value = json!({
            "time": frame.max(0.0) / scene.fps(),
            "width": width,
            "height": height,
            "rateNum": scene.rate_num.max(1),
            "rateDen": scene.rate_den.max(1),
            "clips": clips,
            "canvas": canvas_of(scene),
        });
        let request: PreviewFrameRequest = serde_json::from_value(request_value)
            .map_err(|error| format!("预览请求无效：{error}"))?;
        let pixels = concat_export::preview_frame(&self.pool, &request)
            .map_err(|error| format!("预览失败：{error}"))?;
        Ok(PreviewFrame {
            pixels,
            width,
            height,
        })
    }
}

/// 用 Concat 自己的预览合成器渲染场景中的一帧（CPU RGBA）。
///
/// 与导出共用同一份图层/相机求值；不经过编码，适合编辑器视口与拖动预览。
pub fn preview_frame(scene: &Scene, scene_dir: &Path, frame: f64) -> Result<PreviewFrame, String> {
    preview_frame_sized(scene, scene_dir, frame, 0, 0)
}

/// Like [`preview_frame`], at a chosen preview size; zero keeps the scene's
/// output frame. A smaller frame is quicker to composite while dragging.
pub fn preview_frame_sized(
    scene: &Scene,
    scene_dir: &Path,
    frame: f64,
    width: u32,
    height: u32,
) -> Result<PreviewFrame, String> {
    ScenePreviewer::new().preview_frame(scene, scene_dir, frame, width, height)
}

/// 场景的图层 clip 列表（导出与预览共用）。
fn scene_clips(
    scene: &Scene,
    scene_dir: &Path,
    probes: &mut HashMap<PathBuf, (u32, u32)>,
) -> Result<Vec<Value>, String> {
    let (width, height) = viewport_of(scene);
    let fps = scene.fps();
    let duration_frames = scene.duration_frames().ceil().max(1.0);
    let mut ordered: Vec<&SceneLayer> = scene.layers.iter().collect();
    ordered.sort_by_key(|layer| layer.z);

    let mut clips: Vec<Value> = Vec::new();
    for (track, layer) in ordered.iter().enumerate() {
        if !layer.visible {
            continue; // hidden layers need no asset, and a missing one must not fail the scene
        }
        let Some(asset) = layer.asset.as_ref() else {
            continue; // 图形图层需要素材化后才能进入导出
        };
        let path = resolve(scene_dir, asset);
        let (media_width, media_height) = media_size(&path, probes)?;

        let start_frame = layer.in_frame.max(0.0);
        let frames = if layer.duration_frames > 0.0 {
            layer.duration_frames
        } else {
            (duration_frames - start_frame).max(1.0)
        };
        let start = start_frame / fps;
        let duration = (frames / fps).max(1.0 / fps);

        let samples = sample_layer(scene, layer, start_frame, frames, width, height);
        if samples.is_empty() {
            return Err(format!("图层 {} 没有可渲染的帧", layer.name));
        }

        let mut animation = Vec::new();
        for (at, plan) in &samples {
            animation.push(key("offsetX", *at, plan.transform.offset_x));
            animation.push(key("offsetY", *at, plan.transform.offset_y));
            animation.push(key("scale", *at, plan.transform.scale));
            animation.push(key("rotation", *at, plan.transform.rotation));
        }

        clips.push(json!({
            "path": path.to_string_lossy(),
            "kind": "image",
            "start": start,
            "duration": duration,
            "sourceStart": 0.0,
            "track": track,
            "hidden": !layer.visible,
            "muted": false,
            "volume": 1.0,
            "mediaWidth": media_width,
            "mediaHeight": media_height,
            "hasAudio": false,
            "mediaDuration": Value::Null,
            // keyed 属性是绝对值；base 必须是中性值，避免与关键帧叠加
            "scale": 1.0,
            "offsetX": 0.0,
            "offsetY": 0.0,
            "rotation": 0.0,
            "stretchX": 1.0,
            "stretchY": 1.0,
            "opacity": layer.opacity,
            "animation": animation,
        }));
    }

    if clips.is_empty() {
        return Err("场景中没有带素材的图层".to_owned());
    }
    Ok(clips)
}

/// 单帧预览的 clip 列表：每个可见图层按当前帧求一次 Transform，不做整条相机动画。
fn scene_clips_at(
    scene: &Scene,
    scene_dir: &Path,
    frame: f64,
    probes: &mut HashMap<PathBuf, (u32, u32)>,
) -> Result<Vec<Value>, String> {
    let (width, height) = viewport_of(scene);
    let fps = scene.fps();
    // One frame past the last shown frame keeps a preview at the very end
    // inside its clips rather than on a black gap.
    let duration = (scene.duration_frames().ceil().max(1.0) + 1.0) / fps;
    let plans: HashMap<String, LayerPlan> = scene
        .plan_layers(frame, width, height)
        .into_iter()
        .map(|plan| (plan.layer_id.clone(), plan))
        .collect();
    let mut ordered: Vec<&SceneLayer> = scene.layers.iter().collect();
    ordered.sort_by_key(|layer| layer.z);

    let mut clips: Vec<Value> = Vec::new();
    for (track, layer) in ordered.iter().enumerate() {
        let Some(plan) = plans.get(&layer.id) else {
            continue;
        };
        let Some(asset) = layer.asset.as_ref() else {
            continue;
        };
        let path = resolve(scene_dir, asset);
        let (media_width, media_height) = media_size(&path, probes)?;
        clips.push(json!({
            "path": path.to_string_lossy(),
            "kind": "image",
            "start": 0.0,
            "duration": duration,
            "sourceStart": 0.0,
            "track": track,
            "hidden": false,
            "muted": false,
            "volume": 1.0,
            "mediaWidth": media_width,
            "mediaHeight": media_height,
            "hasAudio": false,
            "mediaDuration": Value::Null,
            // No animation here: the base values are this frame's transform.
            "scale": plan.transform.scale,
            "offsetX": plan.transform.offset_x,
            "offsetY": plan.transform.offset_y,
            "rotation": plan.transform.rotation,
            "stretchX": plan.transform.stretch_x,
            "stretchY": plan.transform.stretch_y,
            "opacity": layer.opacity,
            "animation": [],
        }));
    }

    if clips.is_empty() {
        return Err("场景中没有可见的素材图层".to_owned());
    }
    Ok(clips)
}

fn media_size(
    path: &Path,
    probes: &mut HashMap<PathBuf, (u32, u32)>,
) -> Result<(u32, u32), String> {
    if let Some(size) = probes.get(path) {
        return Ok(*size);
    }
    let info = probe(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let video = info
        .require_video()
        .map_err(|error| format!("{}: {error}", path.display()))?;
    let size = (video.width, video.height);
    probes.insert(path.to_path_buf(), size);
    Ok(size)
}

fn canvas_of(scene: &Scene) -> [u8; 4] {
    [
        (scene.background[0].clamp(0.0, 1.0) * 255.0).round() as u8,
        (scene.background[1].clamp(0.0, 1.0) * 255.0).round() as u8,
        (scene.background[2].clamp(0.0, 1.0) * 255.0).round() as u8,
        (scene.background[3].clamp(0.0, 1.0) * 255.0).round() as u8,
    ]
}

fn viewport_of(scene: &Scene) -> (u32, u32) {
    scene
        .camera
        .keys
        .first()
        .map(|k| (k.pose.viewport_width, k.pose.viewport_height))
        .unwrap_or((1920, 1080))
}

fn resolve(scene_dir: &Path, asset: &str) -> PathBuf {
    let path = Path::new(asset);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        scene_dir.join(path)
    }
}

fn key(property: &str, at: f64, value: f64) -> Value {
    json!({
        "property": property,
        "at": at.clamp(0.0, 1.0),
        "value": value,
        "ease": [0.0, 0.0, 1.0, 1.0],
    })
}

/// 在图层时间范围内采样摄像机投影（最多约 61 个采样点，首尾必含）
fn sample_layer(
    scene: &Scene,
    layer: &SceneLayer,
    start_frame: f64,
    frames: f64,
    width: u32,
    height: u32,
) -> Vec<(f64, LayerPlan)> {
    let steps = (frames / 2.0).ceil().clamp(1.0, 60.0) as i64;
    let mut out = Vec::new();
    for index in 0..=steps {
        let offset = frames * (index as f64) / (steps as f64);
        let frame = start_frame + offset;
        let at = if frames > 0.0 { offset / frames } else { 0.0 };
        if let Some(plan) = scene
            .plan_layers(frame, width, height)
            .into_iter()
            .find(|plan| plan.layer_id == layer.id)
        {
            out.push((at, plan));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scenes_roundtrip_bare_and_inside_a_concat_project() {
        let path =
            std::env::temp_dir().join(format!("concat-studio-scene-{}.json", std::process::id()));
        let mut scene = Scene::new("dialogue", "双人对话", 8000.0, 4500.0, 640, 360);
        scene.layers.push(concat_scene::SceneLayer {
            id: "hero".into(),
            name: "主角".into(),
            kind: concat_scene::LayerKind::Character,
            asset: Some("hero.png".into()),
            transform: concat_scene::SceneTransform {
                x: 3600.0,
                y: 2250.0,
                scale: 0.65,
                ..Default::default()
            },
            in_frame: 0.0,
            duration_frames: 120.0,
            opacity: 1.0,
            visible: true,
            z: 1,
            expression: Some("smile".into()),
        });

        // A bare scene file reads as itself.
        std::fs::write(&path, serde_json::to_string_pretty(&scene).expect("writes"))
            .expect("bare scene");
        assert_eq!(load_scene(&path).expect("reads bare"), scene);

        // Saving turns it into a Concat project document with `scene`.
        save_scene(&path, &scene).expect("saves");
        let document: Value =
            serde_json::from_str(&std::fs::read_to_string(&path).expect("read back"))
                .expect("valid JSON");
        assert!(document["concat"].is_string(), "{document}");
        assert_eq!(document["scene"]["id"], "dialogue");
        assert_eq!(load_scene(&path).expect("reads project"), scene);

        // Saving again keeps the project fields and updates the scene.
        scene.name = "第二幕".into();
        save_scene(&path, &scene).expect("saves again");
        let document: Value =
            serde_json::from_str(&std::fs::read_to_string(&path).expect("read back"))
                .expect("valid JSON");
        assert!(document["concat"].is_string());
        assert_eq!(document["scene"]["name"], "第二幕");
        assert_eq!(load_scene(&path).expect("reads again"), scene);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn preview_frame_uses_the_same_scene_projection_as_export() {
        let demo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../examples/camera_demo");
        let scene = load_scene(&demo.join("scene.json")).expect("the demo scene");
        let first = preview_frame(&scene, &demo, 0.0).expect("frame 0");
        let last = preview_frame(&scene, &demo, 89.0).expect("frame 89");

        assert_eq!(first.width, 1920);
        assert_eq!(first.height, 1080);
        assert_eq!(
            first.pixels.len(),
            (first.width * first.height * 4) as usize
        );
        assert_eq!(last.pixels.len(), (last.width * last.height * 4) as usize);
        assert!(
            first.pixels.chunks_exact(4).any(|pixel| pixel[0] > 16),
            "the background and characters are drawn"
        );
        assert_ne!(first.pixels, last.pixels, "the camera moved between frames");
    }

    #[test]
    fn a_hidden_layer_with_a_missing_asset_does_not_fail_the_preview() {
        let demo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../examples/camera_demo");
        let mut scene = load_scene(&demo.join("scene.json")).expect("the demo scene");
        scene.layers.push(concat_scene::SceneLayer {
            id: "hidden".into(),
            name: "hidden".into(),
            kind: concat_scene::LayerKind::Prop,
            asset: Some("missing.png".into()),
            transform: concat_scene::SceneTransform {
                x: scene.world_width / 2.0,
                y: scene.world_height / 2.0,
                ..Default::default()
            },
            in_frame: 0.0,
            duration_frames: 0.0,
            opacity: 1.0,
            visible: false,
            z: 99,
            expression: None,
        });
        let picture = ScenePreviewer::new()
            .preview_frame(&scene, &demo, 0.0, 320, 180)
            .expect("a hidden layer is not probed");
        assert_eq!(picture.pixels.len(), 320 * 180 * 4);
    }

    #[test]
    fn the_cached_previewer_matches_a_one_off_preview() {
        let demo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../examples/camera_demo");
        let scene = load_scene(&demo.join("scene.json")).expect("the demo scene");
        let mut previewer = ScenePreviewer::new();
        let cached = previewer
            .preview_frame(&scene, &demo, 30.0, 960, 540)
            .expect("cached frame");
        let one_off = preview_frame_sized(&scene, &demo, 30.0, 960, 540).expect("one-off frame");
        assert_eq!(cached.width, one_off.width);
        assert_eq!(cached.height, one_off.height);
        assert_eq!(
            cached.pixels, one_off.pixels,
            "the single-frame path draws the same frame as the export-shaped preview"
        );

        let later = previewer
            .preview_frame(&scene, &demo, 89.0, 960, 540)
            .expect("later frame");
        assert_ne!(cached.pixels, later.pixels, "the camera moved");
    }
}
