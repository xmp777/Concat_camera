# Phase 4：Slint 场景视口 + 编辑器 Scene 面板

> 日期：2026-11-19
> 状态：已完成（审查修复 + 最终回归通过）
> 对应提交：`23de8eba`（审查修复提交见 §7）
> 审查：两个只读子代理（核心 Rust / 编辑器 UI）

## 1. 本阶段目标

- 在编辑器内提供 2D 场景预览与虚拟摄像机拖动。
- 使用 Concat 原生时间轴 playhead，而不是自带一套时间轴。
- 复用 Concat 的预览合成器，不自建渲染器。

不在本阶段范围内：Scene 时间轴片段、Inspector 相机参数、图形图层、Undo、手机端。

## 2. 交付内容

- `concat-scene-view`：独立 Slint 场景视口 Spike（保留，作为最小复现工具）。
- `concat-studio`：
  - `preview_frame` / `preview_frame_sized`；
  - **`ScenePreviewer`**：持久 `ReaderPool` + 素材尺寸缓存 + 单帧静态图层路径；
  - `render_scene_with_cancel`：导出可取消，并转发进度。
- 编辑器：
  - `PaneKind.scene`，可从 Add panel / 面板切换菜单添加；
  - `panes/scene.rs`：打开/保存、playhead 跟随、拖动相机、后台渲染；
  - `SceneView` global；`on_scene!` 只发布场景状态，不重建 lane/clip；
  - 拖动时 480×270（按场景宽高比适配），松手回 960×540；
  - 时间轴有足够内容时隐藏回退滑杆；场景比时间轴长时才显示。
- 工程保护：
  - 拒绝把**当前打开工程的 `concat.json`**当作场景打开或保存，避免丢掉未保存剪辑；
  - Save 在无场景时禁用。

## 3. 子代理审查结论与修复

### P0（已修复）

| 问题 | 修复 |
|---|---|
| Scene 面板可打开/保存当前工程 manifest，绕过 dirty/undo/save lane，可能丢未保存剪辑 | `same_manifest` 守卫：打开与保存都拒绝；Save 无场景时禁用 |
| MCP scene 工具绕过 `CONCAT_MCP_ROOTS` 写入沙箱 | `Backend::writes_under` + 所有 scene 写路径校验；新增 e2e 越界拒绝测试 |

### P1（已修复）

| 问题 | 修复 |
|---|---|
| 过期帧结果回写 `frame`，拖动落在错误帧 | 请求 id + scene generation；结果只更新当前帧；`frame` 表示用户目标帧 |
| 工作线程 panic 后 `busy` 卡死 | `catch_unwind` 包住预览；失败也会清理 in-flight 并继续 pending |
| 每次拖动/结果都全量 `publish` | `on_scene!` + `spawn_publishing` + `publish_scene`，只更新 SceneView |
| 每帧新建 ReaderPool、重复 probe、烘焙整条相机轨道 | `ScenePreviewer` 复用池/尺寸缓存；单帧只求一次 `plan_layers` |
| 拖动速度/方向错误（固定 960×540、忽略旋转/letterbox） | 按相机 viewport 与 `viewport_to_world` 计算；Slint 只在实际图片矩形内取拖动；预览按场景宽高比适配 |
| 播放时场景冻结 | 播放 tick 调用 `follow_playhead(..., coarse=true)` |
| 时间轴有任意 clip 就隐藏滑杆 | 仅当时间轴长度 ≥ 场景长度时隐藏；场景更长时保留滑杆 |
| 组合运镜烘焙的缓动错位 | 边界关键帧使用**下一步**的缓动；新增不同缓动对比测试 |
| 隐藏图层/缺失素材导致整场渲染失败 | 导出与预览跳过 `visible: false`；新增测试 |
| `durationFrames: 0` 的新场景只有 1 帧 | `Scene.duration_frames: Option<f64>` 显式时长；`scene_create` 默认 90 帧，MCP 新增 `durationFrames` |
| `scene_render` 不可取消、无进度 | `render_scene_with_cancel` + MCP `context.cancelled/progress` |
| whisper 构建脚本 metal/vulkan 分支不可编译；fallback 重复打补丁 | 恢复 `let mut bindings`；补丁改为幂等 |

### P2（记录，未阻塞）

- Scene 仍不是时间轴片段；playhead 映射是全局时间，不是片段本地时间。
- 拖动写关键帧没有 Undo、没有 dirty 标记；会保留原关键帧缓动，但不进入工程命令栈。
- 场景路径/帧号不随工作区保存；重启后 Scene 面板为空。
- 裸 scene 保存会转成 Concat 工程文档，没有 Save As。
- 部分状态文案英文、`concat-studio` 错误中文；Windows 个人使用可接受。
- `scene_render` 仍同步执行，已可取消/报进度，但仍占用 MCP 单 worker。

## 4. 验证证据

| 项 | 命令 / 操作 | 结果 |
|---|---|---|
| 场景 | `cargo test -p concat-scene` | 12 passed |
| 预览/导出 | `cargo test -p concat-studio` | 4 passed（往返、预览、缓存一致性、隐藏图层） |
| MCP | `cargo test -p concat-mcp` | 10 lib + 5 e2e（含沙箱拒绝） |
| 编辑器 | `cargo test -p concat --lib` | 243 passed |
| 构建 | `cargo build -p concat`（MSVC） | 通过 |
| 实机 | 编辑器 Add panel → Scene → Open 示例场景 | 通过 |
| 时间轴 | 点击时间轴 → Scene 显示 frame 21/90，回退滑杆隐藏 | 通过 |
| 截图 | `docs/assets/editor_scene_timeline.png`、`editor_scene_reviewed.png` | 已生成 |

> 最终回归已通过：`concat-scene` 12、`concat-studio` 4、`concat-mcp` 10 lib + 5 e2e；编辑器 243。

## 5. 已知限制

- Windows-only；macOS/metal、手机端不承诺。
- 当前 Scene 预览读取全局 playhead；真正的 Scene 片段（`scene_id` + 起止）尚未实现。
- Inspector 尚未暴露 Camera 参数（X/Y/Zoom/Rotation/关键帧）。
- MCP scene 工具若被指向**另一个正在编辑器里打开的工程**的 manifest，仍可能产生双写；编辑器面板自身已拒绝。
- 跟随目标只解算图层静态世界坐标。

## 6. 下一阶段

1. **Scene 时间轴片段**：工程模型/命令/时间轴 UI；场景帧由片段本地时间决定。
2. **Inspector Camera 区**：X/Y/Zoom/Rotation/关键帧，接 Undo 命令栈。
3. 编辑器导出统一 `plan_frame_with_camera`。
4. 图形图层素材化；跟随逐帧解算；parallax。

下一阶段结束按 SOP §12 再启动子代理审查。

## 7. Git

- 阶段实现：`34144962`、`38ace250`、`23de8eba`
- 流程文档：`9260a7e8`
- 审查修复：`d643012d`
- 推送：`origin/main` 已同步（`git status -sb` 无 ahead）