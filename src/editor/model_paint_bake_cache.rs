//! Persistent, content-addressed cache of packed bake maps and their reports.
use super::{IdSource, Progress, ProjectionSettings, mesh, mesh::Maps};
use crate::editor::{model_document::Object, model_paint_data::custom};
use lz4_flex::frame::{FrameDecoder, FrameEncoder, FrameInfo};
use bevy::log::warn;
use std::{
    fs::{self, File, FileTimes, OpenOptions},
    io::{self, BufReader, BufWriter, Read, Write},
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock, atomic::Ordering},
    time::SystemTime,
};

const MAGIC: &[u8; 8] = b"BEVYBK04";
const MAX_BYTES: u64 = 2 << 30;
const MAX_RESOLUTION: u32 = 4096;
static CACHE_WRITE: Mutex<()> = Mutex::new(());
type Sources = [Option<blake3::Hash>; 2];

#[derive(Clone)]
pub(super) struct Cache(Option<PathBuf>);

#[cfg(test)]
impl Cache {
    pub(super) fn at(path: PathBuf) -> Self {
        Self(Some(path))
    }
}

impl Default for Cache {
    fn default() -> Self {
        if cfg!(test) {
            return Self(None);
        }
        let root = if cfg!(windows) {
            std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
        } else if cfg!(target_os = "macos") {
            std::env::home_dir().map(|home| home.join("Library/Caches"))
        } else {
            std::env::var_os("XDG_CACHE_HOME")
                .map(PathBuf::from)
                .or_else(|| std::env::home_dir().map(|home| home.join(".cache")))
        };
        Self(root.map(|root| root.join("BevyEditor/bake-cache")))
    }
}

struct Fingerprint(blake3::Hasher);

impl Fingerprint {
    fn number(&mut self, value: impl Into<u64>) {
        self.0.update(&value.into().to_le_bytes());
    }

    fn floats(&mut self, values: impl IntoIterator<Item = f32>) {
        for value in values {
            self.number(value.to_bits());
        }
    }

    fn bytes(&mut self, bytes: &[u8]) {
        self.number(bytes.len() as u64);
        self.0.update(bytes);
    }
}

fn baker_version() -> blake3::Hash {
    static VERSION: OnceLock<blake3::Hash> = OnceLock::new();
    *VERSION.get_or_init(|| {
        let mut hash = Fingerprint(blake3::Hasher::new());
        for file in [
            include_bytes!("model_paint_bake.rs").as_slice(),
            include_bytes!("model_paint_bake_mesh.rs"),
            include_bytes!("model_paint_bake_raster.rs"),
            include_bytes!("model_paint_bake_raster.wgsl"),
            include_bytes!("model_paint_bake_rays_gpu.rs"),
            include_bytes!("model_paint_bake_rays.wgsl"),
            include_bytes!("model_paint_bake_project.rs"),
            include_bytes!("model_paint_bake_source.rs"),
            include_bytes!("model_paint_bake_cage.rs"),
            include_bytes!("model_paint_bake_fit.rs"),
            include_bytes!("model_paint_bake_aa.rs"),
            include_bytes!("model_paint_bake_denoise.rs"),
            include_bytes!("model_paint_bake_denoise_gpu.rs"),
            include_bytes!("model_paint_bake_denoise.wgsl"),
            include_bytes!("model_paint_bake_padding.rs"),
            include_bytes!("model_paint_bake_padding.wgsl"),
            include_bytes!("model_paint_bake_custom.rs"),
            include_bytes!("model_paint_bake_flow.rs"),
            include_bytes!("model_paint_bake_relief.rs"),
            include_bytes!("model_paint_bake_high_relief.rs"),
            include_bytes!("model_mesh.rs"),
            include_bytes!("model_pick.rs"),
            include_bytes!("model_uv_mesh.rs"),
            include_bytes!("model_udim.rs"),
            include_bytes!("../runtime/fbx.rs"),
        ] {
            hash.bytes(file);
        }
        hash.0.finalize()
    })
}

