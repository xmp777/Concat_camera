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
- [x] **Phase 7：图形图层素材化**（2026-11-21，熔断后经外部审计并入第 4 轮修复，全部收敛）
  - [x] 7.1 ShapeData 模型 + materialize_shape 像素级正确（e2e：Agent 四工具调用 → MP4）
  - [x] 7.2 三轮内部审查收敛 + 外部审计（DeepSeek）并入：N1 路径穿越（纯 hash 命名）、播放回写冻结、同 id 场景更新、轨插入位置、最后一帧 viewport 保护、cargo fmt 门禁、运行期产物 untrack
  - [x] 7.3 熔断后恢复：外部审计与内部审查双通道清零严重项；全量回归绿（fmt check 干净）
- [x] **Phase 8a：多场景容器**（2026-11-21，模型层完成）
  - [x] 8a.1 `scenes` map（顶层键，按场景 id 存取）+ 旧单键 `scene` 读取兼容
  - [x] 8a.2 AddSceneClip 支持多场景共存（每片段绑定自己的场景）
  - [x] 8a.3 展开按 clip.scene_id 从 map 取对应场景（含 per-clip viewport/fps/背景色）
  - [x] **Phase 8b：场景选择器 UI**（2026-11-21，已推送 2b8a3a8d）
  - [x] 8b.1 Scene 面板切换：Select 下拉列出工程场景，切到 id 对应场景并重渲染
  - [x] 8b.2 scene.pick i18n 键（26 locale）
- [x] **Phase 9：场景片段 trim/speed 语义 + 跟随逐帧映射**（2026-11-21，[REVIEW_PASSED]）
  - [x] 9.1 展开窗口改为场景帧区间（source_start×fps 起，speed 缩放流速）；面板映射同公式（4 元组）
  - [x] 9.2 跟随逐帧确认：相机键烘焙已按帧采样 plan_layers，目标图层逐帧解算结构成立
  - [x] 9.3 外部审查 W1/W2/W3 修复：判别性 head-trim/speed 测试、死变量清理、日志 untrack
- [x] **严格审查：重复造轮子/绕弯路盘点**（2026-11-21，已推送 b74364f4）
  - [x] materialize_shape 委托 render_shape（tiny-skia 依赖移除；与编辑器 shape 片段共用栅格化）
  - [x] plan_frame_with_camera 死管道确认（记录待架构评审处置）
  - [x] SCENE_KEY 双定义 / Easing 双轨确认并记录理由
- [ ] Phase 10：MCP 场景片段工具 + 编辑器实机走查（多场景切换/shape 渲染/速度导出）

## 卡点记录

（无）——Phase 7 熔断已恢复，Phase 9 严格审查 [REVIEW_PASSED]。
