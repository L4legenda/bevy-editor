//! Short explanations for the control under the pointer or keyboard focus.
use super::image_viewer::edit::EraserMode;
use super::model_viewer::{ModelViewAction, editing::EditAction as ModelEditAction};
use bevy::{input_focus::InputFocusVisible, prelude::*};

use super::{
    Action, Browser, Control, Editor, InputFocus, MUTED, PrimaryWindow, TEXT, Tool,
    UiGlobalTransform, WELL, available, column, surface, text,
};

const WIDTH: f32 = 326.0;
const DELAY: f32 = 0.45;

/// Explanations may also belong to sliders and fields without a button action.
#[derive(Component)]
pub(in crate::editor) struct Help(pub String);

/// Labels can explain themselves without intercepting clicks on nearby controls.
pub(in crate::editor) fn attach(commands: &mut Commands, entity: Entity, hint: impl Into<String>) {
    commands.entity(entity).insert((
        Help(hint.into()),
        super::Hovered::default(),
        Pickable {
            is_hoverable: true,
            should_block_lower: false,
        },
    ));
}

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
        .insert((GlobalZIndex(1500), Pickable::IGNORE));
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
            Action::Generation(super::GenerationAction::Generate) => {
                if editor.generation.running {
                    "Генерация уже выполняется. Дождитесь завершения или отмените её.".into()
                } else if !editor.generation.backend_ready {
                    "Установите веса Mini и нажмите «Включить модель» во вкладке 3D генерации."
                        .into()
                } else {
                    "Выберите изображение и проверьте параметры генерации.".into()
                }
            }
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
    if editor.viewing_model.is_some() {
        use super::model_viewer::editing::{EditAction, toolbar_action};
        let model_hint = match toolbar_action(action) {
            Some(EditAction::Tool(Tool::Universal)) => Some(
                "Выделение (Q) · Щелчок выбирает объект или элемент сетки, перетаскивание ЛКМ выделяет рамкой. Shift добавляет, Ctrl исключает.",
            ),
            Some(EditAction::Tool(Tool::Move)) => Some(
                "Перемещение (G) · Перетаскивайте выделение за цветные оси. G, затем X/Y/Z и число задают точное смещение.",
            ),
            Some(EditAction::Tool(Tool::Rotate)) => Some(
                "Поворот (R) · Вращайте выделение за цветные кольца. R, затем X/Y/Z и число задают угол в градусах.",
            ),
            Some(EditAction::Tool(Tool::Scale)) => Some(
                "Масштаб (S) · Меняйте размер выделения за манипулятор. S, затем X/Y/Z и число задают точный масштаб.",
            ),
            Some(EditAction::Local) => Some(
                "Локальные оси модели · Оси манипулятора следуют повороту активного объекта. Повторное нажатие возвращает мировые оси.",
            ),
            Some(EditAction::Snap) => Some(
                "Привязка модели: 0,25 при движении, 15° при вращении и 0,1 при масштабировании. Ctrl временно включает привязку.",
            ),
            _ => None,
        };
        if let Some(hint) = model_hint {
            return Some(hint.into());
        }
    }
    Some(match action {
        Action::BrowserAutoUnwrap =>
            "Авторазвёртка для редактируемого импорта OBJ и FBX: строить только недостающие UV, строить для всех наборов или требовать готовые UV.".into(),
        Action::BrowserDetectUdim => super::model_viewer::editing::uv::UDIM_IMPORT_HINT.into(),
        Action::BrowserTextureSize =>
            "Разрешение новых наборов текстур при открытии модели. Сохранённые наборы используют своё разрешение.".into(),
        Action::BrowserBakeOnLoad =>
            "Подготовить карты видимых наборов при первом входе в режим рисования. Сохранённые карты используются повторно.".into(),
        Action::BrowserBakeQuality =>
            "Качество ручного и автоматического запекания для моделей без записанных настроек. Нажмите, чтобы выбрать следующий вариант.".into(),
        Action::BrowserBakeMatchNames =>
            "Для новых моделей сопоставлять детали high-poly и low-poly по именам с учётом окончаний _high и _low.".into(),
        Action::Generation(super::GenerationAction::Open) => "Открыть генерацию 3D модели по изображению. Настройки сохраняются при переключении вкладок.".into(),
        Action::Generation(super::GenerationAction::Toggle) if editor.generation.active => "Закрыть вкладку генерации и вернуться к предыдущему документу.".into(),
        Action::Generation(super::GenerationAction::Toggle) => "Открыть генерацию 3D модели по изображению.".into(),
        Action::Generation(super::GenerationAction::Close) => "Закрыть вкладку генерации и вернуться к предыдущему документу.".into(),
        Action::Generation(super::GenerationAction::Advanced) => "Шаги диффузии, CFG, разрешение поверхности, размер блока и seed.".into(),
        Action::Generation(super::GenerationAction::RandomSeed) => "При подключении генерации новое случайное число даст другой вариант. Выключите для повторяемого результата.".into(),
        Action::Generation(super::GenerationAction::Simplify) => "Создать облегчённую сетку с указанным целевым числом треугольников.".into(),
        Action::Generation(super::GenerationAction::ToggleWireframe) => "Показать рёбра полигонов поверх модели. Повторное нажатие скрывает сетку.".into(),
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
            "Выделение (Q) · Щелчок выбирает объект, перетаскивание ЛКМ выделяет рамкой. Shift добавляет, Ctrl исключает. Поля инспектора относятся к активному объекту."
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
        Action::Play if editor.play.running() => "F5 · Остановить запущенное игровое окно и вернуться к редактированию.".into(),
        Action::Play => "F5 · Запустить открытый уровень в Bevy с текущими изменениями. Используется выбранная камера или первая камера сцены. В игре: ПКМ + WASD / Q E, Shift — быстрее, Esc — выход.".into(),
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
        Action::BrowserNewScene => "Создать пустую сцену в выбранной папке.".into(),
        Action::BrowserOpenEntry => "Открыть выбранный файл или папку.".into(),
        Action::BrowserRevealEntry => {
            "Открыть папку в проводнике или показать выбранный файл в содержащей его папке.".into()
        }
        Action::BrowserCreateScene => "Enter · Создать сцену.".into(),
        Action::BrowserDialogueMenu => "Создать диалог или персонажа в этой папке · наведите курсор для выбора документа.".into(),
        Action::BrowserPrimitiveMenu => "Создать 3D-фигуру в этой папке · наведите курсор для выбора фигуры.".into(),
        Action::BrowserCreatePrimitive(primitive) => format!("{} · задать название и параметры новой 3D-модели.", primitive.label()),
        Action::BrowserConfirmPrimitive => "Enter · Создать GLB-модель с указанными размерами и детализацией.".into(),
        Action::BrowserCreateImage => "Enter · Создать изображение выбранного размера.".into(),
        Action::BrowserCreateFolder => "Enter · Создать папку с указанным названием.".into(),
        Action::BrowserCancelPopup => "Esc · Закрыть меню или отменить создание папки.".into(),
        Action::BrowserDeleteFile => "Delete · Удалить файл. До закрытия редактора его можно вернуть через Ctrl+Z.".into(),
        Action::BrowserRestoreFile => "Ctrl+Z · Вернуть последний удалённый файл. При закрытии редактора корзина очищается окончательно.".into(),
        Action::EditBrowserImage(..) => "Открыть изображение в режиме редактирования.".into(),
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
            super::ImageEditAction::SizeMenu => "Библиотека кистей или параметры карандаша и ластика. Колесо или Shift + движение мыши меняют размер.".into(),
            super::ImageEditAction::Redo => "Ctrl+Shift+Z / Ctrl+Y · Повторить изменение изображения.".into(),
            super::ImageEditAction::ModelPose => "Тяните за квадрат на запястье или щиколотке, чтобы согнуть конечность. Для вращения выберите сустав и потяните кольцо. Esc отменяет жест.".into(),
            super::ImageEditAction::ModelPoseReset => "Вернуть все суставы к исходной позе.".into(),
            super::ImageEditAction::ColorPicker(_) => "Выбрать цвет. Изменения применяются по ОК; Esc — отмена.".into(),
            super::ImageEditAction::SwapColors => "X · Поменять основной и фоновый цвета местами.".into(),
            super::ImageEditAction::ResetColors => "D · Вернуть чёрный основной и белый фоновый цвет.".into(),
            super::ImageEditAction::ColorConfirm => "Применить выбранный цвет · Enter.".into(),
            super::ImageEditAction::ColorCancel => "Отменить выбор цвета · Esc.".into(),
            super::ImageEditAction::AddSwatch => "Добавить новый цвет в образцы.".into(),
            super::ImageEditAction::EraserMode(mode) => match mode {
                EraserMode::Brush => "Обычный ластик с настройкой жёсткости.".into(),
                EraserMode::Pixel => "Стирать целые пиксели без сглаживания.".into(),
            },
            super::ImageEditAction::Tool(tool) => match tool {
                crate::editor::photo_document::PhotoTool::Pencil => "B · Карандаш. Рисование по пиксельной сетке без сглаживания. ПКМ по инструменту — выбрать кисть или карандаш.".into(),
                crate::editor::photo_document::PhotoTool::Brush => "B · Кисть PhotoCraft со сглаживанием. Рисует в выбранном слое.".into(),
                crate::editor::photo_document::PhotoTool::Eraser => "E · Стирать. Стрелка или ПКМ — выбрать обычный или пиксельный ластик.".into(),
                crate::editor::photo_document::PhotoTool::Fill => "G · Заливка связанной области выбранным цветом.".into(),
                crate::editor::photo_document::PhotoTool::Gradient => "U · Протяни градиент по холсту. Shift — шаг 45°, Esc — отмена.".into(),
                crate::editor::photo_document::PhotoTool::Eyedropper => "I · Пипетка. Взять цвет изображения.".into(),
                crate::editor::photo_document::PhotoTool::Move => "V · Перетащить содержимое выбранного слоя.".into(),
                crate::editor::photo_document::PhotoTool::Hand => "H / Пробел · Перемещать изображение в окне.".into(),
                crate::editor::photo_document::PhotoTool::Select(_) => "Выделение. Shift — добавить, Alt — вычесть, Shift+Alt — пересечь. Ctrl+D — снять.".into(),
                crate::editor::photo_document::PhotoTool::Transform => "Ctrl+T · Трансформация выделения или слоёв. Enter — применить, Esc — отменить.".into(),
            },
            super::ImageEditAction::AddLayer => "Создать прозрачный слой над выбранным.".into(),
            super::ImageEditAction::Selection(super::image_viewer::edit::selection::SelectionAction::FlipSelection(horizontal)) => {
                if horizontal {
                    "Отразить выделенный фрагмент слева направо."
                } else {
                    "Отразить выделенный фрагмент сверху вниз."
                }.into()
            }
            super::ImageEditAction::DrawingTool => "B · Рисование. Стрелка рядом или ПКМ — выбрать кисть или карандаш. Shift+B — переключить инструмент.".into(),
            super::ImageEditAction::PaintMenu => "Выбрать кисть со сглаживанием или пиксельный карандаш. Shift+B — переключить инструмент.".into(),
            super::ImageEditAction::EraserMenu => "Выбрать обычный или пиксельный ластик.".into(),
            super::ImageEditAction::DeleteLayer => "Удалить выбранный слой. Ctrl+Z отменяет удаление.".into(),
            super::ImageEditAction::Visible(_) => "Показать или скрыть слой.".into(),
            super::ImageEditAction::SelectLayer(_) => "Выбрать слой для рисования и редактирования.".into(),
            super::ImageEditAction::Layers(action) => super::image_viewer::edit::layers::ux::action_help(action),
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
        Action::BrowserDuplicateFile => "Создать копию сохранённого файла в той же папке. Файл слоёв изображения и настройки импорта копируются вместе с ним.".into(),
        Action::BrowserRecoverModel => "Открыть черновик завершившегося сеанса как отдельную модель рядом с исходным GLB.".into(),
        Action::BrowserRenameFile => "F2 · Переименовать выбранный файл. Расширение сохраняется, ссылки проекта обновляются.".into(),
        Action::ModelView(ModelViewAction::Edit(ModelEditAction::Menu)) =>
            "Выберите режим: объект, редактирование сетки, UV-развёртка или рисование. Tab возвращает к объекту.".into(),
        Action::ModelView(ModelViewAction::Paint(action)) => action.hint().into(),
        Action::ModelView(ModelViewAction::ExportGame) =>
            "Сохранить игровой GLB с готовыми материалами и анимациями.".into(),
        Action::ModelView(ModelViewAction::Uv(action)) => {
            use super::model_viewer::editing::uv::UvAction;
            match action {
                UvAction::Preview => "Построить быстрый предпросмотр с xatlas. Нажмите «Применить UV», чтобы сохранить результат в документе.",
                UvAction::Accept => "Применить предложенную UV-карту или отметить предложенный DA Wand участок для следующей развёртки.",
                UvAction::Dismiss => "Закрыть предложение и вернуться к текущей UV-карте.",
                UvAction::Brush => "Проведите ЛКМ в 3D для защиты области от швов. Shift + ЛКМ стирает защиту.",
                UvAction::ClearMask => "Снять защиту швов на активном объекте.",
                UvAction::Suggest => "Щёлкните по поверхности. Локальная модель DA Wand предложит связный участок вокруг этой грани.",
                UvAction::Heatmap => "Показать угловое искажение на модели и UV-карте. Бирюзовый соответствует 1×, красный — 2× и выше, пурпурный — схлопнутой грани.",
                UvAction::Unwrap => "Автоматически развернуть активный объект через PartUV Geometric. Расчёт идёт в фоне; Ctrl+Z отменяет результат.",
                UvAction::Pack => "Упаковать текущие острова в атлас с выбранным отступом. Швы сохраняются; Ctrl+Z отменяет упаковку.",
                UvAction::Cancel => "Остановить расчёт. Исходная UV-карта останется без изменений.",
                UvAction::Compare => "Показать исходную UV-карту и вернуться к результату без изменения документа.",
                UvAction::Checker => "Шахматная текстура помогает заметить растяжение. Это только просмотр: материалы модели сохраняются.",
                UvAction::Seams => "Показать границы UV-островов на 3D-модели.",
                UvAction::Fit => "Вписать атлас 0–1 в окно UV-карты. Колесо приближает под курсором.",
                UvAction::SelectAll => "Выделить все UV-острова активного объекта.",
                UvAction::Deselect => "Снять выделение островов.",
                UvAction::Rotate => "Повернуть выбранные UV-острова на 90° вокруг центра выделения.",
                UvAction::ScaleUp | UvAction::ScaleDown => "Изменить размер выделенных UV-островов. Геометрия модели не меняется.",
                UvAction::Resolution => "Разрешение следующего атласа: 1024, 2048 или 4096. Отступ задаётся в пикселях.",
                UvAction::Padding => "Расстояние между островами для следующей авторазвёртки: 4, 8 или 16 пикселей.",
                UvAction::Quality(_) => "Выберите баланс между растяжением поверхности и количеством швов для следующей развёртки.",
                UvAction::DetectUdim => super::model_viewer::editing::uv::UDIM_IMPORT_HINT,
            }.into()
        },
        Action::ModelView(ModelViewAction::Edit(ModelEditAction::Xray)) =>
            "Alt+Z · X-Ray: полупрозрачная модель, скрытые вершины и рёбра, выделение сквозь геометрию.".into(),
        Action::ModelView(ModelViewAction::Edit(ModelEditAction::ExtrudeTool)) =>
            "Потяните ручку «+» над выделением или нажмите E. Во время экструзии X, Y и Z ограничивают движение по мировой оси.".into(),
        Action::ModelView(ModelViewAction::Edit(ModelEditAction::BeginBevel { vertices: false })) =>
            "Фаска (Bevel) · Ctrl+B".into(),
        Action::ModelView(ModelViewAction::Edit(ModelEditAction::BeginLoopCut)) =>
            "Разрез петлёй (Loop Cut) · Ctrl+R".into(),
        Action::ModelView(ModelViewAction::Edit(ModelEditAction::BeginKnife { only_selected: false })) =>
            "Нож (Knife) · K".into(),
        Action::ModelView(ModelViewAction::Edit(ModelEditAction::Element(kind))) => {
            use crate::editor::model_mesh::Element;
            match kind {
                Element::Vertex => "1 · Выбирать вершины сетки.",
                Element::Edge => "2 · Выбирать рёбра сетки.",
                Element::Face => "3 · Выбирать грани сетки.",
            }.into()
        }
        Action::ViewModel(index) => format!(
            "Нажмите, чтобы рассмотреть «{}». Перетащите модель в 3D-вид, чтобы добавить её в сцену.",
            editor.project.catalog[index].label
        ),
        Action::Select(target) => format!(
            "{} · Выбрать для редактирования. Num . приблизит свободный вид.",
            editor.label(target)
        ),
        _ => return None,
    })
}

