//! Short explanations for the control under the pointer or keyboard focus.
use bevy::{input_focus::InputFocusVisible, prelude::*};

use super::{
    Action, Browser, Control, Editor, InputFocus, MUTED, PrimaryWindow, TEXT, Tool,
    UiGlobalTransform, WELL, available, column, surface, text,
};

const WIDTH: f32 = 326.0;
const DELAY: f32 = 0.45;

#[derive(Resource)]
pub(in crate::editor) struct Hints {
    panel: Entity,
    label: Entity,
    target: Option<Entity>,
    elapsed: f32,
}

pub(super) fn setup(commands: &mut Commands, root: Entity, font: &Handle<Font>) {
    let panel = surface(
        commands,
        root,
        Node {
            display: Display::None,
            position_type: PositionType::Absolute,
            width: px(WIDTH),
            padding: UiRect::all(px(10)),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(7)),
            ..column(0.0)
        },
        WELL,
    );
    commands
        .entity(panel)
        .insert((ZIndex(100), Pickable::IGNORE));
    let label = text(commands, panel, font, "", 12.0, TEXT);
    commands.entity(label).insert(TextLayout::default());
    commands.insert_resource(Hints {
        panel,
        label,
        target: None,
        elapsed: 0.0,
    });
}

fn description(action: Action, editor: &Editor, browser: &Browser) -> Option<String> {
    let command = if cfg!(target_os = "macos") {
        "Cmd"
    } else {
        "Ctrl"
    };
    if !available(editor, browser, action) {
        return Some(match action {
            Action::Undo => "Пока нет действий для отмены.".into(),
            Action::Redo => "Нет отменённых действий для повтора.".into(),
            Action::Align if editor.pilot => "Сначала выйдите из вида камеры через Esc.".into(),
            Action::BrowseBack => "Вы уже в начале истории папок.".into(),
            Action::BrowseForward => "Нет следующей папки в истории.".into(),
            Action::BrowseUp => "Это корневая папка ассетов проекта.".into(),
            _ if !editor.has_scene() => "Сначала откройте сцену в файловом менеджере.".into(),
            _ if !editor.ready => "Дождитесь загрузки сцены.".into(),
            _ => "Выберите подходящий объект в дереве сцены.".into(),
        });
    }
    Some(match action {
        Action::Open(room) => format!(
            "{} · Открыть сцену «{}». Правки остальных вкладок останутся в памяти.",
            editor.project.scenes[room].asset, editor.project.scenes[room].label
        ),
        Action::CloseTab(room) => format!(
            "{command}+W · Закрыть сцену «{}». При несохранённых изменениях появится выбор действия.",
            editor.project.scenes[room].label
        ),
        Action::OpenBrowser => format!(
            "{command}+O · Найти и открыть сцену через файловый менеджер. Открытые сцены появятся во вкладках."
        ),
        Action::Tool(Tool::Move) => "Двигать (W) · Перемещать объект за цветные оси в 3D-виде.".into(),
        Action::Tool(Tool::Rotate) => "Поворот (E) · Вращать объект за цветные кольца.".into(),
        Action::Tool(Tool::Scale) => "Масштаб (R) · Изменять масштаб объекта по выбранной оси.".into(),
        Action::Snap => {
            "Привязка: 0,25 м при движении, 15° при вращении и 0,1 при масштабировании.".into()
        }
        Action::Undo => format!(
            "{command}+Z · Отменить последнее действие. Перемещение камеры отменяется одним жестом."
        ),
        Action::Redo => format!("{command}+Y · Повторить отменённое действие."),
        Action::Save => format!("{command}+S · Сохранить текущую сцену, объекты и камеры."),
        Action::Play => {
            "Запустить игровое окно с текущей сценой. Сначала сохраните изменения.".into()
        }
        Action::Focus => "F · Приблизить свободный вид к выбранному объекту.".into(),
        Action::Pilot if editor.pilot => {
            "Esc · Вернуться к свободному виду, сохранив перемещения камеры.".into()
        }
        Action::Pilot => {
            "Управлять выбранной камерой: ПКМ + WASD / Q E.".into()
        }
        Action::Align => {
            "Перенести выбранную камеру в текущую позицию и направление свободного вида.".into()
        }
        Action::Duplicate => format!("{command}+D · Создать копию выбранного экземпляра модели."),
        Action::Delete => "Delete · Удалить добавленную модель. Действие можно отменить.".into(),
        Action::Reset => {
            "Вернуть исходные координаты и настройки выбранного объекта. Действие можно отменить."
                .into()
        }
        Action::Drawer => {
            "Ctrl+Space · Открыть или свернуть ассеты проекта. Верхний край панели можно перетаскивать."
                .into()
        }
        Action::BrowseBack => "Вернуться к предыдущей папке.".into(),
        Action::BrowseForward => "Перейти к следующей папке в истории.".into(),
        Action::BrowseUp => "Открыть родительскую папку.".into(),
        Action::BrowseRefresh => "Обновить список после добавления файлов на диске.".into(),
        Action::BrowseClear => "Очистить поиск и показать содержимое текущей папки.".into(),
        Action::BrowseModel(index) => format!(
            "Выбрать «{}». Перетащите модель в 3D-вид, чтобы добавить её в сцену.",
            editor.project.catalog[index].label
        ),
        Action::Select(target) => format!(
            "{} · Выбрать для редактирования. F приблизит свободный вид.",
            editor.label(target)
        ),
        _ => return None,
    })
}

