# Phase 12：scenes map 读取迁移 + 形状资产随嵌入迁移

> 日期：2026-11-21
> 状态：已完成（审查 2 轮收敛）
> 对应提交：`0639f483`
> 子代理审查：2 轮（第 1 轮无 P0、1×P1 提交前补齐；第 2 轮全部复核通过）

## 1. 本阶段目标

- 场景读取全面走 Phase 8 的 `scenes` map；显式 id 落空不再静默回退。
- 补齐 Phase 11 审查遗留的测试缺口（map-hit 路径、同 id 不同值重插）。
- 不在本阶段范围内：编辑器 UI 变更、expand 渲染容错语义重构、图片图层
  资产迁移。

## 2. 交付内容

- **`read_scene_by_id` 语义收紧**（concat-studio）：
  - `Some(id)` + map 存在 → 必须命中，落空报错（原为静默回退 legacy
    `scene` 键——放置是有副作用的操作，拼错 id 拿到错误场景比报错糟）。
  - `Some(id)` + 无 map → legacy 字段，且场景 id 必须与请求一致。
  - `None` → map 第一条（serde_json BTreeMap 字典序，确定性）→ legacy →
    裸场景。
  - 新增 `SCENES_KEY` 常量；`expand_project_scenes` 字面量改用常量
    （**渲染期静默回退保留**：导出必须出片，与读取路径有意不同）。
- **新发现缺口修复：形状资产随嵌入迁移**。场景文件的形状 PNG 物化在
  场景文件旁（`assets/scene-shapes/<指纹>.png` 相对路径），嵌入工程后
  工程侧渲染按工程目录解析 → 预览/导出**静默丢形状**（此前无任何端到端
  测试覆盖"场景 → 工程 → 渲染"链路）。修复：`scene_to_timeline`（Agent
  侧写入点）构建请求前 `carry_shape_assets` 把 PNG 复制进工程目录：
  - `ensure_writable(project_dir)` 沙箱先行；
  - `materialized_shape_file` 只接受精确布局
    `assets/scene-shapes/<16hex>.png`（components 匹配 + 指纹校验），
    `..`/绝对路径/子目录/任意文件名全拒（审查子代理用 Windows rustc
    探针实测 18 种对抗输入，无逃逸）；
  - 源缺失或目标已存在则跳过（内容寻址，同名即同内容）；
  - 复制走 tmp+rename（与 materialize_shape 同法，撕裂文件不会因
    存在性检查被永久当真）。
- **测试补齐**：
  - studio：`a_named_scene_id_answers_from_the_scenes_map_or_errs`（map
    命中不看 legacy、落空报错、None 取字典序首条）、
    `a_legacy_document_agrees_with_the_requested_id`（legacy 同 id/错 id
    /未点名/裸场景）。
  - project：`re_inserting_a_changed_scene_updates_the_map_and_adds_a_clip`
    （同 id 不同值 → applied true、map 内容替换、新 clip、旧 clip 重绑）。
  - mcp：`the_asset_carry_takes_fingerprint_pngs_alone`（表驱动对抗输入）。
  - e2e：双场景走线——第二场景（90 帧+形状）放置 → project_save → 读
    concat.json → **carry 断言**（PNG 在工程目录）→ `scene_render` 按
    map 中的 sceneId 渲染 → probe 时长 ≈3.0s（第二场景自己的 90 帧）→
    map 落空 sceneId 报错。

## 3. 验证证据

| 项 | 命令 / 操作 | 结果 |
|---|---|---|
| MCP 测试 | `cargo test -p concat-mcp` | 11 lib + 7 e2e 全绿 |
| project 测试 | `cargo test -p concat-project` | 269 lib + 4 doc 全绿 |
| studio 测试 | `cargo test -p concat-studio --lib` | 13 lib 全绿 |
| host lib | `cargo test -p concat-host --lib` | 182 全绿 |
| fmt 门禁 | `cargo fmt --check` | 干净 |
| 预先存在失败 | `cargo test -p concat-host --test export` | 3 失败（`one_clip_exports_whole_at_every_rate` 等，音频时长漂移 ~70ms）——**已验证在 HEAD（未含本阶段改动）同样失败**，环境性，不属本阶段 |

## 4. 子代理审查结论与修复

### 第 1 轮（无 P0）

| 级别 | 问题 | 处理 |
|---|---|---|
| P1-a | carry 修复无测试钉住（e2e 时长断言对 carry 不敏感；新函数无单测） | **提交前补齐**：e2e carry 断言 + 表驱动单测 |
| P1-b | expand 静默回退的理由 | 成立（导出容错），落空时跳 clip/记日志留 P2 跟进 |
| P2-1 | fs::copy 非原子 | **已改 tmp+rename** |
| P2-2 | 源 PNG 缺失静默 continue（与要消灭的现象同类） | 已知限制，见 §5 |
| P2-3 | 工具描述未提 carry | **已补** |
| P2-5 | legacy 镜像重插后不刷新（scene.rs 只缺才写 vs write_scene 总刷新） | 记录，Phase 13 |
| P2-6 | 图片图层（kind=image）相对 asset 同样不随嵌入迁移 | 记录，Phase 13 |

沙箱实测（审查代理）：18 种对抗 asset 形态（反斜杠、`..`、UNC、drive-relative、
`\\?\`、子目录等）全部 confined 或拒绝；反斜杠形式通过检查但源/目标均在
沙箱内（Windows 下 `\` 即分隔符，语义一致）。

## 5. 已知限制

- 源 PNG 不在场景文件旁（场景被单独移动/分享）时静默不复制——与
  "渲染静默丢形状"同类，待 Phase 13 给 warning 或报错。
- 编辑器"插入时间轴"按钮（`concat/src/panes/scene.rs add_to_timeline`）
  走 Studio 直接 apply，不经过 MCP 工具——独立打开的场景文件嵌入后
  同样存在形状资产缺口，待编辑器阶段补同款物化。
- 图片图层相对 asset 不 carry（指纹命名只覆盖形状）。
- legacy `scene` 镜像在重插后保持旧值（map 优先读取下无实际影响）。

## 6. 下一阶段（Phase 13 候选）

- 编辑器插入路径的形状资产物化（与 MCP carry 同语义）。
- 图片图层资产迁移 / carry 源缺失 warning。
- expand 两类回退区分（clip 无 id 合理回退 vs id 落空跳过+日志）。
- legacy 镜像写路径统一（write_scene 语义应用到重插路径）。
- 跟随逐帧解析深化 / parallax 死管道处置（架构评审）。

## 7. Git

- 阶段实现 + 审查修复：`0639f483`
- 推送：`origin/main` 已同步（`01971438..0639f483`）
