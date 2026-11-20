use super::*;
use crate::editor::{
    model_document::{Document, MaterialSlot, Operation},
    model_mesh::{Corner, Face, Geometry},
};

fn fixture() -> (Glb, Object) {
    let folder = std::env::temp_dir().join(format!(
        "bevy-model-glb-{}",
        bevy::asset::uuid::Uuid::new_v4()
    ));
    fs::create_dir_all(&folder).unwrap();
    let root = json!({"asset":{"version":"2.0","extras":{"preserve":"yes"}},"scene":0,"scenes":[{"nodes":[0]}],"nodes":[{"name":"original","extras":{"script":42}}],"meshes":[],"materials":[{"name":"gold","pbrMetallicRoughness":{"baseColorFactor":[1.0,0.6,0.1,1.0]}}],"buffers":[{"byteLength":0}]});
    let mut encoded = serde_json::to_vec(&root).unwrap();
    align(&mut encoded, b' ');
    let mut bytes = Vec::new();
    for v in [0x46546c67, 2, 20 + encoded.len() as u32] {
        bytes.extend_from_slice(&v.to_le_bytes());
    }
    chunk(&mut bytes, 0x4e4f534a, &encoded);
    let path = folder.join("model.glb");
    fs::write(&path, &bytes).unwrap();
    let source = Glb::parse(path, bytes).unwrap();
    let geometry = Geometry {
        loose_edges: default(),
        uv_seams: default(),
        positions: vec![Vec3::ZERO, Vec3::X, Vec3::Y],
        faces: vec![Face {
            corners: (0..3).map(Corner::new).collect(),
            material: 0,
            smooth: false,
        }],
    };
    let object = Object {
        id: 0,
        source_node: Some(0),
        name: "edited".into(),
        geometry: geometry.into(),
        transform: Transform::from_xyz(1., 2., 3.),
        materials: vec![MaterialSlot {
            authored: None,
            paint: None,
            material: default(),
            source: json!({"material":0,"extras":{"keep":true}}),
        }],
        hidden: false,
    };
    (source, object)
}

#[test]
fn model_glb_assigns_translucent_material_to_only_selected_faces_and_keeps_it_on_save() {
    use crate::editor::{
        model_document::{MaterialDraft, Mode},
        model_mesh::Element,
    };
    let (source, mut object) = fixture();
    object.geometry.positions.push(Vec3::X + Vec3::Y);
    object.geometry.faces.push(Face {
        corners: [1, 3, 2].into_iter().map(Corner::new).collect(),
        material: 0,
        smooth: false,
    });
    let mut doc = Document::new(source, vec![object]);
    doc.mode = Mode::Edit;
    doc.selection.kind = Element::Face;
    doc.selection.faces.insert(1);
    let draft = MaterialDraft {
        name: "Стекло".into(),
        color: bevy::color::Srgba::hex("#9CCFE080").unwrap(),
        roughness: 0.2,
        metallic: 0.0,
    };
    let mut assets = Assets::<StandardMaterial>::default();
    let glass = draft.slot(&mut assets);
    assert_eq!(assets.get(&glass.material).unwrap().alpha_mode, AlphaMode::Blend);
    doc.assign_material(glass, &Handle::default()).unwrap();
    assert_eq!(doc.objects[0].geometry.faces[0].material, 0);
    assert_eq!(doc.objects[0].geometry.faces[1].material, 1);
    doc.undo();
    assert_eq!(doc.objects[0].geometry.faces[1].material, 0);
    doc.redo();
    assert_eq!(doc.objects[0].geometry.faces[1].material, 1);
    doc.save().unwrap();
    doc.save().unwrap();
    let loaded = Glb::load(&doc.source.path).unwrap();
    let asset = bevy::gltf::gltf::Gltf::from_slice(loaded.data()).unwrap();
    assert_eq!(asset.materials().count(), 2);
    let indices: Vec<_> = asset.nodes().next().unwrap().mesh().unwrap().primitives()
        .map(|primitive| primitive.material().index()).collect();
    assert_eq!(indices, [Some(0), Some(1)]);
    let glass = asset.materials().nth(1).unwrap();
    assert_eq!(glass.name(), Some("Стекло"));
    assert_eq!(glass.alpha_mode(), bevy::gltf::gltf::material::AlphaMode::Blend);
    assert!((glass.pbr_metallic_roughness().base_color_factor()[3] - 128.0 / 255.0).abs() < 0.00001);
}

