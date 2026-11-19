// SPDX-License-Identifier: AGPL-3.0-or-later
// 2D Mini Studio / Concat_camera 虚拟摄像机与 2D 场景视口投影引擎

//! 虚拟摄像机（Virtual Camera）与视口投影系统
//!
//! 在 2D 小剧场和动画创作中，负责将世界空间（World Space）坐标映射到
//! 目标输出画框（Viewport / Camera Space），并提供推拉摇移、相对运镜与关键帧缓动计算。

use concat_core::animate::Ease;
use serde::{Deserialize, Serialize};

/// 虚拟摄像机位姿与视口参数
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct VirtualCamera {
    /// 摄像机镜头中心在世界空间中的 X 坐标
    pub x: f64,
    /// 摄像机镜头中心在世界空间中的 Y 坐标
    pub y: f64,
    /// 变焦系数（1.0 为无变焦，>1.0 为推近放大，<1.0 为拉远缩小）
    pub zoom: f64,
    /// 镜头倾角（单位：弧度，顺时针为正）
    pub rotation: f64,
    /// 输出目标画框宽度（像素）
    pub viewport_width: u32,
    /// 输出目标画框高度（像素）
    pub viewport_height: u32,
}

impl Default for VirtualCamera {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            zoom: 1.0,
            rotation: 0.0,
            viewport_width: 1920,
            viewport_height: 1080,
        }
    }
}

impl VirtualCamera {
    /// 创建一个新的摄像机实例
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            viewport_width: width,
            viewport_height: height,
            ..Default::default()
        }
    }

    /// 将世界空间中的二维点映射到摄像机画框的屏幕视口坐标
    ///
    /// 变换流程：
    /// 1. 平移至摄像机局部中心：P' = P - CameraPos
    /// 2. 变焦缩放：P'' = P' * Zoom
    /// 3. 反向倾斜旋转：P''' = Rotate(-Rotation) * P''
    /// 4. 平移至屏幕物理中心：P_screen = P''' + ScreenCenter
    pub fn world_to_viewport(&self, world_x: f64, world_y: f64) -> (f64, f64) {
        let center_x = self.viewport_width as f64 / 2.0;
        let center_y = self.viewport_height as f64 / 2.0;

        // 1. 相对摄像机中心偏移
        let dx = world_x - self.x;
        let dy = world_y - self.y;

        // 2. 变焦缩放
        let sx = dx * self.zoom;
        let sy = dy * self.zoom;

        // 3. 旋转（摄像机旋转 θ 时，视野中物体反向旋转 -θ）
        let cos_r = (-self.rotation).cos();
        let sin_r = (-self.rotation).sin();
        let rx = sx * cos_r - sy * sin_r;
        let ry = sx * sin_r + sy * cos_r;

        // 4. 映射到屏幕中心
        (rx + center_x, ry + center_y)
    }

    /// 将屏幕画框中的鼠标或点击像素坐标，反算回世界空间坐标
    pub fn viewport_to_world(&self, view_x: f64, view_y: f64) -> (f64, f64) {
        let center_x = self.viewport_width as f64 / 2.0;
        let center_y = self.viewport_height as f64 / 2.0;

        // 1. 减去屏幕中心偏移
        let rx = view_x - center_x;
        let ry = view_y - center_y;

        // 2. 逆向旋转（恢复 +θ）
        let cos_r = self.rotation.cos();
        let sin_r = self.rotation.sin();
        let sx = rx * cos_r - ry * sin_r;
        let sy = rx * sin_r + ry * cos_r;

        // 3. 逆向变焦
        let safe_zoom = if self.zoom.abs() < 1e-6 { 1e-6 } else { self.zoom };
        let dx = sx / safe_zoom;
        let dy = sy / safe_zoom;

        // 4. 恢复世界中心偏移
        (dx + self.x, dy + self.y)
    }

    /// 在两个摄像机状态之间按进度与缓动曲线进行高精度插值
    pub fn interpolate(&self, target: &Self, t: f64, ease: Ease) -> Self {
        let progress = ease.apply(t.clamp(0.0, 1.0));
        Self {
            x: self.x + (target.x - self.x) * progress,
            y: self.y + (target.y - self.y) * progress,
            zoom: self.zoom + (target.zoom - self.zoom) * progress,
            rotation: self.rotation + (target.rotation - self.rotation) * progress,
            viewport_width: target.viewport_width,
            viewport_height: target.viewport_height,
        }
    }

    /// 生成可直接传递给 GPU 合成着色器的 4x4 仿射变换矩阵（列主序 Column-Major）
    pub fn view_projection_matrix(&self) -> [f32; 16] {
        let w = self.viewport_width as f64;
        let h = self.viewport_height as f64;

        // 归一化设备坐标 (NDC) 缩放与平移系数
        let scale_x = (2.0 * self.zoom / w) as f32;
        let scale_y = (-2.0 * self.zoom / h) as f32; // Y 轴向下翻转适配 UI 坐标系

        let cos_r = (-self.rotation).cos() as f32;
        let sin_r = (-self.rotation).sin() as f32;

        let trans_x = (-self.x) as f32;
        let trans_y = (-self.y) as f32;

        let m00 = scale_x * cos_r;
        let m01 = scale_x * -sin_r;
        let m03 = (trans_x * cos_r - trans_y * sin_r) * scale_x;

        let m10 = scale_y * sin_r;
        let m11 = scale_y * cos_r;
        let m13 = (trans_x * sin_r + trans_y * cos_r) * scale_y;

        [
            m00,  m10, 0.0, 0.0,
            m01,  m11, 0.0, 0.0,
            0.0,  0.0, 1.0, 0.0,
            m03,  m13, 0.0, 1.0,
        ]
    }
}

