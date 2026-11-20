// SPDX-License-Identifier: AGPL-3.0-or-later
// 2D Mini Studio / Concat_camera：2D 场景、角色图层与摄像机轨道

//! 2D 场景领域模型：世界像素坐标的图层、角色表情差分、摄像机关键帧轨道与相对运镜。
//!
//! 该 crate 不依赖渲染器：它只负责“当前帧应该看到什么”，输出可直接映射到
//! `concat-core::timeline::Transform` 的图层计划（`LayerPlan`），由渲染/导出层消费。

use concat_camera::{CameraMoveType, VirtualCamera};
use concat_core::animate::Ease;
use concat_core::timeline::Transform;
use serde::{Deserialize, Serialize};

/// 可序列化的缓动曲线（与 `concat_core::animate::Ease` 等价）
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Easing {
    pub x1: f64,
    pub y1: f64,
    pub x2: f64,
    pub y2: f64,
}

impl Default for Easing {
    fn default() -> Self {
        Self::EASE_IN_OUT
    }
}

impl Easing {
    pub const LINEAR: Self = Self {
        x1: 0.0,
        y1: 0.0,
        x2: 1.0,
        y2: 1.0,
    };
    pub const EASE_IN: Self = Self {
        x1: 0.42,
        y1: 0.0,
        x2: 1.0,
        y2: 1.0,
    };
    pub const EASE_OUT: Self = Self {
        x1: 0.0,
        y1: 0.0,
        x2: 0.58,
        y2: 1.0,
    };
    pub const EASE_IN_OUT: Self = Self {
        x1: 0.42,
        y1: 0.0,
        x2: 0.58,
        y2: 1.0,
    };

    pub fn to_ease(self) -> Ease {
        Ease::new(self.x1, self.y1, self.x2, self.y2)
    }
}

/// 图层类型
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LayerKind {
    Background,
    Character,
    Prop,
    Shape,
    Image,
}

/// 场景图层的世界变换（位置为世界像素坐标，旋转为顺时针角度）
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct SceneTransform {
    pub x: f64,
    pub y: f64,
    pub scale: f64,
    pub rotation_deg: f64,
    pub stretch_x: f64,
    pub stretch_y: f64,
}

impl Default for SceneTransform {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            scale: 1.0,
            rotation_deg: 0.0,
            stretch_x: 1.0,
            stretch_y: 1.0,
        }
    }
}

/// 图形种类（rect / ellipse / line）
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShapeKind {
    /// 填充矩形
    Rect,
    /// 内切椭圆
    Ellipse,
    /// 水平线：`ShapeData::size` 的宽为线长，粗细取 `stroke_width`
    Line,
}

/// 图形图层的形状定义：素材化（渲染为 PNG）的唯一事实源。
///
/// 颜色是 rgba 的 0..=1；`fill` 与 `stroke` 都可缺省——都缺的图形没有
/// 东西可画，素材化时按空图处理。`size` 是形状自己的像素（世界像素与
/// 相机投影由图层 transform 与 [`Scene::plan_layers`] 承担）。
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ShapeData {
    #[serde(default)]
    pub kind: ShapeKind,
    /// 形状像素宽
    #[serde(default = "default_shape_width")]
    pub width: f64,
    /// 形状像素高；line 时粗细取 `stroke_width`
    #[serde(default = "default_shape_height")]
    pub height: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<[f32; 4]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stroke: Option<[f32; 4]>,
    /// 描边粗细（像素），向内描。camelCase 的 `strokeWidth` 也收——
    /// MCP 的 schema 与手写 JSON 两种拼法都指同一个字段。
    #[serde(default, alias = "strokeWidth")]
    pub stroke_width: f64,
}

fn default_shape_width() -> f64 {
    100.0
}

fn default_shape_height() -> f64 {
    100.0
}

impl Default for ShapeData {
    fn default() -> Self {
        Self {
            kind: ShapeKind::Rect,
            width: 100.0,
            height: 100.0,
            fill: Some([1.0, 1.0, 1.0, 1.0]),
            stroke: None,
            stroke_width: 0.0,
        }
    }
}

