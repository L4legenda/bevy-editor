use super::*;
use crate::editor::model_import_mesh::AutoUnwrap;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

fn import_files(root: &Path, parent: &Path, sources: Vec<PathBuf>) -> ImportReport {
    super::import_files(root, parent, sources, AutoUnwrap::default())
}

fn import_editable_files(root: &Path, parent: &Path, sources: Vec<PathBuf>) -> ImportReport {
    super::import_editable_files(root, parent, sources, AutoUnwrap::default())
}

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "bevy-browser-files-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(root.join("assets/textures")).unwrap();
        fs::create_dir_all(root.join("external")).unwrap();
        Self(root)
    }
    fn assets(&self) -> PathBuf {
        self.0.join("assets")
    }
    fn source(&self, name: &str, data: &[u8]) -> PathBuf {
        let path = self.0.join("external").join(name);
        fs::write(&path, data).unwrap();
        path
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn obj_import_writes_an_editable_glb_with_local_materials_and_embedded_color() {
    use crate::editor::model_document::glb::Glb;
    let fixture = Fixture::new();
    let texture = image::RgbaImage::from_pixel(2, 2, image::Rgba([11, 80, 190, 120]));
    fs::create_dir(fixture.0.join("external/materials")).unwrap();
    texture
        .save(fixture.0.join("external/materials/paint.png"))
        .unwrap();
    fixture.source(
        "materials/palette.mtl",
        b"newmtl Cloth\nKd 0.8 0.3 0.2\nmap_Kd paint.png\nnewmtl Metal\nKd 0.3 0.5 0.7\nPm 0.6\n",
    );
    let source = fixture.source("Panel.OBJ", b"mtllib materials/palette.mtl\no Panel\nv 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nv 2 0 0\nv 2 1 0\nvt 0 0\nvt 0.5 0\nvt 0.5 1\nvt 0 1\nvt 1 0\nvt 1 1\nvn 0 0 1\nusemtl Cloth\nf 1/1/1 2/2/1 3/3/1 4/4/1\nusemtl Metal\nf 2/2/1 5/5/1 6/6/1 3/3/1\n");
    let root = fixture.assets();
    fs::write(root.join("Panel.glb"), b"existing").unwrap();
    let report = import_files(&root, Path::new(""), vec![source.clone(), source]);
    assert_eq!(report.imported, [PathBuf::from("Panel (2).glb")]);
    assert_eq!(report.skipped, 1);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert_eq!(fs::read(root.join("Panel.glb")).unwrap(), b"existing");
    let glb = Glb::load(&root.join(&report.imported[0])).unwrap();
    let mesh = &glb.json["meshes"][0];
    let primitives = mesh["primitives"].as_array().unwrap();
    assert_eq!(primitives.len(), 2);
    let cloth = primitives
        .iter()
        .find(|p| {
            glb.json["materials"][p["material"].as_u64().unwrap() as usize]["name"] == "Cloth"
        })
        .unwrap();
    let uvs = obj_test_uvs(&glb, cloth);
    let bytes = glb.data();
    let json_len = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
    let bin_start = 28 + json_len;
    for corner in [[0., 1.], [0.5, 1.], [0.5, 0.], [0., 0.]] {
        assert!(uvs.contains(&corner), "{uvs:?}");
    }
    let cloth = &glb.json["materials"][cloth["material"].as_u64().unwrap() as usize];
    assert_eq!(cloth["alphaMode"], "BLEND");
    assert_eq!(
        cloth["pbrMetallicRoughness"]["baseColorTexture"]["index"],
        0
    );
    assert_eq!(
        mesh["extras"]["bevy_editor_topology"]["primitives"][0][0]
            .as_array()
            .unwrap()
            .len(),
        4
    );
    let png =
        &glb.json["bufferViews"][glb.json["images"][0]["bufferView"].as_u64().unwrap() as usize];
    let start = bin_start + png["byteOffset"].as_u64().unwrap() as usize;
    let end = start + png["byteLength"].as_u64().unwrap() as usize;
    assert_eq!(
        image::load_from_memory(&bytes[start..end])
            .unwrap()
            .into_rgba8(),
        texture
    );
}

fn obj_test_uvs(
    glb: &crate::editor::model_document::glb::Glb,
    primitive: &serde_json::Value,
) -> Vec<[f32; 2]> {
    let uv =
        &glb.json["accessors"][primitive["attributes"]["TEXCOORD_0"].as_u64().unwrap() as usize];
    let view = &glb.json["bufferViews"][uv["bufferView"].as_u64().unwrap() as usize];
    let bytes = glb.data();
    let bin = 28 + u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
    let start = bin + view["byteOffset"].as_u64().unwrap() as usize;
    let end = start + view["byteLength"].as_u64().unwrap() as usize;
    bytes[start..end]
        .chunks_exact(8)
        .map(|uv| {
            [
                f32::from_le_bytes(uv[..4].try_into().unwrap()),
                f32::from_le_bytes(uv[4..].try_into().unwrap()),
            ]
        })
        .collect()
}

#[test]
fn obj_import_unwraps_missing_material_sets_and_keeps_authored_uvs() {
    use crate::editor::model_document::glb::Glb;
    let fixture = Fixture::new();
    fixture.source(
        "Mixed.mtl",
        b"newmtl Keep\nKd 1 0 0\nnewmtl Blank\nKd 0 1 0\nnewmtl Other\nKd 0 0 1\n",
    );
    let source = fixture.source(
        "Mixed.obj",
        b"mtllib Mixed.mtl\no Mixed\nv 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nv 2 0 0\nv 2 1 0\nv 3 0 0\nv 3 1 0\nvt 0.125 0.125\nvt 0.375 0.125\nvt 0.375 0.375\nvt 0.125 0.375\nusemtl Keep\nf 1/1 2/2 3/3 4/4\nusemtl Blank\nf 2 5 6 3\no NoUv\nusemtl Other\nf 5 7 8 6\n",
    );
    let root = fixture.assets();
    let off = super::import_files(&root, Path::new(""), vec![source.clone()], AutoUnwrap::Off);
    assert!(off.imported.is_empty());
    assert_eq!(off.mesh_unwrapped_sets, 0);
    assert!(off.status().contains("отсутствуют UV"), "{}", off.status());
    assert!(!root.join("Mixed.glb").exists());
    let report = import_files(&root, Path::new(""), vec![source.clone()]);
    assert_eq!(report.imported, [PathBuf::from("Mixed.glb")]);
    assert_eq!(report.mesh_unwrapped_sets, 2, "{}", report.status());
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    let glb = Glb::load(&root.join(&report.imported[0])).unwrap();
    let sets: HashMap<_, _> = glb.json["meshes"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|mesh| mesh["primitives"].as_array().unwrap())
        .map(|primitive| {
            let material = &glb.json["materials"][primitive["material"].as_u64().unwrap() as usize];
            (
                material["name"].as_str().unwrap(),
                obj_test_uvs(&glb, primitive),
            )
        })
        .collect();
    assert_eq!(sets.len(), 3);
    let authored = &sets["Keep"];
    assert_eq!(authored.len(), 4);
    for uv in [
        [0.125, 0.875],
        [0.375, 0.875],
        [0.375, 0.625],
        [0.125, 0.625],
    ] {
        assert!(authored.contains(&uv), "{authored:?}");
    }
    let all = super::import_editable_files(&root, Path::new(""), vec![source], AutoUnwrap::All);
    assert!(all.errors.is_empty(), "{}", all.status());
    assert_eq!(all.mesh_unwrapped_sets, 3);
    let glb = Glb::load(&root.join(&all.imported[0])).unwrap();
    let keep = glb.json["meshes"].as_array().unwrap().iter()
        .flat_map(|mesh| mesh["primitives"].as_array().unwrap())
        .find(|primitive| {
            let slot = primitive["material"].as_u64().unwrap() as usize;
            glb.json["materials"][slot]["name"] == "Keep"
        })
        .unwrap();
    let rebuilt = obj_test_uvs(&glb, keep);
    assert_ne!(&rebuilt, authored);
    assert!(rebuilt.iter().all(|uv| uv.iter().all(|v| (0. ..=1.).contains(v))));
    for name in ["Blank", "Other"] {
        let uvs = &sets[name];
        assert!(
            uvs.iter()
                .flatten()
                .all(|coordinate| (0. ..=1.).contains(coordinate))
        );
        let [a, b, c] = [uvs[0], uvs[1], uvs[2]].map(bevy::math::Vec2::from_array);
        assert!((b - a).perp_dot(c - a).abs() > 0.01, "{name}: {uvs:?}");
    }
}

#[test]
fn obj_import_with_no_surfaces_does_not_publish_a_glb() {
    let fixture = Fixture::new();
    let source = fixture.source("Empty.obj", b"v 0 0 0\nv 1 0 0\nl 1 2\n");
    let root = fixture.assets();
    let report = import_files(&root, Path::new(""), vec![source]);
    assert!(report.imported.is_empty());
    assert_eq!(report.errors.len(), 1);
    assert!(!root.join("Empty.glb").exists());
}

#[test]
fn created_png_has_the_requested_size_and_opaque_white_pixels() {
    let fixture = Fixture::new();
    let path = create_image(
        &fixture.assets(),
        Path::new("textures"),
        "Белый холст.PNG",
        37,
        19,
    )
    .unwrap();
    assert_eq!(path, PathBuf::from("textures/Белый холст.png"));
    let image = image::open(fixture.assets().join(path))
        .unwrap()
        .into_rgba8();
    assert_eq!(image.dimensions(), (37, 19));
    assert!(image.pixels().all(|pixel| pixel.0 == [255, 255, 255, 255]));
    let tiny = create_image(&fixture.assets(), Path::new(""), "Точка", 1, 1).unwrap();
    assert_eq!(
        image::image_dimensions(fixture.assets().join(tiny)).unwrap(),
        (1, 1)
    );
}

#[test]
fn image_creation_does_not_overwrite_existing_files_even_with_concurrent_requests() {
    let fixture = Fixture::new();
    let root = fixture.assets();
    fs::write(
        root.join("textures/Новое изображение.png"),
        b"original artwork",
    )
    .unwrap();
    assert_eq!(
        suggested_image_name(&root, Path::new("textures")),
        "Новое изображение (2)"
    );
    let paths = std::thread::scope(|scope| {
        let a = scope.spawn(|| {
            create_image(&root, Path::new("textures"), "Новое изображение", 12, 10).unwrap()
        });
        let b = scope.spawn(|| {
            create_image(&root, Path::new("textures"), "Новое изображение", 12, 10).unwrap()
        });
        [a.join().unwrap(), b.join().unwrap()]
    });
    assert_ne!(paths[0], paths[1]);
    assert_eq!(
        fs::read(root.join("textures/Новое изображение.png")).unwrap(),
        b"original artwork"
    );
    for path in paths {
        assert_eq!(image::image_dimensions(root.join(path)).unwrap(), (12, 10));
    }
}

#[test]
fn invalid_image_dimensions_names_and_destinations_create_no_files() {
    let fixture = Fixture::new();
    let root = fixture.assets();
    for (width, height) in [
        (0, 100),
        (10, 0),
        (8193, 1),
        (8192, 8192),
        (u32::MAX, u32::MAX),
    ] {
        assert!(create_image(&root, Path::new("textures"), "Bad", width, height).is_err());
    }
    for name in ["", " ", "../outside", "a/b", "NUL", ".png"] {
        assert!(create_image(&root, Path::new("textures"), name, 10, 10).is_err());
    }
    for parent in [Path::new("../external"), fixture.0.as_path()] {
        assert!(create_image(&root, parent, "Outside", 10, 10).is_err());
    }
    assert!(validate_image_size(4096, 4096).is_ok());
    assert!(validate_image_size(8192, 2048).is_ok());
    assert_eq!(fs::read_dir(root.join("textures")).unwrap().count(), 0);
    assert!(!fixture.0.join("external/Outside.png").exists());
}

#[test]
fn browser_import_copies_multiple_images_and_preserves_sources_and_existing_files() {
    let fixture = Fixture::new();
    let source = fixture.source("Текстура.PNG", b"new image bytes");
    fs::write(
        fixture.assets().join("textures/Текстура.PNG"),
        b"existing image",
    )
    .unwrap();
    let svg = fixture.source("icon.svg", b"svg content");
    let report = import_files(
        &fixture.assets(),
        Path::new("textures"),
        vec![source.clone(), svg.clone()],
    );
    assert!(report.errors.is_empty());
    assert_eq!(report.imported.len(), 2);
    assert_eq!(
        fs::read(fixture.assets().join("textures/Текстура.PNG")).unwrap(),
        b"existing image"
    );
    assert_eq!(
        fs::read(fixture.assets().join("textures/Текстура (2).PNG")).unwrap(),
        b"new image bytes"
    );
    assert_eq!(fs::read(&source).unwrap(), b"new image bytes");
    assert_eq!(fs::read(&svg).unwrap(), b"svg content");
    assert!(
        report
            .imported
            .iter()
            .all(|path| path.starts_with("textures"))
    );
}

#[test]
fn editable_fbx_import_keeps_static_objects_and_embeds_both_texture_sources() {
    use crate::editor::model_document::glb::Glb;
    let fixture = Fixture::new();
    let external = include_bytes!("../../tests/fixtures/fbx/props_external.FBX");
    let embedded = include_bytes!("../../tests/fixtures/fbx/embedded/props_embedded.fbx");
    let texture = include_bytes!("../../tests/fixtures/fbx/textures/checker.png");
    fs::create_dir_all(fixture.0.join("external/textures")).unwrap();
    fixture.source("textures/checker.png", texture);
    let root = fixture.assets();
    let report = import_editable_files(
        &root,
        Path::new(""),
        vec![
            fixture.source("External.FBX", external),
            fixture.source("Embedded.fbx", embedded),
        ],
    );
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert_eq!(
        report.imported,
        [PathBuf::from("External.glb"), PathBuf::from("Embedded.glb")]
    );
    assert_eq!(
        fs::read(fixture.0.join("external/External.FBX")).unwrap(),
        external
    );
    assert_eq!(
        fs::read(fixture.0.join("external/Embedded.fbx")).unwrap(),
        embedded
    );
    let expected = image::load_from_memory(texture).unwrap().into_rgba8();
    for path in report.imported {
        let glb = Glb::load(&root.join(path)).unwrap();
        let meshes = glb.json["meshes"].as_array().unwrap();
        assert_eq!(meshes.len(), 2, "{}", glb.path.display());
        let primitives: Vec<_> = meshes
            .iter()
            .flat_map(|mesh| mesh["primitives"].as_array().unwrap())
            .collect();
        assert_eq!(primitives.len(), 3);
        let textured: Vec<_> = primitives
            .iter()
            .filter_map(|primitive| {
                let material = &glb.json["materials"][primitive["material"].as_u64()? as usize];
                material["pbrMetallicRoughness"]["baseColorTexture"]["index"].as_u64()
            })
            .collect();
        assert!(!textured.is_empty(), "{}", glb.path.display());
        let bytes = glb.data();
        let bin = 28 + u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
        for index in textured {
            let image = glb.json["textures"][index as usize]["source"]
                .as_u64()
                .unwrap() as usize;
            let view = glb.json["images"][image]["bufferView"].as_u64().unwrap() as usize;
            let view = &glb.json["bufferViews"][view];
            let start = bin + view["byteOffset"].as_u64().unwrap() as usize;
            let end = start + view["byteLength"].as_u64().unwrap() as usize;
            let pixels = image::load_from_memory(&bytes[start..end])
                .unwrap()
                .into_rgba8();
            assert_eq!(pixels, expected, "{}", glb.path.display());
        }
    }
}

#[test]
fn editable_fbx_import_uses_scene_units_and_keeps_uvs() {
    use crate::editor::model_document::glb::Glb;
    let fixture = Fixture::new();
    let model = fixture.source(
        "Panel.fbx",
        include_bytes!("../../tests/fixtures/fbx/panel_ascii.fbx"),
    );
    let root = fixture.assets();
    let report = import_editable_files(&root, Path::new(""), vec![model]);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    let glb = Glb::load(&root.join(&report.imported[0])).unwrap();
    let primitive = &glb.json["meshes"][0]["primitives"][0];
    let accessor =
        &glb.json["accessors"][primitive["attributes"]["POSITION"].as_u64().unwrap() as usize];
    for (label, expected) in [("min", [0.25, 0.5, 0.]), ("max", [1.25, 1.5, 0.])] {
        for (component, expected) in accessor[label].as_array().unwrap().iter().zip(expected) {
            assert!(
                (component.as_f64().unwrap() - expected).abs() < 1e-4,
                "{accessor:?}"
            );
        }
    }
    let uvs = obj_test_uvs(&glb, primitive);
    assert!(uvs.iter().any(|uv| uv[0] > 0.9));
    assert!(uvs.iter().any(|uv| uv[1] > 0.9));
}

#[test]
fn a_single_fbx_drop_copies_relative_textures_and_preserves_existing_files() {
    let fixture = Fixture::new();
    let model = fixture.source(
        "Props.FBX",
        include_bytes!("../../tests/fixtures/fbx/props_external.FBX"),
    );
    let texture = include_bytes!("../../tests/fixtures/fbx/textures/checker.png");
    fs::create_dir_all(fixture.0.join("external/textures")).unwrap();
    fixture.source("textures/checker.png", texture);
    fs::create_dir(fixture.assets().join("models")).unwrap();
    let drop_model = || import_files(&fixture.assets(), Path::new("models"), vec![model.clone()]);
    let first = drop_model();
    assert!(first.errors.is_empty(), "{:?}", first.errors);
    assert_eq!(first.imported, [PathBuf::from("models/Props.FBX")]);
    assert_eq!(first.textures_copied, 1);
    let dest = fixture.assets().join("models/textures/checker.png");
    assert_eq!(fs::read(&dest).unwrap(), texture);
    let again = drop_model();
    assert!(again.errors.is_empty(), "{:?}", again.errors);
    assert_eq!(again.textures_copied, 0);
    assert_eq!(fs::read_dir(dest.parent().unwrap()).unwrap().count(), 1);

    let mut existing = texture.to_vec();
    existing[0] ^= 1;
    fs::write(&dest, &existing).unwrap();
    let collision = drop_model();
    assert_eq!(collision.textures_copied, 0);
    assert!(
        collision
            .errors
            .iter()
            .any(|e| e.contains("checker.png") && e.contains("другая текстура"))
    );
    assert_eq!(fs::read(dest).unwrap(), existing);
}

#[cfg(unix)]
#[test]
fn external_fbx_textures_cannot_follow_a_project_symlink_to_write_outside() {
    let fixture = Fixture::new();
    let source =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fbx/props_external.FBX");
    fs::create_dir(fixture.assets().join("models")).unwrap();
    let outside = fixture.0.join("external");
    std::os::unix::fs::symlink(&outside, fixture.assets().join("models/textures")).unwrap();
    let report = import_files(&fixture.assets(), Path::new("models"), vec![source]);
    assert_eq!(report.imported.len(), 1);
    assert_eq!(report.textures_copied, 0);
    assert!(!report.errors.is_empty());
    assert!(!outside.join("checker.png").exists());
}

#[test]
fn browser_import_copies_fbx_models_alongside_their_textures() {
    let fixture = Fixture::new();
    let model = fixture.source(
        "Props.FBX",
        include_bytes!("../../tests/fixtures/fbx/embedded/props_embedded.fbx"),
    );
    let image = fixture.source(
        "checker.png",
        include_bytes!("../../tests/fixtures/fbx/textures/checker.png"),
    );
    let report = import_files(&fixture.assets(), Path::new(""), vec![model.clone(), image]);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert_eq!(
        report.imported,
        [PathBuf::from("Props.FBX"), PathBuf::from("checker.png")]
    );
    assert_eq!(
        fs::read(fixture.assets().join("Props.FBX")).unwrap(),
        fs::read(model).unwrap()
    );
}

#[test]
fn browser_import_rejects_unsupported_files_and_missing_files_without_stopping_the_batch() {
    let fixture = Fixture::new();
    let image = fixture.source("photo.jpg", b"jpeg");
    let text = fixture.source("notes.txt", b"important document");
    let missing = fixture.0.join("missing.png");
    let report = import_files(
        &fixture.assets(),
        Path::new(""),
        vec![text.clone(), missing, image],
    );
    assert_eq!(report.imported, vec![PathBuf::from("photo.jpg")]);
    assert_eq!(report.errors.len(), 2);
    assert!(!fixture.assets().join("notes.txt").exists());
    assert!(text.exists());
    assert!(report.status().contains("1"));
}

#[test]
fn browser_import_skips_repeated_sources_and_files_already_in_the_destination() {
    let fixture = Fixture::new();
    let source = fixture.source("photo.webp", b"webp");
    let report = import_files(
        &fixture.assets(),
        Path::new("textures"),
        vec![source.clone(), source],
    );
    assert_eq!(report.imported.len(), 1);
    assert_eq!(report.skipped, 1);
    let report = import_files(
        &fixture.assets(),
        Path::new("textures"),
        vec![fixture.assets().join("textures/photo.webp")],
    );
    assert!(report.imported.is_empty());
    assert_eq!(report.skipped, 1);
    assert!(!fixture.assets().join("textures/photo (2).webp").exists());
}

#[test]
fn browser_new_folder_uses_the_selected_parent_and_never_replaces_an_existing_entry() {
    let fixture = Fixture::new();
    let path = create_folder(&fixture.assets(), Path::new("textures"), "Картинки").unwrap();
    assert_eq!(path, PathBuf::from("textures/Картинки"));
    assert!(fixture.assets().join(&path).is_dir());
    fs::write(fixture.assets().join(&path).join("keep.txt"), b"keep").unwrap();
    assert!(create_folder(&fixture.assets(), Path::new("textures"), "Картинки").is_err());
    assert_eq!(
        fs::read(fixture.assets().join(path).join("keep.txt")).unwrap(),
        b"keep"
    );
    fs::write(fixture.assets().join("textures/file"), b"keep").unwrap();
    assert!(create_folder(&fixture.assets(), Path::new("textures"), "file").is_err());
    assert!(create_folder(&fixture.assets(), Path::new(""), "Новая папка").is_ok());
    assert_eq!(
        suggested_name(&fixture.assets(), Path::new("")),
        "Новая папка (2)"
    );
}

#[test]
fn browser_folder_names_reject_hidden_paths_and_nonportable_names() {
    for name in [
        "",
        " ",
        ".",
        "..",
        ".hidden",
        "../escape",
        "folder/name",
        "folder\\name",
        "C:drive",
        "trailing.",
        " trailing",
        "trailing ",
        "CON",
        "con.txt",
        "NUL",
        "COM1",
        "lpt9",
        "bad?name",
        "line\nfeed",
    ] {
        assert!(
            validate_name(name).is_err(),
            "Accepted invalid folder name: {name:?}"
        );
    }
    for name in [
        "Картинки",
        "Textures 2",
        "texture.pack",
        "COM10",
        "AUXiliary",
    ] {
        assert!(validate_name(name).is_ok());
    }
}

#[test]
fn browser_writes_cannot_escape_the_asset_root() {
    let fixture = Fixture::new();
    let source = fixture.source("photo.png", b"png");
    for parent in [Path::new("../external"), fixture.0.as_path()] {
        assert!(create_folder(&fixture.assets(), parent, "escape").is_err());
        let report = import_files(&fixture.assets(), parent, vec![source.clone()]);
        assert!(report.imported.is_empty());
        assert_eq!(report.errors.len(), 1);
    }
    assert!(!fixture.0.join("external/escape").exists());
}
