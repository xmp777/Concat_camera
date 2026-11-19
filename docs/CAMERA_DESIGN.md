# 虚拟摄像机与 2D 场景设计（Concat_camera）

> 状态：**P1 + Phase 2/3 能力闭环**（2026-11-19）
> 适用范围：`jub0t/Concat` 的 fork `xmp777/Concat_camera`（AGPL-3.0-or-later）
> 目标：在 Concat 的剪辑/合成底座上，增加 **2D 场景 + 虚拟摄像机 + 运镜预设**，把它升维为 2D 小剧场创作工作站；**不重写剪辑器底座**。

---

## 1. 当前已落地（代码事实）

### 1.1 `concat-camera` crate

位置：`src/crates/concat-camera/`

| 能力 | API | 状态 |
|---|---|---|
| 摄像机位姿 | `VirtualCamera { x, y, zoom, rotation, viewport_width, viewport_height }` | ✅ |
| 世界 → 视口 | `world_to_viewport(world_x, world_y) -> (f64, f64)` | ✅ |
| 视口 → 世界 | `viewport_to_world(view_x, view_y) -> (f64, f64)` | ✅ |
| 图层变换投影 | `project_transform(&Transform, frame_w, frame_h) -> Transform` | ✅ |
| 两机位插值 | `interpolate(&target, t, Ease)` | ✅ |
| GPU 矩阵 | `view_projection_matrix() -> [f32; 16]`（列主序，NDC，Y 翻转） | ✅ |
| 运镜预设 | `CameraMoveType::{PushIn, PullOut, Pan, FocusTarget, Follow}` + `compute_target_pose(current)` | ✅ |
| 组合运镜 | `concat_scene::MoveSequence`（步骤链式相对，每步以上一步结束机位为起点） | ✅ |
| 单元测试 | 6 项（双向映射、中心、变焦、推近、投影恒等/平移缩放） | ✅ |

### 1.2 `concat-render` 集成

- `FramePlan` 新增 `camera: Option<VirtualCamera>`；
- 新增 `plan_frame_with_camera(timeline, time, Option<VirtualCamera>)`：
  对每个可见 clip 的 `clip.transform_at(time)` 调用 `camera.project_transform(...)`；
- `plan_frame(...)` 保持原行为（`camera = None`），**不改变现有剪辑路径**；
- 新增测试 `plans_layers_with_virtual_camera_projection`（1920×1080、相机 x=192px、zoom=1.5，验证 scale 与 offset）。

### 1.3 验证结果（本机，GNU 工具链）

| 命令 | 结果 |
|---|---|
| `cargo test -p concat-camera` | 6 passed |
| `cargo test -p concat-render` | **130 passed**, 1 ignored（含相机 plan 测试） |
| `cargo build -p concat-cli` | 成功（`concat-cli.exe`） |
| `concat-cli probe <phase2_video.mp4>` | `h264 1920x1080 @30fps, 1.000s` |

### 1.4 P1 能力闭环（已落地）

| 项 | 实现 | 验证 |
|---|---|---|
| `concat-scene` crate | Scene / SceneLayer（世界像素）/ CameraTrack / MovePreset / `plan_layers` | 5 tests |
| 相对运镜 | `MovePreset::target_pose` + `pose_at`（以当前机位为起点，复用 `Ease`） | 单测 |
| 无头渲染器 `concat-studio` | Scene → `ExportClip`（相机逐帧采样，中性 base + 绝对关键帧）→ `concat_export::render_on` | 示例 MP4 |
| MCP 工具 `scene_render` | `scenePath`/`output`/`crf`/`preset`，headless 可用 | e2e 测试 + 实跑 |
| 示例工程 | `examples/camera_demo/`（背景+2角色+道具，推近 90 帧） | `out.mp4` 3s 1920×1080 h264 |

### 1.4a Phase 2/3 能力（已落地）

