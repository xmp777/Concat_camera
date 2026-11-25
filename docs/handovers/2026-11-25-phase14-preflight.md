# Phase 14 前置基线交接

> 日期：2026-11-25
> 状态：已完成
> 对应提交：本轮 `feat(phase-14): establish walkthrough baseline and fix mux audio cutoff`
> 子代理审查：两轮独立只读审查；第 2 轮 [REVIEW_PASSED]

## 1. 本阶段目标

- 建立可复现的 Phase 14 编辑器走查基线，并修复历史 host export 音频时长漂移。
- 不在本阶段范围内：编辑器实机走查、图片资产 carry、expand 回退统一、相机死管道清理、跟随逐帧与 parallax 实现。

## 2. 交付内容

- `.walkthrough/` 保留最小 fixture：`opening.json`、`assets/scene-shapes/61808269636e7950.png`、`走查工程/concat.json`。
- `.gitignore` 排除 recents、opened、lock、工程生成 assets 和走查输出，避免把机器状态与运行产物入库。
- `HANDOVER.md` 重写为当前 Phase 0–13 完成、Phase 14 前置基线完成的交接；SOP 修正根目录交接入口与阶段状态。
- `concat-media::audio::mux` 的 `Feed.end` 改为单调最大值。H.264 B 帧按解码顺序写入时，末尾包的 PTS 可能小于已写入包；旧逻辑用末尾包覆盖流结尾，导致按较短流截止时提前截断音频。

## 3. 验证证据

| 项 | 命令 / 操作 | 结果 |
|---|---|---|
| 格式 | `cargo fmt --all -- --check` | 通过 |
| 静态检查 | `cargo check -p concat-media`（MSVC） | 通过 |
| 音频单元测试 | `cargo test -p concat-media --lib audio::` | 36 passed |
| Host 导出回归 | `cargo test -p concat-host --test export` | 27 passed |
| 历史失败复现 | `one_clip_exports_whole_at_every_rate` | 修复前 24 fps 稳定得到 5.931s；修复后七种帧率全通过 |
| 包时间戳定位 | `ffprobe` 检查源 AAC、混合流与 24 fps 成片 | 成片最后一个完整 AAC 包停在 5.909333s，解码时长 5.931s |
| Fixture 指纹 | `ShapeData::cache_key` + `concat_studio::fnv1a` | `61808269636e7950`，与文件名一致 |
| 差异空白 | `git diff --check HEAD` | 通过 |

## 4. 子代理审查结论与修复

### P0（必须修复）

| 问题 | 位置 | 修复 | 回归测试 |
|---|---|---|---|
| 无 | — | — | — |

### P1（修复或写豁免）

| 问题 | 位置 | 处理 | 理由 |
|---|---|---|---|
| mux 把 B 帧重排后的末尾 PTS 当作流结尾 | `src/crates/concat-media/src/audio.rs:1532` | `Feed.end` 只增不减 | 保持较短流截止语义，同时尊重最大显示结束时间 |
| 交接文档仍把音频漂移写成待复现 | `HANDOVER.md` | 记录根因、症状与修复 | 与已验证状态一致 |
| SOP 入口指向不存在的 `docs/HANDOVER.md` | `docs/SOP.md:11` | 改为根目录 `HANDOVER.md` | 后续 Agent 可按路径读取 |
| `git diff --check` 因 Markdown 行尾空格失败 | `HANDOVER.md:3-5` | 移除行尾空格 | 恢复差异门禁 |

### P2 / 后续

- 审查建议补充 `Feed.end` 的纯单元边界测试；现有七帧率 host 回归已能捕获原始缺陷，本阶段不扩大改动。
- 目录地图中 `HANDOVER` 的归属已在第 2 轮审查后同步修正。

## 5. 已知限制

- 本机 `concat-media` 全量测试有两项既有硬件编码码率断言失败：H.264 Media Foundation 平均码率 393/1000 kb/s；HEVC NVENC CBR 1359/1000 kb/s。两轮独立复跑结果一致，与本次 mux 改动无关。
- Phase 14 尚未执行真实编辑器交互；当前只完成 fixture、自动回归与文档基线。

## 6. 下一阶段

1. 从 `.walkthrough/走查工程/concat.json` 的副本启动编辑器，打开 `.walkthrough/opening.json`。
2. 验证形状 PNG 字节级搬运、金色 Scene 片段、map/legacy 镜像、undo/redo、幂等重插与保存重开。
3. 截图写入 `docs/assets/`，再走独立审查、回归、交接、提交和推送闭环。

## 7. Git

- 实现提交：见本轮 `feat(phase-14): establish walkthrough baseline and fix mux audio cutoff`。
- 审查修复：包含在同一阶段提交中。
- 推送：`origin/main`。
