// SPDX-License-Identifier: AGPL-3.0-or-later
// concat-studio：把 Scene JSON 渲染为 MP4

use std::path::PathBuf;
use std::process::ExitCode;

use concat_scene::Scene;
use concat_studio::{RenderOptions, render_scene};

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let Some(command) = args.next() else {
        usage();
        return ExitCode::from(2);
    };
    if command != "render" {
        usage();
        return ExitCode::from(2);
    }
    let Some(scene_path) = args.next() else {
        usage();
        return ExitCode::from(2);
    };
    let Some(output) = args.next() else {
        usage();
        return ExitCode::from(2);
    };

    let mut options = RenderOptions {
        output: PathBuf::from(output),
        ..Default::default()
    };
    while let Some(flag) = args.next() {
        match flag.as_str() {
            "--crf" => {
                if let Some(value) = args.next() {
                    options.crf = value.parse().unwrap_or(options.crf);
                }
            }
            "--preset" => {
                if let Some(value) = args.next() {
                    options.preset = value;
                }
            }
            other => {
                eprintln!("未知参数：{other}");
                usage();
                return ExitCode::from(2);
            }
        }
    }

    let scene_path = PathBuf::from(scene_path);
    let text = match std::fs::read_to_string(&scene_path) {
        Ok(text) => text,
        Err(error) => {
            eprintln!("无法读取场景 {}：{error}", scene_path.display());
            return ExitCode::FAILURE;
        }
    };
    let scene: Scene = match serde_json::from_str(&text) {
        Ok(scene) => scene,
        Err(error) => {
            eprintln!("场景 JSON 无效：{error}");
            return ExitCode::FAILURE;
        }
    };
    let scene_dir = scene_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."));

    let mut progress = |frame: i64, total: i64, stage: &'static str| {
        if total > 0 {
            eprintln!("{stage}: {frame}/{total}");
        } else {
            eprintln!("{stage}");
        }
    };

    match render_scene(&scene, scene_dir, &options, &mut progress) {
        Ok(()) => {
            println!("已导出：{}", options.output.display());
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

fn usage() {
    eprintln!("用法：concat-studio render <scene.json> <output.mp4> [--crf 20] [--preset medium]");
}