| 项 | 实现 | 验证 |
|---|---|---|
| 跟随目标 | `MovePreset { target: Some(layer_id), offset_x/y, zoom }` + `CameraMoveType::Follow`；`target_pose_in(&scene)` 解析图层世界坐标 | 单测（含无目标退化） |
| 对焦图层 | `FocusTarget` 同时支持 `target` 图层解析（坐标由图层覆盖） | 单测 |
| 组合运镜 | `MoveSequence { steps }`；`pose_at_in` 逐段求值，`bake_in` 生成多个关键帧；`Scene::add_move` / `add_sequence` 直接烘焙进相机轨道 | 单测（两步链式、中间帧、JSON 往返） |
| 相对语义验收 | 同一推近预设从两个不同机位出发，各自得到 `current.zoom * factor`，不依赖硬编码世界坐标 | 单测 `same_relative_preset_works_from_any_start_pose` |
| 工程持久化 | `concat_studio::save_scene` 写入 **Concat 工程文档**（`concat`/`version`/`timelines` + 顶层 `scene`）；`load_scene` 兼容裸 Scene 与工程文档 | `concat-studio` 单测 |
| MCP 场景 CRUD | `scene_create`、`scene_add_layer`（upsert）、`scene_camera_move`（组合/跟随）、`scene_get`；`scene_render` 兼容工程文档 | 5 个 e2e 全过（含从零建双人场景并导出） |

**关键实现细节**：Concat 的 keyed 属性是**绝对值**，clip 的 base 必须取中性值（scale 1 / offset 0 / rotation 0），
否则会与关键帧叠加导致位移与缩放翻倍。`concat-studio` 已按此处理，并用示例的像素位置验证。

截图：`docs/assets/camera_demo_frame0.png`（起始）与 `camera_demo_frame89.png`（推近结束）。

### 1.5 重要事实：编辑器内导出尚未接入相机

`plan_frame_with_camera(..., Some(camera))` 在编辑器/CLI 的导出路径中**仍未被调用**：
`concat-export` 与 `concat-cli` 依旧走 `plan_frame(...)`（camera=None）；commit 只在 `FramePlan` 拷贝处透传 `camera` 字段。

P1 的无头路径已经绕过这一点：`concat-studio` 把相机运动逐帧烘焙进 `ExportClip` 的关键帧，再交给同一套
`render_on` 合成/编码。编辑器内的实时相机预览与"打开工程导出"仍需后续把相机求值接进 `plan_frame` 调用点。

### 1.6 Phase 4 Slint 视口 Spike（已落地，独立窗口）

新增 `concat-scene-view`（`src/crates/concat-scene-view`）：

- 打开裸 `scene.json` 或 Concat 工程文档（顶层 `scene`）；
- 用 **`concat_studio::preview_frame_sized`** 渲染当前帧（内部调用 `concat_export::preview_frame`，
  复用 Concat 的预览合成器；960×540，拖动时更轻）；
- Slint `Image` 显示 RGBA（`SharedPixelBuffer<Rgba8Pixel>`，只在 UI 线程创建）；
- 渲染在后台线程，最新请求优先；UI 线程不跑解码/合成；
- 在图像上拖动 = 平移相机（屏幕增量 → 世界像素，按 zoom 反算），并在当前帧写一个绝对关键帧；
- `Save` 调 `concat_studio::save_scene` 写回工程文档。

验证：`cargo run -p concat-scene-view -- examples/camera_demo/scene.json`，启动截图
`docs/assets/scene_view_spike.png`；`concat-studio` 单测 `preview_frame_uses_the_same_scene_projection_as_export`
断言帧 0 与 89 的 RGBA 不同且尺寸为 1920×1080。

**编辑器内 Pane（已接线）**：新增 `PaneKind.scene`，可从任意面板的切换菜单或窗口右上角 "Add panel" 添加；
Rust 侧 `panes/scene.rs` 负责打开/保存/拖动，`studio/publish.rs` 把图像与状态发布到 `SceneView` global。
运行截图（菜单）：`docs/assets/editor_scene_menu.png`。编辑器的 `cargo check -p concat`、i18n 与 dock 单测在 MSVC 下通过。

后续完善：把 Scene 加入默认工作区预设（当前不改默认布局，避免覆盖用户已保存布局）、面板内直接选择场景文件后的路径记忆、
以及把场景预览与时间轴联动（当前按场景自身帧号）。

---

## 2. 坐标系统与数学约定（必须统一）

### 2.1 空间

| 空间 | 单位 | 说明 |
|---|---|---|
| World | 像素 | 2D 场景世界（例如 8000×4500），Y 向下 |
| Camera/Viewport | 像素 | 输出画框（例如 1920×1080），Y 向下 |
| NDC | [-1,1] | GPU 顶点着色器输入，Y 已翻转 |

### 2.2 `concat-core::Transform` 的语义（来自源码）

