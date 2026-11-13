use std::{
    future::Future,
    io,
    net::{Ipv4Addr, TcpListener},
    path::{Path, PathBuf},
    sync::Arc,
    thread::JoinHandle,
    time::Duration,
};

use axum::{
    body::Body,
    http::{Request, StatusCode},
    middleware::{self, Next},
    response::IntoResponse,
};
use bevy::prelude::Resource as BevyResource;
use rmcp::{
    ErrorData, Peer, RoleClient, RoleServer, ServerHandler, ServiceExt,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::*,
    service::{RequestContext, ServiceError},
    tool, tool_handler, tool_router,
    transport::{
        StreamableHttpClientTransport, stdio,
        streamable_http_client::StreamableHttpClientTransportConfig,
        streamable_http_server::{
            StreamableHttpService, session::local::LocalSessionManager,
            tower::StreamableHttpServerConfig,
        },
    },
};
use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;

use super::{Bridge, Command, Payload, types::*};
use crate::project::EditorProject;

const INSTRUCTIONS: &str = "Control the open Bevy Editor project. Read get_editor_state and list_objects first. Object IDs are qualified by scene; obtain new IDs after changing scenes. Mutations appear immediately and use the editor's undo/redo history. Scene edits remain unsaved until save_scene. Finish any active user edit before issuing mutations. capture_viewport returns the rendered 3D view as a PNG image.";

fn info() -> ServerConfig {
    ServerConfig::new(
        ServerCapabilities::builder()
            .enable_tools()
            .enable_resources()
            .build(),
    )
    .with_server_info(Implementation::new(
        "bevy-editor",
        env!("CARGO_PKG_VERSION"),
    ))
    .with_instructions(INSTRUCTIONS)
}

#[derive(Clone)]
pub(super) struct EditorServer {
    bridge: Bridge,
    tool_router: ToolRouter<Self>,
}

#[tool_router]
impl EditorServer {
    pub(super) fn new(bridge: Bridge) -> Self {
        Self {
            bridge,
            tool_router: Self::tool_router(),
        }
    }

    async fn call(
        &self,
        command: Command,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let reply = tokio::select! {
            _ = context.ct.cancelled() => return Ok(CallToolResult::error(vec![ContentBlock::text("Request cancelled")])),
            reply = self.bridge.request(command) => reply,
        };
        Ok(match reply {
            Ok(Payload::Json(value)) => CallToolResult::structured(value),
            Ok(Payload::Image { png, width, height }) => CallToolResult::success(vec![
                ContentBlock::text(format!("Live editor viewport, {width}×{height}")),
                ContentBlock::image(png, "image/png"),
            ]),
            Err(message) => CallToolResult::error(vec![ContentBlock::text(message)]),
        })
    }

    #[tool(
        description = "Read the live editor status, active scene, selection, dirty flag, view and undo/redo availability",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn get_editor_state(
        &self,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        self.call(Command::State, context).await
    }

    #[tool(
        description = "List the project's configured scenes and their open, active and unsaved states",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn list_scenes(
        &self,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        self.call(Command::Scenes, context).await
    }

    #[tool(
        description = "List active scene objects with scene-qualified IDs, world transforms and editable axes",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn list_objects(
        &self,
        Parameters(args): Parameters<Page>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        self.call(Command::Objects(args), context).await
    }

    #[tool(
        description = "List project GLB models available for add_model",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn list_assets(
        &self,
        Parameters(args): Parameters<Page>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        self.call(Command::Assets(args), context).await
    }

