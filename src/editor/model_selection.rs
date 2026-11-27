//! Selection feedback belongs to the model viewport, never to the GLTF asset.
use super::*;
use crate::editor::model_mesh::ComponentHit;
use bevy::{
    asset::{AssetPath, RenderAssetUsages, embedded_asset, embedded_path},
    camera::visibility::NoFrustumCulling,
    gizmos::config::{GizmoConfigGroup, GizmoConfigStore},
    light::{NotShadowCaster, NotShadowReceiver},
    mesh::MeshVertexBufferLayoutRef,
    pbr::{MaterialPipeline, MaterialPipelineKey},
    reflect::TypePath,
    render::render_resource::{
        AsBindGroup, CompareFunction, RenderPipelineDescriptor, SpecializedMeshPipelineError,
    },
    shader::ShaderRef,
};

// Keep the object bounds and transform controls legible under HDR tonemapping.
pub(super) const SELECTED: Color = Color::linear_rgb(2.8, 2.2, 1.3);
pub(super) const BORDER: Color = Color::srgb(0.055, 0.035, 0.085);

#[derive(Default, Reflect, GizmoConfigGroup)]
pub(in crate::editor) struct SelectedDepth;
#[derive(Default, Reflect, GizmoConfigGroup)]
pub(in crate::editor) struct SelectedXray;

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
#[bind_group_data(FaceKey)]
pub(in crate::editor) struct FaceMaterial {
    #[uniform(0)]
    params: Vec4,
    xray: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(in crate::editor) struct FaceKey {
    xray: bool,
}
impl From<&FaceMaterial> for FaceKey {
    fn from(material: &FaceMaterial) -> Self {
        Self {
            xray: material.xray,
        }
    }
}
impl Material for FaceMaterial {
    fn vertex_shader() -> ShaderRef {
        Self::fragment_shader()
    }
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Path(
            AssetPath::from_path_buf(embedded_path!("model_selection.wesl"))
                .with_source("embedded"),
        )
    }
    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Blend
    }
    fn depth_bias(&self) -> f32 {
        // Draw X-Ray feedback after the translucent model and depth-writing gizmos.
        if self.xray { f32::INFINITY } else { 0.0 }
    }
    fn enable_prepass() -> bool {
        false
    }
    fn enable_shadows() -> bool {
        false
    }
    fn specialize(
        _: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _: &MeshVertexBufferLayoutRef,
        key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        descriptor.primitive.cull_mode = None;
        if let Some(depth) = &mut descriptor.depth_stencil {
            depth.depth_write_enabled = Some(false);
            depth.bias.constant = 8;
            depth.depth_compare = Some(if key.bind_group_data.xray {
                CompareFunction::Always
            } else {
                CompareFunction::GreaterEqual
            });
        }
        Ok(())
    }
}

// Mesh colors store the tint in RGB and the primitive type in alpha.
// A negative alpha marks a face fill during a transform.
const ORANGE: Vec3 = Vec3::new(1.0, 0.25, 0.018);
const ACTIVE: Vec3 = Vec3::ONE;
const CAGE: Vec3 = Vec3::new(0.012, 0.016, 0.022);
const WIRE_EDGE: Vec3 = Vec3::new(0.015, 0.022, 0.033);
const UV_SEAM: Vec3 = Vec3::new(1.0, 0.040, 0.013);

pub(super) fn highlighted(selection: &MeshSelection, edge: Edge, faces: &[usize]) -> bool {
    match selection.kind {
        Element::Vertex => {
            selection.vertices.contains(&edge.0) || selection.vertices.contains(&edge.1)
        }
        Element::Edge => selection.edges.contains(&edge),
        Element::Face => faces.iter().any(|face| selection.faces.contains(face)),
    }
}

