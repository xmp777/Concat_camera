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
- [x] **Phase 10：MCP sceneId 适配 + 编辑器实机走查**（2026-11-21，已推送 6d3e3e6e/18c795e6）
  - [x] 10.1 `scene_render` 加 `sceneId` 参数（多场景文档按 id 渲染指定场景；schema 文档化）
  - [x] 10.2 编辑器重建 + 实机走查：多场景切换/双金色片段/面板 Frame 0/90 同步正确（截图 `docs/assets/phase10_multiscene_walkthrough.png`）
  - [x] 10.3 门禁：MCP 10 lib + 6 e2e 全绿，MSVC build 通过
- [x] **Phase 11：MCP `scene_to_timeline`（Agent 侧时间轴闭环）**（2026-11-21，2 轮审查收敛，详见 `docs/handovers/2026-11-21-phase11-scene-to-timeline.md`）
  - [x] 11.1 新工具 `scene_to_timeline`：场景文件 → `AddSceneClip` 请求 → 命令栈（与编辑器插入按钮同路径）；可选 `sceneId` 走 `read_scene_by_id`
  - [x] 11.2 审查修复 3×P1：参数定名 `path`（对外接口定版前零成本窗口）；幂等分支 `applied: false`（幽灵 undo 步骤，Phase 8a 既有 bug）+ 单测；sceneId 对齐
  - [x] 11.3 自查 P0：`scene_render` 的 `sceneId` 因参数检查顺序从未可用（Phase 10 引入、无 e2e）——take 前移 + e2e 补断言
  - [x] 11.4 环境：LLVM_HOME 接通 MSYS2 clang（Skia/编辑器构建恢复）；Skia m153 源码预解包绕开符号链接特权限制
  - [x] 11.5 门禁：MCP 10 lib + 7 e2e；project 268 lib + 4 doc；workspace check（除 editor/android）+ `cargo check -p concat`（Skia 全量）通过；fmt 干净
- [x] **Phase 12：scenes map 读取迁移 + 形状资产随嵌入迁移**（2026-11-21，2 轮审查收敛，详见 `docs/handovers/2026-11-21-phase12-scenes-map-migration.md`）
  - [x] 12.1 `read_scene_by_id` 收紧：显式 id 在 map 落空改报错（原静默回退 legacy 键）；无 map 的 legacy 文档 id 必须一致；None 取 map 字典序首条
  - [x] 12.2 新发现缺口修复：形状 PNG 物化在场景文件旁、嵌入工程后预览/导出静默丢形状——`scene_to_timeline` 增 `carry_shape_assets`（ensure_writable 先行 + 精确指纹布局校验 + tmp+rename），沙箱边界经审查代理 18 种对抗输入实测无逃逸
  - [x] 12.3 测试补齐：studio map-hit/legacy 双测、project 同 id 不同值重插测、mcp 表驱动对抗单测、e2e 双场景走线（carry 断言 + map-hit 渲染时长断言 + 落空报错）
  - [x] 12.4 门禁：MCP 11 lib + 7 e2e；project 269 lib + 4 doc；studio 13 lib；host 182 lib；fmt 干净（host export 3 失败为预存在环境性问题，已验证 HEAD 同样失败）
- [x] **Phase 13：编辑器插入路径形状物化 + legacy 镜像统一**（2026-11-21，1 轮审查通过 + P2 顺手修复，详见 `docs/handovers/2026-11-21-phase13-editor-carry.md`）
  - [x] 13.1 搬运下沉 concat-studio（`carry_shape_assets`/`materialized_shape_file` 公共 API，MCP wrapper 只剩沙箱校验；真实复制测试逐字节/幂等/缺源三段）
  - [x] 13.2 编辑器"插入时间轴"按钮接线：独立文件插入前搬运 PNG 进工程目录，失败进状态行中止；嵌入场景跳过
  - [x] 13.3 legacy `scene` 镜像统一：AddSceneClip 移动 map 时总刷新镜像（对齐 write_scene；幂等早退不盲修外部陈旧镜像，注释写明）
  - [x] 13.4 门禁：studio 15 lib；project 269 lib + 4 doc；MCP 11 lib + 7 e2e；editor 245 lib；fmt 干净

## 当前队列（Phase 14 候选）

- [ ] 图片图层相对 asset 随嵌入迁移 / carry 源缺失 warning 化
- [ ] expand 两类回退区分（clip 无 id 合理回退 vs id 落空跳过+日志，需日志设施决策）
- [ ] 跟随逐帧解析深化（图层关键帧轨道就绪后自动生效，见 Phase 9 交付）
- [ ] parallax/plan_frame_with_camera 死管道处置（待架构评审）
- [ ] 编辑器 UI：Scene 面板实机走查（13.2 接线后插入含形状场景的端到端确认）

## 卡点记录

（无）——Phase 7 熔断已恢复，Phase 9/11/12/13 审查收敛。
