# Phase 5：Scene 时间轴片段 + Inspector Camera 区 + 导出接相机

> 日期：2026-11-20
> 状态：已完成（子代理审查 2 次，P0 全清，回归通过）
> 对应提交：`3f4ccbe3`（模型+命令）→ `67359911`（展开）→ `a3ca4186`（编辑器）→ `ec23f914`（seam 测试）→ `7d75fa74`（审查修复）→ `d17dcff7`（Camera 区段修复）
> 审查：两个只读子代理（核心 Rust / 编辑器 UI），P0 各 2 个，全部实锤修复

## 1. 本阶段目标

- Scene 成为时间轴上的一种片段（`scene_id` + 起点/时长），场景帧由**片段本地时间**决定。
- Inspector 暴露相机参数（X/Y/Zoom/Rotation/关键帧），接 Undo 命令栈。
- 编辑器的预览与导出都产出相机正确的场景帧。

不在本阶段范围内：多场景（`scenes` map）、图形图层素材化、跟随逐帧解算、parallax、MCP 场景片段工具、相机曲线编辑器。

## 2. 交付内容

### 工程模型（concat-project）

- `ClipKind::Scene`（serde `"scene"`）+ `Clip.scene_id`；`settle_clip`/`flatten` 按"无媒体"处理，flatten 对 Scene 片段失败软跳过（展开拥有渲染）。
- **场景存放在文档顶层 `scene` 字段**（raw Value，与 `concat-studio::SCENE_KEY` 同键，字节级往返）。场景命令经 `concat-scene` 解析-变更-写回，解析失败是 `CommandError` 而非文档拒载。
- 新命令（`commands/scene.rs`）：`AddSceneClip`（场景嵌入-或-匹配；片段时长=场景时长/帧率；**先验轨后嵌入**，TrackGone 不留半改状态）、`SetSceneCameraKey`（upsert，未提及字段沿用该帧求值位姿，viewport 永不被键控）、`RemoveSceneCameraKey`。全部走快照式 undo，`applied=false` 防幻影编辑。
- `concat-project` 新依赖 concat-scene/concat-camera（仅命令臂使用）；schema 里 scene/easing 字段 `schemars(skip)`（MCP 场景工具仍是手写 schema，见已知限制）。

### 场景展开（concat-studio，预览与导出共用的渲染语义）

- `expand_project_scenes(project, dir)`：把每个 Scene 片段替换为其场景图层重建的图像片段（时间轴窗口内），**相机运动按窗口重新采样烘焙为线性 Scale/OffsetX/OffsetY/Rotation/StretchX/StretchY 关键帧**（`at` 为片段内 0..=1）——与整场导出（`scene_clips`）同一机制，缓动由场景相机轨道自身的采样承担。
- **每层一条轨**（合成器每轨同一时刻只认一个 clip，后者覆盖前者——审查实机复现了共轨丢层后修复）：最低 z 层留原轨（背景轨语义随它），其余逐层向上 mint `{track}-{clip}-{layer}` 轨。
- 场景背景遵循 Concat 的 background 语义：仅当场景片段位于最下方图像轨时，展开片段携带 `Background::Color`。
- 图形图层（无素材）/不可见图层/缺素材跳过；`scn{N}` 合成 media id 防碰撞；foreign/缺失 `scene_id` 的片段从展开时间线移除（渲染为无）。

### 编辑器（concat）

- **Scene 面板双源**：工程场景（顶层 `scene` 字段——面板跟随文档，任何来源的编辑经 `sync_from_project` 落到面板；revision 门控 + 占位面板排队）与独立文件（旧行为：本地编辑 + Save）。
- **本地时间映射** `scene_frame_at`：片段内 = 本地时钟；片段外 = 最近片段的边界帧；独立场景保持全局映射。回退滑杆只在"时间轴没有引用本场景的片段"时显示。
- **拖动相机 = 命令**：`SetSceneCameraKey` 经 `scene-camera-drag` 手势（DragEnded 关闭 + 900ms 重开门），undoable/autosave；独立文件拖拽保持本地编辑。
- **Inspector Camera 区**：Scene 片段选中时唯一的"相机"标签——X/Y/Zoom/Rotation 行（度）+ 移除关键帧按钮；只覆写拨动的字段；`scene-camera-inspector` 手势。
- 时间轴 UI：金色 `kind-scene` + 取景框徽章（dark/light 双主题、拖拽 chip、lane 高度）；"插入时间轴"按钮；`AddToTimeline` 无工程时提示。
- 监视器与导出接展开：`request_for` 与监视器 flatten 前都先 `expand_project_scenes`（监视器的 revision 缓存覆盖展开结果）。

