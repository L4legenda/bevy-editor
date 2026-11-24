//! Low-resolution graph evaluation and GPU copies for the open node canvas.
use super::{Graph, Runtime, bake, file, library, mesh_maps, surface};
use crate::editor::model_paint_data::texture_fill::GraphImages;
use bevy::{
    asset::{AssetId, RenderAssetUsages},
    prelude::*,
    render::{
        ExtractSchedule, MainWorld, Render, RenderApp, RenderSystems,
        render_asset::RenderAssets,
        render_resource::{CommandEncoderDescriptor, Extent3d, TextureDimension},
        renderer::{RenderDevice, RenderQueue},
        texture::GpuImage,
    },
    tasks::{AsyncComputeTaskPool, Task, block_on, poll_once},
};
use material_painter_graph::Tex;
use std::{
    collections::HashMap,
    hash::{DefaultHasher, Hash, Hasher},
    path::PathBuf,
    sync::{Arc, Mutex},
};

const PREVIEW_SIZE: u32 = 128;

#[derive(Resource, Default)]
pub(in crate::editor::ui::model_viewer) struct Uploads(HashMap<AssetId<Image>, Arc<Tex>>);

#[derive(Resource, Default)]
struct Pending(HashMap<AssetId<Image>, Arc<Tex>>);

pub(in crate::editor::ui::model_viewer) struct PreviewPlugin;

impl Plugin for PreviewPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Uploads>();
        if let Some(render) = app.get_sub_app_mut(RenderApp) {
            render
                .init_resource::<Pending>()
                .add_systems(ExtractSchedule, extract)
                .add_systems(
                    Render,
                    upload
                        .after(RenderSystems::PrepareAssets)
                        .before(RenderSystems::Queue),
                );
        }
    }
}

fn extract(mut world: ResMut<MainWorld>, mut pending: ResMut<Pending>) {
    pending
        .0
        .extend(std::mem::take(&mut world.resource_mut::<Uploads>().0));
    let images = world.resource::<Assets<Image>>();
    pending.0.retain(|id, _| images.contains(*id));
}

fn upload(
    mut pending: ResMut<Pending>,
    images: Res<RenderAssets<GpuImage>>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
) {
    let mut encoder = None;
    pending.0.retain(|id, source| {
        let Some(image) = images.get(*id) else {
            return true;
        };
        encoder
            .get_or_insert_with(|| {
                device.create_command_encoder(&CommandEncoderDescriptor {
                    label: Some("Material graph node preview copies"),
                })
            })
            .copy_texture_to_texture(
                source.copy_info(),
                image.texture.as_image_copy(),
                source.extent(),
            );
        false
    });
    if let Some(encoder) = encoder {
        queue.submit([encoder.finish()]);
    }
}

struct Request {
    graph: Graph,
    images: Arc<GraphImages>,
    paths: Vec<PathBuf>,
    maps: Option<Arc<bake::Maps>>,
    surface: Option<Arc<surface::Source>>,
}

impl Request {
    fn same_inputs(&self, other: &Self) -> bool {
        mesh_maps::same(self.maps.as_ref(), other.maps.as_ref())
            && mesh_maps::same(self.surface.as_ref(), other.surface.as_ref())
    }

    fn inputs_key(&self) -> u64 {
        let mut key = DefaultHasher::new();
        self.maps.as_ref().map(Arc::as_ptr).hash(&mut key);
        self.surface.as_ref().map(Arc::as_ptr).hash(&mut key);
        key.finish()
    }
}

impl PartialEq for Request {
    fn eq(&self, other: &Self) -> bool {
        self.graph == other.graph && self.images == other.images && self.same_inputs(other)
    }
}

struct Reply {
    generation: u64,
    request: Arc<Request>,
    runtime: Option<Mutex<Runtime>>,
    surface: Option<surface::Textures>,
    thumbs: HashMap<u64, Arc<Tex>>,
}

struct Presented {
    source: Arc<Tex>,
    image: Handle<Image>,
}

#[derive(Default)]
pub(in crate::editor::ui::model_viewer::paint) struct Previews {
    generation: u64,
    desired: Option<Arc<Request>>,
    job: Option<Task<Reply>>,
    completed: Option<Reply>,
    images: HashMap<u64, Presented>,
}

