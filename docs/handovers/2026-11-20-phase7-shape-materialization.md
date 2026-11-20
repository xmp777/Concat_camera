# Phase 7：图形图层素材化

> 日期：2026-11-20
> 状态：审查中
> 对应提交：（审查通过后填）
> 审查：1 个只读子代理 + 主会话自查（沙箱回归 / NaN 防护，均在审查报告前主动修复）

## 1. 本阶段目标

- 场景图形图层（rect/ellipse/line）不再被展开跳过：**形状定义是唯一事实源**（scene JSON），渲染/预览时按内容素材化为 PNG 并交给 Concat 合成器，与图片图层完全同规。
- MCP 闭环：Agent 经 `scene_add_layer(shape)` + `scene_render` 画出图形。

不在本阶段范围内：渐变/文字/圆角、编辑器画布上绘制图形（UI 侧）、多场景、跟随逐帧解算。

## 2. 交付内容

- **模型**（concat-scene）：`ShapeKind`（rect/ellipse/line）+ `ShapeData {kind,width,height,fill,stroke,stroke_width}`（RGBA 0..=1；serde default 兼容旧文档）+ `SceneLayer.shape: Option<ShapeData>`；`cache_key()` 内容指纹（FNV 文件名）；`has_non_finite` 覆盖 shape 数值。
- **素材化**（concat-studio）：`materialize_shape`（tiny-skia：rect 填充+内描边、椭圆四段贝塞尔、line；透明底；钳 1..=4096px）+ `layer_asset_path`（asset 直用；shape → `cache/scene-shapes/{指纹}.png`，工程目录不可写时退临时目录——MCP 只读场景语义保持）；三处接线（`scene_clips`/`scene_clips_at`/`expand_project_scenes`）。
- **MCP**：`scene_add_layer` 加 `shape` 参数（schema + serde 解析；kind=shape 无 figure 报错）；scene 摘要带 shape；新 e2e `an_agent_draws_a_shape_layer_and_renders_it`。
- 测试 +6：serde 往返/指纹稳定、像素断言（rect 中心 fill、椭圆角透明）、展开复用 PNG、e2e 全链。

## 3. 验证证据

（审查回归后填终版）

## 4. 子代理审查结论与修复

（待审查报告）

### 主会话自查（审查前主动修复）

- **MCP 沙箱回归**：shape 素材化让 `scene_render` 会写 scene_dir/cache——曾临时给 scene_render 加 scene_dir 可写校验，后改为 temp 回退（语义保持"只写 output"），e2e（读仓库场景写 home）不再被拒。
- **NaN 防护**：`Scene::has_non_finite` 补 shape 的 width/height/stroke_width。

## 5. 已知限制

（待审查后填）

## 6. 下一阶段

（待审查后填）

## 7. Git

（待推送后填）
