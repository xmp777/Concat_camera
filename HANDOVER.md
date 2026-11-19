# Concat_camera 交接单（P1 状态）

> 最后更新：2026-11-19
> 仓库：`G:\Projects\Concat_camera`（fork of `jub0t/Concat`，AGPL-3.0-or-later）
> 远程：`origin` = `xmp777/Concat_camera`，`upstream` = `jub0t/Concat`
> 当前：**P0/P1 完成；Phase 2（跟随 + 组合运镜）与 Phase 3（工程持久化 + MCP 场景 CRUD）已闭环；MSVC 为正式基线**
> **接手请先读 `docs/SOP.md`（标准作业程序）与 `docs/CAMERA_DESIGN.md`。**

---

## 0. 快速上手

```powershell
# 0) 环境（FFMPEG_DIR / ORT / PATH）
. G:\Projects\Concat_camera\setup-env.ps1

# 1) MSVC 构建/测试必须在 vcvars64 下（Rust 目录覆盖已指向 MSVC）
cmd /c "call `"C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat`" >nul && cargo test -p concat-scene"
cmd /c "call `"C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat`" >nul && cargo test -p concat-mcp"

# 2) 直接渲染示例场景
cargo run -p concat-studio -- render examples/camera_demo/scene.json examples/camera_demo/out.mp4

# 3) 生成/查看示例（已在仓库提交帧截图）
#    docs/assets/camera_demo_frame0.png / camera_demo_frame89.png
```

工作目录：`G:\Projects\Concat_camera\src`（Cargo workspace 在 `src/`，不是仓库根）。

---

## 1. 已完成

### P0（已推送，commit `977a6946` 起）
- `concat-camera`：世界↔视口投影、图层 Transform 投影、两机位插值、运镜预设雏形（6 tests）。
- `concat-render`：`FramePlan.camera` + `plan_frame_with_camera`（130 tests 全过）。
- `concat-cli`：构建通过，能 probe `phase2_video.mp4`（h264 1920×1080 30fps）。
- `docs/CAMERA_DESIGN.md`：坐标约定、数学、限制、P1 方案。
- 未提交的 parallax 实验已 stash：`stash@{0}`（公式需重推 + 测试，勿直接合入）。

### P1（commit `322b4a1a`）
- **`concat-scene` crate**（`src/crates/concat-scene`）：
  - `Scene { world_*, rate_*, background, layers, camera }`
  - `SceneLayer { kind, asset, SceneTransform(世界像素), in_frame/duration_frames, opacity, z, expression }`
  - `CameraTrack`（关键帧 + 可序列化 Easing）；`MovePreset`（相对推近/拉远/平移/对焦 + `pose_at`/`bake`）
  - `Scene::plan_layers(frame, w, h)`：可见性 + z 排序 + 世界→画框 `Transform`
  - 5 tests：JSON 往返、相机插值、相对预设、投影、时间范围/排序
- **`concat-studio`（无头渲染器）**：
  - Scene → 每图层 `ExportClip`（图片资产；相机运动逐帧采样为绝对关键帧）
  - 调用 Concat 自己的 `concat_export::render_on`（不重写解码/合成/编码）
  - `concat-studio render scene.json out.mp4 [--crf] [--preset]`
- **MCP 工具 `scene_render`**（`concat-mcp`）：
  - 参数 `scenePath` / `output` / `crf?` / `preset?`
  - headless 可用；e2e 测试 `an_agent_renders_a_camera_scene_headless` 通过
- **示例** `examples/camera_demo/`：背景 + 2 角色 + 道具，90 帧推近（zoom 1.0→1.6）
- **MSVC 正式基线**：VS Build Tools 已安装；`rustup override` 指向 `1.93.1-x86_64-pc-windows-msvc`；
  `concat-scene`/`concat-camera`/`concat-mcp` 的测试与构建在 MSVC 下通过。

### Phase 2/3（本次，2026-11-19）
- **跟随与组合运镜**：
  - `CameraMoveType::Follow`；`MovePreset` 新增 `offset_x/offset_y/zoom`，`target` 图层由 `target_pose_in(scene)` 解析。
  - `FocusTarget` 也支持 `target` 图层（坐标以图层为准）。
  - `MoveSequence { steps }`：每步以上一步结束机位为起点；`pose_at_in` 求中间帧、`bake_in` 生成多关键帧。
  - `Scene::add_move` / `Scene::add_sequence`：从触发帧的当前机位烘焙进轨道。
  - `concat-scene` 共 11 tests（含“同一相对预设从不同机位出发”验收）。
- **工程持久化**：
  - `concat_studio::save_scene` 写 **Concat 工程文档**（`concat`/`version`/`timelines` + 顶层 `scene`）；
  - `load_scene` 同时接受裸 Scene JSON 与工程文档；`concat-studio save <scene.json> <project.json>`。
- **MCP 场景 CRUD**：
  - `scene_create`（工程文档 + 场景、输出尺寸/帧率/背景）；
  - `scene_add_layer`（upsert；未给字段保留原值）；
  - `scene_camera_move`（`moves[]`：push_in/pull_out/pan/follow/focus_target，链式烘焙）；
  - `scene_get`（图层/相机摘要）；
  - `scene_render` 改为兼容工程文档。
  - 新 e2e：`an_agent_builds_and_renders_a_two_character_scene_headless`（从零建双人场景 → 跟随运镜 → 640×360 MP4）。
- **MSVC 下验证**：`concat-scene` 11、`concat-camera` 6、`concat-studio` 1、`concat-mcp` 10 lib + 5 e2e。

---

## 2. 验证记录（本机，2026-11-19）

