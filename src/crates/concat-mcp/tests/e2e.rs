// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! The MCP server as an agent meets it: a process spoken to over its
//! standard input and output, from the handshake through edits, a frame
//! and an export to the end of the session - headless, and through a
//! running server as the window's Automation page runs one.

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use serde_json::{Value, json};

/// The server process and its two pipes.
struct Agent {
    child: Child,
    input: ChildStdin,
    lines: mpsc::Receiver<Value>,
    next: i64,
}

impl Agent {
    /// The MCP server with a scratch folder for a home, so it reads and
    /// writes nowhere real.
    fn spawn(home: &Path, mode: &str, extra: &[(&str, String)]) -> Agent {
        let mut command = Command::new(env!("CARGO_BIN_EXE_concat-mcp"));
        command
            .arg(mode)
            .env("HOME", home)
            .env("USERPROFILE", home)
            .env("APPDATA", home.join("AppData"))
            .env("LOCALAPPDATA", home.join("LocalAppData"))
            .env("XDG_CONFIG_HOME", home.join(".config"))
            .env("XDG_DATA_HOME", home.join(".local/share"))
            .env_remove("CONCAT_API_ADDRESS")
            .env_remove("CONCAT_API_TOKEN")
            .env_remove("CONCAT_MCP_ROOTS")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        for (key, value) in extra {
            command.env(key, value);
        }
        let mut child = command.spawn().expect("the MCP server starts");
        let input = child.stdin.take().expect("stdin");
        let output: ChildStdout = child.stdout.take().expect("stdout");
        let (send, lines) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(output).lines() {
                let Ok(line) = line else { return };
                let value: Value = serde_json::from_str(&line)
                    .unwrap_or_else(|_| panic!("stdout carries only JSON-RPC: {line:?}"));
                if send.send(value).is_err() {
                    return;
                }
            }
        });
        Agent {
            child,
            input,
            lines,
            next: 1,
        }
    }

    fn send(&mut self, message: &Value) {
        writeln!(self.input, "{message}").expect("writes");
        self.input.flush().expect("flushes");
    }

    fn line(&self) -> Value {
        self.lines
            .recv_timeout(Duration::from_secs(120))
            .expect("the server answers")
    }

    /// A request, and every line up to its answer: the notifications on
    /// the way, then the answer.
    fn ask(&mut self, method: &str, params: Value) -> (Vec<Value>, Value) {
        let id = self.next;
        self.next += 1;
        self.send(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }));
        let mut notes = Vec::new();
        loop {
            let line = self.line();
            if line.get("id") == Some(&json!(id)) {
                return (notes, line);
            }
            notes.push(line);
        }
    }

    /// A tool's result: its structured content when it has some, its text
    /// otherwise. Fails on a protocol error.
    fn tool(&mut self, name: &str, arguments: Value) -> Value {
        let (_, reply) = self.ask(
            "tools/call",
            json!({ "name": name, "arguments": arguments }),
        );
        let result = reply
            .get("result")
            .unwrap_or_else(|| panic!("{name}: {reply}"))
            .clone();
        assert_eq!(result["isError"], false, "{name}: {result}");
        result
    }

    /// Closes the server's input and waits for it to go.
    fn end(mut self) {
        drop(self.input);
        for _ in 0..200 {
            if let Ok(Some(status)) = self.child.try_wait() {
                assert!(status.success(), "{status}");
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let _ = self.child.kill();
        panic!("the server did not exit when its input closed");
    }
}

#[test]
fn an_agent_edits_previews_and_exports_a_project_headless() {
    let scratch = tempfile::tempdir().expect("scratch");
    let home = scratch.path();
    let mut agent = Agent::spawn(home, "headless", &[]);

    let (_, welcome) = agent.ask(
        "initialize",
        json!({
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": { "name": "e2e", "version": "1" },
        }),
    );
    let welcome = &welcome["result"];
    assert_eq!(welcome["protocolVersion"], "2025-06-18");
    assert_eq!(welcome["serverInfo"]["name"], "concat");
    assert!(welcome["capabilities"]["tools"].is_object());
    assert!(
        welcome["instructions"]
            .as_str()
            .is_some_and(|text| text.contains("project"))
    );
    agent.send(&json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }));

    let (_, listed) = agent.ask("tools/list", json!({}));
    let tools = listed["result"]["tools"].as_array().expect("tools");
    for name in [
        "project_create",
        "edit_apply",
        "title_add",
        "preview_frame",
        "export_wait",
    ] {
        let tool = tools
            .iter()
            .find(|tool| tool["name"] == name)
            .unwrap_or_else(|| panic!("no {name}"));
        assert_eq!(tool["inputSchema"]["type"], "object");
    }

    let version = agent.tool("concat_version", json!({}));
    assert_eq!(version["structuredContent"]["mcp"]["mode"], "headless");

    let created = agent.tool(
        "project_create",
        json!({
            "location": home.to_string_lossy(),
            "name": "Agent Cut",
            "video": { "width": 320, "height": 180, "rateNum": 30, "rateDen": 1 },
        }),
    );
    assert_eq!(created["structuredContent"]["project"]["name"], "Agent Cut");
    let path = home.join("Agent Cut").to_string_lossy().into_owned();

    let titled = agent.tool(
        "title_add",
        json!({
            "path": path,
            "start": 0.0,
            "duration": 1.0,
            "above": true,
            "style": { "content": "Hello, agent", "background": "#202020ff" },
        }),
    );
    let title = titled["structuredContent"]["createdId"].clone();
    assert!(title.is_string(), "{titled}");

    // "agent" in yellow, and the summary says which words wear what.
    let styled = agent.tool(
        "title_style_words",
        json!({
            "path": path,
            "clipId": title,
            "start": 7,
            "end": 12,
            "patch": { "color": "#ffd400" },
        }),
    );
    let clips = styled["structuredContent"]["timeline"]["clips"]
        .as_array()
        .expect("clips");
    let runs = clips
        .iter()
        .find(|clip| clip["id"] == title)
        .map(|clip| clip["textRuns"].clone())
        .expect("the title");
    assert_eq!(
        runs,
        json!([{ "start": 7, "end": 12, "color": "#ffd400" }]),
        "{styled}"
    );

    let captioned = agent.tool(
        "captions_add",
        json!({
            "path": path,
            "captions": [
                { "start": 0.0, "end": 0.5, "text": "one" },
                { "start": 0.5, "end": 1.0, "text": "two" },
            ],
        }),
    );
    assert_eq!(captioned["structuredContent"]["canUndo"], true);

    let keyed = agent.tool(
        "keyframe_set",
        json!({ "path": path, "clipId": title, "property": "opacity", "at": 1.0, "value": 0.25 }),
    );
    let read = agent.tool("timeline_read", json!({ "path": path }));
    let clips = read["structuredContent"]["timeline"]["clips"]
        .as_array()
        .expect("clips")
        .clone();
    assert_eq!(clips.len(), 3, "{read}");
    let first = clips
        .iter()
        .find(|clip| clip["id"] == title)
        .expect("the title");
    assert_eq!(first["text"], "Hello, agent");
    assert_eq!(first["keyframed"], json!(["opacity"]));
    assert_eq!(
        keyed["structuredContent"]["timeline"],
        read["structuredContent"]["timeline"]
    );

    // A wrong argument is the agent's to correct: a tool error, not a
    // protocol one.
    let (_, wrong) = agent.ask(
        "tools/call",
        json!({ "name": "clip_speed", "arguments": { "path": path, "clipId": title, "speed": "fast" } }),
    );
    assert_eq!(wrong["result"]["isError"], true, "{wrong}");
    let (_, unknown) = agent.ask(
        "tools/call",
        json!({ "name": "make_it_pop", "arguments": {} }),
    );
    assert_eq!(unknown["error"]["code"], -32602);

    let undone = agent.tool("edit_undo", json!({ "path": path }));
    let after_undo = undone["structuredContent"]["timeline"]["clips"]
        .as_array()
        .and_then(|clips| clips.iter().find(|clip| clip["id"] == title))
        .expect("the title")
        .clone();
    assert!(
        after_undo.get("keyframed").is_none(),
        "the key is undone: {after_undo}"
    );

    let frame = agent.tool("preview_frame", json!({ "path": path, "time": 0.25 }));
    let image = &frame["content"][0];
    assert_eq!(image["type"], "image");
    assert_eq!(image["mimeType"], "image/png");
    assert!(
        image["data"]
            .as_str()
            .is_some_and(|data| data.starts_with("iVBORw0KGgo")),
        "a PNG"
    );
    assert_eq!(
        frame["structuredContent"]["width"], 320,
        "the timeline's own size fits"
    );

    agent.tool("project_save", json!({ "path": path }));
    let saved = std::fs::read_to_string(Path::new(&path).join("concat.json")).expect("saved");
    assert!(saved.contains("Hello, agent"));

    let output = home.join("agent.mp4").to_string_lossy().into_owned();
    let started = agent.tool(
        "export_start",
        json!({ "path": path, "output": output, "preset": "ultrafast" }),
    );
    let job = started["structuredContent"]["job"].clone();
    assert_eq!(job, "j1");
    let (notes, waited) = agent.ask(
        "tools/call",
        json!({
            "name": "export_wait",
            "arguments": { "job": job, "timeoutSeconds": 120 },
            "_meta": { "progressToken": "export" },
        }),
    );
    let status = &waited["result"]["structuredContent"];
    assert_ne!(status["state"], "running", "{waited}");
    assert!(!notes.is_empty(), "progress was reported");
    let mut last = -1.0;
    for note in &notes {
        assert_eq!(note["method"], "notifications/progress");
        assert_eq!(note["params"]["progressToken"], "export");
        let progress = note["params"]["progress"].as_f64().expect("a number");
        assert!(progress > last, "progress rises");
        last = progress;
    }
    if status["state"] == "done" {
        assert!(Path::new(&output).exists());
    }

    // Writes stay under the home this server was given.
    let (_, outside) = agent.ask(
        "tools/call",
        json!({ "name": "project_create", "arguments": { "location": "/", "name": "nope" } }),
    );
    assert_eq!(outside["result"]["isError"], true, "{outside}");

    let (_, pong) = agent.ask("ping", json!({}));
    assert_eq!(pong["result"], json!({}));
    agent.end();
}

