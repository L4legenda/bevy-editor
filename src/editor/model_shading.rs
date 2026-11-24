//! Viewport shading is local to the model workspace.
use super::*;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::editor) enum Mode {
    Wire,
    Solid,
    #[default]
    Material,
    Rendered,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::editor) enum ShadingAction {
    Set(Mode),
    PreviewLighting,
}

#[derive(Resource, Default)]
pub(super) struct Shading {
    pub mode: Mode,
    pub preview_lighting: bool,
}

impl Shading {
    pub(super) fn viewport(&self, mode: crate::editor::model_document::Mode) -> (Mode, bool) {
        if mode == crate::editor::model_document::Mode::Paint {
            (Mode::Rendered, true)
        } else {
            (self.mode, self.mode == Mode::Rendered && self.preview_lighting)
        }
    }

    pub fn apply(&mut self, action: ShadingAction) {
        match action {
            ShadingAction::Set(mode) => self.mode = mode,
            ShadingAction::PreviewLighting => self.preview_lighting = !self.preview_lighting,
        }
    }
}

#[derive(Component)]
pub(super) struct ShadingButton(Mode);

pub(super) fn setup(commands: &mut Commands, toolbar: Entity) {
    let group = surface(
        commands,
        toolbar,
        Node {
            padding: UiRect::all(px(2)),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(6)),
            flex_shrink: 0.0,
            ..row(2.0)
        },
        PANEL,
    );
    let xray = icons::button(
        commands,
        group,
        icons::Glyph::Xray,
        "Рентген · Alt+Z",
        Action::ModelView(ModelViewAction::Edit(editing::EditAction::Xray)),
        Look::Regular,
    );
    commands
        .entity(xray)
        .insert(editing::controls::EditButton(editing::EditAction::Xray));

    for (mode, glyph, label, hint) in [
        (
            Mode::Wire,
            icons::Glyph::Globe,
            "Каркас",
            "Показать геометрию рёбрами без заливки поверхностей.",
        ),
        (
            Mode::Solid,
            icons::Glyph::ShadingSolid,
            "Сплошной режим",
            "Нейтральное покрытие и студийный свет для проверки формы.",
        ),
        (
            Mode::Material,
            icons::Glyph::ShadingMaterial,
            "Просмотр материалов",
            "Цвета и текстуры модели без освещения и окружения.",
        ),
        (
            Mode::Rendered,
            icons::Glyph::ShadingRendered,
            "Рендер сцены",
            "Материалы модели, окружение и настроенные источники света текущей сцены.",
        ),
    ] {
        let action = Action::ModelView(ModelViewAction::Shading(ShadingAction::Set(mode)));
        let entity = icons::button(commands, group, glyph, label, action, Look::Regular);
        commands.entity(entity).insert(ShadingButton(mode));
        control_hints::attach(commands, entity, hint);
    }
}

pub(super) fn refresh(
    shading: Res<Shading>,
    mut buttons: Query<(
        &ShadingButton,
        &Hovered,
        &mut BackgroundColor,
        &mut BorderColor,
    )>,
) {
    for (button, hovered, mut color, mut border) in &mut buttons {
        let active = shading.mode == button.0;
        color.0 = if active {
            ACTIVE
        } else if hovered.0 {
            WELL
        } else {
            Color::NONE
        };
        *border = BorderColor::all(if active {
            FOCUS.with_alpha(0.7)
        } else {
            EDGE.with_alpha(0.3)
        });
    }
}
