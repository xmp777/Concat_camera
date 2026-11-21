# Phase 13：编辑器插入路径形状物化 + legacy 镜像统一

> 日期：2026-11-21
> 状态：已完成（审查 1 轮通过 + P2 顺手修复）
> 对应提交：（提交后补）
> 子代理审查：1 轮（无 P0/P1，4 个审查重点全部通过）

## 1. 本阶段目标

- 编辑器"插入时间轴"按钮获得与 MCP `scene_to_timeline` 相同的形状资产
  搬运语义（Phase 12 只修了 MCP 路径，编辑器路径同缺口）。
- legacy `scene` 镜像写路径统一：重插路径也刷新镜像（原"只在缺失时写"
  与 `write_scene` 总刷新不对称）。
- 不在本阶段范围内：图片图层资产迁移、expand 两类回退区分、MCP 源缺失
  warning。

## 2. 交付内容

- **搬运逻辑下沉 concat-studio**：`carry_shape_assets(scene, scene_dir,
  project_dir)` 与 `materialized_shape_file(asset)`（指纹布局判定）成为
  studio 公共 API——MCP 与编辑器共用同一实现，写入方（沙箱根、写校验）
  由调用方把关。MCP 的 wrapper 只剩 ensure_writable + 错误映射；表驱动
  对抗测试随函数移至 studio（mcp 留一条契约性断言），另加真实复制测试
  `carrying_moves_shape_pngs_beside_the_scene_into_the_project`（复制逐
  字节一致 / 幂等不覆盖 / 缺源静默跳过）。
- **编辑器接线**（`concat/src/panes/scene.rs add_to_timeline`）：独立
  文件（state.path 为 Some）插入前搬运 `state.dir` →
  `studio.project_folder`；嵌入场景（path None）已在工程内，跳过；搬运
  失败进状态行并中止插入。
- **legacy 镜像统一**（concat-project AddSceneClip）：map 写入后总是
  镜像到字典序第一条（原"缺失才写"）——旧读者看到的场景与 map 首条
  永不漂移。

## 3. 验证证据

| 项 | 命令 / 操作 | 结果 |
|---|---|---|
| studio 测试 | `cargo test -p concat-studio --lib` | 15 lib 全绿（含 2 个新搬运测试） |
| project 测试 | `cargo test -p concat-project` | 269 lib + 4 doc 全绿 |
| MCP 测试 | `cargo test -p concat-mcp` | 11 lib + 7 e2e 全绿 |
| 编辑器测试 | `cargo test -p concat --lib` | 245 全绿 |
| fmt 门禁 | `cargo fmt --check` | 干净 |

## 4. 子代理审查结论与修复

第 1 轮：**无 P0/P1 阻塞**，4 个审查重点全部通过。关键核验：

- `state.dir` 不会漂移（只在 open()/sync replaced 分支赋值，Arc 不可变，
  save 永远写回 state.path，无 Save-As）；
- `session.is_some()` 而 `project_folder` 为 None 不可能（同点成对赋值，
  全工程唯一 session 赋值处）——接线里的 None 分支是防御性死代码；
- 搬运目标 `project_folder` 与渲染解析根 `resolve(scene_dir, asset)` 同根；
- 幂等重插不被资产问题阻塞（缺源/已有目标跳过；只有真实 I/O 故障才
  中止插入）；
- 镜像统一后幂等早退仍在镜像刷新之前（同场景重插 `applied:false`、
  零文档改动）；字典序首条与 `write_scene`/`read_scene_by_id(None)` 三处
  语义对齐。

P2 顺手修复（3 项）：apply 注释收窄（幂等早退不修复外部旧文档陈旧镜像，
下次真实写入自愈）；carry 的 rename 失败清理 staged `.tmp`（对齐
materialize_shape 纪律）；carry 文档注释视角改为"MCP 沙箱写根 / 编辑器
本地信任"。审查代理本机沙箱受 ffmpeg/pkg-config 环境限制，project 269+4
与 fmt 独立复跑验证，studio/mcp/editor 基于静态核验 + 本会话全绿采信；
P2 修复后本会话复跑 studio 15 / project 269 / fmt 全绿。

## 5. 已知限制

- carry 源缺失仍是静默跳过（MCP 与编辑器一致）——warning 化待后续。
- 图片图层（kind=image）相对 asset 不随嵌入迁移。
- expand 的 id 落空静默顶替第一场景（导出容错）——两类回退区分待
  架构评审。

## 6. 下一阶段

（待填：图片图层 carry / expand 区分 / 跟随逐帧 / parallax 处置）

## 7. Git

- 阶段实现 + 审查修复：（提交后补）
