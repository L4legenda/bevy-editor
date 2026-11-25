use super::*;
use crate::{EditorProject, NodePolicy};

async fn tick_until<T>(app: &mut App, pending: tokio::task::JoinHandle<T>) -> T {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !pending.is_finished() {
        assert!(Instant::now() < deadline, "MCP request did not complete");
        app.update();
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    pending.await.ok().unwrap()
}

#[tokio::test(flavor = "current_thread")]
async fn sdk_client_discovers_tools_and_changes_the_live_app_over_http() {
    use rmcp::{
        ServiceExt,
        transport::{
            StreamableHttpClientTransport,
            streamable_http_client::StreamableHttpClientTransportConfig,
        },
    };
    let scratch = Scratch::new();
    let mut editor = editor();
    editor.root = scratch.0.clone();
    editor.project.asset_root = scratch.0.clone();
    let project = editor.project.clone();
    let (mut app, bridge) = app(editor);
    let server = server::start(&project, bridge).unwrap();
    let file = scratch.0.join(".bevy-editor/mcp.json");
    let endpoint: Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    let transport = StreamableHttpClientTransport::from_config(
        StreamableHttpClientTransportConfig::with_uri(
            endpoint["url"].as_str().unwrap().to_string(),
        )
        .auth_header(endpoint["token"].as_str().unwrap()),
    );
    let client = ().serve(transport).await.unwrap();
    let tools = client.list_tools(None).await.unwrap();
    assert_eq!(tools.tools.len(), 33);
    for name in [
        "get_editor_state",
        "get_paint_state",
        "replace_paint_mesh",
        "list_paint_presets",
        "edit_paint",
        "paint_stroke",
        "poly_fill",
        "select_surface",
        "save_model",
        "export_textures",
        "bake_maps",
        "get_bake_state",
        "cancel_bake",
        "set_paint_view",
        "cage_preview",
        "capture_paint_view",
        "set_transform",
        "add_model",
        "save_scene",
        "undo",
        "capture_viewport",
    ] {
        assert!(tools.tools.iter().any(|tool| tool.name == name));
    }
    let peer = client.peer().clone();
    let request = serde_json::from_value(json!({ "name": "list_paint_presets" })).unwrap();
    let result = tick_until(&mut app, tokio::spawn(async move { peer.call_tool(request).await }))
        .await.unwrap();
    let presets = result.structured_content.unwrap();
    assert_eq!(presets["channels"].as_array().unwrap().len(), 8);
    assert_eq!(presets["brush_tips"].as_array().unwrap().len(), 24);

    let peer = client.peer().clone();
    let request = serde_json::from_value(json!({ "name": "get_paint_state" })).unwrap();
    let result = tick_until(&mut app, tokio::spawn(async move { peer.call_tool(request).await }))
        .await.unwrap();
    assert_eq!(result.is_error, Some(true));
    assert!(serde_json::to_string(&result.content).unwrap().contains("Open a 3D model"));
    let peer = client.peer().clone();
    let request = serde_json::from_value(json!({ "name": "edit_paint", "arguments": {
        "model_id": 0, "set": 0, "edit": {"operation": "add_layer", "kind": "Paint"}
    }})).unwrap();
    let result = tick_until(&mut app, tokio::spawn(async move { peer.call_tool(request).await }))
        .await.unwrap();
    assert_eq!(result.is_error, Some(true));
    assert!(serde_json::to_string(&result.content).unwrap().contains("Open a 3D model"));
    let peer = client.peer().clone();
    let request = serde_json::from_value(
        json!({ "name": "add_model", "arguments": { "asset": project.catalog[0].asset } }),
    )
    .unwrap();
    let result = tick_until(
        &mut app,
        tokio::spawn(async move { peer.call_tool(request).await }),
    )
    .await
    .unwrap();
    assert_ne!(result.is_error, Some(true));
    assert_eq!(app.world().resource::<Editor>().edits.models.len(), 1);
    assert_eq!(app.world().resource::<Editor>().undo.len(), 1);
    let peer = client.peer().clone();
    let request = serde_json::from_value(json!({ "name": "set_transform", "arguments": {
        "object_id": result.structured_content.unwrap()["id"], "transform": { "scale": [0, 0, 0] }
    } }))
    .unwrap();
    let result = tick_until(
        &mut app,
        tokio::spawn(async move { peer.call_tool(request).await }),
    )
    .await
    .unwrap();
    assert_eq!(result.is_error, Some(true));
    assert_eq!(app.world().resource::<Editor>().undo.len(), 1);
    assert_eq!(
        client.list_resources(None).await.unwrap().resources.len(),
        3
    );
    client.cancel().await.unwrap();
    drop(server);
    assert!(!file.exists());
}

fn http_status(port: u16, host: &str, token: &str, origin: Option<&str>) -> u16 {
    use std::io::{Read, Write};
    let mut stream = std::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, port)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let origin = origin
        .map(|origin| format!("Origin: {origin}\r\n"))
        .unwrap_or_default();
    write!(stream, "GET /mcp HTTP/1.1\r\nHost: {host}\r\nAuthorization: Bearer {token}\r\n{origin}Connection: close\r\n\r\n").unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    response.split_whitespace().nth(1).unwrap().parse().unwrap()
}