#[test]
fn model_glb_paint_round_trip_keeps_layers_and_flattened_standard_material() {
    use crate::editor::model_paint_data::{Surface, Stack, Layer, LayerKind, solid};
    use std::sync::Arc;
    let (mut source, mut object) = fixture();
    let mut red = Layer::new("Красный".into(),LayerKind::Paint);
    red.channels[0]=Some(solid(16,[255,0,0,255]));
    let mut blue=Layer::new("Синий".into(),LayerKind::Paint);
    blue.channels[0]=Some(solid(16,[0,0,255,255]));blue.opacity=50.;
    let surface=Arc::new(Surface{size:16,base:std::array::from_fn(|_|solid(16,[255;4])),stack:Stack{layers:vec![red,blue],selected:1},material:source.json["materials"][0].clone()});
    object.materials[0].paint=Some(surface.clone());
    source.save(&[object],&[]).unwrap();
    let loaded=Glb::load(&source.path).unwrap();
    let mesh=loaded.json["nodes"][0]["mesh"].as_u64().unwrap()as usize;
    let primitive=&loaded.json["meshes"][mesh]["primitives"][0];
    let restored=loaded.paint_surface(primitive).unwrap().unwrap();
    assert_eq!(*restored,*surface);
    assert_eq!(&restored.composite(0)[..4],&[128,0,128,255]);
    let mat=primitive["material"].as_u64().unwrap()as usize;
    let tex=loaded.json["materials"][mat]["pbrMetallicRoughness"]["baseColorTexture"]["index"].as_u64().unwrap()as usize;
    let image=loaded.json["textures"][tex]["source"].as_u64().unwrap()as usize;
    let view=loaded.json["images"][image]["bufferView"].as_u64().unwrap()as usize;
    let start=loaded.json["bufferViews"][view]["byteOffset"].as_u64().unwrap()as usize;
    let len=loaded.json["bufferViews"][view]["byteLength"].as_u64().unwrap()as usize;
    let png=image::load_from_memory(&loaded.bin[start..start+len]).unwrap().to_rgba8();
    assert_eq!(png.get_pixel(4,4).0,[128,0,128,255]);
    assert!(bevy::gltf::gltf::Gltf::from_slice(loaded.data()).is_ok());
    fs::remove_dir_all(source.path.parent().unwrap()).unwrap();
}
#[test]
fn model_glb_round_trip_keeps_extruded_edge() {
    use crate::editor::{
        model_document::from_mesh,
        model_mesh::{Element, Selection},
    };
    use bevy::{asset::RenderAssetUsages, mesh::PrimitiveTopology};

    let (mut source, mut object) = fixture();
    let mut selection = Selection { vertices: [0].into(), ..default() };
    object.geometry.extrude(&mut selection, 0.5).unwrap();
    let line = *object.geometry.loose_edges.first().unwrap();
    assert_eq!(selection.vertices, [line.1].into());
    source.save(&[object.clone()], &[]).unwrap();

    let parsed = bevy::gltf::gltf::Gltf::from_slice(source.data()).unwrap();
    let mesh = parsed.meshes().next().unwrap();
    let mut restored = (*object.geometry).clone();
    restored.loose_edges.clear();
    restored.positions.pop();
    let primitive = mesh.primitives().find(|p| p.mode() == bevy::gltf::gltf::mesh::Mode::Lines).unwrap();
    let reader = primitive.reader(|_| parsed.blob.as_deref());
    let mut lines = Mesh::new(PrimitiveTopology::LineList, RenderAssetUsages::MAIN_WORLD);
    lines.insert_attribute(Mesh::ATTRIBUTE_POSITION, reader.read_positions().unwrap().collect::<Vec<_>>());
    from_mesh(&lines, &mut restored, 0).unwrap();
    assert_eq!(restored, *object.geometry);

    selection.convert(Element::Edge, &restored);
    selection.edges.insert(line);
    restored.extrude(&mut selection, 0.25).unwrap();
    restored.validate().unwrap();
    assert!(restored.loose_edges.is_empty());
    assert_eq!(restored.faces.len(), 2);
    assert_eq!(restored.faces[1].corners.len(), 4);
    assert_eq!(selection.edges.len(), 1);
    fs::remove_dir_all(source.path.parent().unwrap()).unwrap();
}
#[test]
fn model_glb_round_trip_preserves_materials_extras_and_real_mesh_data() {
    let (mut source, object) = fixture();
    let materials = source.json["materials"].clone();
    source.save(&[object.clone()], &[]).unwrap();
    let bytes = fs::read(&source.path).unwrap();
    let parsed = bevy::gltf::gltf::Gltf::from_slice(&bytes).unwrap();
    assert_eq!(parsed.nodes().next().unwrap().name(), Some("edited"));
    let primitive = parsed.meshes().next().unwrap().primitives().next().unwrap();
    let blob = parsed.blob.as_ref().unwrap();
    let reader = primitive.reader(|buffer| (buffer.index() == 0).then_some(blob.as_slice()));
    assert_eq!(
        reader.read_positions().unwrap().collect::<Vec<_>>(),
        vec![[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]]
    );
    assert_eq!(
        reader
            .read_indices()
            .unwrap()
            .into_u32()
            .collect::<Vec<_>>(),
        [0, 1, 2]
    );
    let saved = Glb::load(&source.path).unwrap();
    assert_eq!(saved.json["materials"], materials);
    assert_eq!(saved.json["nodes"][0]["extras"]["script"], 42);
    assert_eq!(
        saved.json["meshes"][0]["primitives"][0]["extras"]["keep"],
        true
    );
    source.save(&[object], &[]).unwrap();
    assert_eq!(fs::read(&source.path).unwrap(), bytes);
    fs::remove_dir_all(source.path.parent().unwrap()).unwrap();
}
#[test]
fn model_glb_refuses_external_changes_and_model_undo_tracks_saved_state() {
    let (source, object) = fixture();
    let path = source.path.clone();
    let mut doc = Document::new(source, vec![object]);
    doc.selected = [0].into();
    doc.operate(Operation::Duplicate).unwrap();
    assert_eq!(doc.objects.len(), 2);
    assert!(doc.dirty());
    doc.undo();
    assert!(!doc.dirty());
    doc.redo();
    assert!(doc.dirty());
    fs::write(&path, b"externally changed").unwrap();
    assert!(doc.save().unwrap_err().contains("другой программой"));
    assert_eq!(fs::read(&path).unwrap(), b"externally changed");
    assert!(doc.dirty());
    fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

#[test]
fn model_document_mesh_edits_do_not_change_duplicate_sources_and_failed_operations_rollback() {
    let (source, object) = fixture();
    let folder = source.path.parent().unwrap().to_path_buf();
    let original = object.clone();
    let mut doc = Document::new(source, vec![object]);
    doc.operate(Operation::Duplicate).unwrap();
    assert!(doc.operate(Operation::DeleteFaces).is_err());
    doc.set_mode(super::super::Mode::Edit);
    let mesh = doc.active().unwrap().geometry.clone();
    doc.selection.all(&mesh);
    let before = doc.snapshot();
    doc.transform(&before, Mat4::from_translation(Vec3::X), None);
    doc.checkpoint(before);
    assert_eq!(doc.objects[0], original);
    assert_ne!(doc.objects[1].geometry, original.geometry);
    let before = doc.objects.clone();
    let history = doc.undo.len();
    assert!(doc.operate(Operation::LoopCut).is_err());
    assert_eq!(doc.objects, before);
    assert_eq!(doc.undo.len(), history);
    doc.undo();
    assert_eq!(doc.objects[1].geometry, original.geometry);
    doc.undo();
    assert!(!doc.dirty());
    doc.redo();
    doc.redo();
    assert_eq!(doc.objects, before);
    doc.save().unwrap();
    assert!(!doc.dirty());
    doc.undo();
    assert!(doc.dirty());
    fs::remove_dir_all(folder).unwrap();
}

#[test]
fn model_document_world_scale_preserves_rotated_shape_without_losing_shear() {
    let (source, mut object) = fixture();
    let folder = source.path.parent().unwrap().to_path_buf();
    object.transform.rotation = Quat::from_euler(EulerRot::XYZ, 0.3, 0.7, -0.4);
    let mut doc = Document::new(source, vec![object]);
    let base = doc.snapshot();
    let matrix = Mat4::from_scale(Vec3::new(2.0, 0.6, 1.2));
    doc.transform(&base, matrix, None);
    doc.validate().unwrap();
    let object = doc.active().unwrap();
    for (old, new) in base.objects[0]
        .geometry
        .positions
        .iter()
        .zip(&object.geometry.positions)
    {
        let expected = matrix.transform_point3(base.objects[0].transform.transform_point(*old));
        assert!(expected.distance(object.transform.transform_point(*new)) < 0.0001);
    }
    doc.checkpoint(base.clone());
    doc.undo();
    assert_eq!(doc.objects, base.objects);
    fs::remove_dir_all(folder).unwrap();
}

#[test]
fn model_document_imports_cube_and_plane_as_quads_and_retains_single_triangles() {
    use crate::editor::model_document::from_mesh;
    for (mesh, vertices, faces, edges) in [
        (Mesh::from(Cuboid::from_size(Vec3::ONE)), 8, 6, 12),
        (
            Mesh::from(Plane3d::default().mesh().size(1.0, 1.0)),
            4,
            1,
            4,
        ),
    ] {
        let mut geometry = Geometry::default();
        from_mesh(&mesh, &mut geometry, 0).unwrap();
        geometry.validate().unwrap();
        assert_eq!(geometry.positions.len(), vertices);
        assert_eq!(geometry.faces.len(), faces);
        assert_eq!(geometry.edges().len(), edges);
        assert!(geometry.faces.iter().all(|f| f.corners.len() == 4));
        assert_eq!(geometry.buffers()[&0].indices.len(), faces * 6);
    }
    let mut triangle = Mesh::new(
        bevy::mesh::PrimitiveTopology::TriangleList,
        bevy::asset::RenderAssetUsages::MAIN_WORLD,
    );
    triangle.insert_attribute(
        Mesh::ATTRIBUTE_POSITION,
        vec![[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]],
    );
    let mut geometry = Geometry::default();
    from_mesh(&triangle, &mut geometry, 0).unwrap();
    assert_eq!(geometry.faces.len(), 1);
    assert_eq!(geometry.faces[0].corners.len(), 3);
}

#[test]
fn model_glb_saved_cube_reopens_with_six_editable_quads() {
    use crate::editor::model_document::from_mesh;
    let (mut source, mut object) = fixture();
    let mut geometry = Geometry::default();
    from_mesh(&Mesh::from(Cuboid::from_size(Vec3::ONE)), &mut geometry, 0).unwrap();
    object.geometry = geometry.into();
    let mut doc = Document::new(source.clone(), vec![object.clone()]);
    assert!(!doc.dirty());
    doc.set_mode(crate::editor::model_document::Mode::Edit);
    assert_eq!(doc.selection.vertices.len(), 8);
    assert!(!doc.dirty());
    source.save(&[object], &[]).unwrap();
    let parsed = bevy::gltf::gltf::Gltf::from_slice(source.data()).unwrap();
    let blob = parsed.blob.as_ref().unwrap();
    let primitive = parsed.meshes().next().unwrap().primitives().next().unwrap();
    let reader = primitive.reader(|buffer| (buffer.index() == 0).then_some(blob.as_slice()));
    let mut mesh = Mesh::new(
        bevy::mesh::PrimitiveTopology::TriangleList,
        bevy::asset::RenderAssetUsages::MAIN_WORLD,
    );
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_POSITION,
        reader.read_positions().unwrap().collect::<Vec<_>>(),
    );
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_NORMAL,
        reader.read_normals().unwrap().collect::<Vec<_>>(),
    );
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_UV_0,
        reader
            .read_tex_coords(0)
            .unwrap()
            .into_f32()
            .collect::<Vec<_>>(),
    );
    mesh.insert_indices(bevy::mesh::Indices::U32(
        reader.read_indices().unwrap().into_u32().collect(),
    ));
    let mut reopened = Geometry::default();
    from_mesh(&mesh, &mut reopened, 0).unwrap();
    reopened.validate().unwrap();
    assert_eq!(reopened.faces.len(), 6);
    assert!(reopened.faces.iter().all(|f| f.corners.len() == 4));
    assert_eq!(reopened.edges().len(), 12);
    fs::remove_dir_all(source.path.parent().unwrap()).unwrap();
}

