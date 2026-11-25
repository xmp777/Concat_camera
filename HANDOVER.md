# Concat_camera 交接单

> 最后更新：2026-11-26
> 仓库：`G:\Projects\Concat_camera`（fork of `jub0t/Concat`，AGPL-3.0-or-later）
> 远程：`origin` = `xmp777/Concat_camera`，`upstream` = `jub0t/Concat`
> 当前：Phase 0–14 已完成；下一阶段为 Phase 15 图片资产 carry 与缺源 warning。

## 快速上手

```powershell
# 环境：FFmpeg / ONNX Runtime / clang / Skia
. G:\Projects\Concat_camera\setup-env.ps1

# MSVC 构建与测试
cmd /c "call `"C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat`" >nul && cd /d G:\Projects\Concat_camera\src && cargo test -p concat-scene"

# 编辑器构建（Skia 首次编译较慢）
cmd /c "call `"C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat`" >nul && cd /d G:\Projects\Concat_camera\src && cargo build -p concat"
```

Cargo workspace 在 `src/`，不在仓库根。阶段状态以 `PROGRESS.md` 为准，详细证据在 `docs/handovers/`，相机数学在 `docs/CAMERA_DESIGN.md`。

## 当前状态

- Phase 0–13：已完成，阶段稳定基线为 `50fbcfa3`；Phase 14.1 音频修复基线为 `4ddc1074`。
- 多场景容器、严格 `read_scene_by_id`、形状资产物化/搬运、编辑器插入接线、`AddSceneClip` 幂等与 legacy 镜像统一均已落地。
- Phase 14 实机走查已完成：Scene 预览、2 秒金色片段、PNG 字节级搬运、map/legacy 镜像、undo/redo、幂等重插与保存重开均通过；截图见 `docs/assets/phase14_windows_walkthrough.png`，证据见 `docs/handovers/2026-11-26-phase14-ui-walkthrough.md`。
- host export 音频漂移已定位并修复：B 帧重排曾让 mux 的流结尾时间回退，24 fps 成片音频缩短至 5.931s；`Feed.end` 改为单调最大值，七种帧率回归均通过。
- 图片图层相对 asset 尚未随嵌入迁移，缺源仍可能静默跳过。
- `expand_project_scenes` 仍保留显式 id 落空回退，待统一严格语义并补日志。
- `plan_frame_with_camera` / `FramePlan.camera` 是未接入生产的通用相机管道；实际场景相机使用 `concat-scene` 关键帧烘焙。
- `SceneLayer` 当前只有静态 transform；“跟随逐帧”需要先设计图层关键帧轨道和求值顺序。

## 下一步

1. 推进图片资产 carry / 缺源 warning（Phase 15）。
2. 统一 expand 两类回退并补日志（Phase 16）。
3. 清理相机死管道，完成跟随逐帧与 parallax 架构评审；本轮不实现 parallax。

## 关键路径

- 编辑器 Scene 面板：`src/crates/concat/src/panes/scene.rs`
- 场景读取/展开/资产搬运：`src/crates/concat-studio/src/lib.rs`
- 场景命令与幂等：`src/crates/concat-project/src/commands/scene.rs`
- MCP 工具与 e2e：`src/crates/concat-mcp/src/tools.rs`、`src/crates/concat-mcp/tests/e2e.rs`
- 渲染计划：`src/crates/concat-render/src/plan.rs`
- 走查 fixture：`.walkthrough/`

## 已知问题

- 本机 `concat-media` 全量测试有两项既有硬件编码码率断言失败：H.264 Media Foundation 平均码率 393/1000 kb/s、HEVC NVENC CBR 1359/1000 kb/s；与本次音频封装修复无关，相关音频 36 项与 host export 27 项均通过。
- 根目录旧的 Phase 8 交接内容已移除，历史细节保留在 `docs/handovers/`。