impl ShapeData {
    /// 内容指纹（素材化缓存的文件名成分）：只取决定像素的字段。文本
    /// 形式刻意直白（Debug 枚举名 + 定点小数），稳定跨进程——调用方拿
    /// 它做文件名，而不是自己再发明一份。
    pub fn cache_key(&self) -> String {
        fn rgba(c: Option<[f32; 4]>) -> String {
            match c {
                // 位模式级精度：.3 位小数会把 0.12375 与 0.12351 压成同
                // 一个键（u8 量化其实不同），先到先得的缓存就把后者钉死
                // 在前者的像素上。
                Some([r, g, b, a]) => format!(
                    "{:08x},{:08x},{:08x},{:08x}",
                    r.to_bits(),
                    g.to_bits(),
                    b.to_bits(),
                    a.to_bits()
                ),
                None => "none".into(),
            }
        }
        format!(
            "{:?}-{}x{}-f({})-s({})-w{}",
            self.kind,
            self.width,
            self.height,
            rgba(self.fill),
            rgba(self.stroke),
            self.stroke_width
        )
        // 颜色的指纹取 f32 的位模式级精度（rgba 里 .3 小数会碰撞），
        // 见 rgba()。
    }
    }
}

/// 场景中的一个对象（背景/角色/道具/图形/图片）
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SceneLayer {
    pub id: String,
    pub name: String,
    pub kind: LayerKind,
    /// 相对工程目录的素材路径；图形图层为 `None`（形状走 [`SceneLayer::shape`]
    /// 素材化）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset: Option<String>,
    /// 图形定义；仅 `kind == Shape` 携带。素材化把这一份像素画出来，
    /// JSON 里它仍是唯一事实源。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shape: Option<ShapeData>,
    #[serde(default)]
    pub transform: SceneTransform,
    /// 入点（帧）
    #[serde(default)]
    pub in_frame: f64,
    /// 时长（帧）；0 表示到场景结尾
    #[serde(default)]
    pub duration_frames: f64,
    #[serde(default = "one")]
    pub opacity: f64,
    #[serde(default = "yes")]
    pub visible: bool,
    #[serde(default)]
    pub z: i32,
    /// 角色表情差分 key（仅 Character 使用）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expression: Option<String>,
}

fn one() -> f64 {
    1.0
}

fn yes() -> bool {
    true
}

impl SceneLayer {
    pub fn is_visible_at(&self, frame: f64) -> bool {
        if !self.visible {
            return false;
        }
        let end = if self.duration_frames > 0.0 {
            self.in_frame + self.duration_frames
        } else {
            f64::INFINITY
        };
        frame >= self.in_frame && frame < end
    }
}

/// 摄像机关键帧
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CameraKey {
    /// 帧号（允许亚帧）
    pub frame: f64,
    pub pose: VirtualCamera,
    /// 从本关键帧到下一关键帧的缓动
    #[serde(default)]
    pub easing: Easing,
}

/// 摄像机关键帧轨道
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CameraTrack {
    #[serde(default)]
    pub keys: Vec<CameraKey>,
}

impl CameraTrack {
    pub fn from_key(frame: f64, pose: VirtualCamera) -> Self {
        Self {
            keys: vec![CameraKey {
                frame,
                pose,
                easing: Easing::default(),
            }],
        }
    }