fn import_geometry(source: &Glb) -> Result<Geometry, String> {
    use crate::editor::model_document::from_mesh_with_topology;
    let parsed = bevy::gltf::gltf::Gltf::from_slice(source.data()).unwrap();
    let mesh = parsed.nodes().find_map(|node| node.mesh()).unwrap();
    let blob = parsed.blob.as_ref().unwrap();
    let topology = &source.json["meshes"][mesh.index()]["extras"]["bevy_editor_topology"];
    let mut geometry = Geometry::default();
    for primitive in mesh.primitives() {
        let reader = primitive.reader(|buffer| (buffer.index() == 0).then_some(blob.as_slice()));
        let mut mesh = Mesh::new(
            bevy::mesh::PrimitiveTopology::TriangleList,
            bevy::asset::RenderAssetUsages::MAIN_WORLD,
        );
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_POSITION,
            reader.read_positions().unwrap().collect::<Vec<_>>(),
        );
        if let Some(normals) = reader.read_normals() {
            mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals.collect::<Vec<_>>());
        }
        for (channel, attribute) in [(0, Mesh::ATTRIBUTE_UV_0), (1, Mesh::ATTRIBUTE_UV_1)] {
            if let Some(uv) = reader.read_tex_coords(channel) {
                mesh.insert_attribute(attribute, uv.into_f32().collect::<Vec<_>>());
            }
        }
        if let Some(colors) = reader.read_colors(0) {
            mesh.insert_attribute(
                Mesh::ATTRIBUTE_COLOR,
                colors.into_rgba_f32().collect::<Vec<_>>(),
            );
        }
        if let Some(indices) = reader.read_indices() {
            mesh.insert_indices(bevy::mesh::Indices::U32(indices.into_u32().collect()));
        }
        from_mesh_with_topology(
            &mesh,
            &mut geometry,
            primitive.index(),
            (topology["version"].as_u64() == Some(1))
                .then(|| &topology["primitives"][primitive.index()]),
        )?;
    }
    if topology["version"].as_u64() == Some(1) {
        geometry.restore_uv_seams(&topology["uv_seams"])?;
    }
    geometry.validate()?;
    Ok(geometry)
}