pub(super) fn source_hash(path: &Path, progress: &Progress) -> Option<blake3::Hash> {
    let mut file = File::open(path).ok()?;
    let stamp = file.metadata().ok()?;
    let mut fingerprint = Fingerprint(blake3::Hasher::new());
    fingerprint.bytes(
        path.extension()
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_encoded_bytes(),
    );
    let mut block = [0; 65536];
    loop {
        progress.check().ok()?;
        let n = file.read(&mut block).ok()?;
        if n == 0 {
            break;
        }
        fingerprint.0.update(&block[..n]);
    }
    let current = fs::metadata(path).ok()?;
    (stamp.len() == current.len() && stamp.modified().ok()? == current.modified().ok()?)
        .then(|| fingerprint.0.finalize())
}

fn source_hashes(settings: &ProjectionSettings, progress: &Progress) -> Option<Sources> {
    let mut hashes = [None; 2];
    if let Some(source) = &settings.path {
        for (path, hash) in std::iter::once(source).chain(settings.cage.iter()).zip(&mut hashes) {
            *hash = Some(source_hash(path, progress)?);
        }
    }
    Some(hashes)
}

fn key(
    objects: &[Object],
    object: usize,
    slot: usize,
    size: u32,
    settings: &ProjectionSettings,
    sources: Sources,
    progress: &Progress,
) -> Option<blake3::Hash> {
    let mut hash = Fingerprint(blake3::Hasher::new());
    hash.bytes(MAGIC);
    hash.bytes(baker_version().as_bytes());
    hash.number(object as u64);
    hash.number(slot as u64);
    hash.number(size);
    hash.number(settings.quality as u8);
    hash.number(settings.maps.bits());
    hash.number(settings.id_source as u8);
    let sampling = settings.sampling();
    for count in [sampling.occlusion, sampling.thickness, sampling.sky] {
        hash.number(count);
    }
    hash.floats(sampling.visibility);
    hash.number(sampling.grid);
    let surface = settings.surface;
    hash.number(surface.radius);
    hash.number(surface.contrast);
    hash.number(surface.edge_reach);
    hash.number(u8::from(surface.auto_edge_reach));
    hash.number(surface.sharpness);
    hash.number(surface.hard_angle);
    let mut recipes = settings.custom.as_ref().clone();
    for recipe in &mut recipes {
        recipe.name.clear();
    }
    recipes.sort_unstable_by_key(|recipe| recipe.id);
    hash.bytes(&serde_json::to_vec(&recipes).ok()?);
    for source in sources {
        hash.bytes(source.as_ref().map_or(&[][..], |digest| digest.as_bytes()));
    }
    if settings.path.is_some() {
        hash.bytes(&[
            u8::from(settings.auto_fit),
            u8::from(settings.match_names),
            u8::from(settings.fill_misses),
            u8::from(settings.prefer_front_faces),
            u8::from(settings.average_normals),
            settings.front,
            settings.rear,
        ]);
        if let Some(manual) = &settings.manual {
            hash.number(manual.values.len() as u64);
            for (i, &value) in manual.values.iter().enumerate() {
                if i % 4096 == 0 {
                    progress.check().ok()?;
                }
                hash.floats([value]);
            }
        } else {
            hash.number(0_u64);
        }
    }
    hash.number(objects.iter().filter(|object| !object.hidden).count() as u64);
    for object in objects.iter().filter(|object| !object.hidden) {
        hash.number(object.id as u64);
        hash.bytes(object.name.as_bytes());
        hash.floats(object.transform.translation.to_array());
        hash.floats(object.transform.rotation.to_array());
        hash.floats(object.transform.scale.to_array());
        hash.number(object.materials.len() as u64);
        for slot in 0..object.materials.len() {
            hash.floats(object.uv_offset(slot).to_array());
            if settings.id_source == IdSource::Materials && settings.path.is_none() {
                let material = object.materials[slot].source["material"].as_u64();
                hash.number(u8::from(material.is_some()));
                hash.number(material.unwrap_or(0));
            }
        }
        let geometry = &object.geometry;
        hash.number(geometry.positions.len() as u64);
        for (i, position) in geometry.positions.iter().enumerate() {
            if i % 4096 == 0 {
                progress.check().ok()?;
            }
            hash.floats(position.to_array());
        }
        hash.number(geometry.faces.len() as u64);
        for face in &geometry.faces {
            progress.check().ok()?;
            hash.number(face.material as u64);
            hash.number(u8::from(face.smooth));
            hash.number(face.corners.len() as u64);
            for corner in &face.corners {
                hash.number(corner.vertex as u64);
                hash.floats(corner.uv.to_array());
                hash.number(u8::from(corner.source_normal.is_some()));
                if let Some(normal) = corner.source_normal {
                    hash.floats(normal.to_array());
                }
                if settings.id_source == IdSource::VertexColors && settings.path.is_none() {
                    hash.number(u8::from(corner.id_color.is_some()));
                    if let Some(rgb) = corner.id_color {
                        hash.floats(rgb.to_array());
                    }
                }
            }
        }
    }
    Some(hash.0.finalize())
}