#[test]
fn a_modern_client_discovers_and_is_told_which_versions_are_spoken() {
    let scratch = tempfile::tempdir().expect("scratch");
    let mut agent = Agent::spawn(scratch.path(), "headless", &[]);
    let meta = |version: &str| {
        json!({
            "io.modelcontextprotocol/protocolVersion": version,
            "io.modelcontextprotocol/clientInfo": { "name": "e2e", "version": "1" },
            "io.modelcontextprotocol/clientCapabilities": {},
        })
    };
    let (_, discovered) = agent.ask("server/discover", json!({ "_meta": meta("2026-07-28") }));
    let result = &discovered["result"];
    assert_eq!(result["resultType"], "complete");
    assert_eq!(result["ttlMs"], 0);
    assert_eq!(result["cacheScope"], "public");
    assert!(
        result["supportedVersions"]
            .as_array()
            .is_some_and(|versions| versions.contains(&json!("2026-07-28")))
    );
    assert_eq!(
        result["_meta"]["io.modelcontextprotocol/serverInfo"]["name"],
        "concat"
    );

    let (_, listed) = agent.ask("tools/list", json!({ "_meta": meta("2026-07-28") }));
    assert_eq!(listed["result"]["resultType"], "complete");
    assert_eq!(listed["result"]["ttlMs"], 0);
    assert_eq!(listed["result"]["cacheScope"], "public");
    assert!(
        listed["result"]["tools"]
            .as_array()
            .is_some_and(|tools| tools.len() > 20)
    );

    let (_, refused) = agent.ask("tools/list", json!({ "_meta": meta("1900-01-01") }));
    assert_eq!(refused["error"]["code"], -32022);
    assert_eq!(refused["error"]["data"]["requested"], "1900-01-01");

    let (_, malformed) = agent.ask(
        "tools/list",
        json!({ "_meta": { "io.modelcontextprotocol/protocolVersion": "2026-07-28" } }),
    );
    assert_eq!(malformed["error"]["code"], -32602);

    agent.send(&json!({ "jsonrpc": "2.0", "id": 99, "method": "no/such" }));
    let missing = agent.line();
    assert_eq!(missing["error"]["code"], -32601);
    agent.send(&json!("not an object"));
    assert_eq!(agent.line()["error"]["code"], -32600);
    writeln!(agent.input, "{{ not json").expect("writes");
    assert_eq!(agent.line()["error"]["code"], -32700);
    agent.end();
}

