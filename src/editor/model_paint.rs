//! Material Paint workspace state. Rendering and mesh editing stay isolated.
use super::*;
use crate::editor::model_document::Mode;
use crate::editor::model_paint_data::custom;
pub(in crate::editor) use crate::editor::model_paint_data::LayerKind;
use crate::editor::model_paint_data::mask::{
    Filter, FilterKind, Generator, GeneratorInput, GeneratorKind, MAX_FILTERS, MAX_MASK_LAYERS, Mask, MaskLayer,
};
use crate::editor::model_paint_data::{Blend, Layer, MAX_LAYER_DEPTH, MAX_LAYERS, Stack, Surface, texture_fill};
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
#[path = "model_paint_brush_defaults.rs"]
mod brush_defaults;
#[path = "model_paint_dock.rs"]
pub(super) mod dock;
#[path = "model_paint_sidebar.rs"]
pub(super) mod sidebar;
pub(in crate::editor) use sidebar::State as PaintDock;
pub(in crate::editor::ui) use sidebar::PanelBody;
#[path = "model_paint_engine.rs"]
pub(super) mod engine;
pub(in crate::editor) use engine::surface::{PolygonReplay, Stencil, StencilCache, StrokeCoverage};
#[path = "model_paint_export.rs"]
pub(in crate::editor) mod export;
#[path = "model_paint_bake.rs"]
pub(super) mod bake;
pub(in crate::editor) use bake::{import_bake_map_bits, import_generator_inputs, import_levels};
#[path = "model_paint_pathtrace.rs"]
pub(super) mod pathtrace;
#[path = "model_paint_post.rs"]
pub(super) mod post;

#[path = "model_paint_materials.rs"]
mod materials;
use materials::{MATERIALS, MaterialType};
#[path = "model_paint_presets.rs"]
pub(super) mod presets;
pub(in crate::editor) use presets::{ProjectGraph, project_fill, project_graph, project_mask};

pub(in crate::editor) fn project_preview_shader(
    payload: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    let graph = material_graph::read(payload)?;
    material_graph::file::validate_shader(&graph)?;
    Ok(material_graph::encode(&graph))
}
#[path = "model_paint_graph.rs"]
mod material_graph;
#[path = "model_paint_folder_graph.rs"]
mod folder_graph;
#[path = "model_paint_node_library.rs"]
mod node_library;
#[path = "model_paint_shader_preview.rs"]
pub(super) mod shader_preview;
pub(super) use material_graph::previews::PreviewPlugin as GraphPreviewPlugin;
#[path = "model_paint_shelf_preview.rs"]
mod shelf_preview;
#[path = "model_paint_smart.rs"]
mod smart;
use smart::SMART_MATERIALS;

#[path = "model_paint_tips.rs"]
mod tips;
#[path = "model_paint_ui.rs"]
mod ui;
#[path = "model_paint_sets.rs"]
pub(super) mod sets;
pub(super) use ui::{ShaderMaker, decorate_shader_code, refresh, refresh_graph, refresh_shader_maker, setup};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::editor) enum PaintTool {
    Brush,
    Eraser,
    Fill,
    Picker,
    Path,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::editor) enum FillMode {
    Triangle,
    #[default]
    Polygon,
    Loop,
    Flat,
    Part,
    Object,
    UvIsland,
    Set,
}