impl Cache {
    pub(super) fn clear(&self) -> io::Result<usize> {
        let Some(directory) = &self.0 else {
            return Ok(0);
        };
        let _lock = CACHE_WRITE.lock().map_err(|_| io::Error::other("bake cache lock poisoned"))?;
        let entries = match fs::read_dir(directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(0),
            Err(error) => return Err(error),
        };
        let mut removed = 0;
        for entry in entries {
            let entry = entry?;
            let name = entry.file_name();
            let is_bake = name.to_str().and_then(|name| name.strip_suffix(".bake"))
                .is_some_and(|hash| hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit()));
            if is_bake && entry.file_type()?.is_file() {
                match fs::remove_file(entry.path()) {
                    Ok(()) => removed += 1,
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error),
                }
            }
        }
        Ok(removed)
    }

    pub(super) fn bake(
        &self,
        objects: Vec<Object>,
        object: usize,
        slot: usize,
        size: u32,
        settings: &ProjectionSettings,
        progress: &Progress,
        device: Option<&mesh::RayDevice>,
    ) -> Result<(Maps, bool), String> {
        let entry = self.0.as_ref().and_then(|dir| {
            let sources = source_hashes(settings, progress)?;
            let key = key(&objects, object, slot, size, settings, sources, progress)?;
            Some((dir.join(format!("{key}.bake")), key, sources))
        });
        progress.check()?;
        if let Some((path, key, _)) = &entry
            && let Ok(mut maps) = load(
                path,
                *key,
                size,
                settings.path.is_some(),
                &settings.custom,
                progress,
            )
        {
            maps.selection = settings.maps;
            progress.check()?;
            progress.rows.store(1000, Ordering::Relaxed);
            return Ok((maps, true));
        }
        let maps = if let Some(device) = device {
            match mesh::bake_on_device(objects.clone(), object, slot, size, settings, progress, Some(device)) {
                Ok(maps) => maps,
                Err(error) if error.starts_with("GPU bake") => {
                    progress.check()?;
                    warn!("GPU bake failed; retrying on the CPU: {error}");
                    progress.rows.store(0, Ordering::Relaxed);
                    mesh::bake_on_device(objects, object, slot, size, settings, progress, None)?
                }
                Err(error) => return Err(error),
            }
        } else {
            mesh::bake_on_device(objects, object, slot, size, settings, progress, None)?
        };
        if let Some((path, key, sources)) = entry
            && source_hashes(settings, progress) == Some(sources)
        {
            let _ = store(&path, key, &maps, progress);
        }
        progress.check()?;
        Ok((maps, false))
    }
}

fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "Invalid bake cache entry")
}

fn active(progress: &Progress) -> io::Result<()> {
    progress.check().map_err(io::Error::other)
}

