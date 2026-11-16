use super::*;
use crate::editor::area_selection;
use bevy::camera::primitives::Aabb;

pub(super) struct SceneArea {
    pub start: Vec2,
    pub end: Vec2,
    pub hit: Option<Selection>,
    pub room: super::super::Room,
}
pub(super) fn targets(
    editor: &Editor,
    scene: &Scene,
    camera: &Camera,
    view: &GlobalTransform,
    area: Rect,
    children: &Query<&Children>,
    transforms: &Query<&GlobalTransform>,
    meshes: &Query<
        (
            &Aabb,
            &GlobalTransform,
            &InheritedVisibility,
            Option<&RenderLayers>,
        ),
        With<Mesh3d>,
    >,
) -> Vec<Selection> {
    let project = |p| {
        camera
            .world_to_viewport(view, p)
            .ok()
            .map(|p| p + Vec2::Y * ui::TOP)
    };
    let roots = editor
        .imported
        .iter()
        .enumerate()
        .map(|(i, n)| (Selection::Imported(i), n.entity))
        .chain(editor.edits.models.iter().filter_map(|m| {
            scene
                .models
                .get(&m.id)
                .map(|(root, _)| (Selection::Model(m.id), *root))
        }));
    let mut hits = Vec::new();
    for (target, root) in roots {
        if editor.object_hidden(target) {
            continue;
        }
        let Some((min, max, world)) = crate::editor::selection_outline::bounds(
            root,
            &RenderLayers::layer(0),
            children,
            transforms,
            meshes,
        ) else {
            continue;
        };
        let points = (0..8).map(|i| {
            world.transform_point3(Vec3::new(
                if i & 1 == 0 { min.x } else { max.x },
                if i & 2 == 0 { min.y } else { max.y },
                if i & 4 == 0 { min.z } else { max.z },
            ))
        });
        if area_selection::projected_bounds(points, project)
            .is_some_and(|bounds| area_selection::overlaps(area, bounds))
        {
            hits.push(target);
        }
    }
    for shot in shots(editor) {
        let p = editor.camera(shot).pose.transform().translation;
        if p.distance_squared(editor.view.translation) > 0.04
            && project(p).is_some_and(|p| area.contains(p))
        {
            hits.push(Selection::Camera(shot));
        }
    }
    if editor.show_lights {
        for light in &editor.edits.lights {
            let target = Selection::Light(light.id);
            if !editor.object_hidden(target)
                && project(light.pose.transform().translation).is_some_and(|p| area.contains(p))
            {
                hits.push(target);
            }
        }
    }
    hits
}
