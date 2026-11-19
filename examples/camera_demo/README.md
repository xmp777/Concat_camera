# camera_demo — 2D 场景 + 虚拟摄像机

一个可直接渲染的最小示例：背景 + 两个角色 + 道具，摄像机在 90 帧内推近（zoom 1.0 → 1.6）。

## 运行

```bash
# 直接渲染
cargo run -p concat-studio -- render examples/camera_demo/scene.json examples/camera_demo/out.mp4

# 或通过 MCP（Agent 路径）
# 工具 scene_render: { "scenePath": ".../scene.json", "output": ".../out.mp4" }
```

环境变量：`FFMPEG_DIR` 指向 FFmpeg shared build（`setup-env.ps1` 会设置）。

## 结构

- `scene.json`：`concat-scene` 场景（世界坐标、图层、摄像机关键帧）
- `assets/`：测试用纯色 PNG（背景/角色/道具）
- 输出 `out.mp4`：1920×1080 / 30fps / h264

素材路径相对 `scene.json` 所在目录解析。