/// 运镜预设定义
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum CameraMoveType {
    /// 推镜头：按比例放大特写
    PushIn { factor: f64 },
    /// 拉镜头：缩小展现全局
    PullOut { factor: f64 },
    /// 水平/垂直平移运镜（相对当前坐标）
    Pan { delta_x: f64, delta_y: f64 },
    /// 角色特写对焦：平滑将摄像机中心锁定到目标角色锚点
    FocusTarget { target_x: f64, target_y: f64, target_zoom: f64 },
}

impl CameraMoveType {
    /// 基于当前摄像机状态，生成运镜结束时的目标位姿
    pub fn compute_target_pose(&self, current: &VirtualCamera) -> VirtualCamera {
        let mut target = *current;
        match self {
            Self::PushIn { factor } => {
                target.zoom *= factor;
            }
            Self::PullOut { factor } => {
                target.zoom /= if factor.abs() < 1e-6 { 1.0 } else { *factor };
            }
            Self::Pan { delta_x, delta_y } => {
                target.x += delta_x;
                target.y += delta_y;
            }
            Self::FocusTarget { target_x, target_y, target_zoom } => {
                target.x = *target_x;
                target.y = *target_y;
                target.zoom = *target_zoom;
            }
        }
        target
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_world_viewport_bidirectional_mapping() {
        let mut camera = VirtualCamera::new(1920, 1080);
        camera.x = 500.0;
        camera.y = 300.0;
        camera.zoom = 1.5;
        camera.rotation = 0.25; // 弧度

        let original_world_x = 750.0;
        let original_world_y = 420.0;

        // 投影到视口
        let (vx, vy) = camera.world_to_viewport(original_world_x, original_world_y);

        // 从视口反投影回世界坐标
        let (reconstructed_wx, reconstructed_wy) = camera.viewport_to_world(vx, vy);

        assert!((original_world_x - reconstructed_wx).abs() < 1e-6);
        assert!((original_world_y - reconstructed_wy).abs() < 1e-6);
    }

    #[test]
    fn test_camera_center_projects_to_screen_center() {
        let camera = VirtualCamera {
            x: 1234.5,
            y: 678.9,
            zoom: 2.0,
            rotation: 1.0,
            viewport_width: 1920,
            viewport_height: 1080,
        };

        // 摄像机中心点本身必须精确投影到画框中心 (960, 540)
        let (vx, vy) = camera.world_to_viewport(camera.x, camera.y);
        assert!((vx - 960.0).abs() < 1e-6);
        assert!((vy - 540.0).abs() < 1e-6);
    }

    #[test]
    fn test_camera_zoom_in_scaling() {
        let mut camera = VirtualCamera::new(1920, 1080);
        camera.x = 0.0;
        camera.y = 0.0;
        camera.zoom = 1.0;

        let (vx1, _) = camera.world_to_viewport(100.0, 0.0);
        camera.zoom = 2.0;
        let (vx2, _) = camera.world_to_viewport(100.0, 0.0);

        // 变焦放大 2 倍后，相对中心距离应翻倍
        assert_eq!(vx1 - 960.0, 100.0);
        assert_eq!(vx2 - 960.0, 200.0);
    }

    #[test]
    fn test_push_in_move_preset() {
        let camera = VirtualCamera::new(1920, 1080);
        let push_move = CameraMoveType::PushIn { factor: 1.5 };
        let target = push_move.compute_target_pose(&camera);

        assert_eq!(target.zoom, 1.5);
        assert_eq!(target.x, camera.x);
        assert_eq!(target.y, camera.y);

        // 验证贝塞尔插值中间帧
        let mid = camera.interpolate(&target, 0.5, Ease::LINEAR);
        assert!((mid.zoom - 1.25).abs() < 1e-6);
    }
}