    #[tool(
        description = "Open a configured scene in a tab, retaining unsaved drafts in other tabs. Poll get_editor_state until ready",
        annotations(
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn open_scene(
        &self,
        Parameters(args): Parameters<SceneArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        self.call(Command::Open(args), context).await
    }

    #[tool(
        description = "Select an object in the editor hierarchy and inspector",
        annotations(
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn select_object(
        &self,
        Parameters(args): Parameters<ObjectArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        self.call(Command::Select(args), context).await
    }

    #[tool(
        description = "Atomically set absolute world position, rotation in degrees and/or scale. Locked axes are respected; creates one undo step",
        annotations(
            destructive_hint = true,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn set_transform(
        &self,
        Parameters(args): Parameters<TransformArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        self.call(Command::Transform(args), context).await
    }

    #[tool(
        description = "Add and select one instance of a project GLB model. Use an asset from list_assets. Can be undone",
        annotations(destructive_hint = false, open_world_hint = false)
    )]
    async fn add_model(
        &self,
        Parameters(args): Parameters<AddModelArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        self.call(Command::AddModel(args), context).await
    }

    #[tool(
        description = "Add and select a light using an existing editor preset. Can be undone",
        annotations(destructive_hint = false, open_world_hint = false)
    )]
    async fn add_light(
        &self,
        Parameters(args): Parameters<AddLightArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        self.call(Command::AddLight(args), context).await
    }

    #[tool(
        description = "Atomically change a light's name, enabled state, color, intensity, range, shadows or temperature. Can be undone",
        annotations(
            destructive_hint = true,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn set_light(
        &self,
        Parameters(args): Parameters<LightArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        self.call(Command::Light(args), context).await
    }

    #[tool(
        description = "Remove an added model or light from the scene. Can be undone. Imported source objects cannot be deleted",
        annotations(destructive_hint = true, open_world_hint = false)
    )]
    async fn remove_object(
        &self,
        Parameters(args): Parameters<ObjectArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        self.call(Command::Remove(args), context).await
    }

    #[tool(
        description = "Show or hide an object using the editor's persistent local visibility preference. Does not change scene data",
        annotations(
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn set_visibility(
        &self,
        Parameters(args): Parameters<VisibilityArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        self.call(Command::Visibility(args), context).await
    }

    #[tool(
        description = "Set the free editor view position and look-at target without modifying scene cameras",
        annotations(
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn set_view(
        &self,
        Parameters(args): Parameters<ViewArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        self.call(Command::View(args), context).await
    }

    #[tool(
        description = "Save the active scene through the editor's normal validation and atomic file write. Wait for model geometry to finish loading first",
        annotations(
            destructive_hint = true,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn save_scene(
        &self,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        self.call(Command::Save, context).await
    }

    #[tool(
        description = "Undo one edit in the active scene, including changes made in the UI or through MCP",
        annotations(destructive_hint = true, open_world_hint = false)
    )]
    async fn undo(&self, context: RequestContext<RoleServer>) -> Result<CallToolResult, ErrorData> {
        self.call(Command::Undo, context).await
    }

    #[tool(
        description = "Redo one undone edit in the active scene",
        annotations(destructive_hint = true, open_world_hint = false)
    )]
    async fn redo(&self, context: RequestContext<RoleServer>) -> Result<CallToolResult, ErrorData> {
        self.call(Command::Redo, context).await
    }

    #[tool(
        description = "Capture the live rendered 3D viewport as a PNG image (up to 1600×1000)",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn capture_viewport(
        &self,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        self.call(Command::Capture, context).await
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for EditorServer {
    fn get_info(&self) -> ServerConfig {
        info()
    }

    async fn list_resources(
        &self,
        _: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, ErrorData> {
        Ok(ListResourcesResult {
            resources: vec![
                Resource::new("bevy-editor://state", "Live editor state"),
                Resource::new("bevy-editor://scenes", "Project scenes"),
                Resource::new(
                    "bevy-editor://objects",
                    "Active scene objects (first 100; use list_objects for pagination)",
                ),
            ],
            ..Default::default()
        })
    }

    async fn read_resource(
        &self,
        args: ReadResourceRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, ErrorData> {
        let command = match args.uri.as_str() {
            "bevy-editor://state" => Command::State,
            "bevy-editor://scenes" => Command::Scenes,
            "bevy-editor://objects" => Command::Objects(Page::default()),
            _ => {
                return Err(ErrorData::resource_not_found(
                    "Unknown editor resource",
                    None,
                ));
            }
        };
        let reply = tokio::select! {
            _ = context.ct.cancelled() => return Err(ErrorData::internal_error("Request cancelled", None)),
            reply = self.bridge.request(command) => reply,
        };
        match reply {
            Ok(Payload::Json(value)) => Ok(ReadResourceResult::new(vec![ResourceContents::text(
                value.to_string(),
                args.uri,
            )])
            .into()),
            Ok(_) => Err(ErrorData::internal_error("Expected a JSON resource", None)),
            Err(message) => Err(ErrorData::internal_error(message, None)),
        }
    }
}

#[derive(Serialize, Deserialize)]
struct Endpoint {
    url: String,
    token: String,
    process_id: u32,
}

fn descriptor(project: &EditorProject) -> PathBuf {
    project.asset_root.join(".bevy-editor/mcp.json")
}

#[derive(BevyResource)]
pub(super) struct RunningServer {
    shutdown: Option<oneshot::Sender<()>>,
    worker: Option<JoinHandle<()>>,
    file: PathBuf,
    token: String,
}

impl Drop for RunningServer {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        // An older editor must never remove a newer instance's discovery file.
        if std::fs::read(&self.file)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Endpoint>(&bytes).ok())
            .is_some_and(|endpoint| endpoint.token == self.token)
        {
            let _ = std::fs::remove_file(&self.file);
        }
    }
}

pub(super) fn start(project: &EditorProject, bridge: Bridge) -> io::Result<RunningServer> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
    listener.set_nonblocking(true)?;
    let port = listener.local_addr()?.port();
    let token = uuid::Uuid::new_v4().to_string();
    let endpoint = Endpoint {
        url: format!("http://127.0.0.1:{port}/mcp"),
        token: token.clone(),
        process_id: std::process::id(),
    };
    let file = descriptor(project);
    let parent = file
        .parent()
        .ok_or_else(|| io::Error::other("Invalid MCP discovery path"))?;
    std::fs::create_dir_all(parent)?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let (shutdown, receiver) = oneshot::channel();
    let auth = format!("Bearer {token}");
    let worker = std::thread::Builder::new().name("editor-mcp".into()).spawn(move || {
        runtime.block_on(async move {
            let listener = match tokio::net::TcpListener::from_std(listener) { Ok(listener) => listener, Err(error) => { eprintln!("Editor MCP: {error}"); return; } };
            let mut config = StreamableHttpServerConfig::default();
            config.allowed_hosts = vec![format!("127.0.0.1:{port}"), format!("localhost:{port}")];
            config.allowed_origins = vec![format!("http://127.0.0.1:{port}"), format!("http://localhost:{port}")];
            config.json_response = true;
            let service = StreamableHttpService::new(move || Ok(EditorServer::new(bridge.clone())), Arc::new(LocalSessionManager::default()), config);
            let router = axum::Router::new().nest_service("/mcp", service).layer(middleware::from_fn(
                move |request: Request<Body>, next: Next| { let auth = auth.clone(); async move {
                    let authorized = request.headers().get(axum::http::header::AUTHORIZATION)
                        .and_then(|header| header.to_str().ok()).is_some_and(|header| header == auth);
                    if !authorized { return StatusCode::UNAUTHORIZED.into_response(); }
                    next.run(request).await
                } },
            ));
            tokio::select! {
                result = axum::serve(listener, router).into_future() => { if let Err(error) = result { eprintln!("Editor MCP: {error}"); } },
                _ = receiver => {}
            }
        });
    })?;
    let mut server = RunningServer {
        shutdown: Some(shutdown),
        worker: Some(worker),
        file: file.clone(),
        token,
    };
    let temporary = file.with_extension(format!("{}.tmp", server.token));
    if let Err(error) = (|| -> io::Result<()> {
        std::fs::write(&temporary, serde_json::to_vec_pretty(&endpoint)?)?;
        std::fs::rename(&temporary, &file)
    })() {
        let _ = std::fs::remove_file(&temporary);
        server.file = PathBuf::new();
        return Err(error);
    }
    bevy::log::info!("Editor MCP listening at {}", endpoint.url);
    Ok(server)
}

