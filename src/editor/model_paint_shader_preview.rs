//! Editable shading graphs for the model paint viewport.
use super::super::editing::xray::SourceMaterial;
use super::*;
use bevy::{
    asset::{AssetEvent, RenderAssetUsages, uuid::Uuid},
    mesh::MeshVertexBufferLayoutRef,
    pbr::{ExtendedMaterial, MaterialExtension, MaterialExtensionKey, MaterialExtensionPipeline},
    render::render_resource::{
        AsBindGroup, Extent3d, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError,
        TextureDimension, TextureFormat,
    },
    render::renderer::RenderDevice,
    shader::{Shader, ShaderDefVal},
};
use material_painter_graph::{Graph, Library, shading};
use std::{collections::HashSet, sync::OnceLock};

type PreviewMaterial = ExtendedMaterial<StandardMaterial, Extension>;

#[derive(Clone, Copy, Debug, PartialEq, ShaderType)]
struct Parameters {
    values: [Vec4; shading::MAX_SLOTS],
}

pub(super) struct Document {
    payload: Arc<serde_json::Value>,
    undo: Vec<Arc<serde_json::Value>>,
    redo: Vec<Arc<serde_json::Value>>,
}

impl Document {
    pub(super) fn new(graph: &Graph) -> Self {
        Self {
            payload: Arc::new(material_graph::encode(graph)),
            undo: Vec::new(),
            redo: Vec::new(),
        }
    }

    pub(super) fn data(&self) -> (&Arc<serde_json::Value>, &Arc<texture_fill::GraphImages>) {
        static EMPTY: OnceLock<Arc<texture_fill::GraphImages>> = OnceLock::new();
        (&self.payload, EMPTY.get_or_init(Arc::default))
    }

    pub(super) fn name(&self) -> &str {
        self.payload["name"].as_str().unwrap_or("Граф шейдинга")
    }

    pub(super) fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub(super) fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub(super) fn replace(&mut self, graph: &Graph) {
        let payload = material_graph::encode(graph);
        if payload != *self.payload {
            if self.undo.len() == 32 {
                self.undo.remove(0);
            }
            self.undo
                .push(std::mem::replace(&mut self.payload, Arc::new(payload)));
            self.redo.clear();
        }
    }