    pub fn add_key(&mut self, frame: f64, pose: VirtualCamera, easing: Easing) {
        if let Some(existing) = self
            .keys
            .iter_mut()
            .find(|k| (k.frame - frame).abs() < 1e-9)
        {
            existing.pose = pose;
            existing.easing = easing;
        } else {
            self.keys.push(CameraKey {
                frame,
                pose,
                easing,
            });
        }
        self.keys.sort_by(|a, b| {
            a.frame
                .partial_cmp(&b.frame)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
    }

    /// 在指定帧求值（无关键帧时返回默认摄像机）
    pub fn pose_at(&self, frame: f64) -> VirtualCamera {
        if self.keys.is_empty() {
            return VirtualCamera::default();
        }
        if frame <= self.keys[0].frame {
            return self.keys[0].pose;
        }
        if frame >= self.keys[self.keys.len() - 1].frame {
            return self.keys[self.keys.len() - 1].pose;
        }
        for pair in self.keys.windows(2) {
            let a = &pair[0];
            let b = &pair[1];
            if frame >= a.frame && frame <= b.frame {
                let span = b.frame - a.frame;
                let t = if span <= 0.0 {
                    1.0
                } else {
                    (frame - a.frame) / span
                };
                return a.pose.interpolate(&b.pose, t, a.easing.to_ease());
            }
        }
        self.keys[self.keys.len() - 1].pose
    }
}

/// 可序列化的运镜预设（相对语义由 `CameraMoveType` 决定）
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MovePreset {
    pub name: String,
    pub kind: CameraMoveType,
    /// 持续时长（帧）
    pub duration_frames: f64,
    #[serde(default)]
    pub easing: Easing,
    /// 可选目标图层 id：`Follow` 与 `FocusTarget` 会解析该图层的世界坐标
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    /// 跟随/对焦时的构图偏移（世界像素）
    #[serde(default)]
    pub offset_x: f64,
    #[serde(default)]
    pub offset_y: f64,
    /// 跟随/对焦后的缩放；`None` 表示保持当前缩放
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zoom: Option<f64>,
}

impl Default for MovePreset {
    fn default() -> Self {
        Self {
            name: String::new(),
            kind: CameraMoveType::PushIn { factor: 1.0 },
            duration_frames: 30.0,
            easing: Easing::default(),
            target: None,
            offset_x: 0.0,
            offset_y: 0.0,
            zoom: None,
        }
    }
}

impl MovePreset {
    /// 以当前摄像机为起点求解结束位姿（相对运镜，不解析场景目标）
    pub fn target_pose(&self, current: &VirtualCamera) -> VirtualCamera {
        self.kind.compute_target_pose(current)
    }

    /// 在场景中求解结束位姿：`Follow` / `FocusTarget` 会解析 `target` 图层。
    ///
    /// 找不到目标图层时：`Follow` 保持当前机位（仅应用可选缩放）；
    /// `FocusTarget` 退回 `kind` 内的绝对坐标。
    pub fn target_pose_in(&self, current: &VirtualCamera, scene: &Scene) -> VirtualCamera {
        let target_layer = self.target.as_deref().and_then(|id| scene.layer_by_id(id));
        match (&self.kind, target_layer) {
            (CameraMoveType::Follow, Some(layer)) => {
                let mut target = *current;
                target.x = layer.transform.x + self.offset_x;
                target.y = layer.transform.y + self.offset_y;
                target.zoom = self.zoom.unwrap_or(current.zoom);
                target
            }
            (CameraMoveType::Follow, None) => {
                let mut target = *current;
                target.zoom = self.zoom.unwrap_or(current.zoom);
                target
            }
            (CameraMoveType::FocusTarget { target_zoom, .. }, Some(layer)) => {
                let mut target = *current;
                target.x = layer.transform.x + self.offset_x;
                target.y = layer.transform.y + self.offset_y;
                target.zoom = self.zoom.unwrap_or(*target_zoom);
                target
            }
            _ => self.target_pose(current),
        }
    }

    /// 在 0..1 进度上求值（相对运镜，不解析场景目标）
    pub fn pose_at(&self, current: &VirtualCamera, progress: f64) -> VirtualCamera {
        let target = self.target_pose(current);
        current.interpolate(&target, progress.clamp(0.0, 1.0), self.easing.to_ease())
    }

    /// 在场景中按 0..1 进度求值（跟随/对焦会解析目标图层）
    pub fn pose_at_in(
        &self,
        current: &VirtualCamera,
        scene: &Scene,
        progress: f64,
    ) -> VirtualCamera {
        let target = self.target_pose_in(current, scene);
        current.interpolate(&target, progress.clamp(0.0, 1.0), self.easing.to_ease())
    }

    /// 把预设烘焙成两关键帧的摄像机轨道（不解析场景目标）
    pub fn bake(&self, current: &VirtualCamera, start_frame: f64) -> Vec<CameraKey> {
        vec![
            CameraKey {
                frame: start_frame,
                pose: *current,
                easing: self.easing,
            },
            CameraKey {
                frame: start_frame + self.duration_frames.max(1.0),
                pose: self.target_pose(current),
                easing: self.easing,
            },
        ]
    }

    /// 把预设烘焙成两关键帧的摄像机轨道（跟随/对焦会解析目标图层）
    pub fn bake_in(
        &self,
        current: &VirtualCamera,
        scene: &Scene,
        start_frame: f64,
    ) -> Vec<CameraKey> {
        vec![
            CameraKey {
                frame: start_frame,
                pose: *current,
                easing: self.easing,
            },
            CameraKey {
                frame: start_frame + self.duration_frames.max(1.0),
                pose: self.target_pose_in(current, scene),
                easing: self.easing,
            },
        ]
    }
}

/// 组合运镜：按顺序执行多个预设，每一步都以上一步的结束机位为起点。
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct MoveSequence {
    #[serde(default)]
    pub steps: Vec<MovePreset>,
}

impl MoveSequence {
    /// 总时长（帧），每个步骤至少 1 帧
    pub fn total_frames(&self) -> f64 {
        self.steps
            .iter()
            .map(|step| step.duration_frames.max(1.0))
            .sum()
    }

    /// 在序列偏移 `offset_frames` 处求值；超出结尾时返回最终机位
    pub fn pose_at_in(
        &self,
        current: &VirtualCamera,
        scene: &Scene,
        offset_frames: f64,
    ) -> VirtualCamera {
        let mut pose = *current;
        let mut cursor = 0.0;
        for step in &self.steps {
            let duration = step.duration_frames.max(1.0);
            if offset_frames < cursor + duration {
                let progress = ((offset_frames - cursor) / duration).clamp(0.0, 1.0);
                return step.pose_at_in(&pose, scene, progress);
            }
            pose = step.target_pose_in(&pose, scene);
            cursor += duration;
        }
        pose
    }

    /// 链式求解最终机位
    pub fn target_pose_in(&self, current: &VirtualCamera, scene: &Scene) -> VirtualCamera {
        self.pose_at_in(current, scene, self.total_frames())
    }

    /// 烘焙为多关键帧轨道：每个步骤边界一个关键帧，缓动取该步骤的曲线
    pub fn bake_in(
        &self,
        current: &VirtualCamera,
        scene: &Scene,
        start_frame: f64,
    ) -> Vec<CameraKey> {
        let mut keys = Vec::new();
        let mut pose = *current;
        let mut frame = start_frame;
        for (index, step) in self.steps.iter().enumerate() {
            if index == 0 {
                keys.push(CameraKey {
                    frame,
                    pose,
                    easing: step.easing,
                });
            }
            frame += step.duration_frames.max(1.0);
            pose = step.target_pose_in(&pose, scene);
            // A boundary key's easing governs the segment that starts there:
            // it belongs to the next step, not the one that just ended.
            let easing = self
                .steps
                .get(index + 1)
                .map(|next| next.easing)
                .unwrap_or(step.easing);
            keys.push(CameraKey {
                frame,
                pose,
                easing,
            });
        }
        keys
    }
}

/// 单帧的图层计划（渲染层消费）
#[derive(Clone, Debug, PartialEq)]
pub struct LayerPlan {
    pub layer_id: String,
    pub transform: Transform,
    pub opacity: f64,
    pub z: i32,
}

/// 2D 场景
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Scene {
    pub id: String,
    pub name: String,
    pub world_width: f64,
    pub world_height: f64,
    /// 输出帧率分子（默认 30）
    #[serde(default = "default_rate_num")]
    pub rate_num: i64,
    /// 输出帧率分母（默认 1）
    #[serde(default = "default_rate_den")]
    pub rate_den: i64,
    /// 背景色 RGBA（0..1）
    #[serde(default = "default_background")]
    pub background: [f32; 4],
    /// 场景显式时长（帧）；None 时由图层与相机推断
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_frames: Option<f64>,
    #[serde(default)]
    pub layers: Vec<SceneLayer>,
    pub camera: CameraTrack,
}

fn default_rate_num() -> i64 {
    30
}

fn default_rate_den() -> i64 {
    1
}

fn default_background() -> [f32; 4] {
    [0.06, 0.08, 0.09, 1.0]
}

impl Scene {
    /// 创建一个以场景中心为初始机位的场景
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        world_width: f64,
        world_height: f64,
        viewport_width: u32,
        viewport_height: u32,
    ) -> Self {
        let mut pose = VirtualCamera::new(viewport_width, viewport_height);
        pose.x = world_width / 2.0;
        pose.y = world_height / 2.0;
        Self {
            id: id.into(),
            name: name.into(),
            world_width,
            world_height,
            rate_num: 30,
            rate_den: 1,
            background: default_background(),
            duration_frames: Some(90.0),
            layers: Vec::new(),
            camera: CameraTrack::from_key(0.0, pose),
        }
    }

    /// 输出帧率（帧/秒）
    pub fn fps(&self) -> f64 {
        if self.rate_den <= 0 || self.rate_num <= 0 {
            30.0
        } else {
            self.rate_num as f64 / self.rate_den as f64
        }
    }

    /// 场景时长（帧）：取显式时长、图层结束与摄像机最后一帧的最大值
    pub fn duration_frames(&self) -> f64 {
        let mut end: f64 = self.camera.keys.last().map(|k| k.frame).unwrap_or(0.0) + 1.0;
        for layer in &self.layers {
            let layer_end = if layer.duration_frames > 0.0 {
                layer.in_frame + layer.duration_frames
            } else {
                self.duration_frames
                    .unwrap_or(layer.in_frame + 1.0)
                    .max(layer.in_frame + 1.0)
            };
            end = end.max(layer_end);
        }
        if let Some(explicit) = self.duration_frames {
            end = end.max(explicit);
        }
        end.max(1.0)
    }

    pub fn layer_by_id(&self, id: &str) -> Option<&SceneLayer> {
        self.layers.iter().find(|l| l.id == id)
    }

    /// True when any number in the scene is NaN or infinite. What a command
    /// carrier checks before it applies, mirroring `concat-project`'s
    /// `has_non_finite`: JSON cannot spell one, but a caller in Rust can,
    /// and one stored would poison every pose and transform it touched.
    pub fn has_non_finite(&self) -> bool {
        fn bad(values: impl IntoIterator<Item = f64>) -> bool {
            values.into_iter().any(|value| !value.is_finite())
        }
        let camera_bad = self.camera.keys.iter().any(|key| {
            bad([key.frame])
                || bad([key.pose.x, key.pose.y, key.pose.zoom, key.pose.rotation])
                || bad([key.easing.x1, key.easing.y1, key.easing.x2, key.easing.y2])
        });
        let layers_bad = self.layers.iter().any(|layer| {
            bad([
                layer.transform.x,
                layer.transform.y,
                layer.transform.scale,
                layer.transform.rotation_deg,
                layer.transform.stretch_x,
                layer.transform.stretch_y,
            ]) || bad([layer.in_frame, layer.duration_frames, layer.opacity])
                // 图形定义同样进命令载体：一个 NaN 的形状尺寸/描边会把
                // 素材化画成一张毒图。
                || layer.shape.iter().any(|shape| bad([shape.width, shape.height, shape.stroke_width]))
        });
        camera_bad
            || layers_bad
            || bad([self.world_width, self.world_height])
            || self.duration_frames.is_some_and(|value| bad([value]))
    }

    /// 在 `start_frame` 处把相对运镜烘焙进摄像机轨道。
    ///
    /// 起点取轨道在该帧的当前机位；因此同一预设应用到不同机位会得到各自的结果。
    pub fn add_move(&mut self, preset: &MovePreset, start_frame: f64) {
        let current = self.camera.pose_at(start_frame);
        for key in preset.bake_in(&current, self, start_frame) {
            self.camera.add_key(key.frame, key.pose, key.easing);
        }
    }

    /// 在 `start_frame` 处把组合运镜烘焙进摄像机轨道（步骤间链式相对）。
    pub fn add_sequence(&mut self, sequence: &MoveSequence, start_frame: f64) {
        let current = self.camera.pose_at(start_frame);
        for key in sequence.bake_in(&current, self, start_frame) {
            self.camera.add_key(key.frame, key.pose, key.easing);
        }
    }

    /// 当前帧的可见图层计划：世界像素 → 摄像机画框（归一化 Transform）
    pub fn plan_layers(&self, frame: f64, frame_width: u32, frame_height: u32) -> Vec<LayerPlan> {
        let pose = self.camera.pose_at(frame);
        let fw = frame_width as f64;
        let fh = frame_height as f64;
        let mut plans: Vec<LayerPlan> = self
            .layers
            .iter()
            .filter(|layer| layer.is_visible_at(frame))
            .filter_map(|layer| {
                let (sx, sy) = pose.world_to_viewport(layer.transform.x, layer.transform.y);
                let mut transform = Transform::IDENTITY;
                transform.offset_x = (sx - fw / 2.0) / fw;
                transform.offset_y = (sy - fh / 2.0) / fh;
                transform.scale = layer.transform.scale * pose.zoom;
                transform.rotation = layer.transform.rotation_deg - pose.rotation.to_degrees();
                transform.stretch_x = layer.transform.stretch_x;
                transform.stretch_y = layer.transform.stretch_y;
                Some(LayerPlan {
                    layer_id: layer.id.clone(),
                    transform,
                    opacity: layer.opacity,
                    z: layer.z,
                })
            })
            .collect();
        plans.sort_by_key(|plan| plan.z);
        plans
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cam(x: f64, y: f64, zoom: f64) -> VirtualCamera {
        let mut c = VirtualCamera::new(1920, 1080);
        c.x = x;
        c.y = y;
        c.zoom = zoom;
        c
    }

    #[test]
    fn scene_roundtrips_through_json() {
        let mut scene = Scene::new("s1", "教室", 8000.0, 4500.0, 1920, 1080);
        scene.layers.push(SceneLayer {
            id: "l1".into(),
            name: "小樱".into(),
            kind: LayerKind::Character,
            asset: Some("assets/sakura.png".into()),
            shape: None,
            transform: SceneTransform {
                x: 3600.0,
                y: 2250.0,
                scale: 0.7,
                ..Default::default()
            },
            in_frame: 0.0,
            duration_frames: 300.0,
            opacity: 1.0,
            visible: true,
            z: 2,
            expression: Some("smile".into()),
        });

        let json = serde_json::to_string(&scene).expect("serialises");
        let parsed: Scene = serde_json::from_str(&json).expect("deserialises");
        assert_eq!(parsed, scene);
    }

    #[test]
    fn camera_track_interpolates_between_keys() {
        let mut track = CameraTrack::default();
        let mut a = cam(0.0, 0.0, 1.0);
        a.x = 100.0;
        let mut b = cam(0.0, 0.0, 2.0);
        b.x = 500.0;
        track.add_key(0.0, a, Easing::LINEAR);
        track.add_key(100.0, b, Easing::LINEAR);

        let mid = track.pose_at(50.0);
        assert!((mid.x - 300.0).abs() < 1e-6);
        assert!((mid.zoom - 1.5).abs() < 1e-6);
        assert!((track.pose_at(-10.0).x - 100.0).abs() < 1e-6);
        assert!((track.pose_at(200.0).zoom - 2.0).abs() < 1e-6);
    }

    #[test]
    fn relative_move_preset_starts_from_current_pose() {
        let current = cam(4000.0, 2250.0, 1.5);
        let preset = MovePreset {
            name: "推近".into(),
            kind: CameraMoveType::PushIn { factor: 1.2 },
            duration_frames: 90.0,
            easing: Easing::LINEAR,
            ..Default::default()
        };

        let target = preset.target_pose(&current);
        assert!((target.zoom - 1.8).abs() < 1e-9);
        assert!((target.x - 4000.0).abs() < 1e-9);

        let mid = preset.pose_at(&current, 0.5);
        assert!((mid.zoom - 1.65).abs() < 1e-6);

        let keys = preset.bake(&current, 30.0);
        assert_eq!(keys.len(), 2);
        assert!((keys[1].frame - 120.0).abs() < 1e-9);
        assert!((keys[1].pose.zoom - 1.8).abs() < 1e-9);
    }

    #[test]
    fn plan_layers_projects_world_to_camera_frame() {
        let mut scene = Scene::new("s", "场景", 8000.0, 4500.0, 1920, 1080);
        scene.layers.push(SceneLayer {
            id: "center".into(),
            name: "中心".into(),
            kind: LayerKind::Prop,
            asset: None,
            shape: None,
            transform: SceneTransform {
                x: 4000.0,
                y: 2250.0,
                scale: 1.0,
                ..Default::default()
            },
            in_frame: 0.0,
            duration_frames: 300.0,
            opacity: 1.0,
            visible: true,
            z: 0,
            expression: None,
        });

        let plans = scene.plan_layers(0.0, 1920, 1080);
        assert_eq!(plans.len(), 1);
        assert!(plans[0].transform.offset_x.abs() < 1e-9);
        assert!(plans[0].transform.offset_y.abs() < 1e-9);
        assert!((plans[0].transform.scale - 1.0).abs() < 1e-9);

        // 摄像机右移 192px、zoom 2：图层相对画框应左移并放大
        scene
            .camera
            .add_key(30.0, cam(4192.0, 2250.0, 2.0), Easing::LINEAR);
        let plans = scene.plan_layers(30.0, 1920, 1080);
        assert!((plans[0].transform.scale - 2.0).abs() < 1e-9);
        assert!((plans[0].transform.offset_x - (-0.2)).abs() < 1e-9);
    }

    #[test]
    fn plan_layers_filters_by_time_and_sorts_by_z() {
        let mut scene = Scene::new("s", "场景", 8000.0, 4500.0, 1920, 1080);
        let mut layer = SceneLayer {
            id: "a".into(),
            name: "A".into(),
            kind: LayerKind::Image,
            asset: Some("assets/a.png".into()),
            shape: None,
            transform: SceneTransform::default(),
            in_frame: 50.0,
            duration_frames: 20.0,
            opacity: 1.0,
            visible: true,
            z: 5,
            expression: None,
        };
        scene.layers.push(layer.clone());
        layer.id = "b".into();
        layer.in_frame = 0.0;
        layer.duration_frames = 0.0;
        layer.z = 1;
        scene.layers.push(layer);

        assert_eq!(scene.plan_layers(10.0, 1920, 1080).len(), 1);
        let plans = scene.plan_layers(60.0, 1920, 1080);
        assert_eq!(plans.len(), 2);
        assert_eq!(plans[0].layer_id, "b");
        assert_eq!(plans[1].layer_id, "a");
        assert!(
            scene
                .plan_layers(75.0, 1920, 1080)
                .iter()
                .all(|p| p.layer_id != "a")
        );
    }

    fn layer(id: &str, x: f64, y: f64, z: i32) -> SceneLayer {
        SceneLayer {
            id: id.into(),
            name: id.into(),
            kind: LayerKind::Character,
            asset: Some(format!("assets/{id}.png")),
            shape: None,
            transform: SceneTransform {
                x,
                y,
                ..Default::default()
            },
            in_frame: 0.0,
            duration_frames: 300.0,
            opacity: 1.0,
            visible: true,
            z,
            expression: None,
        }
    }

    #[test]
    fn follow_preset_centers_on_target_layer() {
        let mut scene = Scene::new("s", "场景", 8000.0, 4500.0, 1920, 1080);
        scene.layers.push(layer("hero", 3000.0, 2000.0, 1));

        let preset = MovePreset {
            name: "跟随主角".into(),
            kind: CameraMoveType::Follow,
            duration_frames: 60.0,
            easing: Easing::LINEAR,
            target: Some("hero".into()),
            offset_x: 120.0,
            offset_y: -80.0,
            zoom: Some(1.4),
        };

        let start = cam(4000.0, 2250.0, 1.0);
        let target = preset.target_pose_in(&start, &scene);
        assert!((target.x - 3120.0).abs() < 1e-9);
        assert!((target.y - 1920.0).abs() < 1e-9);
        assert!((target.zoom - 1.4).abs() < 1e-9);

        let mid = preset.pose_at_in(&start, &scene, 0.5);
        assert!((mid.x - 3560.0).abs() < 1e-6);
        assert!((mid.zoom - 1.2).abs() < 1e-6);
    }

    #[test]
    fn follow_without_target_keeps_current_pose() {
        let scene = Scene::new("s", "场景", 8000.0, 4500.0, 1920, 1080);
        let start = cam(1234.0, 567.0, 1.25);
        let preset = MovePreset {
            name: "跟随".into(),
            kind: CameraMoveType::Follow,
            target: None,
            ..Default::default()
        };
        let target = preset.target_pose_in(&start, &scene);
        assert_eq!(target, start);
    }

    #[test]
    fn move_sequence_chains_steps_relative_to_each_other() {
        let scene = Scene::new("s", "场景", 8000.0, 4500.0, 1920, 1080);
        let start = cam(4000.0, 2250.0, 1.0);
        let sequence = MoveSequence {
            steps: vec![
                MovePreset {
                    name: "右移".into(),
                    kind: CameraMoveType::Pan {
                        delta_x: 100.0,
                        delta_y: -40.0,
                    },
                    duration_frames: 30.0,
                    easing: Easing::LINEAR,
                    ..Default::default()
                },
                MovePreset {
                    name: "推近".into(),
                    kind: CameraMoveType::PushIn { factor: 2.0 },
                    duration_frames: 30.0,
                    easing: Easing::LINEAR,
                    ..Default::default()
                },
            ],
        };

        assert!((sequence.total_frames() - 60.0).abs() < 1e-9);
        let mid_first = sequence.pose_at_in(&start, &scene, 15.0);
        assert!((mid_first.x - 4050.0).abs() < 1e-6);
        assert!((mid_first.zoom - 1.0).abs() < 1e-6);

        let end_first = sequence.pose_at_in(&start, &scene, 30.0);
        assert!((end_first.x - 4100.0).abs() < 1e-6);
        assert!((end_first.y - 2210.0).abs() < 1e-6);

        let mid_second = sequence.pose_at_in(&start, &scene, 45.0);
        assert!((mid_second.x - 4100.0).abs() < 1e-6);
        assert!((mid_second.zoom - 1.5).abs() < 1e-6);

        let end = sequence.target_pose_in(&start, &scene);
        assert!((end.x - 4100.0).abs() < 1e-6);
        assert!((end.zoom - 2.0).abs() < 1e-6);
    }

    #[test]
    fn same_relative_preset_works_from_any_start_pose() {
        let scene = Scene::new("s", "场景", 8000.0, 4500.0, 1920, 1080);
        let preset = MovePreset {
            name: "缓慢推近".into(),
            kind: CameraMoveType::PushIn { factor: 1.25 },
            duration_frames: 90.0,
            easing: Easing::EASE_IN_OUT,
            ..Default::default()
        };

        let a = cam(1000.0, 800.0, 1.0);
        let b = cam(6000.0, 3000.0, 1.6);
        let target_a = preset.target_pose_in(&a, &scene);
        let target_b = preset.target_pose_in(&b, &scene);

        assert!((target_a.zoom - 1.25).abs() < 1e-9);
        assert!((target_b.zoom - 2.0).abs() < 1e-9);
        assert!((target_a.x - a.x).abs() < 1e-9);
        assert!((target_b.y - b.y).abs() < 1e-9);
    }

    #[test]
    fn scene_bakes_sequence_into_camera_track() {
        let mut scene = Scene::new("s", "场景", 8000.0, 4500.0, 1920, 1080);
        scene.layers.push(layer("hero", 3000.0, 2000.0, 1));
        scene
            .camera
            .add_key(30.0, cam(5000.0, 2600.0, 1.5), Easing::LINEAR);

        let sequence = MoveSequence {
            steps: vec![
                MovePreset {
                    name: "跟随".into(),
                    kind: CameraMoveType::Follow,
                    duration_frames: 30.0,
                    easing: Easing::LINEAR,
                    target: Some("hero".into()),
                    zoom: Some(1.2),
                    ..Default::default()
                },
                MovePreset {
                    name: "推近".into(),
                    kind: CameraMoveType::PushIn { factor: 2.0 },
                    duration_frames: 30.0,
                    easing: Easing::LINEAR,
                    ..Default::default()
                },
            ],
        };
        scene.add_sequence(&sequence, 30.0);

        let follow_end = scene.camera.pose_at(60.0);
        assert!((follow_end.x - 3000.0).abs() < 1e-6);
        assert!((follow_end.y - 2000.0).abs() < 1e-6);
        assert!((follow_end.zoom - 1.2).abs() < 1e-6);

        let end = scene.camera.pose_at(90.0);
        assert!((end.zoom - 2.4).abs() < 1e-6);
    }

    #[test]
    fn baked_sequence_keeps_each_steps_own_easing() {
        let scene = Scene::new("s", "场景", 8000.0, 4500.0, 1920, 1080);
        let start = cam(4000.0, 2250.0, 1.0);
        let sequence = MoveSequence {
            steps: vec![
                MovePreset {
                    name: "右移".into(),
                    kind: CameraMoveType::Pan {
                        delta_x: 100.0,
                        delta_y: 0.0,
                    },
                    duration_frames: 30.0,
                    easing: Easing::LINEAR,
                    ..Default::default()
                },
                MovePreset {
                    name: "缓入推近".into(),
                    kind: CameraMoveType::PushIn { factor: 2.0 },
                    duration_frames: 30.0,
                    easing: Easing::EASE_IN,
                    ..Default::default()
                },
            ],
        };

        let evaluated = sequence.pose_at_in(&start, &scene, 45.0);
        let track = CameraTrack {
            keys: sequence.bake_in(&start, &scene, 0.0),
        };
        let baked = track.pose_at(45.0);
        assert!(
            (evaluated.zoom - baked.zoom).abs() < 1e-9,
            "the baked track must use step 1's easing after its boundary: {} vs {}",
            evaluated.zoom,
            baked.zoom
        );
        assert!((evaluated.x - baked.x).abs() < 1e-9);
    }

    #[test]
    fn move_sequence_roundtrips_through_json() {
        let sequence = MoveSequence {
            steps: vec![MovePreset {
                name: "跟随".into(),
                kind: CameraMoveType::Follow,
                duration_frames: 45.0,
                easing: Easing::EASE_OUT,
                target: Some("hero".into()),
                offset_x: 10.0,
                offset_y: 20.0,
                zoom: Some(1.3),
            }],
        };
        let json = serde_json::to_string(&sequence).expect("serialises");
        let parsed: MoveSequence = serde_json::from_str(&json).expect("deserialises");
        assert_eq!(parsed, sequence);
    }

    /// 图形定义随图层往返；旧 JSON（无 shape 字段）读到 None；同一形状
    /// 的 cache_key 稳定（缓存文件名靠它跨进程不变），不同形状不同名。
    #[test]
    fn shape_data_roundtrips_and_keys_by_content() {
        let shape = ShapeData {
            kind: ShapeKind::Ellipse,
            width: 320.0,
            height: 180.0,
            fill: Some([1.0, 0.4, 0.2, 1.0]),
            stroke: Some([0.0, 0.0, 0.0, 1.0]),
            stroke_width: 3.0,
        };
        let layer = SceneLayer {
            id: "pill".into(),
            name: "胶囊".into(),
            kind: LayerKind::Shape,
            asset: None,
            shape: Some(shape),
            transform: Default::default(),
            in_frame: 0.0,
            duration_frames: 0.0,
            opacity: 1.0,
            visible: true,
            z: 0,
            expression: None,
        };
        let json = serde_json::to_string(&layer).expect("serialises");
        let restored: SceneLayer = serde_json::from_str(&json).expect("deserialises");
        assert_eq!(restored.shape, Some(shape));
        let key_of = |layer: &SceneLayer| {
            layer
                .shape
                .as_ref()
                .map(|shape| shape.cache_key())
                .unwrap_or_default()
        };
        assert_eq!(key_of(&restored), key_of(&layer), "the key survives a round-trip");

        // 旧文档：没有 shape 字段的图层读作 None。
        let old: SceneLayer = serde_json::from_str(
            r#"{"id":"p","name":"图","kind":"shape","asset":null}"#,
        )
        .expect("an old shape layer reads");
        assert_eq!(old.shape, None);
        assert_ne!(key_of(&old), key_of(&layer), "the fingerprint follows the content");
    }
}
