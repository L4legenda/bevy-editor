use crate::*;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dependency {
    pub scene_id: String,
    pub digest: String,
}

/// Implementations choose UID lookup/path fallback and sandbox filesystem reads.
/// Loading is data-only. No user script or engine object is executed here.
pub trait SceneLoader {
    fn load_scene(&mut self, resource: &Resource) -> Result<Scene>;
}
impl<F: FnMut(&Resource) -> Result<Scene>> SceneLoader for F {
    fn load_scene(&mut self, resource: &Resource) -> Result<Scene> {
        self(resource)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedScene {
    pub id: String,
    pub name: String,
    pub settings: BTreeMap<String, Value>,
    pub resources: BTreeMap<String, Resource>,
    pub entities: Vec<Entity>,
    pub dependencies: Vec<Dependency>,
}
impl ResolvedScene {
    fn discard_unused_resources(&mut self) {
        fn references(value: &Value, pending: &mut Vec<String>) {
            match value {
                Value::Object(values) => {
                    if let Some(key) = values.get("$resource").and_then(Value::as_str) {
                        pending.push(key.to_owned());
                    } else {
                        for value in values.values() {
                            references(value, pending);
                        }
                    }
                }
                Value::Array(values) => {
                    for value in values {
                        references(value, pending);
                    }
                }
                _ => {}
            }
        }
        let mut pending: Vec<_> = self
            .entities
            .iter()
            .filter_map(|entity| entity.model.clone())
            .collect();
        for value in self.settings.values().chain(
            self.entities
                .iter()
                .flat_map(|entity| entity.components.values()),
        ) {
            references(value, &mut pending);
        }
        let mut used = HashSet::new();
        while let Some(key) = pending.pop() {
            if used.insert(key.clone())
                && let Some(data) = self
                    .resources
                    .get(&key)
                    .and_then(|resource| resource.data.as_ref())
            {
                references(data, &mut pending);
            }
        }
        self.resources.retain(|key, _| used.contains(key));
    }
    pub fn validate(&self) -> Result<()> {
        let ids = validate_parts(&self.resources, &self.entities, true)?;
        ensure(
            uuid::Uuid::parse_str(&self.id).is_ok_and(|id| !id.is_nil()) && label(&self.name),
            "Invalid resolved scene identity",
        )?;
        for value in self.settings.values() {
            crate::validate_value(value, &ids, &self.resources, true, 0)?;
        }
        Ok(())
    }
    pub fn fingerprint(&self) -> Result<[u8; 32]> {
        let mut hash = blake3::Hasher::new();
        hash.update(b"bevy-scene-cooker-v1\0");
        let mut entities: Vec<_> = self.entities.iter().collect();
        entities.sort_unstable_by(|a, b| a.id.cmp(&b.id));
        hash.update(&serde_json::to_vec(&(
            &self.id,
            &self.name,
            &self.settings,
            &self.resources,
            entities,
            &self.dependencies,
        ))?);
        Ok(*hash.finalize().as_bytes())
    }
}

fn scoped(prefix: &str, key: &str) -> String {
    format!("{prefix}{key}")
}
fn remap(value: &mut Value, prefix: &str) {
    match value {
        Value::Object(fields) => {
            if let Some(key) = fields.get_mut("$entity") {
                if let Some(text) = key.as_str() {
                    *key = Value::String(scoped(prefix, text));
                }
            } else if let Some(key) = fields.get_mut("$resource") {
                if let Some(text) = key.as_str() {
                    *key = Value::String(scoped(prefix, text));
                }
            } else {
                for value in fields.values_mut() {
                    remap(value, prefix);
                }
            }
        }
        Value::Array(values) => {
            for value in values {
                remap(value, prefix);
            }
        }
        _ => {}
    }
}

struct PendingPatch {
    target: String,
    context: String,
    patch: EntityPatch,
}
#[derive(Debug, Clone, Copy)]
pub struct ResolveLimits {
    pub entities: usize,
    pub resources: usize,
    pub depth: usize,
    pub expanded_bytes: usize,
}
impl Default for ResolveLimits {
    fn default() -> Self {
        Self {
            entities: MAX_ENTITIES,
            resources: MAX_RESOURCES,
            depth: MAX_INSTANCE_DEPTH,
            expanded_bytes: 128 * 1024 * 1024,
        }
    }
}
struct Resolver<'a> {
    loader: &'a mut dyn SceneLoader,
    cache: HashMap<String, std::sync::Arc<Scene>>,
    identities: BTreeMap<String, String>,
    source_sizes: HashMap<String, usize>,
    expanded_bytes: usize,
    ancestors: Vec<String>,
    entities: Vec<Entity>,
    resources: BTreeMap<String, Resource>,
    patches: Vec<PendingPatch>,
    limits: ResolveLimits,
}
impl Resolver<'_> {
    fn register(&mut self, scene: &Scene) -> Result<()> {
        let source = scene.to_source()?;
        let digest = blake3::hash(&source).to_hex().to_string();
        if let Some(previous) = self.identities.insert(scene.id.clone(), digest.clone()) {
            ensure(
                previous == digest,
                format!("Different scenes share UUID {}", scene.id),
            )?;
        }
        self.source_sizes.insert(scene.id.clone(), source.len());
        Ok(())
    }
    fn append(&mut self, scene: &Scene, prefix: &str, mount: Option<&str>) -> Result<()> {
        ensure(
            self.ancestors.len() < self.limits.depth.min(MAX_INSTANCE_DEPTH),
            "Nested scene depth limit exceeded",
        )?;
        ensure(
            !self.ancestors.contains(&scene.id),
            format!("Cyclic scene instance at {}", scene.name),
        )?;
        ensure(
            self.entities.len().saturating_add(scene.entities.len())
                <= self.limits.entities.min(MAX_ENTITIES)
                && self.resources.len().saturating_add(scene.resources.len())
                    <= self.limits.resources.min(MAX_RESOURCES),
            "Expanded scene exceeds entity/resource limit",
        )?;
        self.expanded_bytes = self
            .expanded_bytes
            .saturating_add(self.source_sizes[&scene.id]);
        ensure(
            self.expanded_bytes <= self.limits.expanded_bytes.min(512 * 1024 * 1024),
            "Expanded scene data budget exceeded",
        )?;
        self.ancestors.push(scene.id.clone());
        for (id, resource) in &scene.resources {
            let mut resource = resource.clone();
            // A shared inline material keeps one identity across instances;
            // its scoped alias is still distinct from the local-copy policy.
            if resource.data.is_some() && resource.uid.is_none() {
                resource.uid = Some(format!("{}:{id}", scene.id));
            }
            if let Some(data) = &mut resource.data {
                remap(data, prefix);
            }
            self.resources.insert(scoped(prefix, id), resource);
        }
        for entity in &scene.entities {
            let mut copy = entity.clone();
            copy.id = scoped(prefix, &entity.id);
            copy.parent = entity
                .parent
                .as_ref()
                .map(|key| scoped(prefix, key))
                .or_else(|| mount.map(str::to_owned));
            copy.model = entity.model.as_ref().map(|key| scoped(prefix, key));
            copy.instance = None;
            for value in copy.components.values_mut() {
                remap(value, prefix);
            }
            let mount_id = copy.id.clone();
            self.entities.push(copy);
            if let Some(instance) = &entity.instance {
                let resource = &scene.resources[&instance.resource];
                let key = format!(
                    "{}\0{}",
                    resource.uid.as_deref().unwrap_or(""),
                    resource.path.as_deref().unwrap_or("")
                );
                let child = if let Some(scene) = self.cache.get(&key) {
                    std::sync::Arc::clone(scene)
                } else {
                    let scene = std::sync::Arc::new(self.loader.load_scene(resource)?);
                    self.register(&scene)?;
                    self.cache.insert(key, scene.clone());
                    scene
                };
                self.append(&child, &format!("{mount_id}/"), Some(&mount_id))?;
                for (target, patch) in &instance.overrides {
                    self.patches.push(PendingPatch {
                        target: format!("{mount_id}/{target}"),
                        context: prefix.into(),
                        patch: patch.clone(),
                    });
                }
            }
        }
        self.ancestors.pop();
        Ok(())
    }