### 测试与示例

- 新增 11 个测试：命令层 6（嵌入/往返/wire/undo+幻影/双场景拒绝/移键/无场景）、展开 3（双层+相机烘焙+轨隔离+stretch、裁剪窗口、无场景透传）、seam 1（Session→AddSceneClip→flattened_clips 断言两图像片段异轨+烘焙）、面板映射 1。
- `examples/scene_demo/`：可打开的演示工程（scene-demo），`make_the_scene_clip_demo_project`（ignored 测试）再生成。
- headless 实测：demo 工程 `concat-studio render` → `out.mp4` 3.033s / h264 / 1920×1080 / 91 帧。

## 3. 验证证据

| 项 | 命令 / 操作 | 结果 |
|---|---|---|
| concat-camera | `cargo test -p concat-camera`（MSVC） | 6 passed |
| concat-scene | 同上 | 12 passed |
| concat-project | 同上 | 266 passed（+6 场景命令） |
| concat-studio | 同上 | 7 passed（+3 展开） |
| concat-host lib | 同上 | 172 passed（+1 seam） |
| concat-mcp | 同上 | 10 lib + 5 e2e |
| 编辑器 | `cargo test -p concat --lib`（含 26 locale i18n 校验） | 244 passed |
| 构建 | `cargo build -p concat`（MSVC） | 通过（10m05s） |
| headless | `concat-studio render examples/scene_demo/scene-demo/concat.json out.mp4` + `concat-cli probe` | 3.033s / h264 / 1920×1080 / 30fps |
| 实机 UI | 编辑器打开 scene-demo：金色 Demo 片段在 lanes；Scene 面板自动加载（"Frame 0/90"）；seek 2.5s → 面板 "Frame 75/90"（本地映射）；片段外 → "Frame 90/90"（末帧钳制）；点击片段 → Inspector 标题"相机" | 通过（截图 `docs/assets/scene_clip_timeline.png` 等） |
| 存量归因 | concat-host --test export 3 处音频时长断言失败 | 与本阶段无关：回退 session.rs 后旧代码同样失败（音频尾帧量化，机 related） |

## 4. 子代理审查结论与修复

### P0（全部修复）

| 问题 | 位置 | 修复 | 回归测试 |
|---|---|---|---|
| 展开把所有图层放同一轨；合成器每轨一 clip（后者覆盖）→ 多图层只剩最高层（审查实机渲染复现） | concat-studio/lib.rs 展开循环 | 每层一条轨：最低 z 留原轨，其余 mint 新轨 | studio 测试断言异轨 + mint 轨；host seam 断言 `bg.track != hero.track` |
| 窗口监视器与全部导出 flatten 的是原始工程——Scene 片段被 skip，场景渲染为空白 | concat-host/export.rs request_for；concat/studio/monitor.rs preview flatten | 两处 flatten 前先 `expand_project_scenes` | request_for 路径由 seam 测试的 flatten 层保障；实机监视器出图（shot6-7 无错误 toast） |
| `sync_from_project` 首臂极性反（`Some(true)` 是嵌入不是独立文件）→ 嵌入面板冻结、独立场景首次 seek 即被清空/替换 | concat/panes/scene.rs | 极性修正为 `(Some(false), _)`；`(_,None)` 收紧 `(Some(true),None)`；替换分支 bump generation | 4 组合语义写入注释；实机：打开工程面板即自动加载（shot5-7） |
| 相机拖拽的 sync 写进 `mem::take` 占位面板：真实面板永不同步 + 每次 pointer move 泛滥一个全尺寸渲染 | concat/panes/scene.rs drag → studio | `scene_slot_taken` 标志 + queued sync（handle 恢复面板后补）；drag 不再直接 sync | 代码路径唯一化；实机拖动见 P1-3 的手势折叠日志 |

### P1（修复）

