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

/// 多场景容器：文档顶层按场景 id 存取的 map（Phase 8）。
pub const SCENES_KEY: &str = "scenes";

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

/// 从工程文档（或裸场景 JSON）按 id 读取一个场景。
///
/// - `Some(id)`：文档带 `scenes` map（Phase 8 的多场景容器）时**必须**
///   在 map 里命中——落空即报错，不再静默回退旧的单-`scene` 字段（放置
///   是有副作用的操作，拼错的 id 拿到错误的场景比拿到报错更糟）；无 map
///   的旧文档回退单-`scene` 字段，但场景 id 必须与请求的一致。
/// - `None`：map 存在取第一条（serde_json 的对象按 id 字典序，取最小），
///   否则单-`scene` 字段，否则按裸场景解析。
pub fn read_scene_by_id(document: &Value, scene_id: Option<&str>) -> Result<Scene, String> {
    let parse = |value: &Value| {
        serde_json::from_value::<Scene>(value.clone())
            .map_err(|error| format!("场景 JSON 无效：{error}"))
    };
    let entries = document.get(SCENES_KEY).and_then(|value| value.as_object());
    match (scene_id, entries) {
        // The map answers for named scenes - and only the map: a miss
        // names the error rather than quietly reading another scene.
        (Some(id), Some(entries)) => {
            let value = entries
                .get(id)
                .ok_or_else(|| format!("文档中没有 id 为 {id:?} 的场景"))?;
            parse(value)
        }
        // No map: the legacy single-scene field, and the id must agree.
        (Some(id), None) => {
            let scene = document
                .get(SCENE_KEY)
                .ok_or_else(|| format!("文档中没有 id 为 {id:?} 的场景"))
                .and_then(parse)?;
            if scene.id != id {
                return Err(format!("文档中没有 id 为 {id:?} 的场景"));
            }
            Ok(scene)
        }
        // Unnamed: the map's first entry, else the legacy field, else
        // the whole document as a bare scene.
        (None, Some(entries)) => entries
            .values()
            .next()
            .ok_or_else(|| "文档的 scenes map 为空".to_owned())
            .and_then(parse),
        (None, None) => {
            let value = document
                .get(SCENE_KEY)
                .cloned()
                .unwrap_or_else(|| document.clone());
            parse(&value)
        }
    }
}