    pub(super) fn restore(&mut self, redo: bool) {
        let (source, destination) = if redo {
            (&mut self.redo, &mut self.undo)
        } else {
            (&mut self.undo, &mut self.redo)
        };
        if let Some(payload) = source.pop() {
            destination.push(std::mem::replace(&mut self.payload, payload));
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(in crate::editor::ui::model_viewer) struct ShaderKey(Uuid);

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
#[bind_group_data(ShaderKey)]
pub(in crate::editor::ui::model_viewer) struct Extension {
    shader: Uuid,
    #[uniform(100)]
    parameters: Parameters,
    #[texture(101)]
    #[sampler(105)]
    maps_a: Handle<Image>,
    #[texture(102)]
    maps_b: Handle<Image>,
    #[texture(103)]
    maps_e: Handle<Image>,
    #[texture(104)]
    height: Handle<Image>,
}

impl From<&Extension> for ShaderKey {
    fn from(extension: &Extension) -> Self {
        Self(extension.shader)
    }
}

impl MaterialExtension for Extension {
    fn specialize(
        _: &MaterialExtensionPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _: &MeshVertexBufferLayoutRef,
        key: MaterialExtensionKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        if let Some(fragment) = &mut descriptor.fragment
            && !fragment.shader_defs.iter().any(|definition| {
                matches!(definition, ShaderDefVal::Bool(name, true) if name == "PREPASS_PIPELINE")
            })
        {
            fragment.shader = Handle::Uuid(key.bind_group_data.0, std::marker::PhantomData);
        }
        Ok(())
    }
}

pub(super) fn presets() -> &'static [Graph] {
    static PRESETS: OnceLock<Vec<Graph>> = OnceLock::new();
    PRESETS.get_or_init(|| {
        let mut graphs = shading::presets();
        graphs.push(shading::starter("PBR"));
        graphs
    })
}

const MAPS: [[Option<(bake::BakeMap, usize)>; 4]; 3] = {
    use bake::BakeMap::*;
    [
        [
            Some((Occlusion, 0)),
            Some((Curvature, 0)),
            Some((Cavity, 0)),
            Some((Thickness, 0)),
        ],
        [
            Some((Sky, 0)),
            Some((EdgeDistance, 0)),
            Some((CreaseDistance, 0)),
            Some((PartRandom, 0)),
        ],
        [
            Some((PartId, 0)),
            Some((PartId, 1)),
            Some((PartId, 2)),
            None,
        ],
    ]
};
const NEUTRAL: [[u8; 4]; 3] = [
    [255, 128, 255, 255],
    [255, 255, 255, 128],
    [128, 128, 128, 255],
];

fn image(size: u32, data: Vec<u8>) -> Image {
    Image::new(
        Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::default(),
    )
}

#[derive(Default, Resource)]
pub(in crate::editor::ui::model_viewer) struct Cache {
    program: Option<((usize, usize), Program)>,
    draft: Option<(u64, Program)>,
    observed: Option<((usize, usize), Arc<serde_json::Value>)>,
    diagnostic: Option<String>,
    defaults: Option<([Handle<Image>; 3], Handle<Image>)>,
    maps: HashMap<(usize, sets::SetKey), (Arc<bake::Maps>, [Handle<Image>; 3])>,
}

impl Cache {
    pub(super) fn diagnostic(&self, model: usize, preset: usize) -> Option<&str> {
        self.observed
            .as_ref()
            .filter(|(target, _)| *target == (model, preset))
            .and(self.diagnostic.as_deref())
    }
}

struct Program {
    shader: Uuid,
    code: String,
    parameters: Parameters,
}

#[derive(Clone, Copy)]
enum Preview<'a> {
    Graph(usize),
    Draft(u64, &'a ShadingSource),
}

pub(super) struct ShadingSource {
    compiled: shading::Compiled,
    parameters: Parameters,
}

impl ShadingSource {
    pub(super) fn compile(graph: &Graph, library: &Library) -> Result<Self, String> {
        let compiled = shading::compile(graph, library)?;
        let mut parameters = Parameters {
            values: [Vec4::ZERO; shading::MAX_SLOTS],
        };
        for (slot, values) in parameters
            .values
            .iter_mut()
            .zip(shading::param_values(&compiled, graph, library))
        {
            *slot = Vec4::from_array(values);
        }
        Ok(Self {
            compiled,
            parameters,
        })
    }

    pub(super) fn validate(&self, device: &RenderDevice) -> Result<(), String> {
        shading::validate(device.wgpu_device(), &self.compiled)
    }

    pub(super) fn validate_node(&self, device: &RenderDevice, node: u64) -> Result<(), String> {
        shading::validate_node(device.wgpu_device(), &self.compiled, node)
    }

    fn changes_code(&self, previous: Option<&Program>) -> bool {
        previous.is_none_or(|program| program.code != self.compiled.code)
    }
}

#[derive(Component)]
struct Applied {
    source: Handle<StandardMaterial>,
    handle: Handle<PreviewMaterial>,
}

pub(in crate::editor::ui::model_viewer) struct Plugin;

impl bevy::prelude::Plugin for Plugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Cache>()
            .add_plugins(MaterialPlugin::<PreviewMaterial>::default())
            .add_systems(PostUpdate, sync);
    }
}

fn install(
    source: &ShadingSource,
    previous: Option<&Program>,
    path: &'static str,
    shaders: &mut Assets<Shader>,
) -> Result<Program, String> {
    let shader = previous.map_or_else(Uuid::new_v4, |program| program.shader);
    if source.changes_code(previous) {
        let code = format!(
            "{}\n{}\n{}",
            include_str!("model_paint_shader_preview.wesl"),
            shading::CONTEXT,
            source.compiled.code
        );
        shaders
            .insert(
                bevy::asset::AssetId::Uuid { uuid: shader },
                Shader::from_wesl(code, path),
            )
            .map_err(|error| error.to_string())?;
    }
    Ok(Program {
        shader,
        code: source.compiled.code.clone(),
        parameters: source.parameters,
    })
}

