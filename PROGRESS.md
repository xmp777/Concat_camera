# PROGRESS — Concat_camera（2D Mini Studio）

> 阶段自循环索引。每阶段走 SOP §12 闭环：实现 → 子代理只读审查 → 修复复检（≤3 轮）→ 回归 → 更新本文件 → push。
> 详细交付证据在 `docs/handovers/`；总需求在 `docs/PROJECT_BRIEF.md`（原 `项目任务书.md`）。

## 已完成（历史，见 docs/handovers/）

- [x] Phase 0：仓库审计 + 摄像机/场景数学（`concat-camera`）+ `FramePlan.camera` 钩子
- [x] Phase 1：Headless 场景渲染（`concat-scene`/`concat-studio` + 示例 MP4）
- [x] Phase 2：运镜（相对/跟随/组合烘焙）
- [x] Phase 3：MCP（scene_create/add_layer/camera_move/get/render + e2e）
- [x] Phase 4：Slint 场景视口 + 编辑器 Scene 面板（原生 playhead 驱动）
- [x] Phase 5：Scene 时间轴片段 + Inspector Camera 区 + 预览/导出统一展开（2026-11-20）

## 当前队列

- [x] **Phase 6：渲染健壮性与性能**（2026-11-20，[REVIEW_PASSED]）
  - [x] 6.1 黑屏根因三重奏闭环：透明素材 / 合成 MediaItem 尺寸 None / demo 图层摆位 (0,0)——修复后实机 Scene 面板出图（彩色截图 `docs/assets/scene_clip_colored.png`）
  - [x] 6.2 `scene_clips` 补烘 StretchX/Y（含测试双断言）
  - [x] 6.3 probes 缓存提升到 Session（`expand_project_scenes` 三参签名；三路调用点接入；取舍已记录）
- [x] **上游同步**（2026-11-20）：合并 upstream 178eb64c（concat-doc 文档模型 + concat-sched 调度器 + corpus 测试），适配 host 新 submit API，合并后全量回归绿
- [ ] ⚠️ **Phase 7：图形图层素材化——熔断停机（第 3 轮审查未过）**
  - 已落地：ShapeData 模型 + materialize_shape 像素级正确 + e2e 全链路 + S1/W1/W2/W3/W4/W6/N3 修复（3 轮审查逐轮收敛，全部有测试）
  - **卡点 N1【严重】**：`scene_add_layer` 用未消毒的 `layer.id` 构造形状 PNG 文件名 → `id = "..\..\evil"` 或绝对路径可穿越写根落盘（写盘先于校验、词法 strip_prefix 防不住 `..`）。探针实证于第 3 轮审查
  - **审查员给出的通过路径**：① id 消毒（只留 `[A-Za-z0-9._-]`）或改纯 hash 命名；② 校验先于写盘（canonicalize 后比对）；③ 补拒绝性测试（恶意 id → 拒绝且磁盘无文件）。修掉 N1 即 [REVIEW_PASSED]，建议项无需开轮
  - 下一步：修复 N1（约 30 分钟）→ 第 4 轮复审确认 → 收尾推送。**待用户确认是否继续**
- [ ] Phase 8：多场景（工程 `scenes` map + 场景选择器 + 片段绑定）
- [ ] Phase 9：跟随逐帧解算 + 场景片段 speed/裁剪语义

## 卡点记录

- **2026-11-21 Phase 7 熔断**：3 轮审查-修复循环用尽。轮 1：S1（schema/serde 字段名静默丢描边）+ S2（渲染路径写盘越沙箱）；轮 2：S2 未真正落地（temp 回退仍是渲染期写盘）+ W3 颜色修复丢失；轮 3：S2 核心落地（渲染零写盘）+ W3 真修，但新写入点引入 N1（layer.id 路径穿越）。工作区含 N1 修复所需的全部上下文；相关提交 dcdaa9eb、79e40378 均已本地提交（Phase 6 与上游合并已推送，Phase 7 改动待 N1 清零后一并推送）。

## 卡点记录

（无）
