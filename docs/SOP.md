# 2D Mini Studio — 项目 SOP（标准作业程序）

> 适用对象：任何接手本项目的 Agent / 开发者 / 后续较弱模型。
> 原则：**不要重新设计架构**。按本 SOP 的步骤做；看不懂时先读 `docs/CAMERA_DESIGN.md` 与本文件 §3 模块地图。
> 仓库：`G:\Projects\Concat_camera`（Fork 自 `jub0t/Concat`，AGPL-3.0-or-later）。

---

## 0. 先读这三份文档（顺序）

1. **`docs/HANDOVER.md`** — 当前进度、Git 状态、已验证命令、踩坑。
2. **`docs/CAMERA_DESIGN.md`** — 摄像机/场景数学、坐标约定、已知限制、P1/P2 方案。
3. **`项目任务书.md`**（本仓库 `docs/PROJECT_BRIEF.md`）— 目标与阶段要求。

---

## 1. 环境（必须逐项核对）

| 项 | 位置/版本 | 核对命令 |
|---|---|---|
| Rust | 1.93.1，`x86_64-pc-windows-msvc`（`src/` 目录 override） | `rustup show` |
| MSYS2（仅 GNU 备用） | `G:\msys64` | 非必需 |
| FFmpeg 8.1 shared | `deps/ffdl/ffmpeg-n8.1.3...win64-lgpl-shared-8.1` | `echo $env:FFMPEG_DIR` |
| ONNX Runtime 1.28 | `onnxruntime-win-x64-1.28.0` | `echo $env:ORT_LIB_LOCATION` |
| VS Build Tools 2022 | `C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools` | `vswhere -latest -property installationPath` |
| clang（bindgen） | `G:\msys64\ucrt64\bin` | `echo $env:LIBCLANG_PATH` |

**环境入口脚本**：`. G:\Projects\Concat_camera\setup-env.ps1`（设置 FFMPEG_DIR / ORT / LIBCLANG / PATH）。

**MSVC 构建/测试必须在 vcvars64 下**，命令模板：

```powershell
. G:\Projects\Concat_camera\setup-env.ps1
cmd.exe /c "call `"C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat`" >nul && cargo <args>"
```

---

## 2. 目录与模块地图（不要乱建文件）

```
G:\Projects\Concat_camera\
├─ docs/                  # 设计文档（CAMERA_DESIGN、HANDOVER、PROJECT_BRIEF、SOP 等）
├─ examples/camera_demo/  # 场景 JSON + 测试素材（输出 out.mp4 已被 gitignore）
├─ deps/ffdl/             # FFmpeg 8.1（bin/lib/include）
├─ onnxruntime-win-x64-1.28.0/
└─ src/                   # ★ Cargo workspace 在这里
   ├─ Cargo.toml          # workspace 定义（成员 crates/*）
   ├─ crates/
   │  ├─ concat-core/     # 时间原语、Transform、Ease（上游）
   │  ├─ concat-project/  # 工程 JSON / Command / Undo（上游）
   │  ├─ concat-media/    # 解码/probe（上游）
   │  ├─ concat-render/   # FramePlan / plan_frame / plan_frame_with_camera（上游+相机钩子）
   │  ├─ concat-export/   # render_on / ExportClip / ExportRequest（上游）
   │  ├─ concat-camera/   # ★ 我们的虚拟摄像机（投影/预设/插值）
   │  ├─ concat-scene/    # ★ 我们的 2D 场景/图层/相机轨道/相对运镜
   │  ├─ concat-studio/   # ★ 无头渲染器（Scene → ExportClip → render_on）
   │  ├─ concat-mcp/      # MCP 工具（含我们的 scene_render）
   │  ├─ concat-api/      # 编辑器 JSON-RPC API（上游）
   │  └─ concat/          # Slint 桌面 UI（上游；P2/P4 接入）
   └─ .cargo/config.toml  # 不放机器路径
```

**规则**：
- 新逻辑放 `concat-scene` / `concat-camera` / `concat-studio`；上游文件只加**薄钩子**（如 `FramePlan.camera`）。
- 不引入新的 2D 渲染引擎；场景输出 = `Transform`/`ExportClip`，交给 Concat 合成。

---

## 3. 标准工作流（每次修改都走）

```
修改代码
  ↓
. setup-env.ps1
  ↓
cmd /c "call vcvars64 && cargo check -p <crate>"   # 快速编译检查
  ↓
cmd /c "call vcvars64 && cargo test -p <crate>"    # 对应测试
  ↓
修警告/错误（warnings 视为要处理）
  ↓
更新 docs（CAMERA_DESIGN.md / HANDOVER.md 变更记录）
  ↓
git add -A && git commit（信息格式见 §5）
  ↓
git push origin main（认证已完成；失败先重试）
```

---

## 4. 常用命令清单（可复制）

### 构建/测试
```powershell
# 全量核心测试（常用）
cmd /c "call vcvars64 && cargo test -p concat-camera -p concat-scene -p concat-mcp"

# 只跑某个测试
cmd /c "call vcvars64 && cargo test -p concat-scene plan_layers_projects_world_to_camera_frame"

