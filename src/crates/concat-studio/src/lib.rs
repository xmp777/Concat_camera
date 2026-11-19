// SPDX-License-Identifier: AGPL-3.0-or-later
// 2D Mini Studio / Concat_camera：场景 + 摄像机 → Concat 导出管线

//! 把 `concat-scene` 的 2D 场景转换为 `concat-export` 的导出请求，并调用
//! Concat 自己的 GPU 合成/编码管线输出 MP4。这里不实现任何解码、合成或编码。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use concat_export::{ExportRequest, PreviewFrameRequest, Reporter, render_on};
use concat_media::{ReaderPool, probe};
use concat_project::model::Project;
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

/// 把工程中的 Scene 片段展开为带相机动画的图像片段（预览与导出共用）。
///
/// 编辑器的预览与导出都从 `Session::flattened_clips()` 出发，而 flatten
/// 只认识图片/视频/音频。这里在每个 Scene 片段的时间轴窗口内，把场景
/// 图层（按 z 序）重建为图像片段：相机运动按窗口重新采样，烘焙为
/// Scale/OffsetX/OffsetY/Rotation 关键帧（`at` 为片段内 0..=1，线性缓动
/// ——采样已经走过场景相机轨道自身的缓动），交给 Concat 原有的合成器。
/// 场景背景遵循 Concat 的 background 语义：仅当场景片段位于最下方图像
/// 轨时，它展开出的片段才携带 `Background::Color`。图形图层（无素材）
/// 与不可见图层跳过；场景缺失、无法解析或 `scene_id` 不匹配时按原样
/// 返回——flatten 对 Scene 片段失败软跳过，该处画面为画布色。
pub fn expand_project_scenes(project: &Project, project_dir: &Path) -> Project {
    use concat_project::model::{Background, Clip, ClipKey, ClipKind, KeyEase, KeyProperty};

    let Some(scene) = project
        .extra
        .get(SCENE_KEY)
        .and_then(|value| serde_json::from_value::<Scene>(value.clone()).ok())
    else {
        return project.clone();
    };
    let fps = scene.fps();
    let (width, height) = viewport_of(&scene);
    let scene_end = scene.duration_frames();

    let mut media = project.media.clone();
    let mut media_ids: HashMap<PathBuf, String> = HashMap::new();
    let mut media_seq = 0usize;
    let media_id_for = |path: PathBuf,
                            media: &mut Vec<concat_project::model::MediaItem>,
                            ids: &mut HashMap<PathBuf, String>,
                            seq: &mut usize| -> String {
        ids.get(&path).cloned().unwrap_or_else(|| {
            let id = format!("scn{}", seq);
            *seq += 1;
            media.push(concat_project::model::MediaItem {
                id: id.clone(),
                path: path.to_string_lossy().into_owned(),
                name: path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                duration: None,
                kind: concat_project::model::MediaKind::Image,
                width: None,
                height: None,
                frame_rate: None,
                frame_rate_fraction: None,
                video_codec: None,
                audio_codec: None,
                has_audio: false,
                audio_tracks: Vec::new(),
                placeholder: false,
                color_range: None,
                color_space: Default::default(),
                origin: None,
                extra: Default::default(),
            });
            ids.insert(path.clone(), id.clone());
            id
        })
    };

    let mut probes: HashMap<PathBuf, (u32, u32)> = HashMap::new();
    let mut expanded_timelines: Vec<std::sync::Arc<concat_project::model::Timeline>> =
        Vec::new();
    for timeline in project.timelines.iter() {
        let mut clips: Vec<Clip> = Vec::new();
        // 展开片段所在的轨；展开后统一决定哪条轨拿背景。
        let mut scene_tracks: Vec<String> = Vec::new();
        for clip in timeline.clips.iter() {
            if clip.kind != ClipKind::Scene {
                clips.push((**clip).clone());
                continue;
            }
            if clip.scene_id.as_deref() != Some(scene.id.as_str()) {
                continue;
            }
            let track_id = clip.track_id.clone();
            let window = clip.duration.max(0.0);
            let mut group: Vec<Clip> = Vec::new();
            let mut ordered: Vec<&SceneLayer> = scene.layers.iter().collect();
            ordered.sort_by_key(|layer| layer.z);
            for layer in ordered {
                if !layer.visible {
                    continue; // 不可见图层不需要素材，缺素材也不能毁掉整场
                }
                let Some(asset) = layer.asset.as_ref() else {
                    continue; // 图形图层需要素材化后才能进入导出
                };
                let path = resolve(project_dir, asset);
                // 素材尺寸只影响合成器自己的适配；这里 probe 一次仅用于
                // 失败早报，缓存跟随调用存活。
                if media_size(&path, &mut probes).is_err() {
                    continue; // 缺素材的图层与导出单场景一致：跳过而非失败
                }
                let layer_start = layer.in_frame.max(0.0) / fps;
                let layer_frames = if layer.duration_frames > 0.0 {
                    layer.duration_frames
                } else {
                    (scene_end - layer.in_frame.max(0.0)).max(1.0)
                };
                let layer_end = (layer.in_frame.max(0.0) + layer_frames) / fps;
                let inter_start = layer_start.min(window);
                let inter_end = layer_end.min(window);
                if inter_end - inter_start <= 0.0 {
                    continue;
                }
                let samples = sample_layer(
                    &scene,
                    layer,
                    inter_start * fps,
                    (inter_end - inter_start) * fps,
                    width,
                    height,
                );
                if samples.is_empty() {
                    continue;
                }
                let mut out = Clip::blank(
                    format!("{}-{}", clip.id, layer.id),
                    track_id.clone(),
                    ClipKind::Image,
                    layer.name.clone(),
                    clip.start + inter_start,
                    inter_end - inter_start,
                );
                out.media_id = media_id_for(
                    path,
                    &mut media,
                    &mut media_ids,
                    &mut media_seq,
                );
                out.opacity = layer.opacity;
                // 展开片段保留场景归属：既标记来源（背景轨判定用它），
                // 也不让展开产物在二次展开时被再次当作 Scene 片段。
                out.scene_id = Some(scene.id.clone());
                // keyed 属性是绝对值；base 保持中性（Clip::blank），运动
                // 全部走关键帧。
                out.keys = samples
                    .iter()
                    .flat_map(|(at, plan)| {
                        [
                            (KeyProperty::OffsetX, plan.transform.offset_x),
                            (KeyProperty::OffsetY, plan.transform.offset_y),
                            (KeyProperty::Scale, plan.transform.scale),
                            (KeyProperty::Rotation, plan.transform.rotation),
                        ]
                        .into_iter()
                        .map(move |(property, value)| ClipKey {
                            property,
                            at: *at,
                            value,
                            ease: KeyEase::LINEAR,
                        })
                    })
                    .collect();
                group.push(out);
            }
            if !group.is_empty() {
                scene_tracks.push(track_id);
                clips.extend(group);
            }
        }
        let mut expanded = (**timeline).clone();
        expanded.clips = clips.into_iter().map(std::sync::Arc::new).collect();
        // 场景背景只在最下方图像轨生效——与一个真正的图像片段完全同规。
        let lane = expanded.background_lane().map(str::to_owned);
        if let Some(lane) = lane
            && scene_tracks.iter().any(|track| *track == lane)
        {
            let color = scene_background_hex(&scene);
            for clip in expanded.clips.iter_mut() {
                let clip = std::sync::Arc::make_mut(clip);
                if clip.track_id == lane && clip.scene_id.as_deref() == Some(scene.id.as_str()) {
                    clip.background = Some(Background::Color {
                        color: color.clone(),
                    });
                }
            }
        }
        expanded_timelines.push(std::sync::Arc::new(expanded));
    }

    Project {
        media,
        fonts: project.fonts.clone(),
        timelines: expanded_timelines,
        active_timeline_id: project.active_timeline_id.clone(),
        extra: project.extra.clone(),
        unread: project.unread.clone(),
    }
}

