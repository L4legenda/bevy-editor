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
    assert_eq!(tools.tools.len(), 41);
    for name in [
        "get_editor_state",
        "get_paint_state",
        "replace_paint_mesh",
        "list_paint_presets",
        "edit_paint",
        "paint_stroke",
        "poly_fill",
        "list_paint_surface_parts",
        "select_surface",
        "save_model",
        "save_paint_preset",
        "import_paint_preset",
        "export_paint_preset",
        "export_textures",
        "bake_maps",
        "set_bake_levels",
        "get_bake_state",
        "cancel_bake",
        "set_paint_view",
        "cage_preview",
        "capture_paint_view",
        "render_paint_view",
        "set_transform",
        "add_model",
        "save_scene",
        "undo",
        "capture_viewport",
        "screenshot_app",
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
    assert_eq!(presets["brush_grains"].as_array().unwrap().len(), 25);
    let clouds = presets["generators"].as_array().unwrap().iter()
        .find(|item| item["id"] == "Clouds").unwrap();
    assert_eq!(clouds["controls"][0]["default"], 50.0);
    assert!(presets["generator_inputs"].as_array().unwrap().iter().any(|input| input["key"] == "noise"));
    let levels = presets["mask_filters"].as_array().unwrap().iter()
        .find(|item| item["id"] == "Levels").unwrap();
    assert_eq!(levels["controls"][4]["default"], 100.0);

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

    #[track_caller]
    fn tool(&mut self, app: &mut App, name: &str, arguments: Value) -> Value {
        let reply = self.request(
            app,
            "tools/call",
            json!({"name": name, "arguments": &arguments}),
        );
        assert_ne!(reply["isError"], true, "{name} {arguments}: {reply}");
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

fn stdio_paint_app() -> (Scratch, App, server::RunningServer, StdioClient) {
    use crate::editor::tests::lighting_ui_render::fixture_project;
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
    (scratch, app, server, client)
}

#[test]
#[ignore = "requires GPU and a built bevy-editor-mcp binary; exercises undo and redo across sets"]
fn gpu_stdio_history_switches_to_the_affected_texture_set() {
    use crate::editor::{model_document::Mode, selection_outline::render_tests::frame};

    let (_scratch, mut app, _server, mut client) = stdio_paint_app();
    client.tool(&mut app, "open_paint_model", json!({"asset": "models/block.glb"}));
    let deadline = Instant::now() + REQUEST_TIMEOUT;
    while !app.world().resource::<editing::State>().documents.contains_key(&0) {
        assert!(Instant::now() < deadline);
        frame(&mut app);
    }
    {
        let mut editing = app.world_mut().resource_mut::<editing::State>();
        let doc = editing.documents.get_mut(&0).unwrap();
        let object = &mut doc.objects[0];
        assert_eq!(object.materials.len(), 1);
        object.materials.push(object.materials[0].clone());
        object.geometry.faces[0].material = 1;
        doc.paint_default_size = 512;
        doc.auto_bake_pending = false;
        doc.revision += 1;
    }
    let ready = |client: &mut StdioClient, app: &mut App| {
        let deadline = Instant::now() + REQUEST_TIMEOUT;
        loop {
            frame(app);
            if app.world().resource::<editing::State>().mode(Some(0)) == Mode::Paint {
                let reply = client.tool(app, "get_paint_state", json!({}));
                let state = &reply["structuredContent"];
                if state["paint_busy"] == false && state["brush"].is_object() {
                    return state.clone();
                }
            }
            assert!(Instant::now() < deadline, "Painting workspace stayed busy");
        }
    };
    let edit = |client: &mut StdioClient, app: &mut App, set, edit| {
        client.tool(app, "edit_paint", json!({"model_id": 0, "set": set, "edit": edit}))
            ["structuredContent"].clone()
    };
    assert_eq!(ready(&mut client, &mut app)["texture_sets"].as_array().unwrap().len(), 2);
    edit(&mut client, &mut app, 0, json!({"operation":"add_layer", "kind":"Paint", "name":"First edit"}));
    ready(&mut client, &mut app);
    edit(&mut client, &mut app, 0, json!({"operation":"select_set", "index":1}));
    ready(&mut client, &mut app);
    edit(&mut client, &mut app, 1, json!({"operation":"add_layer", "kind":"Paint", "name":"Second edit"}));
    ready(&mut client, &mut app);
    edit(&mut client, &mut app, 1, json!({"operation":"select_set", "index":0}));
    let history = ready(&mut client, &mut app);
    assert_eq!(history["next_undo_set"], 1);
    assert_eq!(history["undo_history"], json!([
        {"set": 1, "operation": "add_layer", "label": "Добавление слоя"},
        {"set": 0, "operation": "add_layer", "label": "Добавление слоя"}
    ]));

    for (from, operation, to, layer, restored) in [
        (0, "undo", 1, "Second edit", false),
        (1, "undo", 0, "First edit", false),
        (0, "redo", 0, "First edit", true),
        (0, "redo", 1, "Second edit", true),
    ] {
        let state = edit(&mut client, &mut app, from, json!({"operation": operation}));
        assert_eq!(state["active_set"], to);
        assert_eq!(state["layers_bottom_to_top"].as_array().unwrap().iter().any(|row| row["name"] == layer), restored);
        ready(&mut client, &mut app);
    }
    let undone = edit(&mut client, &mut app, 1, json!({"operation": "undo", "steps": 3}));
    assert_eq!(undone["history_steps_requested"], 3);
    assert_eq!(undone["history_steps_applied"], 2);
    assert_eq!(undone["active_set"], 0);
    assert_eq!(undone["next_undo_set"], Value::Null);
    assert_eq!(undone["redo_history"][0]["operation"], "add_layer");
    assert_eq!(undone["redo_history"][0]["set"], 0);
    assert!(!undone["layers_bottom_to_top"].as_array().unwrap().iter().any(|row| row["name"] == "First edit"));

    let redone = edit(&mut client, &mut app, 0, json!({"operation": "redo", "steps": 2}));
    assert_eq!(redone["history_steps_applied"], 2);
    assert_eq!(redone["active_set"], 1);
    assert_eq!(redone["next_redo_set"], Value::Null);
    assert!(redone["layers_bottom_to_top"].as_array().unwrap().iter().any(|row| row["name"] == "Second edit"));
    edit(&mut client, &mut app, 1, json!({"operation": "select_set", "index": 0}));
    let first = ready(&mut client, &mut app);
    assert!(first["layers_bottom_to_top"].as_array().unwrap().iter().any(|row| row["name"] == "First edit"));
    client.close();
}

#[test]
#[ignore = "requires GPU and a built bevy-editor-mcp binary; opens imported SPP and MPP projects"]
fn gpu_stdio_imports_paint_projects_in_both_formats() {
    use crate::editor::{model_document::glb::Glb, model_paint_spp::fixture};
    use std::{fs, path::Path};

    let (scratch, mut app, _server, mut client) = stdio_paint_app();
    let models = scratch.0.join("assets/models");
    let original = fs::read(models.join("block.glb")).unwrap();
    fs::write(models.join("Sample_SPP.glb"), &original).unwrap();
    let source = scratch.0.join("Sample.SPP");
    let cache = format!("texture-cache/4242/1/{}", subcraft_spp::document::channel_bits::ROUGHNESS);
    let spp = fixture::hdf5_file(&[
        ("paint/scene.assbin", fixture::assbin_quad()),
        ("paint/document.bin", fixture::document(0)),
        (&cache, fixture::cache_blob(4, 2, &[120, 255].repeat(8))),
    ]);
    fs::write(&source, &spp).unwrap();
    let broken = scratch.0.join("Broken.spp");
    fs::write(&broken, b"broken").unwrap();
    let failed = client.request(&mut app, "tools/call", json!({
        "name": "import_paint_project", "arguments": {"source": broken, "folder": "models"},
    }));
    assert_eq!(failed["isError"], true, "{failed}");
    assert!(!models.join("Broken_SPP.glb").exists());

    let reply = client.tool(&mut app, "import_paint_project", json!({"source": source, "folder": "models"}));
    let result = &reply["structuredContent"];
    assert_eq!(result["status"], "opening");
    assert_eq!(Path::new(result["asset"].as_str().unwrap()), Path::new("models/Sample_SPP_2.glb"));
    assert!(result["report"]["warnings"].is_array());
    assert_eq!(fs::read(&source).unwrap(), spp);
    assert_eq!(fs::read(models.join("Sample_SPP.glb")).unwrap(), original);
    let opened = Glb::load(&models.join("Sample_SPP_2.glb")).unwrap();
    let surface = opened.paint_surface(&opened.json["meshes"][0]["primitives"][0]).unwrap().unwrap();
    assert!(surface.stack.layers.iter().any(|layer| layer.name == "Rust"));
    assert_eq!(surface.composite(1)[..4], [120, 120, 120, 255]);
    let started = Instant::now();
    loop {
        let state = client.request(&mut app, "tools/call", json!({"name": "get_paint_state", "arguments": {}}));
        if state["structuredContent"]["mode"] == "paint" {
            assert_eq!(state["structuredContent"]["model_id"], result["model_id"]);
            break;
        }
        assert!(started.elapsed() < REQUEST_TIMEOUT);
    }

    let source = scratch.0.join("Sample.mpp");
    fs::write(&source, crate::editor::model_paint_mpp::mcp_fixture()).unwrap();
    let reply = client.tool(&mut app, "import_paint_project", json!({"source": source, "folder": "models"}));
    assert_eq!(Path::new(reply["structuredContent"]["asset"].as_str().unwrap()), Path::new("models/Sample_MPP.glb"));
    assert!(fs::read_dir(&models).unwrap().all(|entry| !entry.unwrap().file_name().to_string_lossy().starts_with(".paint-import-")));
    client.close();
}

#[test]
#[ignore = "requires GPU and a built bevy-editor-mcp binary; tests the complete stdio paint workflow"]
fn gpu_stdio_paint_workflow_saves_and_exports() {
    use crate::editor::{
        model_document::{Mode, glb::Glb},
        model_mesh::{Corner, Face, Geometry},
        selection_outline::render_tests::frame,
    };
    use std::{fs, path::Path};

    let (scratch, mut app, server, mut client) = stdio_paint_app();
    let project = app.world().resource::<Editor>().project.clone();
    let tools = client.request(&mut app, "tools/list", json!({}));
    for name in [
        "open_paint_model",
        "import_paint_project",
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
    let baked = client.tool(
        &mut app,
        "bake_maps",
        json!({
            "target": {"model_id": 0, "set": 0}, "quality": "draft", "maps": ["part_random"], "wait": true,
            "rays": {"occlusion_samples": 24}, "surface": {"grid": 2},
            "prefer_front_faces": false, "average_normals": false,
        }),
    );
    let baked = &baked["structuredContent"];
    assert_eq!(baked["status"], "ready");
    assert_eq!(baked["settings"]["rays"]["occlusion_samples"], 24);
    assert_eq!(baked["settings"]["surface"]["grid"], 2);
    assert_eq!(baked["settings"]["prefer_front_faces"], false);
    assert_eq!(baked["settings"]["average_normals"], false);
    assert!(baked["maps"].as_array().unwrap().contains(&json!("part_random")));

    wait_for_paint(&mut client, &mut app);
    let cached = client.tool(&mut app, "bake_maps", json!({"target": {"model_id": 0}, "wait": true}));
    assert_eq!(cached["structuredContent"]["status"], "ready");
    assert_eq!(cached["structuredContent"]["maps"], baked["maps"]);
    let export_baked = |client: &mut StdioClient, app: &mut App| {
        let reply = client.tool(app, "export_textures", json!({"model_id":0,"folder":".",
            "template":"baked","format":"png","bit_depth":8,"padding":"none","size":64
        }));
        let file = reply["structuredContent"]["files"].as_array().unwrap().iter()
            .filter_map(Value::as_str).find(|file| file.contains("PartRandom")).unwrap();
        image::open(file).unwrap().to_rgba8()
    };
    let original_bake = export_baked(&mut client, &mut app);
    let center = original_bake.get_pixel(32, 32).0;
    assert!(center[0] > 0 && center[3] == 255, "{center:?}");
    let set_levels = |client: &mut StdioClient, app: &mut App, patch: Value| {
        let mut args = json!({"target":{"model_id":0,"set":0}, "map":"part_random"});
        args.as_object_mut().unwrap().extend(patch.as_object().unwrap().clone());
        client.tool(app, "set_bake_levels", args)["structuredContent"]["levels"][0].clone()
    };
    let adjust = set_levels(&mut client, &mut app, json!({"black_percent":100,"gamma_percent":160}));
    assert_eq!(adjust, json!({"map":"part_random","black_percent":100,"white_percent":100,"gamma_percent":160}));
    let invalid_levels = client.request(&mut app, "tools/call", json!({
        "name":"set_bake_levels", "arguments":{"target":{"model_id":0}, "map":"part_random",
            "black_percent":10,"gamma_percent":0}
    }));
    assert_eq!(invalid_levels["isError"], true, "{invalid_levels}");
    let status = client.tool(&mut app, "get_bake_state", json!({"model_id":0,"set":0}));
    assert_eq!(status["structuredContent"]["levels"][0], adjust);
    assert_eq!(export_baked(&mut client, &mut app).get_pixel(32, 32).0, [0,0,0,255]);
    let graded = client.tool(&mut app, "save_model", json!({"model_id":0,"copy_name":"bake-levels.glb"}));
    let graded = Glb::load(Path::new(graded["structuredContent"]["path"].as_str().unwrap())).unwrap();
    let primitives = graded.json["meshes"].as_array().unwrap().iter()
        .flat_map(|mesh| mesh["primitives"].as_array().unwrap());
    assert!(primitives.into_iter().any(|primitive| {
        primitive["extras"]["bevy_editor_bake_levels"] == json!([["part_random",{"black":100,"white":100,"gamma":160}]])
    }));
    assert_eq!(set_levels(&mut client, &mut app, json!({"reset":true}))["gamma_percent"], 100);
    assert_eq!(export_baked(&mut client, &mut app), original_bake);

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
    let invalid_style = client.request(&mut app, "tools/call", json!({
        "name": "paint_stroke", "arguments": {
            "workspace": {"model_id": 0, "set": 0}, "target": {"layer": layer, "space": "uv"},
            "samples": [{"position": [0.5, 0.5]}], "style": {"paint_through": true},
        }
    }));
    assert_eq!(invalid_style["isError"], true, "{invalid_style}");
    let still_before = wait_for_paint(&mut client, &mut app);
    assert_eq!(still_before["undo_steps"], before["undo_steps"]);
    let paint = client.tool(&mut app, "paint_stroke", json!({
        "workspace": {"model_id": 0, "set": 0}, "target": {"layer": layer, "space": "uv"},
        "samples": [{"position": [0.5, 0.5]}],
        "brush": {"size_px": 96, "channels": {"base_color": [210, 10, 170]}, "hardness_percent": 100, "flow_percent": 100},
        "style": {"tip": "Квадрат", "tip_angle_degrees": 0, "pressure_size": false, "pressure_flow": false},
    }));
    assert_eq!(paint["structuredContent"]["paint_busy"], true);
    let painted = wait_for_paint(&mut client, &mut app);
    assert_eq!(
        painted["undo_steps"].as_u64().unwrap(),
        before["undo_steps"].as_u64().unwrap() + 1
    );
    assert_eq!(painted["brush"], before["brush"]);
    {
        let editing = app.world().resource::<editing::State>();
        let surface = editing.documents[&0].objects[0].materials[0].paint.as_ref().unwrap();
        let pixels = surface.stack.layers[layer].channel(0).unwrap();
        let side = surface.size as usize;
        let center = side / 2;
        let alpha = |x, y| pixels[(y * side + x) * 4 + 3];
        assert!(alpha(center + 19, center + 19) > 200, "square tip must paint inside its corner");
        assert!(alpha(center + 38, center) < 10, "the tip's transparent margin sits inside the round radius");
    }
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
    let original = client.tool(&mut app, "set_paint_view", json!({"model_id": 0}));
    let render = |client: &mut StdioClient, app: &mut App, settings| {
        let reply = client.tool(app, "render_paint_view", settings);
        let png = reply["content"].as_array().unwrap().iter().find(|item| item["type"] == "image").unwrap();
        let bytes = base64::engine::general_purpose::STANDARD.decode(png["data"].as_str().unwrap()).unwrap();
        image::load_from_memory(&bytes).unwrap().to_rgba8()
    };
    let independent = render(&mut client, &mut app, json!({"model_id": 0,
        "width": 173, "height": 123, "transparent": true,
        "camera": {"center": [0, 0, 0], "yaw_degrees": 0, "pitch_degrees": 0, "distance": 4, "orthographic": true}
    }));
    assert_eq!(independent.dimensions(), (173, 123));
    assert_eq!(independent.get_pixel(0, 0)[3], 0);
    assert_eq!(independent.get_pixel(86, 61)[3], 255);
    assert!(independent.pixels().any(|pixel| pixel[0] > pixel[1].saturating_add(35)
        && pixel[2] > pixel[1].saturating_add(25)));
    let diagnostic = |client: &mut StdioClient, app: &mut App, preview| {
        render(client, app, json!({"model_id": 0,
            "width": 173, "height": 123, "transparent": true, "preview": preview,
            "camera": {"center": [0, 0, 0], "yaw_degrees": 0, "pitch_degrees": 0, "distance": 4, "orthographic": true}
        }))
    };
    let channel = diagnostic(&mut client, &mut app, json!({"mode": "channel", "channel": "roughness"}));
    let baked = diagnostic(&mut client, &mut app, json!({"mode": "baked", "map": "part_random"}));
    for output in [&channel, &baked] {
        let [red, green, blue, alpha] = output.get_pixel(86, 61).0;
        assert_eq!(alpha, 255);
        assert!(red.abs_diff(green) <= 3 && red.abs_diff(blue) <= 3, "{red} {green} {blue}");
        assert_eq!(output.get_pixel(0, 0)[3], 0);
    }
    assert_ne!(channel.get_pixel(86, 61), baked.get_pixel(86, 61));
    let shifted = render(&mut client, &mut app, json!({"model_id": 0,
        "width": 173, "height": 123, "transparent": true,
        "camera": {"center": [4, 0, 0], "yaw_degrees": 0, "pitch_degrees": 0, "distance": 4, "orthographic": true}
    }));
    assert_eq!(shifted.get_pixel(86, 61)[3], 0);
    let opaque = render(&mut client, &mut app, json!({"model_id": 0, "width": 1731, "height": 311}));
    assert_eq!(opaque.dimensions(), (1731, 311));
    assert_eq!(opaque.get_pixel(0, 0)[3], 255);
    let invalid_render = client.request(&mut app, "tools/call", json!({
        "name": "render_paint_view", "arguments": {"model_id": 0, "width": 4097}
    }));
    assert_eq!(invalid_render["isError"], true, "{invalid_render}");
    let invalid = client.request(&mut app, "tools/call", json!({
        "name": "set_paint_view", "arguments": {"model_id": 0, "yaw_degrees": 45, "post": {"contrast": 5}}
    }));
    assert_eq!(invalid["isError"], true, "{invalid}");
    let unchanged = client.tool(&mut app, "set_paint_view", json!({"model_id": 0}));
    assert_eq!(original["structuredContent"], unchanged["structuredContent"]);
    let post = client.tool(&mut app, "set_paint_view", json!({
        "model_id": 0, "post": {"saturation": 0.0, "tone_map": "linear"},
    }));
    assert_eq!(post["structuredContent"]["post"]["tone_map"], "linear");
    assert_eq!(post["structuredContent"]["post"]["effects"]["saturation"], 0.0);
    let post = client.tool(&mut app, "set_paint_view", json!({
        "model_id": 0, "post": {"bloom_threshold": 2.0, "inherit_tone_map": true},
    }));
    assert!(post["structuredContent"]["post"]["effects"]["curve"].is_null());
    assert_eq!(post["structuredContent"]["post"]["effects"]["saturation"], 0.0);
    assert_eq!(post["structuredContent"]["post"]["effects"]["bloom_threshold"], 2.0);
    let gray = client.tool(&mut app, "capture_paint_view", json!({"model_id": 0}));
    let png = gray["content"].as_array().unwrap().iter().find(|item| item["type"] == "image").unwrap();
    let bytes = base64::engine::general_purpose::STANDARD.decode(png["data"].as_str().unwrap()).unwrap();
    let gray = image::load_from_memory(&bytes).unwrap().to_rgba8();
    assert_eq!(gray.dimensions(), image.dimensions());
    assert!(image.pixels().zip(gray.pixels()).any(|(before, after)| {
        before[0] > before[1].saturating_add(35) && before[2] > before[1].saturating_add(25)
            && after[0].abs_diff(after[1]) < 4 && after[0].abs_diff(after[2]) < 4
    }));

    wait_for_paint(&mut client, &mut app);
    let settings = client.tool(&mut app, "set_paint_view", json!({
        "model_id": 0, "wireframe": true,
        "uv_overlay": {"show_wire": true, "wire_opacity": 0.75, "show_seams_2d": true, "show_seams_3d": true},
        "material": {"height_strength": 2, "emission_strength": 4, "ao_strength": 0.25, "use_baked_normal": true},
    }));
    let settings = &settings["structuredContent"];
    assert_eq!(settings["wireframe"], true);
    assert_eq!(settings["uv_overlay"]["show_seams_3d"], true);
    assert_eq!(settings["material"]["height_strength"], 2.0);
    wait_for_paint(&mut client, &mut app);
    let invalid = client.request(&mut app, "tools/call", json!({
        "name": "set_paint_view", "arguments": {"model_id": 0, "wireframe": false,
            "uv_overlay": {"wire_opacity": 0.01}, "material": {"emission_strength": 0}}
    }));
    assert_eq!(invalid["isError"], true, "{invalid}");
    assert!(invalid.to_string().contains("wire_opacity must be between"), "{invalid}");
    let unchanged = client.tool(&mut app, "set_paint_view", json!({"model_id": 0}));
    assert_eq!(unchanged["structuredContent"], *settings);
    let modified = client.tool(&mut app, "set_paint_view", json!({
        "model_id": 0, "material": {"automatic_height": true},
    }));
    assert!(modified["structuredContent"]["material"]["height_strength"].is_null());
    assert_eq!(modified["structuredContent"]["material"]["emission_strength"], 4.0);
    wait_for_paint(&mut client, &mut app);

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
    let selection = client.tool(&mut app, "select_surface", json!({
        "workspace": {"model_id": 0, "set": 0}, "layer": layer,
        "source": {"kind": "point", "position": [0, 0, 0]}, "mode": "replace", "wait": true,
    }))["structuredContent"].clone();
    assert_eq!(selection["paint_busy"], false);
    assert_eq!(selection["surface_selection"]["status"], "complete", "{selection}");
    assert_eq!(selection["surface_selection"]["triangles_selected"], 2);
    assert_eq!(selection["undo_steps"].as_u64().unwrap(), reopened["undo_steps"].as_u64().unwrap() + 1);
    let waited_error = client.request(&mut app, "tools/call", json!({
        "name": "select_surface", "arguments": {
            "workspace": {"model_id": 0, "set": 0}, "layer": layer, "wait": true,
            "source": {"kind": "parts", "indices": [1]}, "mode": "subtract",
        },
    }));
    assert_eq!(waited_error["isError"], true, "{waited_error}");
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

    let fill = client.tool(&mut app, "edit_paint", json!({
        "model_id": 0, "set": 0, "edit": {"operation": "add_layer", "kind": "Fill"},
    }));
    let fill_index = fill["structuredContent"]["layers_bottom_to_top"].as_array().unwrap().iter()
        .find(|row| row["selected"] == true).unwrap()["index"].as_u64().unwrap() as usize;
    let before_fill = wait_for_paint(&mut client, &mut app);
    let map_path = project.asset_root.join("models/stdio-roughness.png");
    image::RgbaImage::from_pixel(4, 4, image::Rgba([112, 112, 112, 255])).save(&map_path).unwrap();
    let invalid = client.request(&mut app, "tools/call", json!({
        "name": "edit_paint", "arguments": {
            "model_id": 0, "set": 0, "edit": {
                "operation": "update_layer", "index": fill_index, "name": "Invalid fill",
                "fill": {"images": {"base_color": "models/stdio-roughness.png", "roughness": "models/missing.png"}}
            }
        },
    }));
    assert_eq!(invalid["isError"], true, "{invalid}");
    let unchanged = wait_for_paint(&mut client, &mut app);
    assert_eq!(unchanged["undo_steps"], before_fill["undo_steps"]);
    assert_eq!(unchanged["layers_bottom_to_top"][fill_index], before_fill["layers_bottom_to_top"][fill_index]);
    let updated = client.tool(&mut app, "edit_paint", json!({
        "model_id": 0, "set": 0, "edit": {
            "operation": "update_layer", "index": fill_index, "name": "MCP Patina",
            "visible": false, "fill": {
                "channels": {"base_color": [38, 96, 118]},
                "images": {"roughness": "models/stdio-roughness.png"},
                "projection": "Triplanar", "tiling": 2.5, "rotation": 15,
                "variation": {"color": 0.15, "seed": 10}
            }
        },
    }));
    assert_eq!(updated["structuredContent"]["undo_steps"].as_u64().unwrap(),
        before_fill["undo_steps"].as_u64().unwrap() + 1);
    let updated = wait_for_paint(&mut client, &mut app);
    assert_eq!(updated["layers_bottom_to_top"][fill_index]["fill"]["settings"]["tiling"], 2.5);
    assert_eq!(updated["layers_bottom_to_top"][fill_index]["fill"]["textured_channels"], json!(["roughness"]));
    client.tool(&mut app, "edit_paint", json!({
        "model_id": 0, "set": 0, "edit": {
            "operation": "update_layer", "index": fill_index,
            "fill": {"images": {"roughness": null}, "channels": {"roughness": 76}}
        },
    }));
    let cleared = wait_for_paint(&mut client, &mut app);
    assert_eq!(cleared["layers_bottom_to_top"][fill_index]["fill"]["textured_channels"], json!([]));
    client.tool(&mut app, "edit_paint", json!({
        "model_id": 0, "set": 0, "edit": {"operation": "undo"},
    }));
    let restored = wait_for_paint(&mut client, &mut app);
    assert_eq!(restored["layers_bottom_to_top"][fill_index], updated["layers_bottom_to_top"][fill_index]);
    fs::remove_file(&map_path).unwrap();

    let saved = client.tool(
        &mut app,
        "save_model",
        json!({"model_id": 0, "copy_name": "stdio-painted.glb"}),
    );
    let path = saved["structuredContent"]["path"].as_str().unwrap();
    let glb = Glb::load(Path::new(path)).unwrap();
    let saved_post = &glb.json["extras"]["bevy_editor_paint_post"]["effects"];
    assert_eq!(saved_post["saturation"], 0.0);
    assert_eq!(saved_post["bloom_threshold"], 2.0);
    let saved_uv = &glb.json["extras"]["bevy_editor_paint_uv"];
    assert_eq!(saved_uv["show_wire_3d"], true);
    assert_eq!(saved_uv["wire_opacity"], 0.75);
    assert_eq!(saved_uv["show_seams_2d"], true);
    let saved_material = &glb.json["extras"]["bevy_editor_paint_preview"];
    assert!(saved_material["height_strength"].is_null());
    assert_eq!(saved_material["emission_strength"], 4.0);
    assert_eq!(saved_material["ao_strength"], 0.25);
    assert_eq!(saved_material["use_baked_normal"], true);
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
    let patched = &surface.stack.layers[fill_index];
    assert_eq!(patched.name, "MCP Patina");
    assert_eq!(&patched.channels[0].as_ref().unwrap()[..4], &[38, 96, 118, 255]);
    let patched_fill = patched.texture_fill.as_ref().unwrap();
    let settings = patched_fill.settings;
    assert_eq!(settings.tiling, 2.5);
    assert_eq!(settings.rotation, 15.);
    assert_eq!(settings.variation.seed, 10.);
    let roughness = patched_fill.images[1].as_ref().unwrap();
    assert_eq!(roughness.name, "stdio-roughness.png");
    assert_eq!(roughness.pixels().get_pixel(1, 1).0, [112, 112, 112, 255]);

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

    let recipe_file = project.asset_root.join("stdio-export-template.json");
    fs::write(&recipe_file, br#"[
        {"name":"Other","maps":[{"name":"Other","layout":"Grayscale","channels":[{"src":"Zero"}]}]},
        {"name":"Packed Stdio","maps":[{"name":"$mesh_packed","layout":"RGBA","force_16":true,
          "channels":[{"src":"One"},{"src":"Zero"},{"src":"One"},{"src":"One"}]}]}
    ]"#).unwrap();
    let without_name = client.request(&mut app, "tools/call", json!({
        "name":"export_textures", "arguments":{"model_id":0,"folder":".","template_file":"stdio-export-template.json"}
    }));
    assert_eq!(without_name["isError"], true, "{without_name}");
    let packed = client.tool(&mut app, "export_textures", json!({
        "model_id":0,"folder":".","template_file":"stdio-export-template.json", "template_name":"packed stdio",
        "prefix":"RecipeCheck", "size":64, "format":"png", "bit_depth":8,
        "normal":{"use_baked":false,"height_strength":0}
    }));
    let files = packed["structuredContent"]["files"].as_array().unwrap();
    assert_eq!(files.len(), 1);
    let packed_path = Path::new(files[0].as_str().unwrap());
    assert_eq!(packed_path.file_name().unwrap(), "RecipeCheck_packed.png");
    let packed = image::open(packed_path).unwrap();
    let packed = packed.as_rgba16().unwrap();
    assert_eq!(packed.dimensions(), (64,64));
    assert_eq!(packed.get_pixel(32,32).0, [65535,0,65535,65535]);

    let catalog = client.tool(&mut app, "list_paint_presets", json!({}));
    let catalog = &catalog["structuredContent"];
    let texture = catalog["textures"][0].as_str().unwrap();
    assert!(catalog["smart_materials"].as_array().unwrap().contains(&json!("Rusty Iron")));
    let original_layers = wait_for_paint(&mut client, &mut app);
    let misplaced = client.request(&mut app, "tools/call", json!({
        "name": "edit_paint", "arguments": {
            "model_id": 0, "set": 0, "edit": {"operation": "add_smart_material", "preset": "Rusty Iron",
                "target": {"layer": fill_index, "placement": "inside"}}
        },
    }));
    assert_eq!(misplaced["isError"], true, "{misplaced}");
    let unchanged = wait_for_paint(&mut client, &mut app);
    assert_eq!(unchanged["layers_bottom_to_top"], original_layers["layers_bottom_to_top"]);
    assert_eq!(unchanged["undo_steps"], original_layers["undo_steps"]);
    let edit = |client: &mut StdioClient, app: &mut App, edit: Value| {
        client.tool(app, "edit_paint", json!({"model_id": 0, "set": 0, "edit": edit}));
        wait_for_paint(client, app)
    };
    let smart = edit(&mut client, &mut app, json!({
        "operation": "add_smart_material", "preset": "Rusty Iron", "name": "Rust group",
        "target": {"layer": fill_index, "placement": "above"}
    }));
    let folder = smart["layers_bottom_to_top"].as_array().unwrap().iter()
        .find(|row| row["name"] == "Rust group").unwrap();
    let folder_index = folder["index"].as_u64().unwrap() as usize;
    let old_children = folder["layers_inside"].as_u64().unwrap();
    assert!(old_children > 1);
    assert_eq!(folder["selected"], true);
    assert_eq!(folder_index, fill_index + old_children as usize + 1);
    assert_eq!(smart["undo_steps"].as_u64().unwrap(), original_layers["undo_steps"].as_u64().unwrap() + 1);

    let textured = edit(&mut client, &mut app, json!({
        "operation": "add_texture", "preset": texture.to_lowercase(), "name": "Tiled relief",
        "target": {"layer": folder_index, "placement": "inside"}
    }));
    let rows = textured["layers_bottom_to_top"].as_array().unwrap();
    let relief = &rows[folder_index];
    assert_eq!(relief["name"], "Tiled relief");
    assert_eq!(relief["selected"], true);
    assert_eq!(relief["folder"].as_u64().unwrap() as usize, folder_index + 1);
    assert_eq!(relief["fill"]["textured_channels"], json!(["height"]));
    assert_eq!(relief["fill"]["settings"]["projection"], "Triplanar");
    assert_eq!(relief["fill"]["settings"]["tiling"], 2.0);
    assert_eq!(rows[folder_index + 1]["layers_inside"].as_u64().unwrap(), old_children + 1);
    assert_eq!(textured["undo_steps"].as_u64().unwrap(), smart["undo_steps"].as_u64().unwrap() + 1);

    let undone = edit(&mut client, &mut app, json!({"operation":"undo"}));
    assert_eq!(undone["layers_bottom_to_top"], smart["layers_bottom_to_top"]);
    let undone = edit(&mut client, &mut app, json!({"operation":"undo"}));
    assert_eq!(undone["layers_bottom_to_top"], original_layers["layers_bottom_to_top"]);
    edit(&mut client, &mut app, json!({"operation":"redo"}));
    let redone = edit(&mut client, &mut app, json!({"operation":"redo"}));
    assert_eq!(redone["layers_bottom_to_top"], textured["layers_bottom_to_top"]);
    let generated = edit(&mut client, &mut app, json!({
        "operation":"apply_generator", "index":folder_index, "generator":"Clouds",
        "parameters":{"controls":{"0":73,"2":5}, "weights":{"noise":150}, "invert":true}
    }));
    let generator = &generated["layers_bottom_to_top"][folder_index]["mask"]["generator"];
    assert_eq!(generator["values"][0], 73.0);
    assert_eq!(generator["invert"], true);
    assert_eq!(generated["undo_steps"].as_u64().unwrap(), redone["undo_steps"].as_u64().unwrap() + 1);
    let invalid_generator = client.request(&mut app, "tools/call", json!({
        "name":"edit_paint", "arguments":{"model_id":0, "set":0, "edit":{
            "operation":"update_generator", "index":folder_index, "parameters":{"invert":false,"weights":{"noise":500}}
        }}
    }));
    assert_eq!(invalid_generator["isError"], true, "{invalid_generator}");
    let unchanged = wait_for_paint(&mut client, &mut app);
    assert_eq!(unchanged["layers_bottom_to_top"], generated["layers_bottom_to_top"]);
    assert_eq!(unchanged["undo_steps"], generated["undo_steps"]);
    let updated_generator = edit(&mut client, &mut app, json!({
        "operation":"update_generator", "index":folder_index,
        "parameters":{"controls":{"0":62}, "invert":false}
    }));
    assert_eq!(updated_generator["undo_steps"].as_u64().unwrap(), generated["undo_steps"].as_u64().unwrap() + 1);
    let undone = edit(&mut client, &mut app, json!({"operation":"undo"}));
    assert_eq!(undone["layers_bottom_to_top"], generated["layers_bottom_to_top"]);
    let redone = edit(&mut client, &mut app, json!({"operation":"redo"}));
    assert_eq!(redone["layers_bottom_to_top"], updated_generator["layers_bottom_to_top"]);
    let levels = edit(&mut client, &mut app, json!({
        "operation":"add_mask_filter", "index":folder_index, "filter":"Levels",
        "parameters":{"controls":{"3":10,"4":80}}
    }));
    assert_eq!(levels["undo_steps"].as_u64().unwrap(), redone["undo_steps"].as_u64().unwrap() + 1);
    let blur = edit(&mut client, &mut app, json!({
        "operation":"add_mask_filter", "index":folder_index, "filter":"Blur",
        "parameters":{"controls":{"0":3,"1":1}}
    }));
    let invalid_filter = client.request(&mut app, "tools/call", json!({
        "name":"edit_paint", "arguments":{"model_id":0, "set":0, "edit":{
            "operation":"update_mask_filter", "index":folder_index, "filter_index":1,
            "parameters":{"enabled":false, "controls":{"1":0.5}}
        }}
    }));
    assert_eq!(invalid_filter["isError"], true, "{invalid_filter}");
    let unchanged = wait_for_paint(&mut client, &mut app);
    assert_eq!(unchanged["layers_bottom_to_top"], blur["layers_bottom_to_top"]);
    assert_eq!(unchanged["undo_steps"], blur["undo_steps"]);
    let moved = edit(&mut client, &mut app, json!({
        "operation":"move_mask_filter", "index":folder_index, "from":1,"to":0
    }));
    assert_eq!(moved["layers_bottom_to_top"][folder_index]["mask"]["filters"][0]["kind"], "Blur");
    let disabled = edit(&mut client, &mut app, json!({
        "operation":"update_mask_filter", "index":folder_index, "filter_index":0,
        "parameters":{"enabled":false}
    }));
    let removed = edit(&mut client, &mut app, json!({
        "operation":"remove_mask_filter", "index":folder_index, "filter_index":1
    }));
    assert_eq!(removed["layers_bottom_to_top"][folder_index]["mask"]["filter_count"], 1);
    let restored_filters = edit(&mut client, &mut app, json!({"operation":"undo"}));
    assert_eq!(restored_filters["layers_bottom_to_top"], disabled["layers_bottom_to_top"]);
    let fill_mask = edit(&mut client, &mut app, json!({
        "operation":"add_mask_layer", "index":folder_index, "source":{"kind":"fill"},
        "parameters":{"value_percent":35,"opacity_percent":75,"blend":"Add"}
    }));
    assert_eq!(fill_mask["undo_steps"].as_u64().unwrap(), restored_filters["undo_steps"].as_u64().unwrap() + 1);
    let procedural_mask = edit(&mut client, &mut app, json!({
        "operation":"add_mask_layer", "index":folder_index,
        "source":{"kind":"generator","generator":"Clouds","parameters":{"controls":{"0":58}}}
    }));
    assert_eq!(procedural_mask["layers_bottom_to_top"][folder_index]["mask"]["layers_bottom_to_top"][1]["blend"], "Multiply");
    let invalid_layer = client.request(&mut app, "tools/call", json!({
        "name":"edit_paint", "arguments":{"model_id":0,"set":0,"edit":{
            "operation":"update_mask_layer","index":folder_index,"mask_layer":1,
            "parameters":{"enabled":false,"opacity_percent":101}
        }}
    }));
    assert_eq!(invalid_layer["isError"], true, "{invalid_layer}");
    let unchanged = wait_for_paint(&mut client, &mut app);
    assert_eq!(unchanged["layers_bottom_to_top"], procedural_mask["layers_bottom_to_top"]);
    assert_eq!(unchanged["undo_steps"], procedural_mask["undo_steps"]);
    edit(&mut client, &mut app, json!({"operation":"move_mask_layer","index":folder_index,"from":1,"to":0}));
    edit(&mut client, &mut app, json!({
        "operation":"update_generator","index":folder_index,"mask_layer":0,"parameters":{"controls":{"0":63}}
    }));
    let updated_mask = edit(&mut client, &mut app, json!({
        "operation":"update_mask_layer","index":folder_index,"mask_layer":1,"parameters":{"enabled":false}
    }));
    let removed_mask = edit(&mut client, &mut app, json!({"operation":"remove_mask_layer","index":folder_index,"mask_layer":0}));
    assert_eq!(removed_mask["layers_bottom_to_top"][folder_index]["mask"]["layer_count"], 1);
    let restored_mask = edit(&mut client, &mut app, json!({"operation":"undo"}));
    assert_eq!(restored_mask["layers_bottom_to_top"], updated_mask["layers_bottom_to_top"]);
    let named_group = edit(&mut client, &mut app, json!({
        "operation": "add_layer", "kind": "Folder", "name": "Surface details",
        "target": {"layer": folder_index + 1, "placement": "above"}
    }));
    let group = folder_index + 2;
    assert_eq!(named_group["layers_bottom_to_top"][group]["name"], "Surface details");
    let named_fill = edit(&mut client, &mut app, json!({
        "operation": "add_material", "preset": catalog["materials"][0], "name": "Base metal",
        "target": {"layer": group, "placement": "inside"}
    }));
    assert_eq!(named_fill["layers_bottom_to_top"][group]["name"], "Base metal");
    assert_eq!(named_fill["layers_bottom_to_top"][group]["folder"], group + 1);
    let saved = client.tool(&mut app, "save_model", json!({"model_id": 0, "copy_name": "stdio-materials.glb"}));
    let glb = Glb::load(Path::new(saved["structuredContent"]["path"].as_str().unwrap())).unwrap();
    let surface = glb.json["meshes"].as_array().unwrap().iter()
        .flat_map(|mesh| mesh["primitives"].as_array().into_iter().flatten())
        .find_map(|primitive| glb.paint_surface(primitive).unwrap()).unwrap();
    assert_eq!(surface.stack.parents(folder_index).next(), Some(folder_index + 1));
    assert_eq!(surface.stack.layers[folder_index + 1].name, "Rust group");
    assert_eq!(surface.stack.layers[group].name, "Base metal");
    assert_eq!(surface.stack.layers[group + 1].name, "Surface details");
    assert_eq!(surface.stack.parents(group).next(), Some(group + 1));
    let height = surface.stack.layers[folder_index].texture_fill.as_ref().unwrap().images[3].as_ref().unwrap();
    assert_eq!(height.name, format!("{texture}.png"));
    assert_eq!(height.pixels().dimensions(), (1024, 1024));
    let generator = surface.stack.layers[folder_index].mask.as_ref().unwrap().generator.unwrap();
    assert_eq!((generator.values[0], generator.values[2], generator.invert), (62.0, 5.0, false));
    assert_eq!(generator.weight(crate::editor::model_paint_data::mask::GeneratorInput::Noise), 150);
    let filters = &surface.stack.layers[folder_index].mask.as_ref().unwrap().filters;
    assert_eq!(filters.len(), 2);
    assert_eq!(filters[0].kind, crate::editor::model_paint_data::mask::FilterKind::Blur);
    assert!(!filters[0].enabled && filters[0].wrap);
    assert_eq!(filters[1].output, [10.0, 80.0]);
    let mask_layers = &surface.stack.layers[folder_index].mask.as_ref().unwrap().layers;
    assert_eq!(mask_layers.len(), 2);
    assert_eq!(mask_layers[0].generator.unwrap().value(0), 63.0);
    assert_eq!((mask_layers[1].value, mask_layers[1].opacity, mask_layers[1].enabled), (35, 75, false));
    let reset = client.tool(&mut app, "set_paint_view", json!({"model_id": 0, "reset_post": true}));
    assert_eq!(reset["structuredContent"]["post"]["overridden"], false);
    assert_eq!(reset["structuredContent"]["post"]["effects"], original["structuredContent"]["post"]["effects"]);

    let source = scratch.0.join("Sample.mpp");
    let models = scratch.0.join("assets/models");
    fs::write(&source, crate::editor::model_paint_mpp::mcp_fixture()).unwrap();
    let original_collision = models.join("Sample_MPP.glb");
    let block_bytes = fs::read(models.join("block.glb")).unwrap();
    fs::write(&original_collision, &block_bytes).unwrap();
    let outside = client.request(&mut app, "tools/call", json!({
        "name": "import_paint_project", "arguments": {"source": source, "folder": "../models"}
    }));
    assert_eq!(outside["isError"], true, "{outside}");
    let hidden_folder = scratch.0.join("assets/.hidden");
    fs::create_dir(&hidden_folder).unwrap();
    let hidden = client.request(&mut app, "tools/call", json!({
        "name": "import_paint_project", "arguments": {"source": source, "folder": ".hidden"}
    }));
    assert_eq!(hidden["isError"], true, "{hidden}");
    assert!(fs::read_dir(hidden_folder).unwrap().next().is_none());
    let broken_mpp = scratch.0.join("Broken.mpp");
    fs::write(&broken_mpp, b"broken").unwrap();
    let invalid_import = client.request(&mut app, "tools/call", json!({
        "name": "import_paint_project", "arguments": {"source": broken_mpp, "folder": "models"}
    }));
    assert_eq!(invalid_import["isError"], true, "{invalid_import}");
    assert_eq!(app.world().resource::<Editor>().viewing_model, Some(0));
    assert!(!models.join("Broken_MPP.glb").exists());
    let imported = client.tool(&mut app, "import_paint_project", json!({"source": source, "folder": "models"}));
    let imported = &imported["structuredContent"];
    assert_eq!(Path::new(imported["asset"].as_str().unwrap()), Path::new("models/Sample_MPP_2.glb"));
    assert_eq!(imported["status"], "opening");
    assert!(imported["report"]["warnings"].is_array());
    let model = imported["model_id"].as_u64().unwrap() as usize;
    wait_for_mode(&mut app, model);
    let imported_state = wait_for_paint(&mut client, &mut app);
    assert_eq!(imported_state["resolution"], 128);
    assert_eq!(imported_state["layers_bottom_to_top"][0]["name"], "Imported pigment");
    let rejected = client.request(&mut app, "tools/call", json!({
        "name":"edit_paint", "arguments":{"model_id":model,"set":0,
            "edit":{"operation":"update_texture_set","index":0,"resolution":512,"visible":false,"highlighted":true}}
    }));
    assert_eq!(rejected["isError"], true, "{rejected}");
    let unchanged = wait_for_paint(&mut client, &mut app);
    assert_eq!(unchanged["resolution"], 128);
    assert_eq!(unchanged["texture_sets"][0]["highlighted"], false);
    let invalid_resolution = client.request(&mut app, "tools/call", json!({
        "name": "bake_maps", "arguments": {
            "target": {"model_id": model, "set": 0}, "resolution": 500, "quality": "draft", "wait": true,
        }
    }));
    assert_eq!(invalid_resolution["isError"], true);
    assert_eq!(wait_for_paint(&mut client, &mut app)["resolution"], 128);
    let baked_at_size = client.tool(&mut app, "bake_maps", json!({
        "target": {"model_id": model, "set": 0}, "resolution": 512, "quality": "draft",
        "maps": ["part_random"], "wait": true,
    }));
    let report = &baked_at_size["structuredContent"];
    assert_eq!(report["status"], "ready");
    assert_eq!(report["resolution"], 512);
    assert!(report["report"]["uv_texels"].as_u64().unwrap() > 128 * 128);
    assert!(report["maps"].as_array().unwrap().contains(&json!("part_random")));
    client.tool(&mut app, "edit_paint", json!({
        "model_id":model,"set":0,"edit":{"operation":"update_texture_set","index":0,"highlighted":true}
    }));
    let resized = wait_for_paint(&mut client, &mut app);
    assert_eq!(resized["resolution"], 512);
    assert_eq!(resized["texture_sets"][0]["highlighted"], true);
    assert_eq!(resized["layers_bottom_to_top"][0]["name"], "Imported pigment");
    assert_eq!(fs::read(original_collision).unwrap(), block_bytes);
    assert!(fs::read_dir(&models).unwrap().all(|entry| !entry.unwrap().file_name().to_string_lossy().starts_with(".paint-import-")));

    fs::write(models.join("fill-region.obj"), concat!(
        "v -1 -1 0\nv 1 -1 0\nv 1 1 0\nv -1 1 0\n",
        "v -1 -1 -1\nv 1 -1 -1\nv 1 1 -1\nv -1 1 -1\n",
        "vt 0.05 0.05\nvt 0.45 0.05\nvt 0.45 0.95\nvt 0.05 0.95\n",
        "vt 0.55 0.05\nvt 0.95 0.05\nvt 0.95 0.95\nvt 0.55 0.95\n",
        "f 1/1 2/2 3/3 4/4\nf 5/5 6/6 7/7 8/8\n",
    )).unwrap();
    client.tool(&mut app, "replace_paint_mesh", json!({"workspace": {"model_id": model, "set": 0},
        "replacement": {"kind": "file", "path": "models/fill-region.obj"}
    }));
    assert_eq!(wait_for_paint(&mut client, &mut app)["mesh"]["triangles"], 4);
    client.tool(&mut app, "edit_paint", json!({"model_id": model,"set":0,
        "edit": {"operation":"add_layer","kind":"Paint"}
    }));
    let started = wait_for_paint(&mut client, &mut app);
    let layer = started["layers_bottom_to_top"].as_array().unwrap().iter()
        .find(|layer| layer["selected"] == true).unwrap()["index"].as_u64().unwrap() as usize;
    let invalid_fill = client.request(&mut app, "tools/call", json!({
        "name": "poly_fill", "arguments": {"workspace": {"model_id": model, "set": 0},
            "target": {"layer":layer,"space":"uv"}, "position":[0.25,0.5],
            "region": {"kind":"rectangle","start":[0,0],"end":[0.5,1]}}
    }));
    assert_eq!(invalid_fill["isError"], true, "{invalid_fill}");
    let fill = |client: &mut StdioClient, app: &mut App, space, region, color| {
        client.tool(app, "poly_fill", json!({"workspace":{"model_id":model,"set":0},
            "target":{"layer":layer,"space":space}, "region":region,
            "mode":"polygon", "padding_px":0, "brush":{"channels":{"base_color":color},"opacity_percent":100}
        }));
        wait_for_paint(client, app)
    };
    let read = |app: &App, x: f32| {
        let editing = app.world().resource::<editing::State>();
        let surface = editing.documents[&model].objects[0].materials[0].paint.as_ref().unwrap();
        let texels = surface.stack.layers[layer].channel(0).unwrap();
        let side = surface.size as usize;
        let offset = (side / 2 * side + (x * side as f32) as usize) * 4;
        <[u8; 4]>::try_from(&texels[offset..offset + 4]).unwrap()
    };
    let left = fill(&mut client, &mut app, "uv", json!({"kind":"rectangle","start":[0,0],"end":[0.49,1]}), [180, 15, 20]);
    assert_eq!(left["undo_steps"].as_u64().unwrap(), started["undo_steps"].as_u64().unwrap() + 1);
    assert_eq!(read(&app, 0.25), [180, 15, 20, 255]);
    assert_eq!(read(&app, 0.75)[3], 0);
    fill(&mut client, &mut app, "uv", json!({"kind":"lasso","points":[[0.5,0],[1,0],[1,1],[0.5,1]]}), [12, 180, 25]);
    assert_eq!(read(&app, 0.25), [180, 15, 20, 255]);
    assert_eq!(read(&app, 0.75), [12, 180, 25, 255]);
    client.tool(&mut app, "set_paint_view", json!({"model_id": model, "layout":"three",
        "center":[0,0,-0.5], "distance":4, "yaw_degrees":0, "pitch_degrees":0, "orthographic":true
    }));
    fill(&mut client, &mut app, "viewport", json!({"kind":"rectangle","start":[0,0],"end":[1,1],"through":false}), [20, 25, 180]);
    assert_eq!(read(&app, 0.25), [20, 25, 180, 255]);
    assert_eq!(read(&app, 0.75), [12, 180, 25, 255]);
    fill(&mut client, &mut app, "viewport", json!({"kind":"lasso","points":[[0,0],[1,0],[1,1],[0,1]],"through":true}), [40, 70, 160]);
    assert_eq!(read(&app, 0.25), [40, 70, 160, 255]);
    assert_eq!(read(&app, 0.75), [40, 70, 160, 255]);

    fs::write(models.join("cage-source.obj"), concat!(
        "v -2 -2 0.1\nv 2 -2 0.1\nv 2 2 0.1\nv -2 2 0.1\n",
        "f 1 2 3 4\n",
    )).unwrap();
    let cage = |client: &mut StdioClient, app: &mut App, args: Value| {
        client.tool(app, "cage_preview", args)["structuredContent"].clone()
    };
    let wait_for_cage = |client: &mut StdioClient, app: &mut App| {
        let started = Instant::now();
        loop {
            let result = cage(client, app, json!({"model_id": model}));
            if result["report"]["status"] == "ready" {
                return result;
            }
            assert_eq!(result["report"]["status"], "pending", "{result}");
            assert!(started.elapsed() < REQUEST_TIMEOUT, "Cage preview did not finish");
        }
    };
    let initial = cage(&mut client, &mut app, json!({"model_id": model}));
    assert_eq!(initial["visible"], false);
    let shell = cage(&mut client, &mut app, json!({"model_id": model, "show": true}));
    assert_eq!(shell["visible"], true);
    let shell = wait_for_cage(&mut client, &mut app);
    assert_eq!(shell["report"]["mode"], "shell");
    assert!(shell["report"].get("missed").is_none());
    let opened = cage(&mut client, &mut app, json!({
        "model_id": model,
        "projection": {"high_poly": "models/cage-source.obj", "front_percent": 20,
            "match_names": false, "prefer_front_faces": false}
    }));
    assert_eq!(opened["visible"], true);
    let wide = wait_for_cage(&mut client, &mut app);
    assert_eq!(wide["settings"]["front"], 20);
    assert!(wide["report"]["covered"].as_u64().unwrap() > 0, "{wide}");
    let invalid = client.request(&mut app, "tools/call", json!({
        "name": "cage_preview", "arguments": {"model_id": model, "show": false,
            "projection": {"front_percent": 0, "match_names": true}}
    }));
    assert_eq!(invalid["isError"], true, "{invalid}");
    assert_eq!(cage(&mut client, &mut app, json!({"model_id": model})), wide);

    cage(&mut client, &mut app, json!({"model_id": model, "projection": {"front_percent": 1}}));
    let narrow = wait_for_cage(&mut client, &mut app);
    assert_eq!(narrow["settings"]["front"], 1);
    assert_eq!(narrow["settings"]["high_poly"], wide["settings"]["high_poly"]);
    assert!(
        narrow["report"]["missed"].as_u64().unwrap() > wide["report"]["missed"].as_u64().unwrap(),
        "{wide}\n{narrow}"
    );
    let bake = client.tool(&mut app, "get_bake_state", json!({"model_id": model}));
    assert_eq!(bake["structuredContent"]["engine_busy"], false);
    let hidden = cage(&mut client, &mut app, json!({
        "model_id": model, "show": false, "projection": {"high_poly": ""}
    }));
    assert_eq!(hidden["report"]["status"], "hidden");
    assert_eq!(hidden["settings"]["high_poly"], Value::Null);

    let scene_light = app.world().resource::<Editor>().environment().clone();
    let configured = client.tool(&mut app, "set_paint_view", json!({
        "model_id": model, "environment": {
            "preset": "photo_studio", "intensity": 1.25, "rotation_degrees": 32.5,
            "show_background": false, "background_blur": 0.5, "background_brightness": 0.75,
            "key_light": true, "key_intensity": 3.0, "key_azimuth_degrees": 15.0,
            "key_elevation_degrees": 60.0
        }
    }));
    let configured = &configured["structuredContent"];
    let profile = project.asset_root.join(".bevy-editor/model-preview.json");
    let profile_bytes = fs::read(profile).unwrap();
    let saved: Value = serde_json::from_slice(&profile_bytes).unwrap();
    assert_eq!(saved["lighting"]["hdri"], "photo_studio");
    assert_eq!(saved["lighting"]["hdri_brightness"], 1250.0);
    assert_eq!(saved["lighting"]["hdri_rotation"], 32.5);
    assert_eq!(saved["lighting"]["hdri_background"], false);
    assert_eq!(saved["blur"], 0.5);
    assert_eq!(configured["environment"]["key_elevation_degrees"], 60.0);
    frame(&mut app);
    let world = app.world_mut();
    let mut lighting = world.query::<&bevy::light::DirectionalLight>();
    assert!(lighting.iter(world).any(|light| light.illuminance == 3000.0));
    assert_eq!(world.resource::<Editor>().environment(), &scene_light);

    let invalid = client.request(&mut app, "tools/call", json!({
        "name": "set_paint_view", "arguments": {"model_id": model, "distance": 8.0,
            "environment": {"preset": "cloudy", "key_elevation_degrees": 91.0}}
    }));
    assert_eq!(invalid["isError"], true, "{invalid}");
    let unchanged = client.tool(&mut app, "set_paint_view", json!({"model_id": model}));
    assert_eq!(unchanged["structuredContent"]["environment"], configured["environment"]);
    assert_eq!(unchanged["structuredContent"]["camera"], configured["camera"]);
    assert_eq!(fs::read(project.asset_root.join(".bevy-editor/model-preview.json")).unwrap(), profile_bytes);

    let trace = client.tool(&mut app, "set_paint_view", json!({
        "model_id": model, "path_trace": true, "trace_settings": {"samples": 16, "bounces": 2, "ground": true}
    }));
    assert_eq!(trace["structuredContent"]["path_trace"], true);
    assert_eq!(trace["structuredContent"]["trace_settings"], json!({"samples": 16, "bounces": 2, "ground": true}));
    let trace_progress = |client: &mut StdioClient, app: &mut App| {
        client.tool(app, "get_paint_state", json!({}))["structuredContent"]["view"]["trace_progress"].clone()
    };
    let wait_for_trace = |client: &mut StdioClient, app: &mut App, limit| {
        let started = Instant::now();
        loop {
            let progress = trace_progress(client, app);
            assert_eq!(progress["target_samples"], limit);
            if progress["status"] == "complete" {
                assert_eq!(progress["samples"], limit);
                break;
            }
            assert!(["preparing", "rendering"].contains(&progress["status"].as_str().unwrap()), "{progress}");
            assert!(progress["samples"].as_u64().is_none_or(|count| count < limit), "{progress}");
            assert!(started.elapsed() < REQUEST_TIMEOUT, "Trace stayed at {progress}");
        }
    };
    wait_for_trace(&mut client, &mut app, 16);
    let invalid = client.request(&mut app, "tools/call", json!({
        "name": "set_paint_view", "arguments": {"model_id": model, "path_trace": false,
            "trace_settings": {"samples": 4097, "ground": false}}
    }));
    assert_eq!(invalid["isError"], true, "{invalid}");
    let unchanged = client.tool(&mut app, "set_paint_view", json!({"model_id": model}));
    assert_eq!(unchanged["structuredContent"]["path_trace"], true);
    assert_eq!(unchanged["structuredContent"]["trace_settings"], trace["structuredContent"]["trace_settings"]);
    let raised = client.tool(&mut app, "set_paint_view", json!({
        "model_id": model, "trace_settings": {"samples": 24}
    }));
    assert_eq!(raised["structuredContent"]["trace_settings"], json!({"samples": 24, "bounces": 2, "ground": true}));
    wait_for_trace(&mut client, &mut app, 24);
    client.tool(&mut app, "set_paint_view", json!({"model_id": model, "layout": "two"}));
    let paused = trace_progress(&mut client, &mut app);
    assert_eq!(paused["status"], "paused");
    assert_eq!(paused["reason"], "uv_only");
    assert_eq!(paused["samples"], Value::Null);
    client.tool(&mut app, "set_paint_view", json!({"model_id": model, "path_trace": false}));
    assert_eq!(trace_progress(&mut client, &mut app)["status"], "off");

    mcp_state::seed_local_presets(&mut app.world_mut().resource_mut::<PaintState>(), &scratch.0.join("local-presets"));
    let presets = client.tool(&mut app, "list_paint_presets", json!({}));
    let local = presets["structuredContent"]["local_presets"].as_array().unwrap();
    let preset_index = |kind| local.iter().find(|entry| entry["kind"] == kind).unwrap()["index"].clone();
    let material = preset_index("material");
    let mask = preset_index("mask");
    let apply_local = |client: &mut StdioClient, app: &mut App, edit| {
        client.tool(app, "edit_paint", json!({"model_id": model, "set": 0, "edit": edit}));
        wait_for_paint(client, app)
    };
    let before = wait_for_paint(&mut client, &mut app);
    let invalid = client.request(&mut app, "tools/call", json!({
        "name": "edit_paint", "arguments": {"model_id": model, "set": 0,
            "edit": {"operation": "add_local_material", "preset": mask}}
    }));
    assert_eq!(invalid["isError"], true, "{invalid}");
    assert_eq!(wait_for_paint(&mut client, &mut app)["undo_steps"], before["undo_steps"]);
    let added = apply_local(&mut client, &mut app, json!({
        "operation": "add_local_material", "preset": material, "name": "Imported from shelf",
        "target": {"layer": layer, "placement": "above"}
    }));
    let new_row = &added["layers_bottom_to_top"][layer + 1];
    assert_eq!(new_row["name"], "Imported from shelf");
    assert_eq!(new_row["mask"]["has_graph"], true);
    assert_eq!(added["undo_steps"].as_u64().unwrap(), before["undo_steps"].as_u64().unwrap() + 1);
    let masked = apply_local(&mut client, &mut app, json!({
        "operation": "apply_local_mask", "preset": mask, "layer": layer
    }));
    assert_eq!(masked["layers_bottom_to_top"][layer]["mask"]["has_graph"], false);
    assert_eq!(masked["undo_steps"].as_u64().unwrap(), added["undo_steps"].as_u64().unwrap() + 1);
    let undone = apply_local(&mut client, &mut app, json!({"operation":"undo"}));
    assert_eq!(undone["layers_bottom_to_top"][layer]["mask"], Value::Null);
    apply_local(&mut client, &mut app, json!({"operation":"redo"}));
    let saved = client.tool(&mut app, "save_model", json!({
        "model_id": model, "copy_name": "local-presets.glb"
    }));
    let saved = Glb::load(Path::new(saved["structuredContent"]["path"].as_str().unwrap())).unwrap();
    let surface = saved.json["meshes"].as_array().unwrap().iter()
        .flat_map(|mesh| mesh["primitives"].as_array().into_iter().flatten())
        .filter_map(|primitive| saved.paint_surface(primitive).unwrap())
        .find(|surface| surface.stack.layers.get(layer + 1)
            .is_some_and(|layer| layer.name == "Imported from shelf"))
        .expect("The saved model must contain the texture set with the applied preset");
    assert_eq!(surface.size, 512);
    assert!(surface.stack.layers[layer + 1].texture_fill.as_ref().unwrap().graph.is_some());
    assert_eq!(surface.stack.layers[layer].mask.as_ref().unwrap().pixels[0], 96);

    let invalid = client.request(&mut app, "tools/call", json!({
        "name":"save_paint_preset", "arguments":{"workspace":{"model_id": model,"set":0},
            "kind":"material", "name":"Invalid paint capture", "layer":layer}
    }));
    assert_eq!(invalid["isError"], true, "{invalid}");
    let before_save = wait_for_paint(&mut client, &mut app);
    let new_material = client.tool(&mut app, "save_paint_preset", json!({
        "workspace":{"model_id":model,"set":0}, "kind":"material", "name":"Reusable graph fill", "layer":layer + 1
    }));
    let new_mask = client.tool(&mut app, "save_paint_preset", json!({
        "workspace":{"model_id":model,"set":0}, "kind":"mask", "name":"Reusable mask", "layer":layer
    }));
    let [new_material, new_mask] = [&new_material["structuredContent"], &new_mask["structuredContent"]];
    assert_eq!(new_material["kind"], "material");
    assert_eq!(new_mask["kind"], "mask");
    let library = client.tool(&mut app, "list_paint_presets", json!({}));
    let local = library["structuredContent"]["local_presets"].as_array().unwrap();
    for preset in [new_material, new_mask] {
        assert!(local.iter().any(|item| item["index"] == preset["index"] && item["name"] == preset["name"]));
    }
    assert_eq!(local.len(), 4);
    let unchanged = wait_for_paint(&mut client, &mut app);
    assert_eq!(unchanged["undo_steps"], before_save["undo_steps"]);
    assert_eq!(unchanged["layers_bottom_to_top"], before_save["layers_bottom_to_top"]);
    assert_eq!(fs::read_dir(scratch.0.join("local-presets/shelf")).unwrap().count(), 4);

    let invalid_import = client.request(&mut app, "tools/call", json!({
        "name":"import_paint_preset", "arguments":{"source":"missing.mpsmart"}
    }));
    assert_eq!(invalid_import["isError"], true, "{invalid_import}");
    let transferred = [("mpsmart", "Shared graph fill"), ("zip", "Shared graph fill 2")].map(|(format, expected_name)| {
        let output = client.tool(&mut app, "export_paint_preset", json!({
            "workspace":{"model_id":model,"set":0}, "layer":layer + 1,
            "folder":"material-exchange", "name":"Shared graph fill", "format":format
        }));
        let output = &output["structuredContent"];
        let path = Path::new(output["path"].as_str().unwrap());
        assert_eq!(output["format"], format);
        assert_eq!(path.extension().unwrap(), format);
        assert!(path.is_file() && path.starts_with(project.asset_root.join("material-exchange")));
        let reimport = client.tool(&mut app, "import_paint_preset", json!({"source":path}));
        let reimport = &reimport["structuredContent"];
        assert_eq!(reimport["name"], expected_name);
        assert_eq!(reimport["kind"], "material");
        reimport["index"].clone()
    });
    assert_ne!(transferred[0], transferred[1]);
    assert_eq!(fs::read_dir(scratch.0.join("local-presets/shelf")).unwrap().count(), 6);
    assert_eq!(wait_for_paint(&mut client, &mut app)["layers_bottom_to_top"], before_save["layers_bottom_to_top"]);

    open(&mut client, &mut app, "models/block.glb", 0);
    let destination = wait_for_paint(&mut client, &mut app);
    let set = destination["active_set"].as_u64().unwrap();
    for (preset, name) in [
        (&new_material["index"], "Reused on block"),
        (&transferred[0], "Imported MPSMART"),
        (&transferred[1], "Imported ZIP"),
    ] {
        client.tool(&mut app, "edit_paint", json!({"model_id":0,"set":set,
            "edit":{"operation":"add_local_material","preset":preset,"name":name}
        }));
        let ready = wait_for_paint(&mut client, &mut app);
        let added = ready["layers_bottom_to_top"].as_array().unwrap().iter()
            .find(|row| row["name"] == name).unwrap();
        assert_eq!(added["mask"]["has_graph"], true);
    }
    let layer = wait_for_paint(&mut client, &mut app)["layers_bottom_to_top"].as_array().unwrap()
        .iter().position(|row| row["name"] == "Imported ZIP").unwrap();
    client.tool(&mut app, "edit_paint", json!({"model_id":0,"set":set,
        "edit":{"operation":"apply_local_mask","preset":new_mask["index"],"layer":layer}
    }));
    assert_eq!(wait_for_paint(&mut client, &mut app)["layers_bottom_to_top"][layer]["mask"]["has_graph"], false);
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
#[ignore = "requires GPU; captures the editor viewport and full interface through MCP at 200% DPI"]
fn gpu_mcp_capture_renders_the_viewport_at_high_dpi() {
    use crate::editor::{
        selection_outline::render_tests::frame, tests::lighting_ui_render::fixture,
    };

    let (mut app, window, app_target) = fixture();
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
    app.world_mut().spawn((
        Node { position_type: PositionType::Absolute, left: px(0), top: px(0),
            width: px(24), height: px(24), ..default() },
        BackgroundColor(Color::srgb(1., 0., 1.)),
        GlobalZIndex(2000),
    ));
    for _ in 0..3 {
        frame(&mut app);
    }
    let mut first = enqueue(&bridge, Command::ScreenshotApp, Instant::now() + REQUEST_TIMEOUT);
    let mut second = enqueue(&bridge, Command::ScreenshotApp, Instant::now() + REQUEST_TIMEOUT);
    let read = |app: &mut App, pending: &mut oneshot::Receiver<Reply>| {
        let payload = (0..240)
            .find_map(|_| {
                frame(app);
                pending.try_recv().ok()
            })
            .expect("App screenshot did not complete")
            .expect("App screenshot failed");
        let Payload::Image { png, width, height } = payload else {
            panic!("App screenshot must return an image");
        };
        let target = app.world().resource::<Assets<Image>>().get(&app_target).unwrap();
        assert_eq!(UVec2::new(width, height), target.size());
        png
    };
    let png = read(&mut app, &mut first);
    assert_eq!(png, read(&mut app, &mut second));
    let bytes = base64::engine::general_purpose::STANDARD.decode(png).unwrap();
    let image = image::load_from_memory(&bytes).unwrap().into_rgb8();
    let [r, g, b] = image.get_pixel(8, 8).0;
    assert!(r > 230 && g < 20 && b > 230, "PNG must include the UI overlay");
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