```rust
Transform {
    scale: f64,      // 相对"适配画框"的倍数（1 = 填满，保持宽高比）
    offset_x: f64,   // 图片中心相对画框中心的偏移，按画框宽度的比例
    offset_y: f64,   // 按画框高度的比例
    rotation: f64,   // 顺时针旋转，单位：度
    stretch_x/y: f64 // 在 scale 之上对宽/高的额外倍率
}
```

### 2.3 `VirtualCamera` 的语义

- `x, y`：摄像机中心在世界空间的**像素**坐标；
- `zoom`：>1 推近，<1 拉远；
- `rotation`：**弧度**（顺时针为正）——注意与 `Transform.rotation` 的"度"不同，投影时用 `to_degrees()` 转换；
- `viewport_*`：输出画框像素。

### 2.4 投影公式

```
P_screen = ViewportCenter + R(-rotation) * ((P_world - CameraPos) * zoom)
```

反投影：

```
P_world = CameraPos + R(+rotation) * ((P_screen - ViewportCenter) / zoom)
```

`project_transform` 把上述作用到图层变换上：

```
1) world_px = (offset_x * frame_w, offset_y * frame_h)
2) d = world_px - camera_pos
3) r = R(-rotation) * d * zoom
4) new_offset = r / (frame_w, frame_h)
5) new_scale = scale * zoom
6) new_rotation = rotation_deg - camera_rotation_deg
```

### 2.5 已知的模型不一致（P1 必须解决）

1. **世界尺寸缺失**：`project_transform` 把 `Transform.offset * frame_size` 当作世界像素。这对"世界 = 画框大小"成立；对 8000×4500 的大场景不成立。
   → P1 需要在场景/图层数据中显式携带 `world_width/world_height`，或者统一改用世界像素坐标（推荐：场景模型用世界像素，plan 时归一化）。
2. **旋转单位混用**：相机弧度、Transform 度。已在投影时转换，但预设/序列化层要固定单位并写测试。
3. **Y 轴**：UI/世界 Y 向下；GPU NDC Y 向上。`view_projection_matrix` 已翻转；CPU 参考路径需用同一约定验证。
4. **视差（parallax）**：stash 中有一个未提交实验（`stash@{0}`），其公式 `world - camera * p` 不是标准的深度视差（应为 `(world - camera) * p`），且无测试。**未合入**；P1 若做背景视差，需要重新推导并补金标准测试。

---

## 3. 运镜预设：已实现与剩余缺口

已实现：

```rust
enum CameraMoveType {
    PushIn { factor },                               // 相对：zoom *= factor
    PullOut { factor },                              // 相对：zoom /= factor
    Pan { delta_x, delta_y },                        // 相对：位置增量（世界像素）
    FocusTarget { target_x, target_y, target_zoom }, // 绝对点；有 target 图层时由图层覆盖 x/y
    Follow,                                          // 跟随 target 图层（坐标由场景解析）
}
fn compute_target_pose(&self, current: &VirtualCamera) -> VirtualCamera;
fn VirtualCamera::interpolate(&target, t, Ease) -> VirtualCamera;

pub struct MovePreset {
    pub name: String,
    pub kind: CameraMoveType,
    pub duration_frames: f64,
    pub easing: Easing,
    pub target: Option<String>,   // 图层 id
    pub offset_x: f64,            // 跟随/对焦构图偏移（世界像素）
    pub offset_y: f64,
    pub zoom: Option<f64>,        // None = 保持当前缩放
}
impl MovePreset {
    fn target_pose_in(&self, current, scene) -> VirtualCamera;
    fn bake_in(&self, current, scene, start_frame) -> Vec<CameraKey>;
}

pub struct MoveSequence { pub steps: Vec<MovePreset> }
impl MoveSequence {
    fn pose_at_in(&self, current, scene, offset_frames) -> VirtualCamera;
    fn bake_in(&self, current, scene, start_frame) -> Vec<CameraKey>;
}
```

缓动直接复用 `concat-core::animate::Ease`（三次贝塞尔，含 CSS cubic-bezier 求解）。
`Scene::add_move` / `Scene::add_sequence` 从**触发帧的当前机位**开始烘焙，因此同一预设应用到不同机位会得到各自的结果。

剩余缺口（Phase 4+）：

- 跟随只解析图层**静态**世界坐标；图层自身关键帧动画完成后，跟随需要逐帧重解目标点。
- 没有**边界约束**（世界范围、zoom 上下限）。
- 预设未作为可复用资产持久化（分类/收藏/导入导出）；当前以场景内的 `moves` 调用参数形式存在。
- 没有可视化曲线编辑器与运镜组合 UI。