#[test]
fn through_a_running_server_the_agent_shares_its_api() {
    use concat_api::{Api, AppDirs};
    let scratch = tempfile::tempdir().expect("scratch");
    let home = scratch.path().to_path_buf();
    let dirs = AppDirs {
        config: home.join("server-config"),
        data: home.join("server-data"),
    };
    let server = concat_server::Server::start(
        concat_server::Config {
            json: Some("127.0.0.1:0".parse().expect("an address")),
            roots: vec![home.clone()],
            ..concat_server::Config::default()
        },
        move |events| Ok(Api::with_dirs(dirs, events)),
    )
    .expect("a server");
    let address = server.json_addr().expect("listening").to_string();
    let mut agent = Agent::spawn(
        &home,
        "window",
        &[
            ("CONCAT_API_ADDRESS", address.clone()),
            ("CONCAT_API_TOKEN", server.token().to_owned()),
        ],
    );
    let version = agent.tool("concat_version", json!({}));
    assert_eq!(version["structuredContent"]["mcp"]["mode"], "window");
    assert_eq!(version["structuredContent"]["mcp"]["address"], address);
    agent.tool(
        "project_create",
        json!({ "location": home.to_string_lossy(), "name": "Shared" }),
    );
    // The project the agent made is open in the server's API: another
    // caller of that server sees it.
    let mut other = concat_server::client::Client::connect(
        server.json_addr().expect("listening"),
        server.token(),
        Duration::from_secs(5),
    )
    .expect("in");
    let seen = other
        .call(&concat_api::Request::ProjectGet {
            path: home.join("Shared").to_string_lossy().into_owned(),
        })
        .expect("open there");
    assert!(seen["project"].is_object());
    agent.end();
    drop(other);
    server.stop();
}

