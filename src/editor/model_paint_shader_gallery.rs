//! Shader Maker library thumbnails rendered by the material graph runtime.
use super::*;
use crate::editor::model_document::Document;
use material_graph::{mesh_maps, previews, surface};
use material_painter_graph::{Kind, ParamKind, Value, def::NodeDef};
use std::collections::BTreeMap;

#[derive(Component)]
pub(in crate::editor::ui::model_viewer) struct Swatch(pub u64);

pub(super) struct Gallery {
    pub(super) previews: previews::Previews,
    graph: Graph,
    roots: Vec<u64>,
    library: Arc<Library>,
    uses_maps: bool,
    uses_surface: bool,
    maps: Option<Arc<bake::Maps>>,
    surface: Option<Arc<surface::Source>>,
}

impl Gallery {
    pub(super) fn new(
        commands: &mut Commands,
        parent: Entity,
        home: Entity,
        font: &Handle<Font>,
        directory: Option<&Path>,
    ) -> Self {
        let (library, archived) = node_library::palette();
        let mut definitions = Library {
            defs: library
                .defs
                .iter()
                .filter(|def| node_library::visible(&def.id, &archived))
                .cloned()
                .collect(),
            errors: Vec::new(),
            user_dir: None,
        };
        let mut graph = Graph::new("Shader library");
        let mut roots = Vec::new();
        let inputs = INPUTS.map(|(id, _)| graph.add(id, [0.; 2]));
        let [noise, gradient, _, cells, _] = inputs;
        let colored = graph.add("gradient_map", [0.; 2]);
        graph.connect(noise, colored, "input");
        let ramp = graph.node_mut(colored).expect("gradient map was added");
        for (name, color) in [
            ("dark", [0.08, 0.18, 0.28]),
            ("mid", [0.75, 0.38, 0.2]),
            ("light", [0.98, 0.86, 0.55]),
        ] {
            ramp.params.insert(name.into(), Value::Color(color));
        }
        let mut samples = Samples {
            graph: &mut graph,
            noise,
            gradient,
            cells,
            colored,
        };
        let mut uses_maps = false;
        let mut uses_surface = false;
        let user_list = node(commands, parent, column(4.));
        let template_list = node(commands, parent, column(4.));
        let mut add_card = |commands: &mut Commands,
                            list,
                            group,
                            label: &str,
                            action: Command,
                            definition: Option<&NodeDef>| {
            let id = definition.filter(|def| !def.shading).map(|def| {
                uses_maps |= def
                    .params
                    .iter()
                    .any(|param| param.kind == ParamKind::MeshMap);
                uses_surface |= def.surface;
                samples.add(def)
            });
            let button = control(commands, list, font, label, action.clone());
            if let Some(id) = id {
                roots.push(id);
                swatch(commands, button, id);
            }
            let card = control(commands, group, font, label, action);
            commands.entity(card).insert((
                AccessibleLabel(format!("Открыть узел · {label}")),
                Node {
                    width: px(134.),
                    min_height: px(112.),
                    padding: UiRect::all(px(8.)),
                    border: UiRect::all(px(1.)),
                    border_radius: BorderRadius::all(px(6.)),
                    justify_content: JustifyContent::End,
                    align_items: AlignItems::Center,
                    flex_shrink: 0.,
                    ..column(6.)
                },
            ));
            if let Some(id) = id {
                let image = spawn_swatch(commands, id, 72.);
                commands.entity(card).insert_children(0, &[image]);
            }
            if let Some(definition) = definition.filter(|def| !def.description.is_empty()) {
                control_hints::attach(commands, card, &definition.description);
            }
        };
        text(commands, template_list, font, "Шаблоны узлов", 12., TEXT);
        let templates = group(commands, home, font, "Шаблоны узлов");
        let library_count = definitions.defs.len();
        for (index, (label, source)) in TEMPLATES.into_iter().enumerate() {
            let definition = def::parse(&format!("gallery/template/{index}"), source).ok();
            add_card(
                commands,
                template_list,
                templates,
                label,
                Command::Source(source.into(), None),
                definition.as_ref(),
            );
            definitions.defs.extend(definition);
        }
        text(commands, parent, font, "Вход для предпросмотра", 12., TEXT);
        for (index, (_, label)) in INPUTS.into_iter().enumerate() {
            let button = control(commands, parent, font, label, Command::Input(index));
            swatch(commands, button, inputs[index]);
        }
        text(commands, user_list, font, "Мои шейдеры", 12., TEXT);
        let user = group(commands, home, font, "Мои шейдеры");
        let mut builtins = BTreeMap::<&str, Vec<&NodeDef>>::new();
        for def in &definitions.defs[..library_count] {
            if def.id.starts_with("user/") {
                let action = Command::Source(def.source.as_str().into(), Some(def.id.clone()));
                add_card(commands, user_list, user, &def.name, action, Some(def));
            } else {
                builtins.entry(&def.category).or_default().push(def);
            }
        }
        for (path, error) in library
            .errors
            .iter()
            .filter(|(path, _)| path.parent() == directory)
        {
            let name = path
                .file_name()
                .unwrap_or(path.as_os_str())
                .to_string_lossy();
            let label = format!("Исправить · {name}");
            let action = Command::OpenPath(path.clone());
            let button = control(commands, user, font, &label, action.clone());
            commands
                .entity(button)
                .insert(AccessibleLabel(format!("Открыть файл · {name}")));
            control_hints::attach(commands, button, error);
            let button = control(commands, user_list, font, &label, action);
            control_hints::attach(commands, button, error);
        }
        text(commands, parent, font, "Встроенные узлы", 12., TEXT);
        text(commands, home, font, "Встроенные узлы", 17., TEXT);
        for (category, definitions) in builtins {
            text(commands, parent, font, category, 11., MUTED);
            let category_cards = group(commands, home, font, category);
            for def in definitions {
                let action = Command::Source(def.source.as_str().into(), Some(def.id.clone()));
                add_card(
                    commands,
                    parent,
                    category_cards,
                    &def.name,
                    action,
                    Some(def),
                );
            }
        }
        roots.extend(inputs);
        Self {
            previews: previews::Previews::default(),
            graph,
            roots,
            library: Arc::new(definitions),
            uses_maps,
            uses_surface,
            maps: None,
            surface: None,
        }
    }