fn maps_for(
    cache: &mut Cache,
    model: usize,
    key: sets::SetKey,
    baked: Option<&Arc<bake::Maps>>,
    images: &mut Assets<Image>,
) -> [Handle<Image>; 3] {
    let neutral = &cache.defaults.as_ref().unwrap().0;
    let Some(baked) = baked else {
        cache.maps.remove(&(model, key));
        return neutral.clone();
    };
    if let Some((previous, handles)) = cache.maps.get(&(model, key))
        && Arc::ptr_eq(previous, baked)
    {
        return handles.clone();
    }
    let handles = std::array::from_fn(|index| {
        let components = MAPS[index].map(|source| {
            let (map, channel) = source?;
            baked.graph_component(map, channel)
        });
        if components.iter().all(Option::is_none) {
            return neutral[index].clone();
        }
        let edge = baked.size.min(256);
        let sample = |p| (2 * p + 1) * baked.size / (2 * edge);
        let mut data = Vec::with_capacity((edge * edge * 4) as usize);
        for y in 0..edge {
            for x in 0..edge {
                let offset = (sample(y) * baked.size + sample(x)) as usize;
                data.extend(std::array::from_fn::<_, 4, _>(|channel| {
                    components[channel]
                        .as_ref()
                        .map_or(NEUTRAL[index][channel], |read| read(offset))
                }));
            }
        }
        images.add(image(edge, data))
    });
    cache
        .maps
        .insert((model, key), (baked.clone(), handles.clone()));
    handles
}

