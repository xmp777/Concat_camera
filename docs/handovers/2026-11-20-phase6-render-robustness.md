# Phase 6：渲染健壮性与性能

> 日期：2026-11-20
> 状态：已完成（[REVIEW_PASSED]，零严重）
> 对应提交：（见 §7）
> 审查：1 个只读子代理（严苛审查员，git diff 逐行 + 全量测试实跑）

## 1. 本阶段目标

- 消解 Phase 5 遗留的"Scene 面板黑屏"。
- 整场导出与展开路径的烘焙语义对齐（stretch 补烘）。
- 素材 probe 缓存提升到 Session 生命周期，消除逐帧展开的重复文件 I/O。

不在本阶段范围内：Session 级展开结果缓存（revision 键控）、多场景、图形图层素材化。

## 2. 交付内容

- **黑屏根因闭环（三重奏，均已修）**：
  1. 演示/测试素材是**全透明 1×1 PNG**（TINY_PNG）——渲染"正确"但内容透明；
  2. `expand_project_scenes` 的合成 MediaItem **尺寸全 None**——flatten 把 `media.width/height` 交给 ExportClip，合成器按 None 适配 → 空帧（本次修：probe 真实尺寸写入合成项）；
  3. **demo_scene 图层 transform 全 Default(0,0)**——图层摆在世界左上角，相机看世界中心 → 画框外（本次修：图层摆世界中心，照 camera_demo 惯例）。
- `scene_clips`（整场导出）补烘 **StretchX/StretchY** 关键帧（与展开路径同规；resolve.rs 消费链已确认）。
- **probes 缓存提升**：`expand_project_scenes` 第三参由调用方持有；`Session.scene_probes`（Mutex，Session 生命周期）；`request`/`request_for`/监视器三路接入。取舍已记录：素材被外部删除/替换时 monitor（缓存）与导出线程（本地缓存）可能分歧——编辑器内换素材换路径、就地编辑只换像素，均不受影响。
- demo 生成器产出**真实彩色素材**（拷自 camera_demo assets，缺失回退占位）；`examples/scene_demo/` 已用生成器重新生成并提交。
- `PROGRESS.md` 建立（自循环阶段索引）。

## 3. 验证证据

| 项 | 命令 / 操作 | 结果 |
|---|---|---|
| studio / host lib | `cargo test -p concat-studio -p concat-host --lib`（MSVC） | 8 + 172 passed（新增 stretch 烘焙 + probes 缓存行为两个测试） |
| 全量核心 | `cargo test -p concat-project -p concat-scene -p concat-camera -p concat-mcp` | 299 passed |
| 编辑器 | `cargo check -p concat` / `cargo test -p concat --lib` | 通过 / 244 passed |
| headless | `concat-studio render`（彩色素材）+ 帧中心像素抽检 | **RGB(224,89,68)**——非黑，出图 |
| 实机 | 编辑器打开 scene-demo：Scene 面板显示教室背景 + 橙红立绘，"Frame 75/90" 本地映射 | 截图 `docs/assets/scene_clip_colored.png` |

## 4. 子代理审查结论与修复

[REVIEW_PASSED]（零严重）。审查员实跑了三批测试（172+8 / 299 / cargo check），核查了 Send/Sync、锁安全（无嵌套/毒化容忍）、request_for 与 expand 全部调用点更新、stretchX/Y 消费链（resolve.rs 轨道 4/5）、seam 测试有效性（缺盘文件 + probes 预填锁定缓存）、media_size 只缓存成功项。

- 警告 1（示例素材与生成器不一致）→ 已按建议用生成器重新生成示例并提交。
- 警告 2（probe 缓存不失效的 monitor/export 边缘分歧）→ 已在 session.rs 字段注释显式记录取舍。
- 建议 1（stretchY 断言）→ 已补（1.5/0.75 双断言）。
- 建议 2（`&mut **guard` 两级 Deref 可读性 + fallback 死防御）→ 已注明 fallback 为防御分支。
- 建议 3（临时目录清理风格）→ 已统一前置 remove_dir_all。
- 建议 4（concat.lock/out.mp4 删除意图）→ 确认：运行期产物，不再跟踪。

## 5. 已知限制

- API 回放路径（concat-api pixels）仍逐帧全量展开（深克隆 + 重采样）——窗口热路径已被 revision 缓存覆盖，API 侧留待有性能数据再修（Session 级展开结果缓存）。
- probe 缓存不失效的边缘分歧（外部替换素材），见 session.rs 注释。

## 6. 下一阶段

Phase 7：图形图层素材化（rect/ellipse/line → 程序生成 PNG → Image 路径）；随后 Phase 8 多场景、Phase 9 跟随逐帧解算。

## 7. Git

- 阶段实现 + 审查修复：见 `git log`（feat(phase-6)/fix(phase-6)）
- 推送：`origin/main` 已同步（`git status -sb` 无 ahead）