#[derive(Default)]
struct FeedbackMesh {
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    uvs: Vec<[f32; 2]>,
    colors: Vec<[f32; 4]>,
}
impl FeedbackMesh {
    fn push(&mut self, point: Vec3, direction: Vec3, uv: Vec2, color: Vec4) {
        self.positions.push(point.to_array());
        self.normals.push(direction.to_array());
        self.uvs.push(uv.to_array());
        self.colors.push(color.to_array());
    }
    fn dot(&mut self, point: Vec3, color: Vec3) {
        for uv in [
            Vec2::new(-1., -1.),
            Vec2::new(1., -1.),
            Vec2::ONE,
            Vec2::new(-1., -1.),
            Vec2::ONE,
            Vec2::new(-1., 1.),
        ] {
            self.push(point, Vec3::Z, uv, color.extend(0.5));
        }
    }
    fn edge(&mut self, a: Vec3, b: Vec3, colors: [Vec3; 2]) {
        let direction = (b - a).normalize_or_zero();
        for (end, side) in [(0, -1.), (0, 1.), (1, 1.), (0, -1.), (1, 1.), (1, -1.)] {
            self.push(
                [a, b][end],
                direction,
                Vec2::new(side, end as f32),
                colors[end].extend(1.0),
            );
        }
    }
    fn mesh(self) -> Mesh {
        Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        )
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, self.positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, self.normals)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, self.uvs)
        .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, self.colors)
    }
}

fn selection_geometry(
    result: &mut FeedbackMesh,
    doc: &Document,
    object: &Object,
    state: &State,
    transforming: bool,
) {
    let mesh = &object.geometry;
    let selection = &doc.selection;
    let active = selection.active();
    let world = |p| object.transform.transform_point(p);
    for index in selection.selected_faces(mesh) {
        let face = &mesh.faces[index];
        let color = if active == Some(ComponentHit::Face(index)) {
            ORANGE.lerp(ACTIVE, 0.30)
        } else {
            ORANGE
        };
        let fill = if transforming && selection.kind == Element::Face {
            -1.0
        } else {
            0.0
        };
        for triangle in mesh.triangles(face) {
            for corner in triangle {
                result.push(
                    world(mesh.positions[face.corners[corner].vertex]),
                    Vec3::Z,
                    Vec2::ZERO,
                    color.extend(fill),
                );
            }
        }
    }
    if let Some(rendered) = state.render_objects.get(&object.id) {
        for (&edge, faces) in rendered.edges.iter() {
            if !highlighted(selection, edge, faces) {
                continue;
            }
            let colors = match selection.kind {
                Element::Vertex => [edge.0, edge.1].map(|i| {
                    if selection.vertices.contains(&i) {
                        ORANGE
                    } else {
                        CAGE
                    }
                }),
                Element::Edge if active == Some(ComponentHit::Edge(edge)) => [ACTIVE; 2],
                Element::Face
                    if active.is_some_and(|hit| match hit {
                        ComponentHit::Face(i) => faces.contains(&i),
                        _ => false,
                    }) =>
                {
                    [ACTIVE; 2]
                }
                _ => [ORANGE; 2],
            };
            result.edge(
                world(mesh.positions[edge.0]),
                world(mesh.positions[edge.1]),
                colors,
            );
        }
    }
    if selection.kind == Element::Vertex {
        for &vertex in &selection.vertices {
            if let Some(&point) = mesh.positions.get(vertex) {
                let color = if active == Some(ComponentHit::Vertex(vertex)) {
                    ACTIVE
                } else {
                    ORANGE
                };
                result.dot(world(point), color);
            }
        }
    }
}

