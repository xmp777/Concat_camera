# Concat_camera 交接单（P1 状态）

> 最后更新：2026-11-19
> 仓库：`G:\Projects\Concat_camera`（fork of `jub0t/Concat`，AGPL-3.0-or-later）
> 远程：`origin` = `xmp777/Concat_camera`，`upstream` = `jub0t/Concat`
> 当前：**P0 收尾完成并已推送；P1 能力闭环（concat-scene + 相对运镜 + MCP scene_render + MP4）已跑通；MSVC 为正式基线**

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

---

## 2. 验证记录（本机，2026-11-19）

| 项 | 结果 |
|---|---|
| `cargo test -p concat-camera` | 6 passed |
| `cargo test -p concat-scene` | 5 passed |
| `cargo test -p concat-render` | 130 passed, 1 ignored |
| `cargo test -p concat-mcp`（MSVC） | 4 e2e passed（含 scene_render） |
| `concat-cli probe examples/camera_demo/out.mp4` | 3.000s / h264 / 1920×1080 / 30fps |
| 示例截图 | `docs/assets/camera_demo_frame0.png`、`frame89.png` |

### 关键坑（已修复/记录）
1. **keyed 属性是绝对值**：`ExportClip` 的 base（scale/offset/rotation）对已有关键帧的属性必须取中性值，否则叠加导致位移/缩放翻倍（道具曾因此消失）。
2. **相机单位**：`VirtualCamera.rotation` 弧度，`Transform.rotation` 度；投影时转换。
3. **世界尺寸**：`concat-camera::project_transform` 假设世界=画框像素；`concat-scene` 采用世界像素 + 显式 `world_width/height`，plan 时归一化。
4. **Windows 自带 `C:\WINDOWS\SYSTEM32\onnxruntime.dll`** 会被优先加载；调试/运行 MCP 时需把我们的 `onnxruntime.dll` 放在 exe 同目录（已复制到 `target/debug` 与 `deps`）。
5. 本机 ONNX `onnxruntime.lib` 仅 2KB（MinGW 用）；MSVC 构建目前能过（未链接 ORT 符号），若未来链接失败，从 `onnxruntime-1.28.0.nupkg` 恢复官方 MSVC `.lib`。
6. MCP 的 `scene_render` 是同步调用（渲染期间阻塞该工具调用）；大场景应后续改为任务/进度通知。

---

## 3. 代码地图

```
src/crates/
├─ concat-camera/    # 投影/插值/运镜预设（P0）
├─ concat-scene/     # 2D 场景/图层/相机轨道（P1）
├─ concat-studio/    # Scene -> ExportClip -> render_on（P1）
├─ concat-mcp/       # MCP：新增 scene_render 工具
├─ concat-render/    # FramePlan.camera 钩子
├─ concat-export/    # 合成/编码（复用）
└─ concat/           # Slint 编辑器（P2 视口接入）
examples/camera_demo/ # 场景 JSON + 测试素材 + 说明
docs/CAMERA_DESIGN.md # 设计/坐标/限制/P1 方案
```

数据流（P1）：
`scene.json → concat_scene::Scene → plan_layers(camera at frame) → ExportClip(animation keys) → concat_export::render_on → MP4`

---

## 4. 已知限制 / 下一步

**限制**
- 图形图层（shape）尚未进入导出（需要素材化或 Concat 的 title/图层路径）。
- 场景仅存在于独立 `scene.json`，尚未并入 Concat 工程文档；编辑器内的 `plan_frame` 仍未传入相机。
- 相机预设没有持续时间/组合/跟随求解的完整 UI；`FocusTarget` 为绝对点。
- Slint 视口（实时预览/编辑）尚未开始（P2）。
- MCP `scene_render` 同步、无进度/取消。

**下一步（建议）**
1. **P2 Slint 视口 Spike**：在编辑器中央区显示 `concat-studio` 同款场景帧（先 CPU 图像，后 wgpu 纹理），支持相机拖拽。
2. 把 `Scene` 持久化进 Concat 工程（`project.json` 扩展字段），并让编辑器的 `plan_frame` 调用 `plan_frame_with_camera`（审计 8 个调用点）。
3. 图形图层：用 Concat 的 title/Shape 或生成 PNG 后进入同一管线。
4. 相机预设管理 UI（相对/组合/跟随）+ MCP 工具。
5. 清理 stash 中的 parallax（正确公式 `(world-camera)*p` + 测试）。

---

## 5. Git

```
322b4a1a feat(scene): concat-scene model, headless camera renderer (concat-studio), MCP scene_render tool + demo
977a6946 docs(camera): record verified P0 state...
12c6f35f feat(render): integrate virtual camera projection into concat-render and export pipeline
072171ab feat(camera): introduce concat-camera crate...
```

> 注意：`src/` 下有 `rustup override`（MSVC）。GNU 仍可用 `cargo +1.93.1-x86_64-pc-windows-gnu` 临时构建，
> 但正式基线为 MSVC + `vcvars64.bat`。