/// A scene goes in, an MP4 comes out: the camera track is evaluated frame
/// by frame and drawn by the same compositor the editor exports with.
#[test]
fn an_agent_renders_a_camera_scene_headless() {
    let scratch = tempfile::tempdir().expect("scratch");
    let home = scratch.path();
    let mut agent = Agent::spawn(home, "headless", &[]);
    let (_, welcome) = agent.ask(
        "initialize",
        json!({
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": { "name": "e2e", "version": "1" },
        }),
    );
    assert_eq!(welcome["result"]["protocolVersion"], "2025-06-18");
    agent.send(&json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }));

    let scene = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../examples/camera_demo/scene.json")
        .canonicalize()
        .expect("the demo scene is in the repository");
    let output = home.join("camera_demo.mp4");
    let (_, reply) = agent.ask(
        "tools/call",
        json!({
            "name": "scene_render",
            "arguments": {
                "scenePath": scene.to_string_lossy(),
                "output": output.to_string_lossy(),
                "crf": 28,
                "preset": "ultrafast",
            },
        }),
    );
    assert!(
        reply.get("error").is_none(),
        "scene_render refused: {reply}"
    );
    if reply["result"]["isError"].as_bool() == Some(true) {
        panic!("scene_render failed: {reply}");
    }
    assert!(output.is_file(), "the render writes the file: {reply}");

    let info = concat_media::probe(&output).expect("the file probes as media");
    let video = info.require_video().expect("it has pictures");
    assert_eq!((video.width, video.height), (1920, 1080));
    let duration = info.duration.expect("a duration").as_f64();
    assert!((duration - 3.0).abs() < 0.2, "duration {duration}");
}

