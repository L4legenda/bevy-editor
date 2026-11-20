//! GPU UV atlas and view-only model materials. Assets and scene instances stay untouched.
use super::*;
use bevy::{
    asset::RenderAssetUsages,
    camera::{ClearColorConfig, RenderTarget, ScalingMode, visibility::RenderLayers},
    gizmos::config::{GizmoConfigGroup, GizmoConfigStore},
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
};
pub(super) const UV_LAYER: usize = 19;
#[derive(Component)]
pub(in crate::editor) struct AtlasCamera;
#[derive(Component)]
struct AtlasGeometry;
#[derive(Default, Reflect, GizmoConfigGroup)]
pub(super) struct UvSeams;
#[derive(Resource, Default)]
pub(super) struct AssetsState {
    pub(super) checker: Option<Handle<StandardMaterial>>,
    pub(super) flat: Option<Handle<StandardMaterial>>,
    entities: Vec<Entity>,
    meshes: Vec<Handle<Mesh>>,
    stamp: Option<(usize, usize, u64, bool, Vec<usize>, u32, u64)>,
    overlay: Option<Entity>,
    overlay_key: Option<(Entity, usize, u64, bool, u64)>,
    overlay_material: Option<Handle<StandardMaterial>>,
}
pub(super) fn setup(commands: &mut Commands, image: Handle<Image>) {
    commands.spawn((
        AtlasCamera,
        Camera3d::default(),
        Camera {
            is_active: false,
            order: -3,
            clear_color: ClearColorConfig::Custom(Color::srgb(0.035, 0.045, 0.065)),
            ..default()
        },
        RenderTarget::Image(image.into()),
        RenderLayers::layer(UV_LAYER),
        Transform::from_xyz(0.5, 0.5, 10.),
        Projection::Orthographic(OrthographicProjection {
            scaling_mode: ScalingMode::FixedVertical {
                viewport_height: 1.2,
            },
            near: 0.01,
            far: 100.,
            ..OrthographicProjection::default_3d()
        }),
        bevy::render::view::Tonemapping::None,
        bevy::render::view::DebandDither::Disabled,
        Msaa::Off,
    ));
}
pub(super) fn setup_gizmos(mut configs: ResMut<GizmoConfigStore>) {
    let (config, _) = configs.config_mut::<UvSeams>();
    config.render_layers = RenderLayers::layer(LAYER);
    config.depth_bias = -0.0002;
    config.line.width = 1.8;
}