---

## 4. 与 Concat 底座的集成策略（P1）

### 4.1 新增 `concat-scene`（领域核心）

建议结构（serde + 单测）：

```rust
pub struct Scene {
    pub id: SceneId,
    pub name: String,
    pub world_width: f64,   // 世界像素
    pub world_height: f64,
    pub background: [f32; 4],
    pub camera: CameraTrack,       // 关键帧轨道
    pub layers: Vec<SceneLayer>,
}

pub struct SceneLayer {
    pub id: LayerId,
    pub name: String,
    pub kind: LayerKind,           // Background | Character | Prop | Shape | Image
    pub asset: Option<PathBuf>,    // 相对工程路径
    pub transform: Transform,      // 世界变换（像素 → plan 时归一化）
    pub in_frame: u32,
    pub duration_frames: u32,
    pub opacity: f64,
    pub visible: bool,
    pub z: i32,
    pub expression: Option<String>,// 角色表情差分 key
}

pub struct CameraTrack { keys: Vec<(Rational, VirtualCamera)> }
impl CameraTrack { pub fn pose_at(&self, time: Rational) -> VirtualCamera; }

pub struct MovePreset {
    pub name: String,
    pub kind: CameraMoveType,
    pub duration_frames: f64,
    pub easing: Easing,
    pub target: Option<String>,
    pub offset_x: f64,
    pub offset_y: f64,
    pub zoom: Option<f64>,
}

pub struct MoveSequence { pub steps: Vec<MovePreset> }
```

### 4.2 渲染接入（尽量少改上游）

推荐：**场景作为"额外图层源"注入**，而不是侵入 `Clip` 结构：

1. 在 `concat-scene` 中实现 `scene_plan(scene, time, project_dir) -> Vec<PlannedLayer>`（或中间结构），负责：
   - 图层可见性（in/out）；
   - 世界像素 → 画框归一化；
   - 摄像机投影（调用 `concat-camera`）。
2. 在应用侧的薄封装里：
   ```rust
   let mut plan = plan_frame_with_camera(timeline, time, camera_pose);
   plan.layers.extend(scene_layers); // 场景层按 z 合并
   ```
3. 导出/预览的**所有** `plan_frame` 调用点（当前 8 处）统一走该封装；
   用集成测试对比"时间轴预览 / 导出"两路径的同一帧（像素一致或变换一致）。
4. 项目格式：在 Concat project JSON 增加 `scenes` 与 `activeScene`（扩展字段，使用 `#[serde(default)]` 保证旧工程可读）；场景片段通过 `scene_id` 引用场景。

> 不推荐直接把摄像机乘法写进每个 clip 的 Transform 后序列化（不可逆、不可编辑）；应在 plan 阶段求值。

### 4.3 Agent / MCP（已实现）

`concat-mcp` 的工具均为同步、无需打开工程：

| 工具 | 参数 | 说明 |
|---|---|---|
| `scene_create` | scenePath, name, worldWidth/Height, width/height, fps, background, overwrite | 创建 Concat 工程文档 + `scene` 扩展 |
| `scene_add_layer` | scenePath, id, kind, asset?, x/y, scale, rotationDeg, z, inFrame/durationFrames, opacity, visible, expression | 添加或更新图层（未给字段保留原值） |
| `scene_camera_move` | scenePath, atFrame, moves[] | 组合/跟随/推拉摇移，按步骤链式烘焙 |
| `scene_get` | scenePath | 读取场景摘要（图层、相机关键帧、时长） |
| `scene_render` | scenePath, output, crf?, preset? | 复用 Concat 合成/编码导出 MP4 |

验收脚本（无 GUI，已自动化）：`scene_create` → 3 图层（背景 + 2 角色）→ `scene_camera_move`（跟随两角色）→ `scene_render` → ffprobe 校验 640×360 / ~4s。
对应 e2e：`an_agent_builds_and_renders_a_two_character_scene_headless`。

---

## 5. P1 验收标准（能力优先）

1. `concat-scene` 单测：序列化往返、图层 in/out、世界→画框归一化、z 排序。✅（11 tests）
2. `concat-camera`：相对推近/平移/跟随目标的金标准测试（给定相机状态与预设，断言结束位姿与中间帧）。✅
3. 导出接线：`concat-cli render` 使用项目中的摄像机；同一项目导出两帧，帧间画面位移符合预期（可用简单色块图层 + 像素采样断言）。⬜（headless 已通；编辑器/CLI 待接）
4. MCP 闭环：Agent 通过 MCP 完成"建场景 → 加图层 → 相对推近 → 导出 MP4"；ffprobe 输出分辨率/帧数正确。✅
5. 文档：本文件更新 + 示例工程（`examples/camera_demo/`）。✅