fn geometry(doc: &Document, state: &State, transforming: bool) -> FeedbackMesh {
    let mut result = FeedbackMesh::default();
    let active = doc
        .active()
        .filter(|object| doc.mode == Mode::Edit && !object.hidden);
    // Use the same depth rule for the cage and its highlighted components.
    for object in doc.objects.iter().filter(|object| {
        !object.hidden && (state.xray || active.is_some_and(|active| active.id == object.id))
    }) {
        let Some(rendered) = state.render_objects.get(&object.id) else {
            continue;
        };
        let mesh = &object.geometry;
        let editing = active.is_some_and(|active| active.id == object.id);
        let world = |vertex| object.transform.transform_point(mesh.positions[vertex]);
        for (&edge, faces) in rendered.edges.iter() {
            if editing && highlighted(&doc.selection, edge, faces) {
                continue;
            }
            let color = if editing && mesh.uv_seams.contains(&edge) {
                UV_SEAM
            } else {
                WIRE_EDGE
            };
            result.edge(world(edge.0), world(edge.1), [color; 2]);
        }
        if editing && doc.selection.kind == Element::Vertex {
            for vertex in 0..mesh.positions.len() {
                if !doc.selection.vertices.contains(&vertex) {
                    result.dot(world(vertex), CAGE);
                }
            }
        }
    }
    if let Some(object) = active {
        selection_geometry(&mut result, doc, object, state, transforming);
    }
    result
}

#[derive(Resource, Default)]
pub(in crate::editor) struct Overlay {
    entity: Option<Entity>,
    mesh: Option<Handle<Mesh>>,
    materials: Option<[Handle<FaceMaterial>; 2]>,
    stamp: Option<(Entity, u64, u64, Mode, bool, bool)>,
    geometry: bool,
    pixel_scale: f32,
}
pub(in crate::editor) struct SelectionPlugin;
impl Plugin for SelectionPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "model_selection.wesl");
        app.add_plugins(MaterialPlugin::<FaceMaterial>::default())
            .init_resource::<Overlay>()
            .init_gizmo_group::<SelectedDepth>()
            .init_gizmo_group::<SelectedXray>()
            .add_systems(Startup, setup);
    }
}
fn setup(mut configs: ResMut<GizmoConfigStore>) {
    let (depth, _) = configs.config_mut::<SelectedDepth>();
    depth.render_layers = RenderLayers::layer(LAYER);
    depth.depth_bias = -0.01;
    depth.line.width = 3.2;
    let (xray, _) = configs.config_mut::<SelectedXray>();
    xray.render_layers = RenderLayers::layer(LAYER);
    xray.depth_bias = -1.0;
    xray.line.width = 3.2;
}

