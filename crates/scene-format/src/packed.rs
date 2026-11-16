//! Little-endian fixed-width tables + interned UTF-8 + lazy component blobs.
//! No unsafe, transmute, native-layout structs or unchecked pointer arithmetic.
use crate::*;

const MAGIC: &[u8; 8] = b"BSCN\r\n\x1a\n";
const HEADER: usize = 128;
const ENTITY: usize = 80;
const RESOURCE: usize = 32;
const NONE: u32 = u32::MAX;
const MAX_PACKED_BYTES: usize = 512 * 1024 * 1024;
const COMPRESSED_MAGIC: &[u8; 8] = b"BSCZ\r\n\x1a\n";

/// Optional transport representation. Raw tables remain available for borrowing
/// and random access; compression always requires a bounded owned buffer.
pub fn compress(bytes: &[u8]) -> Result<Vec<u8>> {
    PackedScene::open(bytes, None)?;
    let compressed = lz4_flex::block::compress(bytes);
    let mut output = Vec::with_capacity(24 + compressed.len());
    output.extend_from_slice(COMPRESSED_MAGIC);
    output.extend_from_slice(&VERSION.to_le_bytes());
    output.extend_from_slice(&number(bytes.len())?.to_le_bytes());
    output.extend_from_slice(&number(compressed.len())?.to_le_bytes());
    output.extend_from_slice(&0u32.to_le_bytes());
    output.extend_from_slice(&compressed);
    Ok(output)
}

/// Decodes only the transport layer. Open the returned bytes with PackedScene
/// to validate its checksum, fingerprint, tables and values before using them.
pub fn decompress(bytes: &[u8]) -> Result<std::borrow::Cow<'_, [u8]>> {
    if bytes.starts_with(MAGIC) {
        return Ok(std::borrow::Cow::Borrowed(bytes));
    }
    ensure(
        bytes.len() >= 24
            && bytes.len() <= MAX_PACKED_BYTES
            && &bytes[..8] == COMPRESSED_MAGIC
            && get32(bytes, 8) == VERSION
            && get32(bytes, 20) == 0,
        "Invalid compressed scene header",
    )?;
    let length = get32(bytes, 12) as usize;
    ensure(
        (HEADER..=MAX_PACKED_BYTES).contains(&length)
            && get32(bytes, 16) as usize == bytes.len() - 24
            && length <= (bytes.len() - 24).saturating_mul(255).saturating_add(65536),
        "Invalid compressed scene size",
    )?;
    let mut output = Vec::new();
    output
        .try_reserve_exact(length)
        .map_err(|_| Error("Cannot allocate decompressed scene".into()))?;
    output.resize(length, 0);
    let written = lz4_flex::block::decompress_into(&bytes[24..], &mut output)
        .map_err(|error| Error(format!("Invalid LZ4 scene: {error}")))?;
    ensure(written == length, "Decompressed scene length mismatch")?;
    Ok(std::borrow::Cow::Owned(output))
}

