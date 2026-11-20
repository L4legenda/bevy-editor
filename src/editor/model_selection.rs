//! Selection feedback belongs to the model viewport, never to the GLTF asset.
use super::*;
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
    window::RequestRedraw,
};

// The viewport tone maps HDR colors; a luminous core keeps stars readable on
// both dark materials and the default brightly lit orange model.
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
    color: Vec4,
    #[uniform(1)]
    style: Vec4,
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

// Geometry is rebuilt only for document/selection changes. Screen-size contours
// are expanded by the shader, and nebula coordinates remain object-local.
#[derive(Default)]
struct FeedbackMesh {
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    uvs: Vec<[f32; 2]>,
    coordinates: Vec<[f32; 4]>,
}
impl FeedbackMesh {
    fn push(&mut self, p: Vec3, n: Vec3, uv: Vec2, coordinate: Vec4) {
        self.positions.push(p.to_array());
        self.normals.push(n.to_array());
        self.uvs.push(uv.to_array());
        self.coordinates.push(coordinate.to_array());
    }
    fn star(&mut self, p: Vec3, coordinate: Vec3, active: bool) {
        let tag = if active { 2.0 / 3.0 } else { 1.0 / 3.0 };
        for uv in [
            Vec2::new(-1., -1.),
            Vec2::new(1., -1.),
            Vec2::ONE,
            Vec2::new(-1., -1.),
            Vec2::ONE,
            Vec2::new(-1., 1.),
        ] {
            self.push(p, Vec3::Z, uv, coordinate.extend(tag));
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
        .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, self.coordinates)
    }
}
fn geometry(doc: &Document, object: &Object, state: &State) -> FeedbackMesh {
    let mut result = FeedbackMesh::default();
    let mesh = &object.geometry;
    if mesh.positions.is_empty() {
        return result;
    }
    let (min, max) = mesh.positions.iter().fold(
        (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)),
        |(min, max), &p| (min.min(p), max.max(p)),
    );
    let center = (min + max) * 0.5;
    let span = (max - min).max_element().max(0.000001);
    let coordinate = |p: Vec3| (p - center) / span + Vec3::splat(0.5);
    let world = |p| object.transform.transform_point(p);
    if doc.selection.kind == Element::Vertex {
        for &vertex in &doc.selection.vertices {
            if let Some(&p) = mesh.positions.get(vertex) {
                result.star(world(p), coordinate(p), doc.selection.vertices.len() == 1);
            }
        }
        return result;
    }
    if doc.selection.kind == Element::Face {
        for &index in &doc.selection.faces {
            let Some(face) = mesh.faces.get(index) else {
                continue;
            };
            let n = mesh.normal(face).abs();
            let uv = |p: Vec3| {
                let p = coordinate(p);
                let projected = if n.x >= n.y && n.x >= n.z {
                    p.yz()
                } else if n.y >= n.z {
                    p.xz()
                } else {
                    p.xy()
                };
                projected + Vec2::new((index % 31) as f32 * 0.67, (index % 47) as f32 * 0.73)
            };
            for triangle in mesh.triangles(face) {
                let local = triangle.map(|corner| mesh.positions[face.corners[corner].vertex]);
                let points = local.map(world);
                let normal = (points[1] - points[0])
                    .cross(points[2] - points[0])
                    .normalize_or_zero();
                for i in 0..3 {
                    result.push(
                        points[i],
                        normal,
                        uv(local[i]),
                        coordinate(local[i]).extend(0.),
                    );
                }
            }
            let p = mesh.center(face);
            result.star(world(p), coordinate(p), false);
        }
    }
    if doc.selection.kind != Element::Vertex {
        if let Some(rendered) = state.render_objects.get(&object.id) {
            for (&edge, faces) in rendered.edges.iter() {
                let selected = if doc.selection.kind == Element::Edge {
                    doc.selection.edges.contains(&edge)
                } else {
                    faces.iter().any(|f| doc.selection.faces.contains(f))
                };
                if !selected {
                    continue;
                }
                let a = mesh.positions[edge.0];
                let b = mesh.positions[edge.1];
                let direction = (world(b) - world(a)).normalize_or_zero();
                for (p, uv) in [
                    (a, Vec2::new(-1., 0.)),
                    (a, Vec2::new(1., 0.)),
                    (b, Vec2::ONE),
                    (a, Vec2::new(-1., 0.)),
                    (b, Vec2::ONE),
                    (b, Vec2::new(-1., 1.)),
                ] {
                    result.push(world(p), direction, uv, coordinate(p).extend(1.0));
                }
            }
        }
    }
    result
}

#[derive(Resource, Default)]
pub(in crate::editor) struct Overlay {
    entity: Option<Entity>,
    mesh: Option<Handle<Mesh>>,
    materials: Option<[Handle<FaceMaterial>; 2]>,
    stamp: Option<(Entity, u64, u64)>,
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
    mut redraw: MessageWriter<RequestRedraw>,
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
            doc.mode == Mode::Edit && editor.pending.is_none() && editor.viewing_image.is_none()
        })
        .and_then(|doc| {
            doc.active()
                .filter(|object| !object.hidden)
                .map(|object| (doc, object))
        })
        .zip(viewer.root);
    let Some(((doc, object), root)) = selected else {
        if let Some(entity) = overlay.entity
            && let Ok((mut visible, _)) = fills.get_mut(entity)
        {
            *visible = Visibility::Hidden;
        }
        overlay.stamp = None;
        return;
    };
    let stamp = (root, doc.revision, doc.ui_revision);
    if overlay.stamp != Some(stamp) {
        let feedback = geometry(doc, object, &state);
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
                color: Vec4::new(0.22, 0.075, 0.40, 0.768),
                style: Vec4::new(window.scale_factor(), 0., 0., 0.),
                xray: false,
            }),
            materials.add(FaceMaterial {
                color: Vec4::new(0.22, 0.075, 0.40, 0.30),
                style: Vec4::new(window.scale_factor(), 0., 0., 0.),
                xray: true,
            }),
        ]);
    }
    for handle in overlay.materials.as_ref().unwrap() {
        if materials
            .get(handle)
            .is_some_and(|m| m.style.x != window.scale_factor())
            && let Some(mut m) = materials.get_mut(handle)
        {
            m.style.x = window.scale_factor();
        }
    }
    if overlay.geometry && window.focused {
        redraw.write(RequestRedraw);
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
