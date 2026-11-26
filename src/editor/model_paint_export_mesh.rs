use super::{
    Document, ExportDirectory, ExportKind, FileOptions, Report, SetName, safe_stem, set_names,
};
use crate::editor::{model_document::Object, model_mesh::Geometry};
use std::{
    collections::{HashMap, HashSet},
    io::{self, Write},
    path::Path,
};

pub(super) struct Snapshot {
    objects: Vec<Object>,
    sets: HashMap<(usize, usize), SetName>,
}

impl Snapshot {
    pub(super) fn capture(doc: &Document) -> Self {
        Self {
            objects: doc.objects.clone(),
            sets: set_names(doc, None)
                .into_iter()
                .map(|(object, slot, name)| ((object.id, slot), name))
                .collect(),
        }
    }

    pub(super) fn write(self, parent: &Path, model: &str) -> io::Result<Report> {
        if self
            .objects
            .iter()
            .all(|object| object.geometry.positions.is_empty())
        {
            return Err(io::Error::other("У модели нет вершин для экспорта"));
        }
        let mut output = ExportDirectory::create(parent, model, "mesh", FileOptions::default())?;
        let stem = format!("{}_mesh", safe_stem(model));
        let material_file = format!("{stem}.mtl");
        let mut obj = output.file(&format!("{stem}.obj"))?;
        let mut mtl = output.file(&material_file)?;
        writeln!(obj, "# Bevy Editor\nmtllib {material_file}")?;
        writeln!(mtl, "# Bevy Editor")?;
        self.write_geometry(&mut obj, &mut mtl)?;
        obj.flush()?;
        mtl.flush()?;
        drop((obj, mtl));
        Ok(output.finish(ExportKind::Mesh))
    }

