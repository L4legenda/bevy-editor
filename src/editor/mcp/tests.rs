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
    assert_eq!(tools.tools.len(), 17);
    for name in [
        "get_editor_state",
        "set_transform",
        "add_model",
        "save_scene",
        "undo",
        "capture_viewport",
    ] {
        assert!(tools.tools.iter().any(|tool| tool.name == name));
    }
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
