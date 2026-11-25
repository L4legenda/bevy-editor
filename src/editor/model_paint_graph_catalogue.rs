//! Start page and model graph browser for the material workspace.
use super::*;
use crate::editor::model_document::Document;

#[derive(Component)]
pub(in crate::editor::ui::model_viewer) struct PresetSwatch(pub usize);

#[derive(Default)]
pub(super) struct PresetPreviews {
    previews: Previews,
    outputs: Option<Vec<Option<u64>>>,
}

impl PresetPreviews {
    pub(super) fn prepare(&mut self) {
        if self.outputs.is_some() {
            return;
        }
        let library = material_graph::library();
        let mut gallery = Graph::new("Material presets");
        let outputs: Vec<_> = material_graph::PRESETS.iter().map(|preset| {
            let graph = (preset.build)();
            let output = graph.outputs(&library)[0]?;
            let ids: HashMap<_, _> = graph.nodes.iter().map(|node| {
                (node.id, gallery.add(&node.def, node.pos))
            }).collect();
            for node in graph.nodes {
                let destination = gallery.node_mut(ids[&node.id]).unwrap();
                destination.params = node.params;
                destination.links = node.links.into_iter().map(|(input, from)| (input, ids[&from])).collect();
            }
            Some(ids[&output])
        }).collect();
        self.previews.set_gallery(&gallery, outputs.iter().flatten().copied().collect());
        self.outputs = Some(outputs);
    }

    pub(super) fn update(
        &mut self, assets: &mut Assets<Image>, uploads: &mut Uploads,
        renderer: Option<(&bevy::render::renderer::RenderDevice, &bevy::render::renderer::RenderQueue)>,
    ) -> bool {
        self.previews.update(assets, uploads, renderer)
    }

    pub(super) fn image(&self, index: usize) -> Option<&Handle<Image>> {
        self.previews.image((*self.outputs.as_ref()?.get(index)?)?)
    }
}

pub(super) fn populate(
    commands: &mut Commands,
    root: Entity,
    font: &Handle<Font>,
    page: GraphPage,
    model: usize,
    doc: &Document,
    session: &Session,
    saved: &[graph_shelf::Entry],
    preset_previews: &PresetPreviews,
    images: &mut Assets<Image>,
) {
    commands.entity(root).despawn_related::<Children>();
    if page == GraphPage::Home {
        text(commands, root, font, "Создать материал", 19., TEXT);
        let templates = cards(commands, root);
        for (index, recipe) in material_graph::PRESETS.iter().enumerate() {
            card(commands, templates, font, recipe.name, recipe.description,
                preset_previews.image(index).cloned(), PaintAction::AddGraphMaterial(index));
        }
        let actions = cards(commands, root);
        button(commands, actions, font, "Импорт .mpgraph или ZIP…", PaintAction::ImportGraphFile);
        button(commands, actions, font, "Добавить граф маски к выбранному слою", PaintAction::AddGraphMask);
    }

    text(commands, root, font, "Графы модели", 19., TEXT);
    let mut shown = false;
    for object in &doc.objects {
        for (slot, material) in object.materials.iter().enumerate() {
            let binding = (object.id, slot);
            let stack = if session.binding == Some(binding) {
                &session.stack
            } else if let Some(surface) = &material.paint {
                &surface.stack
            } else {
                continue;
            };
            let mut group = None;
            for (index, layer) in stack.layers.iter().enumerate() {
                for kind in [ProjectGraphKind::Fill, ProjectGraphKind::Mask, ProjectGraphKind::Folder] {
                    let target = ProjectGraphTarget { binding, layer: index, kind };
                    if !target.exists(stack) { continue; }
                    shown = true;
                    let group = *group.get_or_insert_with(|| {
                        text(commands, root, font, &texture_set_label(doc, object, slot), 12., MUTED);
                        cards(commands, root)
                    });
                    let (category, payload, preview) = match kind {
                        ProjectGraphKind::Fill => {
                            let fill = layer.texture_fill.as_ref().unwrap();
                            ("Заливка", fill.graph.as_deref(), fill.images.iter().flatten().next().map(Arc::as_ref))
                        }
                        ProjectGraphKind::Mask => {
                            let mask = layer.mask.as_ref().unwrap().graph.as_ref().unwrap();
                            ("Маска", Some(mask.payload.as_ref()), mask.output.as_deref())
                        }
                        ProjectGraphKind::Folder => ("Группа", layer.folder_graph.as_deref(), None),
                    };
                    let title = payload.and_then(|payload| payload["name"].as_str())
                        .filter(|name| !name.is_empty()).unwrap_or(&layer.name);
                    let description = if title == layer.name { category.into() } else {
                        format!("{category} · {}", layer.name)
                    };
                    card(commands, group, font, title, &description, thumbnail(preview, images),
                        PaintAction::OpenProjectGraph { model, target });
                }
            }
        }
    }
    if !shown {
        text(commands, root, font, "В этой модели ещё нет графов", 12., MUTED);
    }
    if session.pending_project_graph.is_some() {
        text(commands, root, font, "Открытие графа…", 12., MUTED);
    } else if session.smart_loading {
        text(commands, root, font, "Расчёт графа…", 12., MUTED);
    }
    if page == GraphPage::Home && !saved.is_empty() {
        text(commands, root, font, "Сохранённые графы", 19., TEXT);
        let saved_cards = cards(commands, root);
        for (index, entry) in saved.iter().enumerate() {
            card(commands, saved_cards, font, &entry.name, "Добавить как слой заливки",
                thumbnail(entry.thumbnail.as_deref(), images), PaintAction::OpenSavedGraph(index));
        }
    }
}

fn thumbnail(bitmap: Option<&texture_fill::Bitmap>, images: &mut Assets<Image>) -> Option<Handle<Image>> {
    bitmap.map(|bitmap| images.add(shelf_image(bitmap.preview_pixels().clone())))
}

fn cards(commands: &mut Commands, parent: Entity) -> Entity {
    node(commands, parent, Node { flex_wrap: FlexWrap::Wrap, row_gap: px(8), ..row(8.) })
}

fn card(
    commands: &mut Commands, parent: Entity, font: &Handle<Font>, title: &str, description: &str,
    image: Option<Handle<Image>>, action: PaintAction,
) {
    let card = button(commands, parent, font, "", action);
    let is_template = matches!(action, PaintAction::AddGraphMaterial(_));
    let has_preview = is_template || image.is_some();
    commands.entity(card).insert((Node {
        flex_shrink: 0.,
        width: px(184),
        min_height: px(if has_preview { 148 } else { 82 }),
        align_items: AlignItems::Start,
        padding: UiRect::all(px(10)),
        border: UiRect::all(px(1)),
        border_radius: BorderRadius::all(px(7)),
        ..column(5.)
    }, AccessibleLabel(title.to_owned())));
    if has_preview {
        let mut preview = commands.spawn((BackgroundColor(WELL), Node {
            width: px(162), height: px(92), ..default()
        }, Pickable::IGNORE, ChildOf(card)));
        if let PaintAction::AddGraphMaterial(index) = action {
            preview.insert(PresetSwatch(index));
        }
        if let Some(image) = image {
            preview.insert(ImageNode::new(image));
        }
    }
    let title = text(commands, card, font, title, 13., TEXT);
    let detail = text(commands, card, font, description, 11., MUTED);
    for label in [title, detail] {
        commands.entity(label).insert((Pickable::IGNORE, TextLayout::default(), Node { max_width: percent(100), ..default() }));
    }
}
