//! Transient viewport state for the material slots shown in the paint workspace.
use super::*;
use bevy::{
    asset::{AssetPath, embedded_asset, embedded_path},
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
use std::{collections::BTreeSet, sync::Arc};

pub(super) type SetKey = (usize, usize);

#[derive(Default, Resource)]
pub(in crate::editor::ui::model_viewer) struct ShelfPick {
    tree: Option<((usize, u64, u64), editing::input::Bvh)>,
    pub hover: Option<(usize, SetKey)>,
}

impl ShelfPick {
    pub(super) fn hit(
        &mut self,
        model: usize,
        doc: &crate::editor::model_document::Document,
        visibility: &SetVisibility,
        ray: (Vec3, Vec3),
    ) -> Option<SetKey> {
        let key = (model, doc.revision, visibility.revision);
        if self.tree.as_ref().is_none_or(|(current, _)| *current != key) {
            self.tree = Some((key, editing::input::Bvh::from_objects_where(
                &doc.objects, |object, slot| visibility.shown(object, slot),
            )));
        }
        let (object, face, _) = self.tree.as_ref()?.1.cast(ray.0, ray.1, None)?;
        let object = doc.objects.iter().find(|candidate| candidate.id == object)?;
        let slot = object.geometry.faces.get(face)?.material;
        object.materials.get(slot).map(|_| (object.id, slot))
    }
}

#[derive(Clone, Default)]
pub(in crate::editor::ui::model_viewer) struct SetVisibility {
    hidden: Arc<BTreeSet<SetKey>>,
    pub revision: u64,
}
impl SetVisibility {
    pub fn shown(&self, object: usize, slot: usize) -> bool {
        !self.hidden.contains(&(object, slot))
    }
    fn toggle(&mut self, key: SetKey) {
        toggle(Arc::make_mut(&mut self.hidden), key);
        self.revision += 1;
    }
}

#[derive(Default)]
pub(super) struct Appearance {
    pub visibility: SetVisibility,
    pub pinned: BTreeSet<SetKey>,
    flash: Option<(SetKey, f64)>,
    pub shown_count: usize,
}
impl Appearance {
    pub fn count(&mut self, doc: &crate::editor::model_document::Document) {
        let objects = &doc.objects;
        let count = objects
            .iter()
            .filter(|object| !object.hidden)
            .map(|o| o.materials.len())
            .sum::<usize>();
        let hidden = self
            .visibility
            .hidden
            .iter()
            .filter(|&&(id, slot)| {
                objects
                    .iter()
                    .any(|o| o.id == id && !o.hidden && slot < o.materials.len())
            })
            .count();
        self.shown_count = count - hidden;
    }

    pub fn can_toggle(&self, key: SetKey) -> bool {
        !self.visibility.shown(key.0, key.1) || self.shown_count > 1
    }

    pub fn toggle_visibility(&mut self, key: SetKey) {
        self.visibility.toggle(key);
    }

    pub fn toggle_pin(&mut self, key: SetKey) {
        toggle(&mut self.pinned, key);
    }

    pub fn flash(&mut self, key: SetKey, now: f64) {
        self.flash = Some((key, now));
    }

    fn flash_at(&self, now: f64) -> Option<(SetKey, f32)> {
        let (key, then) = self.flash?;
        let age = (now - then).max(0.);
        if age >= 1.6 {
            return None;
        }
        let pulse = 0.5 + 0.5 * (age * std::f64::consts::TAU * 1.5).cos();
        Some((key, (pulse * (1. - age / 1.6)) as f32))
    }
}

fn toggle(set: &mut BTreeSet<SetKey>, key: SetKey) {
    if !set.remove(&key) {
        set.insert(key);
    }
}

#[derive(Component)]
pub(in crate::editor::ui::model_viewer) struct SetPart(pub usize, pub usize);
#[derive(Component)]
pub(in crate::editor::ui::model_viewer) struct SetRow(pub SetKey);

#[derive(Component)]
pub(in crate::editor::ui::model_viewer) struct Highlight {
    entity: Entity,
    material: Handle<HighlightMaterial>,
}
#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub(in crate::editor::ui::model_viewer) struct HighlightMaterial {
    #[uniform(0)]
    color: Vec4,
}
impl Material for HighlightMaterial {
    fn fragment_shader() -> ShaderRef {
        AssetPath::from_path_buf(embedded_path!("model_paint_set_highlight.wesl"))
            .with_source("embedded")
            .into()
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
        _: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        descriptor.primitive.cull_mode = None;
        if let Some(depth) = &mut descriptor.depth_stencil {
            depth.depth_write_enabled = Some(false);
            depth.depth_compare = Some(CompareFunction::GreaterEqual);
            depth.bias.constant = 8;
        }
        Ok(())
    }
}

