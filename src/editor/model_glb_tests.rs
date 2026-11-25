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
            bake_fixes: Default::default(),
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
fn paint_camera_angle_marks_the_document_dirty_but_framing_alone_does_not() {
    use crate::editor::model_document::ModelCamera;

    let (source, object) = fixture();
    let folder = source.path.parent().unwrap().to_path_buf();
    let mut doc = Document::new(source, vec![object]);
    let camera = ModelCamera {
        center: [1., 2., 3.], distance: 6., yaw: 0.1, pitch: 0.2,
        fov: crate::editor::viewport::EDITOR_FOV, orthographic: false,
    };
    doc.model_camera = Some(camera);
    assert!(!doc.dirty());
    let changed = ModelCamera { fov: 75_f32.to_radians(), ..camera };
    doc.model_camera = Some(changed);
    assert!(doc.dirty());
    doc.save().unwrap();
    let mut reopened = Document::new(Glb::load(&doc.source.path).unwrap(), doc.objects.clone());
    assert_eq!(reopened.model_camera, Some(changed));
    reopened.model_camera = Some(ModelCamera { center: [2., 3., 4.], distance: 4., ..changed });
    assert!(!reopened.dirty());
    fs::remove_dir_all(folder).unwrap();
}

#[test]
fn paint_mesh_replacement_transfers_active_layers_and_round_trips_through_glb() {
    use crate::editor::{
        model_document::paint_mesh_replacement::{Shape, primitive},
        model_paint_data::{Layer, LayerKind, Stack, Surface, mask::{BakedInputs, Mask, INPUT_CHANNELS}, solid},
    };

    let (source, original) = fixture();
    let folder = source.path.parent().unwrap().to_path_buf();
    let mut other = original.clone();
    other.id = 1;
    other.source_node = None;
    other.materials[0].source["extras"]["bevy_editor_udim"] = json!(1013);
    let mut layer = Layer::new("Мазки".into(), LayerKind::Paint);
    layer.mask = Some(Mask::new(solid(16, [173; 4]), Vec::new()).into());
    let surface = Arc::new(Surface {
        size: 16,
        base: std::array::from_fn(|_| solid(16, [128; 4])),
        stack: Stack { layers: vec![layer, Layer::new("Заливка".into(), LayerKind::Fill)], selected: 1 },
        material: json!({"name": "gold"}),
        baked: Some(Arc::new(BakedInputs {
            size: 16, edge_reach: 1., disabled_inputs: 0,
            texels: vec![[128; INPUT_CHANNELS]; 16 * 16], part_ids: None, custom: vec![],
        })),
    });
    other.materials[0].paint = Some(surface.clone());
    let mut doc = Document::new(source, vec![original, other]);
    doc.paint_set = Some(1);
    doc.select_object(1, false);
    let mut materials = Assets::default();
    let report = doc.replace_paint_mesh(primitive(Shape::Plane).unwrap(), true, false, true, &mut materials).unwrap();
    assert_eq!((report.transferred, report.omitted), (1, 1));
    assert!(doc.auto_bake_pending);
    let check_surface = |paint: &Surface| {
        assert_eq!(paint.size, 16);
        assert_eq!(paint.stack.selected, 1);
        assert_eq!(paint.stack.layers[0].name, "Мазки");
        assert_eq!(paint.stack.layers[1].name, "Заливка");
        assert_eq!(paint.stack.layers[0].mask.as_ref().unwrap().pixels[0], 173);
        assert!(paint.baked.is_none());
    };
    let slot = &doc.objects[0].materials[0];
    check_surface(slot.paint.as_deref().unwrap());
    assert!(slot.udim().is_none());
    assert!(doc.can_restore_paint_mesh(false));
    doc.undo();
    assert_eq!(doc.paint_set_index(), 1);
    assert!(Arc::ptr_eq(doc.objects[1].materials[0].paint.as_ref().unwrap(), &surface));
    assert!(doc.can_restore_paint_mesh(true));
    doc.redo();
    assert_eq!(doc.paint_set_index(), 0);
    assert_eq!(doc.objects.len(), 1);
    for _ in 0..2 {
        doc.save().unwrap();
        let saved = Glb::load(&doc.source.path).unwrap();
        let live: Vec<_> = saved.json["nodes"].as_array().unwrap().iter()
            .filter_map(|node| node["mesh"].as_u64()).collect();
        assert_eq!(live.len(), 1);
        let primitive = &saved.json["meshes"][live[0] as usize]["primitives"][0];
        check_surface(&saved.paint_surface(primitive).unwrap().unwrap());
    }
    fs::remove_dir_all(folder).unwrap();
}