#[test]
fn model_glb_manual_uv_seams_survive_loop_cut_undo_and_reopen() {
    use crate::editor::{model_document::Mode, model_mesh::{Edge, Element}};

    let (source, mut object) = fixture();
    object.geometry = Geometry::default().into();
    crate::editor::model_document::from_mesh(
        &Mesh::from(Cuboid::from_size(Vec3::ONE)), &mut object.geometry, 0,
    ).unwrap();
    let mut doc = Document::new(source, vec![object]);
    doc.mode = Mode::Edit;
    doc.selection.kind = Element::Edge;
    let marked = *doc.active().unwrap().geometry.edges().keys().next().unwrap();
    doc.selection.edges.insert(marked);
    doc.operate(Operation::MarkUvSeam).unwrap();
    assert_eq!(doc.active().unwrap().geometry.uv_seams, [marked].into());
    doc.operate(Operation::ClearUvSeam).unwrap();
    assert!(doc.active().unwrap().geometry.uv_seams.is_empty());
    doc.undo();
    doc.operate(Operation::LoopCut).unwrap();
    let mesh = &doc.active().unwrap().geometry;
    assert_eq!(mesh.uv_seams.len(), 2);
    let expected = mesh.uv_seam_positions();
    doc.save().unwrap();
    let saved = Glb::load(&doc.source.path).unwrap();
    let imported = import_geometry(&saved).unwrap();
    assert_eq!(imported.uv_seams.len(), 2);
    for [a, b] in expected {
        let edge = Edge::new(
            imported.positions.iter().position(|p| p.to_array() == a).unwrap(),
            imported.positions.iter().position(|p| p.to_array() == b).unwrap(),
        );
        assert!(imported.uv_seams.contains(&edge));
    }
}

