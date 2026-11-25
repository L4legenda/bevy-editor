use super::*;

pub(super) struct BrushLibraryUi {
    pub root: Entity,
    brushes: Entity,
    category: Option<usize>,
    rows: Vec<Entity>,
    font: Handle<Font>,
}

impl BrushLibraryUi {
    pub fn setup(commands: &mut Commands, parent: Entity, font: &Handle<Font>) -> Self {
        let root = node(
            commands,
            parent,
            Node {
                width: percent(100),
                height: percent(100),
                ..column(10.0)
            },
        );
        let header = node(
            commands,
            root,
            Node {
                justify_content: JustifyContent::SpaceBetween,
                width: percent(100),
                ..row(8.0)
            },
        );
        text(commands, header, font, "Библиотека кистей", 15.0, TEXT);
        let library = brush_library::library();
        text(
            commands,
            header,
            font,
            &format!("{} кистей", library.presets.len()),
            11.0,
            MUTED,
        );
        let lists = node(
            commands,
            root,
            Node {
                width: percent(100),
                flex_grow: 1.0,
                min_height: px(0),
                align_items: AlignItems::Stretch,
                ..row(10.0)
            },
        );
        let categories = Self::list(
            commands,
            lists,
            Node {
                width: percent(31),
                flex_shrink: 0.0,
                ..column(3.0)
            },
        );
        for (index, name) in library.categories.iter().enumerate() {
            let item = button(
                commands,
                categories,
                font,
                category_label(name),
                Action::ImageEdit(EditAction::BrushCategory(index)),
                Look::Regular,
            );
            commands.entity(item).entry::<Node>().and_modify(|mut n| {
                n.width = percent(100);
                n.min_height = px(30);
            });
        }
        let brushes = Self::list(
            commands,
            lists,
            Node {
                flex_grow: 1.0,
                min_width: px(0),
                ..column(5.0)
            },
        );
        Self {
            root,
            brushes,
            category: None,
            rows: Vec::new(),
            font: font.clone(),
        }
    }

    fn list(commands: &mut Commands, parent: Entity, mut layout: Node) -> Entity {
        layout.overflow = Overflow::scroll_y();
        layout.min_height = px(0);
        let list = node(commands, parent, layout);
        commands
            .entity(list)
            .insert((Scroller, ScrollPosition::default()));
        list
    }

    pub fn show_category(
        &mut self,
        commands: &mut Commands,
        images: &mut Assets<Image>,
        category: usize,
    ) {
        if self.category == Some(category) {
            return;
        }
        self.category = Some(category);
        for row in self.rows.drain(..) {
            commands.entity(row).despawn();
        }
        commands
            .entity(self.brushes)
            .insert(ScrollPosition::default());
        let library = brush_library::library();
        let Some(group) = library.groups.get(category) else {
            return;
        };
        for &index in group {
            let name = library.presets[index].name.as_str();
            let card = control(
                commands,
                self.brushes,
                Action::ImageEdit(EditAction::BrushPreset(Some(index))),
                Look::Asset,
                Node {
                    width: percent(100),
                    flex_shrink: 0.0,
                    padding: UiRect::axes(px(9), px(5)),
                    border: UiRect::all(px(1)),
                    border_radius: BorderRadius::all(px(6)),
                    ..column(2.0)
                },
            );
            commands.entity(card).insert(AccessibleLabel(name.into()));
            text(commands, card, &self.font, name, 12.0, TEXT);
            if let Some(preview) = brush_library::preview(index) {
                commands.spawn((
                    ImageNode {
                        image_mode: bevy::ui::widget::NodeImageMode::Stretch,
                        ..ImageNode::new(images.add(preview))
                    },
                    Node {
                        width: percent(100),
                        height: px(60),
                        ..default()
                    },
                    Pickable::IGNORE,
                    ChildOf(card),
                ));
            }
            self.rows.push(card);
        }
    }
}

fn category_label(name: &str) -> &str {
    match name {
        "Grunge" => "Гранж",
        "Markers" => "Маркеры",
        "Oils" => "Масло",
        "Charcoals" => "Уголь",
        "Pastels" => "Пастель",
        "Street Art" => "Стрит-арт",
        "Gouache" => "Гуашь",
        "Pens" => "Ручки",
        "Creative" => "Эффекты",
        "Paints" => "Живопись",
        "Lettering" => "Леттеринг",
        "Watercolors" => "Акварель",
        "Digital" => "Цифровые",
        "Inks" => "Тушь",
        "Basics" => "Базовые",
        "Design" => "Дизайн",
        "Pencils" => "Карандаши",
        "Comics" => "Комиксы",
        _ => name,
    }
}