# 构建 studio 渲染器
cmd /c "call vcvars64 && cargo build -p concat-studio"

# 构建 MCP server
cmd /c "call vcvars64 && cargo build -p concat-mcp"
```

### 运行示例渲染
```powershell
# 渲染示例场景（PNG + 相机推近）
& .\target\debug\concat-studio.exe render examples\camera_demo\scene.json examples\camera_demo\out.mp4

# 裸场景转 Concat 工程文档（顶层 scene），再渲染
& .\target\debug\concat-studio.exe save examples\camera_demo\scene.json examples\camera_demo\scene_project.json
& .\target\debug\concat-studio.exe render examples\camera_demo\scene_project.json examples\camera_demo\out.mp4

# 用 concat-cli 验证输出
& .\target\debug\concat-cli.exe probe examples\camera_demo\out.mp4
```

### Slint 场景视口 Spike
```powershell
# 打开示例场景（滑杆选帧、拖动画面移动相机、Save 写回）
cargo run -p concat-scene-view -- examples/camera_demo/scene.json
```

### MCP 手动验证（无 GUI）
```powershell
# 用我们的 e2e 测试跑通 Agent 闭环
cmd /c "call vcvars64 && cargo test -p concat-mcp an_agent_renders_a_camera_scene_headless"
```

### Git / 上游同步
```powershell
# 同步上游（我们集中在新增 crate，冲突极少）
git fetch upstream
git merge --no-ff upstream/main
cargo test -p concat-camera -p concat-scene -p concat-mcp   # 验证后再提交

# 提交
git add -A
git commit -m "feat(scene): <说明>"
git push origin main
```

---

## 5. 功能食谱（照做，不自由发挥）

### 5.1 修改场景/图层数据字段
- 改 `concat-scene` 的 `Scene`/`SceneLayer`/`CameraKey`。
- 新增字段必须 `#[serde(default)]`，保证旧 JSON 能读。
- 同步补单测（序列化往返 + 行为）。
- 同步更新 `docs/CAMERA_DESIGN.md` 与 `HANDOVER.md`。

### 5.2 新增 MCP 工具（recipe）
1. 在 `concat-mcp/src/tools.rs` 的 `Action` 加一个枚举变体。
2. 在 `catalogue()` 增加 `Tool { ... }`（名称/标题/描述/动作）。
3. 在 `input_schema()` 里提供 JSON Schema（或复用 `schemas.method`）。
4. 在 `run()` 里加分发；参数校验给清晰错误。
5. 在 `samples()` 里加一个示例输入，跑 `cargo test -p concat-mcp`。
6. 在 e2e 里补一条真实调用测试（看 `an_agent_renders_a_camera_scene_headless` 模式）。

### 5.3 新增运镜预设类型
- 在 `concat-camera` 的 `CameraMoveType` 加变体；`compute_target_pose` 里实现相对/绝对计算。
- 在 `concat-scene` 的 `MovePreset` 可直接使用。
- 补单测：给不同起点，断言目标位姿与中间帧。

### 5.4 新增图层种类/图形
- 图形图层（rect/ellipse/line）目前没有素材；先用 `image`/`tiny-skia` 生成 PNG 后走 `Image` 路径（不要重写合成器）。
- 扩展 `LayerKind` 时补序列化往返测试。

### 5.5 修改导出参数
- `concat-studio::render_scene` 的 `RenderOptions` + request JSON 字段；同时看 `concat-export` 的 `ExportRequest` 字段。
- 修改后用示例场景跑一次，检查 `concat-cli probe` 输出（分辨率/帧率/时长）。

### 5.6 编辑器导出接相机（后续）
- 目标：让编辑器/CLI 的导出调用 `plan_frame_with_camera`。
- 先搜索 `plan_frame(` 的所有调用点（当前 8 处），统一封装后再改。
- 必须加"预览帧与导出帧一致"的像素测试。

### 5.7 上游同步
- 见 §4 Git；合并后必须跑测试；有冲突先解决再提交。
- 冲突高发：`Cargo.lock`、`concat-mcp/tools.rs`、`concat-render/plan.rs`、`concat-export/lib.rs`。

### 5.8 修渲染错误
- 先定位是"数据（Transform）"还是"合成/编码（Concat）"。
- 用 `concat-cli probe` 检查输入；用 `--selftest`/示例工程复现。
- 相机问题先查 `concat-camera` 测试 + `CAMERA_DESIGN.md` 的坐标约定。

---

## 6. 每个任务完成后的验收检查清单

- [ ] `cargo check` 通过（对应 crate）
- [ ] 相关 `cargo test` 通过（新增功能必须有测试）
- [ ] 无新增 `warning`（如引入则修复）
- [ ] 示例工程能导出且 `probe` 正常
- [ ] 更新了 `docs/CAMERA_DESIGN.md` 或 `HANDOVER.md`
- [ ] Commit 信息清晰（`feat/fix/docs`）
- [ ] Push 成功（`git status -sb` 干净且 `ahead 0`）

---

## 7. 常见问题与排错（踩坑记录）