fn load(
    path: &Path,
    key: blake3::Hash,
    size: u32,
    projected: bool,
    recipes: &[custom::Recipe],
    progress: &Progress,
) -> io::Result<Maps> {
    let file = File::open(path)?;
    if file.metadata()?.len() > MAX_BYTES {
        return Err(invalid());
    }
    let mut reader = BufReader::new(file);
    let mut magic = [0; MAGIC.len()];
    let mut digest = [0; 32];
    reader.read_exact(&mut magic)?;
    reader.read_exact(&mut digest)?;
    if &magic != MAGIC || &digest != key.as_bytes() {
        return Err(invalid());
    }
    let mut decoder = FrameDecoder::new(reader);
    let mut header = [0; 9 * 4];
    decoder.read_exact(&mut header)?;
    let [
        stored_size,
        flags,
        filled,
        hits,
        filled_misses,
        raised,
        lowered,
        unresolved,
        edge_reach,
    ] = std::array::from_fn(|i| u32::from_le_bytes(header[i * 4..i * 4 + 4].try_into().unwrap()));
    let edge_reach = f32::from_bits(edge_reach);
    if size == 0
        || size > MAX_RESOLUTION
        || size != stored_size
        || flags > 3
        || !edge_reach.is_finite()
        || edge_reach <= 0.
        || projected != (flags & 1 != 0)
        || filled == 0
        || filled > size * size
        || hits
            .checked_add(filled_misses)
            .is_none_or(|total| total > filled)
    {
        return Err(invalid());
    }
    let count = (size * size) as usize;
    let mut values = vec![[0; mesh::COMPONENTS]; count];
    let mut covered = vec![0; count];
    let mut projection = projected.then(|| vec![[0; 5]; count]);
    for stream in [values.as_flattened_mut(), &mut covered] {
        read_chunks(&mut decoder, stream, progress)?;
    }
    if let Some(projection) = &mut projection {
        read_chunks(&mut decoder, projection.as_flattened_mut(), progress)?;
    }
    let mut recipes: Vec<_> = recipes.iter().collect();
    recipes.sort_unstable_by_key(|recipe| recipe.id);
    let mut custom = Vec::with_capacity(recipes.len());
    for recipe in recipes {
        let mut values = vec![0; count];
        read_chunks(&mut decoder, &mut values, progress)?;
        custom.push(custom::Plane {
            recipe: recipe.clone(),
            values: values.into(),
        });
    }
    if decoder.read(&mut [0])? != 0
        || covered.iter().filter(|&&coverage| coverage == 1).count() != filled as usize
        || covered.iter().any(|&coverage| coverage > 3)
    {
        return Err(invalid());
    }
    drop(decoder);
    if let Ok(file) = OpenOptions::new().write(true).open(path) {
        let _ = file.set_times(FileTimes::new().set_modified(SystemTime::now()));
    }
    Ok(Maps {
        size,
        filled: filled as usize,
        values,
        covered,
        projection,
        hits: hits as usize,
        filled_misses: filled_misses as usize,
        gpu_rays: false,
        gpu_projection: false,
        gpu_denoise: false,
        gpu_padding: false,
        selection: super::MapSelection::ALL,
        cage_fit: (flags & 2 != 0).then_some(mesh::FitReport {
            raised: raised as usize,
            lowered: lowered as usize,
            unresolved: unresolved as usize,
        }),
        edge_reach,
        inputs: Default::default(),
        custom,
    })
}

fn read_chunks(reader: &mut impl Read, data: &mut [u8], progress: &Progress) -> io::Result<()> {
    for block in data.chunks_mut(256 * 1024) {
        active(progress)?;
        reader.read_exact(block)?;
    }
    Ok(())
}

struct Staged(PathBuf);

