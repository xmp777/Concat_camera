// SPDX-License-Identifier: AGPL-3.0-or-later
// 2D Mini Studio / Concat_camera：场景 + 摄像机 → Concat 导出管线

//! 把 `concat-scene` 的 2D 场景转换为 `concat-export` 的导出请求，并调用
//! Concat 自己的 GPU 合成/编码管线输出 MP4。这里不实现任何解码、合成或编码。

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use concat_export::{ExportRequest, Reporter, render_on};
use concat_media::probe;
use concat_scene::{LayerPlan, Scene, SceneLayer};
use serde_json::{Value, json};

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
    let (width, height) = viewport_of(scene);
    let fps = scene.fps();
    let duration_frames = scene.duration_frames().ceil().max(1.0);

    let mut ordered: Vec<&SceneLayer> = scene.layers.iter().collect();
    ordered.sort_by_key(|layer| layer.z);

    let mut clips: Vec<Value> = Vec::new();
    for (track, layer) in ordered.iter().enumerate() {
        let Some(asset) = layer.asset.as_ref() else {
            continue; // 图形图层需要素材化后才能进入导出
        };
        let path = resolve(scene_dir, asset);
        let info = probe(&path).map_err(|error| format!("{}: {error}", path.display()))?;
        let video = info
            .require_video()
            .map_err(|error| format!("{}: {error}", path.display()))?;

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
            "mediaWidth": video.width,
            "mediaHeight": video.height,
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

    let canvas = [
        (scene.background[0].clamp(0.0, 1.0) * 255.0).round() as u8,
        (scene.background[1].clamp(0.0, 1.0) * 255.0).round() as u8,
        (scene.background[2].clamp(0.0, 1.0) * 255.0).round() as u8,
        (scene.background[3].clamp(0.0, 1.0) * 255.0).round() as u8,
    ];

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

    let cancel = AtomicBool::new(false);
    let reporter = Reporter {
        progress,
        cancel: &cancel,
    };
    render_on(&request, None, reporter).map_err(|e| format!("导出失败：{e}"))?;
    Ok(())
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
