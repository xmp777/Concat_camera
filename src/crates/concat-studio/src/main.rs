// SPDX-License-Identifier: AGPL-3.0-or-later
// concat-studio：把 Scene JSON 渲染为 MP4

use std::path::PathBuf;
use std::process::ExitCode;

use concat_studio::{RenderOptions, load_scene, render_scene, save_scene};

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let Some(command) = args.next() else {
        usage();
        return ExitCode::from(2);
    };
    if command == "save" {
        let Some(input) = args.next() else {
            usage();
            return ExitCode::from(2);
        };
        let Some(output) = args.next() else {
            usage();
            return ExitCode::from(2);
        };
        let input = PathBuf::from(input);
        let scene = match load_scene(&input) {
            Ok(scene) => scene,
            Err(error) => {
                eprintln!("{error}");
                return ExitCode::FAILURE;
            }
        };
        let output_path = PathBuf::from(&output);
        return match save_scene(&output_path, &scene) {
            Ok(()) => {
                println!("已保存工程场景：{output}");
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("{error}");
                ExitCode::FAILURE
            }
        };
    }
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
    let scene = match load_scene(&scene_path) {
        Ok(scene) => scene,
        Err(error) => {
            eprintln!("{error}");
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
    eprintln!(
        "用法：\n  concat-studio render <scene.json> <output.mp4> [--crf 20] [--preset medium]\n  concat-studio save <scene.json> <project.json>"
    );
}