#[test]
fn endpoint_checks_authentication_origin_host_and_discovery_ownership() {
    let scratch = Scratch::new();
    let mut project = editor().project;
    project.asset_root = scratch.0.clone();
    let (sender, _receiver) = mpsc::sync_channel(8);
    let bridge = Bridge {
        sender,
        wake: Arc::new(|| {}),
    };
    let first = server::start(&project, bridge.clone()).unwrap();
    let file = scratch.0.join(".bevy-editor/mcp.json");
    let endpoint: Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    let port: u16 = endpoint["url"]
        .as_str()
        .unwrap()
        .strip_prefix("http://127.0.0.1:")
        .unwrap()
        .strip_suffix("/mcp")
        .unwrap()
        .parse()
        .unwrap();
    let token = endpoint["token"].as_str().unwrap();
    let host = format!("127.0.0.1:{port}");
    assert_eq!(http_status(port, &host, "wrong-token", None), 401);
    assert_eq!(
        http_status(port, &host, token, Some("https://example.com")),
        403
    );
    assert_eq!(http_status(port, "example.com", token, None), 403);
    let second = server::start(&project, bridge).unwrap();
    let contents = std::fs::read(&file).unwrap();
    drop(first);
    assert_eq!(std::fs::read(&file).unwrap(), contents);
    drop(second);
    assert!(!file.exists());
}

