//! Editor-only image planes stored as portable textured assets.
use super::{
    Editor, PlacedModel, browser_files, image_previews,
    model_document::{MaterialSlot, Object, glb::Glb},
    model_mesh::{Corner, Face, Geometry},
};
use bevy::prelude::*;
use image::ImageEncoder;
use serde_json::json;
use std::{
    fs, io,
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Component)]
pub(super) struct ReferenceRoot;

pub(super) struct Prepared {
    name: String,
    bytes: Vec<u8>,
}

pub(super) fn prepare(root: &Path, relative: &Path) -> Result<Prepared, String> {
    let root = root.canonicalize().map_err(|error| error.to_string())?;
    let path = root
        .join(relative)
        .canonicalize()
        .map_err(|error| error.to_string())?;
    if !path.starts_with(&root) || !path.is_file() {
        return Err("Изображение должно находиться в ассетах проекта".into());
    }
    let decoded = image_previews::decode_image(&path, UVec2::splat(2048))?;
    let size = decoded.original.as_vec2();
    if size.min_element() <= 0.0 {
        return Err("Изображение пустое".into());
    }
    let size = size * (2.0 / size.max_element());
    let transparent = decoded.rgba.chunks_exact(4).any(|pixel| pixel[3] != 255);
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(
            &decoded.rgba,
            decoded.size.x,
            decoded.size.y,
            image::ExtendedColorType::Rgba8,
        )
        .map_err(|error| error.to_string())?;
    let half_width = size.x * 0.5;
    let corners = [Vec2::Y, Vec2::ONE, Vec2::X, Vec2::ZERO]
        .into_iter()
        .enumerate()
        .map(|(index, uv)| {
            let mut corner = Corner::new(index);
            corner.uv = uv;
            corner
        })
        .collect();
    let object = Object {
        id: 0,
        source_node: None,
        name: "Изображение".into(),
        geometry: Geometry {
            positions: vec![
                Vec3::new(-half_width, 0.0, 0.0),
                Vec3::new(half_width, 0.0, 0.0),
                Vec3::new(half_width, size.y, 0.0),
                Vec3::new(-half_width, size.y, 0.0),
            ],
            faces: vec![Face {
                corners,
                material: 0,
                smooth: false,
            }],
            ..default()
        }
        .into(),
        transform: Transform::IDENTITY,
        materials: vec![MaterialSlot {
            material: default(),
            source: json!({"material": 0}),
            paint: None,
            authored: None,
            bake_fixes: default(),
        }],
        hidden: false,
    };
    let bytes = Glb::new_asset_configured(
        &[object],
        &[png],
        |gltf| {
            gltf["extensionsUsed"] = json!(["KHR_materials_unlit"]);
            gltf["materials"][0] = json!({
                "name": "Изображение",
                "doubleSided": true,
                "alphaMode": if transparent { "BLEND" } else { "OPAQUE" },
                "pbrMetallicRoughness": {"baseColorTexture": {"index": 0}},
                "extensions": {"KHR_materials_unlit": {}},
            });
            Ok(())
        },
        None,
    )?;
    let stem = relative
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("Изображение");
    let name = format!("Референс · {stem}")
        .chars()
        .filter(|c| !c.is_control())
        .take(128)
        .collect();
    Ok(Prepared { name, bytes })
}

fn save(root: &Path, bytes: &[u8]) -> io::Result<String> {
    let folder = crate::data::IMAGE_REFERENCE_DIRECTORY;
    let mut relative = PathBuf::new();
    let mut parent = root.canonicalize()?;
    for component in Path::new(folder).components() {
        relative.push(component);
        match fs::create_dir(parent.join(component)) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
        parent = browser_files::directory(root, &relative)?;
    }
    let filename = format!("{}.glb", blake3::hash(bytes).to_hex());
    let path = parent.join(&filename);
    let matches = || {
        fs::symlink_metadata(&path).is_ok_and(|metadata| {
            metadata.file_type().is_file() && metadata.len() == bytes.len() as u64
        }) && fs::read(&path).is_ok_and(|existing| existing == bytes)
    };
    if !matches() {
        let mut file = tempfile::NamedTempFile::new_in(&parent)?;
        file.write_all(bytes)?;
        file.as_file().sync_all()?;
        if let Err(error) = file.persist_noclobber(&path)
            && (error.error.kind() != io::ErrorKind::AlreadyExists || !matches())
        {
            return Err(error.error);
        }
    }
    Ok(format!("{folder}/{filename}"))
}

pub(super) fn place(
    editor: &mut Editor,
    reference: Prepared,
    pose: Transform,
) -> Result<(), String> {
    let asset = save(&editor.root, &reference.bytes).map_err(|error| error.to_string())?;
    editor.finish_camera_navigation();
    let before = editor.edits.clone();
    editor
        .add_named_model(
            PlacedModel {
                selector: None,
                uid: None,
                id: 0,
                asset,
                pose: pose.into(),
                bounds: None,
            },
            Some(reference.name.clone()),
        )
        .ok_or("Не удалось добавить объект в сцену")?;
    editor.finish_edit(before);
    editor.status = format!("Добавлен {}", reference.name);
    Ok(())
}
