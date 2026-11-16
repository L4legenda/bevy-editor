//! One antialiased curve per connection, without seams between UI rectangles.
use super::*;

#[derive(Component)]
pub(super) struct Connection {
    pub from: u32,
    pub choice: usize,
    pub target: u32,
}

#[derive(Component)]
pub(super) struct Preview;

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub(super) struct Material {
    #[uniform(0)]
    ends: Vec4,
    #[uniform(1)]
    controls: Vec4,
    #[uniform(2)]
    pub(super) tint: Vec4,
    #[uniform(3)]
    pub(super) metrics: Vec4,
    #[uniform(4)]
    pub(super) motion: Vec4,
    a: Vec2,
    b: Vec2,
    zoom: f32,
}

impl UiMaterial for Material {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Path(
            AssetPath::from_path_buf(embedded_path!("narrative_connection.wesl"))
                .with_source("embedded"),
        )
    }
}

pub(super) fn register(app: &mut App) {
    embedded_asset!(app, "narrative_connection.wesl");
    app.add_plugins(UiMaterialPlugin::<Material>::default());
}

pub(super) fn style(
    editor: Res<Editor>,
    links: Res<links::State>,
    effects: Res<effects::State>,
    time: Res<Time>,
    mut wires: Query<(&Connection, &MaterialNode<Material>, &mut Node), Without<Preview>>,
    mut previews: Query<(&MaterialNode<Material>, &mut Node), (With<Preview>, Without<Connection>)>,
    mut materials: ResMut<Assets<Material>>,
) {
    let Some(doc) = editor.narrative.current() else {
        return;
    };
    let Content::Dialogue(d) = &doc.content else {
        return;
    };
    for (connection, handle, mut node) in &mut wires {
        let highlighted =
            links.highlights_edge(connection.from, connection.choice, connection.target);
        let color = if links.drag.as_ref().is_some_and(|drag| {
            drag.moved && (drag.from, drag.choice) == (connection.from, connection.choice)
        }) {
            FOCUS.with_alpha(0.14)
        } else if highlighted {
            GREEN
        } else {
            FOCUS.with_alpha(if connection.from == doc.selected {
                0.85
            } else {
                0.48
            })
        }
        .to_linear();
        let tint = Vec4::new(color.red, color.green, color.blue, color.alpha);
        let width = if highlighted { 2.8 } else { 1.7 };
        let Some(source) = d.node(connection.from) else {
            continue;
        };
        let Some(target) = d.node(connection.target) else {
            continue;
        };
        let a = effects::point(
            &effects,
            doc,
            source,
            effects.answers.output_position(source, connection.choice),
        );
        let b = effects::point(&effects, doc, target, nodes::input_position());
        let motion = effects
            .arrival(effects.edge(connection.from, connection.choice, connection.target))
            .map_or(
                Vec4::new(1., 0., 0., if highlighted { 1. } else { 0. }),
                |p| Vec4::new(p, 0., 1., 0.),
            );
        if materials.get(&handle.0).is_some_and(|m| {
            m.tint != tint
                || m.metrics.z != width
                || m.motion != motion
                || m.a != a
                || m.b != b
                || m.zoom != doc.zoom
        }) {
            if let Some(mut material) = materials.get_mut(&handle.0) {
                material.tint = tint;
                material.metrics.z = width;
                material.motion = motion;
                geometry(&mut node, &mut material, a, b, doc.zoom);
            }
        }
    }
    for (handle, mut node) in &mut previews {
        if let Some(mut material) = materials.get_mut(&handle.0) {
            material.motion = Vec4::new(1., time.elapsed_secs() % 100., 3., 0.);
            if let Some(drag) = links.drag.as_ref().filter(|drag| drag.moved) {
                if let Some(source) = d.node(drag.from) {
                    let a = effects::point(
                        &effects,
                        doc,
                        source,
                        effects.answers.output_position(source, drag.choice),
                    );
                    let b = drag
                        .target
                        .and_then(|id| d.node(id))
                        .map_or(drag.pointer, |target| {
                            effects::point(&effects, doc, target, nodes::input_position())
                        });
                    geometry(&mut node, &mut material, a, b, doc.zoom);
                }
            }
        }
    }
}

pub(super) fn wire(
    commands: &mut Commands,
    parent: Entity,
    a: Vec2,
    b: Vec2,
    color: Color,
    zoom: f32,
    materials: &mut Assets<Material>,
) -> Entity {
    let mut layout = Node {
        position_type: PositionType::Absolute,
        ..default()
    };
    let c = color.to_linear();
    let mut material = Material {
        ends: Vec4::ZERO,
        controls: Vec4::ZERO,
        tint: Vec4::new(c.red, c.green, c.blue, c.alpha),
        metrics: Vec4::new(0., 0., 1.7, 0.),
        motion: Vec4::new(1., 0., 0., 0.),
        a,
        b,
        zoom,
    };
    geometry(&mut layout, &mut material, a, b, zoom);
    let material = materials.add(material);
    commands
        .spawn((
            layout,
            MaterialNode(material),
            Pickable::IGNORE,
            ChildOf(parent),
        ))
        .id()
}

/// Keep the curve attached to animated sockets without recreating UI entities.
pub(super) fn geometry(node: &mut Node, material: &mut Material, a: Vec2, b: Vec2, zoom: f32) {
    let bend = ((b.x - a.x).abs() * 0.5).max(75. * zoom);
    let c1 = a + Vec2::X * bend;
    let c2 = b - Vec2::X * bend;
    let min = (a.min(b).min(c1).min(c2) - Vec2::splat(18.)).floor();
    let max = (a.max(b).max(c1).max(c2) + Vec2::splat(18.)).ceil();
    let size = max - min;
    let p = a - min;
    let q = b - min;
    let c1 = c1 - min;
    let c2 = c2 - min;
    material.ends = Vec4::new(p.x, p.y, q.x, q.y);
    material.controls = Vec4::new(c1.x, c1.y, c2.x, c2.y);
    material.metrics.x = size.x;
    material.metrics.y = size.y;
    material.a = a;
    material.b = b;
    material.zoom = zoom;
    if node.left != px(min.x) {
        node.left = px(min.x);
    }
    if node.top != px(min.y) {
        node.top = px(min.y);
    }
    if node.width != px(size.x) {
        node.width = px(size.x);
    }
    if node.height != px(size.y) {
        node.height = px(size.y);
    }
}