struct Scratch(std::path::PathBuf);
impl Scratch {
    fn new() -> Self {
        let path =
            std::env::temp_dir().join(format!("bevy-editor-mcp-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn editor() -> Editor {
    let project = EditorProject::load(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/legacy/project.json"),
    )
    .unwrap();
    let mut edits = project.read_scene(0).unwrap();
    edits.models.clear();
    edits.lights.clear();
    edits.nodes.clear();
    edits.cameras.clear();
    edits.lighting = None;
    let mut editor = Editor::new(project, edits);
    editor.ready = true;
    editor.reload = false;
    editor.hidden_objects.clear();
    editor.imported.push(super::super::Imported {
        entity: Entity::PLACEHOLDER,
        name: "Source".into(),
        original: Transform::IDENTITY.into(),
        parent: GlobalTransform::IDENTITY,
        local: Transform::IDENTITY,
    });
    editor
}

fn add_model(editor: &mut Editor) -> String {
    let asset = editor.project.catalog[0].asset.clone();
    execute(
        editor,
        Command::AddModel(AddModelArgs {
            asset,
            transform: None,
        }),
    )
    .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string()
}

#[test]
fn transforms_are_atomic_respect_locks_and_share_undo_redo() {
    let mut editor = editor();
    editor.project.scenes[0].node_policies.insert(
        "Source".into(),
        NodePolicy {
            translation: [true, false, true],
            scale: [false; 3],
            ..default()
        },
    );
    let id = object_id(&editor, Selection::Imported(0));
    let transform = |position, scale| {
        Command::Transform(TransformArgs {
            object_id: id.clone(),
            transform: TransformPatch {
                position,
                scale,
                ..default()
            },
        })
    };
    let original = editor.edits.clone();
    assert!(execute(&mut editor, transform(Some([2.0, 1.0, 3.0]), None)).is_err());
    assert_eq!(editor.edits, original);
    assert!(editor.undo.is_empty());
    execute(&mut editor, transform(Some([2.0, 0.0, 3.0]), None)).unwrap();
    assert_eq!(editor.undo.len(), 1);
    assert!(editor.dirty());
    assert!(editor.selection.is_none());
    let changed = editor.edits.clone();
    assert!(
        execute(
            &mut editor,
            transform(Some([4.0, 0.0, 5.0]), Some([2.0; 3]))
        )
        .is_err()
    );
    assert_eq!(editor.edits, changed);
    assert_eq!(editor.undo.len(), 1);
    execute(&mut editor, Command::Undo).unwrap();
    assert_eq!(editor.edits, original);
    execute(&mut editor, Command::Redo).unwrap();
    assert_eq!(editor.edits, changed);
    for scale in [[0.0; 3], [f32::NAN; 3], [101.0; 3]] {
        assert!(execute(&mut editor, transform(None, Some(scale))).is_err());
        assert_eq!(editor.edits, changed);
    }
}

#[test]
fn object_ids_are_scene_qualified_and_model_creation_and_removal_are_undoable() {
    let mut editor = editor();
    let source = object_id(&editor, Selection::Imported(0));
    assert!(
        execute(
            &mut editor,
            Command::Remove(ObjectArgs { object_id: source })
        )
        .is_err()
    );
    let id = add_model(&mut editor);
    let created = editor.edits.clone();
    assert_eq!(editor.undo.len(), 1);
    execute(
        &mut editor,
        Command::Remove(ObjectArgs {
            object_id: id.clone(),
        }),
    )
    .unwrap();
    assert!(editor.edits.models.is_empty());
    execute(&mut editor, Command::Undo).unwrap();
    assert_eq!(editor.edits, created);
    assert!(
        execute(
            &mut editor,
            Command::AddModel(AddModelArgs {
                asset: "../../outside.glb".into(),
                transform: None
            })
        )
        .is_err()
    );
    let other_scene = editor.project.scenes[1].file.clone();
    execute(
        &mut editor,
        Command::Open(SceneArgs {
            scene_id: other_scene,
        }),
    )
    .unwrap();
    assert!(resolve(&editor, &id).is_err());
    assert!(
        editor
            .documents
            .get(&0)
            .is_some_and(|document| document.edits == created)
    );
}

#[test]
fn light_validation_is_atomic_and_the_whole_creation_has_one_undo_step() {
    let mut editor = editor();
    let light = execute(
        &mut editor,
        Command::AddLight(AddLightArgs {
            preset: Some("spot".into()),
            position: Some([1.0, 2.0, 3.0]),
        }),
    )
    .unwrap();
    assert_eq!(editor.undo.len(), 1);
    assert!(editor.edits.lighting.is_some());
    assert_eq!(editor.edits.lights[0].pose.position, [1.0, 2.0, 3.0]);
    let original = editor.edits.clone();
    let args =
        serde_json::from_value(json!({ "object_id": light["id"], "intensity": 1200, "range": -1 }))
            .unwrap();
    assert!(execute(&mut editor, Command::Light(args)).is_err());
    assert_eq!(editor.edits, original);
    let args = serde_json::from_value(json!({ "object_id": light["id"], "intensity": 1200, "color": [0.2, 0.4, 1.0], "use_temperature": false })).unwrap();
    execute(&mut editor, Command::Light(args)).unwrap();
    assert_eq!(editor.undo.len(), 2);
    assert_eq!(editor.edits.lights[0].intensity, 1200.0);
    execute(&mut editor, Command::Undo).unwrap();
    assert_eq!(editor.edits, original);
    execute(&mut editor, Command::Undo).unwrap();
    assert!(editor.edits.lights.is_empty() && editor.edits.lighting.is_none());
}

#[test]
fn inspection_is_paginated_and_view_changes_never_modify_the_scene() {
    let mut editor = editor();
    let first = execute(
        &mut editor,
        Command::Objects(Page {
            offset: 0,
            limit: Some(1),
        }),
    )
    .unwrap();
    assert_eq!(first["items"].as_array().unwrap().len(), 1);
    assert_eq!(first["next_offset"], 1);
    assert_eq!(first["items"][0]["kind"], "imported");
    assert!(
        execute(
            &mut editor,
            Command::Objects(Page {
                offset: 0,
                limit: Some(0)
            })
        )
        .is_err()
    );
    let before = editor.edits.clone();
    execute(
        &mut editor,
        Command::View(ViewArgs {
            position: [0.0, 5.0, 0.0],
            target: [0.0; 3],
        }),
    )
    .unwrap();
    assert!(editor.view.rotation.is_finite());
    assert_eq!(editor.edits, before);
    assert!(editor.undo.is_empty());
}

fn app(editor: Editor) -> (App, Bridge) {
    let (sender, receiver) = mpsc::sync_channel(64);
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .insert_resource(editor)
        .insert_resource(Inbox(Mutex::new(receiver)))
        .init_resource::<Scene>()
        .init_resource::<editing::State>()
        .init_resource::<PaintState>()
        .init_resource::<viewport::PointerState>()
        .init_resource::<ButtonInput<MouseButton>>()
        .add_systems(Update, process);
    (
        app,
        Bridge {
            sender,
            wake: Arc::new(|| {}),
        },
    )
}

fn enqueue(bridge: &Bridge, command: Command, deadline: Instant) -> oneshot::Receiver<Reply> {
    let (reply, receiver) = oneshot::channel();
    bridge
        .sender
        .try_send(Job {
            command,
            reply,
            deadline,
        })
        .ok()
        .unwrap();
    receiver
}

struct StdioClient {
    child: std::process::Child,
    input: Option<std::process::ChildStdin>,
    replies: mpsc::Receiver<String>,
    next: u32,
}

impl StdioClient {
    fn launch(project: &std::path::Path) -> Self {
        use std::{
            io::{BufRead, BufReader},
            process::Stdio,
        };

        let binary = std::env::var_os("BEVY_EDITOR_MCP_STDIO")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                std::env::current_exe()
                    .unwrap()
                    .parent()
                    .unwrap()
                    .parent()
                    .unwrap()
                    .join(format!("bevy-editor-mcp{}", std::env::consts::EXE_SUFFIX))
            });
        let mut command = std::process::Command::new(binary);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0800_0000);
        }
        let mut child = command
            .arg("--project")
            .arg(project)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("Build bevy-editor-mcp before running the stdio workflow test");
        let input = child.stdin.take();
        let output = child.stdout.take().unwrap();
        let (send, replies) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(output).lines() {
                let Ok(line) = line else { break };
                if send.send(line).is_err() {
                    break;
                }
            }
        });
        Self {
            child,
            input,
            replies,
            next: 0,
        }
    }

    fn send(&mut self, message: Value) {
        use std::io::Write;
        let input = self.input.as_mut().unwrap();
        writeln!(input, "{message}").unwrap();
        input.flush().unwrap();
    }

    fn request(&mut self, app: &mut App, method: &str, params: Value) -> Value {
        self.next += 1;
        let id = self.next;
        self.send(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        let started = Instant::now();
        loop {
            for line in self.replies.try_iter() {
                let reply: Value = serde_json::from_str(&line)
                    .expect("Stdout must contain only MCP JSON-RPC messages");
                if reply["id"] == id {
                    assert!(reply["error"].is_null(), "{method}: {reply}");
                    return reply["result"].clone();
                }
            }
            assert!(
                self.child.try_wait().unwrap().is_none(),
                "The stdio bridge exited while handling {method}"
            );
            assert!(
                started.elapsed() < REQUEST_TIMEOUT,
                "The stdio bridge did not respond to {method}"
            );
            crate::editor::selection_outline::render_tests::frame(app);
        }
    }

    fn tool(&mut self, app: &mut App, name: &str, arguments: Value) -> Value {
        let reply = self.request(
            app,
            "tools/call",
            json!({"name": name, "arguments": arguments}),
        );
        assert_ne!(reply["isError"], true, "{name}: {reply}");
        reply
    }

    fn close(mut self) {
        self.input.take();
        let started = Instant::now();
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                assert!(
                    status.success(),
                    "The stdio bridge failed to shut down: {status}"
                );
                return;
            }
            assert!(
                started.elapsed() < Duration::from_secs(5),
                "The stdio bridge did not exit on EOF"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

impl Drop for StdioClient {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

#[test]
#[ignore = "requires GPU and a built bevy-editor-mcp binary; tests the complete stdio paint workflow"]
fn gpu_stdio_paint_workflow_saves_and_exports() {
    use crate::editor::{
        model_document::{Mode, glb::Glb},
        model_mesh::{Corner, Face, Geometry},
        selection_outline::render_tests::frame,
        tests::lighting_ui_render::fixture_project,
    };
    use std::{fs, path::Path};

    let scratch = Scratch::new();
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/legacy");
    let manifest = scratch.0.join("project.json");
    fs::copy(fixture.join("project.json"), &manifest).unwrap();
    for directory in ["levels", "models", "scenes"] {
        let destination = scratch.0.join("assets").join(directory);
        fs::create_dir_all(&destination).unwrap();
        for file in fs::read_dir(fixture.join("assets").join(directory)).unwrap() {
            let file = file.unwrap();
            fs::copy(file.path(), destination.join(file.file_name())).unwrap();
        }
    }
    let project = EditorProject::load(&manifest).unwrap();
    let (mut app, _, _) = fixture_project(project.clone());
    app.world_mut().resource_mut::<Editor>().drawer = false;
    let (sender, receiver) = mpsc::sync_channel(64);
    let server = server::start(
        &project,
        Bridge {
            sender,
            wake: Arc::new(|| {}),
        },
    )
    .unwrap();
    app.insert_resource(Inbox(Mutex::new(receiver)))
        .add_systems(
            Update,
            process
                .after(super::super::handle_actions)
                .before(super::super::begin_room),
        );

    let mut client = StdioClient::launch(&manifest);
    let initialized = client.request(&mut app, "initialize", json!({
        "protocolVersion": rmcp::model::ProtocolVersion::LATEST_WITH_INITIALIZE, "capabilities": {},
        "clientInfo": {"name": "bevy-paint-stdio-test", "version": "1"},
    }));
    assert_eq!(initialized["serverInfo"]["name"], "bevy-editor");
    client.send(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
    let tools = client.request(&mut app, "tools/list", json!({}));
    for name in [
        "open_paint_model",
        "replace_paint_mesh",
        "select_surface",
        "paint_stroke",
        "save_model",
        "export_textures",
        "bake_maps",
        "capture_paint_view",
    ] {
        assert!(
            tools["tools"]
                .as_array()
                .unwrap()
                .iter()
                .any(|tool| tool["name"] == name)
        );
    }

    let open = |client: &mut StdioClient, app: &mut App, asset: &str, index: usize| {
        let reply = client.tool(app, "open_paint_model", json!({"asset": asset}));
        assert_eq!(reply["structuredContent"]["model_id"], index);
        assert_eq!(app.world().resource::<Editor>().viewing_model, Some(index));
    };
    let wait_for_mode = |app: &mut App, index: usize| {
        let started = Instant::now();
        loop {
            let editing = app.world().resource::<editing::State>();
            if editing.mode(Some(index)) == Mode::Paint {
                assert_eq!(app.world().resource::<Editor>().viewing_model, Some(index));
                break;
            }
            assert!(!editing.errors.contains_key(&index), "{:?}", editing.errors);
            assert!(started.elapsed() < REQUEST_TIMEOUT, "Could not open model {index}");
            frame(app);
        }
    };
    let invalid = client.request(&mut app, "tools/call", json!({
        "name": "open_paint_model", "arguments": {"asset": "models/missing.glb"},
    }));
    assert_eq!(invalid["isError"], true);
    assert_eq!(app.world().resource::<Editor>().viewing_model, None);
    open(&mut client, &mut app, "models/block.glb", 0);
    let started = Instant::now();
    while !app.world().resource::<editing::State>().documents.contains_key(&0) {
        assert!(started.elapsed() < REQUEST_TIMEOUT, "Could not load the test model");
        frame(&mut app);
    }
    {
        let mut editing = app.world_mut().resource_mut::<editing::State>();
        let doc = editing.documents.get_mut(&0).unwrap();
        let corners = [Vec2::ZERO, Vec2::X, Vec2::ONE, Vec2::Y]
            .into_iter()
            .enumerate()
            .map(|(vertex, uv)| {
                let mut corner = Corner::new(vertex);
                corner.uv = uv;
                corner
            })
            .collect();
        doc.objects[0].geometry = Geometry {
            positions: vec![
                Vec3::new(-1., -1., 0.),
                Vec3::new(1., -1., 0.),
                Vec3::new(1., 1., 0.),
                Vec3::new(-1., 1., 0.),
            ],
            faces: vec![Face {
                corners,
                material: 0,
                smooth: false,
            }],
            ..default()
        }
        .into();
        doc.objects[0].transform = Transform::IDENTITY;
        doc.paint_default_size = 512;
        doc.auto_bake_pending = false;
        doc.revision += 1;
    }
    wait_for_mode(&mut app, 0);
    let wait_for_paint = |client: &mut StdioClient, app: &mut App| {
        let started = Instant::now();
        loop {
            let state = client.tool(app, "get_paint_state", json!({}));
            let state = &state["structuredContent"];
            if state["mode"] == "paint"
                && state["paint_busy"] == false
                && state["brush"].is_object()
            {
                return state.clone();
            }
            assert!(
                started.elapsed() < REQUEST_TIMEOUT,
                "Painting workspace stayed busy: {state}"
            );
        }
    };
    assert_eq!(wait_for_paint(&mut client, &mut app)["resolution"], 512);
    let view = client.tool(
        &mut app,
        "set_paint_view",
        json!({
            "model_id": 0, "center": [0, 0, 0], "distance": 4, "yaw_degrees": 0, "pitch_degrees": 0,
            "layout": "three", "preview": {"mode": "material"},
        }),
    );
    assert_eq!(view["structuredContent"]["layout"], "three");
    let started = client.tool(
        &mut app,
        "bake_maps",
        json!({
            "target": {"model_id": 0, "set": 0}, "quality": "draft", "maps": ["part_random"],
        }),
    );
    assert_eq!(started["structuredContent"]["status"], "running");
    let now = Instant::now();
    loop {
        let report = client.tool(&mut app, "get_bake_state", json!({"model_id": 0, "set": 0}));
        let report = &report["structuredContent"];
        if report["status"] == "ready" {
            assert!(
                report["maps"]
                    .as_array()
                    .unwrap()
                    .contains(&json!("part_random"))
            );
            break;
        }
        assert_eq!(report["status"], "running", "{report}");
        assert!(now.elapsed() < REQUEST_TIMEOUT, "The bake did not complete");
    }

    wait_for_paint(&mut client, &mut app);
    let added = client.tool(
        &mut app,
        "edit_paint",
        json!({
            "model_id": 0, "set": 0, "edit": {"operation": "add_layer", "kind": "Paint"},
        }),
    );
    let layer = added["structuredContent"]["layers_bottom_to_top"]
        .as_array()
        .unwrap()
        .iter()
        .find(|layer| layer["selected"] == true)
        .unwrap()["index"]
        .as_u64()
        .unwrap() as usize;
    let before = wait_for_paint(&mut client, &mut app);
    let paint = client.tool(&mut app, "paint_stroke", json!({
        "workspace": {"model_id": 0, "set": 0}, "target": {"layer": layer, "space": "uv"},
        "samples": [{"position": [0.5, 0.5]}],
        "brush": {"size_px": 96, "channels": {"base_color": [210, 10, 170]}, "hardness_percent": 100, "flow_percent": 100},
    }));
    assert_eq!(paint["structuredContent"]["paint_busy"], true);
    let painted = wait_for_paint(&mut client, &mut app);
    assert_eq!(
        painted["undo_steps"].as_u64().unwrap(),
        before["undo_steps"].as_u64().unwrap() + 1
    );
    let capture = client.tool(&mut app, "capture_paint_view", json!({"model_id": 0}));
    let png = capture["content"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["type"] == "image")
        .unwrap();
    let png = base64::engine::general_purpose::STANDARD
        .decode(png["data"].as_str().unwrap())
        .unwrap();
    let image = image::load_from_memory(&png).unwrap().to_rgba8();
    assert!(image.width() > 1 && image.height() > 1);
    assert!(
        image
            .pixels()
            .any(|pixel| pixel[0] > pixel[1].saturating_add(35)
                && pixel[2] > pixel[1].saturating_add(25))
    );

    open(&mut client, &mut app, "models/blue_block.glb", 1);
    wait_for_mode(&mut app, 1);
    open(&mut client, &mut app, "models/block.glb", 0);
    let reopened = wait_for_paint(&mut client, &mut app);
    assert_eq!(reopened["model_id"], 0);
    assert_eq!(reopened["active_set"], painted["active_set"]);
    assert_eq!(reopened["layers_bottom_to_top"], painted["layers_bottom_to_top"]);
    assert_eq!(app.world().resource::<Editor>().model_tabs, vec![0, 1]);

    let select = |client: &mut StdioClient, app: &mut App, source: Value, mode: &str| {
        let reply = client.tool(app, "select_surface", json!({
            "workspace": {"model_id": 0, "set": 0},
            "layer": layer, "source": source, "mode": mode,
        }));
        assert_eq!(reply["structuredContent"]["surface_selection"]["status"], "running");
        wait_for_paint(client, app)
    };
    let selection = select(&mut client, &mut app, json!({"kind":"point","position":[0,0,0]}), "replace");
    assert_eq!(selection["surface_selection"]["status"], "complete", "{selection}");
    assert_eq!(selection["surface_selection"]["triangles_selected"], 2);
    assert_eq!(selection["undo_steps"].as_u64().unwrap(), reopened["undo_steps"].as_u64().unwrap() + 1);
    let invalid = select(&mut client, &mut app, json!({"kind":"parts","indices":[1]}), "subtract");
    assert_eq!(invalid["surface_selection"]["status"], "failed");
    assert_eq!(invalid["undo_steps"], selection["undo_steps"]);
    let parts = json!({"kind":"parts","indices":[0]});
    let subtracted = select(&mut client, &mut app, parts.clone(), "subtract");
    assert_eq!(subtracted["surface_selection"]["triangles_in_active_set"], 2);
    assert_eq!(subtracted["undo_steps"].as_u64().unwrap(), selection["undo_steps"].as_u64().unwrap() + 1);
    let added = select(&mut client, &mut app, parts, "add");
    assert_eq!(added["undo_steps"].as_u64().unwrap(), subtracted["undo_steps"].as_u64().unwrap() + 1);
    for _ in 0..2 {
        client.tool(&mut app, "edit_paint", json!({
            "model_id": 0, "set": 0, "edit": {"operation": "undo"},
        }));
        wait_for_paint(&mut client, &mut app);
    }
    let masked = wait_for_paint(&mut client, &mut app);
    assert_eq!(masked["undo_steps"], selection["undo_steps"]);

    let replace = |client: &mut StdioClient, app: &mut App, replacement: Value| {
        let reply = client.tool(app, "replace_paint_mesh", json!({
            "workspace": {"model_id": 0, "set": 0}, "replacement": replacement,
        }));
        reply["structuredContent"].clone()
    };
    let primitive = replace(&mut client, &mut app, json!({"kind":"primitive","shape":"rounded_cube"}));
    assert_eq!(primitive["status"], "replaced");
    assert_eq!(primitive["transferred_sets"], 1);
    assert!(wait_for_paint(&mut client, &mut app)["mesh"]["triangles"].as_u64().unwrap() > 2);
    assert_eq!(
        replace(&mut client, &mut app, json!({"kind":"undo"}))["status"],
        "undone"
    );
    assert_eq!(wait_for_paint(&mut client, &mut app)["mesh"]["triangles"], 2);

    let missing = client.request(&mut app, "tools/call", json!({
        "name": "replace_paint_mesh",
        "arguments": {
            "workspace": {"model_id": 0, "set": 0},
            "replacement": {"kind": "file", "path": "models/missing.obj"},
        },
    }));
    assert_eq!(missing["isError"], true, "{missing}");
    assert_eq!(wait_for_paint(&mut client, &mut app)["mesh"]["triangles"], 2);
    fs::write(project.asset_root.join("models/replacement.obj"), concat!(
        "v -1 -1 0\nv 1 -1 0\nv 0 1 0\n",
        "vt 0 0\nvt 1 0\nvt 0.5 1\n",
        "f 1/1 2/2 3/3\n",
    )).unwrap();
    let imported = replace(&mut client, &mut app, json!({"kind":"file","path":"models/replacement.obj"}));
    assert_eq!(imported["status"], "replaced");
    assert_eq!(imported["transferred_sets"], 1);
    let replaced = wait_for_paint(&mut client, &mut app);
    assert_eq!(replaced["mesh"]["triangles"], 1);
    assert_eq!(replaced["active_set"], reopened["active_set"]);
    assert_eq!(replaced["layers_bottom_to_top"], masked["layers_bottom_to_top"]);
    assert_eq!(replace(&mut client, &mut app, json!({"kind":"undo"}))["status"], "undone");
    assert_eq!(wait_for_paint(&mut client, &mut app)["mesh"]["triangles"], 2);
    assert_eq!(replace(&mut client, &mut app, json!({"kind":"redo"}))["status"], "redone");
    assert_eq!(wait_for_paint(&mut client, &mut app)["mesh"]["triangles"], 1);

    let saved = client.tool(
        &mut app,
        "save_model",
        json!({"model_id": 0, "copy_name": "stdio-painted.glb"}),
    );
    let path = saved["structuredContent"]["path"].as_str().unwrap();
    let glb = Glb::load(Path::new(path)).unwrap();
    let mesh = glb.json["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find_map(|node| node["mesh"].as_u64())
        .unwrap() as usize;
    let surface = glb
        .paint_surface(&glb.json["meshes"][mesh]["primitives"][0])
        .unwrap()
        .unwrap();
    let middle = ((512 * 256) + 256) * 4;
    assert_eq!(
        &surface.stack.layers[layer].channels[0].as_ref().unwrap()[middle..middle + 4],
        &[210, 10, 170, 255]
    );
    assert_eq!(&surface.stack.layers[layer].mask.as_ref().unwrap().pixels[middle..middle + 4], &[255; 4]);

    let export = client.tool(&mut app, "export_textures", json!({
        "model_id": 0, "folder": ".", "all_sets": false, "template": "separate", "format": "png", "bit_depth": 8, "size": 512,
    }));
    let color = export["structuredContent"]["files"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .find(|file| file.contains("BaseColor"))
        .unwrap();
    let exported = image::open(color).unwrap().to_rgba8();
    assert_eq!(exported.get_pixel(256, 256).0, [210, 10, 170, 255]);
    client.close();
    drop(server);
}

#[test]
fn capture_uses_the_camera_target_including_its_dpi_scale() {
    use bevy::camera::ImageRenderTarget;

    for scale_factor in [1.0, 1.25, 2.0] {
        let (mut app, bridge) = app(editor());
        let handle = Handle::<Image>::default();
        let target = RenderTarget::Image(ImageRenderTarget {
            handle: handle.clone(),
            scale_factor,
        });
        app.insert_resource(viewport::Surface(handle));
        app.world_mut()
            .spawn((viewport::EditorCamera, target.clone()));
        let _reply = enqueue(&bridge, Command::Capture, Instant::now() + REQUEST_TIMEOUT);
        app.update();
        let world = app.world_mut();
        let screenshot = world.query::<&Screenshot>().single(world).unwrap();
        assert_eq!(
            screenshot.0.normalize(None),
            target.normalize(None),
            "Capture lost the camera's DPI scale"
        );
    }
}

#[test]
#[ignore = "requires GPU; captures the real editor viewport through MCP at 200% DPI"]
fn gpu_mcp_capture_renders_the_viewport_at_high_dpi() {
    use crate::editor::{
        selection_outline::render_tests::frame, tests::lighting_ui_render::fixture,
    };

    let (mut app, window, _) = fixture();
    app.world_mut()
        .get_mut::<Window>(window)
        .unwrap()
        .resolution
        .set_scale_factor_override(Some(2.0));
    let (sender, receiver) = mpsc::sync_channel(64);
    let bridge = Bridge {
        sender,
        wake: Arc::new(|| {}),
    };
    app.insert_resource(Inbox(Mutex::new(receiver)))
        .add_systems(
            Update,
            process
                .after(super::super::handle_actions)
                .before(super::super::begin_room),
        );
    for _ in 0..24 {
        frame(&mut app);
    }
    {
        let world = app.world_mut();
        let target = world
            .query_filtered::<&RenderTarget, With<viewport::EditorCamera>>()
            .single(world)
            .unwrap();
        assert!(matches!(target, RenderTarget::Image(image) if image.scale_factor == 2.0));
    }

    let output = std::env::var_os("BEVY_EDITOR_TEST_OUTPUT_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("bevy-editor-mcp-capture"));
    std::fs::create_dir_all(&output).unwrap();
    let mut captures = Vec::new();
    for (index, position) in [[6.0, 4.0, 7.0], [-5.0, 3.0, 2.0]].into_iter().enumerate() {
        let mut view = enqueue(
            &bridge,
            Command::View(ViewArgs {
                position,
                target: [0.0; 3],
            }),
            Instant::now() + REQUEST_TIMEOUT,
        );
        for _ in 0..5 {
            frame(&mut app);
        }
        assert!(view.try_recv().unwrap().is_ok());
        let mut pending = enqueue(&bridge, Command::Capture, Instant::now() + REQUEST_TIMEOUT);
        let payload = (0..240)
            .find_map(|_| {
                frame(&mut app);
                pending.try_recv().ok()
            })
            .expect("MCP capture did not complete")
            .expect("MCP capture failed");
        let Payload::Image { png, width, height } = payload else {
            panic!("Capture must return an image");
        };
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(png)
            .unwrap();
        let image = image::load_from_memory(&bytes).unwrap().into_rgb8();
        assert_eq!(image.dimensions(), (width, height));
        assert!(width <= 1600 && height <= 1000);
        let visible = image
            .pixels()
            .filter(|p| p.0.iter().any(|v| *v > 24))
            .count();
        assert!(
            visible > (width * height / 20) as usize,
            "MCP returned a black capture"
        );
        std::fs::write(output.join(format!("mcp-viewport-dpi2-{index}.png")), bytes).unwrap();
        captures.push(image);
    }
    assert_eq!(captures[0].dimensions(), captures[1].dimensions());
    let changed = captures[0]
        .pixels()
        .zip(captures[1].pixels())
        .filter(|(a, b)| {
            a.0.iter()
                .zip(b.0)
                .any(|(a, b)| (*a as i16 - b as i16).abs() > 20)
        })
        .count();
    assert!(
        changed > (captures[0].width() * captures[0].height() / 50) as usize,
        "Capture must follow the changed live camera view"
    );
}

#[test]
fn expired_cancelled_and_busy_requests_never_mutate_the_editor() {
    let editor = editor();
    let command = Command::AddModel(AddModelArgs {
        asset: editor.project.catalog[0].asset.clone(),
        transform: None,
    });
    let (mut app, bridge) = app(editor);
    let mut expired = enqueue(
        &bridge,
        command.clone(),
        Instant::now() - Duration::from_secs(1),
    );
    drop(enqueue(
        &bridge,
        command.clone(),
        Instant::now() + REQUEST_TIMEOUT,
    ));
    app.update();
    assert!(expired.try_recv().is_err());
    assert!(app.world().resource::<Editor>().edits.models.is_empty());
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .press(MouseButton::Left);
    let mut busy = enqueue(&bridge, command.clone(), Instant::now() + REQUEST_TIMEOUT);
    let mut state = enqueue(&bridge, Command::State, Instant::now() + REQUEST_TIMEOUT);
    app.update();
    assert!(busy.try_recv().unwrap().is_err());
    assert!(state.try_recv().unwrap().is_ok());
    assert!(app.world().resource::<Editor>().edits.models.is_empty());
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .reset_all();
    let mut accepted = enqueue(&bridge, command, Instant::now() + REQUEST_TIMEOUT);
    app.update();
    assert!(accepted.try_recv().unwrap().is_ok());
    assert_eq!(app.world().resource::<Editor>().edits.models.len(), 1);
    assert_eq!(app.world().resource::<Editor>().undo.len(), 1);
}

#[test]
fn saving_uses_loaded_geometry_and_never_writes_before_validation_succeeds() {
    let scratch = Scratch::new();
    let mut editor = editor();
    let asset = &editor.project.catalog[0].asset;
    let model_file = scratch.0.join(asset);
    std::fs::create_dir_all(model_file.parent().unwrap()).unwrap();
    std::fs::copy(editor.root.join(asset), &model_file).unwrap();
    editor.root = scratch.0.clone();
    add_model(&mut editor);
    let model_id = editor.edits.models[0].id;
    let path = editor.root.join(&editor.definition().file);
    let (mut app, bridge) = app(editor);
    let mut pending = enqueue(&bridge, Command::Save, Instant::now() + REQUEST_TIMEOUT);
    app.update();
    assert!(pending.try_recv().unwrap().is_err());
    assert!(!path.exists());
    let entity = app.world_mut().spawn(GlobalTransform::IDENTITY).id();
    app.world_mut().spawn((
        Aabb::from_min_max(Vec3::splat(-0.5), Vec3::splat(0.5)),
        GlobalTransform::IDENTITY,
        ChildOf(entity),
    ));
    app.world_mut()
        .resource_mut::<Scene>()
        .models
        .insert(model_id, (entity, String::new()));
    let mut saved = enqueue(&bridge, Command::Save, Instant::now() + REQUEST_TIMEOUT);
    app.update();
    if let Err(error) = saved.try_recv().unwrap() {
        panic!("Expected the loaded model to save: {error}");
    }
    let contents: crate::LevelEdits =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert!(contents.models[0].bounds.is_some());
    assert_eq!(app.world().resource::<Editor>().edits, contents);
    assert!(!app.world().resource::<Editor>().dirty());
}
