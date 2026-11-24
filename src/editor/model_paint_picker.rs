//! Sample the composite material under the pointer without changing texture sets.
use super::*;
use crate::editor::model_document::MeshData;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct MaterialSample {
    pub color: [u8; 3],
    pub roughness: u8,
    pub metallic: u8,
}

enum Sampled {
    Material(MaterialSample),
    Mask(u8),
}

struct Hit {
    set: sets::SetKey,
    uv: Vec2,
    mesh: MeshData,
    offset: Vec2,
}

impl Hit {
    fn new(object: &Object, slot: usize, uv: Vec2) -> Option<Self> {
        (uv.is_finite()
            && uv.cmpge(Vec2::splat(-1.0e-5)).all()
            && uv.cmple(Vec2::splat(1.00001)).all()
            && object.materials.get(slot).is_some())
        .then(|| Self {
            set: (object.id, slot),
            uv: uv.clamp(Vec2::ZERO, Vec2::ONE),
            mesh: object.geometry.clone(),
            offset: object.uv_offset(slot),
        })
    }
}

#[derive(Default)]
pub(super) struct Runtime {
    request: Option<Request>,
    geometry: Option<((usize, GeometryVersion), Arc<editing::input::Bvh>)>,
}

struct Request {
    model: usize,
    binding: sets::SetKey,
    mask: Option<usize>,
    stage: Stage,
}

enum Stage {
    Locate(
        Task<(
            (usize, GeometryVersion),
            Arc<editing::input::Bvh>,
            Option<Hit>,
        )>,
    ),
    Sample(Hit),
    Cpu(Task<Option<Sampled>>),
    Gpu(u64),
    Ready(Result<Sampled, String>),
}

impl Runtime {
    pub fn begin(
        &mut self,
        model: usize,
        doc: &Document,
        session: &Session,
        viewer: &Viewer,
        view: (bool, Rect),
        cursor: Vec2,
    ) {
        let Some(binding) = session.binding else {
            return;
        };
        let (uv_view, rect) = view;
        let visibility = &session.set_appearance.visibility;
        let stage = if uv_view {
            let hit = doc
                .objects
                .iter()
                .find(|object| object.id == binding.0 && !object.hidden)
                .filter(|object| visibility.shown(object.id, binding.1))
                .and_then(|object| Hit::new(object, binding.1, (cursor - rect.min) / rect.size()));
            match hit {
                Some(hit) => Stage::Sample(hit),
                None => Stage::Ready(Err("Выберите точку внутри карты активного набора.".into())),
            }
        } else {
            let stamp = (model, (doc.revision, visibility.revision));
            let cached = self
                .geometry
                .as_ref()
                .filter(|(version, _)| *version == stamp)
                .map(|(_, tree)| tree.clone());
            let objects = doc.objects.clone();
            let (origin, direction) = camera_map(viewer, rect).ray(cursor);
            let visibility = visibility.clone();
            Stage::Locate(AsyncComputeTaskPool::get().spawn(async move {
                let tree = cached.unwrap_or_else(|| {
                    Arc::new(editing::input::Bvh::from_objects_where(
                        &objects,
                        |object, slot| visibility.shown(object, slot),
                    ))
                });
                let hit = surface_hit(&tree, &objects, origin, direction);
                (stamp, tree, hit)
            }))
        };
        self.request = Some(Request {
            model,
            binding,
            stage,
            mask: (session.editing_mask && session.has_mask()).then_some(session.stack.selected),
        });
    }

    pub fn reply(&mut self, id: u64, result: Result<MaterialSample, String>) {
        if let Some(request) = self.request.as_mut()
            && matches!(request.stage, Stage::Gpu(current) if current == id)
        {
            request.stage = Stage::Ready(result.map(Sampled::Material));
        }
    }