impl Previews {
    pub fn set(
        &mut self,
        graph: &Graph,
        images: &Arc<GraphImages>,
        maps: Option<&Arc<bake::Maps>>,
        surface: Option<&Arc<surface::Source>>,
    ) {
        let mut graph = Graph::clone(graph);
        graph.name.clear();
        graph.next_id = 1;
        graph.res = PREVIEW_SIZE;
        graph.match_set = false;
        graph.tags.clear();
        graph.params.clear();
        for node in &mut graph.nodes {
            node.pos = [0.0; 2];
        }
        self.images.retain(|id, _| graph.node(*id).is_some());
        let mut paths: Vec<_> = graph
            .nodes
            .iter()
            .flat_map(|node| node.params.values())
            .filter_map(|value| match value {
                material_painter_graph::Value::Image(Some(path)) => Some(path.clone()),
                _ => None,
            })
            .collect();
        paths.sort_unstable();
        paths.dedup();
        let maps = maps
            .filter(|_| surface.is_some() || !mesh_maps::used(&graph).is_empty())
            .cloned();
        let request = Request {
            graph,
            images: images.clone(),
            paths,
            maps,
            surface: surface.cloned(),
        };
        if self.desired.as_deref() != Some(&request) {
            self.desired = Some(Arc::new(request));
        }
    }

    pub fn reset(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.desired = None;
        self.completed = None;
        self.images.clear();
    }

    pub fn image(&self, id: u64) -> Option<&Handle<Image>> {
        self.images.get(&id).map(|preview| &preview.image)
    }

    pub fn update(
        &mut self,
        assets: &mut Assets<Image>,
        uploads: &mut Uploads,
        renderer: Option<(&RenderDevice, &RenderQueue)>,
    ) -> bool {
        let mut changed = false;
        if let Some(reply) = self.job.as_mut().and_then(|task| block_on(poll_once(task))) {
            self.job = None;
            if reply.generation == self.generation {
                if self
                    .desired
                    .as_ref()
                    .is_some_and(|request| Arc::ptr_eq(request, &reply.request))
                {
                    self.present(&reply.thumbs, assets, uploads);
                    changed = true;
                }
                self.completed = Some(reply);
            }
        }
        if self.job.is_none()
            && let Some((device, queue)) = renderer
            && let Some(request) = self.desired.as_ref().filter(|request| {
                self.completed
                    .as_ref()
                    .is_none_or(|done| !Arc::ptr_eq(request, &done.request))
            })
        {
            let (device, queue) = (device.clone(), queue.clone());
            let generation = self.generation;
            let request = request.clone();
            let (runtime, surface) = self.completed.take().map_or((None, None), |done| {
                let same_inputs = done.request.same_inputs(&request);
                let runtime = if done.request.paths == request.paths
                    && done.request.images == request.images
                {
                    done.runtime.and_then(|runtime| runtime.into_inner().ok())
                } else {
                    None
                };
                (runtime, done.surface.filter(|_| same_inputs))
            });
            self.job = Some(AsyncComputeTaskPool::get().spawn(async move {
                let mut graph = request.graph.clone();
                let mut runtime = match runtime {
                    Some(runtime) => Some(runtime),
                    None => {
                        let mut runtime =
                            Runtime::new(device.wgpu_device(), &queue, super::assets());
                        file::bind_images(
                            &mut graph,
                            &request.images,
                            &mut runtime,
                            &device,
                            &queue,
                        )
                        .ok()
                        .map(|_| runtime)
                    }
                };
                let mut thumbs = HashMap::new();
                let surface = surface.or_else(|| {
                    request.surface.as_ref().map(|source| {
                        surface::Textures::new(
                            source,
                            request.maps.as_deref(),
                            graph.res,
                            &device,
                            &queue,
                        )
                    })
                });
                if let Some(runtime) = &mut runtime {
                    let mesh = mesh_maps::Textures::new(
                        &mesh_maps::used(&graph),
                        request.maps.as_deref(),
                        graph.res,
                        &device,
                        &queue,
                    );
                    runtime.evaluate(
                        device.wgpu_device(),
                        &queue,
                        &graph,
                        &library(),
                        &|id| mesh.get(id),
                        surface.as_ref().map(surface::Textures::inputs).as_ref(),
                        request.inputs_key(),
                        false,
                    );
                    for node in &graph.nodes {
                        if let Some(texture) = runtime.thumb(node.id) {
                            thumbs.insert(node.id, texture.clone());
                        }
                    }
                }
                Reply {
                    generation,
                    request,
                    runtime: runtime.map(Mutex::new),
                    surface,
                    thumbs,
                }
            }));
        }
        changed
    }