/// 场景背景的 `#rrggbb`，透明按黑处理（合成器画布语义）。
fn scene_background_hex(scene: &Scene) -> String {
    let [r, g, b, _a] = scene.background;
    format!(
        "#{:02x}{:02x}{:02x}",
        (r.clamp(0.0, 1.0) * 255.0).round() as u8,
        (g.clamp(0.0, 1.0) * 255.0).round() as u8,
        (b.clamp(0.0, 1.0) * 255.0).round() as u8,
    )
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

    /// 一张真实的 1×1 PNG：probe 与合成器都要能读它。
    const TINY_PNG: &[u8] = &[
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48,
        0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00,
        0x00, 0x1F, 0x15, 0xC4, 0x89, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x44, 0x41, 0x54, 0x78,
        0x9C, 0x63, 0xF8, 0xCF, 0xC0, 0x00, 0x00, 0x03, 0x01, 0x01, 0x00, 0x18, 0xDD, 0x8D,
        0xB0, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
    ];

    fn write_asset(dir: &Path, name: &str) {
        std::fs::write(dir.join(name), TINY_PNG).unwrap();
    }

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

    /// 一个双图层场景片段（背景全程 + 角色后半段进出场）展开为两个图像
    /// 片段：位置/时长按时间轴窗口换算，相机运动烘焙为线性关键帧，素材
    /// 落进合成 media 项，展开片段保留场景归属。
    #[test]
    fn a_scene_clip_expands_to_layer_clips_with_camera_keys() {
        use concat_project::{Command, Editor};

        let mut scene = Scene::new("sc1", "展开", 4000.0, 2250.0, 1920, 1080);
        scene.layers.push(concat_scene::SceneLayer {
            id: "bg".into(),
            name: "背景".into(),
            kind: concat_scene::LayerKind::Background,
            asset: Some("bg.png".into()),
            transform: Default::default(),
            in_frame: 0.0,
            duration_frames: 0.0,
            opacity: 1.0,
            visible: true,
            z: 0,
            expression: None,
        });
        scene.layers.push(concat_scene::SceneLayer {
            id: "hero".into(),
            name: "主角".into(),
            kind: concat_scene::LayerKind::Character,
            asset: Some("hero.png".into()),
            transform: Default::default(),
            in_frame: 30.0,
            duration_frames: 60.0,
            opacity: 1.0,
            visible: true,
            z: 1,
            expression: None,
        });
        // 推近：zoom 1.0 → 2.0。相机尾帧计入场景时长（91 帧 → 3.033s）。
        scene.camera.add_key(
            90.0,
            concat_camera::VirtualCamera {
                zoom: 2.0,
                ..scene.camera.pose_at(90.0)
            },
            concat_scene::Easing::LINEAR,
        );
        let scene_seconds = scene.duration_frames() / scene.fps();

        let mut editor = Editor::new();
        let clip_id = editor
            .apply(Command::AddSceneClip {
                track_id: None,
                above: false,
                start: 1.0,
                scene: scene.clone(),
                name: None,
            })
            .unwrap()
            .created_id
            .unwrap();
        let project = editor.project();
        // 素材在工程目录下（相对路径按工程目录解析）。
        let dir = std::env::temp_dir().join(format!("concat-studio-expand-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        write_asset(&dir, "bg.png");
        write_asset(&dir, "hero.png");

        let expanded = expand_project_scenes(project, &dir);
        let timeline = expanded.active();
        // 展开替换了 Scene 片段本身：flatten 对 Scene 失败软跳过，替换
        // 后那里就是真实的图像片段。
        assert!(timeline.clip(&clip_id).is_none(), "the scene clip is consumed");

        let image_clips: Vec<_> = timeline
            .clips
            .iter()
            .filter(|clip| clip.kind == concat_project::model::ClipKind::Image)
            .collect();
        assert_eq!(image_clips.len(), 2, "两个图层各展开为一个图像片段");
        // 背景层：场景本地 [0s, scene_seconds] → 时间轴 [1s, 1+scene_seconds]。
        let background = image_clips
            .iter()
            .find(|clip| (clip.duration - scene_seconds).abs() < 1e-9)
            .expect("the background expands across the whole window");
        assert!((background.start - 1.0).abs() < 1e-9);
        // 角色层：本地 [1s, 3s] → 时间轴 [2s, 4s]。
        let hero = image_clips
            .iter()
            .find(|clip| (clip.duration - 2.0).abs() < 1e-9)
            .expect("the character expands over its own span");
        assert!((hero.start - 2.0).abs() < 1e-9);
        for clip in [&background, &hero] {
            assert_eq!(clip.scene_id.as_deref(), Some("sc1"), "provenance kept");
            assert!(
                expanded
                    .media
                    .iter()
                    .any(|item| item.id == clip.media_id && item.kind == concat_project::model::MediaKind::Image),
                "the synthetic media item exists"
            );
            // 相机推近：scale 关键帧首尾不同（1/zoom 与 1/2zoom 的差异由
            // 投影决定，这里只断言"运动被烘焙了"）。
            let scales: Vec<_> = clip
                .keys
                .iter()
                .filter(|key| key.property == concat_project::model::KeyProperty::Scale)
                .collect();
            assert!(scales.len() >= 2, "camera motion baked as keys");
            assert_ne!(scales[0].value, scales[scales.len() - 1].value);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 被裁短的 Scene 片段限制展开窗口：窗口外的图层不出现，跨窗口边
    /// 界的图层被截到窗口内。
    #[test]
    fn a_trimmed_scene_clip_limits_the_window() {
        use concat_project::{Command, Editor};

        let mut scene = Scene::new("sc1", "窗口", 4000.0, 2250.0, 1920, 1080);
        scene.layers.push(concat_scene::SceneLayer {
            id: "bg".into(),
            name: "背景".into(),
            kind: concat_scene::LayerKind::Background,
            asset: Some("bg.png".into()),
            transform: Default::default(),
            in_frame: 0.0,
            duration_frames: 0.0,
            opacity: 1.0,
            visible: true,
            z: 0,
            expression: None,
        });
        scene.layers.push(concat_scene::SceneLayer {
            id: "late".into(),
            name: "晚进场".into(),
            kind: concat_scene::LayerKind::Prop,
            asset: Some("prop.png".into()),
            transform: Default::default(),
            in_frame: 60.0,
            duration_frames: 30.0,
            opacity: 1.0,
            visible: true,
            z: 1,
            expression: None,
        });

        let mut editor = Editor::new();
        editor
            .apply(Command::AddSceneClip {
                track_id: None,
                above: false,
                start: 0.0,
                scene,
                name: None,
            })
            .unwrap();
        // 命令里的时长来自场景（3s）；裁到 1.5s。
        let clip_id = editor.project().active().clips[0].id.clone();
        editor
            .apply(Command::TrimClip {
                clip_id,
                edge: concat_project::commands::TrimEdge::End,
                delta: -1.5,
                ripple: false,
            })
            .unwrap();
        let project = editor.project();
        let dir = std::env::temp_dir().join(format!("concat-studio-window-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        write_asset(&dir, "bg.png");
        write_asset(&dir, "prop.png");

        let expanded = expand_project_scenes(project, &dir);
        let timeline = expanded.active();
        let image_clips: Vec<_> = timeline
            .clips
            .iter()
            .filter(|clip| clip.kind == concat_project::model::ClipKind::Image)
            .collect();
        assert_eq!(image_clips.len(), 1, "晚进场的图层在窗口外，不展开");
        assert!(
            (image_clips[0].duration - 1.5).abs() < 1e-9,
            "背景截到窗口长"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 没有场景的工程原样返回：展开是增量的，不是必须的。
    #[test]
    fn a_project_without_a_scene_is_returned_unchanged() {
        use concat_project::Editor;

        let editor = Editor::new();
        let project = editor.project();
        let expanded = expand_project_scenes(project, std::path::Path::new("."));
        assert_eq!(expanded, *project);
    }
}
