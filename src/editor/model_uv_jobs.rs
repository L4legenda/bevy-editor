use super::*;
use std::collections::HashMap;

pub(super) fn run_layout(
    request: backend::Request,
    sets: Vec<Option<(usize, u32)>>,
    progress: Arc<backend::Progress>,
) -> Result<backend::Output, String> {
    if request.options.method == backend::Method::Suggest || sets.iter().all(Option::is_none) {
        return backend::run(request, progress);
    }
    let mut groups = BTreeMap::<_, Vec<usize>>::new();
    for (triangle, set) in sets.into_iter().enumerate() {
        groups.entry(set).or_default().push(triangle);
    }
    let total = groups.len();
    let mut layout = backend::Output {
        suggestion: Vec::new(),
        uv: vec![[[0.; 2]; 3]; request.triangles.len()],
        charts: 0,
        utilization: 0.,
    };
    let protected: BTreeSet<_> = request.options.protected.iter().copied().collect();
    for (batch, (set, indices)) in groups.into_iter().enumerate() {
        let mut positions = Vec::new();
        let mut vertices = HashMap::new();
        let triangles = indices.iter().map(|&index| {
            request.triangles[index].map(|vertex| *vertices.entry(vertex).or_insert_with(|| {
                let id = positions.len();
                positions.push(request.positions[vertex]);
                id
            }))
        }).collect();
        let options = backend::Options {
            materials: if request.options.materials.is_empty() {
                Vec::new()
            } else {
                indices.iter().map(|&index| request.options.materials[index]).collect()
            },
            protected: indices.iter().enumerate()
                .filter_map(|(local, index)| protected.contains(index).then_some(local))
                .collect(),
            method: request.options.method,
            guide_seams: request.options.guide_seams,
            seed: None,
        };
        let group = backend::Request {
            positions,
            triangles,
            threshold: request.threshold,
            resolution: request.resolution,
            padding: request.padding,
            existing_uv: request.existing_uv.as_ref()
                .map(|uv| indices.iter().map(|&index| uv[index]).collect()),
            options,
        };
        progress.set_tile(batch, total, set.map(|(_, tile)| tile));
        let result = backend::run(group, progress.clone())?;
        for (index, uv) in indices.into_iter().zip(result.uv) {
            layout.uv[index] = uv;
        }
        layout.charts += result.charts;
        layout.utilization += result.utilization / total as f32;
    }
    Ok(layout)
}

pub(super) fn start(
    uv: &mut UvState,
    index: usize,
    doc: &Document,
    action: UvAction,
    seed_face: Option<usize>,
) {
    let Some(object) = doc.active() else { return };
    let result = topology::input(
        &object.geometry,
        [1.1, 1.25, 1.5][uv.quality],
        uv.resolution,
        uv.padding,
    );
    let mut input = match result {
        Ok(input) => input,
        Err(e) => {
            uv.error = e;
            return;
        }
    };
    let options = &mut input.request.options;
    match action {
        UvAction::Pack => {
            input.request.existing_uv = Some(
                input
                    .corners
                    .iter()
                    .map(|(fi, corners)| {
                        let face = &object.geometry.faces[*fi];
                        let offset = object.uv_offset(face.material);
                        corners.map(|i| (face.corners[i].uv - offset).to_array())
                    })
                    .collect(),
            )
        }
        UvAction::Suggest => {
            options.method = backend::Method::Suggest;
            options.seed = input
                .corners
                .iter()
                .position(|(fi, _)| Some(*fi) == seed_face);
            if options.seed.is_none() {
                uv.error = "Выберите грань модели".into();
                return;
            }
        }
        UvAction::Preview | UvAction::Unwrap => {
            if action == UvAction::Preview {
                options.method = backend::Method::Preview;
            }
            options.guide_seams = true;
            options.materials = input
                .corners
                .iter()
                .map(|(fi, _)| object.geometry.faces[*fi].material)
                .collect();
            if let Some(mask) = uv.masks.get(&(doc.source.path.clone(), object.id)) {
                options.protected = input
                    .request
                    .triangles
                    .iter()
                    .enumerate()
                    .filter_map(|(i, f)| mask.contains(&face_key(*f)).then_some(i))
                    .collect();
            }
        }
        _ => return,
    }
    if matches!(action, UvAction::Preview | UvAction::Unwrap) {
        input.cut_manual_seams(&object.geometry.uv_seams);
    }
    let progress = Arc::new(backend::Progress::default());
    let worker_progress = progress.clone();
    let sets = input.corners.iter().map(|(face, _)| {
        let slot = object.geometry.faces[*face].material;
        object.materials.get(slot)?.udim().map(|tile| (slot, tile.number()))
    }).collect();
    let request = input.request;
    uv.job = Some(Job {
        source: doc.source.path.clone(),
        action,
        index,
        object: object.id,
        original: object.geometry.clone(),
        settings: (request.resolution, request.padding),
        corners: input.corners,
        progress,
        task: IoTaskPool::get().spawn(async move { run_layout(request, sets, worker_progress) }),
    });
    uv.elapsed = 0.;
    uv.progress = 0.;
    uv.status = "Подготовка поверхности…".into();
    uv.error.clear();
    uv.proposal = None;
    uv.comparing = false;
    uv.drag = None;
    uv.tool = Tool::Select;
    uv.brush_at = None;
}

fn visible_source(
    editor: &Editor,
    state: &State,
    index: usize,
    id: usize,
    original: &document::MeshData,
    source: &std::path::Path,
) -> bool {
    editor.viewing_model == Some(index)
        && editor.viewing_image.is_none()
        && editor.pending.is_none()
        && state
            .documents
            .get(&index)
            .filter(|d| d.mode == Mode::Uv && d.source.path == source)
            .and_then(Document::active)
            .is_some_and(|o| o.id == id && !o.hidden && &o.geometry == original)
}