#[derive(Default)]
struct Drawing {
    positions: Vec<[f32; 3]>,
    colors: Vec<[f32; 4]>,
}
impl Drawing {
    fn triangle(&mut self, points: [Vec2; 3], depth: f32, color: Color) {
        for p in points {
            self.positions.push([p.x, p.y, depth]);
            self.colors.push(color.to_linear().to_f32_array());
        }
    }
    fn line(&mut self, a: Vec2, b: Vec2, depth: f32, width: f32, color: Color) {
        let n = (b - a).perp().normalize_or_zero() * width * 0.5;
        self.triangle([a + n, a - n, b + n], depth, color);
        self.triangle([a - n, b - n, b + n], depth, color);
    }
    fn mesh(self) -> Mesh {
        let n = self.positions.len();
        Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        )
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, self.positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0., 0., 1.]; n])
        .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, self.colors)
    }
}
fn hue(id: usize) -> Color {
    let h = (id as f32 * 0.618034).fract();
    Color::hsl(h * 360., 0.46, 0.56)
}
pub(super) fn refresh(
    mut commands: Commands,
    editor: Res<Editor>,
    editing: Res<State>,
    uv: Res<UvState>,
    ui: Res<ui::UvUi>,
    mut cache: ResMut<AssetsState>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut meshes: ResMut<Assets<Mesh>>,
    canvas: Query<(&ComputedNode, &UiGlobalTransform), With<ui::UvCanvas>>,
    mut cameras: Query<(&mut Camera, &mut Transform, &mut Projection), With<AtlasCamera>>,
) {
    let Ok((mut camera, mut transform, mut projection)) = cameras.single_mut() else {
        return;
    };
    let doc = editor.viewing_model.and_then(|i| editing.documents.get(&i));
    let showing =
        doc.is_some_and(|d| d.mode == Mode::Uv) && editor.pending.is_none() && uv.split > 0.002;
    camera.is_active = showing;
    if !showing {
        cache.stamp = None;
        return;
    }
    let Some((index, doc, object)) = editor.viewing_model.and_then(|i| {
        editing
            .documents
            .get(&i)
            .and_then(|d| d.active().map(|o| (i, d, o)))
    }) else {
        return;
    };
    let Ok((computed, _)) = canvas.get(ui.canvas) else {
        return;
    };
    let pixels = computed.size().max(Vec2::ONE).round().as_uvec2();
    if let Some(mut image) = images.get_mut(&ui.image) {
        if image.texture_descriptor.size.width != pixels.x
            || image.texture_descriptor.size.height != pixels.y
        {
            image.resize(Extent3d {
                width: pixels.x,
                height: pixels.y,
                depth_or_array_layers: 1,
            });
        }
    }
    let aspect = (pixels.x as f32 / pixels.y as f32).max(0.01);
    let height = 1.18_f32.max(1.18 / aspect) / uv.zoom;
    *transform = Transform::from_xyz(uv.center.x, uv.center.y, 10.);
    *projection = Projection::Orthographic(OrthographicProjection {
        scaling_mode: ScalingMode::FixedVertical {
            viewport_height: height,
        },
        near: 0.01,
        far: 100.,
        ..OrthographicProjection::default_3d()
    });
    let width = height / (pixels.y as f32).max(1.);
    let line_key = (width * 1e7).round() as u32;
    let stamp = (
        index,
        object.id,
        doc.revision,
        uv.comparing,
        uv.selected.iter().copied().collect(),
        line_key,
        uv.overlay_revision,
    );
    if cache.stamp.as_ref() == Some(&stamp) {
        return;
    }
    cache.stamp = Some(stamp);
    if cache
        .flat
        .as_ref()
        .is_none_or(|h| !materials.contains(h.id()))
    {
        cache.flat = Some(materials.add(StandardMaterial {
            base_color: Color::WHITE,
            unlit: true,
            double_sided: true,
            cull_mode: None,
            ..default()
        }));
    }
    for entity in std::mem::take(&mut cache.entities) {
        commands.entity(entity).try_despawn();
    }
    cache.meshes.clear();
    let geometry = uv.display_geometry(index, object);
    let mut backdrop = Drawing::default();
    for x in 0..16 {
        for y in 0..16 {
            let a = Vec2::new(x as f32 / 16., y as f32 / 16.);
            let b = a + Vec2::splat(1. / 16.);
            let color = if (x + y) % 2 == 0 {
                Color::srgb(0.065, 0.085, 0.115)
            } else {
                Color::srgb(0.08, 0.10, 0.13)
            };
            backdrop.triangle([a, Vec2::new(b.x, a.y), b], -0.02, color);
            backdrop.triangle([a, b, Vec2::new(a.x, b.y)], -0.02, color);
        }
    }
    let mut faces = Drawing::default();
    let mut lines = Drawing::default();
    let shades = Shades::new(index, &uv, doc, object);
    for (fi, face) in geometry.faces.iter().enumerate() {
        let id = uv.atlas.islands.get(fi).copied().unwrap_or(0);
        let selected = uv.selected.contains(&id);
        let color = if selected {
            Color::srgb(0.95, 0.72, 0.32)
        } else {
            hue(id)
        };
        for (ti, triangle) in geometry.triangles(face).into_iter().enumerate() {
            let shade = shades
                .color(&uv, fi, ti, triangle.map(|i| face.corners[i].vertex))
                .unwrap_or(color);
            faces.triangle(triangle.map(|i| face.corners[i].uv), 0., shade);
        }
        for (a, b) in face.corners.iter().zip(face.corners.iter().cycle().skip(1)) {
            lines.line(
                a.uv,
                b.uv,
                0.003,
                width * if selected { 1.7 } else { 0.8 },
                if selected {
                    Color::srgb(1., 0.88, 0.55)
                } else {
                    Color::srgb(0.11, 0.16, 0.22)
                },
            );
        }
    }
    for (a, b) in [
        (Vec2::ZERO, Vec2::X),
        (Vec2::X, Vec2::ONE),
        (Vec2::ONE, Vec2::Y),
        (Vec2::Y, Vec2::ZERO),
    ] {
        lines.line(a, b, 0.004, width * 1.5, Color::srgb(0.32, 0.50, 0.59));
    }
    for drawing in [backdrop, faces, lines] {
        let mesh = meshes.add(drawing.mesh());
        let entity = commands
            .spawn((
                AtlasGeometry,
                Mesh3d(mesh.clone()),
                MeshMaterial3d(cache.flat.clone().unwrap()),
                RenderLayers::layer(UV_LAYER),
                Transform::IDENTITY,
                Visibility::Visible,
            ))
            .id();
        cache.entities.push(entity);
        cache.meshes.push(mesh);
    }
}

