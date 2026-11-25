//! Search and placement for the material graph's node picker.
use super::*;
use bevy::{input_focus::{FocusCause, InputFocus}, text::EditableText, ui_widgets::TextInput};
use std::collections::HashSet;

const WIDTH: f32 = 248.;
const HEIGHT: f32 = 320.;
const MARGIN: f32 = 6.;

#[derive(Clone, Copy)]
struct Placement {
    cursor: Vec2,
    graph: IVec2,
    source: Option<WireSource>,
}

pub(super) struct Palette {
    root: Entity,
    search: Entity,
    results: Entity,
    placement: Option<Placement>,
    open: bool,
    reset: bool,
    query: Option<String>,
    first: Option<(usize, f32)>,
}

impl Palette {
    pub(super) fn new(commands: &mut Commands, parent: Entity, font: &Handle<Font>) -> Self {
        let root = surface(commands, parent, Node {
            display: Display::None,
            position_type: PositionType::Absolute,
            padding: UiRect::all(px(6.)),
            border: UiRect::all(px(1.)),
            border_radius: BorderRadius::all(px(6.)),
            ..column(5.)
        }, CHROME);
        commands.entity(root).insert((BorderColor::all(EDGE), ZIndex(10),
            AccessibleLabel("Палитра узлов".into())));
        text(commands, root, font, "Поиск узлов", 11., MUTED);
        let search = commands.spawn((
            TextInput, EditableText::new(""), crate::ui_theme::text_cursor_style(),
            AccessibleLabel("Поиск узлов".into()), TextColor(TEXT),
            TextFont { font: font.clone().into(), font_size: FontSize::Px(12.), ..default() },
            TextLayout::no_wrap(), BackgroundColor(WELL), BorderColor::all(EDGE),
            Node {
                height: px(27.), flex_shrink: 0., min_width: px(0),
                border: UiRect::all(px(1.)), border_radius: BorderRadius::all(px(4.)),
                padding: UiRect::axes(px(6.), px(4.)), overflow: Overflow::clip(), ..default()
            }, ChildOf(root),
        )).id();
        let results = node(commands, root, Node {
            overflow: Overflow::scroll_y(), flex_grow: 1., min_height: px(0), ..column(4.)
        });
        commands.entity(results).insert(Scroller);
        Self { root, search, results, placement: None, open: false, reset: false, query: None, first: None }
    }

    pub(super) fn is_open(&self) -> bool { self.open }

    pub(super) fn toggle(&mut self) {
        if self.open { self.close(); } else { self.show(None); }
    }

    pub(super) fn at(&mut self, cursor: Vec2, graph: IVec2, source: Option<WireSource>) {
        self.show(Some(Placement { cursor, graph, source }));
    }

    fn show(&mut self, placement: Option<Placement>) {
        self.placement = placement;
        self.open = true;
        self.reset = true;
        self.query = None;
    }

    pub(super) fn close(&mut self) {
        self.placement = None;
        self.open = false;
        self.first = None;
    }

    pub(super) fn invalidate(&mut self) { self.query = None; }

    pub(super) fn clear_focus(&self, focus: &mut InputFocus) {
        if focus.get() == Some(self.search) { focus.clear(); }
    }

    pub(super) fn uses_default_position(&self) -> bool { self.placement.is_none() }

    pub(super) fn pick(&mut self, index: usize, position: IVec2) -> Option<Edit> {
        if !self.open { return None; }
        let edit = match self.placement {
            Some(Placement { graph, source: Some(source), .. }) => Edit::AddConnected(index, graph, source),
            Some(Placement { graph, .. }) => Edit::Add(index, graph),
            None => Edit::Add(index, position),
        };
        self.close();
        Some(edit)
    }