impl Drop for Staged {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn store(path: &Path, key: blake3::Hash, maps: &Maps, progress: &Progress) -> io::Result<()> {
    let directory = path.parent().ok_or_else(invalid)?;
    fs::create_dir_all(directory)?;
    let staged = Staged(directory.join(format!(".{}.tmp", bevy::asset::uuid::Uuid::new_v4())));
    let mut writer = BufWriter::new(
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&staged.0)?,
    );
    writer.write_all(MAGIC)?;
    writer.write_all(key.as_bytes())?;
    let info = FrameInfo::new()
        .content_checksum(true)
        .content_size(Some(36 + maps.bytes() as u64));
    let mut encoder = FrameEncoder::with_frame_info(info, writer);
    let raised = maps.cage_fit.as_ref().map_or(0, |fit| fit.raised);
    let lowered = maps.cage_fit.as_ref().map_or(0, |fit| fit.lowered);
    let unresolved = maps.cage_fit.as_ref().map_or(0, |fit| fit.unresolved);
    let flags = u32::from(maps.projection.is_some()) | (u32::from(maps.cage_fit.is_some()) << 1);
    for value in [
        maps.size,
        flags,
        maps.filled as u32,
        maps.hits as u32,
        maps.filled_misses as u32,
        raised as u32,
        lowered as u32,
        unresolved as u32,
        maps.edge_reach.to_bits(),
    ] {
        encoder.write_all(&value.to_le_bytes())?;
    }
    let projection = maps
        .projection
        .as_deref()
        .unwrap_or_default()
        .as_flattened();
    let mut custom: Vec<_> = maps.custom.iter().collect();
    custom.sort_unstable_by_key(|plane| plane.recipe.id);
    for stream in [maps.values.as_flattened(), &maps.covered, projection]
        .into_iter()
        .chain(custom.into_iter().map(|plane| plane.values.as_slice()))
    {
        for block in stream.chunks(256 * 1024) {
            active(progress)?;
            encoder.write_all(block)?;
        }
    }
    encoder.finish()?.flush()?;
    let _lock = CACHE_WRITE.lock().map_err(|_| io::Error::other("bake cache lock poisoned"))?;
    active(progress)?;
    fs::rename(&staged.0, path)?;
    prune(directory, MAX_BYTES);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::model_mesh::{Corner, Face, Geometry};
    use bevy::prelude::*;

    fn quad() -> Object {
        Object {
            id: 0,
            source_node: None,
            name: "Plane".into(),
            geometry: Geometry {
                positions: vec![Vec3::ZERO, Vec3::X, Vec3::X - Vec3::Z, Vec3::NEG_Z],
                faces: vec![Face {
                    corners: [Vec2::ZERO, Vec2::X, Vec2::ONE, Vec2::Y]
                        .into_iter()
                        .enumerate()
                        .map(|(index, uv)| {
                            let mut corner = Corner::new(index);
                            corner.uv = uv;
                            corner
                        })
                        .collect(),
                    material: 0,
                    smooth: false,
                }],
                ..default()
            }
            .into(),
            transform: Transform::IDENTITY,
            materials: vec![],
            hidden: false,
        }
    }

