//! Material Paint workspace state. Rendering and mesh editing stay isolated.
use super::*;
use crate::editor::model_document::Mode;
use crate::editor::model_document::paint_mesh_replacement::{self, Command as MeshCommand};
use crate::editor::model_paint_data::custom;
pub(in crate::editor) use crate::editor::model_paint_data::LayerKind;
use crate::editor::model_paint_data::mask::{
    Filter, FilterKind, Generator, GeneratorInput, GeneratorKind, MAX_FILTERS, MAX_MASK_LAYERS, Mask, MaskLayer,
};
use crate::editor::model_paint_data::{Blend, Layer, MAX_LAYER_DEPTH, MAX_LAYERS, Stack, Surface, texture_fill};
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

#[derive(Component)]
pub(in crate::editor) struct GraphCanvasStage;
#[path = "model_paint_brush_defaults.rs"]
mod brush_defaults;
#[path = "model_paint_mesh_tools.rs"]
pub(in crate::editor) mod mesh_tools;
#[path = "model_paint_save_copy.rs"]
pub(super) mod save_copy;
#[path = "model_paint_history.rs"]
mod history;
#[path = "model_paint_diagnostics.rs"]
mod diagnostics;
#[cfg(feature = "mcp")]
#[path = "model_paint_mcp.rs"]
pub(in crate::editor) mod mcp_state;
#[cfg(feature = "mcp")]
#[path = "model_paint_surface_selection.rs"]
mod surface_selection;
#[cfg(feature = "mcp")]
#[path = "model_paint_mcp_preview.rs"]
pub(in crate::editor::ui::model_viewer) mod mcp_preview;
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
pub(in crate::editor) use post::DocumentPost;

#[path = "model_paint_materials.rs"]
mod materials;
use materials::{MATERIALS, MaterialType};
#[path = "model_paint_presets.rs"]
pub(super) mod presets;
#[path = "model_paint_graph_shelf.rs"]
mod graph_shelf;
#[path = "model_paint_online.rs"]
mod online;
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
#[path = "model_paint_uv_seams.rs"]
pub(super) mod uv_seams;
#[path = "model_paint_sets.rs"]
pub(super) mod sets;
pub(super) use ui::{ShaderMaker, decorate_shader_code, refresh, refresh_graph, refresh_shader_maker, setup};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "mcp", derive(serde::Serialize), serde(rename_all = "snake_case"))]
pub(in crate::editor) enum PaintTool {
    Brush,
    Eraser,
    Fill,
    Picker,
    Path,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "mcp", derive(serde::Serialize, serde::Deserialize, rmcp::schemars::JsonSchema), serde(rename_all = "snake_case"), schemars(crate = "rmcp::schemars"))]
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
#[cfg_attr(feature = "mcp", derive(serde::Serialize, serde::Deserialize, rmcp::schemars::JsonSchema), serde(rename_all = "snake_case"), schemars(crate = "rmcp::schemars"))]
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
    Online,
    Downloaded,
    Smart,
    Brushes,
    Textures,
    Generators,
    Graphs,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[cfg_attr(feature = "mcp", derive(serde::Serialize, serde::Deserialize, rmcp::schemars::JsonSchema), serde(rename_all = "snake_case"), schemars(crate = "rmcp::schemars"))]