/// An agent builds a two-character dialogue scene from nothing - no project
/// open, no hand-written JSON - then moves the camera between the characters
/// and renders it: the Phase 3 acceptance path.
#[test]
fn an_agent_builds_and_renders_a_two_character_scene_headless() {
    let scratch = tempfile::tempdir().expect("scratch");
    let home = scratch.path();
    let assets = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../examples/camera_demo/assets")
        .canonicalize()
        .expect("the demo assets are in the repository");
    let mut agent = Agent::spawn(home, "headless", &[]);
    let (_, welcome) = agent.ask(
        "initialize",
        json!({
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": { "name": "e2e", "version": "1" },
        }),
    );
    assert_eq!(welcome["result"]["protocolVersion"], "2025-06-18");
    agent.send(&json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }));

    let scene_path = home.join("dialogue.json");
    let created = agent.tool(
        "scene_create",
        json!({
            "scenePath": scene_path.to_string_lossy(),
            "name": "双人对话",
            "worldWidth": 8000.0,
            "worldHeight": 4500.0,
            "width": 640,
            "height": 360,
            "fps": 30,
            "background": [0.10, 0.12, 0.15, 1.0],
        }),
    );
    assert_eq!(created["structuredContent"]["scene"]["layers"], json!([]));
    assert!(
        created["structuredContent"]["scene"]["durationFrames"]
            .as_f64()
            .is_some_and(|frames| frames >= 89.0),
        "a fresh scene has a usable length: {created}"
    );

    for (id, kind, asset, x, z) in [
        ("bg", "background", "bg.png", 4000.0, 0),
        ("hero", "character", "char_a.png", 3600.0, 1),
        ("heroine", "character", "char_b.png", 4400.0, 2),
    ] {
        let asset = assets.join(asset);
        let added = agent.tool(
            "scene_add_layer",
            json!({
                "scenePath": scene_path.to_string_lossy(),
                "id": id,
                "name": id,
                "kind": kind,
                "asset": asset.to_string_lossy(),
                "x": x,
                "y": 2250.0,
                "scale": if kind == "background" { 1.0 } else { 0.65 },
                "z": z,
                "durationFrames": 120.0,
            }),
        );
        assert_eq!(
            added["structuredContent"]["scene"]["layers"]
                .as_array()
                .map(Vec::len),
            Some(z as usize + 1),
            "{added}"
        );
    }

    let moved = agent.tool(
        "scene_camera_move",
        json!({
            "scenePath": scene_path.to_string_lossy(),
            "atFrame": 0,
            "moves": [
                { "type": "follow", "target": "hero", "durationFrames": 60, "zoom": 1.15, "easing": "ease_in_out" },
                { "type": "follow", "target": "heroine", "durationFrames": 60, "easing": "ease_in_out" },
            ],
        }),
    );
    let camera = &moved["structuredContent"]["scene"]["camera"];
    assert_eq!(camera["keys"].as_array().map(Vec::len), Some(3), "{moved}");

    let read = agent.tool(
        "scene_get",
        json!({ "scenePath": scene_path.to_string_lossy() }),
    );
    let scene = &read["structuredContent"]["scene"];
    assert_eq!(scene["layers"].as_array().map(Vec::len), Some(3));
    assert_eq!(scene["viewport"]["width"], 640);
    assert_eq!(scene["viewport"]["height"], 360);

    let output = home.join("dialogue.mp4");
    let rendered = agent.tool(
        "scene_render",
        json!({
            "scenePath": scene_path.to_string_lossy(),
            "output": output.to_string_lossy(),
            "crf": 30,
            "preset": "ultrafast",
        }),
    );
    assert!(output.is_file(), "the render writes the file: {rendered}");
    let info = concat_media::probe(&output).expect("the file probes as media");
    let video = info.require_video().expect("it has pictures");
    assert_eq!((video.width, video.height), (640, 360));
    let duration = info.duration.expect("a duration").as_f64();
    assert!((duration - 4.0).abs() < 0.3, "duration {duration}");

    // The file is a Concat project document with the scene under `scene`.
    let document: Value =
        serde_json::from_str(&std::fs::read_to_string(&scene_path).expect("read back"))
            .expect("valid JSON");
    assert!(document["concat"].is_string(), "a Concat document");
    assert_eq!(document["scene"]["id"], "双人对话");

    // Scene writes are sandboxed like the project tools.
    let outside_path = home
        .parent()
        .expect("the scratch has a parent")
        .join("outside-scene.json");
    let (_, outside) = agent.ask(
        "tools/call",
        json!({
            "name": "scene_create",
            "arguments": {
                "scenePath": outside_path.to_string_lossy(),
                "name": "nope",
            },
        }),
    );
    assert_eq!(outside["result"]["isError"], true, "{outside}");
    agent.end();
}