    fn apply_patches(&mut self) -> Result<()> {
        if self.patches.is_empty() {
            return Ok(());
        }
        let index: HashMap<_, _> = self
            .entities
            .iter()
            .enumerate()
            .map(|(i, e)| (e.id.clone(), i))
            .collect();
        let mut removed = HashSet::new();
        let mut edited = HashSet::new();
        for pending in &self.patches {
            let i = *index.get(&pending.target).ok_or_else(|| {
                Error(format!(
                    "Override target no longer exists: {}",
                    pending.target
                ))
            })?;
            let entity = &mut self.entities[i];
            let patch = &pending.patch;
            if patch.remove {
                removed.insert(i);
                continue;
            }
            edited.insert(i);
            if let Some(name) = &patch.name {
                entity.name = Some(name.clone());
            }
            if let Some(transform) = patch.transform {
                entity.transform = transform;
            }
            if let Some(parent) = &patch.parent {
                entity.parent = Some(scoped(&pending.context, parent));
            } else if patch.detach {
                entity.parent = pending.context.strip_suffix('/').map(str::to_owned);
            }
            if let Some(order) = patch.order {
                entity.order = order;
            }
            if patch.remove_model {
                entity.model = None;
            }
            if let Some(model) = &patch.model {
                entity.model = Some(scoped(&pending.context, model));
            }
            for kind in &patch.remove_components {
                ensure(
                    entity.components.remove(kind).is_some(),
                    format!("Cannot remove missing component {kind} from {}", entity.id),
                )?;
            }
            for (kind, value) in &patch.components {
                let mut value = value.clone();
                remap(&mut value, &pending.context);
                entity.components.insert(kind.clone(), value);
            }
        }
        if removed.is_empty() {
            return Ok(());
        }
        // Check the effective links before removal can hide structural conflicts.
        if self
            .patches
            .iter()
            .any(|p| p.patch.detach || p.patch.parent.is_some())
        {
            topological_order(&self.entities, false)?;
        }
        let mut children = vec![Vec::new(); self.entities.len()];
        for (i, entity) in self.entities.iter().enumerate() {
            if let Some(parent) = &entity.parent {
                children[*index
                    .get(parent)
                    .ok_or_else(|| Error(format!("Missing parent {parent}")))?]
                .push(i);
            }
            if let Some((mount, _)) = entity.id.rsplit_once('/')
                && entity.parent.as_deref() != Some(mount)
                && let Some(&owner) = index.get(mount)
            {
                children[owner].push(i);
            }
        }
        let mut pending: Vec<_> = removed.iter().copied().collect();
        while let Some(i) = pending.pop() {
            for child in &children[i] {
                if removed.insert(*child) {
                    pending.push(*child);
                }
            }
        }
        ensure(
            removed.is_disjoint(&edited),
            "An override edits an entity deleted by another override",
        )?;
        let mut i = 0;
        self.entities.retain(|_| {
            let keep = !removed.contains(&i);
            i += 1;
            keep
        });
        Ok(())
    }
}

pub fn resolve(scene: &Scene, loader: &mut dyn SceneLoader) -> Result<ResolvedScene> {
    resolve_with_limits(scene, loader, ResolveLimits::default())
}
pub fn resolve_with_limits(
    scene: &Scene,
    loader: &mut dyn SceneLoader,
    limits: ResolveLimits,
) -> Result<ResolvedScene> {
    let mut resolver = Resolver {
        loader,
        cache: HashMap::new(),
        identities: BTreeMap::new(),
        source_sizes: HashMap::new(),
        expanded_bytes: 0,
        ancestors: Vec::new(),
        entities: Vec::new(),
        resources: BTreeMap::new(),
        patches: Vec::new(),
        limits,
    };
    resolver.register(scene)?;
    resolver.append(scene, "", None)?;
    resolver.apply_patches()?;
    let mut resolved = ResolvedScene {
        id: scene.id.clone(),
        name: scene.name.clone(),
        settings: scene.settings.clone(),
        resources: resolver.resources,
        entities: resolver.entities,
        dependencies: resolver
            .identities
            .into_iter()
            .map(|(scene_id, digest)| Dependency { scene_id, digest })
            .collect(),
    };
    resolved.validate()?;
    resolved.discard_unused_resources();
    Ok(resolved)
}
