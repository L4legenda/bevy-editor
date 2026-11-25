//! Static OBJ and FBX geometry for projection, in the same coordinates as Bevy's FBX loader.
use super::{Progress, SourceTriangle, SourceUse};
use super::super::triangle_id_color;
use crate::runtime::fbx::{geometry_matrix, load_mesh_file};
use bevy::prelude::*;
use std::path::Path;

pub(super) fn mesh_triangles(
    path: &Path,
    format: ufbx::FileFormat,
    usage: impl Fn(&str) -> SourceUse,
    progress: &Progress,
) -> Result<Vec<SourceTriangle>, String> {
    let scene = load_mesh_file(path, format).map_err(|error| error.to_string())?;
    let mut output = Vec::new();
    let mut indices = Vec::new();
    let xyz = |v: ufbx::Vec3| Vec3::new(v.x as f32, v.y as f32, v.z as f32);
    for node in &scene.nodes {
        progress.check()?;
        let Some(mesh) = &node.mesh else { continue };
        let mut named_node = node;
        while (named_node.is_geometry_transform_helper || named_node.is_scale_helper)
            && let Some(parent) = &named_node.parent
        {
            named_node = parent;
        }
        let name = &*named_node.element.name;
        let name = if name.is_empty() {
            &*mesh.element.name
        } else {
            name
        };
        let usage = usage(name);
        if usage == SourceUse::Omit {
            continue;
        }
        let transform = geometry_matrix(node);
        let matrix = Mat3::from_mat4(transform);
        let determinant = matrix.determinant();
        if !transform.is_finite() || !determinant.is_finite() || determinant == 0. {
            continue;
        }
        let normal_matrix = matrix.inverse().transpose();
        for (i, &face) in mesh.faces.iter().enumerate() {
            if i % 4096 == 0 {
                progress.check()?;
            }
            if face.num_indices < 3 {
                continue;
            }
            let slot = mesh.face_material.get(i).copied().unwrap_or_default() as usize;
            let material = node.materials.get(slot).or_else(|| mesh.materials.get(slot));
            let material = material.map(|material| material.element.typed_id);
            let count = ufbx::triangulate_face_vec(&mut indices, mesh, face) as usize * 3;
            for triangle in indices[..count].chunks_exact(3) {
                let mut corners = [triangle[0], triangle[1], triangle[2]].map(|i| i as usize);
                if determinant < 0. {
                    corners.swap(1, 2);
                }
                let points =
                    corners.map(|i| transform.transform_point3(xyz(mesh.vertex_position[i])));
                let normals = if mesh.vertex_normal.exists {
                    corners.map(|i| normal_matrix * xyz(mesh.vertex_normal[i]))
                } else {
                    [Vec3::ZERO; 3]
                };
                let color = triangle_id_color(corners.map(|i| {
                    mesh.vertex_color.indices.get(i)
                        .and_then(|&index| mesh.vertex_color.values.get(index as usize))
                        .map(|color| Vec3::new(color.x as f32, color.y as f32, color.z as f32))
                }));
                output.extend(
                    SourceTriangle::new(points, normals, named_node.element.typed_id as usize)
                        .map(|triangle| triangle.with_usage(usage).with_id(material, color)),
                );
            }
        }
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::super::{Projection, ProjectionSettings};
    use super::*;

    #[test]
    fn baked_fbx_keeps_object_name_and_converts_centimeters() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fbx/panel_ascii.fbx");
        let progress = Progress::default();
        let projection = Projection::load(
            &path,
            "Panel_low",
            1.,
            &ProjectionSettings::default(),
            &progress,
        )
        .unwrap();
        assert_eq!(projection.source.triangles.len(), 2);
        let hit = projection
            .sample(Vec3::new(0.75, 1., -0.025), Vec3::Z, Vec3::Z, None)
            .unwrap();
        assert!(hit.ray_hit && (hit.height - 0.5).abs() < 1e-5);
        assert!(
            projection
                .sample(Vec3::new(0.75, 2., 0.), Vec3::Z, Vec3::Z, None)
                .is_none()
        );
        assert!(
            Projection::load(
                &path,
                "Other_low",
                1.,
                &ProjectionSettings::default(),
                &progress
            )
            .is_err()
        );
    }

    #[test]
    fn baked_obj_keeps_named_groups_and_projects_quads() {
        let path = std::env::temp_dir().join(format!(
            "bevy-paint-source-{}.OBJ",
            bevy::asset::uuid::Uuid::new_v4()
        ));
        std::fs::write(
            &path,
            concat!(
                "o Panel_high.002\n",
                "v 0 0 0.03\nv 1 0 0.03\nv 1 1 0.03\nv 0 1 0.03\nf 1 2 3 4\n",
                "o Neighbor_high\n",
                "v 0 0 0.04\nv 1 0 0.04\nv 1 1 0.04\nv 0 1 0.04\nf 5 6 7 8\n",
            ),
        )
        .unwrap();
        let load = |match_names| {
            Projection::load(
                &path,
                "Panel_low",
                1.,
                &ProjectionSettings {
                    match_names,
                    ..default()
                },
                &Progress::default(),
            )
        };
        let named = load(true);
        let all = load(false);
        std::fs::remove_file(path).unwrap();
        for (projection, faces, expected_height) in
            [(named.unwrap(), 2, 0.6), (all.unwrap(), 4, 0.8)]
        {
            assert_eq!(projection.source.triangles.len(), 4);
            assert_eq!(
                projection
                    .source
                    .triangles
                    .iter()
                    .filter(|triangle| triangle.usage == SourceUse::Project)
                    .count(),
                faces
            );
            let hit = projection
                .sample(Vec3::new(0.4, 0.6, 0.), Vec3::Z, Vec3::Z, None)
                .unwrap();
            assert!(hit.ray_hit && (hit.height - expected_height).abs() < 1e-5);
            assert!(hit.normal.abs_diff_eq(Vec3::Z, 1e-5));
        }
    }

    #[test]
    fn baked_high_source_reuses_geometry_and_relief_until_file_changes() {
        use std::sync::Arc;

        let path = std::env::temp_dir().join(format!(
            "bevy-bake-source-cache-{}.obj",
            bevy::asset::uuid::Uuid::new_v4()
        ));
        let mesh = |height| {
            format!("o Panel_high\nv 0 0 {height}\nv 1 0 {height}\nv 0 1 {height}\nf 1 2 3\n")
        };
        std::fs::write(&path, mesh("0.02")).unwrap();
        let settings = ProjectionSettings::default();
        let progress = Progress::default();
        let first = Projection::load(&path, "Panel_low", 1., &settings, &progress).unwrap();
        let edited = ProjectionSettings {
            front: 10,
            ..settings.clone()
        };
        let second = Projection::load(&path, "PANEL-low.001", 1., &edited, &progress).unwrap();
        assert!(Arc::ptr_eq(&first.source, &second.source));
        assert_ne!(first.front, second.front);
        let relief = first.build_relief(1., true, &progress).unwrap();
        assert!(Arc::ptr_eq(
            &relief,
            &second.build_relief(1., true, &progress).unwrap()
        ));
        assert!(Arc::ptr_eq(
            &relief,
            &second.build_relief(0.5, false, &progress).unwrap()
        ));
        assert!(!Arc::ptr_eq(
            &relief,
            &second.build_relief(0.5, true, &progress).unwrap()
        ));
        assert!(Projection::load(&path, "Other_low", 1., &settings, &progress).is_err());

        std::fs::write(&path, mesh("0.04")).unwrap();
        let changed = Projection::load(&path, "Panel_low", 1., &settings, &progress).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert!(!Arc::ptr_eq(&first.source, &changed.source));
        assert_eq!(changed.source.triangles[0].points[0].z, 0.04);
    }
}