fn sync(
    mut commands: Commands,
    mut editor: ResMut<Editor>,
    editing: Res<editing::State>,
    paint: Res<PaintState>,
    baked: Res<bake::State>,
    runtime: Res<engine::Runtime>,
    maker: Res<super::ShaderMaker>,
    mut cache: ResMut<Cache>,
    device: Option<Res<RenderDevice>>,
    mut assets: (
        ResMut<Assets<Shader>>,
        ResMut<Assets<Image>>,
        ResMut<Assets<PreviewMaterial>>,
    ),
    materials: Res<Assets<StandardMaterial>>,
    mut changes: MessageReader<AssetEvent<StandardMaterial>>,
    parts: Query<(Entity, &SourceMaterial, &sets::SetPart, Option<&Applied>)>,
) {
    let changed: HashSet<_> = changes
        .read()
        .filter_map(|change| match change {
            AssetEvent::Modified { id } => Some(*id),
            _ => None,
        })
        .collect();
    let (shaders, images, preview_materials) = &mut assets;
    let active = editor
        .viewing_model
        .filter(|_| editor.pending.is_none() && editor.viewing_image.is_none())
        .and_then(|model| {
            let doc = editing.documents.get(&model)?;
            let session = paint.sessions.get(&model)?;
            if doc.mode != Mode::Paint {
                return None;
            }
            let view = if session.panel == Panel::Shader {
                maker
                    .draft()
                    .map(|(revision, source)| Preview::Draft(revision, source))?
            } else {
                if session.diagnostic_3d() {
                    return None;
                }
                Preview::Graph(session.preview_shader?)
            };
            Some((model, doc, session, view))
        });
    const COMPILE_ERROR: &str = "Не удалось собрать шейдер предпросмотра · ";
    if let Some((model, _, session, view)) = active {
        cache.maps.retain(|(index, _), _| *index == model);
        if let Preview::Draft(revision, source) = view {
            if cache
                .draft
                .as_ref()
                .is_none_or(|(current, _)| *current != revision)
            {
                let previous = cache.draft.as_ref().map(|(_, program)| program);
                if let Ok(program) = install(source, previous, "editor/shading_draft.wesl", shaders)
                {
                    cache.draft = Some((revision, program));
                }
            }
        } else if let Preview::Graph(preset) = view
            && let Some(document) = session.shader_graphs.get(&preset)
            && cache.observed.as_ref().is_none_or(|(key, payload)| {
                *key != (model, preset) || !Arc::ptr_eq(payload, &document.payload)
            })
            && let Some(device) = device.as_deref()
        {
            let target = (model, preset);
            cache.observed = Some((target, document.payload.clone()));
            let previous = cache.program.as_ref().map(|(_, program)| program);
            let result = (|| {
                let graph = material_graph::read(&document.payload)?;
                let source = ShadingSource::compile(&graph, &material_graph::library())?;
                if source.changes_code(previous) {
                    source.validate(device)?;
                }
                install(&source, previous, "editor/shading_preview.wesl", shaders)
            })();
            match result {
                Ok(program) => {
                    cache.program = Some((target, program));
                    cache.diagnostic = None;
                    if editor.status.starts_with(COMPILE_ERROR) {
                        editor.status.clear();
                    }
                }
                Err(error) => {
                    editor.status = format!("{COMPILE_ERROR}{error}");
                    cache.diagnostic = Some(editor.status.clone());
                }
            }
        }
        cache.defaults.get_or_insert_with(|| {
            (
                NEUTRAL.map(|rgba| images.add(image(1, rgba.to_vec()))),
                images.add(image(1, vec![128, 128, 128, 255])),
            )
        });
    } else {
        cache.maps.clear();
        if editor.status.starts_with(COMPILE_ERROR) {
            editor.status.clear();
        }
    }

    for (entity, source, part, applied) in &parts {
        let selected = active.and_then(|(model, doc, session, view)| {
            let program = match view {
                Preview::Draft(..) => cache.draft.as_ref().map(|(_, program)| program),
                Preview::Graph(preset) => cache
                    .program
                    .as_ref()
                    .filter(|(target, _)| *target == (model, preset))
                    .map(|(_, program)| program),
            }?;
            let base = runtime
                .working_material(model, part.0, part.1)
                .unwrap_or(&source.0);
            Some((
                model,
                doc,
                session,
                program.shader,
                program.parameters,
                base,
            ))
        });
        let Some((model, doc, session, shader, parameters, source)) = selected else {
            if let Some(applied) = applied {
                commands
                    .entity(entity)
                    .remove::<(Applied, MeshMaterial3d<PreviewMaterial>)>()
                    .insert(MeshMaterial3d(applied.source.clone()));
            }
            continue;
        };
        let Some(base) = materials.get(source) else {
            continue;
        };
        let key = (part.0, part.1);
        let maps = baked.maps(model, doc, key, session.texture_size);
        let [maps_a, maps_b, maps_e] = maps_for(&mut cache, model, key, maps, images);
        let height = runtime
            .preview(model, part.0, part.1, 3)
            .unwrap_or_else(|| cache.defaults.as_ref().unwrap().1.clone());
        let extension = Extension {
            shader,
            parameters,
            maps_a,
            maps_b,
            maps_e,
            height,
        };
        if let Some(applied) = applied
            && applied.source == *source
            && !changed.contains(&source.id())
            && preview_materials
                .get(&applied.handle)
                .is_some_and(|material| {
                    material.extension.shader == extension.shader
                        && material.extension.height == extension.height
                        && material.extension.maps_a == extension.maps_a
                        && material.extension.maps_b == extension.maps_b
                        && material.extension.maps_e == extension.maps_e
                })
        {
            let handle = &applied.handle;
            if preview_materials.get(handle).unwrap().extension.parameters != parameters {
                preview_materials
                    .get_mut(handle)
                    .unwrap()
                    .extension
                    .parameters = parameters;
            }
            continue;
        }
        let mut base = base.clone();
        base.opaque_render_method = bevy::material::OpaqueRendererMethod::Forward;
        let handle = preview_materials.add(PreviewMaterial { base, extension });
        commands
            .entity(entity)
            .remove::<MeshMaterial3d<StandardMaterial>>()
            .insert((
                MeshMaterial3d(handle.clone()),
                Applied {
                    source: source.clone(),
                    handle,
                },
            ));
    }
}