fn put32(bytes: &mut [u8], at: usize, value: u32) {
    bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
}
fn get32(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
}
fn number(value: usize) -> Result<u32> {
    u32::try_from(value).map_err(|_| Error("Scene section exceeds 4 GiB".into()))
}
fn digest(bytes: &[u8]) -> [u8; 32] {
    let mut hash = blake3::Hasher::new();
    hash.update(&bytes[..56]);
    hash.update(&bytes[88..]);
    *hash.finalize().as_bytes()
}
#[derive(Default)]
struct Pool {
    bytes: Vec<u8>,
    index: HashMap<Vec<u8>, (u32, u32)>,
}
impl Pool {
    fn intern(&mut self, value: &[u8]) -> Result<(u32, u32)> {
        if value.is_empty() {
            return Ok((0, 0));
        }
        if let Some(range) = self.index.get(value) {
            return Ok(*range);
        }
        let range = (number(self.bytes.len())?, number(value.len())?);
        self.bytes.extend_from_slice(value);
        self.index.insert(value.to_vec(), range);
        Ok(range)
    }
    fn field(&mut self, bytes: &mut [u8], at: usize, value: &[u8]) -> Result<()> {
        let (offset, length) = self.intern(value)?;
        put32(bytes, at, offset);
        put32(bytes, at + 4, length);
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
struct Metadata {
    id: String,
    name: String,
    settings: BTreeMap<String, Value>,
    dependencies: Vec<Dependency>,
}

pub fn cook(scene: &ResolvedScene) -> Result<Vec<u8>> {
    scene.validate()?;
    let order = topological_order(&scene.entities, false)?;
    let indices: HashMap<_, _> = order
        .iter()
        .enumerate()
        .map(|(packed, source)| (scene.entities[*source].id.as_str(), packed as u32))
        .collect();
    let resource_indices: HashMap<_, _> = scene
        .resources
        .keys()
        .enumerate()
        .map(|(i, key)| (key.as_str(), i as u32))
        .collect();
    let mut entities = vec![0; ENTITY * order.len()];
    let mut resources = vec![0; RESOURCE * scene.resources.len()];
    let mut strings = Pool::default();
    let mut blobs = Pool::default();
    for (packed_index, source_index) in order.iter().enumerate() {
        let entity = &scene.entities[*source_index];
        let row = &mut entities[packed_index * ENTITY..(packed_index + 1) * ENTITY];
        strings.field(row, 0, entity.id.as_bytes())?;
        strings.field(row, 8, entity.name.as_deref().unwrap_or("").as_bytes())?;
        put32(
            row,
            16,
            entity.parent.as_deref().map_or(NONE, |key| indices[key]),
        );
        put32(
            row,
            20,
            entity
                .model
                .as_deref()
                .map_or(NONE, |key| resource_indices[key]),
        );
        put32(row, 24, entity.order as u32);
        for (i, value) in entity
            .transform
            .translation
            .iter()
            .chain(&entity.transform.rotation)
            .chain(&entity.transform.scale)
            .enumerate()
        {
            put32(row, 32 + i * 4, value.to_bits());
        }
        if !entity.components.is_empty() {
            blobs.field(row, 72, &serde_json::to_vec(&entity.components)?)?;
        }
    }
    for (i, (key, resource)) in scene.resources.iter().enumerate() {
        let row = &mut resources[i * RESOURCE..(i + 1) * RESOURCE];
        strings.field(row, 0, key.as_bytes())?;
        strings.field(row, 8, resource.kind.as_bytes())?;
        strings.field(row, 16, resource.path.as_deref().unwrap_or("").as_bytes())?;
        blobs.field(row, 24, &serde_json::to_vec(resource)?)?;
    }
    let metadata = Metadata {
        id: scene.id.clone(),
        name: scene.name.clone(),
        settings: scene.settings.clone(),
        dependencies: scene.dependencies.clone(),
    };
    let meta_range = blobs.intern(&serde_json::to_vec(&metadata)?)?;
    let total = HEADER
        .checked_add(entities.len())
        .and_then(|v| v.checked_add(resources.len()))
        .and_then(|v| v.checked_add(strings.bytes.len()))
        .and_then(|v| v.checked_add(blobs.bytes.len()))
        .ok_or_else(|| Error("Packed size overflow".into()))?;
    ensure(total <= MAX_PACKED_BYTES, "Packed scene exceeds 512 MiB")?;
    let mut bytes = vec![0; HEADER];
    bytes[..8].copy_from_slice(MAGIC);
    put32(&mut bytes, 8, VERSION);
    put32(&mut bytes, 12, HEADER as u32);
    bytes[16..24].copy_from_slice(&(total as u64).to_le_bytes());
    bytes[24..56].copy_from_slice(&scene.fingerprint()?);
    put32(&mut bytes, 88, number(order.len())?);
    put32(&mut bytes, 92, number(scene.resources.len())?);
    put32(&mut bytes, 96, number(strings.bytes.len())?);
    put32(&mut bytes, 100, number(blobs.bytes.len())?);
    put32(&mut bytes, 104, HEADER as u32);
    put32(&mut bytes, 108, number(HEADER + entities.len())?);
    put32(
        &mut bytes,
        112,
        number(HEADER + entities.len() + resources.len())?,
    );
    put32(
        &mut bytes,
        116,
        number(HEADER + entities.len() + resources.len() + strings.bytes.len())?,
    );
    put32(&mut bytes, 120, meta_range.0);
    put32(&mut bytes, 124, meta_range.1);
    bytes.extend_from_slice(&entities);
    bytes.extend_from_slice(&resources);
    bytes.extend_from_slice(&strings.bytes);
    bytes.extend_from_slice(&blobs.bytes);
    let checksum = digest(&bytes);
    bytes[56..88].copy_from_slice(&checksum);
    Ok(bytes)
}

/// A checked, borrowing view. Common fields need no JSON decode or String clones.
/// Component and inline-resource blobs are decoded only on demand.
#[derive(Debug, Clone, Copy)]
pub struct PackedScene<'a> {
    bytes: &'a [u8],
    entities: &'a [u8],
    resources: &'a [u8],
    strings: &'a [u8],
    blobs: &'a [u8],
}
impl<'a> PackedScene<'a> {
    pub fn open(bytes: &'a [u8], expected_fingerprint: Option<&[u8; 32]>) -> Result<Self> {
        ensure(
            bytes.len() >= HEADER && bytes.len() <= MAX_PACKED_BYTES,
            "Invalid packed scene length",
        )?;
        ensure(
            &bytes[..8] == MAGIC && get32(bytes, 8) == VERSION && get32(bytes, 12) == HEADER as u32,
            "Unsupported packed scene header",
        )?;
        ensure(
            u64::from_le_bytes(bytes[16..24].try_into().unwrap()) == bytes.len() as u64,
            "Packed scene length mismatch",
        )?;
        ensure(
            bytes[56..88] == digest(bytes),
            "Packed scene checksum mismatch",
        )?;
        if let Some(expected) = expected_fingerprint {
            ensure(bytes[24..56] == *expected, "Stale packed scene fingerprint")?;
        }
        let count = get32(bytes, 88) as usize;
        let resource_count = get32(bytes, 92) as usize;
        ensure(
            count <= MAX_ENTITIES && resource_count <= MAX_RESOURCES,
            "Packed scene count limit exceeded",
        )?;
        let entities_end = HEADER
            .checked_add(count * ENTITY)
            .ok_or_else(|| Error("Entity section overflow".into()))?;
        let resources_end = entities_end
            .checked_add(resource_count * RESOURCE)
            .ok_or_else(|| Error("Resource section overflow".into()))?;
        let strings_end = resources_end
            .checked_add(get32(bytes, 96) as usize)
            .ok_or_else(|| Error("String section overflow".into()))?;
        let blobs_end = strings_end
            .checked_add(get32(bytes, 100) as usize)
            .ok_or_else(|| Error("Blob section overflow".into()))?;
        ensure(
            blobs_end == bytes.len()
                && get32(bytes, 104) as usize == HEADER
                && get32(bytes, 108) as usize == entities_end
                && get32(bytes, 112) as usize == resources_end
                && get32(bytes, 116) as usize == strings_end,
            "Invalid or overlapping packed sections",
        )?;
        let scene = Self {
            bytes,
            entities: &bytes[HEADER..entities_end],
            resources: &bytes[entities_end..resources_end],
            strings: &bytes[resources_end..strings_end],
            blobs: &bytes[strings_end..blobs_end],
        };
        scene.blob(get32(bytes, 120), get32(bytes, 124))?;
        let mut ids = HashSet::with_capacity(count);
        for i in 0..count {
            let entity = scene.entity(i).unwrap();
            scene.text(get32(entity.row, 0), get32(entity.row, 4))?;
            scene.text(get32(entity.row, 8), get32(entity.row, 12))?;
            scene.blob(get32(entity.row, 72), get32(entity.row, 76))?;
            ensure(
                identifier(entity.id(), true) && ids.insert(entity.id()),
                "Invalid or duplicate packed entity ID",
            )?;
            ensure(
                entity.parent().is_none_or(|parent| parent < i)
                    && entity
                        .model()
                        .is_none_or(|resource| resource < resource_count),
                "Invalid packed parent/model index",
            )?;
            ensure(get32(entity.row, 28) == 0, "Nonzero reserved entity field")?;
            entity.transform().validate()?;
        }
        let mut resource_ids = HashSet::with_capacity(resource_count);
        for i in 0..resource_count {
            let resource = scene.resource(i).unwrap();
            for at in [0, 8, 16] {
                scene.text(get32(resource.row, at), get32(resource.row, at + 4))?;
            }
            scene.blob(get32(resource.row, 24), get32(resource.row, 28))?;
            ensure(
                identifier(resource.key(), true)
                    && resource_ids.insert(resource.key())
                    && identifier(resource.kind(), false)
                    && (resource.path().is_empty() || asset_path(resource.path())),
                "Invalid packed resource ID/type/path",
            )?;
        }
        Ok(scene)
    }
    fn range(pool: &'a [u8], offset: u32, length: u32) -> Result<&'a [u8]> {
        let start = offset as usize;
        let end = start
            .checked_add(length as usize)
            .ok_or_else(|| Error("Packed range overflow".into()))?;
        pool.get(start..end)
            .ok_or_else(|| Error("Packed range outside its section".into()))
    }
    fn text(&self, offset: u32, length: u32) -> Result<&'a str> {
        std::str::from_utf8(Self::range(self.strings, offset, length)?)
            .map_err(|_| Error("Invalid packed UTF-8".into()))
    }
    fn blob(&self, offset: u32, length: u32) -> Result<&'a [u8]> {
        Self::range(self.blobs, offset, length)
    }
    pub fn entity_count(&self) -> usize {
        self.entities.len() / ENTITY
    }
    pub fn resource_count(&self) -> usize {
        self.resources.len() / RESOURCE
    }
    pub fn fingerprint(&self) -> [u8; 32] {
        self.bytes[24..56].try_into().unwrap()
    }
    pub fn entity(&self, index: usize) -> Option<EntityView<'a>> {
        let start = index.checked_mul(ENTITY)?;
        Some(EntityView {
            scene: *self,
            row: self.entities.get(start..start.checked_add(ENTITY)?)?,
        })
    }
    pub fn resource(&self, index: usize) -> Option<ResourceView<'a>> {
        let start = index.checked_mul(RESOURCE)?;
        Some(ResourceView {
            scene: *self,
            row: self.resources.get(start..start.checked_add(RESOURCE)?)?,
        })
    }
    pub fn unpack(&self) -> Result<ResolvedScene> {
        let metadata: Metadata =
            serde_json::from_slice(self.blob(get32(self.bytes, 120), get32(self.bytes, 124))?)?;
        let resources: BTreeMap<_, _> = (0..self.resource_count())
            .map(|i| {
                let resource = self.resource(i).unwrap();
                Ok((resource.key().to_owned(), resource.decode()?))
            })
            .collect::<Result<_>>()?;
        let entities = (0..self.entity_count())
            .map(|i| {
                let view = self.entity(i).unwrap();
                Ok(Entity {
                    id: view.id().into(),
                    name: (!view.name().is_empty()).then(|| view.name().into()),
                    parent: view.parent().map(|i| self.entity(i).unwrap().id().into()),
                    order: view.order(),
                    transform: view.transform(),
                    model: view.model().map(|i| self.resource(i).unwrap().key().into()),
                    instance: None,
                    components: view.components()?,
                })
            })
            .collect::<Result<_>>()?;
        let resolved = ResolvedScene {
            id: metadata.id,
            name: metadata.name,
            settings: metadata.settings,
            resources,
            entities,
            dependencies: metadata.dependencies,
        };
        resolved.validate()?;
        Ok(resolved)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct EntityView<'a> {
    scene: PackedScene<'a>,
    row: &'a [u8],
}
impl<'a> EntityView<'a> {
    pub fn id(&self) -> &'a str {
        self.scene
            .text(get32(self.row, 0), get32(self.row, 4))
            .unwrap()
    }
    pub fn name(&self) -> &'a str {
        self.scene
            .text(get32(self.row, 8), get32(self.row, 12))
            .unwrap()
    }
    pub fn parent(&self) -> Option<usize> {
        let v = get32(self.row, 16);
        (v != NONE).then_some(v as usize)
    }
    pub fn model(&self) -> Option<usize> {
        let v = get32(self.row, 20);
        (v != NONE).then_some(v as usize)
    }
    pub fn order(&self) -> i32 {
        get32(self.row, 24) as i32
    }
    pub fn transform(&self) -> Transform {
        let values: [f32; 10] =
            std::array::from_fn(|i| f32::from_bits(get32(self.row, 32 + 4 * i)));
        Transform {
            translation: values[..3].try_into().unwrap(),
            rotation: values[3..7].try_into().unwrap(),
            scale: values[7..].try_into().unwrap(),
        }
    }
    pub fn component_bytes(&self) -> &'a [u8] {
        self.scene
            .blob(get32(self.row, 72), get32(self.row, 76))
            .unwrap()
    }
    pub fn components(&self) -> Result<BTreeMap<String, Value>> {
        if self.component_bytes().is_empty() {
            Ok(BTreeMap::new())
        } else {
            Ok(serde_json::from_slice(self.component_bytes())?)
        }
    }
}
#[derive(Debug, Clone, Copy)]
pub struct ResourceView<'a> {
    scene: PackedScene<'a>,
    row: &'a [u8],
}
impl<'a> ResourceView<'a> {
    pub fn key(&self) -> &'a str {
        self.scene
            .text(get32(self.row, 0), get32(self.row, 4))
            .unwrap()
    }
    pub fn kind(&self) -> &'a str {
        self.scene
            .text(get32(self.row, 8), get32(self.row, 12))
            .unwrap()
    }
    pub fn path(&self) -> &'a str {
        self.scene
            .text(get32(self.row, 16), get32(self.row, 20))
            .unwrap()
    }
    pub fn decode(&self) -> Result<Resource> {
        Ok(serde_json::from_slice(
            self.scene.blob(get32(self.row, 24), get32(self.row, 28))?,
        )?)
    }
}