    pub(super) fn update(
        &mut self, commands: &mut Commands, nodes: &mut Query<&mut Node>, font: &Handle<Font>,
        library: &Library, archived: &HashSet<String>, shading: bool, folder: bool,
        stage: Rect, root: Vec2, cursor: Option<Vec2>, mouse: &ButtonInput<MouseButton>,
        keys: &ButtonInput<KeyCode>, focus: &mut InputFocus, editable: &mut Query<&mut EditableText>,
    ) -> Option<(usize, f32)> {
        let size = Vec2::new(WIDTH, HEIGHT).min((stage.size() - Vec2::splat(2. * MARGIN)).max(Vec2::ONE));
        let anchor = self.placement.map_or(stage.min, |at| stage.min + at.cursor);
        let min = stage.min + Vec2::splat(MARGIN);
        let top_left = anchor.clamp(min, (stage.max - size - Vec2::splat(MARGIN)).max(min));
        if self.open && (keys.just_pressed(KeyCode::Escape)
            || (!self.reset && mouse.any_just_pressed([MouseButton::Left, MouseButton::Right])
                && cursor.is_none_or(|p| !Rect::from_corners(top_left, top_left + size).contains(p))))
        { self.close(); }
        if let Ok(mut node) = nodes.get_mut(self.root) {
            let display = if self.open { Display::Flex } else { Display::None };
            if node.display != display { node.display = display; }
            if self.open {
                let offset = top_left - root;
                node.left = px(offset.x);
                node.top = px(offset.y);
                node.width = px(size.x);
                node.height = px(size.y);
            }
        }
        if !self.open {
            self.clear_focus(focus);
            return None;
        }
        let mut field = editable.get_mut(self.search).ok()?;
        if std::mem::take(&mut self.reset) {
            field.editor_mut().set_text("");
            focus.set(self.search, FocusCause::Auto);
            commands.entity(self.results).insert(ScrollPosition::default());
        }
        let query = field.value().to_string().trim().to_lowercase();
        if self.query.as_ref() != Some(&query) {
            let linked = self.placement.is_some_and(|at| at.source.is_some());
            self.first = self.populate(commands, font, library, archived, shading, folder, linked, &query);
            self.query = Some(query);
            commands.entity(self.results).insert(ScrollPosition::default());
        }
        (focus.get() == Some(self.search) && keys.any_just_pressed([KeyCode::Enter, KeyCode::NumpadEnter]))
            .then_some(self.first).flatten()
    }

    fn populate(
        &self, commands: &mut Commands, font: &Handle<Font>, library: &Library,
        archived: &HashSet<String>, shading: bool, folder: bool, linked: bool, query: &str,
    ) -> Option<(usize, f32)> {
        commands.entity(self.results).despawn_related::<Children>();
        let mut first = None;
        for (category, ids) in library.categories() {
            let mut matching = library.defs.iter().enumerate().filter(|(_, def)| {
                def.shading == shading && ids.contains(&def.id) && node_library::visible(&def.id, archived)
                    && (!linked || !def.inputs.is_empty())
                    && (!folder || def.id != material_painter_graph::OUTPUT_DEF)
                    && matches_query(&def.name, &def.id, &category, query)
            }).peekable();
            if matching.peek().is_none() { continue; }
            text(commands, self.results, font, &category, 11., MUTED);
            for (index, def) in matching {
                let choice = (index, card_height(def.inputs.len(), def.shading));
                first.get_or_insert(choice);
                let button = local_button(commands, self.results, font, &def.name);
                commands.entity(button).insert(Help(def.description.clone())).observe(
                    move |event: On<PointerClick>, mut ui: ResMut<PaintUi>, mut messages: MessageWriter<Action>| {
                        if event.button == PointerButton::Primary {
                            ui.graph.pick_palette_node(choice, &mut messages);
                        }
                    },
                );
            }
        }
        if first.is_none() { text(commands, self.results, font, "Узлы не найдены", 12., MUTED); }
        first
    }
}

fn matches_query(name: &str, id: &str, category: &str, query: &str) -> bool {
    query.is_empty() || [name, id, category].iter().any(|value| value.to_lowercase().contains(query))
}