impl FillMode {
    const OPTIONS: [(&'static str, Self); 8] = [
        ("Треугольник", Self::Triangle),
        ("Полигон", Self::Polygon),
        ("Лента граней", Self::Loop),
        ("Поверхность", Self::Flat),
        ("Связная часть", Self::Part),
        ("Объект", Self::Object),
        ("UV-остров", Self::UvIsland),
        ("Материал", Self::Set),
    ];
}
impl PaintTool {
    pub fn label(self) -> &'static str {
        match self {
            Self::Brush => "Кисть",
            Self::Eraser => "Ластик",
            Self::Fill => "Заливка полигонов",
            Self::Picker => "Пипетка",
            Self::Path => "Контур",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub(in crate::editor) enum Panel {
    Layers,
    Maps,
    Sets,
    Preview,
    Shader,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::editor) enum View {
    Split,
    Three,
    Two,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::editor) enum Preview3d {
    #[default]
    Material,
    Channel(usize),
    Baked(bake::BakeMap),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::editor) enum Category {
    Materials,
    Smart,
    Brushes,
    Textures,
    Generators,
    Graphs,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub(in crate::editor) enum Symmetry {
    #[default]
    Off,
    X,
    Y,
    Z,
}
#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub(in crate::editor) enum BrushProjection {
    #[default]
    Screen = 0,
    Surface = 1,
    Uv = 2,
}
impl Symmetry {
    fn normal(self) -> Vec3 {
        match self {
            Self::Off => Vec3::ZERO,
            Self::X => Vec3::X,
            Self::Y => Vec3::Y,
            Self::Z => Vec3::Z,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::editor) enum Parameter {
    Size,
    Flow,
    Opacity,
    Hardness,
    Spacing,
    Stabilizer,
    BrushScatter,
    BrushSizeJitter,
    BrushAngleJitter,
    GrainStrength,
    GrainScale,
    GrainAngle,
    BrushAngleLimit,
    FillAngle,
    FillPadding,
    TextureTiling,
    TextureRotation,
    TextureOffsetU,
    TextureOffsetV,
    TextureSharpness,
    TextureNormalStrength,
    TextureHeightStrength,
    TextureOpacityStrength,
    TextureEmissionTint(usize),
    TextureSpecularStrength,
    FillColorVariation,
    FillRoughVariation,
    FillHeightVariation,
    FillVariationScale,
    FillVariationSeed,
    FillPartVariation,
    ExportPaddingWidth,
    ExportHeightStrength,
    CageRadius,
    CageStrength,
    TipAngle,
    Roughness,
    Metallic,
    BrushHeight,
    BrushAlpha,
    BrushSpecular,
    BrushEmission(usize),
    LayerOpacity,
    FilterValue(usize, FilterKind, usize),
}
impl Parameter {
    fn label(self) -> &'static str {
        match self {
            Self::Size => "Размер",
            Self::Flow => "Поток, %",
            Self::Opacity => "Сила, %",
            Self::Hardness => "Жёсткость, %",
            Self::Spacing => "Шаг, %",
            Self::Stabilizer => "Стабилизация, px",
            Self::BrushScatter => "Разброс, %",
            Self::BrushSizeJitter => "Случайный размер, %",
            Self::BrushAngleJitter => "Случайный угол, %",
            Self::GrainStrength => "Сила зерна, %",
            Self::GrainScale => "Масштаб зерна",
            Self::GrainAngle => "Поворот зерна, °",
            Self::BrushAngleLimit => "Предел наклона, °",
            Self::FillAngle => "Максимальный угол, °",
            Self::FillPadding => "Отступ по UV, пикс.",
            Self::TextureTiling => "Повтор",
            Self::TextureRotation => "Поворот, °",
            Self::TextureOffsetU => "Смещение U",
            Self::TextureOffsetV => "Смещение V",
            Self::TextureSharpness => "Резкость смешивания",
            Self::TextureNormalStrength => "Сила нормалей",
            Self::TextureHeightStrength => "Сила карты высоты",
            Self::TextureOpacityStrength => "Сила карты непрозрачности",
            Self::TextureEmissionTint(channel) => ["Оттенок свечения R", "Оттенок свечения G", "Оттенок свечения B"][channel],
            Self::TextureSpecularStrength => "Сила карты бликов",
            Self::FillColorVariation => "Разброс цвета, %",
            Self::FillRoughVariation => "Разброс шероховатости, %",
            Self::FillHeightVariation => "Разброс высоты, %",
            Self::FillVariationScale => "Масштаб шума",
            Self::FillVariationSeed => "Зерно шума",
            Self::FillPartVariation => "Разброс по деталям, %",
            Self::ExportPaddingWidth => "Ширина, пикс.",
            Self::ExportHeightStrength => "Сила рельефа из высоты",
            Self::CageRadius => "Радиус оболочки, px",
            Self::CageStrength => "Сила кисти оболочки",
            Self::TipAngle => "Поворот наконечника, °",
            Self::Roughness => "Шероховатость, %",
            Self::Metallic => "Металличность, %",
            Self::BrushHeight => "Высота кисти, %",
            Self::BrushAlpha => "Непрозрачность материала, %",
            Self::BrushSpecular => "Уровень блика, %",
            Self::BrushEmission(index) => ["Свечение R, %", "Свечение G, %", "Свечение B, %"][index],
            Self::LayerOpacity => "Непрозрачность, %",
            Self::FilterValue(_, kind, value) => kind.controls()[value].0,
        }
    }
    fn bounds(self) -> (f64, f64) {
        match self {
            Self::Size => (1., 800.),
            Self::Spacing => (1., 300.),
            Self::Stabilizer => (0., 200.),
            Self::BrushScatter | Self::BrushSizeJitter | Self::BrushAngleJitter => (0., 100.),
            Self::TipAngle | Self::GrainAngle => (-180., 180.),
            Self::GrainScale => (0.25, 64.),
            Self::BrushAngleLimit => (10., 90.),
            Self::FillAngle => (0., 180.),
            Self::FillPadding => (0., 16.),
            Self::TextureTiling => (0.05, 64.),
            Self::TextureRotation => (-180., 180.),
            Self::TextureOffsetU | Self::TextureOffsetV => (-1., 1.),
            Self::TextureSharpness => (1., 16.),
            Self::TextureNormalStrength | Self::TextureSpecularStrength => (0., 2.),
            Self::TextureOpacityStrength | Self::TextureEmissionTint(_) => (0., 1.),
            Self::TextureHeightStrength => (-2., 2.),
            Self::FillVariationScale => (0.5, 60.),
            Self::ExportPaddingWidth => (1., 256.),
            Self::ExportHeightStrength => (0., 8.),
            Self::CageRadius => (5., 300.),
            Self::CageStrength => (0.1, 4.),
            Self::FilterValue(_, kind, value) => {
                let (_, _, min, max) = kind.controls()[value];
                (min as f64, max as f64)
            }
            _ => (0., 100.),
        }
    }

    fn fill_parameter(self) -> bool {
        matches!(self, Self::TextureTiling | Self::TextureRotation | Self::TextureOffsetU
            | Self::TextureOffsetV | Self::TextureSharpness | Self::TextureNormalStrength | Self::TextureHeightStrength
            | Self::TextureOpacityStrength | Self::TextureEmissionTint(_) | Self::TextureSpecularStrength
            | Self::FillColorVariation | Self::FillRoughVariation | Self::FillHeightVariation
            | Self::FillVariationScale | Self::FillVariationSeed | Self::FillPartVariation)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(in crate::editor) enum ShelfMaterial {
    Basic(usize),
    Smart(usize),
    User(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::editor) enum ShelfPlacement {
    Top,
    Above(usize),
    Below(usize),
    Inside(usize),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::editor) enum PaintAction {
    Tool(PaintTool),
    Panel(Panel),
    View(View),
    Preview3d(Preview3d),
    Category(Category),
    BrushTip(Option<usize>),
    ImportBrushTip,
    SaveBrushDefaults,
    ResetBrushDefaults,
    ToggleRecovery,
    ApplyHistoryPreference(Entity),
    FollowStroke,
    PressureSize,
    PressureFlow,
    Grain(Option<usize>),
    GrainInvert,
    GrainUv,
    Symmetry(Symmetry),
    BrushProjection(BrushProjection),
    PaintThrough,
    FillMode(FillMode),
    FillLasso,
    FillThrough,
    FillErase,
    PathSmooth,
    PathClosed,
    PathAlign,
    PathApply,
    PathClear,
    Channel(usize),
    MultiChannelBrush,
    BrushChannel(usize),
    Add(LayerKind),
    Select(usize),
    RenameLayer(Entity),
    CollapseFolder(usize),
    Visible(usize),
    BlendMenu,
    Blend(Blend),
    Remove,
    Duplicate,
    ClearLayer,
    GroupLayer,
    Move(i8),
    MoveLayer(usize, ShelfPlacement),
    FolderMenu,
    MoveInto(usize),
    MoveOut,
    AddMask(bool),
    RemoveMask,
    EditMask,
    SelectLayerMask(usize),
    MaskValue(u8),
    PreviewMask,
    PathTrace,
    PostPanel,
    TraceSamples(i8),
    TraceBounces(i8),
    TraceGround,
    PreviewShader(Option<usize>),
    MaskLayerMenu,
    AddMaskLayer,
    AddGeneratorLayer(GeneratorKind),
    ApplyGenerator(GeneratorKind, Option<usize>),
    Generator(Option<usize>, GeneratorEdit),
    GeneratorInputs(Option<usize>),
    GeneratorInputPicker(Option<usize>),
    MaskLayer(usize, MaskLayerEdit),
    FilterMenu,
    AddFilter(FilterKind),
    ToggleFilter(usize),
    RemoveFilter(usize),
    MoveFilter(usize, i8),
    AdjustFilter(usize, usize, i8),
    Undo,
    Redo,
    Set(usize),
    SetMenu,
    SetHide(usize, usize),
    SetPin(usize, usize),
    ExportMaps(bool),
    ExportMesh,
    FillTextureImport,
    FillTextureRemove,
    FillTextureProjection(texture_fill::Projection),
    FillTextureOpacity(texture_fill::OpacitySource),
    FillVariationOptions,
    ExportPadding(export::Padding),
    ExportFileMenu,
    ExportFormat(export::Format),
    ExportDepth(export::Depth),
    ExportSize(Option<u32>),
    ExportBakedNormal,
    ExportRevealFolder,
    ExportTemplateMenu,
    ImportExportTemplate,
    ExportTemplate(export::Template),
    CopyExportTemplate,
    SaveExportTemplate,
    ExportOutput(usize),
    ExportComponent(usize),
    ExportSourceMenu,
    EditExportTemplate { recipe: usize, output: usize, channel: usize, edit: export::recipe::Edit },
    BakeStart,
    BakeCancel,
    BakeClearDiskCache,
    BakePickHigh,
    BakeClearHigh,
    BakePickCage,
    BakeClearCage,
    BakeAutoFitCage,
    BakeInspectCage,
    BakePaintCage,
    BakeResetCagePaint,
    BakeMatchNames,
    BakeFillMisses,
    BakePreferFrontFaces,
    BakeAverageNormals,
    BakeRange { front: bool, percent: u8 },
    BakeQuality(bake::Quality),
    BakeIdSource(bake::IdSource),
    BakeMapOptions,
    BakeAdjustLevels(bake::BakeMap, bake::levels::Field, i8),
    BakeResetLevels(bake::BakeMap),
    BakeFixTool(bake::BakeMap),
    BakeFixValue(u8),
    BakeClearFix(bake::BakeMap),
    BakeToggleMap(bake::BakeMap),
    BakeSelectAllMaps(bool),
    BakeCustomAdd(custom::Kind),
    BakeCustomRemove(u64),
    BakeCustomAdjust(u64, custom::Field, i8),
    BakeCustomRename(u64),
    BakeCustomDuplicate(u64),
    BakeCustomMove(u64, i8),
    BakeCustomGizmo(u64),
    BakeLivePreview,
    BakeShowOnModel,
    BakeLiveMasks,
    BakePreview(Option<bake::BakeMap>),
    Resolution(u32),
    Color(usize),
    Preset(usize),
    ShelfClearSearch,
    ShelfTypeMenu,
    ShelfType(Option<MaterialType>),
    AddMaterial(usize),
    AddSmartMaterial(usize),
    AddGraphMaterial(usize),
    AddGraphMask,
    ImportGraphFile,
    ImportShaderGraph,
    ExportGraphFile,
    ExportGraphPackage,
    GraphSelectNode(u64),
    GraphAdjust(u64, usize, usize, i8),
    GraphBitmapPick(material_graph::ParameterTarget),
    GraphMapMenu(material_graph::ParameterTarget),
    GraphRebake,
    GraphCanvas,
    GraphEdit(material_graph::Edit),
    ShaderToGraph { model: usize, definition: usize, library_revision: usize },
    ImportShelfMaterial,
    SaveShelfMaterial,
    ExportShelfMaterial,
    ExportShelfPackage,
    SaveShelfMask,
    ApplyShelfMaterial(usize),
    ApplyShelfMask(usize, Option<usize>),
    DropMaterial(ShelfMaterial, ShelfPlacement),
    DropMaterialOnSet(ShelfMaterial, usize, usize),
    Reset,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::editor) enum MaskLayerEdit {
    Toggle,
    Remove,
    Move(i8),
    Blend,
    Value(i8),
    Opacity(i8),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::editor) enum GeneratorEdit {
    Set(GeneratorKind),
    Remove,
    Invert,
    Adjust(usize, i8),
    IdColor([u8; 3]),
    IdTolerance(i8),
    Weight(GeneratorInput, i16),
    ResetWeights,
    EdgeWidth(i8),
    CustomWeight(u64, i16),
}

impl PaintAction {
    fn edits_graph(self) -> bool {
        matches!(self, Self::GraphAdjust(..) | Self::GraphEdit(_) | Self::GraphBitmapPick(_))
    }
    pub(in crate::editor) fn hint(self) -> &'static str {
        match self {
            Self::Tool(PaintTool::Brush) => {
                "1 / B · Кисть. Shift+клик проводит прямой мазок от конца предыдущего."
            }
            Self::Tool(PaintTool::Eraser) => "2 / E · Ластик. Shift+клик проводит прямой мазок от конца предыдущего.",
            Self::Tool(PaintTool::Picker) => "I · Пипетка материала.",
            Self::Tool(PaintTool::Fill) => "3 / P · Заливка полигонов.",
            Self::Tool(PaintTool::Path) => "4 / L · Контур. ЛКМ ставит и перемещает точки, ПКМ удаляет точку, Enter применяет контур.",
            Self::MultiChannelBrush => "Наносить один мазок на несколько каналов материала.",
            Self::BrushChannel(_) => "Включить или выключить канал для следующих мазков.",
            Self::Add(_) => "Добавить слой в стек выбранного набора текстур.",
            Self::Remove => "Удалить выбранный слой.",
            Self::Duplicate => "Создать копию выбранного слоя.",
            Self::ClearLayer => "Очистить все каналы выбранного слоя краски и сохранить его маску и настройки.",
            Self::GroupLayer => "Поместить выбранный слой или группу в новую группу на том же уровне.",
            Self::MoveLayer(..) => "Переместить слой или вложенную группу в отмеченное место.",
            Self::RenameLayer(_) => "Применить имя слоя · Enter.",
            Self::CollapseFolder(_) => "Свернуть или развернуть список слоёв группы.",
            Self::Undo => "Ctrl+Z · Отменить изменение стека слоёв.",
            Self::Redo => "Ctrl+Shift+Z · Повторить изменение стека слоёв.",
            Self::Visible(_) => "Включить или выключить слой.",
            Self::BlendMenu | Self::Blend(_) => "Выбрать режим смешивания слоя.",
            Self::Move(_) => "Изменить порядок слоя.",
            Self::FolderMenu | Self::MoveInto(_) => "Переместить слой или группу в другую группу.",
            Self::MoveOut => "Вынести слой или группу на уровень выше.",
            Self::AddMask(_) => "Добавить маску видимости к слою или группе.",
            Self::RemoveMask => "Удалить маску выбранного слоя.",
            Self::EditMask => "Переключить кисть между маской и содержимым слоя.",
            Self::SelectLayerMask(_) => "Выбрать маску этого слоя для рисования.",
            Self::MaskValue(_) => "Выбрать цвет кисти для маски.",
            Self::PreviewMask => "Показать маску вместо материала в UV-виде.",
            Self::PathTrace => "F10 · Переключить трассировку света в 3D-предпросмотре.",
            Self::PostPanel => "Показать настройки цвета и эффектов 3D-предпросмотра.",
            Self::TraceSamples(_) => "Число отсчётов на пиксель. Дополнительные отсчёты уменьшают шум.",
            Self::TraceBounces(_) => "Предельное число отражений света.",
            Self::TraceGround => "Добавить плоскость под моделью для теней в трассированном виде.",
            Self::PreviewShader(_) => "Выбрать шейдер 3D-предпросмотра рисования.",
            Self::MaskLayerMenu => "Открыть стек слоёв маски.",
            Self::AddMaskLayer => "Добавить заливку поверх нарисованной маски.",
            Self::AddGeneratorLayer(_) => "Добавить процедурный слой маски.",
            Self::ApplyGenerator(..) => "Применить генератор к маске слоя.",
            Self::Generator(_, GeneratorEdit::Set(_)) => "Выбрать источник процедурной маски.",
            Self::GeneratorInputs(_) => "Настроить вклад запечённых карт и процедурных узоров.",
            Self::GeneratorInputPicker(_) => "Добавить вход генератора маски.",
            Self::Generator(_, _) => "Изменить генератор маски.",
            Self::MaskLayer(_, MaskLayerEdit::Blend) => "Переключить режим смешивания заливки.",
            Self::MaskLayer(_, _) => "Изменить заливку маски.",
            Self::FilterMenu | Self::AddFilter(_) => "Добавить фильтр маски.",
            Self::ToggleFilter(_) => "Включить или выключить фильтр.",
            Self::RemoveFilter(_) => "Удалить фильтр маски.",
            Self::MoveFilter(_, _) => "Изменить порядок фильтров маски.",
            Self::AdjustFilter(_, _, _) => "Изменить параметр фильтра маски.",
            Self::Select(_) => "Выбрать слой и показать его свойства.",
            Self::Set(_) => "Выбрать объект и набор текстур модели.",
            Self::SetMenu => "Выбрать набор текстур прямо из панели слоёв. У каждого набора свой стек и история.",
            Self::SetHide(..) => "Показать или скрыть грани этого набора в режиме рисования.",
            Self::SetPin(..) => "Закрепить подсветку граней набора в 3D-виде.",
            Self::ExportMaps(true) => "Экспортировать карты всех наборов по выбранному шаблону.",
            Self::ExportMaps(false) => "Экспортировать карты текущего набора по выбранному шаблону.",
            Self::ExportMesh => "Сохранить сетку OBJ и материалы MTL с текущей UV-развёрткой.",
            Self::ExportTemplateMenu => "Показать шаблоны экспорта.",
            Self::ImportExportTemplate => "Добавить шаблоны из файла JSON Material Painter.",
            Self::CopyExportTemplate => "Создать редактируемую копию выбранного шаблона.",
            Self::SaveExportTemplate => "Записать шаблон в JSON, совместимый с Material Painter.",
            Self::ExportOutput(_) => "Выбрать выходную карту шаблона.",
            Self::ExportComponent(_) => "Выбрать канал выходной карты.",
            Self::ExportSourceMenu => "Выбрать источник значения канала.",
            Self::EditExportTemplate { .. } => "Изменить пользовательский шаблон экспорта.",
            Self::ExportTemplate(_) => "Выбрать набор выходных карт и способ упаковки каналов.",
            Self::ExportFileMenu => "Настроить формат, глубину цвета и разрешение файлов.",
            Self::ExportFormat(export::Format::Jpeg) => "Сохранить JPEG без альфа-канала.",
            Self::ExportFormat(_) => "Выбрать формат файлов текстур.",
            Self::ExportDepth(_) => "Выбрать глубину цвета выходных файлов.",
            Self::ExportSize(_) => "Задать разрешение файлов без изменения текстур модели.",
            Self::ExportBakedNormal => "Добавить запечённые нормали high-poly к рисованным нормалям и рельефу.",
            Self::ExportRevealFolder => "Открыть папку с файлами после успешного экспорта.",
            Self::FillTextureImport => "Выбрать изображение для активного канала слоя заливки.",
            Self::FillTextureRemove => "Убрать изображение из активного канала.",
            Self::FillTextureProjection(_) => "Задать проекцию изображений выбранного слоя.",
            Self::FillTextureOpacity(_) => "Выбрать источник непрозрачности в изображении.",
            Self::FillVariationOptions => "Настроить шум и разброс параметров по деталям выбранного слоя заливки.",
            Self::ExportPadding(padding) => match padding {
                export::Padding::Infinite => "Заполнить пустое пространство цветом ближайшего UV-острова.",
                export::Padding::Fixed => "Продлить цвета UV-островов на заданное число пикселей.",
                export::Padding::Transparent => "Продлить цвета до края и обнулить альфа-канал за пределами UV-островов.",
                export::Padding::None => "Экспортировать каналы без расширения цветов за пределы UV-островов.",
            },
            Self::BakeStart => "Запечь карты меша для выбранного набора и его разрешения.",
            Self::BakeAdjustLevels(..) => "Изменить уровни карты. Shift увеличивает шаг.",
            Self::BakeResetLevels(_) => "Сбросить уровни карты выбранного набора.",
            Self::BakeFixTool(_) => "Рисовать по запечённой карте. Ластик восстанавливает запекание.",
            Self::BakeClearFix(_) => "Удалить все мазки на этой карте.",
            Self::BakeFixValue(_) => "Задать значение, которое наносит кисть.",
            Self::BakeCancel => "Остановить запекание карт.",
            Self::BakeClearDiskCache => "Удалить сохранённые на диске результаты запекания. Открытые карты останутся доступны.",
            Self::BakePickHigh => "Выбрать GLB, OBJ или FBX с детальной геометрией для проекции.",
            Self::BakeClearHigh => "Убрать источник high-poly и вернуться к картам текущей модели.",
            Self::BakePickCage => "Выбрать проекционную оболочку GLB, OBJ или FBX с топологией текущего объекта.",
            Self::BakeClearCage => "Вернуться к лучам с заданной внешней дальностью.",
            Self::BakeAutoFitCage => "Подогнать дальность лучей по high-poly перед запеканием.",
            Self::BakeInspectCage => "Показать оболочку и попадания лучей в 3D до запекания.",
            Self::BakePaintCage => "ЛКМ расширяет оболочку, Ctrl+ЛКМ сужает, Shift+ЛКМ восстанавливает исходную дальность.",
            Self::BakeResetCagePaint => "Сбросить ручную правку оболочки текущего объекта.",
            Self::BakeMatchNames => "Сопоставить объекты по именам с учётом окончаний _low и _high.",
            Self::BakeFillMisses => "Заполнить промахи данными ближайшей поверхности high-poly в пределах зоны поиска.",
            Self::BakePreferFrontFaces => "Сначала искать лицевые стороны. Если их нет на луче, учитывать обратные.",
            Self::BakeAverageNormals => "Усреднять направления лучей на совпадающих вершинах и UV-швах.",
            Self::BakeRange { .. } => "Дальность лучей в процентах от диагонали видимой модели.",
            Self::BakeQuality(quality) => quality.hint(),
            Self::BakeIdSource(_) => "Задать источник цветов карты ID.",
            Self::BakeMapOptions => "Выбрать дополнительные карты для следующего запекания.",
            Self::BakeToggleMap(_) => "Включить или пропустить карту при следующем запекании.",
            Self::BakeSelectAllMaps(true) => "Запечь все встроенные карты.",
            Self::BakeSelectAllMaps(false) => "Запечь основные карты и добавленные пользовательские карты.",
            Self::BakeCustomAdd(_) => "Добавить пользовательскую карту запекания.",
            Self::BakeCustomRemove(_) => "Удалить настройки пользовательской карты.",
            Self::BakeCustomAdjust(..) => "Изменить настройки карты и запечь набор заново.",
            Self::BakeCustomRename(_) => "Применить имя карты.",
            Self::BakeCustomDuplicate(_) => "Скопировать настройки карты.",
            Self::BakeCustomMove(..) => "Переместить карту в списке.",
            Self::BakeCustomGizmo(_) => "Настроить направление или точку в 3D-виде.",
            Self::BakeLivePreview => "Обновлять выбранную пользовательскую карту при изменении её параметров.",
            Self::BakeShowOnModel => "Показывать выбранную карту на поверхности активного набора.",
            Self::BakeLiveMasks => "Показывать материал с живой картой во входах масок.",
            Self::BakePreview(_) => "Показать запечённую карту в UV-виде.",
            Self::Resolution(_) => "Изменить размер карт этого набора. Слои и маски масштабируются, история набора сбрасывается.",
            Self::Channel(_) => "Выбрать канал материала.",
            Self::Category(_) => "Открыть категорию библиотеки.",
            Self::BrushTip(_) => "Выбрать форму отпечатка кисти и ластика.",
            Self::ImportBrushTip => "Загрузить наконечник из изображения.",
            Self::SaveBrushDefaults => "Сохранить размер, жёсткость, шаг кисти и отступ UV-заливки для следующих запусков.",
            Self::ResetBrushDefaults => "Восстановить стандартные параметры кисти и отступа UV-заливки.",
            Self::ToggleRecovery => "Включить или выключить запись новых автокопий GLB.",
            Self::ApplyHistoryPreference(_) => "Сохранить настройку истории правок.",
            Self::FollowStroke => "Поворачивать наконечник по направлению движения.",
            Self::PressureSize => "Нажим пера управляет размером отпечатка.",
            Self::PressureFlow => "Нажим пера управляет потоком отдельного отпечатка.",
            Self::Grain(_) => "Выбрать текстуру зерна кисти.",
            Self::GrainInvert => "Поменять светлые и тёмные участки зерна.",
            Self::GrainUv => "Разместить зерно по UV вместо проекции на поверхность.",
            Self::Symmetry(_) => "Отражать мазки в 3D относительно оси модели. X переключает симметрию по X.",
            Self::BrushProjection(BrushProjection::Screen) => {
                "Рисовать с камеры по видимой поверхности."
            }
            Self::BrushProjection(BrushProjection::Surface) => {
                "Класть отпечаток на касательную плоскость поверхности."
            }
            Self::BrushProjection(BrushProjection::Uv) => {
                "Рисовать в координатах UV в точке под курсором."
            }
            Self::PaintThrough => "Закрашивать скрытые поверхности в экранной проекции.",
            Self::FillMode(mode) => match mode {
                FillMode::Triangle => "Залить один треугольник сетки.",
                FillMode::Polygon => "Залить целую грань модели.",
                FillMode::Loop => "Пройти по противоположным рёбрам через ленту четырёхугольников.",
                FillMode::Flat => "Залить соседние грани до излома, превышающего заданный угол.",
                FillMode::Part => "Залить соединённые рёбрами грани одной части модели.",
                FillMode::Object => "Залить все части объекта в активном наборе текстур.",
                FillMode::UvIsland => "Залить участок развёртки, ограниченный UV-швами.",
                FillMode::Set => "Залить геометрию всего активного набора текстур.",
            },
            Self::FillLasso => "Shift+перетаскивание рисует лассо. Без этой настройки используется рамка.",
            Self::FillThrough => "Включать скрытые грани при выделении рамкой или лассо в 3D.",
            Self::FillErase => "Стирать выбранные области. Ctrl временно переключает между стиранием и заливкой.",
            Self::PathSmooth => "Сгладить линию между опорными точками.",
            Self::PathClosed => "Соединить последнюю точку с первой.",
            Self::PathAlign => "Поворачивать наконечник по касательной к контуру.",
            Self::PathApply => "Enter · Провести кистью вдоль контура.",
            Self::PathClear => "Esc · Убрать все точки контура.",
            Self::Color(_) => "Выбрать цвет кисти.",
            Self::Preset(_) => "Использовать материал для кисти.",
            Self::ShelfClearSearch => "Очистить поиск ресурсов.",
            Self::ShelfTypeMenu => "Фильтровать по типу материала.",
            Self::ShelfType(_) => "Показать материалы выбранного типа.",
            Self::AddMaterial(_) => "Добавить материал как слой заливки.",
            Self::AddSmartMaterial(_) => "Добавить умный материал как группу слоёв.",
            Self::AddGraphMaterial(_) => "Рассчитать граф на GPU и добавить заливку.",
            Self::AddGraphMask => "Подключить выход графа к маске выбранного слоя.",
            Self::ImportGraphFile => "Добавить граф Material Painter из файла .mpgraph или ZIP-пакета.",
            Self::ImportShaderGraph => "Открыть граф шейдинга Material Painter из файла .mpgraph или ZIP-пакета.",
            Self::ExportGraphFile => "Сохранить выбранный граф в файл .mpgraph.",
            Self::ExportGraphPackage => "Сохранить граф, узлы WGSL и изображения в ZIP-пакет.",
            Self::GraphSelectNode(_) => "Показать параметры узла.",
            Self::GraphAdjust(..) => "Изменить параметр и пересчитать граф.",
            Self::GraphBitmapPick(_) => "Выбрать изображение для узла графа.",
            Self::GraphMapMenu(_) => "Выбрать запечённую карту модели.",
            Self::GraphRebake => "Обновить графы по запечённым картам.",
            Self::GraphCanvas => "Открыть или закрыть холст графа.",
            Self::GraphEdit(_) => "Изменить схему материала.",
            Self::ShaderToGraph { .. } => "Добавить узел WGSL в открытый граф.",
            Self::ImportShelfMaterial => "Импортировать пресет Material Painter из .mpsmart или ZIP-пакета.",
            Self::ExportShelfMaterial => "Экспортировать заливку или группу в файл .mpsmart.",
            Self::ExportShelfPackage => "Экспортировать заливку или группу с узлами WGSL в ZIP-пакет.",
            Self::SaveShelfMaterial => "Сохранить заливку или группу в библиотеке.",
            Self::SaveShelfMask => "Сохранить маску слоя в библиотеке.",
            Self::ApplyShelfMaterial(_) => "Добавить сохранённый материал.",
            Self::ApplyShelfMask(..) => "Заменить маску слоя из библиотеки.",
            Self::DropMaterial(..) | Self::DropMaterialOnSet(..) => "Добавить материал в выбранное место стека.",
            Self::View(View::Split) => "F1 · Показать модель и UV рядом.",
            Self::View(View::Three) => "F2 · Показать модель в 3D.",
            Self::View(View::Two) => "F3 · Показать UV-развёртку.",
            Self::Preview3d(Preview3d::Material) => "M · Вернуть отображение материала.",
            Self::Preview3d(_) => "Показать карту на 3D-модели. C переключает карты.",
            Self::Panel(_) => "Открыть панель рабочего пространства рисования.",
            Self::Reset => "Восстановить расположение панелей: 3D и UV рядом, библиотека снизу.",
        }
    }
}
struct ShaderGraphReturn {
    panel: Panel,
    material: (Option<(usize, usize)>, usize, bool),
    shader: Option<usize>,
    payload: Arc<serde_json::Value>,
}

struct Session {
    stack: Stack,
    undo: Vec<Stack>,
    redo: Vec<Stack>,
    history_limit: usize,
    tool: PaintTool,
    panel: Panel,
    view: View,
    category: Category,
    graph_node: Option<u64>,
    graph_canvas: bool,
    shader_graph_return: Option<ShaderGraphReturn>,
    folder_view: Option<folder_graph::View>,
    preview_shader: Option<usize>,
    path_trace: bool,
    preview_3d: Preview3d,
    uv_view: engine::uv_navigation::UvView,
    preview_bakes: Vec<bake::BakeMap>,
    post_panel: bool,
    trace_settings: pathtrace::Settings,
    shader_canvas: bool,
    shader_graphs: BTreeMap<usize, shader_preview::Document>,
    shader_import: Option<bevy::tasks::Task<Result<Option<material_painter_graph::Graph>, String>>>,
    shader_import_error: Option<String>,
    channel: usize,
    set: usize,
    set_menu: bool,
    color: usize,
    preset: usize,
    size: f32,
    flow: f32,
    opacity: f32,
    hardness: f32,
    spacing: f32,
    stabilizer: f32,
    brush_scatter: f32,
    brush_size_jitter: f32,
    brush_angle_jitter: f32,
    pressure_size: bool,
    pressure_flow: bool,
    grain: Option<usize>,
    grain_strength: f32,
    grain_scale: f32,
    grain_angle: f32,
    grain_invert: bool,
    grain_uv: bool,
    symmetry: Symmetry,
    projection: BrushProjection,
    paint_through: bool,
    angle_limit: f32,
    fill_mode: FillMode,
    fill_angle: f32,
    fill_variation_options: bool,
    fill_lasso: bool,
    fill_through: bool,
    fill_erase: bool,
    fill_padding: f32,
    path_smooth: bool,
    path_closed: bool,
    path_align: bool,
    path_count: usize,
    path_command: Option<PaintAction>,
    tip: Option<usize>,
    tip_angle: f32,
    follow_stroke: bool,
    roughness: f32,
    metallic: f32,
    multi_channel_brush: bool,
    brush_channels: u8,
    brush_height: f32,
    brush_alpha: f32,
    brush_specular: f32,
    brush_emission: [f32; 3],
    revision: u64,
    blend_menu: bool,
    folder_menu: bool,
    editing_mask: bool,
    mask_value: u8,
    show_mask: bool,
    mask_layer_menu: bool,
    open_generator_inputs: Option<Option<usize>>,
    generator_input_picker: bool,
    filter_menu: bool,
    binding: Option<(usize, usize)>,
    smart_loading: bool,
    graph_bake: Option<std::sync::Weak<bake::Maps>>,
    graph_surfaces: HashMap<(usize, usize), Arc<material_graph::surface::Source>>,
    graph_map_menu: Option<material_graph::ParameterTarget>,
    histories: HashMap<(usize, usize), (Vec<Stack>, Vec<Stack>)>,
    busy: bool,
    export_settings: export::prefs::Settings,
    export_template_menu: bool,
    export_output: usize,
    export_component: usize,
    export_source_menu: bool,
    export_file_menu: bool,
    bake_busy: bool,
    bake_cache_clearing: bool,
    bake_map_options: bool,
    bake_ready: bool,
    bake_picking: bool,
    bake_projection: bake::ProjectionSettings,
    bake_inspect_cage: bool,
    bake_paint_cage: bool,
    cage_radius: f32,
    cage_strength: f32,
    cage_edits: HashMap<usize, crate::editor::model_cage_data::Edit>,
    cage_snapshot: Arc<crate::editor::model_cage_data::Edits>,
    cage_edit_version: u64,
    bake_cage_status: String,
    bake_view: Option<bake::BakeMap>,
    bake_fix: Option<bake::BakeMap>,
    bake_fix_value: u8,
    bake_custom_gizmo: Option<u64>,
    bake_live: bool,
    bake_on_model: bool,
    bake_live_masks: bool,
    texture_size: u32,
    set_appearance: sets::Appearance,
    resize_request: Option<u32>,
    picked: Option<[u8; 3]>,
    queued: std::collections::VecDeque<PaintAction>,
}
impl Default for Session {
    fn default() -> Self {
        let defaults = brush_defaults::Settings::default();
        Self {
            stack: Stack::default(),
            undo: vec![],
            redo: vec![],
            history_limit: editing::recovery::DEFAULT_UNDO_STEPS,
            tool: PaintTool::Brush,
            panel: Panel::Layers,
            view: View::Split,
            category: Category::Materials,
            graph_node: None,
            graph_canvas: false,
            shader_graph_return: None,
            folder_view: None,
            preview_shader: None,
            path_trace: false,
            preview_3d: Preview3d::Material,
            uv_view: Default::default(),
            preview_bakes: Vec::new(),
            post_panel: false,
            trace_settings: pathtrace::Settings { precomposed: true, ..default() },
            shader_canvas: false,
            shader_graphs: BTreeMap::new(),
            shader_import: None,
            shader_import_error: None,
            channel: 0,
            set: 0,
            set_menu: false,
            color: 0,
            preset: 0,
            size: defaults.size,
            flow: 100.,
            opacity: 100.,
            hardness: defaults.hardness,
            spacing: defaults.spacing,
            stabilizer: 0.,
            brush_scatter: 0.,
            brush_size_jitter: 0.,
            brush_angle_jitter: 0.,
            pressure_size: true,
            pressure_flow: false,
            grain: None,
            grain_strength: 100.,
            grain_scale: 4.,
            grain_angle: 0.,
            grain_invert: false,
            grain_uv: false,
            symmetry: Symmetry::Off,
            projection: BrushProjection::Screen,
            paint_through: false,
            angle_limit: 85.,
            fill_mode: FillMode::default(),
            fill_angle: 30.,
            fill_variation_options: false,
            fill_lasso: false,
            fill_through: false,
            fill_erase: false,
            fill_padding: defaults.fill_padding,
            path_smooth: true,
            path_closed: false,
            path_align: true,
            path_count: 0,
            path_command: None,
            tip: None,
            tip_angle: 0.,
            follow_stroke: false,
            roughness: 55.,
            metallic: 0.,
            multi_channel_brush: false,
            brush_channels: 0b111,
            brush_height: 50.,
            brush_alpha: 100.,
            brush_specular: 50.,
            brush_emission: [0.; 3],
            revision: 0,
            blend_menu: false,
            folder_menu: false,
            editing_mask: false,
            mask_value: 255,
            show_mask: false,
            mask_layer_menu: false,
            open_generator_inputs: None,
            generator_input_picker: false,
            filter_menu: false,
            binding: None,
            smart_loading: false,
            graph_bake: None,
            graph_surfaces: HashMap::new(),
            graph_map_menu: None,
            histories: HashMap::new(),
            busy: false,
            bake_busy: false,
            export_settings: export::prefs::Settings::default(),
            export_template_menu: false,
            export_output: 0,
            export_component: 0,
            export_source_menu: false,
            export_file_menu: false,
            bake_cache_clearing: false,
            bake_map_options: false,
            bake_ready: false,
            bake_picking: false,
            bake_projection: bake::ProjectionSettings::default(),
            bake_inspect_cage: false,
            bake_paint_cage: false,
            cage_radius: 40.,
            cage_strength: 1.,
            cage_edits: HashMap::new(),
            cage_snapshot: Arc::default(),
            cage_edit_version: 0,
            bake_cage_status: String::new(),
            bake_view: None,
            bake_fix: None,
            bake_fix_value: 50,
            bake_custom_gizmo: None,
            bake_live: true,
            bake_on_model: true,
            bake_live_masks: false,
            texture_size: crate::editor::model_paint_data::DEFAULT_SIZE,
            set_appearance: Default::default(),
            resize_request: None,
            picked: None,
            queued: Default::default(),
        }
    }
}
impl Session {
    fn inherit_brush(&mut self, source: &Self) {
        self.size = source.size;
        self.flow = source.flow;
        self.opacity = source.opacity;
        self.hardness = source.hardness;
        self.spacing = source.spacing;
        self.stabilizer = source.stabilizer;
        self.brush_scatter = source.brush_scatter;
        self.brush_size_jitter = source.brush_size_jitter;
        self.brush_angle_jitter = source.brush_angle_jitter;
        self.pressure_size = source.pressure_size;
        self.pressure_flow = source.pressure_flow;

        self.tip = source.tip;
        self.tip_angle = source.tip_angle;
        self.follow_stroke = source.follow_stroke;
        self.grain = source.grain;
        self.grain_strength = source.grain_strength;
        self.grain_scale = source.grain_scale;
        self.grain_angle = source.grain_angle;
        self.grain_invert = source.grain_invert;
        self.grain_uv = source.grain_uv;
        self.symmetry = source.symmetry;
        self.projection = source.projection;
        self.paint_through = source.paint_through;
        self.angle_limit = source.angle_limit;

        self.color = source.color;
        self.picked = source.picked;
        self.preset = source.preset;
        self.roughness = source.roughness;
        self.metallic = source.metallic;
        self.multi_channel_brush = source.multi_channel_brush;
        self.brush_channels = source.brush_channels;
        self.brush_height = source.brush_height;
        self.brush_alpha = source.brush_alpha;
        self.brush_specular = source.brush_specular;
        self.brush_emission = source.brush_emission;
        self.mask_value = source.mask_value;
        self.bake_fix_value = source.bake_fix_value;
    }

    fn showing_bake(&self) -> bool {
        self.panel == Panel::Maps && self.bake_view.is_some()
    }

    fn active_bake_fix(&self) -> Option<bake::BakeMap> {
        self.bake_fix.filter(|map| {
            self.bake_ready && self.showing_bake() && self.bake_view == Some(*map)
                && !self.bake_live_masks && !self.cage_brush_active() && !self.custom_gizmo_active()
        })
    }

    fn showing_map_on_model(&self) -> bool {
        self.showing_bake() && self.bake_on_model && self.view != View::Two
    }

    fn diagnostic_channel(&self) -> Option<usize> {
        match self.preview_3d {
            Preview3d::Channel(channel) if self.view != View::Two && self.panel != Panel::Shader => Some(channel),
            _ => None,
        }
    }

    fn diagnostic_bake(&self) -> Option<bake::BakeMap> {
        if self.view == View::Two || self.panel == Panel::Shader {
            return None;
        }
        match self.preview_3d {
            Preview3d::Channel(_) => None,
            Preview3d::Baked(map) => Some(map),
            Preview3d::Material if self.showing_map_on_model() => self.bake_view,
            Preview3d::Material => None,
        }
    }

    fn diagnostic_3d(&self) -> bool {
        self.diagnostic_channel().is_some() || self.diagnostic_bake().is_some()
            || (self.bake_live_masks && self.view != View::Two)
    }

    fn paint_blocked_by_bake(&self) -> bool {
        (self.showing_bake() || self.diagnostic_bake().is_some())
            && self.tool != PaintTool::Picker && self.active_bake_fix().is_none()
    }

    fn custom_gizmo_active(&self) -> bool {
        self.panel == Panel::Maps && self.view != View::Two && self.bake_custom_gizmo.is_some()
    }

    fn cage_brush_active(&self) -> bool {
        self.panel == Panel::Maps && self.view != View::Two && self.bake_paint_cage
            && self.bake_projection.path.is_some() && self.bake_projection.cage.is_none()
    }

    fn can_paint_layer(&self) -> bool {
        self.stack.shown(self.stack.selected)
            && ((self.editing_mask && self.has_mask())
                || self.stack.layers.get(self.stack.selected).is_some_and(|l| l.kind == LayerKind::Paint))
    }

    fn stroke_channels(&self) -> u16 {
        if self.editing_mask && self.has_mask() {
            1 << crate::editor::model_paint_data::MASK_TARGET
        } else if self.multi_channel_brush {
            u16::from(self.brush_channels)
        } else {
            1 << self.channel
        }
    }
    fn available(&self, action: PaintAction) -> bool {
        if let PaintAction::Preview3d(mode) = action {
            return match mode {
                Preview3d::Material => true,
                Preview3d::Channel(channel) => channel < CHANNELS.len(),
                Preview3d::Baked(map) => self.preview_bakes.contains(&map),
            };
        }
        if let PaintAction::BrushChannel(channel) = action {
            return self.multi_channel_brush && channel < 8 && self.brush_channels != 1 << channel;
        }
        if let Some(shader) = self.editing_shader() {
            match action {
                PaintAction::Undo => return shader.can_undo(),
                PaintAction::Redo => return shader.can_redo(),
                PaintAction::GraphAdjust(..) | PaintAction::GraphEdit(_) => return true,
                PaintAction::GraphBitmapPick(_) => return false,
                _ => {}
            }
        }
        match action {
            PaintAction::ApplyGenerator(_, target) => {
                return self.binding.is_some() && !self.busy && self.resize_request.is_none()
                    && self.stack.layers.get(target.unwrap_or(self.stack.selected)).is_some();
            }
            PaintAction::Preset(i) => return i < MATERIALS.len(),
            PaintAction::SaveShelfMaterial | PaintAction::ExportShelfMaterial | PaintAction::ExportShelfPackage => {
                return !self.busy && self.stack.layers.get(self.stack.selected).is_some_and(|layer| {
                    layer.kind == LayerKind::Fill || (layer.kind == LayerKind::Folder
                        && self.stack.layers[self.stack.block(self.stack.selected)].iter().any(|l| l.kind == LayerKind::Fill))
                });
            }
            PaintAction::SaveShelfMask => return !self.busy && self.has_mask(),
            PaintAction::ApplyShelfMaterial(i) => {
                return self.can_insert_material(ShelfMaterial::User(i), Some(self.insertion()));
            }
            PaintAction::ApplyShelfMask(_, target) => {
                return self.binding.is_some() && !self.busy && !self.smart_loading && self.resize_request.is_none()
                    && self.stack.layers.get(target.unwrap_or(self.stack.selected)).is_some();
            }
            PaintAction::AddSmartMaterial(i) => {
                return self.can_insert_material(ShelfMaterial::Smart(i), Some(self.insertion()));
            }
            PaintAction::AddGraphMaterial(i) => {
                return self.binding.is_some() && !self.busy && self.resize_request.is_none()
                    && !self.smart_loading && self.stack.layers.len() < MAX_LAYERS
                    && i < material_graph::PRESETS.len();
            }
            PaintAction::AddGraphMask => {
                return !self.busy && !self.smart_loading && self.binding.is_some()
                    && self.resize_request.is_none()
                    && self.stack.layers.get(self.stack.selected)
                        .is_some_and(|layer| layer.mask.as_ref().is_none_or(|mask| mask.graph.is_none()));
            }
            PaintAction::ImportShaderGraph => {
                return self.panel == Panel::Preview && self.shader_import.is_none();
            }
            PaintAction::ImportGraphFile => {
                return self.binding.is_some() && !self.busy && !self.smart_loading
                    && self.resize_request.is_none() && self.stack.layers.len() < MAX_LAYERS;
            }
            PaintAction::ExportGraphFile | PaintAction::ExportGraphPackage => {
                return !self.smart_loading && self.graph().is_some() && self.active_folder_view().is_none();
            }
            PaintAction::GraphCanvas => return self.graph().is_some()
                || (self.panel == Panel::Layers && !self.editing_mask
                    && folder_graph::can_open(&self.stack)),
            PaintAction::PreviewShader(value) => return value.is_none_or(|id| {
                id < shader_preview::presets().len() || self.shader_graphs.contains_key(&id)
            }),
            PaintAction::GraphRebake => return self.binding.is_some() && !self.busy
                && !self.smart_loading && self.resize_request.is_none(),
            PaintAction::GraphAdjust(..) | PaintAction::GraphEdit(_) | PaintAction::GraphBitmapPick(_) => {
                if let Some(view) = self.active_folder_view() {
                    if self.busy || self.smart_loading || self.resize_request.is_some() { return false; }
                    return match action {
                        PaintAction::GraphEdit(edit) => folder_graph::can_edit(&self.stack, &view.graph, edit),
                        PaintAction::GraphAdjust(id, param, _, _) => folder_graph::adjustment_allowed(&self.stack, &view.graph, id, param),
                        PaintAction::GraphBitmapPick(target) => self.binding.is_some() && folder_graph::bitmap_target(&self.stack, &view.graph, target),
                        _ => false,
                    };
                }
                return self.binding.is_some() && !self.busy && !self.smart_loading
                    && self.resize_request.is_none()
                    && self.graph().is_some();
            }
            PaintAction::AddMaterial(i) => {
                return self.can_insert_material(ShelfMaterial::Basic(i), Some(self.insertion()));
            }
            PaintAction::DropMaterial(material, target) => {
                return self.can_insert_material(material, self.shelf_insertion(target));
            }
            PaintAction::DropMaterialOnSet(..) => return self.available(PaintAction::Set(self.set)) && !self.smart_loading,
            PaintAction::FillTextureImport => return !self.busy && self.binding.is_some()
                && self.stack.layers.get(self.stack.selected).is_some_and(|layer| layer.kind == LayerKind::Fill),
            PaintAction::FillTextureRemove => return !self.busy && self.active_texture_fill()
                .is_some_and(|fill| fill.images[self.channel].is_some()),
            PaintAction::FillTextureProjection(_) => return !self.busy && self.active_texture_fill().is_some(),
            PaintAction::FillTextureOpacity(_) => return !self.busy && self.active_texture_fill()
                .is_some_and(|fill| fill.images[4].is_some()),
            PaintAction::FillVariationOptions => return self.stack.layers.get(self.stack.selected)
                .is_some_and(|layer| layer.kind == LayerKind::Fill),
            PaintAction::ExportMaps(_) => return !self.busy && self.resize_request.is_none() && self.binding.is_some(),
            PaintAction::ExportMesh => return !self.busy && self.resize_request.is_none(),
            PaintAction::BakeStart => return !self.busy && !self.bake_busy && !self.bake_picking && !self.bake_cache_clearing && self.binding.is_some(),
            PaintAction::BakeCancel => return self.bake_busy,
            PaintAction::BakeClearDiskCache => return !self.bake_busy && !self.bake_cache_clearing,
            PaintAction::BakePickHigh => return !self.bake_busy && !self.bake_picking,
            PaintAction::BakePickCage => return self.bake_projection.path.is_some() && !self.bake_busy && !self.bake_picking,
            PaintAction::BakeClearCage => return self.bake_projection.cage.is_some() && !self.bake_busy && !self.bake_picking,
            PaintAction::BakeAutoFitCage => return self.bake_projection.path.is_some() && self.bake_projection.cage.is_none() && !self.bake_busy && !self.bake_picking,
            PaintAction::BakeInspectCage => return self.bake_projection.path.is_some(),
            PaintAction::BakeQuality(_) | PaintAction::BakeIdSource(_)
            | PaintAction::BakeToggleMap(_) | PaintAction::BakeSelectAllMaps(_) => {
                return !self.bake_busy;
            }
            PaintAction::BakeAdjustLevels(..) | PaintAction::BakeResetLevels(_) => return self.bake_ready,
            PaintAction::BakeFixTool(_) | PaintAction::BakeClearFix(_) | PaintAction::BakeFixValue(_) => {
                return self.bake_ready && !self.busy && !self.bake_busy;
            }
            PaintAction::Tool(tool) if self.active_bake_fix().is_some() => {
                return matches!(tool, PaintTool::Brush | PaintTool::Eraser);
            }
            PaintAction::BakeCustomAdd(_) | PaintAction::BakeCustomRemove(_) | PaintAction::BakeCustomAdjust(..)
            | PaintAction::BakeCustomRename(_) | PaintAction::BakeCustomDuplicate(_) | PaintAction::BakeCustomMove(..)
            | PaintAction::BakeCustomGizmo(_) => {
                return !self.bake_busy && !self.bake_picking;
            }
            PaintAction::BakePaintCage => {
                return self.bake_projection.path.is_some() && self.bake_projection.cage.is_none() && !self.bake_busy && !self.bake_picking;
            }
            PaintAction::BakeResetCagePaint => return !self.bake_busy && self.binding.is_some_and(|(object, _)| self.cage_edits.contains_key(&object)),
            PaintAction::BakeClearHigh | PaintAction::BakeMatchNames | PaintAction::BakeFillMisses
            | PaintAction::BakePreferFrontFaces | PaintAction::BakeAverageNormals | PaintAction::BakeRange { .. } => {
                return self.bake_projection.path.is_some() && !self.bake_busy && !self.bake_picking;
            }
            PaintAction::BakePreview(map) => return map.is_none_or(|map| {
                (self.bake_live && matches!(map, bake::BakeMap::Custom(_)))
                    || (self.bake_ready && (!map.requires_high_poly() || self.bake_projection.path.is_some()))
            }),
            PaintAction::BakeLiveMasks => return self.bake_live_masks
                || (self.bake_live && matches!(self.bake_view, Some(bake::BakeMap::Custom(_)))),
            PaintAction::SetHide(object, slot) => return !self.busy && self.resize_request.is_none()
                && self.set_appearance.can_toggle((object, slot)),
            PaintAction::Resolution(size) => return !self.busy && self.binding.is_some()
                && self.resize_request.is_none() && self.texture_size != size
                && crate::editor::model_paint_data::RESOLUTIONS.contains(&size),
            PaintAction::PathApply => return self.tool == PaintTool::Path && self.path_count >= 2
                && self.can_paint_layer() && !self.paint_blocked_by_bake(),
            PaintAction::PathClear => return self.tool == PaintTool::Path && self.path_count != 0,
            _ => {}
        }
        if (self.busy || self.resize_request.is_some())
            && matches!(
                action,
                PaintAction::Add(_)
                    | PaintAction::Select(_)
                    | PaintAction::RenameLayer(_)
                    | PaintAction::CollapseFolder(_)
                    | PaintAction::Visible(_)
                    | PaintAction::BlendMenu
                    | PaintAction::Blend(_)
                    | PaintAction::Remove
                    | PaintAction::Duplicate
                    | PaintAction::ClearLayer
                    | PaintAction::GroupLayer
                    | PaintAction::Move(_)
                    | PaintAction::MoveLayer(..)
                    | PaintAction::FolderMenu
                    | PaintAction::MoveInto(_)
                    | PaintAction::MoveOut
                    | PaintAction::AddMask(_)
                    | PaintAction::RemoveMask
                    | PaintAction::EditMask
                    | PaintAction::SelectLayerMask(_)
                    | PaintAction::AddMaskLayer
                    | PaintAction::AddGeneratorLayer(_)
                    | PaintAction::Generator(..)
                    | PaintAction::MaskLayer(..)
                    | PaintAction::AddFilter(_)
                    | PaintAction::ToggleFilter(_)
                    | PaintAction::RemoveFilter(_)
                    | PaintAction::MoveFilter(_, _)
                    | PaintAction::AdjustFilter(_, _, _)
                    | PaintAction::Undo
                    | PaintAction::Redo
                    | PaintAction::Set(_)
            )
        {
            return false;
        }
        match action {
            PaintAction::AddMask(_) => self
                .stack
                .layers
                .get(self.stack.selected)
                .is_some_and(|layer| layer.mask.is_none()),
            PaintAction::PaintThrough => self.projection == BrushProjection::Screen,
            PaintAction::EditMask
            | PaintAction::RemoveMask
            | PaintAction::PreviewMask
            | PaintAction::MaskLayerMenu
            | PaintAction::FilterMenu => self.has_mask(),
            PaintAction::SelectLayerMask(index) => self.stack.layers.get(index)
                .is_some_and(|layer| layer.mask.is_some()),
            PaintAction::AddMaskLayer | PaintAction::AddGeneratorLayer(_) => {
                self.has_mask() && self.mask_layers().len() < MAX_MASK_LAYERS
            }
            PaintAction::GeneratorInputs(index) => self.generator(index).is_some(),
            PaintAction::GeneratorInputPicker(index) => {
                self.open_generator_inputs == Some(index) && self.generator(index).is_some()
            }
            PaintAction::Generator(index, action) => {
                let generator = self.generator(index);
                match action {
                    GeneratorEdit::Set(kind) => {
                        self.has_mask()
                            && index.is_none_or(|i| i < self.mask_layers().len())
                            && generator.is_none_or(|g| g.kind != kind)
                    }
                    GeneratorEdit::Remove => index.is_none() && generator.is_some(),
                    GeneratorEdit::Invert => generator.is_some(),
                    GeneratorEdit::IdColor(color) => generator.is_some_and(|g| {
                        g.weight(GeneratorInput::Id) != 0 && g.id_color != Some(color)
                    }),
                    GeneratorEdit::IdTolerance(direction) => generator.is_some_and(|g| {
                        g.weight(GeneratorInput::Id) != 0 && if direction > 0 {
                            g.id_tolerance < 100
                        } else {
                            g.id_tolerance > 1
                        }
                    }),
                    GeneratorEdit::Weight(input, value) => generator.is_some_and(|g| {
                        (-Generator::WEIGHT_LIMIT..=Generator::WEIGHT_LIMIT).contains(&value)
                            && g.weight(input) != value
                    }),
                    GeneratorEdit::ResetWeights => generator.is_some_and(|g| g.weights.is_some()),
                    GeneratorEdit::CustomWeight(id, weight) => generator.is_some_and(|g| {
                        id != 0 && (-200..=200).contains(&weight) && g.custom.iter()
                            .find(|input| input.id == id)
                            .map_or_else(|| weight != 0 && g.custom.iter().any(|input| input.id == 0),
                                |input| input.weight != weight)
                    }),
                    GeneratorEdit::EdgeWidth(direction) => generator.is_some_and(|g| {
                        (g.weight(GeneratorInput::Edges) != 0 || g.weight(GeneratorInput::Creases) != 0)
                            && if direction > 0 { g.edge_width < 500 } else { g.edge_width > 2 }
                    }),
                    GeneratorEdit::Adjust(i, direction) => generator.is_some_and(|g| {
                        Generator::CONTROLS.get(i).is_some_and(|&(_, min, max, _)| {
                            if direction > 0 {
                                g.values[i] < max
                            } else {
                                g.values[i] > min
                            }
                        })
                    }),
                }
            }
            PaintAction::MaskLayer(i, action) => self.mask_layers().get(i).is_some_and(|layer| {
                let adjustable = |value, direction| {
                    if direction > 0 {
                        value < 100
                    } else {
                        value > 0
                    }
                };
                match action {
                    MaskLayerEdit::Move(direction) => i
                        .checked_add_signed(direction as isize)
                        .is_some_and(|to| to < self.mask_layers().len()),
                    MaskLayerEdit::Value(direction) => adjustable(layer.value, direction),
                    MaskLayerEdit::Opacity(direction) => adjustable(layer.opacity, direction),
                    _ => true,
                }
            }),
            PaintAction::AddFilter(_) => self.has_mask() && self.filters().len() < MAX_FILTERS,
            PaintAction::ToggleFilter(i) | PaintAction::RemoveFilter(i) => i < self.filters().len(),
            PaintAction::MoveFilter(i, direction) => {
                i < self.filters().len()
                    && (i as isize + direction as isize)
                        .try_into()
                        .is_ok_and(|to: usize| to < self.filters().len())
            }
            PaintAction::AdjustFilter(i, slot, direction) => {
                self.filters().get(i).is_some_and(|filter| {
                    filter
                        .kind
                        .controls()
                        .get(slot)
                        .is_some_and(|&(_, _, min, max)| {
                            if direction > 0 {
                                filter.value(slot) < max
                            } else {
                                filter.value(slot) > min
                            }
                        })
                })
            }
            PaintAction::MaskValue(_) => self.editing_mask && self.has_mask(),
            PaintAction::BlendMenu | PaintAction::Blend(_) => self
                .stack
                .layers
                .get(self.stack.selected)
                .is_some_and(|layer| layer.kind != LayerKind::Folder),
            PaintAction::Undo => !self.undo.is_empty(),
            PaintAction::Redo => !self.redo.is_empty(),
            PaintAction::Add(_) => {
                self.stack.layers.len() < MAX_LAYERS
                    && self
                        .stack
                        .layers
                        .get(self.stack.selected)
                        .is_none_or(|layer| {
                            layer.kind != LayerKind::Folder || layer.depth < MAX_LAYER_DEPTH
                        })
            }
            PaintAction::Remove => !self.stack.layers.is_empty(),
            PaintAction::RenameLayer(_) => self.stack.selected < self.stack.layers.len(),
            PaintAction::CollapseFolder(index) => self.stack.listed(index)
                && self.stack.layers[index].kind == LayerKind::Folder,
            PaintAction::Duplicate => {
                !self.stack.layers.is_empty()
                    && self.stack.layers.len() + self.stack.block(self.stack.selected).len()
                        <= MAX_LAYERS
            }
            PaintAction::ClearLayer => self.stack.layers.get(self.stack.selected).is_some_and(|layer| {
                layer.kind == LayerKind::Paint && layer.channels.iter().any(Option::is_some)
            }),
            PaintAction::GroupLayer => self.stack.selected < self.stack.layers.len()
                && self.stack.layers.len() < MAX_LAYERS
                && self.stack.layers[self.stack.block(self.stack.selected)].iter()
                    .all(|layer| layer.depth < MAX_LAYER_DEPTH),
            PaintAction::Move(direction) => {
                !self.stack.layers.is_empty() && self.stack.adjacent(direction).is_some()
            }
            PaintAction::MoveLayer(from, target) => self.layer_move_destination(from, target).is_some(),
            PaintAction::FolderMenu => {
                (0..self.stack.layers.len()).any(|i| self.stack.can_move_into(i))
            }
            PaintAction::MoveInto(folder) => self.stack.can_move_into(folder),
            PaintAction::MoveOut => self
                .stack
                .layers
                .get(self.stack.selected)
                .is_some_and(|layer| layer.depth != 0),
            _ => true,
        }
    }
    fn has_mask(&self) -> bool {
        self.stack
            .layers
            .get(self.stack.selected)
            .is_some_and(|layer| layer.mask.is_some())
    }
    fn filters(&self) -> &[Filter] {
        self.stack
            .layers
            .get(self.stack.selected)
            .and_then(|layer| layer.mask.as_ref())
            .map_or(&[], |mask| &mask.filters)
    }
    fn edit_filters(&mut self) -> &mut Vec<Filter> {
        Arc::make_mut(
            self.stack.layers[self.stack.selected]
                .mask
                .as_mut()
                .unwrap(),
        )
        .edit_filters()
    }
    fn mask_layers(&self) -> &[MaskLayer] {
        self.stack
            .layers
            .get(self.stack.selected)
            .and_then(|layer| layer.mask.as_ref())
            .map_or(&[], |mask| &mask.layers)
    }
    fn edit_mask_layers(&mut self) -> &mut Vec<MaskLayer> {
        Arc::make_mut(
            self.stack.layers[self.stack.selected]
                .mask
                .as_mut()
                .unwrap(),
        )
        .edit_layers()
    }
    fn generator(&self, index: Option<usize>) -> Option<Generator> {
        let mask = self.stack.layers.get(self.stack.selected)?.mask.as_ref()?;
        match index {
            Some(i) => mask.layers.get(i)?.generator,
            None => mask.generator,
        }
    }
    fn id_palette(&self) -> &[[u8; 3]] {
        self.stack.layers.get(self.stack.selected)
            .and_then(|layer| layer.mask.as_ref())
            .and_then(|mask| mask.context.as_ref())
            .map_or(&[], |context| context.id_palette())
    }
    fn disabled_generator_inputs(&self) -> u32 {
        self.stack.layers.get(self.stack.selected)
            .and_then(|layer| layer.mask.as_ref())
            .and_then(|mask| mask.context.as_ref())
            .map_or(0, |context| context.disabled_inputs())
    }

    fn apply_shelf_mask(&mut self, mask: Arc<Mask>, target: Option<usize>) {
        self.checkpoint();
        self.stack.selected = target.unwrap_or(self.stack.selected);
        if mask.graph.is_some() {
            self.graph_bake = None;
            if let Some(binding) = self.binding {
                self.graph_surfaces.remove(&binding);
            }
        }
        self.stack.layers[self.stack.selected].mask = Some(mask);
        let selected = self.stack.selected;
        folder_graph::sync_standalone_mask(&mut self.stack, selected);
        self.panel = Panel::Layers;
        self.editing_mask = true;
        self.mask_value = 0;
        self.filter_menu = false;
        self.mask_layer_menu = false;
        self.open_generator_inputs = None;
        self.generator_input_picker = false;
        self.revision += 1;
    }

    fn apply_generator(&mut self, kind: GeneratorKind, target: Option<usize>, custom_map: Option<u64>) {
        if !self.available(PaintAction::ApplyGenerator(kind, target)) {
            return;
        }
        self.checkpoint();
        self.stack.selected = target.unwrap_or(self.stack.selected);
        let mut generator = Generator::new(kind);
        if let Some(id) = custom_map.filter(|_| kind == GeneratorKind::CustomMap) {
            generator.custom[0] = custom::Input { id, weight: 100 };
        }
        let mask = Arc::make_mut(self.stack.layers[self.stack.selected].mask.get_or_insert_with(|| {
            Arc::new(Mask::new(crate::editor::model_paint_data::solid(
                self.texture_size, [255, 0, 0, 255],
            ), Vec::new()))
        }));
        mask.use_affine();
        *mask.edit_generator() = Some(generator);
        self.panel = Panel::Layers;
        self.editing_mask = true;
        self.mask_value = 0;
        self.folder_menu = false;
        self.filter_menu = false;
        self.mask_layer_menu = false;
        self.open_generator_inputs = None;
        self.generator_input_picker = false;
        self.revision += 1;
    }

    fn edit_generator(&mut self, index: Option<usize>, action: GeneratorEdit) {
        let mask = Arc::make_mut(
            self.stack.layers[self.stack.selected]
                .mask
                .as_mut()
                .unwrap(),
        );
        let generator = match index {
            Some(i) => &mut mask.edit_layers()[i].generator,
            None => {
                if matches!(action, GeneratorEdit::Set(_)) {
                    mask.use_affine();
                }
                mask.edit_generator()
            }
        };
        match action {
            GeneratorEdit::Set(kind) => *generator = Some(Generator::new(kind)),
            GeneratorEdit::Remove => *generator = None,
            GeneratorEdit::Invert => {
                let generator = generator.as_mut().unwrap();
                generator.invert = !generator.invert;
            }
            GeneratorEdit::IdColor(color) => generator.as_mut().unwrap().id_color = Some(color),
            GeneratorEdit::IdTolerance(direction) => {
                let generator = generator.as_mut().unwrap();
                generator.id_tolerance = (i16::from(generator.id_tolerance)
                    + i16::from(direction.signum())).clamp(1, 100) as u8;
            }
            GeneratorEdit::Weight(input, value) => generator.as_mut().unwrap().set_weight(input, value),
            GeneratorEdit::ResetWeights => generator.as_mut().unwrap().weights = None,
            GeneratorEdit::CustomWeight(id, weight) => {
                let generator = generator.as_mut().unwrap();
                let slot = generator.custom.iter().position(|input| input.id == id)
                    .or_else(|| generator.custom.iter().position(|input| input.id == 0)).unwrap();
                generator.custom[slot] = if weight == 0 { custom::Input::default() }
                    else { custom::Input { id, weight } };
            }
            GeneratorEdit::EdgeWidth(direction) => {
                let generator = generator.as_mut().unwrap();
                generator.edge_width = (generator.edge_width as i16 + i16::from(direction.signum()) * 2)
                    .clamp(2, 500) as u16;
            }
            GeneratorEdit::Adjust(i, direction) => {
                let generator = generator.as_mut().unwrap();
                let (_, min, max, step) = Generator::CONTROLS[i];
                generator.values[i] =
                    (generator.values[i] + direction.signum() as f32 * step).clamp(min, max);
            }
        }
    }
    fn value(&self, parameter: Parameter) -> f32 {
        let fill = self.active_texture_fill().map(|fill| fill.settings).unwrap_or_default();
        match parameter {
            Parameter::Size => self.size,
            Parameter::Flow => self.flow,
            Parameter::Opacity => self.opacity,
            Parameter::Hardness => self.hardness,
            Parameter::Spacing => self.spacing,
            Parameter::Stabilizer => self.stabilizer,
            Parameter::BrushScatter => self.brush_scatter,
            Parameter::BrushSizeJitter => self.brush_size_jitter,
            Parameter::BrushAngleJitter => self.brush_angle_jitter,
            Parameter::GrainStrength => self.grain_strength,
            Parameter::GrainScale => self.grain_scale,
            Parameter::GrainAngle => self.grain_angle,
            Parameter::BrushAngleLimit => self.angle_limit,
            Parameter::FillAngle => self.fill_angle,
            Parameter::FillPadding => self.fill_padding,
            Parameter::TextureTiling => fill.tiling,
            Parameter::TextureRotation => fill.rotation,
            Parameter::TextureOffsetU => fill.offset[0],
            Parameter::TextureOffsetV => fill.offset[1],
            Parameter::TextureSharpness => fill.sharpness,
            Parameter::TextureNormalStrength => fill.normal_strength,
            Parameter::TextureHeightStrength => fill.height_strength,
            Parameter::TextureOpacityStrength => fill.opacity_strength,
            Parameter::TextureEmissionTint(channel) => fill.emission_tint[channel],
            Parameter::TextureSpecularStrength => fill.specular_strength,
            Parameter::FillColorVariation => fill.variation.color * 100.,
            Parameter::FillRoughVariation => fill.variation.roughness * 100.,
            Parameter::FillHeightVariation => fill.variation.height * 100.,
            Parameter::FillVariationScale => fill.variation.scale,
            Parameter::FillVariationSeed => fill.variation.seed,
            Parameter::FillPartVariation => fill.variation.parts * 100.,
            Parameter::ExportPaddingWidth => self.export_settings.width as f32,
            Parameter::ExportHeightStrength => self.export_settings.file.normal.height_strength,
            Parameter::CageRadius => self.cage_radius,
            Parameter::CageStrength => self.cage_strength,
            Parameter::TipAngle => self.tip_angle,
            Parameter::Roughness => self.roughness,
            Parameter::Metallic => self.metallic,
            Parameter::BrushHeight => self.brush_height,
            Parameter::BrushAlpha => self.brush_alpha,
            Parameter::BrushSpecular => self.brush_specular,
            Parameter::BrushEmission(channel) => self.brush_emission[channel],
            Parameter::LayerOpacity => self
                .stack
                .layers
                .get(self.stack.selected)
                .map_or(100., |layer| layer.opacity),
            Parameter::FilterValue(i, _, value) => self
                .filters()
                .get(i)
                .map_or(0., |filter| filter.value(value)),
        }
    }
    fn set_value(&mut self, parameter: Parameter, value: f32) {
        if self.value(parameter) == value {
            return;
        }
        if parameter.fill_parameter() {
            if self.busy || !self.available(PaintAction::FillVariationOptions) || !value.is_finite() {
                return;
            }
            let (min, max) = parameter.bounds();
            let value = value.clamp(min as f32, max as f32);
            self.edit_texture_fill(|fill| {
                let settings = &mut fill.settings;
                match parameter {
                    Parameter::TextureTiling => settings.tiling = value,
                    Parameter::TextureRotation => settings.rotation = value,
                    Parameter::TextureOffsetU => settings.offset[0] = value,
                    Parameter::TextureOffsetV => settings.offset[1] = value,
                    Parameter::TextureSharpness => settings.sharpness = value,
                    Parameter::TextureNormalStrength => settings.normal_strength = value,
                    Parameter::TextureHeightStrength => settings.height_strength = value,
                    Parameter::TextureOpacityStrength => settings.opacity_strength = value,
                    Parameter::TextureEmissionTint(channel) => settings.emission_tint[channel] = value,
                    Parameter::TextureSpecularStrength => settings.specular_strength = value,
                    Parameter::FillColorVariation => settings.variation.color = value / 100.,
                    Parameter::FillRoughVariation => settings.variation.roughness = value / 100.,
                    Parameter::FillHeightVariation => settings.variation.height = value / 100.,
                    Parameter::FillVariationScale => settings.variation.scale = value,
                    Parameter::FillVariationSeed => settings.variation.seed = value,
                    Parameter::FillPartVariation => settings.variation.parts = value / 100.,
                    _ => unreachable!(),
                }
            });
            return;
        }
        if let Parameter::FilterValue(i, kind, slot) = parameter {
            if !self.busy
                && self.filters().get(i).is_some_and(|f| f.kind == kind)
                && kind.controls().get(slot).is_some()
                && value.is_finite()
            {
                self.checkpoint();
                self.edit_filters()[i].set_value(slot, value);
                self.revision += 1;
            }
            return;
        }
        if parameter == Parameter::LayerOpacity {
            if self.busy {
                return;
            }
            self.checkpoint();
        }
        let fill_channel = if !self.editing_mask
            && self.stack.layers.get(self.stack.selected).is_some_and(|l| l.kind == LayerKind::Fill)
        {
            match parameter {
                Parameter::Roughness => Some(1),
                Parameter::Metallic => Some(2),
                _ => None,
            }
        } else {
            None
        };
        if fill_channel.is_some() {
            if self.busy {
                return;
            }
            self.checkpoint();
        }
        match parameter {
            Parameter::Size => self.size = value,
            Parameter::Flow => self.flow = value,
            Parameter::Opacity => self.opacity = value,
            Parameter::Hardness => self.hardness = value,
            Parameter::Spacing => self.spacing = value,
            Parameter::Stabilizer => self.stabilizer = value,
            Parameter::BrushScatter => self.brush_scatter = value,
            Parameter::BrushSizeJitter => self.brush_size_jitter = value,
            Parameter::BrushAngleJitter => self.brush_angle_jitter = value,
            Parameter::GrainStrength => self.grain_strength = value,
            Parameter::GrainScale => self.grain_scale = value,
            Parameter::GrainAngle => self.grain_angle = value,
            Parameter::BrushAngleLimit => self.angle_limit = value,
            Parameter::FillAngle => self.fill_angle = value,
            Parameter::FillPadding => self.fill_padding = value,
            Parameter::TextureTiling | Parameter::TextureRotation | Parameter::TextureOffsetU
            | Parameter::TextureOffsetV | Parameter::TextureSharpness | Parameter::TextureNormalStrength | Parameter::TextureHeightStrength
            | Parameter::TextureOpacityStrength | Parameter::TextureEmissionTint(_) | Parameter::TextureSpecularStrength
            | Parameter::FillColorVariation | Parameter::FillRoughVariation | Parameter::FillHeightVariation
            | Parameter::FillVariationScale | Parameter::FillVariationSeed | Parameter::FillPartVariation => unreachable!(),
            Parameter::ExportPaddingWidth => self.export_settings.width = value.round().clamp(1., 256.) as u32,
            Parameter::ExportHeightStrength => self.export_settings.file.normal.height_strength = value.clamp(0., 8.),
            Parameter::CageRadius => self.cage_radius = value.clamp(5., 300.),
            Parameter::CageStrength => self.cage_strength = value.clamp(0.1, 4.),
            Parameter::TipAngle => self.tip_angle = value,
            Parameter::Roughness => self.roughness = value,
            Parameter::Metallic => self.metallic = value,
            Parameter::BrushHeight => self.brush_height = value,
            Parameter::BrushAlpha => self.brush_alpha = value,
            Parameter::BrushSpecular => self.brush_specular = value,
            Parameter::BrushEmission(channel) => self.brush_emission[channel] = value,
            Parameter::LayerOpacity => {
                if let Some(layer) = self.stack.layers.get_mut(self.stack.selected) {
                    layer.opacity = value;
                }
            }
            Parameter::FilterValue(..) => unreachable!(),
        }
        self.revision += 1;
        if let Some(channel) = fill_channel {
            let value = (value * 2.55).round() as u8;
            self.set_solid_fill(channel, [value; 3]);
        }
    }
    fn edit(&mut self, action: PaintAction) {
        if !self.available(action) {
            return;
        }
        let edit = matches!(
            action,
            PaintAction::Add(_)
                | PaintAction::AddMaterial(_)
                | PaintAction::DropMaterial(ShelfMaterial::Basic(_), _)
                | PaintAction::Visible(_)
                | PaintAction::Remove
                | PaintAction::Duplicate
                | PaintAction::ClearLayer
                | PaintAction::GroupLayer
                | PaintAction::Move(_)
                | PaintAction::MoveLayer(..)
                | PaintAction::MoveInto(_)
                | PaintAction::MoveOut
                | PaintAction::AddMask(_)
                | PaintAction::RemoveMask
                | PaintAction::AddMaskLayer
                | PaintAction::AddGeneratorLayer(_)
                | PaintAction::Generator(..)
                | PaintAction::MaskLayer(..)
                | PaintAction::AddFilter(_)
                | PaintAction::ToggleFilter(_)
                | PaintAction::RemoveFilter(_)
                | PaintAction::MoveFilter(_, _)
        ) || matches!(action, PaintAction::Blend(mode) if self.stack.layers[self.stack.selected].blend != mode);
        if edit {
            self.checkpoint();
        }
        let fill = self
            .stack
            .layers
            .get(self.stack.selected)
            .is_some_and(|l| l.kind == LayerKind::Fill)
            && !self.editing_mask
            && matches!(action, PaintAction::Color(_));
        if fill {
            if self.busy {
                return;
            }
            self.checkpoint();
        }
        match action {
            PaintAction::Tool(value) => self.tool = value,
            PaintAction::Panel(value) => {
                if value == self.panel {
                    return;
                }
                self.shader_graph_return = if value == Panel::Shader && self.graph_canvas_open() {
                    self.graph().map(|payload| ShaderGraphReturn {
                        panel: self.panel,
                        material: (self.binding, self.stack.selected, self.editing_mask),
                        shader: self.preview_shader,
                        payload: payload.clone(),
                    })
                } else {
                    None
                };
                self.set_menu = false;
                if value == Panel::Maps {
                    self.preview_3d = Preview3d::Material;
                }
                if self.panel == Panel::Preview || value == Panel::Preview {
                    self.graph_node = None;
                    self.graph_map_menu = None;
                }
                self.panel = value;
            }
            PaintAction::View(value) => self.view = value,
            PaintAction::Preview3d(mode) => {
                self.preview_3d = mode;
                if self.showing_map_on_model() {
                    self.bake_on_model = false;
                }
                self.bake_live_masks = false;
                self.bake_fix = None;
                if self.view == View::Two && mode != Preview3d::Material {
                    self.view = View::Split;
                }
            }
            PaintAction::Category(value) => self.category = value,
            PaintAction::GraphSelectNode(node) => self.graph_node = Some(node),
            PaintAction::GraphMapMenu(target) => {
                self.graph_map_menu = (self.graph_map_menu != Some(target)).then_some(target);
            }
            PaintAction::GraphCanvas => {
                if self.panel == Panel::Preview {
                    self.shader_canvas = !self.shader_canvas;
                } else {
                    self.graph_canvas = !self.graph_canvas;
                }
                self.graph_map_menu = None;
                self.refresh_folder_view();
            }
            PaintAction::BrushTip(tip) => self.tip = tip,
            PaintAction::ImportBrushTip | PaintAction::SaveBrushDefaults | PaintAction::ResetBrushDefaults => {}
            PaintAction::ToggleRecovery | PaintAction::ApplyHistoryPreference(_) => {}
            PaintAction::FollowStroke => self.follow_stroke = !self.follow_stroke,
            PaintAction::PressureSize => self.pressure_size = !self.pressure_size,
            PaintAction::PressureFlow => self.pressure_flow = !self.pressure_flow,
            PaintAction::Grain(grain) => self.grain = grain,
            PaintAction::GrainInvert => self.grain_invert = !self.grain_invert,
            PaintAction::GrainUv => self.grain_uv = !self.grain_uv,
            PaintAction::Symmetry(symmetry) => self.symmetry = symmetry,
            PaintAction::BrushProjection(projection) => self.projection = projection,
            PaintAction::PaintThrough => self.paint_through = !self.paint_through,
            PaintAction::FillMode(mode) => self.fill_mode = mode,
            PaintAction::FillLasso => self.fill_lasso = !self.fill_lasso,
            PaintAction::FillThrough => self.fill_through = !self.fill_through,
            PaintAction::FillErase => self.fill_erase = !self.fill_erase,
            PaintAction::PathSmooth => self.path_smooth = !self.path_smooth,
            PaintAction::PathClosed => self.path_closed = !self.path_closed,
            PaintAction::PathAlign => self.path_align = !self.path_align,
            PaintAction::PathApply | PaintAction::PathClear => self.path_command = Some(action),
            PaintAction::MultiChannelBrush => self.multi_channel_brush = !self.multi_channel_brush,
            PaintAction::BrushChannel(channel) if channel < 8 => {
                let channels = self.brush_channels ^ (1 << channel);
                if channels != 0 {
                    self.brush_channels = channels;
                }
            }
            PaintAction::BrushChannel(_) => {}
            PaintAction::Channel(value) => {
                self.channel = value.min(7);
                if self.editing_mask {
                    self.graph_node = None;
                    self.graph_map_menu = None;
                }
                self.editing_mask = false;
                self.show_mask = false;
            }
            PaintAction::Select(i) => self.select_layer(i),
            PaintAction::RenameLayer(_) => return,
            PaintAction::CollapseFolder(index) => {
                let layer = &mut self.stack.layers[index];
                layer.collapsed = !layer.collapsed;
                if layer.collapsed && self.stack.block(index).contains(&self.stack.selected)
                    && self.stack.selected != index
                {
                    self.select_layer(index);
                }
            }
            PaintAction::Add(kind) => {
                let name = match kind {
                    LayerKind::Paint => "Слой",
                    LayerKind::Fill => "Заливка",
                    LayerKind::Folder => "Группа",
                };
                let mut layer = Layer::new(format!("{name} {}", self.stack.layers.len() + 1), kind);
                if kind == LayerKind::Fill {
                    let color = self.brush_color();
                    layer.channels[0] = Some(crate::editor::model_paint_data::solid(
                        self.texture_size,
                        [color[0], color[1], color[2], 255],
                    ));
                    let r = (self.roughness * 2.55).round() as u8;
                    let m = (self.metallic * 2.55).round() as u8;
                    layer.channels[1] = Some(crate::editor::model_paint_data::solid(
                        self.texture_size,
                        [r, r, r, 255],
                    ));
                    layer.channels[2] = Some(crate::editor::model_paint_data::solid(
                        self.texture_size,
                        [m, m, m, 255],
                    ));
                }
                self.insert_layer(layer);
            }
            PaintAction::Visible(i) => {
                if let Some(layer) = self.stack.layers.get_mut(i) {
                    layer.visible = !layer.visible;
                }
            }
            PaintAction::BlendMenu => self.blend_menu = !self.blend_menu,
            PaintAction::Blend(mode) => {
                self.stack.layers[self.stack.selected].blend = mode;
                self.blend_menu = false;
            }
            PaintAction::Remove => {
                let block = self.stack.block(self.stack.selected);
                let at = block.start;
                self.stack.layers.drain(block);
                self.stack.selected = at.min(self.stack.layers.len().saturating_sub(1));
                self.folder_menu = false;
            }
            PaintAction::Duplicate => {
                let block = self.stack.block(self.stack.selected);
                let at = block.end;
                let mut layers = self.stack.layers[block].to_vec();
                folder_graph::rekey(&mut layers);
                layers.last_mut().unwrap().name.push_str(" — копия");
                self.stack.selected = at + layers.len() - 1;
                self.stack.layers.splice(at..at, layers);
            }
            PaintAction::ClearLayer => {
                self.stack.layers[self.stack.selected].channels = Default::default();
            }
            PaintAction::GroupLayer => {
                let block = self.stack.block(self.stack.selected);
                let at = block.end;
                let depth = self.stack.layers[self.stack.selected].depth;
                let count = self.stack.layers.iter().filter(|layer| layer.kind == LayerKind::Folder).count();
                for layer in &mut self.stack.layers[block] {
                    layer.depth += 1;
                }
                self.insert_layer_at(Layer::new(format!("Группа {}", count + 1), LayerKind::Folder), (at, depth));
                self.select_layer(at);
            }
            PaintAction::Move(direction) => {
                let at = self.stack.adjacent(direction).unwrap();
                let depth = self.stack.layers[self.stack.selected].depth;
                self.stack.move_block(at, depth);
            }
            PaintAction::MoveLayer(from, target) => {
                let (at, depth) = self.layer_move_destination(from, target).unwrap();
                self.select_layer(from);
                self.stack.move_block(at, depth);
            }
            PaintAction::FolderMenu => self.folder_menu = !self.folder_menu,
            PaintAction::MoveInto(folder) => {
                self.stack
                    .move_block(folder, self.stack.layers[folder].depth + 1);
                self.folder_menu = false;
            }
            PaintAction::MoveOut => {
                let folder = self.stack.parents(self.stack.selected).next().unwrap();
                self.stack
                    .move_block(folder + 1, self.stack.layers[folder].depth);
                self.folder_menu = false;
            }
            PaintAction::AddMask(white) => {
                let value = if white { 255 } else { 0 };
                self.stack.layers[self.stack.selected].mask = Some(Arc::new(Mask::new(
                    crate::editor::model_paint_data::solid(
                        self.texture_size,
                        [value, value, value, 255],
                    ),
                    Vec::new(),
                )));
                self.editing_mask = true;
                self.mask_value = 255 - value;
            }
            PaintAction::RemoveMask => {
                let selected = self.stack.selected;
                self.stack.layers[selected].mask = None;
                folder_graph::sync_standalone_mask(&mut self.stack, selected);
                self.editing_mask = false;
            }
            PaintAction::EditMask => {
                self.editing_mask = !self.editing_mask;
                self.graph_node = None;
                self.graph_map_menu = None;
            }
            PaintAction::SelectLayerMask(index) => {
                self.select_layer(index);
                self.editing_mask = true;
            }
            PaintAction::MaskValue(value) => self.mask_value = value,
            PaintAction::PreviewMask => self.show_mask = !self.show_mask,
            PaintAction::PathTrace => {
                self.path_trace = !self.path_trace;
                if self.path_trace {
                    self.preview_3d = Preview3d::Material;
                    self.preview_shader = None;
                    self.shader_canvas = false;
                    if self.view == View::Two {
                        self.view = View::Three;
                    }
                }
            }
            PaintAction::PostPanel => self.post_panel = !self.post_panel,
            PaintAction::TraceSamples(direction) => {
                let samples = self.trace_settings.samples;
                self.trace_settings.samples = if direction < 0 { samples / 2 } else { samples * 2 }
                    .clamp(16, 2048);
            }
            PaintAction::TraceBounces(direction) => {
                self.trace_settings.bounces = self.trace_settings.bounces
                    .saturating_add_signed(i32::from(direction)).clamp(1, 16);
            }
            PaintAction::TraceGround => self.trace_settings.ground = !self.trace_settings.ground,
            PaintAction::PreviewShader(value) => {
                self.preview_3d = Preview3d::Material;
                self.path_trace = false;
                if value != self.preview_shader {
                    self.graph_node = None;
                    self.graph_map_menu = None;
                }
                self.preview_shader = value;
                if let Some(id) = value {
                    self.shader_graphs.entry(id).or_insert_with(|| {
                        shader_preview::Document::new(&shader_preview::presets()[id])
                    });
                }
            }
            PaintAction::MaskLayerMenu => self.mask_layer_menu = !self.mask_layer_menu,
            PaintAction::AddMaskLayer => self.edit_mask_layers().push(MaskLayer::default()),
            PaintAction::AddGeneratorLayer(kind) => self.edit_mask_layers().push(MaskLayer {
                generator: Some(Generator::new(kind)),
                ..default()
            }),
            PaintAction::Generator(index, action) => {
                if let GeneratorEdit::Weight(input, value) = action
                    && value != 0
                    && self.generator(index).is_some_and(|g| g.weight(input) == 0)
                {
                    self.generator_input_picker = false;
                }
                if let GeneratorEdit::CustomWeight(id, weight) = action
                    && weight != 0
                    && self.generator(index).is_some_and(|g| g.custom.iter().all(|input| input.id != id))
                {
                    self.generator_input_picker = false;
                }
                self.edit_generator(index, action);
            }
            PaintAction::GeneratorInputs(index) => {
                self.open_generator_inputs =
                    (self.open_generator_inputs != Some(index)).then_some(index);
                self.generator_input_picker = false;
            }
            PaintAction::GeneratorInputPicker(_) => self.generator_input_picker = !self.generator_input_picker,
            PaintAction::MaskLayer(i, action) => {
                let adjust = |value: u8, direction: i8| {
                    (i16::from(value) + i16::from(direction.signum()) * 5).clamp(0, 100) as u8
                };
                let layers = self.edit_mask_layers();
                let layer = &mut layers[i];
                match action {
                    MaskLayerEdit::Toggle => layer.enabled = !layer.enabled,
                    MaskLayerEdit::Remove => {
                        layers.remove(i);
                    }
                    MaskLayerEdit::Move(direction) => {
                        layers.swap(i, i.checked_add_signed(direction as isize).unwrap());
                    }
                    MaskLayerEdit::Blend => layer.blend = layer.blend.next(),
                    MaskLayerEdit::Value(direction) => layer.value = adjust(layer.value, direction),
                    MaskLayerEdit::Opacity(direction) => {
                        layer.opacity = adjust(layer.opacity, direction)
                    }
                }
            }
            PaintAction::FilterMenu => self.filter_menu = !self.filter_menu,
            PaintAction::AddFilter(kind) => self.edit_filters().push(Filter::new(kind)),
            PaintAction::ToggleFilter(i) => {
                let filter = &mut self.edit_filters()[i];
                filter.enabled = !filter.enabled;
            }
            PaintAction::RemoveFilter(i) => {
                self.edit_filters().remove(i);
            }
            PaintAction::MoveFilter(i, direction) => {
                self.edit_filters()
                    .swap(i, (i as isize + direction as isize) as usize);
            }
            PaintAction::AdjustFilter(i, slot, direction) => {
                let filter = self.filters()[i];
                self.set_value(
                    Parameter::FilterValue(i, filter.kind, slot),
                    filter.value(slot) + direction.signum() as f32 * filter.kind.step(slot),
                );
                return;
            }
            PaintAction::Undo => {
                if let Some(stack) = self.undo.pop() {
                    self.redo.push(self.stack.clone());
                    Self::trim_history(&mut self.redo, self.history_limit);
                    self.stack = stack;
                    self.graph_bake = None;
                    if let Some(binding) = self.binding {
                        self.graph_surfaces.remove(&binding);
                    }
                }
            }
            PaintAction::Redo => {
                if let Some(stack) = self.redo.pop() {
                    self.undo.push(self.stack.clone());
                    Self::trim_history(&mut self.undo, self.history_limit);
                    self.stack = stack;
                    self.graph_bake = None;
                    if let Some(binding) = self.binding {
                        self.graph_surfaces.remove(&binding);
                    }
                }
            }
            PaintAction::Set(value) => {
                self.set = value;
                self.set_menu = false;
            }
            PaintAction::SetMenu => self.set_menu = !self.set_menu,
            PaintAction::SetHide(object, slot) => self.set_appearance.toggle_visibility((object, slot)),
            PaintAction::SetPin(object, slot) => self.set_appearance.toggle_pin((object, slot)),
            PaintAction::Resolution(size) => self.resize_request = Some(size),
            PaintAction::FillTextureImport => {}
            PaintAction::FillTextureRemove => {
                let channel = self.channel;
                self.edit_texture_fill(|fill| fill.images[channel] = None);
            }
            PaintAction::FillTextureProjection(projection) => {
                if self.active_texture_fill().is_some_and(|fill| fill.settings.projection != projection) {
                    self.edit_texture_fill(|fill| fill.settings.projection = projection);
                }
            }
            PaintAction::FillTextureOpacity(source) => {
                if self.active_texture_fill().is_some_and(|fill| fill.settings.opacity_source != source) {
                    self.edit_texture_fill(|fill| fill.settings.opacity_source = source);
                }
            }
            PaintAction::FillVariationOptions => self.fill_variation_options = !self.fill_variation_options,
            PaintAction::ExportPadding(padding) => self.export_settings.padding = padding,
            PaintAction::ExportFileMenu => {
                self.export_file_menu = !self.export_file_menu;
                self.export_template_menu = false;
            }
            PaintAction::ExportFormat(format) => self.export_settings.file.set_format(format),
            PaintAction::ExportDepth(depth) => {
                if self.export_settings.file.format.depths().contains(&depth) {
                    self.export_settings.file.depth = depth;
                }
            }
            PaintAction::ExportSize(size) => {
                self.export_settings.file.size =
                    size.filter(|size| crate::editor::model_paint_data::RESOLUTIONS.contains(size));
            }
            PaintAction::ExportBakedNormal => self.export_settings.file.normal.use_baked = !self.export_settings.file.normal.use_baked,
            PaintAction::ExportRevealFolder => self.export_settings.reveal_folder = !self.export_settings.reveal_folder,
            PaintAction::ExportTemplateMenu => {
                self.export_template_menu = !self.export_template_menu;
                self.export_file_menu = false;
            }
            PaintAction::ExportTemplate(template) => {
                self.export_settings.template = template;
                self.export_template_menu = matches!(template, export::Template::Custom(_));
                self.export_output = 0;
                self.export_component = 0;
                self.export_source_menu = false;
            }
            PaintAction::ExportOutput(output) => {
                self.export_output = output;
                self.export_component = 0;
                self.export_source_menu = false;
            }
            PaintAction::ExportComponent(channel) => self.export_component = channel,
            PaintAction::ExportSourceMenu => self.export_source_menu = !self.export_source_menu,
            PaintAction::ExportMaps(_) | PaintAction::ExportMesh | PaintAction::ImportExportTemplate | PaintAction::CopyExportTemplate
            | PaintAction::SaveExportTemplate | PaintAction::EditExportTemplate { .. } | PaintAction::BakeStart | PaintAction::BakeCancel | PaintAction::BakeClearDiskCache | PaintAction::BakePreview(_)
            | PaintAction::BakePickHigh | PaintAction::BakeClearHigh | PaintAction::BakePickCage | PaintAction::BakeClearCage | PaintAction::BakeAutoFitCage
            | PaintAction::BakeMatchNames | PaintAction::BakeFillMisses | PaintAction::BakePreferFrontFaces
            | PaintAction::BakeAverageNormals | PaintAction::BakeInspectCage | PaintAction::BakeRange { .. } => {}
            PaintAction::BakeQuality(_) | PaintAction::BakeIdSource(_) | PaintAction::BakeMapOptions
            | PaintAction::BakeToggleMap(_) | PaintAction::BakeSelectAllMaps(_)
            | PaintAction::BakeAdjustLevels(..) | PaintAction::BakeResetLevels(_)
            | PaintAction::BakeFixTool(_) | PaintAction::BakeClearFix(_) | PaintAction::BakeFixValue(_) => {}
            PaintAction::BakeCustomAdd(_) | PaintAction::BakeCustomRemove(_) | PaintAction::BakeCustomAdjust(..)
            | PaintAction::BakeCustomRename(_) | PaintAction::BakeCustomDuplicate(_) | PaintAction::BakeCustomMove(..)
            | PaintAction::BakeCustomGizmo(_) | PaintAction::BakeLivePreview | PaintAction::BakeShowOnModel | PaintAction::BakeLiveMasks => {}
            PaintAction::BakePaintCage | PaintAction::BakeResetCagePaint => {}
            PaintAction::Color(value) => {
                self.color = value.min(COLORS.len() - 1);
                self.picked = None;
            }
            PaintAction::Preset(value) => {
                self.preset = value;
                let material = &MATERIALS[value];
                self.roughness = material.roughness * 100.;
                self.metallic = material.metallic * 100.;
                self.picked = Some(material.rgb());
                self.tool = PaintTool::Brush;
                self.editing_mask = false;
                self.show_mask = false;
            }
            PaintAction::AddSmartMaterial(_) | PaintAction::AddGraphMaterial(_) | PaintAction::AddGraphMask
            | PaintAction::ImportGraphFile | PaintAction::ImportShaderGraph | PaintAction::ExportGraphFile | PaintAction::ExportGraphPackage
            | PaintAction::GraphAdjust(..) | PaintAction::GraphEdit(_) | PaintAction::GraphBitmapPick(_) | PaintAction::GraphRebake
            | PaintAction::ShaderToGraph { .. }
            | PaintAction::DropMaterial(ShelfMaterial::Smart(_) | ShelfMaterial::User(_), _)
            | PaintAction::DropMaterialOnSet(..)
            | PaintAction::ImportShelfMaterial | PaintAction::ExportShelfMaterial | PaintAction::ExportShelfPackage | PaintAction::SaveShelfMaterial | PaintAction::SaveShelfMask | PaintAction::ApplyShelfMaterial(_)
            | PaintAction::ApplyShelfMask(..) | PaintAction::ApplyGenerator(..)
            | PaintAction::ShelfClearSearch | PaintAction::ShelfTypeMenu | PaintAction::ShelfType(_) => unreachable!(),
            PaintAction::AddMaterial(value)
            | PaintAction::DropMaterial(ShelfMaterial::Basic(value), _) => {
                let insertion = match action {
                    PaintAction::DropMaterial(_, target) => self.shelf_insertion(target).unwrap(),
                    _ => self.insertion(),
                };
                self.insert_layer_at(MATERIALS[value].layer(self.texture_size), insertion);
                self.panel = Panel::Layers;
                self.editing_mask = false;
                self.show_mask = false;
            }
            PaintAction::Reset => {
                self.view = View::Split;
                self.panel = Panel::Layers;
                self.category = Category::Materials;
                self.graph_canvas = false;
            }
        }
        self.stack.reveal_selection();
        self.editing_mask &= self.has_mask();
        self.show_mask &= self.has_mask();
        self.revision += 1;
        if fill {
            self.set_solid_fill(0, self.brush_color());
        }
    }
    fn insertion(&self) -> (usize, usize) {
        self.stack.layers.get(self.stack.selected).map_or((0, 0), |selected| {
            if selected.kind == LayerKind::Folder {
                (self.stack.selected, selected.depth + 1)
            } else if selected.depth != 0 {
                (self.stack.selected + 1, selected.depth)
            } else {
                (self.stack.layers.len(), 0)
            }
        })
    }

    fn shelf_insertion(&self, target: ShelfPlacement) -> Option<(usize, usize)> {
        let (index, inside, above) = match target {
            ShelfPlacement::Top => return Some((self.stack.layers.len(), 0)),
            ShelfPlacement::Above(index) => (index, false, true),
            ShelfPlacement::Below(index) => (index, false, false),
            ShelfPlacement::Inside(index) => (index, true, false),
        };
        let layer = self.stack.layers.get(index)?;
        if inside {
            return (layer.kind == LayerKind::Folder).then_some((index, layer.depth + 1));
        }
        let block = self.stack.block(index);
        Some((if above { block.end } else { block.start }, layer.depth))
    }

    fn layer_move_destination(&self, from: usize, target: ShelfPlacement) -> Option<(usize, usize)> {
        let anchor = match target {
            ShelfPlacement::Above(index) | ShelfPlacement::Below(index) | ShelfPlacement::Inside(index) => index,
            ShelfPlacement::Top => return None,
        };
        let original_depth = self.stack.layers.get(from)?.depth;
        let block = self.stack.block(from);
        if block.contains(&anchor) {
            return None;
        }
        let (at, depth) = self.shelf_insertion(target)?;
        if depth == original_depth && (block.start..=block.end).contains(&at) {
            return None;
        }
        self.stack.layers[block].iter()
            .all(|layer| layer.depth - original_depth + depth <= MAX_LAYER_DEPTH)
            .then_some((at, depth))
    }

    fn can_insert_material(&self, material: ShelfMaterial, insertion: Option<(usize, usize)>) -> bool {
        if self.binding.is_none() || self.busy || self.resize_request.is_some() {
            return false;
        }
        let Some((_, depth)) = insertion else {
            return false;
        };
        match material {
            ShelfMaterial::Basic(index) => index < MATERIALS.len() && depth <= MAX_LAYER_DEPTH
                && self.stack.layers.len() < MAX_LAYERS,
            ShelfMaterial::User(_) => !self.smart_loading && depth <= MAX_LAYER_DEPTH
                && self.stack.layers.len() < MAX_LAYERS,
            ShelfMaterial::Smart(index) => !self.smart_loading && depth < MAX_LAYER_DEPTH
                && SMART_MATERIALS.get(index).is_some_and(|recipe| self.stack.layers.len() + recipe.len() <= MAX_LAYERS),
        }
    }

    fn insert_layer(&mut self, layer: Layer) {
        self.insert_layer_at(layer, self.insertion());
    }

    fn insert_layer_at(&mut self, mut layer: Layer, (at, depth): (usize, usize)) {
        layer.depth = depth;
        self.stack.layers.insert(at, layer);
        self.stack.selected = at;
        self.stack.reveal_selection();
    }

    fn insert_smart_material(&mut self, mut layers: Vec<Layer>, (at, depth): (usize, usize)) {
        self.graph_bake = None;
        if let Some(binding) = self.binding {
            self.graph_surfaces.remove(&binding);
        }
        folder_graph::rekey(&mut layers);
        let folder = layers.len() - 1;
        for layer in &mut layers {
            layer.depth += depth;
        }
        self.checkpoint();
        self.stack.layers.splice(at..at, layers);
        self.stack.selected = at + folder;
        self.stack.reveal_selection();
        self.panel = Panel::Layers;
        self.editing_mask = false;
        self.show_mask = false;
        self.revision += 1;
    }

    fn set_solid_fill(&mut self, channel: usize, [r, g, b]: [u8; 3]) {
        self.stack.layers[self.stack.selected].channels[channel] = Some(
            crate::editor::model_paint_data::solid(self.texture_size, [r, g, b, 255]),
        );
    }

    fn active_texture_fill(&self) -> Option<&texture_fill::Fill> {
        self.stack.layers.get(self.stack.selected)?.texture_fill.as_deref()
    }

    fn graph(&self) -> Option<&Arc<serde_json::Value>> {
        self.graph_data().map(|(payload, _)| payload)
    }

    fn graph_data(&self) -> Option<(&Arc<serde_json::Value>, &Arc<texture_fill::GraphImages>)> {
        self.graph_data_for(self.panel)
    }

    fn graph_data_for(&self, panel: Panel) -> Option<(&Arc<serde_json::Value>, &Arc<texture_fill::GraphImages>)> {
        if panel == Panel::Preview {
            return self.shader_graphs.get(&self.preview_shader?).map(shader_preview::Document::data);
        }
        if let Some(view) = self.folder_view_for(panel) {
            Some((&view.payload, &view.images))
        } else if self.editing_mask {
            let graph = self.stack.layers.get(self.stack.selected)?.mask.as_ref()?.graph.as_ref()?;
            Some((&graph.payload, &graph.images))
        } else {
            let fill = self.active_texture_fill()?;
            Some((fill.graph.as_ref()?, &fill.graph_images))
        }
    }

    fn active_folder_view(&self) -> Option<&folder_graph::View> {
        self.folder_view_for(self.panel)
    }

    fn folder_view_for(&self, panel: Panel) -> Option<&folder_graph::View> {
        let view = self.folder_view.as_ref()?;
        (panel == Panel::Layers && self.graph_canvas && !self.editing_mask
            && self.stack.layers.get(self.stack.selected).is_some_and(|layer|
                layer.kind == LayerKind::Folder && layer.graph_id == Some(view.folder)))
            .then_some(view)
    }

    fn refresh_folder_view(&mut self) {
        if self.panel == Panel::Shader
            && self.shader_graph_return.as_ref().is_some_and(|target| target.panel == Panel::Layers)
        {
            return;
        }
        if self.panel != Panel::Layers || !self.graph_canvas || self.editing_mask
            || !folder_graph::is_group(&self.stack)
        {
            self.folder_view = None;
            return;
        }
        if self.active_folder_view().is_some_and(|view| view.revision == self.revision) {
            return;
        }
        if !folder_graph::can_open(&self.stack) {
            self.folder_view = None;
            return;
        }
        let mut view = folder_graph::start(&mut self.stack, self.texture_size, self.revision);
        if let Some(previous) = &self.folder_view {
            if previous.payload == view.payload { view.payload = previous.payload.clone(); }
            if previous.images == view.images { view.images = previous.images.clone(); }
            if previous.labels == view.labels { view.labels = previous.labels.clone(); }
        }
        self.folder_view = Some(view);
    }

    fn edit_folder_view(&mut self, action: PaintAction) -> Result<Option<(Stack, folder_graph::Masks)>, String> {
        self.refresh_folder_view();
        if !self.available(action) { return Ok(None); }
        let Some(view) = self.active_folder_view() else { return Ok(None) };
        let mut graph = view.graph.clone();
        let mut next = self.stack.clone();
        let node = match action {
            PaintAction::GraphAdjust(id, parameter, component, direction) => {
                if !folder_graph::step_parameter(&mut graph, id, parameter, component, direction) {
                    return Ok(None);
                }
                if folder_graph::managed(&graph, id).is_some() {
                    if folder_graph::adjust(&mut next, &view.graph, &mut graph, id, parameter, self.texture_size)
                        && !crate::editor::model_paint_data::same_stack(&next, &self.stack)
                    {
                        self.commit_folder_view(next, Some(id));
                    }
                    return Ok(None);
                }
                Some(id)
            }
            PaintAction::GraphEdit(edit) => {
                if let material_graph::Edit::ClearBitmap(target) = edit
                    && folder_graph::clear_channel_image(&mut next, &graph, target)
                {
                    if !crate::editor::model_paint_data::same_stack(&next, &self.stack) {
                        self.commit_folder_view(next, self.graph_node);
                    }
                    return Ok(None);
                }
                folder_graph::edit_graph(&self.stack, &mut graph, edit)?
            }
            _ => return Ok(None),
        };
        if graph == view.graph { return Ok(None); }
        let masks = folder_graph::changed_masks(&self.stack, &view.graph, &graph);
        folder_graph::store(&mut next.layers[next.selected], &graph);
        if masks.iter().all(|(_, graph)| graph.is_none()) {
            for (index, _) in masks {
                if let Some(mask) = next.layers[index].mask.as_mut() {
                    Arc::make_mut(mask).clear_graph();
                }
            }
            self.commit_folder_view(next, node);
            Ok(None)
        } else {
            Ok(Some((next, masks)))
        }
    }

    fn commit_folder_view(&mut self, stack: Stack, node: Option<u64>) {
        self.checkpoint();
        self.stack = stack;
        self.graph_map_menu = None;
        self.graph_node = node.or(self.graph_node);
        self.revision += 1;
        self.refresh_folder_view();
    }

    fn graph_canvas_open(&self) -> bool {
        match self.panel {
            Panel::Preview => self.shader_canvas && self.editing_shader().is_some(),
            Panel::Shader => false,
            _ => self.graph_canvas && self.graph().is_some(),
        }
    }

    fn editing_shader(&self) -> Option<&shader_preview::Document> {
        if self.panel == Panel::Preview {
            self.shader_graphs.get(&self.preview_shader?)
        } else {
            None
        }
    }

    fn shader_graph_destination(&self, shading: bool) -> Option<&ShaderGraphReturn> {
        let target = self.shader_graph_return.as_ref()?;
        if self.panel != Panel::Shader || shading != (target.panel == Panel::Preview) {
            return None;
        }
        let same = if shading {
            self.shader_canvas && self.preview_shader == target.shader
        } else {
            self.graph_canvas && !self.busy && !self.smart_loading && self.resize_request.is_none()
                && (self.binding, self.stack.selected, self.editing_mask) == target.material
        };
        (same && self.graph_data_for(target.panel)
            .is_some_and(|(payload, _)| Arc::ptr_eq(payload, &target.payload)))
            .then_some(target)
    }

    fn insert_shader_node(&mut self, definition: usize, library_revision: usize) -> Result<PaintAction, String> {
        let library = material_graph::library();
        let node = library.defs.get(definition)
            .filter(|node| library_revision == node_library::revision() && node_library::available(&node.id))
            .ok_or("Библиотека узлов изменилась · выберите узел ещё раз")?;
        let target = self.shader_graph_destination(node.shading)
            .ok_or("Открытый граф изменился или не подходит для этого узла")?;
        let mut graph = material_graph::read(&target.payload)?;
        let edit = material_graph::Edit::Add(definition, ui::position_next_to_graph(&graph));
        if self.folder_view_for(target.panel).is_some()
            && !folder_graph::can_edit(&self.stack, &graph, edit)
        {
            return Err("Этот узел недоступен в графе группы".into());
        }
        edit.apply_for(&mut graph, None, node.shading)?;
        self.edit(PaintAction::Panel(target.panel));
        Ok(PaintAction::GraphEdit(edit))
    }

    fn finish_shader_import(&mut self, active: bool) {
        let Some(result) = self.shader_import.as_mut()
            .and_then(|task| bevy::tasks::block_on(bevy::tasks::poll_once(task)))
        else {
            return;
        };
        self.shader_import = None;
        match result {
            Ok(Some(graph)) => {
                let count = shader_preview::presets().len() + 1;
                let id = self.shader_graphs.last_key_value()
                    .map_or(count, |(&id, _)| count.max(id + 1));
                self.shader_graphs.insert(id, shader_preview::Document::new(&graph));
                if active && self.panel == Panel::Preview {
                    self.edit(PaintAction::PreviewShader(Some(id)));
                    self.shader_canvas = true;
                }
            }
            Ok(None) => {}
            Err(error) => {
                self.shader_import_error = Some(format!("Не удалось открыть граф · {error}"));
            }
        }
        self.revision += 1;
    }

    fn sync_project_shader(&mut self, preview: Option<&Arc<serde_json::Value>>) {
        let id = shader_preview::presets().len();
        if let Some(payload) = preview {
            let current = self.shader_graphs.get(&id);
            if current.is_none_or(|shader| !Arc::ptr_eq(shader.data().0, payload)) {
                self.shader_graphs.insert(id, shader_preview::Document::from_payload(payload.clone()));
                self.edit(PaintAction::PreviewShader(Some(id)));
                self.revision += 1;
            }
        } else if self.shader_graphs.remove(&id).is_some() {
            if self.preview_shader == Some(id) {
                self.edit(PaintAction::PreviewShader(None));
                self.shader_canvas = false;
            }
            self.revision += 1;
        }
    }

    fn viewport_layout(&self, draft_preview: bool) -> View {
        if (self.panel == Panel::Shader && draft_preview)
            || (self.shader_canvas && self.editing_shader().is_some())
        {
            View::Split
        } else {
            self.view
        }
    }

    fn edit_texture_fill(&mut self, edit: impl FnOnce(&mut texture_fill::Fill)) {
        self.checkpoint();
        self.stack.layers[self.stack.selected].edit_texture_fill(edit);
        self.revision += 1;
    }
    fn checkpoint(&mut self) {
        self.record_undo(self.stack.clone());
    }

    fn record_undo(&mut self, previous: Stack) {
        self.undo.push(previous);
        Self::trim_history(&mut self.undo, self.history_limit);
        self.redo.clear();
    }

    fn trim_history(history: &mut Vec<Stack>, limit: usize) {
        history.drain(..history.len().saturating_sub(limit));
    }

    fn set_history_limit(&mut self, limit: usize) {
        self.history_limit = limit;
        Self::trim_history(&mut self.undo, limit);
        Self::trim_history(&mut self.redo, limit);
        for (undo, redo) in self.histories.values_mut() {
            Self::trim_history(undo, limit);
            Self::trim_history(redo, limit);
        }
    }

    fn select_layer(&mut self, index: usize) {
        self.graph_node = None;
        self.graph_map_menu = None;
        self.stack.selected = index.min(self.stack.layers.len().saturating_sub(1));
        self.folder_menu = false;
        self.editing_mask = false;
        self.mask_layer_menu = false;
        self.filter_menu = false;
    }

    fn rename_layer(&mut self, name: &str) -> Result<(), &'static str> {
        let name = name.trim();
        if name.is_empty() || name.chars().any(char::is_control) || name.chars().count() > 120 {
            return Err("Введите имя слоя от 1 до 120 символов без управляющих знаков");
        }
        let index = self.stack.selected;
        if self.stack.layers[index].name != name {
            self.checkpoint();
            self.stack.layers[index].name = name.to_owned();
            self.revision += 1;
        }
        Ok(())
    }
    fn brush_color(&self) -> [u8; 3] {
        self.picked.unwrap_or(COLORS[self.color])
    }
    fn active(&self, action: PaintAction) -> bool {
        match action {
            PaintAction::Tool(value) => self.tool == value,
            PaintAction::BakeFixTool(map) => self.active_bake_fix() == Some(map),
            PaintAction::BakeFixValue(value) => self.bake_fix_value == value,
            PaintAction::Panel(value) => self.panel == value,
            PaintAction::View(value) => self.view == value,
            PaintAction::Preview3d(value) => self.preview_3d == value
                && (value != Preview3d::Material || !self.diagnostic_3d()),
            PaintAction::Category(value) => self.category == value,
            PaintAction::GraphCanvas => self.graph_canvas_open(),
            PaintAction::GraphSelectNode(node) => self.graph_node == Some(node),
            PaintAction::ExportPadding(padding) => self.export_settings.padding == padding,
            PaintAction::ExportFileMenu => self.export_file_menu,
            PaintAction::ExportFormat(format) => self.export_settings.file.format == format,
            PaintAction::ExportDepth(depth) => self.export_settings.file.depth == depth,
            PaintAction::ExportSize(size) => self.export_settings.file.size == size,
            PaintAction::ExportBakedNormal => self.export_settings.file.normal.use_baked,
            PaintAction::ExportRevealFolder => self.export_settings.reveal_folder,
            PaintAction::ExportTemplate(template) => self.export_settings.template == template,
            PaintAction::ExportTemplateMenu => self.export_template_menu,
            PaintAction::ExportOutput(output) => self.export_output == output,
            PaintAction::ExportComponent(channel) => self.export_component == channel,
            PaintAction::ExportSourceMenu => self.export_source_menu,
            PaintAction::BrushTip(value) => self.tip == value,
            PaintAction::FollowStroke => self.follow_stroke,
            PaintAction::PressureSize => self.pressure_size,
            PaintAction::PressureFlow => self.pressure_flow,
            PaintAction::Grain(grain) => self.grain == grain,
            PaintAction::GrainInvert => self.grain_invert,
            PaintAction::GrainUv => self.grain_uv,
            PaintAction::Symmetry(symmetry) => self.symmetry == symmetry,
            PaintAction::BrushProjection(projection) => self.projection == projection,
            PaintAction::FillMode(mode) => self.fill_mode == mode,
            PaintAction::FillLasso => self.fill_lasso,
            PaintAction::FillThrough => self.fill_through,
            PaintAction::FillErase => self.fill_erase,
            PaintAction::Resolution(size) => self.texture_size == size,
            PaintAction::PathSmooth => self.path_smooth,
            PaintAction::PathClosed => self.path_closed,
            PaintAction::PathAlign => self.path_align,
            PaintAction::PaintThrough => {
                self.projection == BrushProjection::Screen && self.paint_through
            }
            PaintAction::Channel(value) => self.channel == value,
            PaintAction::MultiChannelBrush => self.multi_channel_brush,
            PaintAction::BrushChannel(channel) => channel < 8 && self.brush_channels & (1 << channel) != 0,
            PaintAction::FillTextureProjection(value) => self.active_texture_fill()
                .is_some_and(|fill| fill.settings.projection == value),
            PaintAction::FillTextureOpacity(value) => self.active_texture_fill()
                .is_some_and(|fill| fill.settings.opacity_source == value),
            PaintAction::FillVariationOptions => self.fill_variation_options,
            PaintAction::Select(value) => self.stack.selected == value,
            PaintAction::EditMask => self.editing_mask,
            PaintAction::SelectLayerMask(index) => self.editing_mask && self.stack.selected == index,
            PaintAction::PreviewMask => self.show_mask,
            PaintAction::PathTrace => self.path_trace,
            PaintAction::PostPanel => self.post_panel,
            PaintAction::TraceGround => self.trace_settings.ground,
            PaintAction::PreviewShader(value) => !self.path_trace && self.preview_shader == value,
            PaintAction::MaskLayerMenu => self.mask_layer_menu,
            PaintAction::Generator(index, GeneratorEdit::Set(kind)) => {
                self.generator(index).is_some_and(|g| g.kind == kind)
            }
            PaintAction::GeneratorInputs(index) => self.open_generator_inputs == Some(index),
            PaintAction::GeneratorInputPicker(index) => self.open_generator_inputs == Some(index) && self.generator_input_picker,
            PaintAction::Generator(index, GeneratorEdit::Invert) => {
                self.generator(index).is_some_and(|g| g.invert)
            }
            PaintAction::Generator(index, GeneratorEdit::IdColor(color)) => {
                self.generator(index).is_some_and(|g| {
                    g.id_color.or_else(|| self.id_palette().first().copied()) == Some(color)
                })
            }
            PaintAction::MaskLayer(i, MaskLayerEdit::Toggle) => {
                self.mask_layers().get(i).is_some_and(|l| l.enabled)
            }
            PaintAction::FilterMenu => self.filter_menu,
            PaintAction::ToggleFilter(i) => self.filters().get(i).is_some_and(|f| f.enabled),
            PaintAction::MaskValue(value) => self.editing_mask && self.mask_value == value,
            PaintAction::BlendMenu => self.blend_menu,
            PaintAction::Blend(mode) => self
                .stack
                .layers
                .get(self.stack.selected)
                .is_some_and(|layer| layer.blend == mode),
            PaintAction::Set(value) => self.set == value,
            PaintAction::SetMenu => self.set_menu,
            PaintAction::SetHide(object, slot) => !self.set_appearance.visibility.shown(object, slot),
            PaintAction::SetPin(object, slot) => self.set_appearance.pinned.contains(&(object, slot)),
            PaintAction::BakePreview(map) => self.bake_view == map,
            PaintAction::BakeMatchNames => self.bake_projection.match_names,
            PaintAction::BakeFillMisses => self.bake_projection.fill_misses,
            PaintAction::BakePreferFrontFaces => self.bake_projection.prefer_front_faces,
            PaintAction::BakeAverageNormals => self.bake_projection.average_normals,
            PaintAction::BakeAutoFitCage => self.bake_projection.auto_fit,
            PaintAction::BakeQuality(quality) => self.bake_projection.quality == quality,
            PaintAction::BakeIdSource(source) => self.bake_projection.id_source == source,
            PaintAction::BakeMapOptions => self.bake_map_options,
            PaintAction::BakeToggleMap(map) => self.bake_projection.maps.contains(map),
            PaintAction::BakeInspectCage => self.bake_inspect_cage,
            PaintAction::BakePaintCage => self.bake_paint_cage,
            PaintAction::BakeCustomGizmo(id) => self.bake_custom_gizmo == Some(id),
            PaintAction::BakeLivePreview => self.bake_live,
            PaintAction::BakeShowOnModel => self.bake_on_model,
            PaintAction::BakeLiveMasks => self.bake_live_masks,
            PaintAction::BakeRange { front, percent } => {
                percent == if front { self.bake_projection.front } else { self.bake_projection.rear }
            }
            PaintAction::Color(value) => COLORS.get(value).is_some_and(|rgb| *rgb == self.brush_color()),
            PaintAction::Preset(value) => self.preset == value,
            PaintAction::ApplyGenerator(kind, _) => self.generator(None).is_some_and(|g| g.kind == kind),
            _ => false,
        }
    }
}
#[derive(Resource, Default)]
pub(in crate::editor) struct PaintState {
    sessions: HashMap<usize, Session>,
    brush_defaults: brush_defaults::Store,
    last_painted_model: Option<usize>,
    tips: tips::Library,
    shelf: ShelfFilter,
    library: presets::Library,
    texture_import: Option<TextureImport>,
    smart_import: Option<SmartImport>,
    shelf_drop: Option<(usize, sets::SetKey, ShelfMaterial)>,
    graph_export: Option<bevy::tasks::Task<Result<Option<std::path::PathBuf>, String>>>,
    #[cfg(test)]
    shader_graph_test_path: Option<Option<std::path::PathBuf>>,
    #[cfg(test)]
    graph_bitmap_test_path: Option<Option<std::path::PathBuf>>,
}

impl PaintState {
    pub(super) fn view_channel(&self, model: usize) -> Option<usize> {
        self.sessions.get(&model)?.diagnostic_channel()
    }

    pub(super) fn viewport_shortcut(
        &self,
        index: Option<usize>,
        keys: &ButtonInput<KeyCode>,
    ) -> Option<ModelViewAction> {
        let session = self.sessions.get(&index?)?;
        if session.panel == Panel::Shader || session.graph_canvas_open()
            || session.cage_brush_active() || session.custom_gizmo_active()
        {
            return None;
        }
        if keys.any_just_pressed([KeyCode::KeyF, KeyCode::Home]) {
            return Some(ModelViewAction::Fit);
        }
        if keys.just_pressed(KeyCode::KeyW) {
            return Some(ModelViewAction::Edit(editing::EditAction::Wireframe));
        }
        if keys.just_pressed(KeyCode::KeyM) || keys.just_pressed(KeyCode::KeyC) {
            let mode = if keys.just_pressed(KeyCode::KeyM) {
                Preview3d::Material
            } else {
                let current = session.diagnostic_bake().map_or(session.preview_3d, Preview3d::Baked);
                let order = [0, 1, 2, 3, 4, 5, 7, 6].into_iter().map(Preview3d::Channel)
                    .chain(session.preview_bakes.iter().copied().map(Preview3d::Baked))
                    .chain(std::iter::once(Preview3d::Material));
                order.clone().skip_while(|&mode| mode != current).nth(1)
                    .unwrap_or(Preview3d::Channel(0))
            };
            return Some(ModelViewAction::Paint(PaintAction::Preview3d(mode)));
        }
        for (key, action) in [
            (KeyCode::F1, PaintAction::View(View::Split)),
            (KeyCode::F2, PaintAction::View(View::Three)),
            (KeyCode::F3, PaintAction::View(View::Two)),
            (KeyCode::F10, PaintAction::PathTrace),
            (KeyCode::KeyX, PaintAction::Symmetry(if session.symmetry == Symmetry::X {
                Symmetry::Off
            } else {
                Symmetry::X
            })),
        ] {
            if keys.just_pressed(key) {
                return Some(ModelViewAction::Paint(action));
            }
        }
        let maps = session.showing_bake();
        for (codes, tool) in [
            (&[KeyCode::Digit1, KeyCode::KeyB][..], PaintTool::Brush),
            (&[KeyCode::Digit2, KeyCode::KeyE], PaintTool::Eraser),
            (&[KeyCode::Digit3, KeyCode::KeyP], PaintTool::Fill),
            (&[KeyCode::Digit4, KeyCode::KeyL], PaintTool::Path),
            (&[KeyCode::KeyI], PaintTool::Picker),
        ] {
            let supported = !maps || (session.active_bake_fix().is_some()
                && matches!(tool, PaintTool::Brush | PaintTool::Eraser));
            if supported && keys.any_just_pressed(codes.iter().copied()) {
                return Some(ModelViewAction::Paint(PaintAction::Tool(tool)));
            }
        }
        None
    }

    fn activate(&mut self, model: usize, history_limit: usize) {
        self.sessions.entry(model).or_insert_with(|| {
            let mut session = Session { history_limit, ..Session::default() };
            self.brush_defaults.settings().apply(&mut session);
            session
        });
        if let Some(previous) = self.last_painted_model.replace(model).filter(|&previous| previous != model)
            && let [Some(source), Some(target)] = self.sessions.get_disjoint_mut([&previous, &model])
        {
            target.inherit_brush(source);
            target.revision += 1;
        }
    }

    pub(super) fn blocks_view_navigation(&self, model: Option<usize>, draft_preview: bool) -> bool {
        model.and_then(|model| self.sessions.get(&model))
            .is_some_and(|session| session.panel == Panel::Shader && !draft_preview)
    }

    pub(super) fn has_3d_view(&self, model: Option<usize>, draft_preview: bool) -> bool {
        model.and_then(|model| self.sessions.get(&model))
            .is_none_or(|session| session.graph_canvas_open() || session.viewport_layout(draft_preview) != View::Two)
    }
}

#[derive(Default)]
struct ShelfFilter {
    query: String,
    material: Option<MaterialType>,
    menu: bool,
    clear: bool,
    revision: u64,
}

impl ShelfFilter {
    fn set_query(&mut self, value: &str) {
        let query = value.trim().to_lowercase();
        if self.query != query {
            self.query = query;
            self.revision += 1;
        }
    }

    fn matches(&self, name: &str) -> bool {
        self.query.is_empty() || name.to_lowercase().contains(&self.query)
    }

    fn allows_material(&self, kind: MaterialType) -> bool {
        self.material.is_none_or(|selected| kind == selected)
    }
}

struct SmartImport {
    model: usize,
    binding: (usize, usize),
    set: usize,
    stack: Stack,
    action: PaintAction,
    mask: bool,
    folder_graph: bool,
    insertion: (usize, usize),
    size: u32,
    maps: Option<Arc<bake::Maps>>,
    surface: Option<Arc<material_graph::surface::Source>>,
    task: bevy::tasks::Task<Result<Vec<Layer>, String>>,
}

struct TextureImport {
    model: usize,
    binding: (usize, usize),
    stack: Stack,
    channel: usize,
    task: bevy::tasks::Task<Result<Option<texture_fill::Bitmap>, String>>,
}
const CHANNELS: [&str; 8] = [
    "Цвет",
    "Шероховатость",
    "Металличность",
    "Высота",
    "Непрозрачность",
    "Свечение",
    "Нормали",
    "Блики",
];
const COLORS: [[u8; 3]; 8] = [
    [224, 186, 122],
    [224, 224, 230],
    [46, 48, 58],
    [164, 61, 68],
    [67, 118, 166],
    [80, 128, 104],
    [135, 96, 166],
    [193, 130, 84],
];
#[derive(Component, PartialEq)]
pub(super) struct GraphLabel(pub usize, pub bool);

#[derive(Component)]
pub(super) struct LayerNameField {
    model: usize,
    binding: Option<(usize, usize)>,
    layer: usize,
    original: String,
}

impl LayerNameField {
    fn matches(&self, model: usize, session: &Session) -> bool {
        self.model == model
            && self.binding == session.binding
            && self.layer == session.stack.selected
            && session.panel == Panel::Layers
            && session.stack.layers.get(self.layer).is_some_and(|layer| layer.name == self.original)
    }
}

pub(super) fn actions(
    mut messages: MessageReader<Action>,
    mut editor: ResMut<Editor>,
    mut paint: ResMut<PaintState>,
    mut editing: ResMut<editing::State>,
    mut images: ResMut<Assets<Image>>,
    time: Res<Time<Real>>,
    device: Option<Res<bevy::render::renderer::RenderDevice>>,
    queue: Option<Res<bevy::render::renderer::RenderQueue>>,
    graph_labels: Query<(&GraphLabel, &bevy::text::EditableText)>,
    mut layer_names: Query<(&LayerNameField, &mut bevy::text::EditableText),
        (Without<GraphLabel>, Without<ui::HistoryPreference>)>,
    baked: Res<bake::State>,
    (export_preferences, mut recovery, recovery_fields): (
        Res<export::prefs::Store>, ResMut<editing::recovery::Preferences>,
        Query<(&ui::HistoryPreference, &bevy::text::EditableText)>,
    ),
) {
    let paint = &mut *paint;
    for (&model, session) in &mut paint.sessions {
        let active = editor.viewing_model == Some(model)
            && editor.pending.is_none()
            && editor.viewing_image.is_none()
            && editing.documents.get(&model).is_some_and(|doc| doc.mode == Mode::Paint);
        session.finish_shader_import(active);
    }
    if let Some(result) = paint.graph_export.as_mut()
        .and_then(|job| bevy::tasks::block_on(bevy::tasks::poll_once(job)))
    {
        paint.graph_export = None;
        match result {
            Ok(Some(path)) => editor.status = format!("Граф экспортирован · {}", path.display()),
            Ok(None) => {}
            Err(error) => editor.status = format!("Не удалось экспортировать граф · {error}"),
        }
    }
    if let Some(result) = paint.library.poll_export() {
        editor.status = match result {
            Ok(report) => {
                let mut status = format!("Экспортировано · {}", report.path.display());
                if report.omitted > 0 {
                    status.push_str(&format!(" · Пропущено данных UV и ссылок на карты: {}", report.omitted));
                }
                if report.adjusted > 0 {
                    status.push_str(&format!(" · Адаптировано настроек: {}", report.adjusted));
                }
                status
            }
            Err(error) => format!("Не удалось экспортировать материал · {error}"),
        };
    }
    if let Some(result) = paint.library.poll() {
        match result {
            Ok((entry, skipped)) => {
                editor.status = if skipped == 0 {
                    format!("В библиотеку добавлен пресет «{}»", entry.name)
                } else {
                    format!("В библиотеку добавлен пресет «{}» · Пропущено данных UV и ссылок на карты: {skipped}", entry.name)
                };
            }
            Err(error) => editor.status = format!("Библиотека рисования · {error}"),
        }
    }
    if let Some(result) = paint.smart_import.as_mut()
        .and_then(|job| bevy::tasks::block_on(bevy::tasks::poll_once(&mut job.task)))
    {
        let job = paint.smart_import.take().unwrap();
        if let Some(session) = paint.sessions.get_mut(&job.model) {
            session.smart_loading = false;
            session.revision += 1;
            if editor.viewing_model == Some(job.model)
                && session.binding == Some(job.binding)
                && session.set == job.set
                && session.texture_size == job.size
                && crate::editor::model_paint_data::same_stack(&session.stack, &job.stack)
                && session.available(job.action)
                && (!job.action.edits_graph() || job.mask == session.editing_mask)
                && job.maps.as_ref().is_none_or(|maps| {
                    editing.documents.get(&job.model)
                        .and_then(|doc| baked.maps(job.model, doc, job.binding, job.size))
                        .is_some_and(|current| Arc::ptr_eq(current, maps))
                })
                && job.surface.as_ref().is_none_or(|source| {
                    editing.documents.get(&job.model).is_some_and(|doc| source.matches(doc, job.binding))
                })
                && editing.documents.get(&job.model).is_some_and(|doc| doc.mode == Mode::Paint)
            {
                match result {
                    Ok(layers) if job.folder_graph && layers.is_empty() => {}
                    Ok(layers) if job.folder_graph => {
                        let stack = Stack { layers, selected: session.stack.selected };
                        let node = match job.action {
                            PaintAction::GraphBitmapPick(material_graph::ParameterTarget::Node(id, _)) => Some(id),
                            _ => None,
                        };
                        session.commit_folder_view(stack, node);
                    }
                    Ok(layers) if job.action == PaintAction::GraphRebake => {
                        session.stack.layers = layers;
                        session.graph_bake = job.maps.as_ref().map(Arc::downgrade);
                        if let Some(source) = job.surface {
                            session.graph_surfaces.insert(job.binding, source);
                        }
                    }
                    Ok(layers) if layers.is_empty()
                        && matches!(job.action, PaintAction::ImportGraphFile | PaintAction::GraphBitmapPick(_)) => {}
                    Ok(mut layers) if let PaintAction::ApplyShelfMask(_, target) = job.action
                        && layers.len() == 1 =>
                    {
                        session.apply_shelf_mask(layers.remove(0).mask.unwrap(), target);
                    }
                    Ok(mut layers) if (job.action.edits_graph() || job.action == PaintAction::AddGraphMask)
                        && layers.len() == 1 =>
                    {
                        let selected = session.stack.selected;
                        let mut prepared = layers.remove(0);
                        session.checkpoint();
                        let layer = &mut session.stack.layers[selected];
                        if job.mask {
                            let new_mask = layer.mask.is_none();
                            layer.mask = prepared.mask.take();
                            folder_graph::sync_standalone_mask(&mut session.stack, selected);
                            if job.action == PaintAction::AddGraphMask {
                                session.editing_mask = true;
                                if new_mask {
                                    session.mask_value = 0;
                                }
                                session.graph_canvas = true;
                                session.graph_node = None;
                                session.graph_map_menu = None;
                            }
                        } else {
                            let mut fill = prepared.texture_fill.take().unwrap();
                            if let Some(previous) = &layer.texture_fill {
                                material_graph::apply_settings(Arc::make_mut(&mut fill), previous.settings);
                            }
                            layer.texture_fill = Some(fill);
                        }
                        if let PaintAction::GraphEdit(material_graph::Edit::Remove(id)) = job.action
                            && session.graph_node == Some(id)
                        {
                            session.graph_node = None;
                        }
                        if let PaintAction::GraphEdit(material_graph::Edit::Add(..)) = job.action {
                            session.graph_node = session.graph()
                                .and_then(|graph| graph["nodes"].as_array()?.last()?.get("id")?.as_u64());
                        }
                    }
                    Ok(layers) if !layers.is_empty()
                        && session.stack.layers.len() + layers.len() <= MAX_LAYERS
                        && layers.iter().all(|layer| layer.depth + job.insertion.1 <= MAX_LAYER_DEPTH) =>
                    {
                        session.insert_smart_material(layers, job.insertion);
                    }
                    Ok(_) => editor.status = "Материал не помещается в стек слоёв".into(),
                    Err(error) => {
                        if job.action == PaintAction::GraphRebake {
                            session.graph_bake = job.maps.as_ref().map(Arc::downgrade);
                            if let Some(source) = job.surface {
                                session.graph_surfaces.insert(job.binding, source);
                            }
                        }
                        editor.status = format!("Не удалось подготовить материал · {error}");
                    }
                }
            } else {
                editor.status = "Операция с материалом отменена после изменения слоя или набора".into();
            }
        }
    }
    if let Some(result) = paint.texture_import.as_mut()
        .and_then(|job| bevy::tasks::block_on(bevy::tasks::poll_once(&mut job.task)))
    {
        let job = paint.texture_import.take().unwrap();
        match result {
            Ok(Some(bitmap)) => {
                if let Some(session) = paint.sessions.get_mut(&job.model)
                    && session.binding == Some(job.binding)
                    && crate::editor::model_paint_data::same_stack(&session.stack, &job.stack)
                    && session.available(PaintAction::FillTextureImport)
                    && editing.documents.get(&job.model).is_some_and(|doc| doc.mode == Mode::Paint)
                {
                    session.edit_texture_fill(|fill| fill.images[job.channel] = Some(Arc::new(bitmap)));
                } else {
                    editor.status = "Загрузка отменена после изменения слоя или набора".into();
                }
            }
            Ok(None) => {}
            Err(error) => editor.status = format!("Не удалось загрузить изображение заливки · {error}"),
        }
    }
    if let Some(result) = paint.tips.finish(&mut images) {
        match result {
            Ok(Some((index, tip))) => {
                let index = paint.last_painted_model.unwrap_or(index);
                if let Some(session) = paint.sessions.get_mut(&index) {
                    session.edit(PaintAction::BrushTip(Some(tip)));
                }
            }
            Ok(None) => {}
            Err(error) => {
                editor.status = format!("Не удалось загрузить наконечник кисти · {error}")
            }
        }
    }
    let pending_drop = paint.shelf_drop.take();
    let Some(index) = editor.viewing_model else {
        return;
    };
    if editor.pending.is_some() || editor.viewing_image.is_some() {
        return;
    }
    let Some(doc) = editing.documents.get(&index).filter(|doc| doc.mode == Mode::Paint) else {
        return;
    };
    paint.activate(index, recovery.settings().undo_steps);
    let paint = &mut *paint;
    let session = paint.sessions.get_mut(&index).unwrap();
    session.sync_project_shader(doc.preview_shader.as_ref());
    let saved_set = doc.paint_set_index();
    if session.set != saved_set {
        session.set = saved_set;
        session.set_menu = false;
        session.revision += 1;
    }
    let stored_settings = export_preferences.settings();
    if session.export_settings != stored_settings {
        if session.export_settings.template != stored_settings.template {
            session.export_output = 0;
            session.export_component = 0;
            session.export_source_menu = false;
        }
        session.export_settings = stored_settings;
        session.revision += 1;
    }
    if let Some(doc) = editing.documents.get(&index) {
        session.set_appearance.count(doc);
    }
    let deferred_drop = pending_drop.filter(|(model, _, _)| *model == index)
        .map(|(_, (object, slot), material)| Action::ModelView(ModelViewAction::Paint(
            PaintAction::DropMaterialOnSet(material, object, slot),
        )));
    for (action, deferred) in messages.read().copied().map(|action| (action, false))
        .chain(deferred_drop.map(|action| (action, true)))
    {
        let session = paint.sessions.get_mut(&index).unwrap();
        if let Action::ModelView(ModelViewAction::Paint(mut action)) = action {
            if let PaintAction::ShaderToGraph { model, definition, library_revision } = action {
                if model != index || editor.browser_popup || editor.hdri_library_open {
                    continue;
                }
                if paint.smart_import.is_some() {
                    editor.status = "Дождитесь завершения операции с материалом".into();
                    continue;
                }
                match session.insert_shader_node(definition, library_revision) {
                    Ok(edit) => action = edit,
                    Err(error) => {
                        editor.status = error;
                        continue;
                    }
                }
            }
            if let PaintAction::DropMaterialOnSet(material, id, slot) = action {
                let key = (id, slot);
                let Some(doc) = editing.documents.get_mut(&index) else { continue };
                let Some(set) = doc.objects.iter()
                    .flat_map(|object| (0..object.materials.len()).map(move |slot| (object, slot)))
                    .position(|(object, candidate)| object.id == id && !object.hidden && candidate == slot)
                    .filter(|_| session.set_appearance.visibility.shown(id, slot))
                else { continue };
                if deferred && (session.set != set || paint.shelf_drop.is_some()) {
                    continue;
                }
                if !deferred && !session.available(action) {
                    continue;
                }
                if session.binding != Some(key) || session.busy {
                    if !deferred {
                        doc.select_object(id, false);
                        session.edit(PaintAction::Set(set));
                        if session.set == set {
                            doc.paint_set = Some(set);
                        }
                    }
                    paint.shelf_drop = Some((index, key, material));
                    continue;
                }
                session.set_appearance.flash(key, time.elapsed_secs_f64());
                action = PaintAction::DropMaterial(material, ShelfPlacement::Top);
            }
            if (session.cage_brush_active()
                || (session.panel == Panel::Maps && session.bake_view.is_some_and(bake::BakeMap::supports_fix)))
                && matches!(action, PaintAction::Undo | PaintAction::Redo)
            {
                if let Some(doc) = editing.documents.get_mut(&index) {
                    if action == PaintAction::Undo {
                        doc.undo();
                    } else {
                        doc.redo();
                    }
                }
                continue;
            }
            if session.active_folder_view().is_some() && action.edits_graph() {
                if paint.smart_import.is_some() || !session.available(action) {
                    continue;
                }
                let bitmap = if let PaintAction::GraphBitmapPick(target) = action {
                    let view = session.active_folder_view().unwrap();
                    Some((target, view.graph.clone()))
                } else {
                    None
                };
                let (mut stack, mut masks) = if bitmap.is_some() {
                    (session.stack.clone(), Vec::new())
                } else {
                    match session.edit_folder_view(action) {
                        Ok(Some(edit)) => edit,
                        Ok(None) => continue,
                        Err(error) => {
                            editor.status = error;
                            continue;
                        }
                    }
                };
                let Some((device, queue)) = device.as_deref().zip(queue.as_deref()) else {
                    editor.status = "GPU для расчёта маски недоступен".into();
                    continue;
                };
                let Some(binding) = session.binding else { continue };
                let (device, queue) = (device.clone(), queue.clone());
                let size = session.texture_size;
                let maps = editing.documents.get(&index)
                    .and_then(|doc| baked.maps(index, doc, binding, size)).cloned();
                let surface = editing.documents.get(&index)
                    .and_then(|doc| material_graph::surface::Source::capture(doc, binding)).map(Arc::new);
                let mut images = session.active_folder_view().unwrap().images.clone();
                let directory = editor.project.asset_root.clone();
                #[cfg(test)]
                let test_path = bitmap.as_ref().and_then(|_| paint.graph_bitmap_test_path.take());
                session.smart_loading = true;
                session.revision += 1;
                paint.smart_import = Some(SmartImport {
                    model: index, binding, set: session.set, stack: session.stack.clone(), action,
                    mask: false, folder_graph: true, insertion: session.insertion(), size,
                    maps: maps.clone(), surface: surface.clone(),
                    task: bevy::tasks::AsyncComputeTaskPool::get().spawn(async move {
                        if let Some((target, graph)) = bitmap {
                            let picked = bevy::tasks::IoTaskPool::get().spawn(async move {
                                #[cfg(test)]
                                if let Some(path) = test_path {
                                    return Ok(path);
                                }
                                crate::editor::asset_monitor::choose_file_path(&directory)
                                    .map_err(|error| error.to_string())
                            }).await?;
                            let Some(path) = picked else { return Ok(Vec::new()) };
                            masks = folder_graph::replace_bitmap(&mut stack, &graph, target, &path)?;
                            let folder = &stack.layers[stack.selected];
                            Arc::make_mut(&mut images).extend(folder.folder_images.iter()
                                .map(|(path, bitmap)| (path.clone(), bitmap.clone())));
                        }
                        let inputs = material_graph::Inputs { maps: maps.as_deref(), surface: surface.as_deref() };
                        for (index, graph) in masks {
                            let layer = &mut stack.layers[index];
                            if let Some(graph) = graph {
                                material_graph::apply_mask(layer, graph, &images, size, inputs, &device, &queue)?;
                            } else if let Some(mask) = layer.mask.as_mut() {
                                Arc::make_mut(mask).clear_graph();
                            }
                        }
                        Ok(stack.layers)
                    }),
                });
                continue;
            }
            if matches!(action, PaintAction::Category(_) | PaintAction::Reset) {
                paint.shelf.menu = false;
            }
            if session.editing_shader().is_some()
                && (action.edits_graph() || matches!(action, PaintAction::Undo | PaintAction::Redo))
            {
                if !session.available(action) {
                    continue;
                }
                let id = session.preview_shader.unwrap();
                let shader = session.shader_graphs.get_mut(&id).unwrap();
                match action {
                    PaintAction::Undo | PaintAction::Redo => {
                        shader.restore(action == PaintAction::Redo);
                        session.graph_node = None;
                    }
                    _ => {
                        let result = (|| {
                            let mut graph = material_graph::read(&shader.data().0)?;
                            let node = match action {
                                PaintAction::GraphAdjust(node, param, component, direction) => {
                                    if !material_graph::adjust(&mut graph, node, param, component, direction) {
                                        return Ok(());
                                    }
                                    Some(node)
                                }
                                PaintAction::GraphEdit(edit) => {
                                    let label = if let material_graph::Edit::LabelParameter(index, group) = edit {
                                        graph_labels.iter().find_map(|(key, text)| {
                                            (*key == GraphLabel(index, group)).then(|| text.editor().text().to_string())
                                        })
                                    } else {
                                        None
                                    };
                                    edit.apply_for(&mut graph, label.as_deref(), true)?
                                }
                                _ => unreachable!(),
                            };
                            shader.replace(&graph);
                            session.graph_node = node.or(session.graph_node).filter(|id| graph.node(*id).is_some());
                            Ok::<(), String>(())
                        })();
                        if let Err(error) = result {
                            editor.status = format!("Не удалось изменить граф · {error}");
                        }
                    }
                }
                if id == shader_preview::presets().len()
                    && let Some(doc) = editing.documents.get_mut(&index)
                    && let Some(saved_graph) = &mut doc.preview_shader
                    && !Arc::ptr_eq(saved_graph, shader.data().0)
                {
                    *saved_graph = shader.data().0.clone();
                    doc.recovery_edits += 1;
                    doc.ui_revision += 1;
                }
                session.revision += 1;
                continue;
            }
            match action {
                PaintAction::ToggleRecovery | PaintAction::ApplyHistoryPreference(_) => {
                    if editor.browser_popup || editor.hdri_library_open {
                        continue;
                    }
                    let mut settings = recovery.settings();
                    match action {
                        PaintAction::ToggleRecovery => settings.enabled = !settings.enabled,
                        PaintAction::ApplyHistoryPreference(entity) => {
                            let Ok((&preference, input)) = recovery_fields.get(entity) else {
                                continue;
                            };
                            let Some(limit) = preference.parse(input) else {
                                let (min, max) = preference.range();
                                editor.status = format!("Введите целое число от {min} до {max}");
                                continue;
                            };
                            match preference {
                                ui::HistoryPreference::Recovery => settings.edits = limit,
                                ui::HistoryPreference::Undo => settings.undo_steps = usize::from(limit),
                            }
                        }
                        _ => unreachable!(),
                    }
                    let history_changed = settings.undo_steps != recovery.settings().undo_steps;
                    editor.status = match recovery.save(settings) {
                        Ok(()) => {
                            if history_changed {
                                for session in paint.sessions.values_mut() {
                                    session.set_history_limit(settings.undo_steps);
                                }
                            }
                            "Настройки истории правок сохранены".into()
                        }
                        Err(error) => format!("Не удалось сохранить настройки истории · {error}"),
                    };
                    continue;
                }
                PaintAction::SaveBrushDefaults | PaintAction::ResetBrushDefaults => {
                    if editor.browser_popup || editor.hdri_library_open {
                        continue;
                    }
                    let reset = action == PaintAction::ResetBrushDefaults;
                    let settings = if reset {
                        brush_defaults::Settings::default()
                    } else {
                        brush_defaults::Settings::capture(session)
                    };
                    match paint.brush_defaults.save(settings) {
                        Ok(()) if reset => {
                            settings.apply(session);
                            session.revision += 1;
                            editor.status = "Стандартные параметры рисования восстановлены".into();
                        }
                        Ok(()) => editor.status = "Стартовые параметры рисования сохранены".into(),
                        Err(error) => {
                            editor.status = format!("Не удалось сохранить параметры рисования · {error}");
                        }
                    }
                    continue;
                }
                PaintAction::ImportShelfMaterial => {
                    let directory = editor.project.asset_root.clone();
                    if let Err(error) = paint.library.start_import(move || {
                        crate::editor::asset_monitor::choose_file_path(&directory)
                            .map_err(|error| error.to_string())
                    }) {
                        editor.status = format!("Не удалось импортировать материал · {error}");
                    }
                    continue;
                }
                PaintAction::SaveShelfMaterial | PaintAction::SaveShelfMask | PaintAction::ExportShelfMaterial | PaintAction::ExportShelfPackage => {
                    if session.available(action) {
                        let selected = &session.stack.layers[session.stack.selected];
                        let capture = if action == PaintAction::SaveShelfMask {
                            presets::Capture::Mask { name: selected.name.clone(), mask: selected.mask.as_ref().unwrap().clone() }
                        } else {
                            presets::Capture::Material {
                                name: selected.name.clone(),
                                layers: session.stack.layers[session.stack.block(session.stack.selected)].to_vec(),
                            }
                        };
                        if matches!(action, PaintAction::ExportShelfMaterial | PaintAction::ExportShelfPackage) {
                            let package = action == PaintAction::ExportShelfPackage;
                            if let Err(error) = paint.library.start_export(capture, package, move || {
                                let title = if package { "Папка для ZIP-пакета материала" } else { "Папка для экспорта материала .mpsmart" };
                                crate::project_manager::choose_folder(title)
                                    .map_err(|error| error.to_string())
                            }) {
                                editor.status = format!("Не удалось экспортировать материал · {error}");
                            }
                            continue;
                        }
                        match paint.library.start_save(capture) {
                            Ok(()) => {
                                session.category = if action == PaintAction::SaveShelfMask {
                                    Category::Generators
                                } else {
                                    Category::Smart
                                };
                                session.revision += 1;
                            }
                            Err(error) => editor.status = format!("Не удалось сохранить пресет · {error}"),
                        }
                    }
                    continue;
                }
                PaintAction::ApplyShelfMask(i, target) => {
                    if session.available(action)
                        && let Some(entry) = paint.library.entries.get(i)
                        && let presets::Kind::Mask(preset) = &entry.kind
                    {
                        if !preset.has_graph() {
                            session.apply_shelf_mask(preset.build(session.texture_size), target);
                        } else if paint.smart_import.is_none() && let Some(binding) = session.binding {
                            let size = session.texture_size;
                            let path = entry.path.clone();
                            session.smart_loading = true;
                            session.revision += 1;
                            paint.smart_import = Some(SmartImport {
                                model: index, binding, set: session.set, stack: session.stack.clone(),
                                action, mask: true, folder_graph: false, insertion: session.insertion(), size,
                                maps: None, surface: None,
                                task: bevy::tasks::AsyncComputeTaskPool::get().spawn(async move {
                                    let mut layer = Layer::new(String::new(), LayerKind::Fill);
                                    layer.mask = Some(presets::load_mask(&path, size)?);
                                    Ok(vec![layer])
                                }),
                            });
                        }
                    }
                    continue;
                }
                PaintAction::ApplyShelfMaterial(i) | PaintAction::DropMaterial(ShelfMaterial::User(i), _) => {
                    if paint.smart_import.is_none()
                        && session.available(action)
                        && let Some(binding) = session.binding
                        && let Some(entry) = paint.library.entries.get(i)
                        && let presets::Kind::Material { layers, depth, .. } = entry.kind
                    {
                        let size = session.texture_size;
                        let insertion = match action {
                            PaintAction::DropMaterial(_, target) => session.shelf_insertion(target).unwrap(),
                            _ => session.insertion(),
                        };
                        if session.stack.layers.len() + layers <= MAX_LAYERS && depth + insertion.1 <= MAX_LAYER_DEPTH {
                            let path = entry.path.clone();
                            let foreign = presets::is_mpsmart(&path);
                            let renderer = device.as_deref().zip(queue.as_deref())
                                .map(|(device, queue)| (device.clone(), queue.clone()));
                            let (maps, surface) = if foreign && let Some(doc) = editing.documents.get(&index) {
                                (
                                    baked.maps(index, doc, binding, size).cloned(),
                                    material_graph::surface::Source::capture(doc, binding).map(Arc::new),
                                )
                            } else {
                                (None, None)
                            };
                            session.smart_loading = true;
                            session.revision += 1;
                            paint.smart_import = Some(SmartImport {
                                model: index, binding, set: session.set, stack: session.stack.clone(),
                                action, mask: false, folder_graph: false, insertion, size,
                                maps: maps.clone(), surface: surface.clone(),
                                task: bevy::tasks::AsyncComputeTaskPool::get().spawn(async move {
                                    let mut layers = presets::load_material(&path, size)?;
                                    if foreign {
                                        let inputs = material_graph::Inputs {
                                            maps: maps.as_deref(), surface: surface.as_deref(),
                                        };
                                        material_graph::prepare_imported(&mut layers, size, inputs,
                                            renderer.as_ref().map(|(device, queue)| (device, queue)))?;
                                    }
                                    Ok(layers)
                                }),
                            });
                        } else {
                            editor.status = "Материал не помещается в стек слоёв".into();
                        }
                    }
                    continue;
                }
                PaintAction::Category(Category::Smart | Category::Generators) => paint.library.ensure(),
                PaintAction::ShelfClearSearch => {
                    paint.shelf.clear = true;
                    continue;
                }
                PaintAction::ShelfTypeMenu => {
                    paint.shelf.menu = !paint.shelf.menu;
                    continue;
                }
                PaintAction::ShelfType(kind) => {
                    if paint.shelf.material != kind {
                        paint.shelf.material = kind;
                        paint.shelf.revision += 1;
                    }
                    paint.shelf.menu = false;
                    continue;
                }
                PaintAction::ApplyGenerator(kind, target) => {
                    let custom_map = editing.documents.get(&index)
                        .and_then(|doc| doc.custom_bakes.first())
                        .map(|recipe| recipe.id);
                    session.apply_generator(kind, target, custom_map);
                    continue;
                }
                PaintAction::ImportShaderGraph => {
                    if session.available(action) {
                        #[cfg(test)]
                        let test_path = paint.shader_graph_test_path.take();
                        let directory = editor.project.asset_root.clone();
                        session.shader_import_error = None;
                        session.shader_import = Some(bevy::tasks::IoTaskPool::get().spawn(async move {
                            let choose = || crate::editor::asset_monitor::choose_file_path(&directory)
                                .map_err(|error| error.to_string());
                            #[cfg(test)]
                            let path = test_path.map_or_else(choose, Ok)?;
                            #[cfg(not(test))]
                            let path = choose()?;
                            path.as_deref().map(material_graph::file::load_shader).transpose()
                        }));
                        session.revision += 1;
                    }
                    continue;
                }
                PaintAction::ExportGraphFile | PaintAction::ExportGraphPackage => {
                    if session.available(action)
                        && let Some((data, images)) = session.graph_data()
                    {
                        if paint.graph_export.is_some() {
                            editor.status = "Дождитесь завершения экспорта графа".into();
                        } else {
                            let (data, images) = (data.clone(), images.clone());
                            let package = action == PaintAction::ExportGraphPackage;
                            let title = if package {
                                "Папка для ZIP-пакета графа"
                            } else {
                                "Папка для экспорта графа .mpgraph"
                            };
                            paint.graph_export = Some(bevy::tasks::IoTaskPool::get().spawn(async move {
                                crate::project_manager::choose_folder(title)
                                    .map_err(|error| error.to_string())?
                                    .map(|directory| {
                                        if package {
                                            material_graph::file::export_package(&directory, &data, &images)
                                        } else {
                                            material_graph::file::export(&directory, &data, &images)
                                        }
                                    })
                                    .transpose()
                            }));
                        }
                    }
                    continue;
                }
                PaintAction::AddGraphMaterial(_) | PaintAction::AddGraphMask | PaintAction::ImportGraphFile
                | PaintAction::GraphAdjust(..) | PaintAction::GraphEdit(_) | PaintAction::GraphBitmapPick(_) => {
                    if paint.smart_import.is_none()
                        && session.available(action)
                        && let Some(binding) = session.binding
                    {
                        let mask = action == PaintAction::AddGraphMask || (action.edits_graph() && session.editing_mask);
                        let adjusted_graph = if action.edits_graph() {
                            if matches!(action, PaintAction::GraphEdit(material_graph::Edit::SetMeshMap(..)))
                                && session.graph_map_menu.take().is_some()
                            {
                                session.revision += 1;
                            }
                            let (previous_payload, previous_images) = session.graph_data().unwrap();
                            let previous_images = previous_images.clone();
                            let mut graph = match material_graph::read(previous_payload) {
                                Ok(graph) => graph,
                                Err(error) => {
                                    editor.status = format!("Не удалось изменить граф · {error}");
                                    continue;
                                }
                            };
                            match action {
                                PaintAction::GraphAdjust(node, param, component, direction) => {
                                    if !material_graph::adjust(&mut graph, node, param, component, direction) {
                                        continue;
                                    }
                                }
                                PaintAction::GraphEdit(edit) => {
                                    let label = if let material_graph::Edit::LabelParameter(index, group) = edit {
                                        graph_labels.iter().find_map(|(key, text)| {
                                            (*key == GraphLabel(index, group)).then(|| text.editor().text().to_string())
                                        })
                                    } else {
                                        None
                                    };
                                    match edit.apply(&mut graph, label.as_deref()) {
                                        Ok(node) => {
                                            let rebuilds = edit.rebuilds();
                                            let payload = if rebuilds {
                                                material_graph::encode(&graph)
                                            } else {
                                                material_graph::encode_without_rebuild(&graph, previous_payload)
                                            };
                                            if previous_payload.as_ref() == &payload {
                                                continue;
                                            }
                                            if !rebuilds {
                                                session.checkpoint();
                                                let layer = &mut session.stack.layers[session.stack.selected];
                                                if mask {
                                                    let mask = Arc::make_mut(layer.mask.as_mut().unwrap());
                                                    Arc::make_mut(mask.graph.as_mut().unwrap()).payload = Arc::new(payload);
                                                    if !matches!(edit, material_graph::Edit::Move(..)) {
                                                        let selected = session.stack.selected;
                                                        folder_graph::sync_standalone_mask(&mut session.stack, selected);
                                                    }
                                                } else {
                                                    Arc::make_mut(layer.texture_fill.as_mut().unwrap()).graph =
                                                        Some(Arc::new(payload));
                                                }
                                                if let Some(node) = node {
                                                    session.graph_node = Some(node);
                                                }
                                                session.revision += 1;
                                                continue;
                                            }
                                        }
                                        Err(error) => {
                                            editor.status = error;
                                            continue;
                                        }
                                    }
                                }
                                PaintAction::GraphBitmapPick(_) => {}
                                _ => unreachable!(),
                            }
                            Some((graph, previous_images))
                        } else if action == PaintAction::AddGraphMask {
                            Some((material_graph::mask_starter(), Arc::default()))
                        } else {
                            None
                        };
                        let Some((device, queue)) = device.as_deref().zip(queue.as_deref()) else {
                            editor.status = "GPU для расчёта графа недоступен".into();
                            continue;
                        };
                        let (device, queue) = (device.clone(), queue.clone());
                        let size = session.texture_size;
                        let maps = editing.documents.get(&index)
                            .and_then(|doc| baked.maps(index, doc, binding, size)).cloned();
                        let surface = editing.documents.get(&index)
                            .and_then(|doc| material_graph::surface::Source::capture(doc, binding)).map(Arc::new);
                        let directory = editor.project.asset_root.clone();
                        let mask_layer = mask.then(|| session.stack.layers[session.stack.selected].clone());
                        #[cfg(test)]
                        let test_path = if matches!(action, PaintAction::GraphBitmapPick(_)) {
                            paint.graph_bitmap_test_path.take()
                        } else {
                            None
                        };
                        session.smart_loading = true;
                        session.revision += 1;
                        paint.smart_import = Some(SmartImport {
                            model: index,
                            binding,
                            set: session.set,
                            stack: session.stack.clone(),
                            action,
                            mask,
                            folder_graph: false,
                            insertion: session.insertion(),
                            size,
                            maps: maps.clone(),
                            surface: surface.clone(),
                            task: bevy::tasks::AsyncComputeTaskPool::get().spawn(async move {
                                let inputs = material_graph::Inputs { maps: maps.as_deref(), surface: surface.as_deref() };
                                let prepare_layer = |graph, images: &texture_fill::GraphImages| {
                                    if let Some(mut layer) = mask_layer {
                                        material_graph::apply_mask(&mut layer, graph, images, size, inputs, &device, &queue)?;
                                        Ok(layer)
                                    } else {
                                        material_graph::prepare_graph(graph, images, size, inputs, &device, &queue)
                                    }
                                };
                                match (action, adjusted_graph) {
                                    (PaintAction::AddGraphMaterial(recipe), _) =>
                                        material_graph::prepare(recipe, size, inputs, &device, &queue),
                                    (PaintAction::GraphBitmapPick(target), Some((mut graph, mut images))) => {
                                        let picked = bevy::tasks::IoTaskPool::get().spawn(async move {
                                            #[cfg(test)]
                                            if let Some(path) = test_path {
                                                return Ok(path);
                                            }
                                            crate::editor::asset_monitor::choose_file_path(&directory)
                                                .map_err(|error| error.to_string())
                                        }).await?;
                                        let Some(path) = picked else { return Ok(Vec::new()) };
                                        material_graph::set_bitmap(&mut graph, &mut images, target, &path)?;
                                        prepare_layer(graph, &images)
                                    }
                                    (_, Some((graph, images))) =>
                                        prepare_layer(graph, &images),
                                    (PaintAction::ImportGraphFile, _) => {
                                        let Some(path) = crate::editor::asset_monitor::choose_file_path(&directory)
                                            .map_err(|error| error.to_string())? else { return Ok(Vec::new()) };
                                        let (graph, images) = material_graph::file::load(&path)?;
                                        material_graph::prepare_graph(graph, &images, size, inputs, &device, &queue)
                                    }
                                    _ => Err("Граф материала не найден".into()),
                                }.map(|layer| vec![layer])
                            }),
                        });
                    }
                    continue;
                }
                PaintAction::AddSmartMaterial(recipe)
                | PaintAction::DropMaterial(ShelfMaterial::Smart(recipe), _) => {
                    if paint.smart_import.is_none()
                        && session.available(action)
                        && let Some(binding) = session.binding
                    {
                        let size = session.texture_size;
                        let insertion = match action {
                            PaintAction::DropMaterial(_, target) => session.shelf_insertion(target).unwrap(),
                            _ => session.insertion(),
                        };
                        session.smart_loading = true;
                        session.revision += 1;
                        paint.smart_import = Some(SmartImport {
                            model: index,
                            binding,
                            set: session.set,
                            stack: session.stack.clone(),
                            action,
                            mask: false,
                            folder_graph: false,
                            insertion,
                            size,
                            maps: None,
                            surface: None,
                            task: bevy::tasks::AsyncComputeTaskPool::get().spawn(async move {
                                SMART_MATERIALS[recipe].prepare(size)
                            }),
                        });
                    }
                    continue;
                }
                PaintAction::FillTextureImport => {
                    if paint.texture_import.is_none()
                        && session.available(action)
                        && let Some(binding) = session.binding
                    {
                        let directory = editor.project.asset_root.clone();
                        paint.texture_import = Some(TextureImport {
                            model: index, binding, stack: session.stack.clone(), channel: session.channel,
                            task: bevy::tasks::IoTaskPool::get().spawn(async move {
                                crate::editor::asset_monitor::choose_file_path(&directory)
                                    .map_err(|error| error.to_string())?
                                    .map(|path| texture_fill::Bitmap::open(&path)).transpose()
                            }),
                        });
                    }
                    continue;
                }
                PaintAction::ExportMaps(_) | PaintAction::ExportMesh | PaintAction::ImportExportTemplate | PaintAction::CopyExportTemplate
                | PaintAction::SaveExportTemplate | PaintAction::EditExportTemplate { .. } | PaintAction::BakeStart | PaintAction::BakeCancel | PaintAction::BakeClearDiskCache | PaintAction::BakePreview(_)
                | PaintAction::BakePickHigh | PaintAction::BakeClearHigh | PaintAction::BakePickCage | PaintAction::BakeClearCage | PaintAction::BakeAutoFitCage
                | PaintAction::BakeMatchNames | PaintAction::BakeFillMisses | PaintAction::BakePreferFrontFaces
                | PaintAction::BakeAverageNormals | PaintAction::BakeInspectCage | PaintAction::BakeRange { .. } => continue,
                PaintAction::BakeQuality(_) | PaintAction::BakeIdSource(_) | PaintAction::BakeMapOptions
                | PaintAction::BakeToggleMap(_) | PaintAction::BakeSelectAllMaps(_)
                | PaintAction::BakeAdjustLevels(..) | PaintAction::BakeResetLevels(_)
                | PaintAction::BakeFixTool(_) | PaintAction::BakeClearFix(_) | PaintAction::BakeFixValue(_) => continue,
                PaintAction::BakeCustomAdd(_) | PaintAction::BakeCustomRemove(_) | PaintAction::BakeCustomAdjust(..)
                | PaintAction::BakeCustomRename(_) | PaintAction::BakeCustomDuplicate(_) | PaintAction::BakeCustomMove(..)
                | PaintAction::BakeCustomGizmo(_) | PaintAction::BakeLivePreview | PaintAction::BakeShowOnModel | PaintAction::BakeLiveMasks => continue,
                PaintAction::BakePaintCage | PaintAction::BakeResetCagePaint => continue,
                PaintAction::Category(Category::Brushes) => paint.tips.ensure(&mut images),
                PaintAction::Category(Category::Textures) => paint.tips.ensure_grains(&mut images),
                PaintAction::Grain(Some(i)) if i >= paint.tips.grains.len() => continue,
                PaintAction::ImportBrushTip => {
                    paint.tips.start(index, editor.project.asset_root.clone());
                    session.revision += 1;
                    continue;
                }
                PaintAction::BrushTip(Some(i)) if i >= paint.tips.entries.len() => continue,
                PaintAction::SetHide(id, slot) | PaintAction::SetPin(id, slot)
                    if !editing.documents.get(&index).is_some_and(|doc| {
                        doc.objects.iter().any(|object| object.id == id && slot < object.materials.len())
                    }) => continue,
                _ => {}
            }
            if !session.available(action) {
                if matches!(action, PaintAction::Undo | PaintAction::Redo) {
                    session.queued.push_back(action);
                }
                continue;
            }
            if let PaintAction::Set(set) = action {
                if let Some(doc) = editing.documents.get_mut(&index) {
                    let object = doc.paint_target(set).map(|(object, slot)| (object.id, slot));
                    if let Some((id, slot)) = object {
                        doc.select_object(id, false);
                        doc.paint_set = Some(set);
                        session.set_appearance.flash((id, slot), time.elapsed_secs_f64());
                    } else {
                        continue;
                    }
                }
            }
            if action == PaintAction::GraphRebake {
                continue;
            }
            if let PaintAction::RenameLayer(entity) = action {
                if let Ok((field, mut text)) = layer_names.get_mut(entity)
                    && field.matches(index, session)
                {
                    match session.rename_layer(&text.value().to_string()) {
                        Ok(()) => {
                            text.editor_mut().set_text(&session.stack.layers[field.layer].name);
                        }
                        Err(error) => editor.status = error.into(),
                    }
                }
                continue;
            }
            session.edit(action);
            if matches!(action, PaintAction::SetHide(..))
                && let Some(doc) = editing.documents.get(&index)
            {
                session.set_appearance.count(doc);
            }
            if session.resize_request.is_some()
                && let Some(doc) = editing.documents.get_mut(&index)
            {
                doc.paint_busy = true;
                doc.paint_pending = true;
            }
        }
    }

    let session = paint.sessions.get_mut(&index).unwrap();
    if paint.smart_import.is_none()
        && session.available(PaintAction::GraphRebake)
        && let Some(binding) = session.binding
        && let Some(doc) = editing.documents.get(&index)
    {
        let maps = baked.maps(index, doc, binding, session.texture_size);
        let stamp = maps.map(Arc::downgrade);
        let new_maps = stamp.as_ref().is_some_and(|stamp| {
            session.graph_bake.as_ref().is_none_or(|old| !old.ptr_eq(stamp))
        });
        let previous_surface = session.graph_surfaces.get(&binding);
        let new_surface = previous_surface.is_none_or(|old| !old.matches(doc, binding));
        if !new_maps && !new_surface {
            return;
        }
        let surface = if new_surface {
            material_graph::surface::Source::capture(doc, binding).map(Arc::new)
        } else {
            previous_surface.cloned()
        };
        let Some(surface) = surface else {
            return;
        };
        let graphs = material_graph::affected_layers(&session.stack, new_maps, new_surface);
        if graphs.is_empty() {
            session.graph_bake = stamp;
            session.graph_surfaces.insert(binding, surface);
        } else if let Some((device, queue)) = device.as_deref().zip(queue.as_deref()) {
            let (device, queue) = (device.clone(), queue.clone());
            let size = session.texture_size;
            let maps = maps.cloned();
            let mut layers = session.stack.layers.clone();
            session.smart_loading = true;
            session.revision += 1;
            paint.smart_import = Some(SmartImport {
                model: index, binding, set: session.set, stack: session.stack.clone(),
                action: PaintAction::GraphRebake, mask: false, folder_graph: false, insertion: session.insertion(), size,
                maps: maps.clone(), surface: Some(surface.clone()),
                task: bevy::tasks::AsyncComputeTaskPool::get().spawn(async move {
                    let inputs = material_graph::Inputs { maps: maps.as_deref(), surface: Some(&surface) };
                    for (index, is_mask, graph) in graphs {
                        material_graph::refresh_layer(
                            &mut layers[index], is_mask, graph, size, inputs, &device, &queue,
                        )?;
                    }
                    Ok(layers)
                }),
            });
        }
    }
}

#[cfg(test)]
#[path = "model_paint_tests.rs"]
mod tests;
