//! Camera-local grading shared by the raster and traced paint previews.
use bevy::{
    core_pipeline::schedule::{Core3d, Core3dSystems},
    prelude::*,
    render::{
        RenderApp, RenderStartup,
        extract_component::{ExtractComponent, ExtractComponentPlugin},
        render_resource::*,
        renderer::{RenderContext, RenderDevice, RenderQueue, ViewQuery},
        view::{DebandDither, ViewTarget},
    },
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::editor) enum Field {
    Contrast,
    Saturation,
    Temperature,
    Tint,
    Bloom,
    BloomThreshold,
    Vignette,
    Chromatic,
    Sharpen,
    Grain,
}

impl Field {
    pub const ALL: [(Self, &'static str); 10] = [
        (Self::Contrast, "Контраст"),
        (Self::Saturation, "Насыщенность"),
        (Self::Temperature, "Температура"),
        (Self::Tint, "Оттенок"),
        (Self::Bloom, "Свечение"),
        (Self::BloomThreshold, "Порог свечения"),
        (Self::Vignette, "Виньетка"),
        (Self::Chromatic, "Хроматическая аберрация"),
        (Self::Sharpen, "Резкость"),
        (Self::Grain, "Плёночное зерно"),
    ];

    pub fn range(self) -> (f32, f32) {
        match self {
            Self::Contrast => (0.5, 1.5),
            Self::Saturation => (0.0, 2.0),
            Self::Temperature | Self::Tint => (-1.0, 1.0),
            Self::BloomThreshold => (0.0, 4.0),
            _ => (0.0, 1.0),
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            Self::Contrast => "Изменить различие между светлыми и тёмными участками.",
            Self::Saturation => "Усилить цвет или сделать предпросмотр чёрно-белым.",
            Self::Temperature => "Сместить баланс белого от синего к янтарному.",
            Self::Tint => "Сместить баланс белого от зелёного к пурпурному.",
            Self::Bloom => "Добавить ореолы вокруг ярких участков.",
            Self::BloomThreshold => "Яркость, начиная с которой появляется свечение.",
            Self::Vignette => "Приглушить яркость ближе к краям кадра.",
            Self::Chromatic => "Развести красный и синий каналы у краёв кадра.",
            Self::Sharpen => "Подчеркнуть мелкие детали и границы.",
            Self::Grain => "Добавить плёночное зерно в экранных координатах.",
        }
    }

    fn target(self, effects: &mut Effects) -> &mut f32 {
        match self {
            Self::Contrast => &mut effects.contrast,
            Self::Saturation => &mut effects.saturation,
            Self::Temperature => &mut effects.temperature,
            Self::Tint => &mut effects.tint,
            Self::Bloom => &mut effects.bloom,
            Self::BloomThreshold => &mut effects.bloom_threshold,
            Self::Vignette => &mut effects.vignette,
            Self::Chromatic => &mut effects.chromatic,
            Self::Sharpen => &mut effects.sharpen,
            Self::Grain => &mut effects.grain,
        }
    }

    pub(in crate::editor::ui::model_viewer) fn value(self, mut effects: Effects) -> f32 {
        *self.target(&mut effects)
    }

    pub(in crate::editor::ui::model_viewer) fn set(self, effects: &mut Effects, value: f32) {
        if value.is_finite() {
            let (min, max) = self.range();
            *self.target(effects) = value.clamp(min, max);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(in crate::editor::ui::model_viewer) struct Effects {
    pub curve: Option<crate::ToneMap>,
    contrast: f32,
    saturation: f32,
    temperature: f32,
    tint: f32,
    bloom: f32,
    bloom_threshold: f32,
    vignette: f32,
    chromatic: f32,
    sharpen: f32,
    grain: f32,
}

impl Default for Effects {
    fn default() -> Self {
        Self {
            curve: None,
            contrast: 1.0,
            saturation: 1.0,
            temperature: 0.0,
            tint: 0.0,
            bloom: 0.0,
            bloom_threshold: 1.0,
            vignette: 0.0,
            chromatic: 0.0,
            sharpen: 0.0,
            grain: 0.0,
        }
    }
}

impl Effects {
    pub fn valid(self) -> bool {
        Field::ALL.iter().all(|(field, _)| {
            let (min, max) = field.range();
            (min..=max).contains(&field.value(self))
        })
    }

    pub(super) fn enabled(self) -> bool {
        Self {
            bloom_threshold: 1.0,
            ..self
        } != Self::default()
    }

    pub fn cycle_curve(&mut self) {
        use crate::ToneMap::{AcesFitted, AgX, BlenderFilmic, KhronosPbrNeutral, Linear};
        self.curve = match self.curve {
            None => Some(AcesFitted),
            Some(AcesFitted) => Some(AgX),
            Some(AgX) => Some(KhronosPbrNeutral),
            Some(KhronosPbrNeutral) => Some(BlenderFilmic),
            Some(BlenderFilmic) => Some(Linear),
            _ => None,
        };
    }
}

#[derive(Clone, Copy, PartialEq)]
pub(super) struct Grade {
    pub exposure: f32,
    pub curve: crate::ToneMap,
    pub effects: Effects,
}

#[derive(Clone, Copy, Component, ExtractComponent, PartialEq)]
#[extract_app(RenderApp)]
pub(super) struct RasterGrade(pub Grade, pub Option<DebandDither>);

pub(in crate::editor::ui::model_viewer) struct Plugin;

impl bevy::app::Plugin for Plugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(ExtractComponentPlugin::<RasterGrade>::default());
        if let Some(render) = app.get_sub_app_mut(RenderApp) {
            render.add_systems(RenderStartup, setup);
            render.add_systems(Core3d, raster.in_set(Core3dSystems::EarlyPostProcess));
        }
    }
}

fn setup(mut commands: Commands, device: Res<RenderDevice>, queue: Res<RenderQueue>) {
    commands.insert_resource(Processor::new(&device, &queue));
}

fn raster(
    view: ViewQuery<(&ViewTarget, &RasterGrade)>,
    mut post: ResMut<Processor>,
    device: Res<RenderDevice>,
    mut context: RenderContext,
) {
    let (target, grade) = view.into_inner();
    if target.main_texture_format() != TextureFormat::Rgba16Float {
        return;
    }
    let texture = target.main_texture();
    let size = UVec2::new(texture.width(), texture.height());
    let swap = target.post_process_write();
    post.record(
        &device,
        context.command_encoder(),
        swap.source,
        swap.destination,
        size,
        grade.0,
        false,
    );
}

struct Level {
    size: UVec2,
    view: TextureView,
}

#[derive(Resource)]
pub(super) struct Processor {
    raster: RenderPipeline,
    display: RenderPipeline,
    down: RenderPipeline,
    up: RenderPipeline,
    grading_layout: BindGroupLayout,
    bloom_layout: BindGroupLayout,
    black: TextureView,
    sampler: Sampler,
    levels: Vec<Level>,
    size: UVec2,
}

fn binding(binding: u32, ty: BindingType) -> BindGroupLayoutEntry {
    BindGroupLayoutEntry {
        binding,
        ty,
        visibility: ShaderStages::FRAGMENT,
        count: None,
    }
}

fn uniform(device: &RenderDevice, values: impl IntoIterator<Item = f32>) -> Buffer {
    let bytes: Vec<_> = values.into_iter().flat_map(f32::to_le_bytes).collect();
    device.create_buffer_with_data(&BufferInitDescriptor {
        label: Some("Paint preview settings"),
        contents: &bytes,
        usage: BufferUsages::UNIFORM,
    })
}

impl Processor {
    pub(super) fn new(device: &RenderDevice, queue: &RenderQueue) -> Self {
        let uniform = BindingType::Buffer {
            ty: BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        };
        let texture = BindingType::Texture {
            sample_type: TextureSampleType::Float { filterable: true },
            view_dimension: TextureViewDimension::D2,
            multisampled: false,
        };
        let sampler = BindingType::Sampler(SamplerBindingType::Filtering);
        let grading_layout = device.create_bind_group_layout(
            "Paint preview color",
            &[
                binding(0, uniform),
                binding(1, texture),
                binding(2, texture),
                binding(3, sampler),
            ],
        );
        let bloom_layout = device.create_bind_group_layout(
            "Paint preview bloom",
            &[
                binding(4, uniform),
                binding(5, texture),
                binding(6, sampler),
            ],
        );
        let module = device.create_and_validate_shader_module(ShaderModuleDescriptor {
            label: Some("Paint preview post processing"),
            source: ShaderSource::Wgsl(include_str!("model_paint_post.wgsl").into()),
        });
        let pipeline = |entry, layout: &BindGroupLayout, format, blend| {
            let layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
                label: Some(entry),
                bind_group_layouts: &[Some(layout)],
                immediate_size: 0,
            });
            device.create_render_pipeline(&RawRenderPipelineDescriptor {
                label: Some(entry),
                layout: Some(&layout),
                vertex: RawVertexState {
                    module: &module,
                    entry_point: Some("fullscreen"),
                    buffers: &[],
                    compilation_options: default(),
                },
                fragment: Some(RawFragmentState {
                    module: &module,
                    entry_point: Some(entry),
                    compilation_options: default(),
                    targets: &[Some(ColorTargetState {
                        format,
                        blend,
                        write_mask: ColorWrites::ALL,
                    })],
                }),
                primitive: default(),
                depth_stencil: None,
                multisample: default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let hdr = TextureFormat::Rgba16Float;
        let additive = BlendComponent {
            src_factor: BlendFactor::One,
            dst_factor: BlendFactor::One,
            operation: BlendOperation::Add,
        };
        let black = device
            .create_texture_with_data(
                queue,
                &TextureDescriptor {
                    label: Some("No preview bloom"),
                    size: Extent3d {
                        width: 1,
                        height: 1,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: TextureDimension::D2,
                    format: hdr,
                    usage: TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                },
                TextureDataOrder::LayerMajor,
                &[0; 8],
            )
            .create_view(&default());
        Self {
            raster: pipeline("grade", &grading_layout, hdr, None),
            display: pipeline(
                "grade",
                &grading_layout,
                TextureFormat::Rgba8UnormSrgb,
                None,
            ),
            down: pipeline("down", &bloom_layout, hdr, None),
            up: pipeline(
                "up",
                &bloom_layout,
                hdr,
                Some(BlendState {
                    color: additive,
                    alpha: additive,
                }),
            ),
            grading_layout,
            bloom_layout,
            black,
            sampler: device.create_sampler(&SamplerDescriptor {
                mag_filter: FilterMode::Linear,
                min_filter: FilterMode::Linear,
                ..default()
            }),
            levels: Vec::new(),
            size: UVec2::ZERO,
        }
    }

    fn prepare_levels(&mut self, device: &RenderDevice, size: UVec2) {
        if self.size == size {
            return;
        }
        self.size = size;
        self.levels.clear();
        let mut level = size / 2;
        while self.levels.len() < 6 && level.min_element() >= 8 {
            let texture = device.create_texture(&TextureDescriptor {
                label: Some("Paint preview bloom mip"),
                size: Extent3d {
                    width: level.x,
                    height: level.y,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format: TextureFormat::Rgba16Float,
                usage: TextureUsages::TEXTURE_BINDING | TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            });
            self.levels.push(Level {
                size: level,
                view: texture.create_view(&default()),
            });
            level /= 2;
        }
    }

    pub(super) fn record(
        &mut self,
        device: &RenderDevice,
        encoder: &mut CommandEncoder,
        hdr: &TextureView,
        destination: &TextureView,
        size: UVec2,
        grade: Grade,
        srgb: bool,
    ) {
        let effects = grade.effects;
        let (bloom, weight) = if effects.bloom > 0.0 {
            self.prepare_levels(device, size);
            for (i, level) in self.levels.iter().enumerate() {
                let (input, input_size) = if i == 0 {
                    (hdr, size)
                } else {
                    (&self.levels[i - 1].view, self.levels[i - 1].size)
                };
                self.bloom(
                    device,
                    encoder,
                    input,
                    input_size,
                    &level.view,
                    effects.bloom_threshold,
                    i == 0,
                    false,
                );
            }
            for i in (1..self.levels.len()).rev() {
                let source = &self.levels[i];
                self.bloom(
                    device,
                    encoder,
                    &source.view,
                    source.size,
                    &self.levels[i - 1].view,
                    0.0,
                    false,
                    true,
                );
            }
            self.levels.first().map_or((&self.black, 0.0), |level| {
                (&level.view, effects.bloom / self.levels.len() as f32)
            })
        } else {
            (&self.black, 0.0)
        };
        let curve = match effects.curve.unwrap_or(grade.curve) {
            crate::ToneMap::None | crate::ToneMap::Linear => 0.0,
            crate::ToneMap::Reinhard => 1.0,
            crate::ToneMap::ReinhardLuminance => 2.0,
            crate::ToneMap::AgX => 3.0,
            crate::ToneMap::KhronosPbrNeutral => 4.0,
            crate::ToneMap::BlenderFilmic => 6.0,
            _ => 5.0,
        };
        let texel = size.as_vec2().recip();
        let uniform = uniform(
            device,
            [
                grade.exposure,
                curve,
                effects.contrast,
                effects.saturation,
                effects.temperature,
                effects.tint,
                effects.vignette,
                weight,
                effects.chromatic,
                effects.grain,
                effects.sharpen,
                0.0,
                texel.x,
                texel.y,
                size.x as f32 / size.y as f32,
                0.0,
            ],
        );
        let group = device.create_bind_group(
            "Paint preview grade",
            &self.grading_layout,
            &[
                BindGroupEntry {
                    binding: 0,
                    resource: uniform.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: BindingResource::TextureView(hdr),
                },
                BindGroupEntry {
                    binding: 2,
                    resource: BindingResource::TextureView(bloom),
                },
                BindGroupEntry {
                    binding: 3,
                    resource: BindingResource::Sampler(&self.sampler),
                },
            ],
        );
        let pipeline = if srgb { &self.display } else { &self.raster };
        draw(encoder, pipeline, &group, destination, false);
    }

    fn bloom(
        &self,
        device: &RenderDevice,
        encoder: &mut CommandEncoder,
        input: &TextureView,
        size: UVec2,
        output: &TextureView,
        threshold: f32,
        first: bool,
        up: bool,
    ) {
        let texel = size.as_vec2().recip();
        let values = uniform(
            device,
            [texel.x, texel.y, threshold, u32::from(first) as f32],
        );
        let group = device.create_bind_group(
            "Paint bloom pass",
            &self.bloom_layout,
            &[
                BindGroupEntry {
                    binding: 4,
                    resource: values.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 5,
                    resource: BindingResource::TextureView(input),
                },
                BindGroupEntry {
                    binding: 6,
                    resource: BindingResource::Sampler(&self.sampler),
                },
            ],
        );
        let pipeline = if up { &self.up } else { &self.down };
        draw(encoder, pipeline, &group, output, up);
    }
}

fn draw(
    encoder: &mut CommandEncoder,
    pipeline: &RenderPipeline,
    group: &BindGroup,
    output: &TextureView,
    preserve: bool,
) {
    let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
        label: Some("Paint post processing"),
        color_attachments: &[Some(RenderPassColorAttachment {
            view: output,
            depth_slice: None,
            resolve_target: None,
            ops: Operations {
                load: if preserve {
                    LoadOp::Load
                } else {
                    LoadOp::Clear(default())
                },
                store: StoreOp::Store,
            },
        })],
        ..default()
    });
    pass.set_pipeline(pipeline);
    pass.set_bind_group(0, group, &[]);
    pass.draw(0..3, 0..1);
}

#[cfg(test)]
#[path = "model_paint_post_tests.rs"]
mod tests;
