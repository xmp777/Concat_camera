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
- [ ] **Phase 7：图形图层素材化**（rect/ellipse/line → 程序生成 PNG → Image 路径，进时间轴与导出）
- [ ] **Phase 8：多场景**（工程 `scenes` map + 场景选择器 + 片段绑定）
- [ ] **Phase 9：跟随逐帧解算**（目标图层关键帧动画完成后的逐帧目标点）+ 场景片段 speed/裁剪语义

## 卡点记录

（无）