pub(super) fn poll_job(uv: &mut UvState, state: &mut State, editor: &Editor) {
    if uv
        .proposal
        .as_ref()
        .is_some_and(|p| !visible_source(editor, state, p.index, p.object, &p.original, &p.source))
    {
        uv.proposal = None;
        uv.comparing = false;
        uv.stamp = None;
        uv.overlay_revision += 1;
        state.rendered = None;
    }
    if uv.job.as_ref().is_some_and(|j| {
        if matches!(j.action, UvAction::Suggest | UvAction::Preview) {
            return !visible_source(editor, state, j.index, j.object, &j.original, &j.source);
        }
        // A committed operation can finish on its source document in the background.
        !state.documents.get(&j.index).is_some_and(|d| {
            d.source.path == j.source
                && d.objects
                    .iter()
                    .any(|o| o.id == j.object && o.geometry == j.original)
        })
    }) {
        uv.job.take();
        uv.status.clear();
        uv.progress = 0.;
    }
    uv.masks.retain(|(path, id), _| {
        state
            .documents
            .values()
            .any(|d| &d.source.path == path && d.objects.iter().any(|o| o.id == *id))
    });
    let Some(result) = uv
        .job
        .as_mut()
        .and_then(|j| block_on(poll_once(&mut j.task)))
    else {
        return;
    };
    let job = uv.job.take().unwrap();
    match result.and_then(|output| finish(uv, state, &job, &output)) {
        Ok(()) => {
            uv.progress = 1.;
            uv.error.clear();
        }
        Err(e) => {
            uv.error = e;
            uv.status.clear();
            uv.progress = 0.;
        }
    }
    uv.comparing = false;
    uv.stamp = None;
    uv.overlay_revision += 1;
    state.rendered = None;
}

fn finish(
    uv: &mut UvState,
    state: &mut State,
    job: &Job,
    output: &backend::Output,
) -> Result<(), String> {
    let settings = job.settings;
    match job.action {
        UvAction::Suggest | UvAction::Preview => {
            let result = if job.action == UvAction::Suggest {
                let mask = output
                    .suggestion
                    .iter()
                    .map(|&i| {
                        let (fi, indices) = job.corners[i];
                        face_key(indices.map(|ci| job.original.faces[fi].corners[ci].vertex))
                    })
                    .collect();
                uv.status = "DA Wand предложил область".into();
                Proposed::Island(mask)
            } else {
                let object = state.documents.get(&job.index).and_then(Document::active)
                    .ok_or("Объект удалён")?;
                uv.status = "Предпросмотр готов".into();
                Proposed::Layout(apply_layout(object, &job.corners, output)?)
            };
            uv.proposal = Some(Proposal {
                source: job.source.clone(),
                index: job.index,
                object: job.object,
                original: job.original.clone(),
                result,
                settings,
            });
        }
        _ => {
            // A previously generated atlas is a valid incumbent only for the same
            // geometry and padding/resolution. Imported and manually edited UVs still repack.
            if job.action == UvAction::Pack
                && uv.layout_settings == Some(settings)
                && uv.before.as_ref().is_some_and(|b| {
                    b.index == job.index && b.object == job.object && b.after == job.original
                })
            {
                let area = |uv: [Vec2; 3]| {
                    let [a, b, c] = uv.map(|p| p.as_dvec2());
                    (b - a).perp_dot(c - a).abs() * 0.5
                };
                let previous: f64 = job
                    .corners
                    .iter()
                    .map(|(fi, tri)| area(tri.map(|c| job.original.faces[*fi].corners[c].uv)))
                    .sum();
                let packed: f64 = output
                    .uv
                    .iter()
                    .map(|t| area(t.map(Vec2::from_array)))
                    .sum();
                if packed <= previous + 1e-8 {
                    uv.status = "Текущая упаковка уже плотнее или равна найденной".into();
                    return Ok(());
                }
            }
            let (before, after) = apply_job(
                state,
                job.index,
                job.object,
                &job.original,
                &job.corners,
                output,
            )?;
            uv.before = Some(Comparison {
                index: job.index,
                object: job.object,
                before,
                after,
            });
            uv.layout_settings = Some(settings);
            uv.status = if job.action == UvAction::Pack {
                "Упаковка готова"
            } else {
                "Развёртка готова"
            }
            .into();
            uv.reveal = 1.;
        }
    }
    Ok(())
}

pub(super) fn accept(uv: &mut UvState, state: &mut State, index: usize) {
    let Some(p) = uv.proposal.take() else {
        return;
    };
    let Some(doc) = state.documents.get(&index) else {
        return;
    };
    if p.index != index
        || doc.source.path != p.source
        || !doc
            .active()
            .is_some_and(|o| o.id == p.object && o.geometry == p.original)
    {
        uv.error = "Объект изменился. Постройте предложение снова.".into();
        return;
    }
    match p.result {
        Proposed::Layout(after) => {
            match commit_layout(state, index, p.object, &p.original, after) {
                Ok((before, after)) => {
                    uv.before = Some(Comparison {
                        index,
                        object: p.object,
                        before,
                        after,
                    });
                    uv.layout_settings = Some(p.settings);
                    uv.status = "Предпросмотр применён".into();
                    uv.reveal = 1.;
                }
                Err(e) => uv.error = e,
            }
        }
        Proposed::Island(mask) => {
            uv.masks
                .entry((doc.source.path.clone(), p.object))
                .or_default()
                .extend(mask);
            uv.status = "Область принята · запустите авторазвёртку".into();
        }
    }
    uv.comparing = false;
    uv.overlay_revision += 1;
    uv.stamp = None;
    state.rendered = None;
}