| 项 | 结果 |
|---|---|
| `cargo test -p concat-camera` | 6 passed |
| `cargo test -p concat-scene` | 11 passed |
| `cargo test -p concat-studio` | 1 passed（裸场景/工程文档往返） |
| `cargo test -p concat-render` | 130 passed, 1 ignored |
| `cargo test -p concat-mcp`（MSVC） | 10 lib + 5 e2e passed（含从零建场景并渲染） |
| `concat-cli probe examples/camera_demo/out.mp4` | 3.000s / h264 / 1920×1080 / 30fps |
| `concat-studio save` + render 工程文档 | 3.000s / h264 / 1920×1080 / 30fps |
| 示例截图 | `docs/assets/camera_demo_frame0.png`、`frame89.png` |

### 关键坑（已修复/记录）
1. **keyed 属性是绝对值**：`ExportClip` 的 base（scale/offset/rotation）对已有关键帧的属性必须取中性值，否则叠加导致位移/缩放翻倍（道具曾因此消失）。
2. **相机单位**：`VirtualCamera.rotation` 弧度，`Transform.rotation` 度；投影时转换。
3. **世界尺寸**：`concat-camera::project_transform` 假设世界=画框像素；`concat-scene` 采用世界像素 + 显式 `world_width/height`，plan 时归一化。
4. **Windows 自带 `C:\WINDOWS\SYSTEM32\onnxruntime.dll`** 会被优先加载；调试/运行 MCP 时需把我们的 `onnxruntime.dll` 放在 exe 同目录（已复制到 `target/debug` 与 `deps`）。
5. 本机 ONNX `onnxruntime.lib` 仅 2KB（MinGW 用）；MSVC 构建目前能过（未链接 ORT 符号），若未来链接失败，从 `onnxruntime-1.28.0.nupkg` 恢复官方 MSVC `.lib`。
6. MCP 的 `scene_render` 是同步调用（渲染期间阻塞该工具调用）；大场景应后续改为任务/进度通知。
7. **场景工程格式**：`save_scene` 写 Concat 工程文档（顶层 `scene`）；`load_scene` 同时接受裸 Scene。读场景一律走 `load_scene`，不要直接 `serde_json::from_str::<Scene>`。
8. **MCP 工具清单预算**：`schema::tests::the_tool_list_fits_an_agents_context` 上限 190KB；新增工具有意精简描述，避免把上下文撑爆。

---

## 3. 代码地图

```
src/crates/
├─ concat-camera/    # 投影/插值/运镜预设（P0）
├─ concat-scene/     # 2D 场景/图层/相机轨道/跟随/组合运镜（P1+）
├─ concat-studio/    # Scene -> ExportClip -> render_on；裸场景/工程文档读写
├─ concat-mcp/       # MCP：scene_create/add_layer/camera_move/get/render
├─ concat-render/    # FramePlan.camera 钩子
├─ concat-export/    # 合成/编码（复用）
└─ concat/           # Slint 编辑器（P2 视口接入）
examples/camera_demo/ # 场景 JSON + 测试素材 + 说明
docs/CAMERA_DESIGN.md # 设计/坐标/限制/方案
```

数据流（P1+）：
`scene.json / project.json(scene) → concat_scene::Scene → plan_layers(camera at frame) → ExportClip(animation keys) → concat_export::render_on → MP4`

---

## 4. 已知限制 / 下一步

**限制**
- 图形图层（shape）尚未进入导出（需要素材化或 Concat 的 title/图层路径）。
- 场景已能写入 Concat 工程文档（顶层 `scene`），但编辑器内还没有场景面板；编辑器内的 `plan_frame` 仍未传入相机，场景片段也未接入时间轴。
- 跟随只解析图层**静态**世界坐标；图层自身关键帧动画完成后，跟随需要逐帧重解目标点。
- 相机预设没有可视化曲线编辑/管理 UI；`FocusTarget` 无目标时为绝对点。
- Slint 视口（实时预览/编辑）尚未开始（Phase 4）。
- MCP 场景工具同步执行、无进度/取消；大场景渲染会阻塞该次调用。

**下一步（建议）**
1. **Phase 4 Slint 视口 Spike**：编辑器中央区显示场景帧（先 CPU 图像，后 wgpu 纹理），支持相机拖拽/推拉预览。
2. 编辑器接入：场景面板 + 把 `plan_frame` 调用点（审计 8 处）统一到 `plan_frame_with_camera`，预览/导出共用相机求值。
3. 图形图层：用 Concat 的 title/Shape 或生成 PNG 后进入同一管线。
4. 跟随增强：图层关键帧动画完成后逐帧解算目标点；加世界边界/zoom 约束。
5. 清理 stash 中的 parallax（正确公式 `(world-camera)*p` + 测试）。

---

## 5. Git

```
Phase 2/3 feat(scene): follow + move sequences, Concat project persistence, MCP scene CRUD + e2e
322b4a1a feat(scene): concat-scene model, headless camera renderer (concat-studio), MCP scene_render tool + demo
977a6946 docs(camera): record verified P0 state...
12c6f35f feat(render): integrate virtual camera projection into concat-render and export pipeline
072171ab feat(camera): introduce concat-camera crate...
```

### 上游同步（2026-11-19）
- `git fetch upstream && git merge --no-ff upstream/main`：**无冲突合并**（上游 v0.2.8/v0.2.9 已包含 beats/waveform/导出修复）。
- 合并后 MSVC 下 `cargo test` 全部通过（camera 6、scene 5、mcp 10+4、studio 编译）。
- 我们的修改集中在 **新增 crate**（concat-camera/scene/studio）与少量钩子（`FramePlan.camera`），所以合并成本低。

> 注意：`src/` 下有 `rustup override`（MSVC）。GNU 仍可用 `cargo +1.93.1-x86_64-pc-windows-gnu` 临时构建，
> 但正式基线为 MSVC + `vcvars64.bat`。