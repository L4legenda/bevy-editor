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
        .insert((GlobalZIndex(90), Pickable::IGNORE));
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
            Action::ImageView(super::ImageAction::Previous) => {
                "Это первое изображение в текущем списке.".into()
            }
            Action::ImageView(super::ImageAction::Next) => {
                "Это последнее изображение в текущем списке.".into()
            }
            _ if editor.asset_viewer_open() || editor.browser_popup => return None,
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
        Action::Tool(Tool::Universal) => {
            "Выбор и преобразование (Q) · Выбирать объекты в сцене и дереве, перемещать за стрелки и площадки, вращать за дуги. Для изменения размера включите Масштаб (R). Режим выбран по умолчанию."
                .into()
        }
        Action::Tool(Tool::Move) => "Двигать (W) · Перемещать объект за цветные оси в 3D-виде.".into(),
        Action::Tool(Tool::Rotate) => "Поворот (E) · Вращать объект за цветные кольца.".into(),
        Action::Tool(Tool::Scale) => "Масштаб (R) · Изменять масштаб объекта по выбранной оси.".into(),
        Action::ToggleSceneTree => {
            if editor.sidebar.tree_collapsed() {
                "Развернуть дерево сцены.".into()
            } else {
                "Свернуть дерево сцены и освободить место для инспектора.".into()
            }
        }
        Action::LocalSpace => {
            "Использовать локальное пространство · Оси манипулятора следуют повороту объекта, координаты инспектора задаются относительно родителя. Повторное нажатие возвращает глобальное пространство."
                .into()
        }
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
        Action::Focus => "Num . · Плавно подлететь к выбранному объекту.".into(),
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
        Action::Drawer => {
            "Ctrl+Space · Открыть или свернуть ассеты проекта. Верхний край панели можно перетаскивать."
                .into()
        }
        Action::BrowseBack => "Вернуться к предыдущей папке.".into(),
        Action::BrowseForward => "Перейти к следующей папке в истории.".into(),
        Action::BrowseUp => "Открыть родительскую папку.".into(),
        Action::BrowseRefresh => "Обновить список после добавления файлов на диске.".into(),
        Action::BrowseClear => "Очистить поиск и показать содержимое текущей папки.".into(),
        Action::BrowserNewFolder => "Ctrl+Shift+N · Создать папку в выбранной папке файлового менеджера.".into(),
        Action::BrowserNewImage => "Создать PNG-изображение с белым фоном в выбранной папке.".into(),
        Action::BrowserCreateImage => "Enter · Создать изображение выбранного размера.".into(),
        Action::BrowserCreateFolder => "Enter · Создать папку с указанным названием.".into(),
        Action::BrowserCancelPopup => "Esc · Закрыть меню или отменить создание папки.".into(),
        Action::BrowserDeleteFile => "Delete · Удалить файл. До закрытия редактора его можно вернуть через Ctrl+Z.".into(),
        Action::BrowserRestoreFile => "Ctrl+Z · Вернуть последний удалённый файл. При закрытии редактора корзина очищается окончательно.".into(),
        Action::ViewImage(index) => format!(
            "Двойной щелчок — открыть «{}» во вкладке. В просмотре: ← → — листать, колесо — масштаб. Перетащите карточку в папку, чтобы переместить файл.",
            browser.files.get(index)?.filename
        ),
        Action::ImageEdit(action) => match action {
            super::ImageEditAction::Menu => "Выберите просмотр или редактирование изображения в PhotoCraft.".into(),
            super::ImageEditAction::View => "Просмотр изображения. Несохранённые слои остаются во вкладке.".into(),
            super::ImageEditAction::Edit => "Редактировать изображение. Рядом создаётся проект слоёв .pcraft.".into(),
            super::ImageEditAction::Save => "Ctrl+S · Сохранить слои .pcraft и обновить исходное изображение.".into(),
            super::ImageEditAction::Undo => "Ctrl+Z · Отменить последнее изменение изображения.".into(),
            super::ImageEditAction::Redo => "Ctrl+Shift+Z / Ctrl+Y · Повторить изменение изображения.".into(),
            super::ImageEditAction::ColorPanel => "Нажмите на цвет, чтобы раскрыть или свернуть палитру HSV. HEX применяется по Enter.".into(),
            super::ImageEditAction::Tool(tool) => match tool {
                crate::editor::photo_document::PhotoTool::Pencil => "B · Карандаш. Рисование по пиксельной сетке без сглаживания. ПКМ по инструменту — выбрать кисть или карандаш.".into(),
                crate::editor::photo_document::PhotoTool::Brush => "B · Кисть PhotoCraft со сглаживанием. Рисует в выбранном слое.".into(),
                crate::editor::photo_document::PhotoTool::Eraser => "E · Ластик. Убирает пиксели выбранного слоя.".into(),
                crate::editor::photo_document::PhotoTool::Fill => "G · Заливка связанной области выбранным цветом.".into(),
                crate::editor::photo_document::PhotoTool::Eyedropper => "I · Пипетка. Взять цвет изображения.".into(),
                crate::editor::photo_document::PhotoTool::Move => "V · Перетащить содержимое выбранного слоя.".into(),
                crate::editor::photo_document::PhotoTool::Hand => "H / Пробел · Перемещать изображение в окне.".into(),
            },
            super::ImageEditAction::AddLayer => "Создать прозрачный слой над выбранным.".into(),
            super::ImageEditAction::DrawingTool => "B · Рисование. ПКМ — выбрать кисть со сглаживанием или пиксельный карандаш. Shift+B — переключить инструмент.".into(),
            super::ImageEditAction::DuplicateLayer => "Дублировать выбранный слой.".into(),
            super::ImageEditAction::DeleteLayer => "Удалить выбранный слой. Ctrl+Z отменяет удаление.".into(),
            super::ImageEditAction::LayerUp => "Переместить слой выше.".into(),
            super::ImageEditAction::LayerDown => "Переместить слой ниже.".into(),
            super::ImageEditAction::Visible(_) => "Показать или скрыть слой.".into(),
            super::ImageEditAction::SelectLayer(_) => "Выбрать слой для рисования и редактирования.".into(),
            _ => "Настройки цвета и инструментов PhotoCraft.".into(),
        },
        Action::ImageView(action) => match action {
            super::ImageAction::Close => "Ctrl+W · Закрыть вкладку изображения.".into(),
            super::ImageAction::Previous => "← · Предыдущее изображение в текущей папке или результатах поиска.".into(),
            super::ImageAction::Next => "→ · Следующее изображение в текущей папке или результатах поиска.".into(),
            super::ImageAction::Fit => "F / 0 · Вписать изображение в окно. Маленькие изображения сохраняют исходный размер.".into(),
            super::ImageAction::ActualSize => "1 · Исходный размер: один пиксель изображения на один пиксель экрана. Перетаскивайте, чтобы рассмотреть детали.".into(),
            super::ImageAction::ZoomIn => "+ / колесо вверх · Увеличить изображение.".into(),
            super::ImageAction::ZoomOut => "− / колесо вниз · Уменьшить изображение.".into(),
            super::ImageAction::Background => "B · Шахматный, тёмный или светлый фон для проверки прозрачности.".into(),
            super::ImageAction::Retry => "Повторно загрузить изображение после исправления файла на диске.".into(),
        },
        Action::OpenImageTab(id) => format!(
            "Открыть вкладку изображения: {}",
            editor.image_tab(id)?.path.display()
        ),
        Action::CloseImageTab(id) => format!(
            "Закрыть вкладку изображения: {}",
            editor.image_tab(id)?.path.display()
        ),
        Action::ViewModel(index) => format!(
            "Нажмите, чтобы рассмотреть «{}». Перетащите модель в 3D-вид, чтобы добавить её в сцену.",
            editor.project.catalog[index].label
        ),
        Action::Select(_) if editor.tool != Tool::Universal =>
            "Для выбора другого объекта включите режим со стрелкой (Q).".into(),
        Action::Select(target) => format!(
            "{} · Выбрать для редактирования. Num . приблизит свободный вид.",
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
    controls: Query<
        (
            Entity,
            &Control,
            &super::Hovered,
            &ComputedNode,
            &UiGlobalTransform,
        ),
        Without<super::drawer::FileCard>,
    >,
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