    #[test]
    fn baked_disk_cache_reopens_and_invalidates_on_inputs_or_corruption() {
        let temp =
            std::env::temp_dir().join(format!("bevy-bake-{}", bevy::asset::uuid::Uuid::new_v4()));
        let directory = temp.join("cache");
        fs::create_dir_all(&temp).unwrap();
        let source = temp.join("plane.obj");
        let write_source = |y: &str| {
            fs::write(
                &source,
                format!("o Plane\nv 0 {y} 0\nv 1 {y} 0\nv 1 {y} -1\nv 0 {y} -1\nf 1 2 3 4\n"),
            )
            .unwrap();
        };
        write_source("0.01");
        let mut right = custom::Recipe::new(custom::Kind::Point, &[]);
        right.id = 20;
        right.point[0] = 80;
        let mut left = right.duplicate(std::slice::from_ref(&right));
        left.id = 4;
        left.point[0] = -80;
        let settings = ProjectionSettings {
            path: Some(source.clone()),
            auto_fit: true,
            custom: vec![right, left].into(),
            ..default()
        };
        let bake = |object, size, settings: &ProjectionSettings| {
            Cache(Some(directory.clone()))
                .bake(vec![object], 0, 0, size, settings, &Progress::default(), None)
                .unwrap()
        };
        let (fresh, cached) = bake(quad(), 16, &settings);
        assert!(!cached);
        assert!(fresh.cage_fit.is_some());
        assert!(fresh.hits > 0);
        let (loaded, cached) = bake(quad(), 16, &settings);
        assert!(cached);
        assert_eq!(fresh.values, loaded.values);
        assert_eq!(fresh.covered, loaded.covered);
        assert_eq!(fresh.projection, loaded.projection);
        for plane in &fresh.custom {
            assert!(loaded.custom.contains(plane));
        }
        assert_ne!(fresh.custom[0].values, fresh.custom[1].values);
        let mut renamed = settings.clone();
        let recipes = std::sync::Arc::make_mut(&mut renamed.custom);
        recipes.swap(0, 1);
        recipes[0].name = "New name".into();
        let (loaded, cached) = bake(quad(), 16, &renamed);
        assert!(cached);
        for plane in &loaded.custom {
            let original = fresh.custom.iter().find(|original| original.recipe.id == plane.recipe.id).unwrap();
            assert_eq!(plane.values, original.values);
        }
        assert_eq!(loaded.custom[0].recipe.name, "New name");
        assert_eq!(fresh.hits, loaded.hits);
        assert_eq!(
            fresh.cage_fit.unwrap().raised,
            loaded.cage_fit.unwrap().raised
        );
        let entry = fs::read_dir(&directory)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let mut damaged = fs::read(&entry).unwrap();
        *damaged.last_mut().unwrap() ^= 0xff;
        fs::write(&entry, damaged).unwrap();
        assert!(!bake(quad(), 16, &settings).1);
        assert!(bake(quad(), 16, &settings).1);
        let minimum = ProjectionSettings { maps: super::super::MapSelection::MINIMUM, ..settings.clone() };
        let (selected, cached) = bake(quad(), 16, &minimum);
        assert!(!cached);
        assert!(!selected.has_map(super::super::BakeMap::Occlusion));
        let (selected_cached, cached) = bake(quad(), 16, &minimum);
        assert!(cached);
        assert_eq!(selected.selection, selected_cached.selection);
        let inputs = selected.generator_inputs();
        let cached_inputs = selected_cached.generator_inputs();
        assert_eq!(inputs.disabled_inputs, cached_inputs.disabled_inputs);
        assert_eq!(inputs.texels, cached_inputs.texels);
        assert!(inputs.custom.iter().all(|plane| cached_inputs.custom.contains(plane)));

        let stamp = fs::metadata(&source).unwrap().modified().unwrap();
        write_source("0.02");
        OpenOptions::new()
            .write(true)
            .open(&source)
            .unwrap()
            .set_modified(stamp)
            .unwrap();
        let (changed, cached) = bake(quad(), 16, &settings);
        assert!(!cached);
        assert_ne!(fresh.projection, changed.projection);
        assert!(!bake(quad(), 32, &settings).1);
        let settings = ProjectionSettings {
            front: settings.front + 1,
            ..settings
        };
        assert!(!bake(quad(), 16, &settings).1);
        let mut shifted = quad();
        shifted.transform.translation.x = 0.1;
        assert!(!bake(shifted, 16, &settings).1);
        let mut changed_uv = quad();
        changed_uv.geometry.faces[0].corners[0].uv.x = 0.1;
        assert!(!bake(changed_uv, 16, &settings).1);

        prune(&directory, 0);
        assert_eq!(fs::read_dir(&directory).unwrap().count(), 0);

        let cache = Cache::at(directory.clone());
        let other_file = directory.join("notes.bake");
        fs::write(&other_file, b"not a cache entry").unwrap();
        assert!(!bake(quad(), 16, &settings).1);
        assert!(bake(quad(), 16, &settings).1);
        assert_eq!(cache.clear().unwrap(), 1);
        assert_eq!(cache.clear().unwrap(), 0);
        assert!(other_file.is_file());
        assert!(!bake(quad(), 16, &settings).1);
        fs::remove_dir_all(&temp).unwrap();
    }
}

fn prune(directory: &Path, limit: u64) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    let mut entries: Vec<_> = entries
        .flatten()
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "bake"))
        .filter_map(|entry| {
            let metadata = entry.metadata().ok()?;
            (metadata.is_file()).then(|| {
                (
                    entry.path(),
                    metadata.len(),
                    metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                )
            })
        })
        .collect();
    entries.sort_unstable_by_key(|entry| std::cmp::Reverse(entry.2));
    let mut retained = 0;
    for (path, bytes, _) in entries {
        if retained + bytes > limit {
            let _ = fs::remove_file(path);
        } else {
            retained += bytes;
        }
    }
}