1. **PowerShell 5.1 不支持 `? :`**：用 `if/else`；脚本用 `.ps1` 或 `.cmd`，别用三元。
2. **`cmd` 找不到 `pwsh`**：WindowsApps 别名在 cmd 下不可见；脚本里用 `powershell.exe`。
3. **`temp` 路径漂移（MSYS2）**：`TEMP=/tmp` 会指到 `G:\msys64\tmp`；稳定路径用应用目录。
4. **QML context property 被同名 QML property 遮蔽**：用 `setSource` 后 `setProperty` 注入。
5. **QtADS 标签栏发白**：必须设置深色 QPalette + 追加 ADS stylesheet 覆盖。
6. **`QProgressDialog` autoClose**：`setValue(max)` 会触发 canceled → 关掉 autoClose/autoReset。
7. **FFmpeg DLL 找不到**：运行必须带 `FFMPEG_DIR\bin` 到 PATH（`setup-env.ps1` 已处理）。
8. **Windows 自带 `System32\onnxruntime.dll` 被优先加载**：把我们的 DLL 放 exe 同目录。
9. **JSON BOM / 控制字符**：写文件用 UTF-8 无 BOM；PowerShell 的 `Set-Content -Encoding UTF8` 会加 BOM。
10. **keyed 属性叠加**：`ExportClip` 的 keyed 属性是绝对值，base 必须中性（scale 1/offset 0）。
11. **x264 导出失败**：确认 FFmpeg build 是 `lgpl-shared`（含 libx264）。
12. **VS Build Tools 链接失败**：确认已在 `vcvars64.bat` 环境下运行 cargo。
13. **Git push 401/SSL**：认证一次即可；若 SSL 握手失败，重试或 `GIT_SSL_NO_VERIFY=1` 临时绕过（仅本机调试）。
14. **不要改上游大文件**：功能放新 crate；上游文件只加钩子。
15. **不要 force-push main**。

---

## 8. 禁止事项（红线）

- ❌ 不引入 Qt/WebView/Tauri；UI 只用 Slint。
- ❌ 不自建 2D 渲染/合成器；用 Concat 的合成器。
- ❌ 不引入新的 MCP 框架（rmcp）；扩展现有 `concat-mcp`。
- ❌ 不重新评估插值库（用 `concat-core::Ease`）。
- ❌ 不在 UI 主线程跑解码/编码/AI/网络。
- ❌ 不做假按钮/空壳功能冒充已实现。
- ❌ 不写无法序列化/不可迁移的字段。
- ❌ 不修改上游文件的既有逻辑，除非必要钩子。

---

## 9. 会话结束交接清单

每次工作结束前，必须：

1. `git status --porcelain` 干净或已提交。
2. `cargo test -p concat-camera -p concat-scene -p concat-mcp` 通过。
3. `examples/camera_demo/out.mp4` 可重新生成（如需）。
4. `docs/HANDOVER.md` 更新"当前进度/已知问题/下一步"。
5. 若改了相机/场景数学，`docs/CAMERA_DESIGN.md` 同步更新。
6. 提交信息使用 `feat|fix|docs|test|chore(scope): 摘要` 格式。

---

## 10. 当前进度（2026-11-19）

| 项 | 状态 |
|---|---|
| Phase 0 审计 | ✅（`docs/CAMERA_DESIGN.md`、`HANDOVER.md`、研究文档已整合） |
| Phase 1 Headless 场景+相机 | ✅ 核心完成（`concat-scene`、`concat-studio`、示例 MP4） |
| Phase 2 运镜 | ✅ 核心完成（相对/跟随/组合烘焙；曲线编辑器 UI 待 Phase 4） |
| Phase 3 MCP | ✅ 核心完成（`scene_create/add_layer/camera_move/get/render` + 工程持久化 + e2e） |
| Phase 4 Slint UI | 🟡 Spike 完成（`concat-scene-view` 独立窗口；编辑器内 Pane 待接线） |
| Phase 5 剪辑增强 | ⬜ 未开始 |
| Phase 6 资产/ASR | ⬜ 未开始 |
| Phase 7 网络/AI | ⬜ 未开始 |

**下一步建议**：把 `concat-scene-view` 接成编辑器可停靠 Pane（`CAMERA_DESIGN.md` §1.6 清单）→ 编辑器预览/导出接相机 → 图形图层素材化 → 跟随逐帧解算/边界约束 → parallax。

---

## 11. 给后续弱模型的使用说明

- **先读** `docs/HANDOVER.md` 和 `docs/CAMERA_DESIGN.md`，再读本 SOP。
- **不要重新设计**：遇到"该不该换个方案"的问题，默认按本 SOP 做；发现方案确实不可行时，才在文档里记录原因并提出替代。
- **小步**：每次只改一个点，改完立刻 `cargo check` + `cargo test`。
- **可回退**：每完成一个独立改动就 commit；不确定的实验放 `git stash`（带消息）。
- **写测试**：新功能必须有单测；修复 bug 必须有回归测试。
- **卡壳时**：查 §7 踩坑记录；仍不行就把错误全文和命令贴给用户，不要硬猜。

> 本 SOP 是活文档：每完成一个阶段，更新 §10 与 §11。