pub(in crate::editor) fn sync(
    editor: Res<Editor>,
    state: Res<State>,
    viewer: Res<Viewer>,
    mut overlay: ResMut<Overlay>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<FaceMaterial>>,
    mut commands: Commands,
    mut fills: Query<(&mut Visibility, &mut MeshMaterial3d<FaceMaterial>)>,
    window: Single<&Window, With<PrimaryWindow>>,
    mut configs: ResMut<GizmoConfigStore>,
) {
    let pixel_scale = window.scale_factor();
    if overlay.pixel_scale != pixel_scale {
        configs.config_mut::<SelectedDepth>().0.line.width = 3.2 * pixel_scale;
        configs.config_mut::<SelectedXray>().0.line.width = 3.2 * pixel_scale;
        overlay.pixel_scale = pixel_scale;
    }
    let selected = editor
        .viewing_model
        .and_then(|i| state.documents.get(&i))
        .filter(|doc| {
            (doc.mode == Mode::Edit || (doc.mode == Mode::Object && state.xray))
                && editor.pending.is_none()
                && editor.viewing_image.is_none()
        })
        .zip(viewer.root);
    let Some((doc, root)) = selected else {
        if let Some(entity) = overlay.entity
            && let Ok((mut visible, _)) = fills.get_mut(entity)
        {
            *visible = Visibility::Hidden;
        }
        overlay.stamp = None;
        return;
    };
    let transforming = state.gesture.as_ref().is_some_and(|gesture| {
        Some(gesture.model) == editor.viewing_model && gesture.transform_tool().is_some()
    });
    let stamp = (
        root, doc.revision, doc.ui_revision, doc.mode, state.xray, transforming,
    );
    if overlay.stamp != Some(stamp) {
        let feedback = geometry(doc, &state, transforming);
        overlay.geometry = !feedback.positions.is_empty();
        if overlay.geometry {
            let mesh = feedback.mesh();
            if let Some(handle) = &overlay.mesh {
                meshes.insert(handle.id(), mesh).unwrap();
            } else {
                overlay.mesh = Some(meshes.add(mesh));
            }
        }
        overlay.stamp = Some(stamp);
    }
    if overlay.materials.is_none() {
        overlay.materials = Some([
            materials.add(FaceMaterial {
                params: Vec4::new(window.scale_factor(), 0.24, 0., 0.),
                xray: false,
            }),
            materials.add(FaceMaterial {
                params: Vec4::new(window.scale_factor(), 0.11, 0., 0.),
                xray: true,
            }),
        ]);
    }
    for handle in overlay.materials.as_ref().unwrap() {
        if materials
            .get(handle)
            .is_some_and(|m| m.params.x != window.scale_factor())
            && let Some(mut m) = materials.get_mut(handle)
        {
            m.params.x = window.scale_factor();
        }
    }
    let material = overlay.materials.as_ref().unwrap()[usize::from(state.xray)].clone();
    if overlay.entity.is_none() && overlay.geometry {
        overlay.entity = Some(
            commands
                .spawn((
                    Name::new("Model selection overlay"),
                    Mesh3d(overlay.mesh.clone().unwrap()),
                    MeshMaterial3d(material.clone()),
                    Transform::IDENTITY,
                    RenderLayers::layer(LAYER),
                    Visibility::Visible,
                    NoFrustumCulling,
                    NotShadowCaster,
                    NotShadowReceiver,
                    Pickable::IGNORE,
                ))
                .id(),
        );
    }
    if let Some(entity) = overlay.entity
        && let Ok((mut visible, mut displayed)) = fills.get_mut(entity)
    {
        *visible = if overlay.geometry {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
        if displayed.0 != material {
            displayed.0 = material;
        }
    }
}

impl State {
    pub(in crate::editor) fn selection_label(&self, index: Option<usize>) -> Option<String> {
        let doc = self.documents.get(&index?)?;
        if doc.mode == Mode::Paint {
            return Some(format!("Рисование · {}", doc.active()?.name));
        }
        if doc.mode == Mode::Uv {
            return Some(format!("UV · {}", doc.active()?.name));
        }
        if doc.mode == Mode::Object {
            return Some(match doc.selected.len() {
                0 => "Объект · ничего не выбрано".into(),
                1 => format!("Объект · {}", doc.active()?.name),
                count => format!("Объекты · выбрано {count} · {}", doc.active()?.name),
            });
        }
        let object = doc.active()?;
        let (name, count, total) = match doc.selection.kind {
            Element::Vertex => (
                "Вершины",
                doc.selection.vertices.len(),
                object.geometry.positions.len(),
            ),
            Element::Edge => (
                "Рёбра",
                doc.selection.edges.len(),
                self.render_objects
                    .get(&object.id)
                    .map_or_else(|| object.geometry.edges().len(), |r| r.edges.len()),
            ),
            Element::Face => (
                "Грани",
                doc.selection.faces.len(),
                object.geometry.faces.len(),
            ),
        };
        Some(format!(
            "{name} · выбрано {count} из {total} · {}",
            object.name
        ))
    }
}

/// Screen-space border offsets, without rebuilding topology or casting rays.
pub(super) fn edge_border(
    viewer: &Viewer,
    rect: Rect,
    pose: &Transform,
    a: Vec3,
    b: Vec3,
) -> Option<[Vec3; 4]> {
    let pixel = |p: Vec3| {
        let p = p - pose.translation;
        let depth = p.dot(*pose.forward());
        (viewer.orthographic || depth > 0.000001).then(|| {
            let scale = view_height(viewer, depth) / rect.height();
            (
                Vec2::new(p.dot(*pose.right()), -p.dot(*pose.up())) / scale,
                scale,
            )
        })
    };
    let (pa, sa) = pixel(a)?;
    let (pb, sb) = pixel(b)?;
    let n = (pb - pa).perp().normalize_or_zero();
    let direction = (*pose.right() * n.x - *pose.up() * n.y) * 2.25;
    Some([
        a + direction * sa,
        b + direction * sb,
        a - direction * sa,
        b - direction * sb,
    ])
}