| 问题 | 处理 |
|---|---|
| AddSceneClip 先嵌入后验轨，TrackGone 留下无 undo 的嵌入 | 先验轨再嵌入（见上） |
| 展开烘焙丢 stretch（面板与导出不一致） | StretchX/Y 入烘焙 keys；旧路径 `scene_clips` 同批修复说明见 CAMERA_DESIGN |
| 场景手势无结束点：相隔任意久的两次拖拽折成一步 | DragEnded `end_gesture` + 900ms 重开门（scene_gesture_gate） |
| sync 每 tick 全量 clone Project + 解析 JSON | revision 门控；after_change 挂接使暂停态 undo/agent 编辑即时上屏 |
| 独立场景拖拽污染工程场景 / 无场景工程 toast 连刷 | standalone 分支守卫（本地编辑，回到旧行为） |
| sync 替换分支不 bump generation → 过期帧可落新源 | replaced 即 bump |

### P1（豁免，附理由）

| 问题 | 豁免理由 |
|---|---|
| Inspector Camera 行每次拨盘走完整命令链（无 echo/commit 去抖）：每 tick 一次快照 + after_change 全套 | 手势折叠保证 undo 语义正确；单人使用的小工程下开销可感知但不阻塞；正确修复需把场景编辑接进 echo 机制（document 层 patch 语义），风险/收益不匹配。后续若做大工程性能调优再处理（记 P2 队列） |
| API/MCP 的 `flattened_clips` 每帧全量重展开（probe I/O + 深克隆） | 窗口热路径已被 revision 缓存覆盖（P0-2 修复后）；API 回放路径低频。Session 级展开缓存列为下一阶段任务 |

### P2（记录，未阻塞）

- `read_scene` 把解析失败混为 SceneGone（文案误导）；write_scene 的 parse-rewrite 会重塑场景 JSON（未知字段丢弃、数字改写）——头注释与实际不符，后续改为在原 Value 上只补丁 camera.keys 或拆 `SceneBad`。
- 相机行拨动把插值中的其他字段冻结为键（已修为只覆写拨动字段）✅。
- 场景片段的 SetClipSpeed / 头部裁剪语义未定义（展开忽略 speed）——命令层应拒绝或按 speed 换算。
- `duration_frames` 口径：展开用未取整值，整场导出用 ceil——相机尾帧为小数时差 1 帧。
- schema：AddSceneClip 的 scene/easing 字段 skip 后，contract schema 描述了一条不可调用命令——contract.rs 应加注或临时隐藏。
- 合成器"每轨一 clip"语义已写进展开注释；`scene_clips`（整场导出）的 track 分配本来正确。
- Scene 面板图片区域在本机截图会话中显示黑（Frame 计数正确、无错误）：疑为后台渲染时序与截图时机，未复现为功能缺陷；**待人工走查确认**（headless 同场景渲染正确、seam 测试证明 flatten 数据正确）。

## 5. 已知限制

- Windows-only（延续）。
- 一个工程一个场景（顶层 `scene`）；多场景需要设计预留的 `scenes` map + 选择器 UI。
- 场景片段不支持变速；头部裁剪重放语义未定义。
- MCP 暂无场景片段工具（scene_* 系列仍面向文档级场景）。
- 展开按调用发生（无 Session 级缓存）——API 回放路径逐帧重展开（见 P1 豁免）。
- Scene 面板图片黑屏待人工确认（见 P2 末条）。

## 6. 下一阶段

1. **人工走查**：编辑器 scene-demo——拖动相机（Scene 面板 + Camera 标签）、插入时间轴、导出 MP4、undo 链；确认 Scene 面板图片显示。
2. Session 级展开缓存（revision 键控）+ probes 提升；按需接 MCP 回放。
3. 图形图层素材化（rect/ellipse/line → PNG/title 路径）。
4. 多场景：`scenes` map + 场景选择器 + 片段绑定选择。
5. 跟随逐帧解算；相机曲线编辑器（FocusTarget/MoveSequence 可视化）。
6. 清理 stash 的 parallax（公式 `(world-camera)*p` + 测试）。

下一阶段结束按 SOP §12 再启动子代理审查。

## 7. Git

- 阶段实现：`3f4ccbe3`、`67359911`、`a3ca4186`、`ec23f914`
- 审查修复：`7d75fa74`
- 区段修复：`d17dcff7`
- 推送：`origin/main`（见 HANDOVER.md 的最终同步记录）
