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

/// 场景中的一个对象（背景/角色/道具/图形/图片）
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SceneLayer {
    pub id: String,
    pub name: String,
    pub kind: LayerKind,
    /// 相对工程目录的素材路径；图形图层为 `None`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset: Option<String>,
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
    /// 可选目标图层（跟随/对焦预留）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
}

impl MovePreset {
    /// 以当前摄像机为起点求解结束位姿（相对运镜）
    pub fn target_pose(&self, current: &VirtualCamera) -> VirtualCamera {
        self.kind.compute_target_pose(current)
    }

    /// 在 0..1 进度上求值（相对运镜）
    pub fn pose_at(&self, current: &VirtualCamera, progress: f64) -> VirtualCamera {
        let target = self.target_pose(current);
        current.interpolate(&target, progress.clamp(0.0, 1.0), self.easing.to_ease())
    }

    /// 把预设烘焙成两关键帧的摄像机轨道
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

    /// 场景时长（帧）：取图层结束与摄像机最后一帧的最大值
    pub fn duration_frames(&self) -> f64 {
        let mut end: f64 = self.camera.keys.last().map(|k| k.frame).unwrap_or(0.0) + 1.0;
        for layer in &self.layers {
            let layer_end = if layer.duration_frames > 0.0 {
                layer.in_frame + layer.duration_frames
            } else {
                layer.in_frame + 1.0
            };
            end = end.max(layer_end);
        }
        end.max(1.0)
    }

    pub fn layer_by_id(&self, id: &str) -> Option<&SceneLayer> {
        self.layers.iter().find(|l| l.id == id)
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
            target: None,
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
}
