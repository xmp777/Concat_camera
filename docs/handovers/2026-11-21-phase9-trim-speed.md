# Phase 9：场景片段 trim/speed 语义 + 跟随逐帧映射

> 日期：2026-11-21
> 状态：审查中
> 对应提交：`faf83ce3`
> 审查：1 个只读子代理

## 1. 本阶段目标

- 场景片段尊重 `source_start`（裁头入点）与 `speed`：展开窗口是场景帧区间
  `[source_start×fps, source_start×fps + duration×speed×fps)`，图层与窗口求交
  后经速度因子换回时间轴秒。
- Scene 面板的 playhead 映射携带同一 4 元组（start/duration/source_start/speed），
  面板帧与展开采样帧由同一公式给出。
- 跟随（Follow）逐帧映射：相机键烘焙已按帧采样 `plan_layers`，Follow 目标在
  每个采样点解析——结构上已成立，本阶段确认并锁定。

不在本阶段范围内：编辑器场景绘制 UI、多场景选择器的实机走查、MCP 场景工具。

## 2. 交付内容

- `expand_project_scenes`：窗口从"时间轴秒 1:1"改为"场景帧区间"，图层交集换回
  时间轴秒时除以 `frames_per_sec = fps × speed`；`source_start` 偏移窗口起点。
- `scene_frame_at`（编辑器面板映射）：签名改 4 元组，inside 分支
  `local = source_start×fps + (seconds - start) × fps × speed`；nearest-boundary
  分支的边界帧按 source_start/speed 计算（不再是 0 或 duration×fps 的硬编码）。
- `scene_follow_playhead`（studio）：5 元组 `(id, start, duration, source_start,
  speed)` 传给面板。
- 测试：`a_scene_clip_with_speed_maps_the_window_across_scene_frames`（TrimClip +
  SetClipSpeed 2× → 展开时长减半）；`scene_frame_at` 测试更新为 4 元组。

## 3. 验证证据

| 项 | 命令 | 结果 |
|---|---|---|
| 全 lib 回归 | `cargo test -p concat-scene -p concat-studio -p concat-host -p concat-mcp -p concat --lib` | 全绿（scene 13 / studio 11 / host 182 / mcp 10+6 / editor 245） |
| fmt check | `cargo fmt --check` | 干净 |
| 推送 | `git push origin main` | `90a7bd21..faf83ce3` |

## 4. 子代理审查结论与修复

（待第 1 轮审查报告）

## 5. 已知限制

- scene clip 的 speed 改变展开窗口时，SetClipSpeed 会先按普通 clip 语义调整
  duration（时长减半）；展开按调整后的 duration 求窗口——两条语义链一致。
- 跟随逐帧：图层 transform 在模型中是静态的，逐帧目标解算与静态解算结果
  相同——结构上已按逐帧采样（plan_layers per frame），图层获得自己的关键帧
  轨道后自动生效。

## 6. 下一阶段

（待审查后填）

## 7. Git

- 阶段实现：`faf83ce3`
- 推送：`origin/main` 已同步