#[derive(Clone)]
struct StdioProxy {
    peer: Peer<RoleClient>,
}

fn proxy_error(error: ServiceError) -> ErrorData {
    match error {
        ServiceError::McpError(error) => error,
        error => ErrorData::internal_error(error.to_string(), None),
    }
}

async fn forward<T>(
    context: RequestContext<RoleServer>,
    request: impl Future<Output = Result<T, ServiceError>>,
) -> Result<T, ErrorData> {
    tokio::select! { _ = context.ct.cancelled() => Err(ErrorData::internal_error("Request cancelled", None)), result = request => result.map_err(proxy_error) }
}

impl ServerHandler for StdioProxy {
    fn get_info(&self) -> ServerConfig {
        info()
    }
    async fn list_tools(
        &self,
        args: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        forward(context, self.peer.list_tools(args)).await
    }
    async fn call_tool(
        &self,
        args: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        forward(context, self.peer.call_tool(args))
            .await
            .map(Into::into)
    }
    async fn list_resources(
        &self,
        args: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, ErrorData> {
        forward(context, self.peer.list_resources(args)).await
    }
    async fn read_resource(
        &self,
        args: ReadResourceRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, ErrorData> {
        forward(context, self.peer.read_resource(args))
            .await
            .map(Into::into)
    }
}

/// Run the MCP stdio bridge for an already open project. Stdout is protocol-only.
pub async fn run_stdio(project_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let project = EditorProject::load(project_path)?;
    let endpoint: Endpoint =
        serde_json::from_slice(&std::fs::read(descriptor(&project)).map_err(|_| {
            io::Error::other("Open this project in Bevy Editor with MCP enabled first")
        })?)?;
    let port = endpoint
        .url
        .strip_prefix("http://127.0.0.1:")
        .and_then(|value| value.strip_suffix("/mcp"))
        .and_then(|port| port.parse::<u16>().ok())
        .filter(|&port| port != 0)
        .ok_or_else(|| io::Error::other("Invalid local editor MCP endpoint"))?;
    let config =
        StreamableHttpClientTransportConfig::with_uri(format!("http://127.0.0.1:{port}/mcp"))
            .auth_header(endpoint.token);
    let transport = StreamableHttpClientTransport::from_config(config);
    let editor = tokio::time::timeout(Duration::from_secs(10), ().serve(transport))
        .await
        .map_err(|_| {
            io::Error::other("The editor MCP connection timed out; reopen the project")
        })??;
    let service = StdioProxy {
        peer: editor.peer().clone(),
    }
    .serve(stdio())
    .await?;
    service.waiting().await?;
    editor.cancel().await?;
    Ok(())
}