    fn write_geometry(&self, obj: &mut impl Write, mtl: &mut impl Write) -> io::Result<()> {
        let mut position = 1;
        let mut attribute = 1;
        let mut object_names = HashSet::new();
        for object in &self.objects {
            let mesh = &object.geometry;
            if mesh.positions.is_empty() {
                continue;
            }
            mesh.validate().map_err(io::Error::other)?;
            let root = safe_stem(&object.name);
            let mut name = root.clone();
            let mut suffix = 2;
            while !object_names.insert(name.to_lowercase()) {
                name = format!("{root}_{suffix}");
                suffix += 1;
            }
            let transform = object.transform.to_matrix();
            let normal_transform = transform.inverse().transpose();
            if !normal_transform.is_finite() {
                return Err(io::Error::other("Недопустимое преобразование объекта"));
            }
            let mirrored = transform.determinant() < 0.;
            writeln!(obj, "o {name}")?;
            for &vertex in &mesh.positions {
                let [x, y, z] = transform.transform_point3(vertex).to_array();
                writeln!(obj, "v {x} {y} {z}")?;
            }
            let smooth = mesh.smooth_normals();
            let mut seen_materials = HashSet::new();
            let mut current_material = None;
            let mut smoothing = None;
            let mut used_positions = vec![false; mesh.positions.len()];
            for face in &mesh.faces {
                if current_material != Some(face.material) {
                    let set = self
                        .sets
                        .get(&(object.id, face.material))
                        .ok_or_else(|| io::Error::other("У грани модели отсутствует материал"))?;
                    let name = match set.tile {
                        Some(tile) => format!("{}.{tile}", set.stem),
                        None => set.stem.clone(),
                    };
                    if seen_materials.insert(face.material) {
                        writeln!(mtl, "newmtl {name}\nKd 0.8 0.8 0.8\n")?;
                    }
                    writeln!(obj, "usemtl {name}")?;
                    current_material = Some(face.material);
                }
                if smoothing != Some(face.smooth) {
                    writeln!(obj, "s {}", if face.smooth { "1" } else { "off" })?;
                    smoothing = Some(face.smooth);
                }
                let flat = mesh.normal(face).normalize_or_zero();
                for corner in &face.corners {
                    used_positions[corner.vertex] = true;
                    // Geometry already contains global, texture-down UVs, including UDIM offsets.
                    writeln!(obj, "vt {} {}", corner.uv.x, 1. - corner.uv.y)?;
                    let local = Geometry::corner_normal(face, corner, flat, &smooth);
                    let [x, y, z] = normal_transform
                        .transform_vector3(local)
                        .normalize_or_zero()
                        .to_array();
                    writeln!(obj, "vn {x} {y} {z}")?;
                }
                for mut triangle in mesh.triangles(face) {
                    if mirrored {
                        triangle.swap(1, 2);
                    }
                    write!(obj, "f")?;
                    for index in triangle {
                        let vertex = position + face.corners[index].vertex;
                        let uv_normal = attribute + index;
                        write!(obj, " {vertex}/{uv_normal}/{uv_normal}")?;
                    }
                    writeln!(obj)?;
                }
                attribute += face.corners.len();
            }
            for edge in &mesh.loose_edges {
                used_positions[edge.0] = true;
                used_positions[edge.1] = true;
                writeln!(obj, "l {} {}", position + edge.0, position + edge.1)?;
            }
            for (index, used) in used_positions.into_iter().enumerate() {
                if !used {
                    writeln!(obj, "p {}", position + index)?;
                }
            }
            position += mesh.positions.len();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::{
        model_document::{MaterialSlot, glb::Glb},
        model_mesh::{Corner, Edge, Face},
    };
    use bevy::prelude::*;
    use std::fs;

    #[test]
    fn export_mesh_reopens_with_world_space_triangles_udim_uvs_and_named_materials() {
        let corners = [
            Vec2::new(2.25, -0.75),
            Vec2::new(2.75, -0.75),
            Vec2::new(2.25, -0.25),
        ]
        .into_iter()
        .enumerate()
        .map(|(vertex, uv)| {
            let mut corner = Corner::new(vertex);
            corner.uv = uv;
            corner.source_normal = Some(Vec3::Z);
            corner
        })
        .collect();
        let object = Object {
            id: 0,
            source_node: None,
            name: "Panel".into(),
            geometry: Geometry {
                positions: vec![Vec3::ZERO, Vec3::X, Vec3::Y, Vec3::Z, Vec3::ONE, -Vec3::X],
                faces: vec![Face {
                    corners,
                    material: 0,
                    smooth: true,
                }],
                loose_edges: [Edge::new(3, 4)].into_iter().collect(),
                ..Geometry::default()
            }
            .into(),
            materials: vec![MaterialSlot {
                bake_fixes: Default::default(),
                material: default(),
                source: serde_json::json!({"name": "Paint", "extras": {"bevy_editor_udim": 1013}}),
                paint: None,
                authored: None,
            }],
            transform: Transform::from_xyz(4., 2., -1.).with_scale(Vec3::new(-2., 3., 1.)),
            hidden: false,
        };
        let other = Object {
            id: 1,
            transform: Transform::IDENTITY,
            hidden: true,
            ..object.clone()
        };
        let root = std::env::temp_dir().join(format!(
            "bevy-export-obj-{}",
            bevy::asset::uuid::Uuid::new_v4()
        ));
        fs::create_dir(&root).unwrap();
        let source = root.join("source.glb");
        fs::write(
            &source,
            Glb::new_asset(std::slice::from_ref(&object)).unwrap(),
        )
        .unwrap();
        let mut doc = Document::new(Glb::load(&source).unwrap(), vec![object, other]);

        let snapshot = Snapshot::capture(&doc);
        doc.objects[0].transform.translation = Vec3::ZERO;
        let report = snapshot.write(&root, "Atlas").unwrap();
        let obj = report.path.join("Atlas_mesh.obj");
        let text = fs::read_to_string(&obj).unwrap();
        assert!(text.contains("mtllib Atlas_mesh.mtl"));
        assert!(text.contains("vt 2.25 1.75"));
        assert!(text.contains("o Panel_2"));
        assert!(text.contains("l 4 5"));
        assert!(text.contains("p 6"));
        let material = fs::read_to_string(report.path.join("Atlas_mesh.mtl")).unwrap();
        for name in ["Panel_Paint.1013", "Panel_Paint_2.1013"] {
            assert!(text.contains(&format!("usemtl {name}\n")));
            assert!(material.contains(&format!("newmtl {name}\n")));
        }
        let scene = crate::runtime::fbx::load_mesh_file(&obj, ufbx::FileFormat::Obj).unwrap();
        let mut triangles = 0;
        let mut mirrored = false;
        for mesh in &scene.meshes {
            for face in mesh.faces.iter().filter(|face| face.num_indices == 3) {
                triangles += 1;
                let i = face.index_begin as usize;
                let point = |n: usize| {
                    let v = mesh.vertex_position[n];
                    Vec3::new(v.x as f32, v.y as f32, v.z as f32)
                };
                let points = [point(i), point(i + 1), point(i + 2)];
                assert!((points[1] - points[0]).cross(points[2] - points[0]).z > 0.);
                mirrored |= points.contains(&Vec3::new(4., 2., -1.))
                    && points.contains(&Vec3::new(2., 2., -1.));
                let normal = mesh.vertex_normal[i];
                assert!(normal.z > 0.99);
                let uv = mesh.vertex_uv[i];
                assert!((2.24..=2.76).contains(&uv.x) && (1.24..=1.76).contains(&uv.y));
            }
        }
        assert_eq!(triangles, 2);
        assert!(mirrored);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn export_mesh_discards_incomplete_obj_and_mtl() {
        let root = std::env::temp_dir().join(format!(
            "bevy-export-obj-fail-{}",
            bevy::asset::uuid::Uuid::new_v4()
        ));
        fs::create_dir(&root).unwrap();
        let object = Object {
            id: 0,
            source_node: None,
            name: "Broken".into(),
            geometry: Geometry {
                positions: vec![Vec3::NAN],
                ..Geometry::default()
            }
            .into(),
            transform: Transform::IDENTITY,
            materials: vec![],
            hidden: false,
        };
        let snapshot = Snapshot {
            objects: vec![object],
            sets: HashMap::new(),
        };
        assert!(snapshot.write(&root, "Atlas").is_err());
        assert!(!root.join("Atlas_mesh").exists());
        fs::remove_dir(root).unwrap();
    }
}