    fn present(
        &mut self,
        thumbs: &HashMap<u64, Arc<Tex>>,
        assets: &mut Assets<Image>,
        uploads: &mut Uploads,
    ) {
        self.images.retain(|id, _| thumbs.contains_key(id));
        for (&id, texture) in thumbs {
            if let Some(presented) = self.images.get_mut(&id) {
                if !Arc::ptr_eq(&presented.source, texture) {
                    presented.source = texture.clone();
                    uploads.0.insert(presented.image.id(), texture.clone());
                }
                continue;
            }
            let image = Image::new_uninit(
                Extent3d {
                    width: texture.width,
                    height: texture.height,
                    depth_or_array_layers: 1,
                },
                TextureDimension::D2,
                texture.format.add_srgb_suffix(),
                RenderAssetUsages::default(),
            );
            let image = assets.add(image);
            uploads.0.insert(image.id(), texture.clone());
            self.images.insert(
                id,
                Presented {
                    source: texture.clone(),
                    image,
                },
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use material_painter_graph::Value;

    #[test]
    fn moving_nodes_and_publishing_parameters_keep_the_preview_request() {
        let mut graph = Graph::starter("Материал");
        let id = graph.nodes[0].id;
        let mut previews = Previews::default();
        let bitmaps = Arc::default();
        previews.set(&graph, &bitmaps, None, None);
        let initial = previews.desired.as_ref().unwrap().clone();

        graph.node_mut(id).unwrap().pos = [14., 20.];
        graph.expose(id, "scale", &library());
        previews.set(&graph, &bitmaps, None, None);
        assert!(Arc::ptr_eq(previews.desired.as_ref().unwrap(), &initial));

        graph
            .node_mut(id)
            .unwrap()
            .params
            .insert("scale".into(), Value::Int(7));
        previews.set(&graph, &bitmaps, None, None);
        assert!(!Arc::ptr_eq(previews.desired.as_ref().unwrap(), &initial));
    }

    #[test]
    #[ignore = "requires GPU; checks reuse of thumbnail textures across a 64-node graph"]
    fn gpu_graph_preview_cache_only_replaces_affected_nodes() {
        use crate::editor::tests::lighting_ui_render::fixture_project;
        use std::{
            path::Path,
            time::{Duration, Instant},
        };

        let project = crate::EditorProject::load(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/legacy/project.json"),
        )
        .unwrap();
        let (app, _, _) = fixture_project(project);
        let device = app.world().resource::<RenderDevice>();
        let queue = app.world().resource::<RenderQueue>();
        let mut graph = Graph::new("Проверка предпросмотра");
        let source = graph.add("noise", [0.; 2]);
        let target = graph.add("levels", [0.; 2]);
        assert!(graph.connect(source, target, "input"));
        for _ in 2..64 {
            graph.add("checker", [0.; 2]);
        }

        let mut previews = Previews::default();
        let mut assets = Assets::default();
        let mut uploads = Uploads::default();
        let bitmaps = Arc::default();
        let mut evaluate = |previews: &mut Previews, graph: &Graph| {
            previews.set(graph, &bitmaps, None, None);
            let start = Instant::now();
            while !previews.update(&mut assets, &mut uploads, Some((device, queue))) {
                assert!(
                    start.elapsed() < Duration::from_secs(30),
                    "Preview evaluation timed out"
                );
                std::thread::sleep(Duration::from_millis(1));
            }
            start.elapsed()
        };
        let first = evaluate(&mut previews, &graph);
        assert_eq!(previews.images.len(), 64);
        let before: HashMap<_, _> = previews
            .images
            .iter()
            .map(|(&id, image)| (id, image.source.clone()))
            .collect();
        graph
            .node_mut(source)
            .unwrap()
            .params
            .insert("scale".into(), Value::Int(20));
        let updated = evaluate(&mut previews, &graph);
        for (&id, preview) in &previews.images {
            assert_eq!(
                Arc::ptr_eq(&before[&id], &preview.source),
                id != source && id != target
            );
        }
        println!("64-node preview job: initial {first:?}, update of two nodes {updated:?}");
    }
}
