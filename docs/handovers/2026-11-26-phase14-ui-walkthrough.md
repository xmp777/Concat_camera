# Phase 14 编辑器形状资产 carry 实机走查交接

> 日期：2026-11-26
> 状态：已完成
> 对应提交：`836c5894`
> 子代理审查：独立只读审查 1 轮，[REVIEW_PASSED]；P0/P1 均为零

## 1. 本阶段目标

- 在真实 Windows 编辑器中验证 Phase 13 的形状资产搬运与 Scene 插入接线，形成可复现的端到端证据。
- 不在本阶段范围内：产品代码改动、图片图层 asset carry、expand 回退统一、相机死管道清理、跟随逐帧与 parallax 实现。

## 2. 交付内容

- 新增实机截图 `docs/assets/phase14_windows_walkthrough.png`（2560×1440），展示 Scene 面板红色形状预览、2 秒金色时间轴片段和恢复后的工程状态。
- 新增本交接，记录启动方式、隔离状态、UI 自动化校准和逐项断言。
- 更新 `PROGRESS.md`、`HANDOVER.md`、`docs/SOP.md`：Phase 14 完成，下一阶段转为 Phase 15。
- 本次 UI 走查交付未修改产品源码；验收覆盖的是既有 `carry_shape_assets`、`AddSceneClip`、legacy 镜像和 undo/redo 路径。Phase 14.1 的 `concat-media` 音频修复见前置提交 `4ddc1074`。

## 3. 验证证据

### 3.1 自动回归

| 项 | 命令 / 操作 | 结果 |
|---|---|---|
| 格式 | `cargo fmt --all -- --check` | 通过 |
| Scene 命令 | `cargo test -p concat-project scene` | 8 passed |
| 形状资产搬运 | `cargo test -p concat-studio carry` | 2 passed |
| 编辑器库 | `cargo test -p concat --lib` | 245 passed |
| 编辑器构建/启动 | Phase 14 前置阶段 MSVC 构建 `concat.exe`，本阶段实际启动 | 通过 |
| 差异空白 | `git diff --check HEAD` | 通过 |

编辑器测试保留既有警告：Slint `viewport-height` 弃用提示、未使用组件 `TextLink`、未使用方法 `selected_scene_id`。本阶段未引入新警告。

### 3.2 实机操作与断言

使用 `G:\Projects\Concat_camera\src\target\debug\concat.exe --project <临时工程目录>` 启动。工程副本位于 ASCII 临时路径，`APPDATA` 与 `LOCALAPPDATA` 重定向到独立临时目录，避免污染入库 fixture 或机器状态。

| 步骤 | 操作 | 结果 |
|---|---|---|
| 1 | Add Pane → Scene | Scene 面板打开并停靠 |
| 2 | Open scene → 原生对话框填入 `G:\Projects\Concat_camera\.walkthrough\opening.json` | 场景载入，红色矩形正确渲染，帧范围 0–59 |
| 3 | Add to timeline | 金色 Scene 片段出现，时间轴总时长 2 秒 |
| 4 | 检查工程 JSON | 顶层和 `timelines[0]` 各 1 个 clip；`kind="scene"`、`sceneId="走查场景"`、`start=0`、`duration=2` |
| 5 | 检查场景容器 | `scenes["走查场景"]` 存在，顶层 legacy `scene` 与 map 条目深度序列化相等 |
| 6 | 比较形状 PNG | 源文件与工程 `assets/scene-shapes/61808269636e7950.png` SHA-256 均为 `7c10c0160fbbc5d4fe8e372380b4584f1566a473e0d983aeac5d0c489667a2e9` |
| 7 | `Ctrl+Z` / `Ctrl+Shift+Z` | clip 数 1 → 0 → 1 |
| 8 | 相同场景再次 Add to timeline | clip 仍为 1 |
| 9 | 重复插入后立即 `Ctrl+Z` | clip 直接回到 0，证明幂等分支没有新增幽灵 undo 快照 |
| 10 | Redo、`Ctrl+S`、关闭并等待进程退出 | 工程锁释放，落盘完成 |
| 11 | 使用相同隔离状态与 `--project` 重启 | Scene 面板、红色预览、2 秒金色片段恢复；JSON 与 PNG 断言再次通过 |

### 3.3 UI 自动化注意点

- 标题栏 Add Pane 是 Slint 自定义控件，当前不暴露标准 UIA `Button`；实机使用截图与几何校准后的坐标点击。
- Scene 面板内的标准按钮会暴露 UIA `Button`，可直接使用 `InvokePattern`，避免停靠宽度变化导致坐标漂移。
- 原生文件对话框的文件名和按钮在当前 UIA 树中显示为 `Pane`，因此通过剪贴板粘贴 Unicode 路径后点击“打开”；这同时绕开 PowerShell 5.1 `Start-Process` 的非 ASCII 参数编码问题。
- 重复走查应复制 `.walkthrough/走查工程` 到临时 ASCII 路径，不要直接修改入库模板。

## 4. 子代理审查结论与修复

### P0（必须修复）

无。

### P1（修复或写豁免）

无。

### P2 / 后续

- 审查建议统一 Phase 14 完成状态：已在 `PROGRESS.md` 与本交接中标记 [REVIEW_PASSED]，与 `HANDOVER.md`、SOP 一致。
- 审查建议明确源码改动边界：已注明本次 UI 交付无源码改动，Phase 14.1 音频修复位于 `4ddc1074`。
- 可为自定义 Add Pane 控件补充可访问性角色和名称，改善 UIA 与辅助技术支持；本阶段不扩大 UI 改动。
- 可把本次临时自动化脚本整理为仓库内可复现工具；当前证据已由截图、JSON 断言和测试结果闭环。

## 5. 已知限制

- 本机 `concat-media` 全量测试仍有两项既有硬件编码码率断言失败：H.264 Media Foundation 平均码率 393/1000 kb/s；HEVC NVENC CBR 1359/1000 kb/s。Phase 14 前置交接已独立复现并记录，与本阶段文档/截图交付无关。
- 编辑器测试首次触发 Skia test-profile 缓存重建，官方二进制下载与源码依赖同步遇到 TLS/网络失败；复用本机同版本 `0.153.3` 已有成功构建产物后，245 项测试通过。未修改依赖版本或项目源码。
- 图片图层的相对 asset 仍不会随嵌入迁移，缺源仍可能静默跳过，留待 Phase 15。

## 6. 下一阶段

1. Phase 15：将形状资产 carry 泛化到图片图层，覆盖安全相对路径、canonical confinement、冲突处理、tmp+rename、结构化报告与缺源 warning。
2. 为编辑器与 MCP 两条入口补一致的单测/e2e/UI 断言。
3. 独立审查通过后更新本交接的审查结论与提交 hash，再提交和推送。

## 7. Git

- 实现提交：`836c5894`
- 审查修复提交：`836c5894`（两项 P2 建议已并入）
- 推送：`origin/main` 已同步；最终 hash 由后续文档提交记录。
