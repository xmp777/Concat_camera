# Phase 11：MCP `scene_to_timeline`（Agent 侧时间轴闭环）

> 日期：2026-11-21
> 状态：已完成（审查 2 轮收敛）
> 对应提交：`f596b57d`
> 子代理审查：2 轮（第 1 轮 3×P1 不阻塞；第 2 轮全部复核通过，无 P0）

## 1. 本阶段目标

- Agent 通过 MCP 把一个已存在的场景文件放上打开工程的时间轴，与编辑器
  "插入时间轴"按钮走同一条命令栈（`AddSceneClip`），不绕过 undo/沙箱。
- 不在本阶段范围内：编辑器 UI 变更、展开/渲染语义改动、场景绘制工具。

## 2. 交付内容

- `concat-mcp` 新工具 `scene_to_timeline`（`Action::SceneToTimeline`）：
  读 `scenePath`（可选 `sceneId` 指名多场景文档中的场景，走
  `read_scene_by_id`，与 `scene_render` 同 helper 同语义）→ 构造
  `edit.apply` + `addSceneClip` 请求，经 `backend.call` 进命令栈。参数：
  `path`（工程路径，与其他工具同命名）、`scenePath`（必填）、`sceneId`
  （可选）、`start`（默认 0）、`trackId`（缺省选第一条空轨）、`above`
  （缺省 false）。未知参数报错。
- schema：手写 JSON schema（`path`/`scenePath` required；`sceneId` 描述
  写明 map 落空回退旧键的行为）；samples() 补样例；schema round-trip
  测试与场景工具同组跳过请求构建（构建需真实文件 IO）。
- **审查发现的既有 bug 修复**：
  - `scene_render` 的 `sceneId` 参数实际从未可用——未知参数检查在
    `take_string` 之前执行，任何带 `sceneId` 的调用被拒（Phase 10 引入，
    全库无 e2e 覆盖）。已把 take 前移到 sweep 之前，e2e 补
    scene_render+sceneId 渲染 MP4 断言。
  - `AddSceneClip` 幂等分支（同 id 场景重复插入）返回 `applied: true`
    制造幽灵 undo 步骤并清空 redo（Phase 8a 引入）。改 `applied: false`
    + 单测 `re_inserting_the_same_scene_changes_nothing`（一次 undo 后
    clip 为 0、redo 可恢复——phantom step 下两条断言分别失败）。
- e2e `an_agent_places_a_saved_scene_on_the_timeline_headless`：建工程 →
  建场景（显式 45 帧）→ `scene_to_timeline` → 断言 `createdId`、
  timeline 中 scene 片段 kind/name/start/duration(1.5s)；同文件重复调用
  幂等（`createdId` 缺省、片段仍 1 个）；`scene_render`+`sceneId` 出片；
  未知参数被拒。
- 环境：`setup-env.ps1` 增加 `LLVM_HOME=G:\msys64\ucrt64`（skia-bindings
  找 `bin\clang-cl.exe` + `lib\clang\<版本>`，MSYS2 clang 两样都有）。
  Skia 源码（m153）已预解包进 registry crate 目录（MSYS tar 把 tar 包内
  符号链接物化为普通文件，绕开 Windows 无符号链接特权 os error 1314），
  编辑器 crate 构建恢复可用（`cargo check -p concat` 4m46s 通过）。

## 3. 验证证据

| 项 | 命令 / 操作 | 结果 |
|---|---|---|
| MCP 测试 | `cargo test -p concat-mcp` | 10 lib + 7 e2e 全绿 |
| project 测试 | `cargo test -p concat-project` | 268 lib + 4 doc 全绿 |
| 工作区检查 | `cargo check --workspace --exclude concat --exclude concat-android` | 通过（0 error） |
| 编辑器 crate | `cargo check -p concat`（含 Skia 全量 C++） | 通过（4m46s，1 个上游既有 dead_code 警告） |
| fmt 门禁 | `cargo fmt --check` | 干净 |

## 4. 子代理审查结论与修复

### 第 1 轮（无 P0，3×P1 不阻塞但属"提交后即永久"决策）

| 级别 | 问题 | 位置 | 处理 |
|---|---|---|---|
| P1 | `projectPath` 与全库 `path` 惯例冲突，提交后即永久 | tools.rs schema+解析 | **已改 `path`**（schema/sample/e2e 同步） |
| P1 | 多场景文档静默放置镜像里的第一个场景（`load_scene` 不读 map） | tools.rs scene_to_timeline | **加 `sceneId`，改走 `read_scene_by_id`** |
| P1 | 幂等分支 `applied: true` 制造幽灵 undo 步骤并清空 redo | concat-project scene.rs:70-79 | **改 `applied: false`** + 单测（见 §2） |
| P2 | 同 id 不同值（改文件重插）无测试；`createdId` 为"缺省"非 null；读文件失败错误码语义；`nearest_free_start` 静默挪后未在描述注明 | — | 记录，Phase 12 处置 |

### 第 2 轮（复检 4 项修复全部成立）

| 项 | 结论 |
|---|---|
| scene_render take 前移 | 通过，无新旁路（sweep 完备性反而更干净；`sceneId: null` 从拒变为合法省略） |
| read_scene_by_id 错误路径 | 文案合理；**遗留**：显式 id 在 map 落空时静默回退 legacy 键（helper 既定设计，Phase 12 决定是否改报错） |
| phantom-step 单测 | 确认两条断言分别钉住两个症状（undo 弹幽灵快照 / redo 被清） |
| 新引入 P0 | 无（沙箱模型、serde 线格式、sweep 完备性均不变） |

**遗留（Phase 12）**：(1) sceneId 显式 id map 落空的静默回退——建议改报错
或保持文档注明；(2) `read_scene_by_id` map-hit 路径补直接测试（双场景
文档按 id 渲染/放置）；(3) 同 id 不同值重插路径补测。

## 5. 已知限制

- 工具返回 `edit.apply` 摘要（含 `createdId`），不主动回读 timeline；
  Agent 用 `timeline_read` 复核（e2e 即如此）。
- `sceneId` 省略且文档是 map-only（无 legacy 镜像）时报
  "场景 JSON 无效：missing field `id`"——文案欠准但此类文档本代码库不产生。
- 编辑器 crate 有上游既有 Slint 警告（modal.slint 弃用属性、未使用组件），
  非本阶段引入，UI 阶段清理。

## 6. 下一阶段

Phase 12：跟随逐帧解析深化 / `scenes` map 读取迁移（编辑器与 studio 全面
走 map 而非旧 `scene` 单键）+ 上述 3 项遗留 / parallax 死管道清理。

## 7. Git

- 阶段实现 + 审查修复：`f596b57d`
- 推送：`origin/main` 已同步（`d8b875a9..f596b57d`）