/// 列出工程文档 `scenes` map 里的全部场景 `(id, name)`；单-`scene`
/// 文档返回它的一个条目。
pub fn list_scenes(document: &Value) -> Vec<(String, String)> {
    let from_map = document
        .get("scenes")
        .and_then(|value| value.as_object())
        .map(|entries| {
            entries
                .iter()
                .filter_map(|(id, scene)| {
                    scene
                        .get("name")
                        .and_then(|name| name.as_str())
                        .map(|name| (id.clone(), name.to_owned()))
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if !from_map.is_empty() {
        return from_map;
    }
    document
        .get(SCENE_KEY)
        .and_then(|scene| {
            let id = scene.get("id")?.as_str()?.to_owned();
            let name = scene.get("name")?.as_str()?.to_owned();
            Some(vec![(id, name)])
        })
        .unwrap_or_default()
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
        let Ok(path) = layer_asset_path(layer, scene_dir) else {
            continue; // 无素材也无形状：跳过（图形已素材化的走 PNG 路径）
        };
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
            // 非均匀拉伸与位移/缩放同批烘焙：投影给出的 stretch 丢了，
            // 整场导出会把带 stretch 的图层悄悄拉直（与展开路径同规）。
            animation.push(key("stretchX", *at, plan.transform.stretch_x));
            animation.push(key("stretchY", *at, plan.transform.stretch_y));
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
        let Ok(path) = layer_asset_path(layer, scene_dir) else {
            continue;
        };
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
/// Scale/OffsetX/OffsetY/Rotation/StretchX/StretchY 关键帧（`at` 为片段内
/// 0..=1，线性缓动——采样已经走过场景相机轨道自身的缓动），交给 Concat
/// 原有的合成器。场景背景遵循 Concat 的 background 语义：仅当场景片段
/// 位于最下方图像轨时，它展开出的片段才携带 `Background::Color`。图形
/// 图层（无素材）与不可见图层跳过；场景缺失、无法解析或 `scene_id`
/// 不匹配时按原样返回——flatten 对 Scene 片段失败软跳过，该处画面为
/// 画布色。
///
/// `probes` 由调用方持有（Session 生命周期）：素材尺寸的 probe 是文件
/// I/O，逐帧展开时命中缓存就不再读文件。
pub fn expand_project_scenes(
    project: &Project,
    project_dir: &Path,
    probes: &mut HashMap<PathBuf, (u32, u32)>,
) -> Project {
    use concat_project::model::{Background, Clip, ClipKey, ClipKind, KeyEase, KeyProperty};

    // A document may carry several scenes (`scenes` keyed by id); a clip
    // expands the scene its `scene_id` names. The legacy single-`scene`
    // field answers for clips that name it, and for documents that have
    // only that.
    let parse = |value: &Value| serde_json::from_value::<Scene>(value.clone()).ok();
    let scenes_map: HashMap<String, Scene> = project
        .extra
        .get(SCENES_KEY)
        .and_then(|value| value.as_object())
        .map(|entries| {
            entries
                .iter()
                .filter_map(|(id, scene)| parse(scene).map(|scene| (id.clone(), scene)))
                .collect()
        })
        .unwrap_or_default();
    let legacy = project.extra.get(SCENE_KEY).and_then(|value| parse(value));
    let scene_of = |clip_scene_id: Option<&str>| -> Option<Scene> {
        clip_scene_id
            .and_then(|id| scenes_map.get(id).cloned())
            .or_else(|| legacy.clone())
    };

    let mut media = project.media.clone();
    let mut media_ids: HashMap<PathBuf, String> = HashMap::new();
    let mut media_seq = 0usize;
    // 合成 MediaItem 的尺寸必须真实：flatten 把 `media.width/height` 交
    // 给 ExportClip，合成器按它适配画面——None 会让展开图层渲染成空帧
    // （Phase 5 验收里"面板黑屏"的实质之一）。
    let media_id_for = |path: PathBuf,
                        media: &mut Vec<concat_project::model::MediaItem>,
                        ids: &mut HashMap<PathBuf, String>,
                        seq: &mut usize,
                        probes: &mut HashMap<PathBuf, (u32, u32)>|
     -> String {
        ids.get(&path).cloned().unwrap_or_else(|| {
            // `scn` 前缀在 "m" 命名空间之外；即便工程里手工造过同名的
            // 素材项，也递增到没有碰撞为止——flatten 按首个 id 匹配。
            let mut id = format!("scn{}", seq);
            while media.iter().any(|item| item.id == id) {
                *seq += 1;
                id = format!("scn{}", seq);
            }
            *seq += 1;
            let (media_width, media_height) = media_size(&path, probes).unwrap_or((0, 0));
            media.push(concat_project::model::MediaItem {
                id: id.clone(),
                path: path.to_string_lossy().into_owned(),
                name: path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                duration: None,
                kind: concat_project::model::MediaKind::Image,
                width: (media_width > 0).then_some(media_width),
                height: (media_height > 0).then_some(media_height),
                frame_rate: None,
                frame_rate_fraction: None,
                video_codec: None,
                audio_codec: None,
                has_audio: false,
                audio_tracks: Vec::new(),
                beats: None,
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

    let mut expanded_timelines: Vec<std::sync::Arc<concat_project::model::Timeline>> = Vec::new();
    for timeline in project.timelines.iter() {
        let mut tracks = timeline.tracks.clone();
        let mut clips: Vec<Clip> = Vec::new();
        // 展开片段所在的轨；展开后统一决定哪条轨拿背景。
        let mut scene_tracks: Vec<(String, String, String)> = Vec::new();
        for clip in timeline.clips.iter() {
            if clip.kind != ClipKind::Scene {
                clips.push((**clip).clone());
                continue;
            }
            // 按片段的 scene_id 找它要播的场景；找不到（或没写）就回落
            // 到文档的第一个场景——单场景工程的片段通常连 id 都省。
            let Some(clip_scene) = scene_of(clip.scene_id.as_deref()).or_else(|| scene_of(None))
            else {
                continue;
            };
            let clip_fps = clip_scene.fps();
            let clip_width = clip_scene
                .camera
                .keys
                .first()
                .map(|key| key.pose.viewport_width)
                .unwrap_or(1920);
            let clip_height = clip_scene
                .camera
                .keys
                .first()
                .map(|key| key.pose.viewport_height)
                .unwrap_or(1080);
            let scene_end = clip_scene.duration_frames();
            // trim/speed 语义：`source_start` 是用户裁头时定的场景本地
            // 入点（秒）；`speed` 缩放场景时间相对时间轴的流速。窗口是
            // 场景帧区间 [in, in + duration × speed × fps)。
            let speed = clip.speed.max(0.01);
            let source_frames = clip.source_start.max(0.0) * clip_fps;
            let frames_per_sec = clip_fps * speed;
            let window_frames = clip.duration.max(0.0) * frames_per_sec;
            let window = (source_frames, source_frames + window_frames);
            let mut ordered: Vec<&SceneLayer> = clip_scene.layers.iter().collect();
            ordered.sort_by_key(|layer| layer.z);
            // 合成器每条轨在同一时刻只认一个 clip（后者覆盖前者）——
            // 场景的图层必须各占一条轨：**第一个实际产出片段的层**留在
            // 场景片段的原轨（背景轨语义随它走；隐藏/缺素材的层不算），
            // 其余每层在原轨之上各 mint 一条新轨，顺序与 z 一致。新轨
            // 插在原轨之后、其余既有轨之前——场景层盖住它下面的剪辑，
            // 但不盖住本来就摆在场景片段上方的字幕与特效。
            let anchor_row = tracks
                .iter()
                .position(|track| track.id == clip.track_id)
                .unwrap_or(tracks.len().saturating_sub(1));
            let mut minted: usize = 0;
            let mut group: Vec<(String, Clip)> = Vec::new();
            for layer in ordered.iter() {
                if !layer.visible {
                    continue; // 不可见图层不需要素材，缺素材也不能毁掉整场
                }
                let Ok(path) = layer_asset_path(layer, project_dir) else {
                    continue; // 无素材也无形状：跳过
                };
                // 素材尺寸只影响合成器自己的适配；probe 一次进调用方的
                // 缓存，逐帧展开时命中即不再读文件（shape 素材在
                // layer_asset_path 已预填尺寸，不会落盘后再读）。
                if media_size(&path, probes).is_err() {
                    continue; // 缺素材的图层与导出单场景一致：跳过而非失败
                }
                let layer_start = layer.in_frame.max(0.0);
                let layer_frames = if layer.duration_frames > 0.0 {
                    layer.duration_frames
                } else {
                    (scene_end - layer.in_frame.max(0.0)).max(1.0)
                };
                let layer_end = layer.in_frame.max(0.0) + layer_frames;
                // 图层的场景帧区间与片段窗口（场景帧）求交；交出的帧区间
                // 再除以速度换回时间轴秒。
                let inter_scene_start = layer_start.max(window.0);
                let inter_scene_end = layer_end.min(window.1);
                if inter_scene_end - inter_scene_start <= 0.0 {
                    continue;
                }
                let timeline_start =
                    clip.start + (inter_scene_start - source_frames) / frames_per_sec;
                let timeline_dur = (inter_scene_end - inter_scene_start) / frames_per_sec;
                let samples = sample_layer(
                    &clip_scene,
                    layer,
                    inter_scene_start,
                    inter_scene_end - inter_scene_start,
                    clip_width,
                    clip_height,
                );
                if samples.is_empty() {
                    continue;
                }
                let layer_track = if group.is_empty() {
                    clip.track_id.clone()
                } else {
                    let id = format!("{}-{}-{}", clip.track_id, clip.id, layer.id);
                    if !tracks.iter().any(|track| track.id == id) {
                        let row = anchor_row + minted + 1;
                        tracks.insert(
                            row.min(tracks.len()),
                            concat_project::model::Track {
                                id: id.clone(),
                                visible: true,
                                muted: false,
                                extra: Default::default(),
                            },
                        );
                        minted += 1;
                    }
                    id
                };
                let mut out = Clip::blank(
                    format!("{}-{}", clip.id, layer.id),
                    layer_track.clone(),
                    ClipKind::Image,
                    layer.name.clone(),
                    timeline_start,
                    timeline_dur,
                );
                out.media_id =
                    media_id_for(path, &mut media, &mut media_ids, &mut media_seq, probes);
                out.opacity = layer.opacity;
                // 展开片段保留场景归属：既标记来源（背景轨判定用它），
                // 也不让展开产物在二次展开时被再次当作 Scene 片段。
                out.scene_id = Some(clip_scene.id.clone());
                // keyed 属性是绝对值；base 保持中性（Clip::blank），运动
                // 全部走关键帧——相机投影与图层自身的非均匀拉伸都在其列。
                out.keys = samples
                    .iter()
                    .flat_map(|(at, plan)| {
                        [
                            (KeyProperty::OffsetX, plan.transform.offset_x),
                            (KeyProperty::OffsetY, plan.transform.offset_y),
                            (KeyProperty::Scale, plan.transform.scale),
                            (KeyProperty::Rotation, plan.transform.rotation),
                            (KeyProperty::StretchX, plan.transform.stretch_x),
                            (KeyProperty::StretchY, plan.transform.stretch_y),
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
                group.push((layer_track, out));
            }
            if !group.is_empty() {
                scene_tracks.extend(group.iter().map(|(track, _)| {
                    (
                        track.clone(),
                        clip_scene.id.clone(),
                        scene_background_hex(&clip_scene),
                    )
                }));
                clips.extend(group.into_iter().map(|(_, clip)| clip));
            }
        }
        let mut expanded = (**timeline).clone();
        expanded.tracks = tracks;
        expanded.clips = clips.into_iter().map(std::sync::Arc::new).collect();
        // 场景背景只在最下方图像轨生效——与一个真正的图像片段完全同规。
        // 多场景下每条轨按自己的场景取色。
        let lane = expanded.background_lane().map(str::to_owned);
        if let Some(lane) = lane
            && let Some((_, lane_colour, lane_scene_id)) =
                scene_tracks.iter().find(|(track, _, _)| *track == lane)
        {
            let color = lane_colour.clone();
            let lane_scene_id = lane_scene_id.clone();
            for clip in expanded.clips.iter_mut() {
                let clip = std::sync::Arc::make_mut(clip);
                if clip.track_id == lane && clip.scene_id.as_deref() == Some(lane_scene_id.as_str())
                {
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

/// FNV-1a（64 位）：图形素材文件名的稳定指纹——std 的 DefaultHasher
/// 带随机键，跨运行会变名，文件名就会漂移。
pub fn fnv1a(text: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// 把图形图层画成 PNG 并写入 `out`。渲染委托给 Concat 自己的
/// `concat_text::render_shape`（与编辑器 shape 片段共用同一套栅格化
/// 路径），这里只做 `ShapeData → ShapeStyle` 的薄转换。
/// fill/stroke 都缺的图形是一张空图——"画了，但什么都没有"。
/// 形状的像素取 `ShapeData` 的宽高（钳到 1..=4096）；世界摆位与相机
/// 投影由图层 transform 承担，与图片图层完全同规。
pub fn materialize_shape(shape: &concat_scene::ShapeData, out: &Path) -> Result<(), String> {
    let width = shape.width.round().clamp(1.0, 4096.0) as u32;
    let height = shape.height.round().clamp(1.0, 4096.0) as u32;
    let style = shape_style_of(shape);
    let rendered = concat_text::render_shape(&style, width, height)
        .map_err(|error| format!("无法渲染形状：{error}"))?;
    // 同一形状可能被预览与导出线程同时首次素材化：先写临时名再就位，
    // 避免两个句柄交错出一张撕裂的 PNG 被存在性检查永久当真。
    let tmp = out.with_extension("tmp");
    std::fs::write(&tmp, &rendered.png)
        .map_err(|error| format!("无法写出形状素材 {}：{error}", out.display()))?;
    std::fs::rename(&tmp, out).map_err(|error| {
        let _ = std::fs::remove_file(&tmp);
        format!("无法就位形状素材 {}：{error}", out.display())
    })
}

/// rgba 0..=1 数组 → ShapeStyle 的 `#rrggbb[aa]` 字符串。
fn rgba_to_hex(rgba: [f32; 4]) -> String {
    let to8 = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    if rgba[3] < 1.0 {
        format!(
            "#{:02x}{:02x}{:02x}{:02x}",
            to8(rgba[0]),
            to8(rgba[1]),
            to8(rgba[2]),
            to8(rgba[3])
        )
    } else {
        format!(
            "#{:02x}{:02x}{:02x}",
            to8(rgba[0]),
            to8(rgba[1]),
            to8(rgba[2])
        )
    }
}

/// `ShapeData` → `concat_text::ShapeStyle`：图形种类映射（Rect→Square、
/// Ellipse→Circle、Line→Line），fill/stroke 转 hex，描边粗细从绝对像素
/// 转为画布高度分数（`render_shape` 的约定）。
fn shape_style_of(shape: &concat_scene::ShapeData) -> concat_text::ShapeStyle {
    use concat_text::{Figure, ShapeStyle};
    let height = shape.height.round().clamp(1.0, 4096.0).max(1.0);
    let figure = match shape.kind {
        concat_scene::ShapeKind::Rect => Figure::Square,
        concat_scene::ShapeKind::Ellipse => Figure::Circle,
        concat_scene::ShapeKind::Line => Figure::Line,
    };
    // render_shape 的换算：图形像素宽 = style.width × 画布高。画布就是
    // 图形自己的像素尺寸，所以宽分数 = 宽/高。
    ShapeStyle {
        figure,
        width: shape.width / height,
        height: 1.0,
        fill: shape.fill.map(rgba_to_hex).unwrap_or_default(),
        stroke: shape.stroke.map(rgba_to_hex).unwrap_or_default(),
        stroke_width: shape.stroke_width / height,
        ..ShapeStyle::default()
    }
}

/// 一个图层的素材落盘路径：**只认 `asset`**。渲染/预览/展开路径在这
/// 里零写盘——形状图层的 PNG 由受写校验保护的写入点（MCP
/// `scene_add_layer`、编辑器保存）物化到 `assets/scene-shapes/` 并回填
/// asset；"有 shape 无 asset"的图层在此跳层，绝不在渲染期写盘（沙箱
/// 不变量：任何写都落在写根之内，见 concat-api 的书面约定）。probes
/// 仍由调用方传入以缓存已存在素材的尺寸。
fn layer_asset_path(layer: &SceneLayer, scene_dir: &Path) -> Result<PathBuf, String> {
    let asset = layer
        .asset
        .as_ref()
        .ok_or_else(|| "图层没有素材".to_owned())?;
    Ok(resolve(scene_dir, asset))
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
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F,
        0x15, 0xC4, 0x89, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0xF8,
        0xCF, 0xC0, 0x00, 0x00, 0x03, 0x01, 0x01, 0x00, 0x18, 0xDD, 0x8D, 0xB0, 0x00, 0x00, 0x00,
        0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
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
            shape: None,
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

    /// `read_scene_by_id` 的多场景语义：命中的 id 从 map 读，map 里
    /// 落空的 id 报错（不静默回退旧字段），没点名时取 map 第一条。
    #[test]
    fn a_named_scene_id_answers_from_the_scenes_map_or_errs() {
        let first = Scene::new("sc1", "第一幕", 4000.0, 2250.0, 640, 360);
        let second = Scene::new("sc2", "第二幕", 4000.0, 2250.0, 640, 360);
        let document = json!({
            "concat": "x",
            "scenes": {
                "sc2": serde_json::to_value(&second).unwrap(),
                "sc1": serde_json::to_value(&first).unwrap(),
            },
            // 旧字段指向第一幕：map 命中的读取不该看它一眼。
            "scene": serde_json::to_value(&second).unwrap(),
        });

        let named = read_scene_by_id(&document, Some("sc1")).expect("the map answers for sc1");
        assert_eq!(
            named.name, "第一幕",
            "read from the map, not the legacy field"
        );
        assert_eq!(
            read_scene_by_id(&document, Some("sc2"))
                .expect("the map answers for sc2")
                .name,
            "第二幕"
        );
        let why = read_scene_by_id(&document, Some("sc3")).expect_err("a miss names the miss");
        assert!(why.contains("sc3"), "{why}");

        // 没点名：map 第一条，按 id 字典序（serde_json 对象有序）。
        let unnamed = read_scene_by_id(&document, None).expect("the first entry");
        assert_eq!(unnamed.id, "sc1");
    }

    /// 无 map 的旧文档按单-`scene` 字段读：点名的 id 必须与它一致。
    #[test]
    fn a_legacy_document_agrees_with_the_requested_id() {
        let scene = Scene::new("dialogue", "双人对话", 4000.0, 2250.0, 640, 360);
        let document = json!({
            "concat": "x",
            "scene": serde_json::to_value(&scene).unwrap(),
        });
        let read = read_scene_by_id(&document, Some("dialogue")).expect("the id agrees");
        assert_eq!(read.id, "dialogue");
        let why = read_scene_by_id(&document, Some("other")).expect_err("a wrong id errs");
        assert!(why.contains("other"), "{why}");
        let unnamed = read_scene_by_id(&document, None).expect("unnamed reads the field");
        assert_eq!(unnamed.id, "dialogue");
        // 裸场景 JSON：没点名按整份文档解析。
        let bare = serde_json::to_value(&scene).unwrap();
        assert_eq!(read_scene_by_id(&bare, None).expect("bare").id, "dialogue");
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
            shape: None,
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
            shape: None,
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
            shape: None,
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

        let expanded = expand_project_scenes(project, &dir, &mut HashMap::new());
        let timeline = expanded.active();
        // 展开替换了 Scene 片段本身：flatten 对 Scene 失败软跳过，替换
        // 后那里就是真实的图像片段。
        assert!(
            timeline.clip(&clip_id).is_none(),
            "the scene clip is consumed"
        );

        let image_clips: Vec<_> = timeline
            .clips
            .iter()
            .filter(|clip| clip.kind == concat_project::model::ClipKind::Image)
            .collect();
        assert_eq!(image_clips.len(), 2, "两个图层各展开为一个图像片段");
        // 合成器每轨在同一时刻只认一个 clip：两层必须各占一条轨，否则
        // 低 z 层整体消失（审查实机复现过）。
        let (bg, hero) = (
            image_clips
                .iter()
                .find(|clip| (clip.duration - scene_seconds).abs() < 1e-9)
                .expect("the background clip"),
            image_clips
                .iter()
                .find(|clip| (clip.duration - 2.0).abs() < 1e-9)
                .expect("the hero clip"),
        );
        assert_ne!(bg.track_id, hero.track_id, "one track per layer");
        assert!(
            timeline
                .tracks
                .iter()
                .any(|track| track.id.ends_with("-hero")),
            "the character rides a minted track of its own"
        );
        // 非均匀拉伸也随投影进关键帧，不悄悄拉直。
        assert!(
            hero.keys
                .iter()
                .any(|key| key.property == concat_project::model::KeyProperty::StretchX)
        );
        // 背景层：场景本地 [0s, scene_seconds] → 时间轴 [1s, 1+scene_seconds]；
        // 角色层：本地 [1s, 3s] → 时间轴 [2s, 4s]。
        assert!((bg.start - 1.0).abs() < 1e-9);
        assert!((hero.start - 2.0).abs() < 1e-9);
        for clip in [bg, hero] {
            assert_eq!(clip.scene_id.as_deref(), Some("sc1"), "provenance kept");
            // 尺寸来自 probe（flatten 交给合成器适配画面）——None 会让
            // 展开图层渲染成空帧。
            assert!(
                expanded.media.iter().any(|item| item.id == clip.media_id
                    && item.kind == concat_project::model::MediaKind::Image
                    && item.width == Some(1)
                    && item.height == Some(1)),
                "the synthetic media item exists with its probed size"
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
            shape: None,
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
            shape: None,
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

        let expanded = expand_project_scenes(project, &dir, &mut HashMap::new());
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
        let expanded =
            expand_project_scenes(project, std::path::Path::new("."), &mut HashMap::new());
        assert_eq!(expanded, *project);
    }

    /// 整场导出的逐层烘焙带上非均匀拉伸：投影给出的 stretch 丢了，
    /// 带 stretch 的图层会被悄悄拉直（与展开路径同规）。
    #[test]
    fn the_whole_scene_export_bakes_stretch() {
        use concat_scene::{LayerKind, SceneLayer};

        let dir =
            std::env::temp_dir().join(format!("concat-studio-stretch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        write_asset(&dir, "bg.png");

        let mut scene = Scene::new("s", "拉伸", 1000.0, 1000.0, 64, 64);
        scene.layers.push(SceneLayer {
            id: "l1".into(),
            name: "层".into(),
            kind: LayerKind::Prop,
            asset: Some("bg.png".into()),
            shape: None,
            transform: concat_scene::SceneTransform {
                stretch_x: 1.5,
                stretch_y: 0.75,
                ..Default::default()
            },
            in_frame: 0.0,
            duration_frames: 0.0,
            opacity: 1.0,
            visible: true,
            z: 0,
            expression: None,
        });

        let clips = scene_clips(&scene, &dir, &mut HashMap::new()).expect("clips");
        let animation = clips[0]["animation"].as_array().expect("animation");
        let stretched_x = animation
            .iter()
            .find(|key| key["property"] == "stretchX" && key["value"] == 1.5);
        let stretched_y = animation
            .iter()
            .find(|key| key["property"] == "stretchY" && key["value"] == 0.75);
        assert!(stretched_x.is_some(), "stretchX baked into the keys");
        assert!(stretched_y.is_some(), "stretchY baked too");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 图形素材化的像素：矩形中心是填充色；椭圆四角透明、中心填色；
    /// line 的中缝是笔色、上下透明。
    #[test]
    fn a_shape_materializes_to_the_pixels_it_names() {
        let dir = std::env::temp_dir().join(format!("concat-studio-shape-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("rect.png");
        materialize_shape(
            &concat_scene::ShapeData {
                kind: concat_scene::ShapeKind::Rect,
                width: 40.0,
                height: 20.0,
                fill: Some([1.0, 0.0, 0.0, 1.0]),
                stroke: None,
                stroke_width: 0.0,
            },
            &out,
        )
        .expect("the rect paints");
        let decoded = image::open(&out).expect("reads back").into_rgba8();
        let (w, h) = (decoded.width(), decoded.height());
        assert_eq!((w, h), (40, 20));
        let centre = decoded.get_pixel(w / 2, h / 2);
        assert_eq!(centre.0, [255, 0, 0, 255], "the fill colour in the middle");

        let ellipse = dir.join("ellipse.png");
        materialize_shape(
            &concat_scene::ShapeData {
                kind: concat_scene::ShapeKind::Ellipse,
                width: 40.0,
                height: 20.0,
                fill: Some([0.0, 1.0, 0.0, 1.0]),
                stroke: None,
                stroke_width: 0.0,
            },
            &ellipse,
        )
        .expect("the ellipse paints");
        let decoded = image::open(&ellipse).expect("reads back").into_rgba8();
        let corner = decoded.get_pixel(0, 0);
        assert_eq!(corner.0[3], 0, "an ellipse's corners stay transparent");
        let centre = decoded.get_pixel(decoded.width() / 2, decoded.height() / 2);
        assert_eq!(centre.0, [0, 255, 0, 255], "green in the middle");

        let line = dir.join("line.png");
        materialize_shape(
            &concat_scene::ShapeData {
                kind: concat_scene::ShapeKind::Line,
                width: 40.0,
                height: 20.0,
                fill: None,
                stroke: Some([0.0, 0.0, 1.0, 1.0]),
                stroke_width: 4.0,
            },
            &line,
        )
        .expect("the line paints");
        let decoded = image::open(&line).expect("reads back").into_rgba8();
        let middle = decoded.get_pixel(decoded.width() / 2, decoded.height() / 2);
        assert_eq!(middle.0, [0, 0, 255, 255], "the pen colour on the line");
        let above = decoded.get_pixel(decoded.width() / 2, 2);
        assert_eq!(above.0[3], 0, "the canvas above the line stays empty");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 展开接住了图形图层：shape 层的 PNG 在受写校验保护的写入点
    /// （MCP scene_add_layer / 编辑器保存）物化并回填 asset，展开路径
    /// 自己零写盘——这里模拟"已素材化"的层（asset 指向真实 PNG），
    /// 断言展开把它当普通图片接进时间线。
    #[test]
    fn a_shape_layer_with_a_materialized_asset_expands() {
        use concat_project::{Command, Editor};
        let dir =
            std::env::temp_dir().join(format!("concat-studio-shape-x-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        write_asset(&dir, "pill.png");

        let mut scene = Scene::new("sc1", "图形", 4000.0, 2250.0, 1920, 1080);
        scene.layers.push(concat_scene::SceneLayer {
            id: "pill".into(),
            name: "胶囊".into(),
            kind: concat_scene::LayerKind::Shape,
            asset: Some("pill.png".into()),
            shape: Some(concat_scene::ShapeData {
                kind: concat_scene::ShapeKind::Rect,
                width: 400.0,
                height: 200.0,
                fill: Some([1.0, 0.0, 0.0, 1.0]),
                stroke: None,
                stroke_width: 0.0,
            }),
            transform: concat_scene::SceneTransform {
                x: 2000.0,
                y: 1125.0,
                ..Default::default()
            },
            in_frame: 0.0,
            duration_frames: 0.0,
            opacity: 1.0,
            visible: true,
            z: 0,
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
        let project = editor.project();

        let expanded = expand_project_scenes(project, &dir, &mut HashMap::new());
        let image_clips: Vec<_> = expanded
            .active()
            .clips
            .iter()
            .filter(|clip| clip.kind == concat_project::model::ClipKind::Image)
            .collect();
        assert_eq!(image_clips.len(), 1, "the shape layer became an image clip");
        let clip = image_clips[0];
        assert!(
            clip.media_id.starts_with("scn"),
            "the shape's PNG rides a synthetic media item"
        );
        let media = expanded
            .media
            .iter()
            .find(|item| item.id == clip.media_id)
            .expect("the media item");
        assert!(
            std::path::Path::new(&media.path).exists(),
            "the PNG is on disk"
        );
        assert_eq!(
            expanded
                .media
                .iter()
                .find(|item| item.id == clip.media_id)
                .and_then(|item| item.width),
            Some(1)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// trim/speed 语义：`source_start` 偏移场景本地入点，`speed` 缩放
    /// 场景时间相对时间轴的流速。2× 速度的 3 秒片段在时间轴上占 1.5
    /// 秒，场景帧以 60fps 的速率流过。
    #[test]
    fn a_scene_clip_with_speed_maps_the_window_across_scene_frames() {
        use concat_project::{Command, Editor};

        let dir = std::env::temp_dir().join(format!("concat-studio-speed-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        write_asset(&dir, "bg.png");

        let mut scene = Scene::new("sc1", "速度", 4000.0, 2250.0, 1920, 1080);
        scene.layers.push(concat_scene::SceneLayer {
            id: "bg".into(),
            name: "背景".into(),
            kind: concat_scene::LayerKind::Background,
            asset: Some("bg.png".into()),
            shape: None,
            transform: concat_scene::SceneTransform {
                x: 2000.0,
                y: 1125.0,
                ..Default::default()
            },
            in_frame: 0.0,
            duration_frames: 0.0,
            opacity: 1.0,
            visible: true,
            z: 0,
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
        // 场景默认 90 帧 @30fps = 3 秒。裁到 1.5 秒时间轴（speed 1.0 时
        // 本地窗口也是 1.5 秒），然后 SetClipSpeed 到 2×：同一时间轴窗
        // 口现在流过 3 秒的场景帧。
        let clip_id = editor.project().active().clips[0].id.clone();
        editor
            .apply(Command::TrimClip {
                clip_id: clip_id.clone(),
                edge: concat_project::commands::TrimEdge::End,
                delta: -1.5,
                ripple: false,
            })
            .unwrap();
        editor
            .apply(Command::SetClipSpeed {
                clip_id: clip_id.clone(),
                speed: 2.0,
                ripple: false,
            })
            .unwrap();
        let project = editor.project();

        let expanded = expand_project_scenes(project, &dir, &mut HashMap::new());
        let timeline = expanded.active();
        let image_clips: Vec<_> = timeline
            .clips
            .iter()
            .filter(|clip| clip.kind == concat_project::model::ClipKind::Image)
            .collect();
        assert_eq!(image_clips.len(), 1, "单图层场景展开出一个图像片段");
        // SetClipSpeed 2× 后片段时长减半：45 帧 @30fps ÷ 2 = 0.75 秒。
        // 展开的图像片段时长与片段一致（不是场景的 3 秒）。
        let clip = timeline
            .clips
            .iter()
            .find(|clip| clip.scene_id.as_deref() == Some("sc1"))
            .expect("the scene clip");
        assert!(
            (clip.duration - 0.75).abs() < 0.1,
            "2× speed halves the window"
        );
        assert!(
            (image_clips[0].duration - clip.duration).abs() < 0.1,
            "expansion covers exactly the clip's window"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