---

## 6. 工具链与平台

- **正式基线（2026-11-19 起）**：Rust 1.93.1 `x86_64-pc-windows-msvc` + VS Build Tools 2022（`vcvars64.bat`）。
  - 构建/测试前先 `call vcvars64.bat`，否则找不到 `link.exe`。
  - `src/` 已设置 `rustup override` 为 MSVC。
  - 已验证：`concat-scene` / `concat-camera` / `concat-mcp`（含 5 个 e2e）、`concat` 编辑器（`cargo check` 与完整 `cargo build`）在 MSVC 下通过。
- **备用**：`x86_64-pc-windows-gnu`（复用 MSYS2 GCC；P0 阶段使用，仍可 `cargo +...-gnu` 临时构建）。
- FFmpeg：本地 `deps/ffdl/ffmpeg-n8.1.3...win64-lgpl-shared-8.1`（`FFMPEG_DIR`；含 MSVC `.lib` 与 DLL）。
- ONNX Runtime 1.28.0：`onnxruntime-win-x64-1.28.0`（`ORT_LIB_LOCATION`）。
  - `onnxruntime.lib` 2KB 是官方 DLL 导入库（与 nupkg 内文件哈希一致），不是 MinGW 误置；编辑器 MSVC 链接已通过。
- Windows 自带 `C:\WINDOWS\SYSTEM32\onnxruntime.dll` 优先级高于 PATH：运行时把我们的 DLL 放 exe 同目录。
- 环境脚本：`setup-env.ps1`（FFMPEG_DIR / ORT / LIBCLANG / PATH）；MSVC 另需 vcvars64。

---

## 7. 已知风险

| 风险 | 影响 | 对策 |
|---|---|---|
| 世界尺寸与 Transform 归一化不一致 | 大场景投影错误 | P1 引入 world_size，金标准像素测试 |
| 相机旋转单位混淆 | 画面方向错误 | 统一在边界转换，测试锁定 |
| 导出调用点遗漏 | 预览/导出不一致 | 统一封装 + 审计 8 个 plan 调用点 |
| 上游改动频繁 | 合并冲突 | 新逻辑放 `concat-scene`，上游只保留 `FramePlan.camera` 钩子 |
| GNU/MSVC 差异 | CI/依赖链接问题 | P0 后切 MSVC 基线 |
| parallax 公式未定 | 背景视差错误 | 保持 stash，重新推导 + 测试后再合入 |
| AGPL 合规 | 分发义务 | 保留 SPDX 头、THIRD_PARTY_NOTICES、源码可得 |

---

## 8. 变更记录

- 2026-11-19：P0 收尾。验证 camera/render 测试；stash parallax 实验；本文件首版；push origin/main。
- 2026-11-19（P1）：新增 `concat-scene`（5 tests）、`concat-studio` 无头渲染器、MCP `scene_render` 工具；
  示例 `examples/camera_demo/` 导出 3s/1920×1080/h264 并通过 MCP e2e 测试。相机 keyed 属性的中性 base 问题已修复。
- 2026-11-19（Phase 2/3）：`CameraMoveType::Follow`、`MovePreset` 目标解析/构图偏移/可选缩放、`MoveSequence`
  链式组合；`Scene::add_move`/`add_sequence`；`concat-scene` 增至 11 tests。`concat-studio` 支持裸场景与
  Concat 工程文档（顶层 `scene`）双向读写，新增 `concat-studio save`。MCP 新增 `scene_create`/`scene_add_layer`/
  `scene_camera_move`/`scene_get`，并新增 e2e `an_agent_builds_and_renders_a_two_character_scene_headless`；
  `concat-mcp` 共 10 lib + 5 e2e 通过。
- 2026-11-19（Phase 4 Spike）：新增 `concat-scene-view`（Slint 独立窗口，显示场景帧、拖动相机、保存）；
  `concat-studio` 新增 `preview_frame`/`preview_frame_sized`（复用 Concat 预览合成器）与预览单测；
  截图 `docs/assets/scene_view_spike.png`。编辑器内可停靠 Pane 的接线仍待做。