#[test]
fn imported_bake_defaults_are_clean_and_explicit_factory_settings_survive_profile_changes() {
    use crate::editor::model_bake_settings::{Defaults, Quality, Settings};

    let (source, object) = fixture();
    let folder = source.path.parent().unwrap().to_path_buf();
    let mut doc = Document::new(source, vec![object]);
    doc.initialize_bake_defaults(Defaults { quality: Quality::High, match_names: false });
    assert_eq!(doc.bake_settings.quality, Quality::High);
    assert!(!doc.bake_settings.match_names);
    assert!(!doc.dirty());

    doc.bake_settings = Settings::default();
    doc.save().unwrap();
    let mut reopened = Document::new(Glb::load(&doc.source.path).unwrap(), doc.objects.clone());
    reopened.initialize_bake_defaults(Defaults { quality: Quality::Ultra, match_names: false });
    assert_eq!(reopened.bake_settings, Settings::default());
    assert!(!reopened.dirty());

    reopened.source.json["extras"] = json!(["third-party data"]);
    reopened.initialize_bake_defaults(Defaults { quality: Quality::Ultra, match_names: false });
    assert_eq!(reopened.bake_settings, Settings::default());
    reopened.save().unwrap();
    let saved = Glb::load(&reopened.source.path).unwrap();
    assert_eq!(saved.json["extras"], json!(["third-party data"]));
    fs::remove_dir_all(folder).unwrap();
}

#[test]
fn paint_preview_settings_are_saved_without_changing_material_channels() {
    use crate::editor::model_document::{PAINT_PREVIEW_EXTRA, PAINT_UV_EXTRA, PaintPreview, PaintUv};

    let (mut source, object) = fixture();
    let folder = source.path.parent().unwrap().to_path_buf();
    source.json["extras"] = json!({"keep": 7});
    let mut doc = Document::new(source, vec![object]);
    let settings = PaintPreview {
        height_strength: Some(1.25),
        emission_strength: 4.75,
        use_baked_normal: true,
        ao_strength: 0.255,
    };
    assert!(!doc.dirty());
    let wire = PaintUv { show_wire: true, show_wire_3d: true, wire_opacity: 0.68, show_seams_2d: true, show_seams_3d: true };
    doc.paint_uv = wire;
    assert!(doc.dirty());
    doc.paint_preview = settings;
    let material = doc.objects[0].materials[0].source.clone();
    doc.save().unwrap();
    assert!(!doc.dirty());
    let saved = Glb::load(&doc.source.path).unwrap();
    assert_eq!(saved.json["extras"]["keep"], 7);
    let mut reopened = Document::new(saved, doc.objects.clone());
    assert_eq!(reopened.paint_preview, settings);
    assert_eq!(reopened.paint_uv, wire);
    assert_eq!(reopened.objects[0].materials[0].source, material);
    assert!(reopened.objects[0].materials[0].paint.is_none());

    reopened.paint_preview = PaintPreview::default();
    reopened.paint_uv = PaintUv::default();
    assert!(reopened.dirty());
    reopened.save().unwrap();
    let saved = Glb::load(&reopened.source.path).unwrap();
    assert_eq!(saved.json["extras"][PAINT_PREVIEW_EXTRA], json!(PaintPreview::default()));
    assert_eq!(saved.json["extras"][PAINT_UV_EXTRA], json!(PaintUv::default()));
    fs::remove_dir_all(folder).unwrap();
}

#[test]
fn legacy_bake_selection_retains_previously_implied_maps() {
    use crate::editor::model_bake_settings::{self as bake, BASIC_MAP_BITS, BENT_NORMAL_BIT};

    let path = Path::new("legacy.glb");
    let mut root = json!({"extras": {"bevy_editor_bake_settings": {"map_bits": 4}}});
    assert_eq!(bake::read(&root, path).map_bits, BASIC_MAP_BITS | 4);
    root["extras"]["bevy_editor_bake_settings"]["map_bits"] = json!(5);
    assert_eq!(bake::read(&root, path).map_bits, BASIC_MAP_BITS | BENT_NORMAL_BIT | 5);

    let explicit = bake::Settings { map_bits: 5, ..Default::default() };
    bake::write(&mut root, path, &explicit).unwrap();
    assert_eq!(bake::read(&root, path), explicit);
}