#[test]
fn model_glb_round_trips_knife_and_multiple_loop_cuts() {
    use crate::editor::model_mesh::{Edge, KnifeSegment, LoopCut, LoopCutOptions};
    let (mut source, mut object) = fixture();
    object.geometry = Geometry {
        positions: vec![Vec3::ZERO, Vec3::X, Vec3::X + Vec3::Y, Vec3::Y],
        faces: vec![Face { corners: (0..4).map(Corner::new).collect(), material: 0, smooth: false }],
        uv_seams: [Edge::new(0,1)].into(),
        ..default()
    }.into();
    let geometry = &mut object.geometry;
    let new_loops = LoopCut::find(geometry, Edge::new(0,1)).unwrap()
        .apply(geometry, LoopCutOptions { count: 3, ..default() }).unwrap();
    let new_knife = geometry.apply_knife(&[KnifeSegment {
        face: 0, ends: [Vec3::new(0.0,0.5,0.0), Vec3::new(0.25,0.5,0.0)],
    }]).unwrap();
    let expected = geometry.edges().keys().map(|e| [geometry.positions[e.0],geometry.positions[e.1]])
        .collect::<Vec<_>>();
    let face_count = geometry.faces.len();
    assert_eq!(new_loops.len(), 3);
    assert_eq!(new_knife.len(), 1);
    source.save(&[object], &[]).unwrap();
    let imported = import_geometry(&Glb::load(&source.path).unwrap()).unwrap();
    assert_eq!(imported.faces.len(), face_count);
    assert_eq!(imported.uv_seams.len(), 4);
    let adjacency = imported.edges();
    for [a,b] in expected {
        let point = |p: Vec3| imported.positions.iter().position(|&at| at==p).unwrap();
        assert!(adjacency.contains_key(&Edge::new(point(a),point(b))));
    }
}