pub(in crate::editor) enum Symmetry {
    #[default]
    Off,
    X,
    Y,
    Z,
}
#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[cfg_attr(feature = "mcp", derive(serde::Serialize, serde::Deserialize, rmcp::schemars::JsonSchema), serde(rename_all = "snake_case"), schemars(crate = "rmcp::schemars"))]
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
    TextureHeightOffset,
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
    BakeOption(bake::BakeControl),
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
            Self::Opacity => "Непрозрачность мазка, %",
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
            Self::TextureHeightOffset => "Смещение карты высоты",
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
            Self::BakeOption(control) => control.label(),
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
            Self::BrushScatter => (0., 200.),
            Self::BrushSizeJitter | Self::BrushAngleJitter => (0., 100.),
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
            Self::TextureHeightStrength => (-10., 10.),
            Self::TextureHeightOffset => (-100., 100.),
            Self::FillVariationScale => (0.5, 60.),
            Self::ExportPaddingWidth => (1., 256.),
            Self::ExportHeightStrength => (0., 8.),
            Self::CageRadius => (5., 300.),
            Self::CageStrength => (0.1, 4.),
            Self::BakeOption(control) => control.bounds(),
            Self::FilterValue(_, kind, value) => {
                let (_, _, min, max) = kind.controls()[value];
                (min as f64, max as f64)
            }
            _ => (0., 100.),
        }
    }

    fn fill_parameter(self) -> bool {
        matches!(self, Self::TextureTiling | Self::TextureRotation | Self::TextureOffsetU
            | Self::TextureOffsetV | Self::TextureSharpness | Self::TextureNormalStrength | Self::TextureHeightStrength | Self::TextureHeightOffset
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
    Downloaded(usize),
    Texture(usize),
    Graph(usize),
    CommunityGraph(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::editor) enum ShelfPlacement {
    Top,
    Above(usize),
    Below(usize),
    Inside(usize),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::editor) enum PreviewParameter {
    FieldOfView,
    Height,
    Emission,
    Occlusion,
    UvWireOpacity,
}

impl PreviewParameter {
    fn label(self) -> &'static str {
        match self {
            Self::FieldOfView => "Угол обзора по вертикали, °",
            Self::Height => "Сила рельефа",
            Self::Emission => "Сила свечения",
            Self::Occlusion => "Сила AO, %",
            Self::UvWireOpacity => "Непрозрачность UV-сетки, %",
        }
    }

    fn min(self) -> f32 {
        match self {
            Self::FieldOfView => 10.,
            Self::UvWireOpacity => 5.,
            _ => 0.,
        }
    }

    fn max(self) -> f32 {
        match self {
            Self::FieldOfView => 90.,
            Self::Height => 8.,
            Self::Emission => 20.,
            Self::Occlusion | Self::UvWireOpacity => 100.,
        }
    }

    fn value(self, doc: &crate::editor::model_document::Document) -> f32 {
        let preview = doc.paint_preview;
        match self {
            Self::FieldOfView => doc.model_camera.map_or(FOV, |camera| camera.fov).to_degrees(),
            Self::Height => preview.height_strength.unwrap_or(1.),
            Self::Emission => preview.emission_strength,
            Self::Occlusion => preview.ao_strength * 100.,
            Self::UvWireOpacity => doc.paint_uv.wire_opacity * 100.,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::editor) enum PreviewEdit {
    Value { model: usize, parameter: PreviewParameter, value: f32 },
    Reset { model: usize },
    AutomaticHeight,
    BakedNormal,
    UvWire,
    UvSeams2d,
    UvSeams3d,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::editor) enum PaintAction {
    MeshReplacement(MeshCommand),
    SaveCopy,
    Tool(PaintTool),
    Panel(Panel),
    View(View),
    Preview3d(Preview3d),
    PreviewMaterial(PreviewEdit),
    Category(Category),
    OnlineProvider(online::Provider),
    OnlineRefresh,
    OnlinePage(usize),
    OnlineResolution(u32),
    OnlineMaterial(online::Provider, usize),
    DownloadOnlineMaterial(online::Provider, usize),
    ApplyDownloaded(usize),
    DownloadedPage(usize),
    DownloadedMenu(Option<usize>),
    ShowDownloadedFiles(usize),
    OpenDownloadedSource(usize),
    DeleteDownload(usize),
    ImportDownloaded,
    InstallCommunity(usize),
    CommunityGraph(usize),
    ShareCommunityGraph,
    SaveGraphShelf,
    ApplySavedGraph(usize),
    OpenSavedGraph(usize),
    SavedGraphMenu(Option<usize>),
    ShareSavedGraph(usize),
    ShareCommunityMaterial,
    SavedMaterialMenu(Option<usize>),
    ShareSavedMaterial(usize),
    ShowSavedMaterial(usize),
    DeleteSavedMaterial(usize),
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
    FillTextureImport(usize),
    FillTextureRemove(usize),
    FillTexturePicker(Option<usize>),
    TextureToChannel(usize, usize),
    AddShelfTexture(usize),
    ShelfTextureMenu(Option<usize>),
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
    BakeRayOptions(bake::RayOptions),
    BakeAutoEdgeReach,
    BakeResetSurface,
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
    AddSmartGraph(usize),
    SmartRecipeMenu(Option<usize>),
    AddGraphMaterial(usize),
    AddGraphMask,
    ImportGraphFile,
    ImportShaderGraph,
    ExportGraphFile,
    ExportGraphPackage,
    ExportGraphOutputs,
    GraphSelectNode(u64),
    GraphClearSelection,
    GraphAdjust(u64, usize, usize, i8),
    GraphBitmapPick(material_graph::ParameterTarget),
    GraphMapMenu(material_graph::ParameterTarget),
    GraphRebake,
    GraphCanvas,
    GraphPage(GraphPage),
    OpenProjectGraph { model: usize, target: ProjectGraphTarget },
    GraphResolutionMenu,
    GraphEdit(material_graph::Edit),
    ShaderToGraph { model: usize, definition: usize, library_revision: usize },
    ShaderUpdateGraph { model: usize, node: u64, definition: usize, library_revision: usize },
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
            Self::PreviewMaterial(PreviewEdit::AutomaticHeight) => "Переключить автоматическую силу рельефа.",
            Self::PreviewMaterial(PreviewEdit::BakedNormal) => "Объединять запечённые нормали с нормалями из слоёв.",
            Self::PreviewMaterial(PreviewEdit::UvWire) => "Показать контуры граней поверх карты в UV-виде.",
            Self::PreviewMaterial(PreviewEdit::UvSeams2d) => "Подсветить границы островов и отмеченные швы поверх UV-карты.",
            Self::PreviewMaterial(PreviewEdit::UvSeams3d) => "Подсветить UV-швы на видимой стороне 3D-модели.",
            Self::PreviewMaterial(PreviewEdit::Value { .. }) => "Изменить предпросмотр материала.",
            Self::PreviewMaterial(PreviewEdit::Reset { .. }) => "Вернуть стандартный угол камеры, сетку и настройки обработки изображения.",
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
            Self::MeshReplacement(_) => "Заменить геометрию и перенести слои в соответствующие наборы текстур.",
            Self::SaveCopy => "Ctrl+Shift+S · Сохранить текущую модель и слои рисования в отдельный GLB.",
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
            Self::FillTextureImport(_) => "Загрузить изображение с диска в этот канал заливки.",
            Self::FillTextureRemove(_) => "Убрать изображение из этого канала.",
            Self::FillTexturePicker(_) => "Выбрать текстуру библиотеки для этого канала.",
            Self::TextureToChannel(..) => "Назначить текстуру выбранному каналу заливки.",
            Self::AddShelfTexture(_) => "Добавить отдельную заливку высоты.",
            Self::ShelfTextureMenu(_) => "Открыть действия с текстурой.",
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
            Self::BakeRayOptions(_) => "Вернуть число лучей из пресета качества и исходные дальности и конус.",
            Self::BakeAutoEdgeReach => "Подбирать дальность поиска рёбер по наименьшему размеру модели.",
            Self::BakeResetSurface => "Вернуть сетку проекции из пресета и исходные параметры кривизны и рёбер.",
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
            Self::OnlineProvider(_) => "Выбрать источник сетевых материалов.",
            Self::OnlineRefresh => "Заново загрузить каталог материалов.",
            Self::OnlinePage(_) => "Открыть страницу результатов поиска.",
            Self::OnlineResolution(_) => "Выбрать разрешение скачиваемых карт.",
            Self::OnlineMaterial(..) => "Добавить материал как слой заливки.",
            Self::DownloadOnlineMaterial(..) => "Сохранить карты на полке «Скачанные».",
            Self::ApplyDownloaded(_) => "Добавить материал с диска как слой заливки.",
            Self::DownloadedPage(_) => "Открыть страницу скачанных материалов.",
            Self::DownloadedMenu(_) => "Действия со скачанными картами.",
            Self::ShowDownloadedFiles(_) => "Открыть папку с картами материала.",
            Self::OpenDownloadedSource(_) => "Открыть страницу материала в браузере.",
            Self::DeleteDownload(_) => "Удалить скачанные карты из локальной библиотеки.",
            Self::ImportDownloaded => "Добавить папку скачанного материала с meta.json.",
            Self::InstallCommunity(_) => "Скачать и добавить пакет в библиотеку рисования.",
            Self::CommunityGraph(_) => "Добавить загруженный граф как слой заливки.",
            Self::ShareCommunityGraph => "Подготовить ZIP-пакет графа на условиях CC0 и открыть форму сообщества.",
            Self::SaveGraphShelf => "Сохранить граф и его ресурсы на полке «Графы».",
            Self::ApplySavedGraph(_) => "Добавить сохранённый граф отдельным слоем заливки.",
            Self::OpenSavedGraph(_) => "Добавить копию графа на модель и открыть её на холсте.",
            Self::SavedGraphMenu(_) => "Открыть действия с сохранённым графом.",
            Self::ShareSavedGraph(_) => "Подготовить ZIP-пакет сохранённого графа на условиях CC0.",
            Self::ShareCommunityMaterial => "Подготовить ZIP-пакет материала на условиях CC0 и открыть форму сообщества.",
            Self::SavedMaterialMenu(_) => "Открыть действия с сохранённым материалом.",
            Self::ShareSavedMaterial(_) => "Подготовить ZIP-пакет сохранённого материала на условиях CC0.",
            Self::ShowSavedMaterial(_) => "Открыть папку с файлом сохранённого материала.",
            Self::DeleteSavedMaterial(_) => "Удалить пресет из библиотеки рисования.",
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
            Self::Preset(_) => "Взять цвет, шероховатость и металличность материала для кисти.",
            Self::ShelfClearSearch => "Очистить поиск ресурсов.",
            Self::ShelfTypeMenu => "Фильтровать по типу материала.",
            Self::ShelfType(_) => "Показать материалы выбранного типа.",
            Self::AddMaterial(_) => "Добавить материал как слой заливки.",
            Self::AddSmartMaterial(_) => "Добавить умный материал как группу слоёв.",
            Self::AddSmartGraph(_) => "Добавить умный материал одним слоем с редактируемым графом.",
            Self::SmartRecipeMenu(_) => "Выбрать способ добавления умного материала.",
            Self::AddGraphMaterial(_) => "Рассчитать граф на GPU и добавить заливку.",
            Self::AddGraphMask => "Подключить выход графа к маске выбранного слоя.",
            Self::ImportGraphFile => "Добавить граф Material Painter из файла .mpgraph или ZIP-пакета.",
            Self::ImportShaderGraph => "Открыть граф шейдинга Material Painter из файла .mpgraph или ZIP-пакета.",
            Self::ExportGraphFile => "Сохранить выбранный граф в файл .mpgraph.",
            Self::ExportGraphPackage => "Сохранить граф, узлы WGSL и изображения в ZIP-пакет.",
            Self::ExportGraphOutputs => "Рассчитать и сохранить выходы графа в отдельные карты PNG.",
            Self::GraphSelectNode(_) => "Показать параметры узла.",
            Self::GraphClearSelection => "Снять выделение узлов.",
            Self::GraphAdjust(..) => "Изменить параметр и пересчитать граф.",
            Self::GraphBitmapPick(_) => "Выбрать изображение для узла графа.",
            Self::GraphMapMenu(_) => "Выбрать запечённую карту модели.",
            Self::GraphRebake => "Обновить графы по запечённым картам.",
            Self::GraphCanvas => "Открыть или закрыть холст графа.",
            Self::GraphPage(_) => "Переключить страницу редактора графов.",
            Self::OpenProjectGraph { .. } => "Открыть существующий граф слоя.",
            Self::GraphResolutionMenu => "Выбрать размер расчёта графа или привязать его к набору текстур.",
            Self::GraphEdit(_) => "Изменить схему материала.",
            Self::ShaderToGraph { .. } => "Добавить узел WGSL в открытый граф.",
            Self::ShaderUpdateGraph { .. } => "Применить сохранённый исходник к узлу на холсте.",
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::editor) enum GraphPage {
    #[default]
    Canvas,
    Home,
    Project,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::editor) enum ProjectGraphKind {
    Fill,
    Mask,
    Folder,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::editor) struct ProjectGraphTarget {
    binding: (usize, usize),
    layer: usize,
    kind: ProjectGraphKind,
}

impl ProjectGraphTarget {
    fn exists(self, stack: &Stack) -> bool {
        let Some(layer) = stack.layers.get(self.layer) else { return false; };
        match self.kind {
            ProjectGraphKind::Fill => layer.texture_fill.as_ref().is_some_and(|fill| fill.graph.is_some()),
            ProjectGraphKind::Mask => layer.mask.as_ref().is_some_and(|mask| mask.graph.is_some()),
            ProjectGraphKind::Folder => folder_graph::can_open_at(stack, self.layer),
        }
    }
}

struct Session {
    stack: Stack,
    undo: Vec<history::Snapshot>,
    redo: Vec<history::Snapshot>,
    history_cache: history::Cache,
    history_limit: usize,
    tool: PaintTool,
    panel: Panel,
    view: View,
    category: Category,
    graph_node: Option<u64>,
    graph_pasted: Option<(u64, u64)>,
    graph_canvas: bool,
    graph_page: GraphPage,
    pending_project_graph: Option<ProjectGraphTarget>,
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
    mesh_menu: bool,
    mesh_loading: bool,
    #[cfg(feature = "mcp")]
    surface_selection: Option<((usize, usize), Result<surface_selection::Report, String>)>,
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
    fill_texture_picker: Option<usize>,
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
    graph_set_sizes: HashMap<(usize, usize), u32>,
    graph_resolution_menu: bool,
    graph_surfaces: HashMap<(usize, usize), Arc<material_graph::surface::Source>>,
    mesh_graph_refresh: std::collections::HashSet<(usize, usize)>,
    graph_map_menu: Option<material_graph::ParameterTarget>,
    histories: HashMap<(usize, usize), (Vec<history::Snapshot>, Vec<history::Snapshot>)>,
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
    bake_option_edit: Option<(bake::BakeControl, f32)>,
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
            history_cache: history::Cache::default(),
            history_limit: editing::recovery::DEFAULT_UNDO_STEPS,
            tool: PaintTool::Brush,
            panel: Panel::Layers,
            view: View::Split,
            category: Category::Materials,
            graph_node: None,
            graph_pasted: None,
            graph_canvas: false,
            graph_page: GraphPage::Canvas,
            pending_project_graph: None,
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
            mesh_menu: false,
            mesh_loading: false,
            #[cfg(feature = "mcp")]
            surface_selection: None,
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
            fill_texture_picker: None,
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
            graph_set_sizes: default(),
            graph_resolution_menu: false,
            graph_surfaces: HashMap::new(),
            mesh_graph_refresh: default(),
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
            bake_option_edit: None,
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

    fn live_mask_map(&self) -> Option<bake::BakeMap> {
        (self.panel == Panel::Maps && self.bake_live_masks && self.bake_live)
            .then(|| self.bake_view.unwrap_or(bake::BakeMap::WorldNormal))
    }

    fn set_cage_inspection(&mut self, enabled: bool) {
        self.bake_inspect_cage = enabled;
        if !enabled {
            self.bake_paint_cage = false;
        } else if self.view == View::Two {
            self.view = View::Split;
        }
        self.revision += 1;
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
            || (self.live_mask_map().is_some() && self.view != View::Two)
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
            PaintAction::SaveShelfMaterial | PaintAction::ExportShelfMaterial | PaintAction::ExportShelfPackage
            | PaintAction::ShareCommunityMaterial => {
                return !self.busy && self.can_capture_material(self.stack.selected);
            }
            PaintAction::SaveShelfMask => return !self.busy && self.has_mask(),
            PaintAction::ApplyShelfMaterial(i) => {
                return self.can_insert_material(ShelfMaterial::User(i), Some(self.insertion()));
            }
            PaintAction::ApplyDownloaded(i) => {
                return self.can_insert_material(ShelfMaterial::Downloaded(i), Some(self.insertion()));
            }
            PaintAction::AddSmartGraph(index) if index >= SMART_MATERIALS.len() => return false,
            PaintAction::OnlineMaterial(..) | PaintAction::AddSmartGraph(_) => {
                return self.binding.is_some() && !self.busy && !self.smart_loading
                    && self.resize_request.is_none() && self.stack.layers.len() < MAX_LAYERS
                    && self.insertion().1 <= MAX_LAYER_DEPTH;
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
            PaintAction::ApplySavedGraph(index) | PaintAction::OpenSavedGraph(index) => {
                return self.can_insert_material(ShelfMaterial::Graph(index), Some(self.insertion()));
            }
            PaintAction::CommunityGraph(index) => {
                return self.can_insert_material(ShelfMaterial::CommunityGraph(index), Some(self.insertion()));
            }
            PaintAction::ShareCommunityGraph | PaintAction::SaveGraphShelf => {
                return !self.smart_loading && self.panel != Panel::Preview && self.editing_shader().is_none()
                    && self.graph().is_some() && self.active_folder_view().is_none();
            }
            PaintAction::ExportGraphOutputs => {
                return self.binding.is_some() && !self.smart_loading && self.panel != Panel::Preview
                    && self.graph().is_some() && self.active_folder_view().is_none();
            }
            PaintAction::GraphCanvas => return self.graph_canvas_open() || self.graph().is_some()
                || (self.panel == Panel::Layers && !self.editing_mask
                    && folder_graph::can_open(&self.stack)),
            PaintAction::GraphPage(page) => return page != GraphPage::Canvas
                || (self.pending_project_graph.is_none() && (self.graph().is_some()
                    || (self.panel == Panel::Layers && !self.editing_mask && folder_graph::can_open(&self.stack)))),
            PaintAction::OpenProjectGraph { .. } => return self.graph_catalogue().is_some()
                && !self.busy && !self.smart_loading && self.resize_request.is_none()
                && self.pending_project_graph.is_none(),
            PaintAction::GraphResolutionMenu => return self.panel != Panel::Preview
                && self.graph().is_some() && self.active_folder_view().is_none(),
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
            PaintAction::FillTextureImport(channel) | PaintAction::FillTexturePicker(Some(channel)) => {
                return !self.busy && self.binding.is_some() && channel < CHANNELS.len()
                    && self.stack.layers.get(self.stack.selected).is_some_and(|layer| layer.kind == LayerKind::Fill);
            }
            PaintAction::TextureToChannel(texture, channel) => {
                return tips::has_texture(texture) && self.available(PaintAction::FillTextureImport(channel));
            }
            PaintAction::AddShelfTexture(texture) => {
                return self.can_insert_material(ShelfMaterial::Texture(texture), Some(self.insertion()));
            }
            PaintAction::FillTextureRemove(channel) => return !self.busy && self.active_texture_fill()
                .is_some_and(|fill| fill.images.get(channel).is_some_and(Option::is_some)),
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
            PaintAction::BakeInspectCage => return self.binding.is_some(),
            PaintAction::BakeQuality(_) | PaintAction::BakeIdSource(_) | PaintAction::BakeRayOptions(_)
            | PaintAction::BakeAutoEdgeReach | PaintAction::BakeResetSurface
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
            | PaintAction::BakePreferFrontFaces => {
                return self.bake_projection.path.is_some() && !self.bake_busy && !self.bake_picking;
            }
            PaintAction::BakeAverageNormals | PaintAction::BakeRange { .. } => {
                return self.binding.is_some() && !self.bake_busy && !self.bake_picking;
            }
            PaintAction::BakePreview(map) => return map.is_none_or(|map| {
                (self.bake_ready || (self.bake_live && self.binding.is_some()))
                    && self.bake_projection.maps.contains(map)
                    && (!map.requires_high_poly() || self.bake_projection.path.is_some())
            }),
            PaintAction::BakeLiveMasks => return self.bake_live_masks
                || (self.bake_live && self.binding.is_some()),
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
                        g.controls().get(i).is_some_and(|&(_, min, max, _)| {
                            if direction > 0 {
                                g.value(i) < max
                            } else {
                                g.value(i) > min
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
            PaintAction::Add(_) => self.can_insert_layer(self.insertion()),
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

    fn can_capture_material(&self, index: usize) -> bool {
        self.stack.layers.get(index).is_some_and(|layer| {
            layer.kind == LayerKind::Fill || (layer.kind == LayerKind::Folder
                && self.stack.layers[self.stack.block(index)].iter().any(|layer| layer.kind == LayerKind::Fill))
        })
    }

    fn capture_shelf_preset(
        &self, index: usize, kind: presets::CaptureKind, name: Option<&str>,
    ) -> Result<presets::Capture, &'static str> {
        let layer = self.stack.layers.get(index).ok_or("Выберите слой")?;
        let name = name.unwrap_or(&layer.name).to_owned();
        match kind {
            presets::CaptureKind::Mask => {
                let mask = layer.mask.clone().ok_or("У слоя нет маски")?;
                Ok(presets::Capture::Mask { name, mask })
            }
            presets::CaptureKind::Material if self.can_capture_material(index) => {
                let layers = self.stack.layers[self.stack.block(index)].to_vec();
                Ok(presets::Capture::Material { name, layers })
            }
            presets::CaptureKind::Material => Err("Выберите заливку или группу с заливкой"),
        }
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
        let mut generator = Generator::new(kind);
        if let Some(id) = custom_map.filter(|_| kind == GeneratorKind::CustomMap) {
            generator.custom[0] = custom::Input { id, weight: 100 };
        }
        self.apply_generator_value(generator, target);
    }

    fn apply_generator_value(&mut self, generator: Generator, target: Option<usize>) {
        if !self.available(PaintAction::ApplyGenerator(generator.kind, target)) {
            return;
        }
        self.checkpoint();
        self.stack.selected = target.unwrap_or(self.stack.selected);
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
                let (_, _, _, step) = generator.controls()[i];
                generator.set_value(i, generator.value(i) + direction.signum() as f32 * step);
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
            Parameter::TextureHeightOffset => fill.height_offset,
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
            Parameter::BakeOption(control) => control.value(&self.bake_projection),
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
                    Parameter::TextureHeightOffset => settings.height_offset = value,
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
            | Parameter::TextureOffsetV | Parameter::TextureSharpness | Parameter::TextureNormalStrength | Parameter::TextureHeightStrength | Parameter::TextureHeightOffset
            | Parameter::TextureOpacityStrength | Parameter::TextureEmissionTint(_) | Parameter::TextureSpecularStrength
            | Parameter::FillColorVariation | Parameter::FillRoughVariation | Parameter::FillHeightVariation
            | Parameter::FillVariationScale | Parameter::FillVariationSeed | Parameter::FillPartVariation => unreachable!(),
            Parameter::ExportPaddingWidth => self.export_settings.width = value.round().clamp(1., 256.) as u32,
            Parameter::ExportHeightStrength => self.export_settings.file.normal.height_strength = value.clamp(0., 8.),
            Parameter::CageRadius => self.cage_radius = value.clamp(5., 300.),
            Parameter::CageStrength => self.cage_strength = value.clamp(0.1, 4.),
            Parameter::BakeOption(_) => unreachable!("bake settings are edited through bake actions"),
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
            PaintAction::GraphSelectNode(node) => {
                self.graph_pasted = None;
                self.graph_node = Some(node);
            }
            PaintAction::GraphClearSelection => {
                self.graph_pasted = None;
                self.graph_node = None;
            }
            PaintAction::GraphMapMenu(target) => {
                self.graph_map_menu = (self.graph_map_menu != Some(target)).then_some(target);
            }
            PaintAction::GraphResolutionMenu => self.graph_resolution_menu = !self.graph_resolution_menu,
            PaintAction::GraphCanvas => {
                self.graph_resolution_menu = false;
                if self.panel == Panel::Preview {
                    self.shader_canvas = !self.shader_canvas;
                } else {
                    self.graph_canvas = !self.graph_canvas;
                    self.graph_page = GraphPage::Canvas;
                    self.pending_project_graph = None;
                }
                self.graph_map_menu = None;
                self.refresh_folder_view();
            }
            PaintAction::GraphPage(page) => {
                if self.panel != Panel::Layers {
                    self.graph_node = None;
                    self.graph_pasted = None;
                }
                self.panel = Panel::Layers;
                self.graph_canvas = true;
                self.graph_page = page;
                self.graph_map_menu = None;
                self.graph_resolution_menu = false;
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
                self.insert_new_layer(kind, None, self.insertion());
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
            PaintAction::PreviewMaterial(_) => {}
            PaintAction::TraceSamples(direction) => {
                let samples = self.trace_settings.samples;
                self.trace_settings.samples = if direction < 0 { samples / 2 } else { samples * 2 }
                    .clamp(*pathtrace::Settings::SAMPLE_RANGE.start(), *pathtrace::Settings::SAMPLE_RANGE.end());
            }
            PaintAction::TraceBounces(direction) => {
                self.trace_settings.bounces = self.trace_settings.bounces
                    .saturating_add_signed(i32::from(direction))
                    .clamp(*pathtrace::Settings::BOUNCE_RANGE.start(), *pathtrace::Settings::BOUNCE_RANGE.end());
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
                if let Some(previous) = self.undo.pop() {
                    let stack = self.history_cache.restore(previous);
                    let current = std::mem::replace(&mut self.stack, stack);
                    self.redo.push(self.history_cache.capture(current));
                    Self::trim_history(&mut self.redo, self.history_limit);
                    self.graph_bake = None;
                    if let Some(binding) = self.binding {
                        self.graph_surfaces.remove(&binding);
                    }
                }
            }
            PaintAction::Redo => {
                if let Some(next) = self.redo.pop() {
                    let stack = self.history_cache.restore(next);
                    let current = std::mem::replace(&mut self.stack, stack);
                    self.undo.push(self.history_cache.capture(current));
                    Self::trim_history(&mut self.undo, self.history_limit);
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
            PaintAction::MeshReplacement(_) => {},
            PaintAction::SaveCopy => {},
            PaintAction::SetHide(object, slot) => self.set_appearance.toggle_visibility((object, slot)),
            PaintAction::SetPin(object, slot) => self.set_appearance.toggle_pin((object, slot)),
            PaintAction::Resolution(size) => self.resize_request = Some(size),
            PaintAction::FillTextureImport(_) | PaintAction::TextureToChannel(..) => {}
            PaintAction::FillTextureRemove(channel) => {
                self.edit_texture_fill(|fill| fill.images[channel] = None);
            }
            PaintAction::FillTexturePicker(channel) => {
                self.fill_texture_picker = channel.filter(|channel| self.fill_texture_picker != Some(*channel));
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
            PaintAction::BakeQuality(_) | PaintAction::BakeIdSource(_) | PaintAction::BakeMapOptions | PaintAction::BakeRayOptions(_)
            | PaintAction::BakeAutoEdgeReach | PaintAction::BakeResetSurface
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
                self.multi_channel_brush = true;
                self.brush_channels |= 0b0000_0111;
                self.tool = PaintTool::Brush;
                self.editing_mask = false;
                self.show_mask = false;
            }
            PaintAction::AddSmartMaterial(_) | PaintAction::AddSmartGraph(_) | PaintAction::AddGraphMaterial(_) | PaintAction::AddGraphMask
            | PaintAction::ImportGraphFile | PaintAction::ImportShaderGraph | PaintAction::ExportGraphFile | PaintAction::ExportGraphPackage | PaintAction::ExportGraphOutputs
            | PaintAction::GraphAdjust(..) | PaintAction::GraphEdit(_) | PaintAction::GraphBitmapPick(_) | PaintAction::GraphRebake
            | PaintAction::ShaderToGraph { .. } | PaintAction::ShaderUpdateGraph { .. } | PaintAction::OpenProjectGraph { .. }
            | PaintAction::AddShelfTexture(_)
            | PaintAction::DropMaterial(ShelfMaterial::Smart(_) | ShelfMaterial::User(_) | ShelfMaterial::Downloaded(_) | ShelfMaterial::Texture(_) | ShelfMaterial::Graph(_) | ShelfMaterial::CommunityGraph(_), _)
            | PaintAction::DropMaterialOnSet(..)
            | PaintAction::ImportShelfMaterial | PaintAction::ExportShelfMaterial | PaintAction::ExportShelfPackage | PaintAction::SaveShelfMaterial | PaintAction::SaveShelfMask | PaintAction::ApplyShelfMaterial(_)
            | PaintAction::ApplyShelfMask(..) | PaintAction::ApplyGenerator(..)
            | PaintAction::ShelfClearSearch | PaintAction::ShelfTypeMenu | PaintAction::ShelfType(_)
            | PaintAction::ShelfTextureMenu(_) | PaintAction::SmartRecipeMenu(_) => unreachable!(),
            PaintAction::OnlineProvider(_) | PaintAction::OnlineRefresh | PaintAction::OnlinePage(_)
            | PaintAction::OnlineResolution(_) | PaintAction::OnlineMaterial(..)
            | PaintAction::DownloadOnlineMaterial(..) | PaintAction::ApplyDownloaded(_) | PaintAction::DownloadedPage(_)
            | PaintAction::DownloadedMenu(_) | PaintAction::ShowDownloadedFiles(_)
            | PaintAction::OpenDownloadedSource(_) | PaintAction::DeleteDownload(_) | PaintAction::ImportDownloaded
            | PaintAction::InstallCommunity(_) | PaintAction::CommunityGraph(_)
            | PaintAction::SaveGraphShelf | PaintAction::ApplySavedGraph(_) | PaintAction::OpenSavedGraph(_)
            | PaintAction::SavedGraphMenu(_) | PaintAction::ShareSavedGraph(_)
            | PaintAction::ShareCommunityGraph | PaintAction::ShareCommunityMaterial
            | PaintAction::SavedMaterialMenu(_) | PaintAction::ShareSavedMaterial(_)
            | PaintAction::ShowSavedMaterial(_) | PaintAction::DeleteSavedMaterial(_) => unreachable!(),
            PaintAction::AddMaterial(value)
            | PaintAction::DropMaterial(ShelfMaterial::Basic(value), _) => {
                let insertion = match action {
                    PaintAction::DropMaterial(_, target) => self.shelf_insertion(target).unwrap(),
                    _ => self.insertion(),
                };
                self.insert_basic_material(value, None, insertion);
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
        let Some((at, depth)) = insertion else {
            return false;
        };
        match material {
            ShelfMaterial::Basic(index) => index < MATERIALS.len() && self.can_insert_layer((at, depth)),
            ShelfMaterial::User(_) | ShelfMaterial::Downloaded(_) | ShelfMaterial::Graph(_) | ShelfMaterial::CommunityGraph(_) => !self.smart_loading && depth <= MAX_LAYER_DEPTH
                && self.stack.layers.len() < MAX_LAYERS,
            ShelfMaterial::Texture(index) => tips::has_texture(index) && !self.smart_loading
                && depth <= MAX_LAYER_DEPTH && self.stack.layers.len() < MAX_LAYERS,
            ShelfMaterial::Smart(index) => !self.smart_loading && depth < MAX_LAYER_DEPTH
                && SMART_MATERIALS.get(index).is_some_and(|recipe| self.stack.layers.len() + recipe.len() <= MAX_LAYERS),
        }
    }

    fn can_insert_layer(&self, (at, depth): (usize, usize)) -> bool {
        at <= self.stack.layers.len() && depth <= MAX_LAYER_DEPTH && self.stack.layers.len() < MAX_LAYERS
    }

    fn can_insert_layers(&self, layers: &[Layer], insertion: (usize, usize)) -> bool {
        self.can_insert_layer(insertion)
            && !layers.is_empty()
            && self.stack.layers.len() + layers.len() <= MAX_LAYERS
            && layers.iter().all(|layer| layer.depth + insertion.1 <= MAX_LAYER_DEPTH)
    }

    fn insert_new_layer(&mut self, kind: LayerKind, name: Option<&str>, insertion: (usize, usize)) {
        let name = name.map_or_else(|| {
            let label = match kind {
                LayerKind::Paint => "Слой",
                LayerKind::Fill => "Заливка",
                LayerKind::Folder => "Группа",
            };
            format!("{label} {}", self.stack.layers.len() + 1)
        }, str::to_owned);
        let mut layer = Layer::new(name, kind);
        if kind == LayerKind::Fill {
            use crate::editor::model_paint_data::solid;
            let [r, g, b] = self.brush_color();
            let roughness = (self.roughness * 2.55).round() as u8;
            let metallic = (self.metallic * 2.55).round() as u8;
            layer.channels[0] = Some(solid(self.texture_size, [r, g, b, 255]));
            layer.channels[1] = Some(solid(self.texture_size, [roughness, roughness, roughness, 255]));
            layer.channels[2] = Some(solid(self.texture_size, [metallic, metallic, metallic, 255]));
        }
        self.insert_layer_at(layer, insertion);
        self.editing_mask = false;
        self.show_mask = false;
    }

    fn insert_basic_material(&mut self, preset: usize, name: Option<&str>, insertion: (usize, usize)) {
        let mut layer = MATERIALS[preset].layer(self.texture_size);
        if let Some(name) = name {
            layer.name = name.to_owned();
        }
        self.insert_layer_at(layer, insertion);
        self.panel = Panel::Layers;
        self.editing_mask = false;
        self.show_mask = false;
    }

    #[cfg(test)]
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

    fn graph_output_preview(&self) -> Option<Arc<texture_fill::Bitmap>> {
        if self.editing_mask {
            self.stack.layers.get(self.stack.selected)?.mask.as_ref()?.graph.as_ref()?.output.clone()
        } else {
            self.active_texture_fill()?.images.iter().flatten().next().cloned()
        }
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
        if self.graph_catalogue().is_some() {
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
        let mut view = folder_graph::start(
            &mut self.stack, self.texture_size, self.revision, self.folder_view.as_ref(),
        );
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
        let previous = matches!(action, PaintAction::GraphEdit(material_graph::Edit::UpdateSource(..)))
            .then(|| material_graph::read(&view.payload)).transpose()?;
        let mut graph = previous.as_ref().unwrap_or(&view.graph).clone();
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
        Ok(self.finish_folder_graph(next, graph, node, previous.as_ref()))
    }

    fn edit_folder_batch(
        &mut self, batch: material_graph::Batch, kind: material_graph::BatchKind,
    ) -> Result<Option<(Stack, folder_graph::Masks)>, String> {
        let Some(view) = self.active_folder_view() else { return Ok(None) };
        let mut graph = view.graph.clone();
        let mut next = self.stack.clone();
        let mut images = view.images.clone();
        let node = batch.apply(&mut graph, &mut images, false, true)?;
        if !Arc::ptr_eq(&images, &view.images) {
            Arc::make_mut(&mut next.layers[next.selected].folder_images).extend(
                images.iter().filter(|(key, _)| !view.images.contains_key(*key))
                    .map(|(key, image)| (key.clone(), image.clone())),
            );
        }
        let changed = graph != view.graph;
        let pending = self.finish_folder_graph(next, graph, node, None);
        if changed && pending.is_none() {
            self.select_graph_batch(kind, node);
        }
        Ok(pending)
    }

    fn finish_folder_graph(
        &mut self, mut next: Stack, graph: material_painter_graph::Graph, node: Option<u64>,
        previous: Option<&material_painter_graph::Graph>,
    ) -> Option<(Stack, folder_graph::Masks)> {
        let view = self.active_folder_view()?;
        let previous = previous.unwrap_or(&view.graph);
        if &graph == previous { return None; }
        let masks = folder_graph::changed_masks(&self.stack, previous, &graph);
        folder_graph::store(&mut next.layers[next.selected], &graph);
        if masks.iter().all(|(_, graph)| graph.is_none()) {
            for (index, _) in masks {
                if let Some(mask) = next.layers[index].mask.as_mut() {
                    Arc::make_mut(mask).clear_graph();
                }
            }
            self.commit_folder_view(next, node);
            None
        } else {
            Some((next, masks))
        }
    }

    fn select_graph_batch(&mut self, kind: material_graph::BatchKind, last: Option<u64>) {
        use material_graph::BatchKind;
        match kind {
            BatchKind::Move => {}
            BatchKind::ReplaceSource => {
                self.graph_node = last;
                self.graph_pasted = None;
            }
            BatchKind::Remove => {
                self.graph_node = None;
                self.graph_pasted = None;
            }
            BatchKind::Paste(first) => {
                self.graph_node = last;
                self.graph_pasted = last.map(|last| (first, last));
            }
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
            _ => self.graph_canvas && (self.graph_page != GraphPage::Canvas || self.graph().is_some()),
        }
    }

    fn graph_catalogue(&self) -> Option<GraphPage> {
        (self.graph_canvas && !matches!(self.panel, Panel::Preview | Panel::Shader)
            && self.graph_page != GraphPage::Canvas).then_some(self.graph_page)
    }

    fn open_project_graph(&mut self, target: ProjectGraphTarget) -> bool {
        if self.binding != Some(target.binding) || !target.exists(&self.stack) {
            return false;
        }
        self.select_layer(target.layer);
        self.stack.reveal_selection();
        self.editing_mask = target.kind == ProjectGraphKind::Mask;
        self.show_mask = false;
        self.graph_pasted = None;
        self.panel = Panel::Layers;
        self.graph_canvas = true;
        self.graph_page = GraphPage::Canvas;
        self.graph_resolution_menu = false;
        self.revision += 1;
        self.refresh_folder_view();
        true
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

    fn update_shader_node(&mut self, id: u64, definition: usize, library_revision: usize) -> Result<PaintAction, String> {
        let library = material_graph::library();
        let next = library.defs.get(definition)
            .filter(|node| library_revision == node_library::revision() && node_library::available(&node.id))
            .ok_or("Библиотека узлов изменилась · вернитесь к графу ещё раз")?;
        let target = self.shader_graph_destination(next.shading)
            .ok_or("Открытый граф изменился")?;
        let source = target.payload["nodes"].as_array().into_iter().flatten()
            .find(|node| node["id"].as_u64() == Some(id)).and_then(|node| node["def"].as_str());
        if source != Some(next.id.as_str()) {
            return Err("Исходный узел больше не использует этот файл WGSL".into());
        }
        let mut graph = material_graph::read(&target.payload)?;
        let edit = material_graph::Edit::UpdateSource(id, definition);
        if self.folder_view_for(target.panel).is_some()
            && !folder_graph::can_edit(&self.stack, &graph, edit)
        {
            return Err("Исходник этого узла группы нельзя заменить".into());
        }
        edit.apply_for(&mut graph, None, next.shading)?;
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
        self.undo.push(self.history_cache.capture(previous));
        Self::trim_history(&mut self.undo, self.history_limit);
        self.redo.clear();
    }

    fn trim_history(history: &mut Vec<history::Snapshot>, limit: usize) {
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

    fn valid_layer_name(name: &str) -> Result<&str, &'static str> {
        let name = name.trim();
        if name.is_empty() || name.chars().any(char::is_control) || name.chars().count() > 120 {
            return Err("Введите имя слоя от 1 до 120 символов без управляющих знаков");
        }
        Ok(name)
    }

    fn rename_layer(&mut self, name: &str) -> Result<(), &'static str> {
        let name = Self::valid_layer_name(name)?;
        let index = self.stack.selected;
        if self.stack.layers[index].name != name {
            self.checkpoint();
            self.stack.layers[index].name = name.to_owned();
            self.revision += 1;
        }
        Ok(())
    }

    fn mark_resize_pending(&self, doc: &mut crate::editor::model_document::Document) {
        if self.resize_request.is_some() {
            doc.paint_busy = true;
            doc.paint_pending = true;
        }
    }

    fn choose_set(&mut self, doc: &mut crate::editor::model_document::Document, set: usize, now: f64) -> bool {
        let Some((object, slot)) = doc.paint_target(set) else { return false };
        let id = object.id;
        doc.select_object(id, false);
        doc.paint_set = Some(set);
        self.set_appearance.flash((id, slot), now);
        self.edit(PaintAction::Set(set));
        true
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
            PaintAction::GraphPage(page) => self.graph_canvas_open() && self.graph_page == page,
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
            PaintAction::MeshReplacement(MeshCommand::Toggle) => self.mesh_menu,
            PaintAction::SetHide(object, slot) => !self.set_appearance.visibility.shown(object, slot),
            PaintAction::SetPin(object, slot) => self.set_appearance.pinned.contains(&(object, slot)),
            PaintAction::BakePreview(map) => self.bake_view == map,
            PaintAction::BakeMatchNames => self.bake_projection.match_names,
            PaintAction::BakeFillMisses => self.bake_projection.fill_misses,
            PaintAction::BakePreferFrontFaces => self.bake_projection.prefer_front_faces,
            PaintAction::BakeAverageNormals => self.bake_projection.average_normals,
            PaintAction::BakeAutoFitCage => self.bake_projection.auto_fit,
            PaintAction::BakeQuality(quality) => self.bake_projection.quality == quality,
            PaintAction::BakeAutoEdgeReach => self.bake_projection.surface.auto_edge_reach,
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
    graph_shelf: graph_shelf::Library,
    online: online::Shelf,
    downloaded: online::downloaded::Library,
    community: online::community::Library,
    community_submission: Option<bevy::tasks::Task<Result<online::community::Submission, String>>>,
    texture_import: Option<TextureImport>,
    smart_import: Option<SmartImport>,
    shelf_drop: Option<(usize, sets::SetKey, ShelfMaterial)>,
    graph_export: Option<bevy::tasks::Task<Result<Option<std::path::PathBuf>, String>>>,
    graph_output_export: Option<bevy::tasks::Task<Result<Option<export::Report>, String>>>,
    #[cfg(test)]
    graph_outputs_test_folder: Option<Option<std::path::PathBuf>>,
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

    pub(in crate::editor) fn graph_shortcuts_active(&self, index: Option<usize>) -> bool {
        index.and_then(|index| self.sessions.get(&index)).is_some_and(|session| {
            session.graph_canvas_open() && session.graph_catalogue().is_none()
        })
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
            .is_some_and(|session| session.graph_catalogue().is_some()
                || (session.panel == Panel::Shader && !draft_preview))
    }

    pub(super) fn has_3d_view(&self, model: Option<usize>, draft_preview: bool) -> bool {
        model.and_then(|model| self.sessions.get(&model))
            .is_none_or(|session| session.graph_catalogue().is_none()
                && (session.graph_canvas_open() || session.viewport_layout(draft_preview) != View::Two))
    }
}

#[derive(Default)]
struct ShelfFilter {
    query: String,
    material: Option<MaterialType>,
    menu: bool,
    saved_material_menu: Option<usize>,
    saved_graph_menu: Option<usize>,
    texture_menu: Option<usize>,
    smart_recipe_menu: Option<usize>,
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
    mut viewer: ResMut<Viewer>,
    mut images: ResMut<Assets<Image>>,
    time: Res<Time<Real>>,
    device: Option<Res<bevy::render::renderer::RenderDevice>>,
    queue: Option<Res<bevy::render::renderer::RenderQueue>>,
    graph_labels: Query<(&GraphLabel, &bevy::text::EditableText)>,
    mut layer_names: Query<(&LayerNameField, &mut bevy::text::EditableText),
        (Without<GraphLabel>, Without<ui::HistoryPreference>)>,
    baked: Res<bake::State>,
    mut graph_ui: Option<ResMut<ui::PaintUi>>,
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
    if let Some(result) = paint.graph_output_export.as_mut()
        .and_then(|job| bevy::tasks::block_on(bevy::tasks::poll_once(job)))
    {
        paint.graph_output_export = None;
        match result {
            Ok(Some(report)) => editor.status = format!("Карты графа сохранены · {} · {}", report.files, report.path.display()),
            Ok(None) => {}
            Err(error) => editor.status = format!("Не удалось экспортировать карты графа · {error}"),
        }
    }
    if let Some(result) = paint.graph_shelf.poll() {
        editor.status = match result {
            Ok(name) => format!("Граф сохранён на полке · {name}"),
            Err(error) => format!("Библиотека графов · {error}"),
        };
    }
    if let Some(result) = paint.community_submission.as_mut()
        .and_then(|job| bevy::tasks::block_on(bevy::tasks::poll_once(job)))
    {
        paint.community_submission = None;
        match result {
            Ok(submission) => {
                let mut status = format!("Пакет CC0 подготовлен · {}", submission.path.display());
                if submission.omitted > 0 {
                    status.push_str(&format!(" · Пропущено данных UV и ссылок на карты: {}", submission.omitted));
                }
                if submission.adjusted > 0 {
                    status.push_str(&format!(" · Адаптировано настроек: {}", submission.adjusted));
                }
                if !cfg!(test) {
                    if let Some(folder) = submission.path.parent()
                        && let Err(error) = crate::project_manager::reveal_folder(folder)
                    {
                        status.push_str(&format!(" · Не удалось открыть папку · {error}"));
                    }
                    if let Err(error) = online::community::open_form(&submission.submit_url()) {
                        status.push_str(&format!(" · Не удалось открыть форму отправки · {error}"));
                    }
                }
                editor.status = status;
            }
            Err(error) => editor.status = format!("Не удалось подготовить пакет сообщества · {error}"),
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
    if let Some(result) = paint.library.poll_delete() {
        editor.status = match result {
            Ok(index) => {
                if paint.shelf.saved_material_menu == Some(index) {
                    paint.shelf.saved_material_menu = None;
                }
                "Пресет удалён из библиотеки".into()
            }
            Err(error) => format!("Не удалось удалить пресет · {error}"),
        };
    }
    paint.online.update();
    if let Some((operation, result)) = paint.downloaded.update() {
        use online::downloaded::Operation;
        editor.status = match (operation, result) {
            (Operation::Download, Ok(())) => "Материал сохранён на полке «Скачанные»".into(),
            (Operation::Download, Err(error)) => format!("Не удалось скачать материал · {error}"),
            (Operation::Delete, Ok(())) => "Скачанные карты удалены".into(),
            (Operation::Delete, Err(error)) => format!("Не удалось удалить скачанные карты · {error}"),
            (Operation::Import, Ok(())) => "Материал добавлен на полку «Скачанные»".into(),
            (Operation::Import, Err(error)) => format!("Не удалось добавить папку материала · {error}"),
        };
    }
    if let Some(result) = paint.community.poll() {
        match result {
            Ok(installed) => {
                if let Some((entry, omitted)) = installed.smart {
                    paint.library.upsert(entry);
                    if omitted > 0 {
                        editor.status = format!("Добавлен материал «{}» · Пропущено данных: {omitted}", installed.record.name);
                    } else {
                        editor.status = format!("В библиотеку добавлен материал «{}»", installed.record.name);
                    }
                } else {
                    editor.status = format!("В библиотеку добавлен ресурс «{}»", installed.record.name);
                }
            }
            Err(error) => editor.status = format!("Библиотека сообщества · {error}"),
        }
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
        if result.is_ok() && matches!(job.action, PaintAction::OnlineMaterial(..)) {
            paint.downloaded.refresh();
        }
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
                            PaintAction::GraphBitmapPick(material_graph::ParameterTarget::Node(id, _))
                            | PaintAction::GraphEdit(material_graph::Edit::UpdateSource(id, _)) => Some(id),
                            _ => None,
                        };
                        session.commit_folder_view(stack, node);
                        if let PaintAction::GraphEdit(material_graph::Edit::Batch(_, kind)) = job.action {
                            let last = session.active_folder_view().and_then(|view| view.graph.nodes.last().map(|node| node.id));
                            session.select_graph_batch(kind, last);
                        }
                    }
                    Ok(layers) if job.action == PaintAction::GraphRebake => {
                        session.stack.layers = layers;
                        session.graph_bake = job.maps.as_ref().map(Arc::downgrade);
                        session.graph_set_sizes.insert(job.binding, job.size);
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
                                session.graph_page = GraphPage::Canvas;
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
                        if let PaintAction::GraphEdit(material_graph::Edit::UpdateSource(id, _)) = job.action {
                            session.graph_node = Some(id);
                        }
                        if let PaintAction::GraphEdit(material_graph::Edit::Add(..) | material_graph::Edit::AddConnected(..)) = job.action {
                            session.graph_node = session.graph()
                                .and_then(|graph| graph["nodes"].as_array()?.last()?.get("id")?.as_u64());
                        }
                        if let PaintAction::GraphEdit(material_graph::Edit::Batch(_, kind)) = job.action {
                            let last = session.graph()
                                .and_then(|graph| graph["nodes"].as_array()?.last()?.get("id")?.as_u64());
                            session.select_graph_batch(kind, last);
                        }
                    }
                    Ok(layers) if session.can_insert_layers(&layers, job.insertion) =>
                    {
                        session.insert_smart_material(layers, job.insertion);
                        if matches!(job.action, PaintAction::OpenSavedGraph(_))
                            || (session.graph_catalogue().is_some()
                                && matches!(job.action, PaintAction::AddGraphMaterial(_) | PaintAction::ImportGraphFile))
                        {
                            session.graph_canvas = true;
                            session.graph_page = GraphPage::Canvas;
                            session.graph_node = None;
                            session.graph_pasted = None;
                            session.graph_map_menu = None;
                        }
                    }
                    Ok(_) => editor.status = "Материал не помещается в стек слоёв".into(),
                    Err(error) => {
                        if job.action == PaintAction::GraphRebake {
                            session.graph_bake = job.maps.as_ref().map(Arc::downgrade);
                            session.graph_set_sizes.insert(job.binding, job.size);
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
                    && session.available(PaintAction::FillTextureImport(job.channel))
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
        if let Some(target) = session.pending_project_graph {
            let same_set = doc.paint_target(session.set).is_some_and(|(object, slot)| (object.id, slot) == target.binding);
            if !same_set || session.graph_catalogue().is_none() {
                session.pending_project_graph = None;
            } else if session.binding == Some(target.binding) && !session.busy && !session.smart_loading {
                session.pending_project_graph = None;
                if !session.open_project_graph(target) {
                    editor.status = "Граф слоя больше недоступен".into();
                }
            }
        }
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
            let mut batch = if let PaintAction::GraphEdit(material_graph::Edit::Batch(id, _)) = action {
                let Some(batch) = graph_ui.as_deref_mut().and_then(|ui| ui.take_graph_batch(id, index, session))
                    .filter(|_| session.graph_canvas_open() && !editor.browser_popup && !editor.hdri_library_open)
                else { continue };
                Some(batch)
            } else {
                None
            };
            if matches!(action, PaintAction::MeshReplacement(_) | PaintAction::SaveCopy) {
                continue;
            }
            if let PaintAction::PreviewMaterial(edit) = action {
                if !editor.browser_popup && !editor.hdri_library_open
                    && let Some(doc) = editing.documents.get_mut(&index)
                    && doc.can_store_paint_preview()
                {
                    let previous = (doc.paint_preview, doc.paint_uv, doc.paint_post, doc.model_camera);
                    match edit {
                        PreviewEdit::AutomaticHeight => {
                            let height = &mut doc.paint_preview.height_strength;
                            *height = if height.is_some() { None } else { Some(1.) };
                        }
                        PreviewEdit::BakedNormal => doc.paint_preview.use_baked_normal = !doc.paint_preview.use_baked_normal,
                        PreviewEdit::UvWire => doc.paint_uv.show_wire = !doc.paint_uv.show_wire,
                        PreviewEdit::UvSeams2d => doc.paint_uv.show_seams_2d = !doc.paint_uv.show_seams_2d,
                        PreviewEdit::UvSeams3d => doc.paint_uv.show_seams_3d = !doc.paint_uv.show_seams_3d,
                        PreviewEdit::Value { model, parameter, value }
                            if model == index && (parameter.min()..=parameter.max()).contains(&value) =>
                        {
                            match parameter {
                                PreviewParameter::FieldOfView => viewer.set_paint_fov(index, doc, value.to_radians()),
                                PreviewParameter::Height => doc.paint_preview.height_strength = Some(value),
                                PreviewParameter::Emission => doc.paint_preview.emission_strength = value,
                                PreviewParameter::Occlusion => doc.paint_preview.ao_strength = value / 100.,
                                PreviewParameter::UvWireOpacity => doc.paint_uv.wire_opacity = value / 100.,
                            }
                        }
                        PreviewEdit::Reset { model } if model == index => {
                            use crate::editor::model_document::{PaintPreview, PaintUv};
                            doc.paint_preview = PaintPreview {
                                height_strength: doc.paint_preview.height_strength,
                                use_baked_normal: doc.paint_preview.use_baked_normal,
                                ..default()
                            };
                            doc.paint_uv = PaintUv { show_wire: doc.paint_uv.show_wire, ..default() };
                            doc.paint_post = None;
                            viewer.set_paint_fov(index, doc, FOV);
                        }
                        PreviewEdit::Value { .. } | PreviewEdit::Reset { .. } => {}
                    }
                    if (doc.paint_preview, doc.paint_uv, doc.paint_post, doc.model_camera) != previous {
                        doc.ui_revision += 1;
                    }
                }
                continue;
            }
            if let PaintAction::ShaderToGraph { model, definition, library_revision }
                | PaintAction::ShaderUpdateGraph { model, definition, library_revision, .. } = action
            {
                if model != index || editor.browser_popup || editor.hdri_library_open {
                    continue;
                }
                if paint.smart_import.is_some() {
                    editor.status = "Дождитесь завершения операции с материалом".into();
                    continue;
                }
                let result = match action {
                    PaintAction::ShaderUpdateGraph { node, .. } => session.update_shader_node(node, definition, library_revision),
                    _ => session.insert_shader_node(definition, library_revision),
                };
                match result {
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
                    let edit = if let PaintAction::GraphEdit(material_graph::Edit::Batch(_, kind)) = action {
                        session.edit_folder_batch(batch.take().unwrap(), kind)
                    } else {
                        session.edit_folder_view(action)
                    };
                    match edit {
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
                if matches!(action, PaintAction::GraphEdit(material_graph::Edit::Batch(..))) {
                    Arc::make_mut(&mut images).extend(stack.layers[stack.selected].folder_images.iter()
                        .map(|(key, image)| (key.clone(), image.clone())));
                }
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
                        let inputs = material_graph::Inputs { maps: maps.as_deref(), surface: surface.as_deref(), live: None };
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
                let mut batch_selection = None;
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
                                    if let material_graph::Edit::Batch(_, kind) = edit {
                                        let node = batch.take().unwrap().apply(&mut graph, &mut Arc::default(), true, false)?;
                                        batch_selection = Some((kind, node));
                                        node
                                    } else {
                                        edit.apply_for(&mut graph, label.as_deref(), true)?
                                    }
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
                if let Some((kind, last)) = batch_selection {
                    session.select_graph_batch(kind, last);
                }
                session.revision += 1;
                continue;
            }
            match action {
                PaintAction::OpenProjectGraph { model, target } => {
                    if model != index || !session.available(action) || editor.browser_popup || editor.hdri_library_open {
                        continue;
                    }
                    if session.binding == Some(target.binding) {
                        if !session.open_project_graph(target) {
                            editor.status = "Граф слоя больше недоступен".into();
                        }
                    } else if let Some(doc) = editing.documents.get_mut(&index) {
                        let set = doc.objects.iter()
                            .flat_map(|object| (0..object.materials.len()).map(move |slot| (object, slot)))
                            .enumerate().find_map(|(set, (object, slot))| {
                                ((object.id, slot) == target.binding
                                    && object.materials[slot].paint.as_ref().is_some_and(|paint| target.exists(&paint.stack)))
                                    .then_some(set)
                            });
                        if set.is_some_and(|set| session.choose_set(doc, set, time.elapsed_secs_f64())) {
                            session.pending_project_graph = Some(target);
                        } else {
                            editor.status = "Граф слоя больше недоступен".into();
                        }
                    }
                    continue;
                }
                PaintAction::GraphPage(GraphPage::Home) => {
                    paint.library.ensure();
                    paint.graph_shelf.ensure(paint.library.root.as_deref());
                }
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
                        let kind = if action == PaintAction::SaveShelfMask {
                            presets::CaptureKind::Mask
                        } else {
                            presets::CaptureKind::Material
                        };
                        let Ok(capture) = session.capture_shelf_preset(session.stack.selected, kind, None) else {
                            continue;
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
                        && let Some(entry) = paint.library.entry(i)
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
                        && let Some(entry) = paint.library.entry(i)
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
                            let preparation = presets::MaterialPreparation {
                                maps: maps.clone(), surface: surface.clone(), renderer,
                            };
                            session.smart_loading = true;
                            session.revision += 1;
                            paint.smart_import = Some(SmartImport {
                                model: index, binding, set: session.set, stack: session.stack.clone(),
                                action, mask: false, folder_graph: false, insertion, size,
                                maps: maps.clone(), surface: surface.clone(),
                                task: bevy::tasks::AsyncComputeTaskPool::get()
                                    .spawn(async move { preparation.load(&path, size) }),
                            });
                        } else {
                            editor.status = "Материал не помещается в стек слоёв".into();
                        }
                    }
                    continue;
                }
                PaintAction::ApplyDownloaded(i) | PaintAction::DropMaterial(ShelfMaterial::Downloaded(i), _) => {
                    if paint.smart_import.is_none() && session.available(action)
                        && let Some(binding) = session.binding
                        && let Some(root) = paint.downloaded.root.clone()
                        && paint.downloaded.operation().is_none_or(|operation| !matches!(operation, online::downloaded::Operation::Delete))
                        && let Some(record) = paint.downloaded.record(i).cloned()
                    {
                        let size = session.texture_size;
                        let insertion = match action {
                            PaintAction::DropMaterial(_, target) => session.shelf_insertion(target).unwrap(),
                            _ => session.insertion(),
                        };
                        session.smart_loading = true;
                        session.revision += 1;
                        paint.smart_import = Some(SmartImport {
                            model: index, binding, set: session.set, stack: session.stack.clone(),
                            action, mask: false, folder_graph: false, insertion, size, maps: None, surface: None,
                            task: bevy::tasks::IoTaskPool::get().spawn(async move {
                                record.layer(&root, size)
                            }),
                        });
                    }
                    continue;
                }
                PaintAction::Category(Category::Smart | Category::Generators) => paint.library.ensure(),
                PaintAction::Category(category @ (Category::Online | Category::Downloaded)) => {
                    paint.library.ensure();
                    paint.downloaded.ensure(paint.library.root.as_deref().map(online::cache_in));
                    if category == Category::Online {
                        paint.community.ensure(paint.library.root.clone());
                        let provider = paint.online.provider;
                        paint.online.show(provider, false);
                    }
                }
                PaintAction::Category(Category::Graphs) => {
                    paint.library.ensure();
                    paint.graph_shelf.ensure(paint.library.root.as_deref());
                    paint.community.ensure(paint.library.root.clone());
                }
                PaintAction::OnlineProvider(provider) => {
                    paint.online.show(provider, false);
                    continue;
                }
                PaintAction::OnlineRefresh => {
                    let provider = paint.online.provider;
                    paint.online.show(provider, true);
                    continue;
                }
                PaintAction::OnlinePage(page) => {
                    paint.online.page = page;
                    paint.online.revision += 1;
                    continue;
                }
                PaintAction::OnlineResolution(resolution) => {
                    if matches!(resolution, 1 | 2 | 4) {
                        paint.online.resolution = resolution;
                        paint.online.revision += 1;
                    }
                    continue;
                }
                PaintAction::OnlineMaterial(provider, entry) => {
                    if paint.smart_import.is_none() && session.available(action)
                        && !paint.downloaded.busy()
                        && provider != online::Provider::Community
                        && provider == paint.online.provider
                        && let Some(binding) = session.binding
                        && let Some(asset) = paint.online.entries.get(entry).cloned()
                    {
                        let resolution = paint.online.resolution();
                        let cache = paint.downloaded.root.clone();
                        session.smart_loading = true;
                        session.revision += 1;
                        paint.smart_import = Some(SmartImport {
                            model: index, binding, set: session.set, stack: session.stack.clone(),
                            action, mask: false, folder_graph: false, insertion: session.insertion(),
                            size: session.texture_size, maps: None, surface: None,
                            task: bevy::tasks::IoTaskPool::get().spawn(async move {
                                online::material_in(cache, provider, &asset, resolution)
                            }),
                        });
                    }
                    continue;
                }
                PaintAction::DownloadOnlineMaterial(provider, entry) => {
                    if provider != online::Provider::Community
                        && provider == paint.online.provider
                        && !paint.smart_import.as_ref().is_some_and(|job| matches!(job.action, PaintAction::OnlineMaterial(..)))
                        && let Some(asset) = paint.online.entries.get(entry).cloned()
                    {
                        editor.status = match paint.downloaded.start(provider, asset, paint.online.resolution()) {
                            Ok(()) => "Загрузка карт материала…".into(),
                            Err(error) => error,
                        };
                    }
                    continue;
                }
                PaintAction::DownloadedPage(page) => {
                    paint.downloaded.page = page;
                    paint.downloaded.menu = None;
                    paint.downloaded.revision += 1;
                    continue;
                }
                PaintAction::DownloadedMenu(id) => {
                    paint.downloaded.menu = id.filter(|id| {
                        paint.downloaded.menu != Some(*id) && paint.downloaded.record(*id).is_some()
                    });
                    paint.downloaded.revision += 1;
                    continue;
                }
                PaintAction::ShowDownloadedFiles(id) => {
                    if let Some(root) = paint.downloaded.root.as_deref()
                        && let Some(record) = paint.downloaded.record(id)
                        && !cfg!(test)
                        && let Err(error) = crate::project_manager::reveal_folder(&record.directory(root))
                    {
                        editor.status = format!("Не удалось открыть папку · {error}");
                    }
                    continue;
                }
                PaintAction::OpenDownloadedSource(id) => {
                    if let Some(record) = paint.downloaded.record(id)
                        && !cfg!(test)
                        && let Err(error) = record.source_url().and_then(|url| {
                            online::community::open_form(&url).map_err(|error| error.to_string())
                        })
                    {
                        editor.status = format!("Не удалось открыть страницу материала · {error}");
                    }
                    continue;
                }
                PaintAction::DeleteDownload(id) => {
                    if paint.smart_import.is_none() {
                        if let Err(error) = paint.downloaded.delete(id) {
                            editor.status = error;
                        }
                    } else {
                        editor.status = "Дождитесь завершения операции с материалом".into();
                    }
                    continue;
                }
                PaintAction::ImportDownloaded => {
                    if !paint.smart_import.as_ref().is_some_and(|job| matches!(job.action, PaintAction::OnlineMaterial(..)))
                        && let Err(error) = paint.downloaded.start_import(|| {
                            crate::project_manager::choose_folder("Папка скачанного материала с meta.json")
                                .map_err(|error| error.to_string())
                        })
                    {
                        editor.status = error;
                    }
                    continue;
                }
                PaintAction::InstallCommunity(i) => {
                    if paint.online.provider == online::Provider::Community
                        && let Some(entry) = paint.online.entries.get(i).cloned()
                    {
                        if let Some(root) = paint.library.root.clone() {
                            paint.community.start(root, entry);
                        } else {
                            editor.status = "Папка библиотеки рисования недоступна".into();
                        }
                    }
                    continue;
                }
                PaintAction::SavedMaterialMenu(index) => {
                    paint.shelf.saved_material_menu = index.filter(|index| {
                        paint.shelf.saved_material_menu != Some(*index)
                            && paint.library.entry(*index).is_some_and(|entry| matches!(entry.kind, presets::Kind::Material { .. }))
                    });
                    paint.shelf.revision += 1;
                    continue;
                }
                PaintAction::ShowSavedMaterial(index) => {
                    if let Some(folder) = paint.library.entry(index).and_then(|entry| entry.path.parent())
                        && !cfg!(test)
                        && let Err(error) = crate::project_manager::reveal_folder(folder)
                    {
                        editor.status = format!("Не удалось открыть папку пресета · {error}");
                    }
                    continue;
                }
                PaintAction::DeleteSavedMaterial(index) => {
                    if paint.smart_import.is_some() || paint.community_submission.is_some() {
                        editor.status = "Дождитесь завершения операции с материалом".into();
                    } else if let Err(error) = paint.library.start_delete(index) {
                        editor.status = format!("Не удалось удалить пресет · {error}");
                    }
                    continue;
                }
                PaintAction::SavedGraphMenu(index) => {
                    paint.shelf.saved_graph_menu = index.filter(|index| {
                        paint.shelf.saved_graph_menu != Some(*index) && *index < paint.graph_shelf.entries.len()
                    });
                    paint.shelf.revision += 1;
                    continue;
                }
                PaintAction::SaveGraphShelf => {
                    if session.available(action) && let Some((payload, images)) = session.graph_data() {
                        paint.library.ensure();
                        paint.graph_shelf.ensure(paint.library.root.as_deref());
                        if let Err(error) = paint.graph_shelf.start_save(payload.clone(), images.clone(), session.graph_output_preview()) {
                            editor.status = format!("Не удалось сохранить граф · {error}");
                        }
                    }
                    continue;
                }
                PaintAction::ShareCommunityGraph | PaintAction::ShareCommunityMaterial | PaintAction::ShareSavedMaterial(_) | PaintAction::ShareSavedGraph(_) => {
                    if !session.available(action) { continue; }
                    if paint.community_submission.is_some() {
                        editor.status = "Дождитесь подготовки предыдущего пакета".into();
                        continue;
                    }
                    paint.library.ensure();
                    let Some(root) = paint.library.root.clone() else {
                        editor.status = "Папка библиотеки рисования недоступна".into();
                        continue;
                    };
                    let draft = if let PaintAction::ShareSavedGraph(index) = action {
                        let Some(entry) = paint.graph_shelf.entries.get(index).cloned() else { continue; };
                        online::community::Draft::SavedGraph(entry)
                    } else if let PaintAction::ShareSavedMaterial(index) = action {
                        let Some(entry) = paint.library.entry(index).cloned() else { continue; };
                        online::community::Draft::SavedSmart(entry)
                    } else if action == PaintAction::ShareCommunityGraph {
                        let Some((payload, images)) = session.graph_data() else { continue; };
                        let preview = session.graph_output_preview();
                        online::community::Draft::Graph { payload: payload.clone(), images: images.clone(), preview }
                    } else {
                        let selected = &session.stack.layers[session.stack.selected];
                        online::community::Draft::Smart(presets::Capture::Material {
                            name: selected.name.clone(),
                            layers: session.stack.layers[session.stack.block(session.stack.selected)].to_vec(),
                        })
                    };
                    editor.status = "Подготовка пакета сообщества…".into();
                    paint.community_submission = Some(bevy::tasks::IoTaskPool::get().spawn(async move {
                        online::community::prepare(&root, draft)
                    }));
                    continue;
                }
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
                PaintAction::ExportGraphOutputs => {
                    if !session.available(action) {
                        continue;
                    }
                    if paint.graph_output_export.is_some() {
                        editor.status = "Дождитесь предыдущего экспорта карт графа".into();
                        continue;
                    }
                    let Some((device, queue)) = device.as_deref().zip(queue.as_deref()) else {
                        editor.status = "GPU для расчёта графа недоступен".into();
                        continue;
                    };
                    let Some((data, images)) = session.graph_data() else { continue; };
                    let (data, images) = (data.clone(), images.clone());
                    let (device, queue) = (device.clone(), queue.clone());
                    let size = session.texture_size;
                    let binding = session.binding.unwrap();
                    let doc = editing.documents.get(&index);
                    let maps = doc.and_then(|doc| baked.maps(index, doc, binding, size)).cloned();
                    let surface = doc.and_then(|doc| material_graph::surface::Source::capture(doc, binding));
                    #[cfg(test)]
                    let test_folder = paint.graph_outputs_test_folder.take();
                    paint.graph_output_export = Some(bevy::tasks::IoTaskPool::get().spawn(async move {
                        let choose = || crate::project_manager::choose_folder("Папка для выходных карт графа")
                            .map_err(|error| error.to_string());
                        #[cfg(test)]
                        let directory = test_folder.map_or_else(choose, Ok)?;
                        #[cfg(not(test))]
                        let directory = choose()?;
                        let Some(directory) = directory else { return Ok(None); };
                        let inputs = material_graph::Inputs {
                            maps: maps.as_deref(),
                            surface: surface.as_ref(),
                            live: None,
                        };
                        material_graph::export_outputs(&directory, &data, &images, size, inputs, &device, &queue)
                            .map(Some)
                    }));
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
                PaintAction::AddGraphMaterial(_) | PaintAction::AddSmartGraph(_) | PaintAction::AddGraphMask | PaintAction::ImportGraphFile
                | PaintAction::CommunityGraph(_) | PaintAction::ApplySavedGraph(_) | PaintAction::OpenSavedGraph(_)
                | PaintAction::DropMaterial(ShelfMaterial::Graph(_) | ShelfMaterial::CommunityGraph(_), _)
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
                            if matches!(action, PaintAction::GraphEdit(material_graph::Edit::Resolution(_)))
                                && std::mem::take(&mut session.graph_resolution_menu)
                            {
                                session.revision += 1;
                            }
                            let (previous_payload, previous_images) = session.graph_data().unwrap();
                            let mut previous_images = previous_images.clone();
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
                                    let result = if let Some(batch) = batch.take() {
                                        batch.apply(&mut graph, &mut previous_images, false, false)
                                    } else {
                                        edit.apply(&mut graph, label.as_deref())
                                    };
                                    match result {
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
                                                    if !matches!(edit, material_graph::Edit::Move(..) | material_graph::Edit::Batch(_, material_graph::BatchKind::Move)) {
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
                        let shelf_graph = match action {
                            PaintAction::ApplySavedGraph(i) | PaintAction::OpenSavedGraph(i)
                            | PaintAction::DropMaterial(ShelfMaterial::Graph(i), _) => {
                                let Some(entry) = paint.graph_shelf.entries.get(i) else { continue; };
                                Some(entry.path.clone())
                            }
                            PaintAction::CommunityGraph(i)
                            | PaintAction::DropMaterial(ShelfMaterial::CommunityGraph(i), _) => {
                                let Some(path) = paint.community.records.get(i)
                                    .filter(|record| record.kind == online::community::Kind::Graph)
                                    .and_then(|record| record.path(paint.library.root.as_deref()?))
                                else { continue; };
                                Some(path)
                            }
                            _ => None,
                        };
                        let insertion = match action {
                            PaintAction::DropMaterial(_, target) => session.shelf_insertion(target).unwrap(),
                            _ => session.insertion(),
                        };
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
                            insertion,
                            size,
                            maps: maps.clone(),
                            surface: surface.clone(),
                            task: bevy::tasks::AsyncComputeTaskPool::get().spawn(async move {
                                let inputs = material_graph::Inputs { maps: maps.as_deref(), surface: surface.as_deref(), live: None };
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
                                    (PaintAction::AddSmartGraph(recipe), _) => {
                                        let (graph, images) = SMART_MATERIALS[recipe].graph(size)?;
                                        material_graph::prepare_graph(graph, &images, size, inputs, &device, &queue)
                                    }
                                    (PaintAction::CommunityGraph(_) | PaintAction::ApplySavedGraph(_) | PaintAction::OpenSavedGraph(_)
                                    | PaintAction::DropMaterial(ShelfMaterial::Graph(_) | ShelfMaterial::CommunityGraph(_), _), _) => {
                                        let path = shelf_graph.as_ref().ok_or("Граф из библиотеки недоступен")?;
                                        let (graph, images) = material_graph::file::load(path)?;
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
                | PaintAction::DropMaterial(ShelfMaterial::Smart(recipe), _)
                | PaintAction::AddShelfTexture(recipe)
                | PaintAction::DropMaterial(ShelfMaterial::Texture(recipe), _) => {
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
                                if matches!(action, PaintAction::AddShelfTexture(_) | PaintAction::DropMaterial(ShelfMaterial::Texture(_), _)) {
                                    tips::texture_layer(recipe)
                                } else {
                                    SMART_MATERIALS[recipe].prepare(size)
                                }
                            }),
                        });
                    }
                    continue;
                }
                PaintAction::FillTexturePicker(channel) => {
                    if channel.is_some() {
                        paint.tips.ensure_grains(&mut images);
                    }
                }
                PaintAction::ShelfTextureMenu(texture) => {
                    paint.shelf.texture_menu = texture.filter(|texture| {
                        tips::has_texture(*texture) && paint.shelf.texture_menu != Some(*texture)
                    });
                    paint.shelf.revision += 1;
                    continue;
                }
                PaintAction::SmartRecipeMenu(recipe) => {
                    paint.shelf.smart_recipe_menu = recipe.filter(|recipe| {
                        *recipe < SMART_MATERIALS.len() && paint.shelf.smart_recipe_menu != Some(*recipe)
                    });
                    paint.shelf.revision += 1;
                    continue;
                }
                PaintAction::FillTextureImport(channel) | PaintAction::TextureToChannel(_, channel) => {
                    if paint.texture_import.is_none()
                        && session.available(action)
                        && let Some(binding) = session.binding
                    {
                        let directory = editor.project.asset_root.clone();
                        session.fill_texture_picker = None;
                        session.revision += 1;
                        paint.texture_import = Some(TextureImport {
                            model: index, binding, stack: session.stack.clone(), channel,
                            task: bevy::tasks::IoTaskPool::get().spawn(async move {
                                match action {
                                    PaintAction::TextureToChannel(texture, _) => tips::texture_bitmap(texture).map(Some),
                                    _ => crate::editor::asset_monitor::choose_file_path(&directory)
                                        .map_err(|error| error.to_string())?
                                        .map(|path| texture_fill::Bitmap::open(&path)).transpose(),
                                }
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
                PaintAction::BakeQuality(_) | PaintAction::BakeIdSource(_) | PaintAction::BakeMapOptions | PaintAction::BakeRayOptions(_)
                | PaintAction::BakeAutoEdgeReach | PaintAction::BakeResetSurface
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
                    session.choose_set(doc, set, time.elapsed_secs_f64());
                }
                continue;
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
                session.mark_resize_pending(doc);
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
        let target_size = device.as_deref()
            .filter(|_| session.graph_set_sizes.get(&binding) != Some(&session.texture_size))
            .map(|device| session.texture_size.min(material_graph::render_limit(device)));
        if !new_maps && !new_surface && target_size.is_none() {
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
        let mesh_replaced = session.mesh_graph_refresh.contains(&binding);
        let graphs = material_graph::affected_layers(&session.stack, new_maps || mesh_replaced, new_surface, target_size);
        if graphs.is_empty() {
            session.mesh_graph_refresh.remove(&binding);
            session.graph_bake = stamp;
            session.graph_surfaces.insert(binding, surface);
            if target_size.is_some() {
                session.graph_set_sizes.insert(binding, session.texture_size);
            }
        } else if let Some((device, queue)) = device.as_deref().zip(queue.as_deref()) {
            session.mesh_graph_refresh.remove(&binding);
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
                    let inputs = material_graph::Inputs { maps: maps.as_deref(), surface: Some(&surface), live: None };
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