#[test]
fn bake_settings_survive_glb_reopen_move_and_document_undo() {
    use crate::editor::model_bake_settings::{self as bake, IdSource, Quality, Settings};

    let (source, object) = fixture();
    let model = source.path.clone();
    let parent = model.parent().unwrap();
    let external_cage = parent.parent().unwrap().join("cage.fbx");
    let mut doc = Document::new(source, vec![object]);
    assert_eq!(doc.bake_settings, Settings::default());
    let before = doc.snapshot();
    let settings = Settings {
        high_poly: Some(parent.join("source/high.obj")),
        cage: Some(external_cage.clone()),
        quality: Quality::Ultra,
        map_bits: 9,
        id_source: IdSource::Materials,
        rays: bake::RayOptions {
            occlusion_samples: Some(91),
            thickness_samples: Some(0),
            cavity_reach: 1324,
            sky_cone: 443,
            ..Default::default()
        },
        surface: bake::SurfaceOptions {
            grid: Some(6),
            radius: 37,
            contrast: 1724,
            edge_reach: 511,
            auto_edge_reach: true,
            sharpness: 375,
            hard_angle: 450,
        },
        front: 12,
        rear: 37,
        match_names: false,
        fill_misses: false,
        prefer_front_faces: false,
        average_normals: false,
        auto_fit: true,
    };
    doc.bake_settings = settings.clone();
    doc.checkpoint(before);
    assert!(doc.dirty());
    doc.undo();
    assert!(!doc.dirty());
    doc.redo();
    assert_eq!(doc.bake_settings, settings);

    doc.source.json["extras"] = json!({"keep": 7});
    doc.save().unwrap();
    assert!(!doc.dirty());
    let read = Glb::load(&model).unwrap();
    assert_eq!(read.json["extras"]["keep"], 7);
    let path = read.json["extras"]["bevy_editor_bake_settings"]["high_poly"].as_str().unwrap();
    assert_eq!(Path::new(path), Path::new("source").join("high.obj"));
    let bytes = fs::read(&model).unwrap();
    let reopened = Document::new(read, doc.objects.clone());
    assert_eq!(reopened.bake_settings, settings);
    assert!(!reopened.dirty());

    let moved = parent.join("moved/model.glb");
    let moved = Glb::parse(moved.clone(), bytes).unwrap();
    let mut restored = bake::read(&moved.json, &moved.path);
    assert_eq!(restored.high_poly.take(), Some(parent.join("moved/source/high.obj")));
    assert_eq!(restored.cage, Some(external_cage));
    fs::remove_dir_all(parent).unwrap();
}

#[test]
fn cage_paint_round_trips_across_material_vertex_reordering() {
    use crate::editor::model_cage_data::{self as cage_data, Edit, Scales};
    use std::sync::Arc;

    let (source, mut object) = fixture();
    let folder = source.path.parent().unwrap().to_owned();
    object.geometry.positions.push(Vec3::new(1., 1., 0.));
    object.geometry.faces[0].material = 1;
    object.geometry.faces.push(Face {
        corners: [2, 1, 3].map(Corner::new).to_vec(),
        material: 0,
        smooth: false,
    });
    object.materials.push(object.materials[0].clone());
    let mut doc = Document::new(source, vec![object.clone()]);
    // The synthetic source node has no mesh until the first encode.
    doc.original.clear();
    let before = doc.snapshot();
    Arc::make_mut(&mut doc.cage_edits).insert(object.id, Edit {
        mesh: object.geometry.clone(),
        scales: Scales { values: Arc::new(vec![1.2, 0.8, 1., 2.]) },
    });
    doc.checkpoint(before);
    assert!(doc.dirty());
    doc.undo();
    assert!(doc.cage_edits.is_empty());
    assert!(!doc.dirty());
    doc.redo();
    doc.save().unwrap();

    let saved = Glb::load(&doc.source.path).unwrap();
    assert_eq!(saved.json["nodes"][0]["extras"]["script"], 42);
    let mut reopened = object.clone();
    reopened.geometry = import_geometry(&saved).unwrap().into();
    assert_ne!(reopened.geometry.positions, object.geometry.positions);
    let mut reopened = Document::new(saved, vec![reopened]);
    let scales = &reopened.cage_edits[&object.id].scales.values;
    for (point, scale) in reopened.objects[0].geometry.positions.iter().zip(scales.iter()) {
        let original = object.geometry.positions.iter().position(|p| p == point).unwrap();
        assert_eq!(*scale, [1.2, 0.8, 1., 2.][original]);
    }
    let mut changed = reopened.objects[0].clone();
    changed.geometry.positions[0].x += 0.125;
    assert!(cage_data::read(&reopened.source.json["nodes"][0], &changed).is_none());
    let before = reopened.snapshot();
    Arc::make_mut(&mut reopened.cage_edits).clear();
    reopened.checkpoint(before);
    assert!(reopened.dirty());
    reopened.save().unwrap();
    let saved = Glb::load(&reopened.source.path).unwrap();
    assert!(cage_data::read(&saved.json["nodes"][0], &reopened.objects[0]).is_none());
    fs::remove_dir_all(folder).unwrap();
}

