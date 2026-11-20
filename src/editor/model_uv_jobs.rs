use super::*;

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
                    .map(|(fi, c)| c.map(|i| object.geometry.faces[*fi].corners[i].uv.to_array()))
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
        task: IoTaskPool::get().spawn(async move { backend::run(request, worker_progress) }),
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
                uv.status = "Предпросмотр готов".into();
                Proposed::Layout(topology::apply(&job.original, &job.corners, output)?.into())
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