    pub fn update(
        &mut self,
        lives: &HashMap<Key, Live>,
        documents: &HashMap<usize, Document>,
        paint: &mut PaintState,
        editor: &mut Editor,
        images: &Assets<Image>,
        materials: &Assets<StandardMaterial>,
        queue: &mut gpu::Queue,
    ) {
        if self
            .geometry
            .as_ref()
            .is_some_and(|((model, _), _)| !documents.contains_key(model))
        {
            self.geometry = None;
        }
        let Some(mut request) = self.request.take() else {
            return;
        };
        let Some(doc) = documents
            .get(&request.model)
            .filter(|doc| doc.mode == Mode::Paint)
        else {
            return;
        };
        let Some(session) = paint.sessions.get_mut(&request.model).filter(|session| {
            editor.viewing_model == Some(request.model)
                && editor.viewing_image.is_none()
                && editor.pending.is_none()
                && session.tool == PaintTool::Picker
                && session.binding == Some(request.binding)
                && !session.showing_bake()
                && !session.graph_canvas_open()
                && session.panel != Panel::Shader
                && request.mask
                    == (session.editing_mask && session.has_mask())
                        .then_some(session.stack.selected)
        }) else {
            return;
        };

        if let Stage::Locate(task) = &mut request.stage {
            let Some((version, tree, hit)) = block_on(poll_once(task)) else {
                self.request = Some(request);
                return;
            };
            self.geometry = Some((version, tree));
            request.stage = match hit {
                Some(hit) => Stage::Sample(hit),
                None => Stage::Ready(Err(
                    "Выберите точку на видимой поверхности с UV-развёрткой.".into(),
                )),
            };
        }
        if let Stage::Sample(hit) = &request.stage {
            if request.mask.is_some() && hit.set != request.binding {
                editor.status = "Выберите точку на активном наборе маски.".into();
                return;
            }
            let (object, slot) = hit.set;
            let Some(object) = doc
                .objects
                .iter()
                .find(|item| {
                    item.id == object
                        && !item.hidden
                        && item.geometry.shares_storage_with(&hit.mesh)
                        && item.uv_offset(slot) == hit.offset
                        && session.set_appearance.visibility.shown(object, slot)
                })
                .filter(|item| item.materials.get(slot).is_some())
            else {
                return;
            };
            let live = lives.get(&(request.model, object.id, slot)).filter(|live| {
                live.source_mesh.shares_storage_with(&object.geometry)
                    && (hit.set == request.binding
                        || live.origin == object.materials[slot]
                        || live
                            .gpu
                            .as_ref()
                            .is_some_and(|gpu| gpu.material == object.materials[slot].material))
            });
            if let Some(live) = live.filter(|live| {
                live.resizing
                    || live.work.is_none() && live.gpu.is_none()
                    || request.mask.is_some() && live.controller.busy()
            }) {
                if live.task.is_some() || live.controller.busy() {
                    self.request = Some(request);
                    return;
                }
            }
            let uv = hit.uv;
            request.stage = match (request.mask, live) {
                (
                    None,
                    Some(Live {
                        gpu: Some(textures),
                        ..
                    }),
                ) => {
                    let id = queue.id();
                    queue.commands.push(gpu::Command::Pick {
                        id,
                        images: [textures.images[0].id(), textures.images[1].id()],
                        uv,
                    });
                    Stage::Gpu(id)
                }
                (
                    Some(mask),
                    Some(Live {
                        work: Some(work), ..
                    }),
                ) => Stage::Ready(
                    sample(&work.surface, uv, Some(mask))
                        .ok_or("На выбранном слое нет маски.".into()),
                ),
                _ => {
                    let source =
                        surface_source(doc, object, slot, images, materials, &mut HashMap::new());
                    let mask = request.mask;
                    Stage::Cpu(
                        AsyncComputeTaskPool::get()
                            .spawn(async move { sample(&source.build(), uv, mask) }),
                    )
                }
            };
        }
        if let Stage::Cpu(task) = &mut request.stage
            && let Some(result) = block_on(poll_once(task))
        {
            request.stage = Stage::Ready(result.ok_or("На выбранном слое нет маски.".into()));
        }
        match request.stage {
            Stage::Ready(Ok(Sampled::Material(sample))) => {
                session.picked = Some(sample.color);
                session.roughness = f32::from(sample.roughness) / 2.55;
                session.metallic = f32::from(sample.metallic) / 2.55;
                let [r, g, b] = sample.color;
                editor.status = format!(
                    "Цвет #{r:02X}{g:02X}{b:02X} · шероховатость {:.0}% · металличность {:.0}%",
                    session.roughness, session.metallic,
                );
                session.tool = PaintTool::Brush;
                session.revision += 1;
            }
            Stage::Ready(Ok(Sampled::Mask(value))) => {
                session.mask_value = value;
                session.tool = PaintTool::Brush;
                session.revision += 1;
            }
            Stage::Ready(Err(error)) => editor.status = error,
            _ => self.request = Some(request),
        }
    }
}

fn surface_hit(
    tree: &editing::input::Bvh,
    objects: &[Object],
    origin: Vec3,
    direction: Vec3,
) -> Option<Hit> {
    let (index, distance) = tree.cast_triangle(origin, direction, |_| true)?;
    let triangle = tree.triangle(index);
    let object = objects.iter().find(|object| object.id == triangle.object)?;
    let face = object.geometry.faces.get(triangle.face)?;
    let indices = object
        .geometry
        .triangles(face)
        .into_iter()
        .find(|indices| {
            indices.map(|i| {
                object
                    .transform
                    .transform_point(object.geometry.positions[face.corners[i].vertex])
            }) == triangle.points
        })?;
    let [a, b, c] = triangle.points;
    let v = origin + direction * distance - a;
    let (ab, ac) = (b - a, c - a);
    let det = ab.length_squared() * ac.length_squared() - ab.dot(ac).powi(2);
    if det <= 0. || !det.is_finite() {
        return None;
    }
    let u = (ac.length_squared() * v.dot(ab) - ab.dot(ac) * v.dot(ac)) / det;
    let w = (ab.length_squared() * v.dot(ac) - ab.dot(ac) * v.dot(ab)) / det;
    let [a, b, c] = indices.map(|i| face.corners[i].uv);
    if (b - a).perp_dot(c - a).abs() <= 1.0e-8 {
        return None;
    }
    let uv = a * (1. - u - w) + b * u + c * w - object.uv_offset(face.material);
    Hit::new(object, face.material, uv)
}

fn sample(surface: &Surface, uv: Vec2, mask: Option<usize>) -> Option<Sampled> {
    let p = (uv * surface.size as f32)
        .as_uvec2()
        .min(UVec2::splat(surface.size - 1));
    if let Some(layer) = mask {
        let opacity = surface.stack.layers.get(layer)?.mask.as_ref()?.effective();
        return Some(Sampled::Mask(
            opacity[((p.y * surface.size + p.x) * 4) as usize],
        ));
    }
    let rect = URect {
        min: p,
        max: p + UVec2::ONE,
    };
    let color = surface.composite_rect(0, rect);
    Some(Sampled::Material(MaterialSample {
        color: [color[0], color[1], color[2]],
        roughness: surface.composite_rect(1, rect)[0],
        metallic: surface.composite_rect(2, rect)[0],
    }))
}