#[test]
fn model_glb_reconstructs_quads_from_the_previously_edited_cube_regression() {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/model-edit/warped-cube.glb");
    let source = Glb::load(&path).unwrap();
    let geometry = import_geometry(&source).unwrap();
    assert_eq!(geometry.positions.len(), 8);
    assert_eq!(
        geometry.faces.len(),
        6,
        "Previously edited non-planar sides must remain whole faces"
    );
    assert!(geometry.faces.iter().all(|f| f.corners.len() == 4));
    assert_eq!(
        geometry.edges().len(),
        12,
        "No visible or selectable triangulation diagonals"
    );
    assert!(
        geometry.positions.iter().any(|p| p.y > 1.2),
        "Preserve the actual deformed model"
    );
}

#[test]
fn model_glb_polygon_topology_survives_warping_and_explicit_triangulation() {
    use crate::editor::{
        model_document::from_mesh,
        model_mesh::{Element, Selection},
    };
    let (source, mut object) = fixture();
    for triangulate in [false, true] {
        let mut geometry = Geometry::default();
        from_mesh(&Mesh::from(Cuboid::from_size(Vec3::ONE)), &mut geometry, 0).unwrap();
        geometry.positions[0] += Vec3::new(0.2, 0.45, 0.3);
        if triangulate {
            let mut selection = Selection {
                kind: Element::Face,
                ..default()
            };
            selection.all(&geometry);
            geometry.triangulate(&mut selection);
        }
        geometry.validate().unwrap();
        let mut expected: Vec<_> = geometry
            .positions
            .iter()
            .map(|p| p.to_array().map(f32::to_bits))
            .collect();
        expected.sort_unstable();
        object.geometry = geometry.into();
        let mut saved = Glb::parse(
            source.path.clone(),
            source.encode(&[object.clone()], &[]).unwrap(),
        )
        .unwrap();
        for _ in 0..3 {
            let reopened = import_geometry(&saved).unwrap();
            assert_eq!(reopened.faces.len(), if triangulate { 12 } else { 6 });
            assert!(
                reopened
                    .faces
                    .iter()
                    .all(|f| f.corners.len() == if triangulate { 3 } else { 4 })
            );
            let mut actual: Vec<_> = reopened
                .positions
                .iter()
                .map(|p| p.to_array().map(f32::to_bits))
                .collect();
            actual.sort_unstable();
            assert_eq!(actual, expected);
            object.geometry = reopened.into();
            saved = Glb::parse(
                source.path.clone(),
                saved.encode(&[object.clone()], &[]).unwrap(),
            )
            .unwrap();
        }
    }
    fs::remove_dir_all(source.path.parent().unwrap()).unwrap();
}

#[test]
fn model_glb_rejects_polygon_metadata_that_does_not_match_the_rendered_surface() {
    use crate::editor::model_document::from_mesh;
    let (source, mut object) = fixture();
    let mut geometry = Geometry::default();
    from_mesh(&Mesh::from(Cuboid::from_size(Vec3::ONE)), &mut geometry, 0).unwrap();
    object.geometry = geometry.into();
    let mut saved =
        Glb::parse(source.path.clone(), source.encode(&[object], &[]).unwrap()).unwrap();
    let mesh = saved.json["nodes"][0]["mesh"].as_u64().unwrap() as usize;
    for polygon in [json!([0, 1, 999999]), json!([0, 1, 2])] {
        saved.json["meshes"][mesh]["extras"]["bevy_editor_topology"]["primitives"][0][0] = polygon;
        assert!(import_geometry(&saved).is_err());
    }
    fs::remove_dir_all(source.path.parent().unwrap()).unwrap();
}