pub(super) fn materials(
    editor: Res<Editor>,
    editing: Res<State>,
    uv: Res<UvState>,
    mut cache: ResMut<AssetsState>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut displayed: Query<(&xray::SourceMaterial, &mut MeshMaterial3d<StandardMaterial>)>,
) {
    let showing = editor
        .viewing_model
        .and_then(|i| editing.documents.get(&i))
        .is_some_and(|d| d.mode == Mode::Uv);
    if !showing || !uv.checker {
        return;
    }
    if cache
        .checker
        .as_ref()
        .is_none_or(|h| !materials.contains(h.id()))
    {
        let size = 512usize;
        let mut data = Vec::with_capacity(size * size * 4);
        for y in 0..size {
            for x in 0..size {
                data.extend_from_slice(if (x / 16 + y / 16) % 2 == 0 {
                    &[193, 220, 226, 255]
                } else {
                    &[47, 74, 104, 255]
                });
            }
        }
        let mut image = Image::new(
            Extent3d {
                width: size as u32,
                height: size as u32,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            data,
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::default(),
        );
        image.sampler = bevy::image::ImageSampler::nearest();
        cache.checker = Some(materials.add(StandardMaterial {
            base_color: Color::WHITE,
            base_color_texture: Some(images.add(image)),
            perceptual_roughness: 0.8,
            double_sided: true,
            cull_mode: None,
            ..default()
        }));
    }
    for (_, mut material) in &mut displayed {
        if Some(&material.0) != cache.checker.as_ref() {
            material.0 = cache.checker.clone().unwrap();
        }
    }
}
pub(super) fn seams(
    editor: Res<Editor>,
    editing: Res<State>,
    uv: Res<UvState>,
    mut gizmos: Gizmos<UvSeams>,
) {
    let Some((index, doc, object)) = editor.viewing_model.and_then(|i| {
        editing
            .documents
            .get(&i)
            .and_then(|d| d.active().map(|o| (i, d, o)))
    }) else {
        return;
    };
    if doc.mode != Mode::Uv || editor.pending.is_some() {
        return;
    }
    let geometry = uv.display_geometry(index, object);
    let world = |i| object.transform.transform_point(geometry.positions[i]);
    if uv.seams {
        for edge in &uv.atlas.seams {
            gizmos.line(world(edge.0), world(edge.1), Color::srgb(1., 0.24, 0.12));
        }
    }
    if !uv.selected.is_empty() && uv.selected.len() < uv.atlas.count {
        let mut edges = BTreeSet::new();
        for (fi, face) in geometry.faces.iter().enumerate() {
            if uv
                .atlas
                .islands
                .get(fi)
                .is_some_and(|id| uv.selected.contains(id))
            {
                edges.extend(face.edges().map(|(a, b)| Edge::new(a, b)));
            }
        }
        for edge in edges.into_iter().take(12_000) {
            gizmos.line(world(edge.0), world(edge.1), Color::srgb(1., 0.8, 0.3));
        }
    }
}

struct Shades<'a> {
    mask: Option<&'a FaceMask>,
    proposal: Option<&'a FaceMask>,
}
impl<'a> Shades<'a> {
    fn new(index: usize, uv: &'a UvState, doc: &Document, object: &Object) -> Self {
        Self {
            mask: uv.masks.get(&(doc.source.path.clone(), object.id)),
            proposal: uv
                .proposal
                .as_ref()
                .filter(|p| {
                    p.index == index && p.object == object.id && p.original == object.geometry
                })
                .and_then(|p| match &p.result {
                    Proposed::Island(mask) => Some(mask),
                    _ => None,
                }),
        }
    }
    fn color(&self, uv: &UvState, fi: usize, ti: usize, vertices: [usize; 3]) -> Option<Color> {
        let key = face_key(vertices);
        if self.proposal.is_some_and(|m| m.contains(&key)) {
            return Some(Color::srgb(0.77, 0.4, 1.));
        }
        if self.mask.is_some_and(|m| m.contains(&key)) && (!uv.heatmap || uv.tool == Tool::Brush) {
            return Some(Color::srgb(0.22, 0.78, 0.95));
        }
        if !uv.heatmap {
            return None;
        }
        let ratio = *uv.atlas.distortion.get(fi)?.get(ti)?;
        if !ratio.is_finite() {
            return Some(Color::srgb(0.9, 0.08, 0.62));
        }
        let t = ((ratio - 1.) * 2.).clamp(0., 2.);
        let stops = [
            Vec3::new(0.16, 0.72, 0.66),
            Vec3::new(1., 0.76, 0.22),
            Vec3::new(0.94, 0.17, 0.15),
        ];
        let c = if t < 1. {
            stops[0].lerp(stops[1], t)
        } else {
            stops[1].lerp(stops[2], t - 1.)
        };
        Some(Color::srgb(c.x, c.y, c.z))
    }
}
pub(super) fn overlay(
    mut commands: Commands,
    editor: Res<Editor>,
    editing: Res<State>,
    uv: Res<UvState>,
    mut cache: ResMut<AssetsState>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let active = editor.viewing_model.and_then(|index| {
        let doc = editing.documents.get(&index)?;
        let object = doc.active()?;
        let entity = editing.render_objects.get(&object.id)?.entity;
        (doc.mode == Mode::Uv
            && !object.hidden
            && editor.pending.is_none()
            && editor.viewing_image.is_none())
        .then_some((index, doc, object, entity))
    });
    let key = active
        .map(|(idx, d, _, parent)| (parent, idx, d.revision, uv.comparing, uv.overlay_revision));
    if key == cache.overlay_key {
        return;
    }
    cache.overlay_key = key;
    if let Some(old) = cache.overlay.take() {
        commands.entity(old).try_despawn();
    }
    let Some((index, doc, object, parent)) = active else {
        return;
    };
    let shades = Shades::new(index, &uv, doc, object);
    if !uv.heatmap && shades.mask.is_none_or(|m| m.is_empty()) && shades.proposal.is_none() {
        return;
    }
    let geom = uv.display_geometry(index, object);
    let mut drawing = Drawing::default();
    let key_light = Vec3::new(-0.4, 0.8, 0.45).normalize();
    for (fi, face) in geom.faces.iter().enumerate() {
        let normal = geom.normal(face);
        for (ti, tri) in geom.triangles(face).into_iter().enumerate() {
            let vertices = tri.map(|i| face.corners[i].vertex);
            if let Some(shade) = shades.color(&uv, fi, ti, vertices) {
                drawing
                    .positions
                    .extend(vertices.map(|i| geom.positions[i].to_array()));
                let [r, g, b, a] = shade.to_linear().to_f32_array();
                drawing.colors.extend(tri.map(|c| {
                    let normal = face.corners[c].source_normal.unwrap_or(normal);
                    let light = 0.72 + 0.28 * normal.dot(key_light).clamp(-1., 1.);
                    [r * light, g * light, b * light, a]
                }));
            }
        }
    }
    if drawing.positions.is_empty() {
        return;
    }
    let material = cache
        .overlay_material
        .get_or_insert_with(|| {
            materials.add(StandardMaterial {
                unlit: true,
                cull_mode: None,
                double_sided: true,
                depth_bias: 8.,
                ..default()
            })
        })
        .clone();
    cache.overlay = Some(
        commands
            .spawn((
                Mesh3d(meshes.add(drawing.mesh())),
                MeshMaterial3d(material),
                RenderLayers::layer(LAYER),
                Transform::IDENTITY,
                ChildOf(parent),
            ))
            .id(),
    );
}