pub(in crate::editor) fn update(
    mut hints: ResMut<Hints>,
    editor: Res<Editor>,
    browser: Res<Browser>,
    focus: Res<InputFocus>,
    focus_visible: Res<InputFocusVisible>,
    time: Res<Time>,
    mouse: Res<ButtonInput<MouseButton>>,
    window: Single<&Window, With<PrimaryWindow>>,
    controls: Query<(
        Entity,
        &Control,
        &super::Hovered,
        &ComputedNode,
        &UiGlobalTransform,
    )>,
    mut panels: Query<&mut Node>,
    mut labels: Query<(&mut Text, &mut TextColor)>,
) {
    let target = if editor.pending.is_some()
        || mouse.any_pressed([MouseButton::Left, MouseButton::Middle, MouseButton::Right])
    {
        None
    } else {
        controls
            .iter()
            .find(|(_, _, hovered, computed, _)| hovered.0 && computed.size().min_element() > 0.0)
            .or_else(|| {
                focus
                    .get()
                    .filter(|_| focus_visible.0)
                    .and_then(|entity| controls.get(entity).ok())
                    .filter(|(_, _, _, computed, _)| computed.size().min_element() > 0.0)
            })
    };
    let entity = target.map(|(entity, ..)| entity);
    if hints.target != entity {
        hints.target = entity;
        hints.elapsed = 0.0;
    } else {
        hints.elapsed += time.delta_secs();
    }
    let content =
        target.and_then(|(_, control, _, _, _)| description(control.action, &editor, &browser));
    let shown = content.is_some() && hints.elapsed >= DELAY;
    if let Ok(mut panel) = panels.get_mut(hints.panel) {
        let display = if shown { Display::Flex } else { Display::None };
        if panel.display != display {
            panel.display = display;
        }
        if let Some((_, _, _, computed, transform)) = target.filter(|_| shown) {
            let scale = window.scale_factor();
            let center = transform.affine().translation / scale;
            let half = computed.size() / scale * 0.5;
            let x = (center.x - half.x).clamp(8.0, (window.width() - WIDTH - 8.0).max(8.0));
            let below = center.y + half.y + 7.0;
            let y = if below + 88.0 < window.height() {
                below
            } else {
                (center.y - half.y - 88.0).max(8.0)
            };
            if panel.left != px(x) || panel.top != px(y) {
                panel.left = px(x);
                panel.top = px(y);
            }
        }
    }
    if let Some(content) = content.filter(|_| shown)
        && let Ok((mut text, mut color)) = labels.get_mut(hints.label)
    {
        if text.0 != content {
            text.0 = content;
        }
        color.set_if_neq(TextColor(MUTED));
    }
}