#[test]
fn mesh_map_levels_are_saved_with_their_material_slots() {
    let (source, mut object) = fixture();
    let path = source.path.clone();
    let original = json!({"black": 12, "white": 90, "gamma": 110});
    object.materials[0].source["extras"]["bevy_editor_bake_levels"] =
        json!([["occlusion", original]]);
    let mut doc = Document::new(source, vec![object]);
    doc.original.clear();
    doc.save().unwrap();
    let reopened = Glb::load(&path).unwrap();
    let primitive = &reopened.json["meshes"][0]["primitives"][0];
    assert_eq!(primitive["extras"]["bevy_editor_bake_levels"][0][1], original);
    assert_eq!(primitive["extras"]["keep"], true);
    fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

#[test]
fn mesh_map_fixes_round_trip_and_can_be_removed_independently_of_levels() {
    use crate::editor::model_paint_data::map_fixes::{BakeMap, Plane, EXTRAS};
    use std::sync::Arc;

    let (source, mut object) = fixture();
    let path = source.path.clone();
    let fixes = Arc::new(vec![(BakeMap::Occlusion, Arc::new(Plane {
        size: 2,
        pixels: vec![0, 0, 64, 128, 13, 127, 61, 255],
    }))]);
    object.materials[0].bake_fixes = fixes.clone();
    let mut doc = Document::new(source, vec![object]);
    doc.original.clear();
    doc.save().unwrap();
    let saved = Glb::load(&path).unwrap();
    let mesh = saved.json["nodes"][0]["mesh"].as_u64().unwrap() as usize;
    let primitive = &saved.json["meshes"][mesh]["primitives"][0];
    assert_eq!(saved.bake_fixes(primitive).unwrap(), fixes);
    assert_eq!(primitive["extras"]["keep"], true);

    doc.objects[0].materials[0].source = primitive.clone();
    doc.objects[0].materials[0].source["extras"]["bevy_editor_bake_levels"] =
        json!([["occlusion", {"black": 0, "white": 90, "gamma": 100}]]);
    let before = doc.snapshot();
    doc.objects[0].materials[0].bake_fixes = Arc::default();
    doc.checkpoint(before);
    doc.save().unwrap();
    let saved = Glb::load(&path).unwrap();
    let mesh = saved.json["nodes"][0]["mesh"].as_u64().unwrap() as usize;
    let primitive = &saved.json["meshes"][mesh]["primitives"][0];
    assert!(primitive["extras"].get(EXTRAS).is_none());
    assert!(primitive["extras"]["bevy_editor_bake_levels"].is_array());
    assert_eq!(primitive["extras"]["keep"], true);
    fs::remove_dir_all(path.parent().unwrap()).unwrap();
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
fn model_glb_udim_preserves_local_texture_coordinates_and_global_islands() {
    use crate::editor::model_document::udim;
    let (source, mut object) = fixture();
    let bounds = [Vec2::splat(0.125), Vec2::new(0.875, 0.125), Vec2::new(0.125, 0.875)];
    let make_face = |offset: Vec2| {
        let mut face = object.geometry.faces[0].clone();
        for (corner, point) in face.corners.iter_mut().zip(bounds) {
            corner.uv = point + offset;
        }
        face
    };
    let repeated = make_face(Vec2::new(0.5, 0.));
    object.geometry.faces = vec![make_face(Vec2::ZERO), make_face(Vec2::X), make_face(Vec2::new(2., -1.))];
    let mut conventional = object.clone();
    conventional.geometry.faces.push(repeated);
    udim::import(&mut conventional, true);
    assert_eq!(conventional.materials.len(), 1);
    assert!(conventional.materials[0].udim().is_none());

    let mut tiled = object.clone();
    udim::import(&mut tiled, false);
    assert_eq!(tiled, object, "opted-out repeating UVs must be preserved verbatim");
    udim::import(&mut object, true);
    let tiles = object.materials.iter().map(|m| m.udim().unwrap().number()).collect::<Vec<_>>();
    assert_eq!(tiles, [1001, 1002, 1013]);
    let bytes = source.encode(&[object.clone()], &[]).unwrap();
    let glb = Glb::parse(source.path.clone(), bytes.clone()).unwrap();
    let parsed = bevy::gltf::gltf::Gltf::from_slice(&bytes).unwrap();
    let gltf_mesh = parsed.nodes().next().unwrap().mesh().unwrap();
    let mut reopened = object.clone();
    for primitive in gltf_mesh.primitives() {
        let slot = primitive.index();
        let meta = &glb.json["meshes"][gltf_mesh.index()]["primitives"][slot];
        assert_eq!(meta["extras"]["bevy_editor_udim"], tiles[slot]);
        assert_eq!(meta["extras"]["keep"], true);
        reopened.materials[slot].source = meta.clone();
        let reader = primitive.reader(|_| parsed.blob.as_deref());
        let written = reader.read_tex_coords(0).unwrap().into_f32().map(Vec2::from_array).collect::<Vec<_>>();
        assert!(written.iter().all(|uv| uv.min_element() >= 0. && uv.max_element() <= 1.));
        for index in 0..3 {
            reopened.geometry.faces[slot].corners[index].uv = written[index];
        }
    }
    udim::import(&mut reopened, false);
    assert_eq!(reopened.geometry, object.geometry);
    assert_eq!(reopened.udim_tiles().map(udim::Tile::number).collect::<Vec<_>>(), tiles);
    fs::remove_dir_all(source.path.parent().unwrap()).unwrap();
}

#[test]
fn model_glb_texture_fill_reopens_source_images_and_projection() {
    use crate::editor::model_paint_data::{Layer, LayerKind, Stack, Surface, solid, texture_fill};
    use image::ImageEncoder;
    use std::sync::Arc;

    let (mut source, mut object) = fixture();
    for (corner, uv) in object.geometry.faces[0].corners.iter_mut().zip([Vec2::ZERO, Vec2::X, Vec2::Y]) {
        corner.uv = uv;
    }
    let mut bytes = Vec::new();
    image::codecs::png::PngEncoder::new(&mut bytes).write_image(
        &[255, 0, 20, 255, 0, 80, 200, 255, 12, 5, 27, 255, 190, 133, 20, 255],
        2, 2, image::ExtendedColorType::Rgba8,
    ).unwrap();
    let mut layer = Layer::new("Текстура".into(), LayerKind::Fill);
    let mut fill = texture_fill::Fill::default();
    fill.settings.projection = texture_fill::Projection::Triplanar;
    fill.settings.rotation = 45.;
    fill.settings.height_strength = 0.2;
    fill.settings.variation = texture_fill::Variation {
        color: 0.65, roughness: 0.3, height: 0.4, seed: 23., ..Default::default()
    };
    let bitmap = Arc::new(texture_fill::Bitmap::decode("pattern.png".into(), &bytes).unwrap());
    fill.images[0] = Some(bitmap.clone());
    fill.images[1] = Some(bitmap.clone());
    fill.images[3] = Some(bitmap);
    layer.texture_fill = Some(Arc::new(fill));
    let mut surface = Surface {
        size: 16, base: std::array::from_fn(|_| solid(16, [255; 4])),
        stack: Stack { layers: vec![layer], selected: 0 },
        material: serde_json::Value::Null, baked: None,
    };
    surface.bind_texture_fills(&object.geometry, 0, Vec2::ZERO, object.transform);
    assert!(surface.composite(0).chunks_exact(4).any(|p| p[0] != p[2]));
    object.materials[0].paint = Some(Arc::new(surface.clone()));
    source.save(std::slice::from_ref(&object), &[]).unwrap();
    let reopened = Glb::load(&source.path).unwrap();
    let primitive = &reopened.json["meshes"][0]["primitives"][0];
    let mut restored = (*reopened.paint_surface(primitive).unwrap().unwrap()).clone();
    let fill = restored.stack.layers[0].texture_fill.as_ref().unwrap();
    assert_eq!(fill.settings.projection, texture_fill::Projection::Triplanar);
    assert_eq!(fill.settings.rotation, 45.);
    assert_eq!(fill.settings.height_strength, 0.2);
    assert_eq!(fill.settings.variation.seed, 23.);
    assert_eq!(fill.settings.variation.height, 0.4);
    assert_eq!(fill.images[0].as_ref().unwrap().bytes, bytes);
    assert_eq!(fill.images[1].as_ref().unwrap().bytes, bytes);
    restored.bind_texture_fills(&object.geometry, 0, Vec2::ZERO, object.transform);
    assert_eq!(restored.composite(0), surface.composite(0));
    assert_eq!(restored.composite(1), surface.composite(1));
    assert_eq!(restored.composite(3), surface.composite(3));
    fs::remove_dir_all(source.path.parent().unwrap()).unwrap();
}

#[test]
fn model_glb_paint_round_trip_keeps_layers_and_flattened_standard_material() {
    use crate::editor::model_paint_data::{Surface, Stack, Layer, LayerKind, solid};
    use std::sync::Arc;
    let (mut source, mut object) = fixture();
    let mut red = Layer::new("Красный".into(),LayerKind::Paint);
    red.channels[0]=Some(solid(16,[255,0,0,255]));
    let mut specular = [32, 32, 32, 255].repeat(16 * 16);
    for row in specular.chunks_exact_mut(16 * 4) {
        for texel in row[8 * 4..].chunks_exact_mut(4) {
            texel.copy_from_slice(&[224, 224, 224, 255]);
        }
    }
    red.channels[7] = Some(Arc::new(specular));
    source.json["materials"][0]["extensions"]["KHR_materials_specular"] =
        serde_json::json!({"specularFactor":0.3,"specularColorFactor":[0.8,0.9,1.0]});
    let mut blue=Layer::new("Синий".into(),LayerKind::Paint);
    blue.channels[0]=Some(solid(16,[0,0,255,255]));blue.opacity=50.;
    blue.depth = 1;
    let mut group = Layer::new("Финишный цвет".into(), LayerKind::Folder);
    group.collapsed = true;
    group.graph_id = Some(425);
    group.folder_graph = Some(Arc::new(serde_json::json!({
        "name": "Финишный цвет",
        "nodes": [{"id": 1, "def": "layer", "pos": [123.0, -456.0]}],
    })));
    let surface=Arc::new(Surface{size:16,base:std::array::from_fn(|_|solid(16,[255;4])),stack:Stack{layers:vec![red,blue,group],selected:2},material:source.json["materials"][0].clone(),baked:None});
    object.materials[0].paint=Some(surface.clone());
    source.save(&[object],&[]).unwrap();
    let loaded=Glb::load(&source.path).unwrap();
    let mesh=loaded.json["nodes"][0]["mesh"].as_u64().unwrap()as usize;
    let primitive=&loaded.json["meshes"][mesh]["primitives"][0];
    let restored=loaded.paint_surface(primitive).unwrap().unwrap();
    assert_eq!(*restored,*surface);
    assert!(!restored.stack.listed(1));
    assert!(restored.stack.shown(1));
    assert_eq!(&restored.composite(0)[..4],&[128,0,128,255]);
    let mat=primitive["material"].as_u64().unwrap()as usize;
    let tex=loaded.json["materials"][mat]["pbrMetallicRoughness"]["baseColorTexture"]["index"].as_u64().unwrap()as usize;
    let image=loaded.json["textures"][tex]["source"].as_u64().unwrap()as usize;
    let view=loaded.json["images"][image]["bufferView"].as_u64().unwrap()as usize;
    let start=loaded.json["bufferViews"][view]["byteOffset"].as_u64().unwrap()as usize;
    let len=loaded.json["bufferViews"][view]["byteLength"].as_u64().unwrap()as usize;
    let png=image::load_from_memory(&loaded.bin[start..start+len]).unwrap().to_rgba8();
    assert_eq!(png.get_pixel(4,4).0,[128,0,128,255]);
    let parsed = bevy::gltf::gltf::Gltf::from_slice(loaded.data()).unwrap();
    let specular = parsed.materials().nth(mat).unwrap().specular().unwrap();
    assert_eq!(specular.specular_factor(), 1.0);
    assert_eq!(specular.specular_color_factor(), [0.8, 0.9, 1.0]);
    let specular_image = specular.specular_texture().unwrap().texture().source();
    let bevy::gltf::gltf::image::Source::View { view, .. } = specular_image.source() else {
        panic!("flattened specular texture is embedded");
    };
    let specular = image::load_from_memory(&loaded.bin[view.offset()..view.offset() + view.length()])
        .unwrap()
        .into_rgba8();
    assert_eq!(specular.get_pixel(4, 4)[3], 64);
    assert_eq!(specular.get_pixel(12, 4)[3], 255);
    fs::remove_dir_all(source.path.parent().unwrap()).unwrap();
}
#[test]
fn model_glb_baked_generators_keep_inputs_and_editable_masks() {
    use crate::editor::model_paint_data::{
        Layer, LayerKind, Stack, Surface,
        mask::{BakedInputs, Generator, GeneratorInput, GeneratorKind, Mask, MaskLayer, PartIds},
        solid,
    };
    use std::sync::Arc;
    let (mut source, mut object) = fixture();
    for (corner, uv) in object.geometry.faces[0].corners.iter_mut().zip([Vec2::ZERO, Vec2::X, Vec2::Y]) {
        corner.uv = uv;
    }
    let left_id = [120, 40, 30];
    let right_id = [144, 40, 30];
    let mut recipe = crate::editor::model_paint_data::custom::Recipe::new(
        crate::editor::model_paint_data::custom::Kind::Flow, &[],
    );
    recipe.adjust(crate::editor::model_paint_data::custom::Field::Strength, 1);
    recipe.adjust(crate::editor::model_paint_data::custom::Field::Wander, -1);
    let custom_id = recipe.id;
    let copy = recipe.duplicate(std::slice::from_ref(&recipe));
    let mut renamed = recipe.clone();
    renamed.name = "Потёки на корпусе".into();
    let recipes = vec![copy, renamed];
    crate::editor::model_paint_data::custom::write_document(&mut source.json, &recipes).unwrap();
    let baked = Arc::new(BakedInputs {
        size: 16,
        edge_reach: 0.2,
        disabled_inputs: 1 << GeneratorInput::Thin as u32,
        texels: (0..256).map(|i| {
            let cavity = if i % 16 < 8 { 0 } else { 255 };
            [255, cavity, 255, 255, 128, 128, 255, 128, 255, 255]
        }).collect(),
        part_ids: Some(PartIds::new(
            (0..256).map(|i| if i % 16 < 8 { left_id } else { right_id }).collect(),
        )),
        custom: vec![crate::editor::model_paint_data::custom::Plane {
            recipe: recipe.clone(),
            values: Arc::new((0..256).map(|i| if i % 16 < 8 { 0 } else { 255 }).collect()),
        }],
    });
    let mut mask = Mask::new(solid(16, [255; 4]), vec![]);
    mask.use_affine();
    let mut generator = Generator::new(GeneratorKind::CavityGrime);
    generator.values[3] = 0.;
    *mask.edit_generator() = Some(generator);
    let mut id_mask = Generator::new(GeneratorKind::IdMask);
    id_mask.id_color = Some(right_id);
    id_mask.id_tolerance = 5;
    mask.edit_layers().push(MaskLayer {
        enabled: false,
        generator: Some(id_mask),
        ..default()
    });
    let mut mixed = id_mask;
    mixed.set_weight(GeneratorInput::Id, -100);
    mixed.set_weight(GeneratorInput::Cavity, 100);
    mixed.edge_width = 80;
    mask.edit_layers().push(MaskLayer {
        enabled: false,
        generator: Some(mixed),
        ..default()
    });
    let mut custom = Generator::new(GeneratorKind::CustomMap);
    custom.values[3] = 0.;
    custom.custom[0] = crate::editor::model_paint_data::custom::Input { id: custom_id, weight: 100 };
    mask.edit_layers().push(MaskLayer {
        enabled: false,
        generator: Some(custom),
        ..default()
    });
    let mut layer = Layer::new("Грязь".into(), LayerKind::Paint);
    layer.channels[0] = Some(solid(16, [0, 0, 0, 255]));
    layer.mask = Some(Arc::new(mask));
    let mut surface = Surface {
        size: 16,
        base: std::array::from_fn(|_| solid(16, [255; 4])),
        stack: Stack { layers: vec![layer], selected: 0 },
        material: source.json["materials"][0].clone(),
        baked: Some(baked.clone()),
    };
    surface.bind_generators(&object.geometry, 0, Vec2::ZERO, object.transform);
    let expected = surface.composite(0);
    assert_eq!((expected[(2 * 16 + 2) * 4], expected[(16 + 12) * 4]), (0, 255));
    object.materials[0].paint = Some(Arc::new(surface));
    source.save(&[object.clone()], &[]).unwrap();
    let loaded = Glb::load(&source.path).unwrap();
    assert_eq!(crate::editor::model_paint_data::custom::read_document(&loaded.json), recipes);
    bevy::gltf::gltf::Gltf::from_slice(loaded.data()).unwrap();
    let mesh = loaded.json["nodes"][0]["mesh"].as_u64().unwrap() as usize;
    let primitive = &loaded.json["meshes"][mesh]["primitives"][0];
    let mut restored = (*loaded.paint_surface(primitive).unwrap().unwrap()).clone();
    assert_eq!(restored.baked.as_deref(), Some(baked.as_ref()));
    restored.bind_generators(&object.geometry, 0, Vec2::ZERO, object.transform);
    assert_eq!(restored.composite(0), expected);
    let mask = Arc::make_mut(restored.stack.layers[0].mask.as_mut().unwrap());
    mask.edit_generator().as_mut().unwrap().invert = true;
    let changed = restored.composite(0);
    assert_eq!((changed[(2 * 16 + 2) * 4], changed[(16 + 12) * 4]), (255, 0));
    let mask = Arc::make_mut(restored.stack.layers[0].mask.as_mut().unwrap());
    assert_eq!(mask.layers[0].generator, Some(id_mask));
    assert_eq!(mask.context.as_ref().unwrap().id_palette(), &[left_id, right_id]);
    *mask.edit_generator() = Some(id_mask);
    let by_id = restored.composite(0);
    assert_eq!((by_id[(2 * 16 + 2) * 4], by_id[(16 + 12) * 4]), (255, 0));
    let mask = Arc::make_mut(restored.stack.layers[0].mask.as_mut().unwrap());
    mask.edit_generator().as_mut().unwrap().id_tolerance = 30;
    let wide = restored.composite(0);
    assert_eq!((wide[(2 * 16 + 2) * 4], wide[(16 + 12) * 4]), (0, 0));
    let mask = Arc::make_mut(restored.stack.layers[0].mask.as_mut().unwrap());
    assert_eq!(mask.layers[1].generator, Some(mixed));
    *mask.edit_generator() = Some(mixed);
    let by_mix = restored.composite(0);
    assert_eq!((by_mix[(2 * 16 + 2) * 4], by_mix[(16 + 12) * 4]), (0, 255));
    let mask = Arc::make_mut(restored.stack.layers[0].mask.as_mut().unwrap());
    assert_eq!(mask.layers[2].generator, Some(custom));
    *mask.edit_generator() = Some(custom);
    let by_custom = restored.composite(0);
    assert_eq!((by_custom[(2 * 16 + 2) * 4], by_custom[(16 + 12) * 4]), (255, 0));
    let mut legacy = primitive.clone();
    legacy["extras"]["bevy_editor_paint"]["baked_inputs"]
        .as_object_mut().unwrap().remove("part_ids");
    let mut legacy = (*loaded.paint_surface(&legacy).unwrap().unwrap()).clone();
    assert!(legacy.baked.as_ref().unwrap().part_ids.is_none());
    legacy.bind_generators(&object.geometry, 0, Vec2::ZERO, object.transform);
    assert_eq!(legacy.composite(0), expected);
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
        let id_colors = read_id_colors(&parsed.document, &primitive, blob, mesh.count_vertices())?;
        from_mesh_with_topology(
            &mesh,
            &mut geometry,
            primitive.index(),
            (topology["version"].as_u64() == Some(1))
                .then(|| &topology["primitives"][primitive.index()]),
            id_colors.as_deref(),
        )?;
    }
    if topology["version"].as_u64() == Some(1) {
        geometry.restore_uv_seams(&topology["uv_seams"])?;
    }
    geometry.validate()?;
    Ok(geometry)
}

#[test]
fn id_colors_survive_glb_reopen_and_subdivision_without_tinting_vertices() {
    let (source, mut object) = fixture();
    object.source_node = None;
    let color = Vec3::new(0.21, 0.40, 0.79);
    for corner in &mut object.geometry.faces[0].corners {
        corner.id_color = Some(color);
    }
    fs::write(&source.path, Glb::new_asset(&[object]).unwrap()).unwrap();
    let reopened = Glb::load(&source.path).unwrap();
    let mut mesh = import_geometry(&reopened).unwrap();
    let mut selection = crate::editor::model_mesh::Selection::default();
    selection.kind = crate::editor::model_mesh::Element::Face;
    selection.faces.insert(0);
    mesh.subdivide(&mut selection).unwrap();
    assert!(mesh.faces.len() > 1);
    for corner in mesh.faces.iter().flat_map(|face| &face.corners) {
        assert_eq!(corner.color, Vec4::ONE);
        assert!(corner.id_color.unwrap().abs_diff_eq(color, f32::EPSILON));
    }
    fs::remove_dir_all(source.path.parent().unwrap()).unwrap();
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