pub(in crate::editor) fn update(
    mut hints: ResMut<Hints>,
    viewer: Option<Res<crate::editor::ui::ImageViewer>>,
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
            Option<&Control>,
            Option<&Help>,
            &super::Hovered,
            &ComputedNode,
            &UiGlobalTransform,
        ),
        Without<super::drawer::FileCard>,
    >,
    mut panels: Query<&mut Node>,
    geometry: Query<&ComputedNode>,
    mut labels: Query<(&mut Text, &mut TextColor)>,
) {
    let target = if editor.pending.is_some()
        || viewer.as_ref().is_some_and(|v| v.edit.size_menu)
        || mouse.any_pressed([MouseButton::Left, MouseButton::Middle, MouseButton::Right])
    {
        None
    } else {
        controls
            .iter()
            .filter(|(_, control, help, hovered, computed, _)| {
                (control.is_some() || help.is_some())
                    && hovered.0
                    && computed.size().min_element() > 0.0
            })
            .min_by(|a, b| (a.4.size().x * a.4.size().y).total_cmp(&(b.4.size().x * b.4.size().y)))
            .or_else(|| {
                focus
                    .get()
                    .filter(|_| focus_visible.0)
                    .and_then(|entity| controls.get(entity).ok())
                    .filter(|(_, _, _, _, computed, _)| computed.size().min_element() > 0.0)
            })
    };
    let entity = target.map(|(entity, ..)| entity);
    if hints.target != entity {
        hints.target = entity;
        hints.elapsed = 0.0;
    } else {
        hints.elapsed += time.delta_secs();
    }
    let content = target.and_then(|(_, control, help, _, _, _)| {
        help.map(|h| h.0.clone())
            .or_else(|| control.and_then(|c| description(c.action, &editor, &browser)))
    });
    let shown = content.is_some() && hints.elapsed >= DELAY;
    if let Ok(mut panel) = panels.get_mut(hints.panel) {
        let display = if shown { Display::Flex } else { Display::None };
        if panel.display != display {
            panel.display = display;
        }
        if let Some((_, _, _, _, computed, transform)) = target.filter(|_| shown) {
            let scale = window.scale_factor();
            let center = transform.affine().translation / scale;
            let half = computed.size() / scale * 0.5;
            let x = (center.x - half.x).clamp(8.0, (window.width() - WIDTH - 8.0).max(8.0));
            let below = center.y + half.y + 7.0;
            let height = geometry
                .get(hints.panel)
                .map_or(0.0, |n| n.size().y / scale);
            let (top, bottom) = if height > 0.0 && below + height + 8.0 <= window.height() {
                (px(below), Val::Auto)
            } else {
                // Anchor the tooltip by its bottom edge so wrapped text grows upward.
                (Val::Auto, px(window.height() - (center.y - half.y) + 7.0))
            };
            if panel.left != px(x) || panel.top != top || panel.bottom != bottom {
                panel.left = px(x);
                panel.top = top;
                panel.bottom = bottom;
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