    pub(super) fn set_inputs(
        &mut self,
        doc: &Document,
        session: &Session,
        maps: Option<&Arc<bake::Maps>>,
        new: bool,
    ) {
        let maps = maps.filter(|_| self.uses_maps || self.uses_surface);
        let surface = session
            .binding
            .filter(|_| self.uses_surface)
            .and_then(|set| {
                self.surface
                    .as_ref()
                    .filter(|source| source.matches(doc, set))
                    .or_else(|| {
                        session
                            .graph_surfaces
                            .get(&set)
                            .filter(|source| source.matches(doc, set))
                    })
                    .cloned()
                    .or_else(|| surface::Source::capture(doc, set).map(Arc::new))
            });
        if new
            || !mesh_maps::same(self.maps.as_ref(), maps)
            || !mesh_maps::same(self.surface.as_ref(), surface.as_ref())
        {
            self.maps = maps.cloned();
            self.surface = surface;
            self.previews.set_gallery(
                &self.graph,
                previews::Gallery {
                    nodes: self.roots.clone(),
                    library: Some(self.library.clone()),
                },
                self.maps.as_ref(),
                self.surface.as_ref(),
            );
        }
    }
}

struct Samples<'a> {
    graph: &'a mut Graph,
    noise: u64,
    gradient: u64,
    cells: u64,
    colored: u64,
}

impl Samples<'_> {
    fn add(&mut self, def: &NodeDef) -> u64 {
        let node = self.graph.add(&def.id, [0.; 2]);
        for (index, input) in def.inputs.iter().enumerate() {
            let sample = match (index, &input.kind) {
                (0, Kind::Color) => self.colored,
                (0, _) => self.noise,
                (1, _) => self.cells,
                _ => self.gradient,
            };
            self.graph.connect(sample, node, &input.name);
        }
        node
    }
}

fn group(commands: &mut Commands, home: Entity, font: &Handle<Font>, label: &str) -> Entity {
    text(commands, home, font, label, 14., TEXT);
    node(
        commands,
        home,
        Node {
            flex_wrap: FlexWrap::Wrap,
            row_gap: px(6.),
            ..row(6.)
        },
    )
}

fn spawn_swatch(commands: &mut Commands, id: u64, size: f32) -> Entity {
    commands
        .spawn((
            Node {
                width: px(size),
                height: px(size),
                flex_shrink: 0.,
                ..default()
            },
            BackgroundColor(WELL),
            Pickable::IGNORE,
            Swatch(id),
        ))
        .id()
}

fn swatch(commands: &mut Commands, button: Entity, id: u64) {
    let image = spawn_swatch(commands, id, 26.);
    commands
        .entity(button)
        .insert_children(0, &[image])
        .entry::<Node>()
        .and_modify(|mut node| {
            node.min_height = px(32);
            node.justify_content = JustifyContent::Start;
        });
}