pub(in crate::editor::ui::model_viewer) struct Plugin;
impl bevy::prelude::Plugin for Plugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "model_paint_set_highlight.wesl");
        app.add_plugins(MaterialPlugin::<HighlightMaterial>::default());
    }
}

pub(in crate::editor::ui::model_viewer) fn refresh(
    mut commands: Commands,
    editor: Res<Editor>,
    editing: Res<editing::State>,
    paint: Res<PaintState>,
    mut shelf: ResMut<ShelfPick>,
    mouse: Res<ButtonInput<MouseButton>>,
    now: Res<Time<Real>>,
    rows: Query<(&SetRow, &crate::editor::ui::Hovered, &ComputedNode)>,
    mut parts: Query<(
        Entity,
        &SetPart,
        &Mesh3d,
        &RenderLayers,
        &mut Visibility,
        Option<&Highlight>,
    )>,
    mut overlays: Query<&mut Visibility, Without<SetPart>>,
    mut materials: ResMut<Assets<HighlightMaterial>>,
    mut redraw: MessageWriter<RequestRedraw>,
) {
    let session = editor
        .viewing_model
        .filter(|i| editing.mode(Some(*i)) == Mode::Paint)
        .and_then(|i| paint.sessions.get(&i));
    let appearance = session.map(|s| &s.set_appearance);
    if (shelf.hover.is_some() || shelf.tree.is_some())
        && (session.is_none() || !mouse.pressed(MouseButton::Left))
    {
        shelf.hover = None;
        shelf.tree = None;
    }
    let channel = session.and_then(Session::diagnostic_channel);
    let diagnostic = session.filter(|s| s.diagnostic_bake().is_some()).and_then(|s| s.binding);
    let hover = rows
        .iter()
        .find(|(_, hover, node)| hover.0 && node.size().min_element() > 0.)
        .map(|(row, _, _)| row.0);
    let hover = shelf.hover.filter(|(model, _)| Some(*model) == editor.viewing_model)
        .map(|(_, key)| key).or(hover);
    let flash = appearance.and_then(|a| a.flash_at(now.elapsed_secs_f64()));
    if flash.is_some() {
        redraw.write(RequestRedraw);
    }
    for (entity, part, mesh, layers, mut visibility, highlight) in &mut parts {
        let key = (part.0, part.1);
        let shown = appearance.is_none_or(|a| a.visibility.shown(key.0, key.1));
        visibility.set_if_neq(if shown {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        });
        let strength = appearance.filter(|_| shown && channel.is_none() && diagnostic != Some(key)).map_or(0., |a| {
            let pin: f32 = if a.pinned.contains(&key) { 0.30 } else { 0. };
            let hover: f32 = if hover == Some(key) { 0.20 } else { 0. };
            let flash = flash
                .filter(|(target, _)| *target == key)
                .map_or(0., |(_, power)| power * 0.36);
            pin.max(hover).max(flash)
        });
        let color = Vec4::new(0.10, 0.65, 1.65, strength);
        if let Some(highlight) = highlight {
            if let Ok(mut visible) = overlays.get_mut(highlight.entity) {
                visible.set_if_neq(if strength > 0. {
                    Visibility::Inherited
                } else {
                    Visibility::Hidden
                });
            }
            if strength > 0.
                && materials
                    .get(&highlight.material)
                    .is_some_and(|material| material.color != color)
            {
                materials.get_mut(&highlight.material).unwrap().color = color;
            }
        } else if strength > 0. {
            let material = materials.add(HighlightMaterial { color });
            let overlay = commands
                .spawn((
                    mesh.clone(),
                    MeshMaterial3d(material.clone()),
                    layers.clone(),
                    NotShadowCaster,
                    NotShadowReceiver,
                    ChildOf(entity),
                ))
                .id();
            commands.entity(entity).try_insert(Highlight {
                entity: overlay,
                material,
            });
        }
    }
}
