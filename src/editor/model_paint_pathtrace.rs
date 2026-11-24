//! Progressive, scene-linear compute preview for the paint workspace.
use bevy::{
    prelude::*,
    render::{
        render_resource::*,
        renderer::{RenderDevice, RenderQueue},
    },
};

use super::editing::input::{Bvh, PickTriangle};
use crate::editor::model_document::Object;

const WORKGROUP: u32 = 8;
const CDF_WIDTH: usize = 512;
const CDF_HEIGHT: usize = 256;
const PANORAMA_WIDTH: usize = CDF_WIDTH * 2;
const PANORAMA_HEIGHT: usize = CDF_HEIGHT * 2;
const SAMPLE_BYTE_OFFSET: usize = (16 + 4 + 2) * size_of::<f32>();

fn encode(values: impl IntoIterator<Item = f32>) -> Vec<u8> {
    values.into_iter().flat_map(f32::to_le_bytes).collect()
}

fn storage(device: &RenderDevice, label: &'static str, contents: &[u8]) -> Buffer {
    device.create_buffer_with_data(&BufferInitDescriptor {
        label: Some(label),
        contents,
        usage: BufferUsages::STORAGE,
    })
}

/// GPU geometry in traversal order. The caller assigns array layers to material
/// slots and can exclude hidden texture sets.
pub(super) struct Scene {
    buffers: [Buffer; 5],
    ground_y: f32,
}

impl Scene {
    pub(super) fn new(
        device: &RenderDevice,
        objects: &[Object],
        layer_for_slot: impl Fn(usize, usize) -> Option<u32>,
    ) -> Option<Self> {
        let mut triangles = Vec::new();
        let mut shading = Vec::new();
        for object in objects.iter().filter(|object| !object.hidden) {
            let transform = object.transform.to_matrix();
            let normal_transform = transform.inverse().transpose();
            for (slot, mut mesh) in object.geometry.buffers() {
                let Some(layer) = layer_for_slot(object.id, slot) else {
                    continue;
                };
                mesh.localize_uv(object.uv_offset(slot));
                for indices in mesh.indices.chunks_exact(3) {
                    let ids = [indices[0], indices[1], indices[2]].map(|index| index as usize);
                    let points = ids.map(|index| {
                        transform.transform_point3(Vec3::from_array(mesh.positions[index]))
                    });
                    let edge_u = points[1] - points[0];
                    let edge_v = points[2] - points[0];
                    let Some(face_normal) = edge_u.cross(edge_v).try_normalize() else {
                        continue;
                    };
                    let normals = ids.map(|index| {
                        normal_transform
                            .transform_vector3(Vec3::from_array(mesh.normals[index]))
                            .try_normalize()
                            .unwrap_or(face_normal)
                    });
                    let uv = ids.map(|index| Vec2::from_array(mesh.uv[index]));
                    let tangents = tangents(edge_u, edge_v, uv, normals);
                    triangles.push(PickTriangle {
                        object: object.id,
                        face: 0,
                        points,
                    });
                    shading.push([
                        normals.map(|normal| normal.extend(0.0)),
                        uv.map(|uv| uv.extend(layer as f32).extend(0.0)),
                        tangents,
                    ]);
                }
            }
        }
        let ground_y = triangles
            .iter()
            .flat_map(|triangle| triangle.points)
            .map(|point| point.y)
            .reduce(f32::min)?;
        let bvh = Bvh::from_triangles(triangles);
        let nodes = bvh.compute_nodes();
        let triangles = bvh.compute_triangles();
        let limit = device.limits();
        let max_bytes = (limit.max_storage_buffer_binding_size as u64).min(limit.max_buffer_size);
        if nodes.len() as u64 * 48 > max_bytes || triangles.len() as u64 * 48 > max_bytes {
            return None;
        }
        let mut hierarchy = Vec::with_capacity(nodes.len() * 48);
        for ([min, max], links) in nodes {
            for value in min
                .extend(0.0)
                .to_array()
                .into_iter()
                .chain(max.extend(0.0).to_array())
            {
                hierarchy.extend_from_slice(&value.to_le_bytes());
            }
            for link in links {
                hierarchy.extend_from_slice(&link.to_le_bytes());
            }
        }
        let mut attributes: [Vec<u8>; 4] =
            std::array::from_fn(|_| Vec::with_capacity(triangles.len() * 48));
        for (index, triangle) in triangles {
            let [normals, uv, tangent] = shading[index];
            let points = triangle.points.map(|point| point.extend(0.0));
            for (bytes, attribute) in attributes.iter_mut().zip([points, normals, uv, tangent]) {
                for value in attribute.into_iter().flat_map(|vector| vector.to_array()) {
                    bytes.extend_from_slice(&value.to_le_bytes());
                }
            }
        }
        let [positions, normals, uv, tangents] = attributes;
        Some(Self {
            buffers: [
                storage(device, "Path trace BVH", &hierarchy),
                storage(device, "Path trace positions", &positions),
                storage(device, "Path trace normals", &normals),
                storage(device, "Path trace UVs", &uv),
                storage(device, "Path trace tangents", &tangents),
            ],
            ground_y,
        })
    }
}

fn tangents(a: Vec3, b: Vec3, uv: [Vec2; 3], normals: [Vec3; 3]) -> [Vec4; 3] {
    let u = uv[1] - uv[0];
    let v = uv[2] - uv[0];
    let det = u.perp_dot(v);
    if det.abs() <= 1e-12 {
        return normals.map(|normal| normal.any_orthonormal_vector().extend(-1.0));
    }
    let du = (a * v.y - b * u.y) / det;
    let dv = (b * u.x - a * v.x) / det;
    normals.map(|normal| {
        let tangent = (du - normal * du.dot(normal))
            .try_normalize()
            .unwrap_or_else(|| normal.any_orthonormal_vector());
        // The shader negates the cross product, so encode the opposite of
        // the editor's bitangent sign.
        let sign = if normal.cross(tangent).dot(dv) < 0.0 {
            1.0
        } else {
            -1.0
        };
        tangent.extend(sign)
    })
}

/// Resident editor HDRI and a distribution derived from its sharpest cube mip.
pub(super) struct Environment {
    pub view: TextureView,
    cdf: Buffer,
    pub max_lod: f32,
}

impl Environment {
    pub(super) fn from_cubemap(
        device: &RenderDevice,
        image: &Image,
        view: TextureView,
    ) -> Option<Self> {
        let radiance = project_cubemap(image)?;
        Some(Self {
            view,
            cdf: storage(
                device,
                "Path trace environment distribution",
                &encode(environment_distribution(
                    PANORAMA_WIDTH,
                    PANORAMA_HEIGHT,
                    &radiance,
                )),
            ),
            max_lod: image.texture_descriptor.mip_level_count.saturating_sub(1) as f32,
        })
    }
}

fn project_cubemap(image: &Image) -> Option<Vec<Vec3>> {
    let descriptor = &image.texture_descriptor;
    let side = descriptor.size.width as usize;
    if side == 0
        || descriptor.size.height as usize != side
        || descriptor.size.depth_or_array_layers != 6
        || descriptor.format != TextureFormat::Rgba16Float
    {
        return None;
    }
    let data = image.data.as_deref()?;
    let mip_bytes = |level: u32| {
        let side = (side >> level).max(1);
        side * side * 8
    };
    let stride = match image.data_order {
        TextureDataOrder::LayerMajor => (0..descriptor.mip_level_count).map(mip_bytes).sum(),
        TextureDataOrder::MipMajor => mip_bytes(0),
    };
    if data.len() < 5 * stride + mip_bytes(0) {
        return None;
    }
    let face_pixels = side * side;
    let faces: Vec<_> = (0..6)
        .flat_map(|face| {
            data[face * stride..face * stride + mip_bytes(0)]
                .chunks_exact(8)
                .map(|texel| {
                    Vec3::from_array(std::array::from_fn(|i| {
                        half::f16::from_le_bytes([texel[i * 2], texel[i * 2 + 1]]).to_f32()
                    }))
                })
        })
        .collect();
    let longitude: Vec<_> = (0..PANORAMA_WIDTH)
        .map(|x| {
            let phi = ((x as f32 + 0.5) / PANORAMA_WIDTH as f32 - 0.5) * std::f32::consts::TAU;
            phi.sin_cos()
        })
        .collect();
    let edge = side - 1;
    let mut pixels = Vec::with_capacity(PANORAMA_WIDTH * PANORAMA_HEIGHT);
    // Sample both axes above the CDF resolution. A sun disk can fall between
    // the centers of the CDF cells even when it covers several cube texels.
    for y in 0..PANORAMA_HEIGHT {
        let theta = (y as f32 + 0.5) / PANORAMA_HEIGHT as f32 * std::f32::consts::PI;
        let (sin_theta, cos_theta) = theta.sin_cos();
        for &(sin_phi, cos_phi) in &longitude {
            let dir = Vec3::new(sin_theta * sin_phi, cos_theta, -sin_theta * cos_phi);
            let (face, uv) = face_coordinates(dir);
            let pixel = ((uv * 0.5 + Vec2::splat(0.5)) * side as f32 - Vec2::splat(0.5))
                .clamp(Vec2::ZERO, Vec2::splat(edge as f32));
            let x = pixel.x as usize;
            let y = pixel.y as usize;
            let next_x = (x + 1).min(edge);
            let next_y = (y + 1).min(edge);
            let fract = pixel.fract();
            let offset = face * face_pixels;
            let upper =
                faces[offset + y * side + x].lerp(faces[offset + y * side + next_x], fract.x);
            let lower = faces[offset + next_y * side + x]
                .lerp(faces[offset + next_y * side + next_x], fract.x);
            pixels.push(upper.lerp(lower, fract.y));
        }
    }
    Some(pixels)
}

fn face_coordinates(direction: Vec3) -> (usize, Vec2) {
    let Vec3 { x, y, z } = direction;
    let magnitude = direction.abs();
    let (face, uv) = if magnitude.x >= magnitude.y.max(magnitude.z) {
        if x > 0.0 {
            (0, Vec2::new(-z, -y))
        } else {
            (1, Vec2::new(z, -y))
        }
    } else if magnitude.y >= magnitude.z {
        if y > 0.0 {
            (2, Vec2::new(x, z))
        } else {
            (3, Vec2::new(x, -z))
        }
    } else if z > 0.0 {
        (4, Vec2::new(x, -y))
    } else {
        (5, Vec2::new(-x, -y))
    };
    (face, uv / magnitude.max_element())
}

fn environment_distribution(width: usize, height: usize, radiance: &[Vec3]) -> Vec<f32> {
    let (w, h) = (CDF_WIDTH, CDF_HEIGHT);
    let luminance = Vec3::new(0.2126, 0.7152, 0.0722);
    let mut density = Vec::with_capacity(w * h);
    let mut rows = Vec::with_capacity(h * (w + 1));
    let mut marginal = Vec::with_capacity(h + 1);
    marginal.push(0.0);
    for y in 0..h {
        let start_y = y * height / h;
        let end_y = ((y + 1) * height / h).max(start_y + 1);
        let theta = (y as f32 + 0.5) * std::f32::consts::PI / h as f32;
        let offset = rows.len();
        let mut sum = 0.0;
        rows.push(0.0);
        for x in 0..w {
            let start_x = x * width / w;
            let end_x = ((x + 1) * width / w).max(start_x + 1);
            let mut light = 0.0;
            for row in start_y..end_y {
                light += radiance[row * width + start_x..row * width + end_x]
                    .iter()
                    .map(|color| color.dot(luminance).clamp(0.0, 65504.0))
                    .filter(|value| value.is_finite())
                    .sum::<f32>();
            }
            let mean = light / ((end_x - start_x) * (end_y - start_y)) as f32;
            let weight = (mean + 0.001) * theta.sin();
            density.push(weight);
            sum += weight;
            rows.push(sum);
        }
        for value in &mut rows[offset..] {
            *value /= sum;
        }
        marginal.push(marginal[y] + sum / w as f32);
    }
    let integral = marginal[h];
    let total = integral / h as f32;
    std::iter::once(total)
        .chain(marginal.into_iter().map(|value| value / integral))
        .chain(rows)
        .chain(density)
        .collect()
}

pub(super) struct MaterialMaps<'a> {
    pub color: &'a TextureView,
    pub roughness: &'a TextureView,
    pub metallic: &'a TextureView,
    pub height: &'a TextureView,
    pub baked_normal: &'a TextureView,
    pub emission: &'a TextureView,
    pub normal: &'a TextureView,
    pub specular: &'a TextureView,
    pub resolution: u32,
}

#[derive(Clone, Copy)]
pub(super) struct Camera {
    pub inverse_view_projection: Mat4,
    pub eye: Vec3,
    pub orthographic: bool,
}

#[derive(Clone, Copy)]
pub(in crate::editor::ui::model_viewer) struct Lighting {
    pub environment_rotation: f32,
    pub environment_intensity: f32,
    pub background: Option<Vec3>,
    pub background_blur: f32,
    pub background_intensity: f32,
    pub key_direction: Vec3,
    pub key_color: Vec3,
    pub key_intensity: f32,
}

#[derive(Clone, Copy)]
pub(super) struct Settings {
    pub samples: u32,
    pub bounces: u32,
    pub ground: bool,
    pub baked_normal: bool,
    pub height_strength: f32,
    pub emission_strength: f32,
    pub precomposed: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            samples: 512,
            bounces: 4,
            ground: false,
            baked_normal: true,
            height_strength: 1.0,
            emission_strength: 1.0,
            precomposed: false,
        }
    }
}

pub(super) struct Pipeline {
    compute: ComputePipeline,
    layout: BindGroupLayout,
    material_sampler: Sampler,
    environment_sampler: Sampler,
}

impl Pipeline {
    pub(super) fn new(device: &RenderDevice) -> Option<Self> {
        let limits = device.limits();
        if limits.max_storage_buffers_per_shader_stage < 7
            || limits.max_sampled_textures_per_shader_stage < 9
            || limits.max_storage_textures_per_shader_stage == 0
            || limits.max_compute_invocations_per_workgroup < WORKGROUP * WORKGROUP
            || limits.max_compute_workgroup_size_x < WORKGROUP
            || limits.max_compute_workgroup_size_y < WORKGROUP
        {
            return None;
        }
        let compute = device.create_compute_pipeline(&RawComputePipelineDescriptor {
            label: Some("Model paint path trace"),
            layout: None,
            module: &device.create_and_validate_shader_module(ShaderModuleDescriptor {
                label: Some("Model paint path trace"),
                source: ShaderSource::Wgsl(include_str!("model_paint_pathtrace.wgsl").into()),
            }),
            entry_point: Some("cs_trace"),
            compilation_options: default(),
            cache: None,
        });
        let layout = BindGroupLayout::from(compute.get_bind_group_layout(0));
        let sampler = SamplerDescriptor {
            mag_filter: FilterMode::Linear,
            min_filter: FilterMode::Linear,
            mipmap_filter: MipmapFilterMode::Linear,
            ..default()
        };
        Some(Self {
            compute,
            layout,
            material_sampler: device.create_sampler(&sampler),
            environment_sampler: device.create_sampler(&sampler),
        })
    }
}

pub(super) struct Target {
    pub output: Texture,
    pub view: TextureView,
    pub size: UVec2,
    accumulation: Buffer,
    uniform: Buffer,
    samples: u32,
    key: Option<(u64, Vec<u8>)>,
}

/// Revisions must change when geometry, environment or textures change in place.
pub(super) struct Frame<'a> {
    pub scene: &'a Scene,
    pub maps: MaterialMaps<'a>,
    pub environment: &'a Environment,
    pub camera: Camera,
    pub lighting: Lighting,
    pub settings: Settings,
    pub revision: u64,
}

impl Target {
    pub(super) fn new(device: &RenderDevice, size: UVec2) -> Option<Self> {
        let limit = device.limits();
        let pixels = u64::from(size.x) * u64::from(size.y);
        let bytes = pixels.checked_mul(16)?;
        if size.min_element() == 0
            || size.max_element() > limit.max_texture_dimension_2d
            || size.max_element().div_ceil(WORKGROUP) > limit.max_compute_workgroups_per_dimension
            || bytes > (limit.max_storage_buffer_binding_size as u64).min(limit.max_buffer_size)
        {
            return None;
        }
        let output = device.create_texture(&TextureDescriptor {
            label: Some("Path trace HDR preview"),
            size: Extent3d {
                width: size.x,
                height: size.y,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::Rgba16Float,
            usage: TextureUsages::STORAGE_BINDING
                | TextureUsages::TEXTURE_BINDING
                | TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = output.create_view(&default());
        Some(Self {
            output,
            view,
            size,
            accumulation: device.create_buffer(&BufferDescriptor {
                label: Some("Path trace accumulation"),
                size: bytes,
                usage: BufferUsages::STORAGE,
                mapped_at_creation: false,
            }),
            uniform: device.create_buffer(&BufferDescriptor {
                label: Some("Path trace uniforms"),
                size: (16 + 8 * 4) * 4,
                usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            samples: 0,
            key: None,
        })
    }

    pub(super) fn samples(&self) -> u32 {
        self.samples
    }

    pub(super) fn step(
        &mut self,
        device: &RenderDevice,
        queue: &RenderQueue,
        pipeline: &Pipeline,
        frame: &Frame,
    ) -> bool {
        let camera = frame.camera;
        let settings = frame.settings;
        let light = frame.lighting;
        let res = frame.maps.resolution.max(1) as f32;
        let mut uniforms = encode(
            camera
                .inverse_view_projection
                .to_cols_array()
                .into_iter()
                .chain(
                    [
                        camera
                            .eye
                            .extend(u32::from(camera.orthographic) as f32)
                            .to_array(),
                        [
                            self.size.x as f32,
                            self.size.y as f32,
                            0.0,
                            settings.bounces.clamp(1, 16) as f32,
                        ],
                        [
                            light.environment_rotation,
                            settings.emission_strength,
                            frame.scene.ground_y,
                            u32::from(settings.ground) as f32,
                        ],
                        [
                            if settings.precomposed {
                                2.0
                            } else {
                                u32::from(settings.baked_normal) as f32
                            },
                            settings.height_strength,
                            1.0 / res,
                            res,
                        ],
                        [
                            light.environment_intensity,
                            frame.environment.max_lod,
                            light.background_blur,
                            u32::from(light.background.is_some()) as f32,
                        ],
                        light
                            .background
                            .unwrap_or_default()
                            .extend(light.background_intensity)
                            .to_array(),
                        light
                            .key_direction
                            .normalize_or_zero()
                            .extend(light.key_intensity)
                            .to_array(),
                        light
                            .key_color
                            .extend(u32::from(light.key_intensity > 0.0) as f32)
                            .to_array(),
                    ]
                    .into_iter()
                    .flatten(),
                ),
        );
        if !self
            .key
            .as_ref()
            .is_some_and(|(revision, key)| *revision == frame.revision && *key == uniforms)
        {
            self.samples = 0;
            self.key = Some((frame.revision, uniforms.clone()));
        }
        if self.samples >= settings.samples.clamp(1, 16_384) {
            return false;
        }
        uniforms[SAMPLE_BYTE_OFFSET..SAMPLE_BYTE_OFFSET + size_of::<f32>()]
            .copy_from_slice(&(self.samples as f32).to_le_bytes());
        queue.write_buffer(&self.uniform, 0, &uniforms);
        let maps = &frame.maps;
        let [nodes, positions, normals, uv, tangents] = &frame.scene.buffers;
        let resources = [
            self.uniform.as_entire_binding(),
            self.accumulation.as_entire_binding(),
            BindingResource::TextureView(&self.view),
            nodes.as_entire_binding(),
            positions.as_entire_binding(),
            normals.as_entire_binding(),
            uv.as_entire_binding(),
            tangents.as_entire_binding(),
            BindingResource::TextureView(maps.color),
            BindingResource::TextureView(maps.roughness),
            BindingResource::TextureView(maps.metallic),
            BindingResource::TextureView(maps.height),
            BindingResource::TextureView(maps.baked_normal),
            BindingResource::Sampler(&pipeline.material_sampler),
            BindingResource::TextureView(&frame.environment.view),
            BindingResource::Sampler(&pipeline.environment_sampler),
            frame.environment.cdf.as_entire_binding(),
            BindingResource::TextureView(maps.emission),
            BindingResource::TextureView(maps.normal),
            BindingResource::TextureView(maps.specular),
        ];
        let entries = std::array::from_fn::<_, 20, _>(|binding| BindGroupEntry {
            binding: binding as u32,
            resource: resources[binding].clone(),
        });
        let group = device.create_bind_group("Path trace frame", &pipeline.layout, &entries);
        let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor {
            label: Some("Paint path trace"),
        });
        {
            let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor::default());
            pass.set_pipeline(&pipeline.compute);
            pass.set_bind_group(0, &group, &[]);
            pass.dispatch_workgroups(
                self.size.x.div_ceil(WORKGROUP),
                self.size.y.div_ceil(WORKGROUP),
                1,
            );
        }
        queue.submit([encoder.finish()]);
        self.samples += 1;
        true
    }
}

#[path = "model_paint_pathtrace_view.rs"]
pub(in crate::editor::ui::model_viewer) mod viewport;

#[cfg(test)]
#[path = "model_paint_pathtrace_tests.rs"]
mod tests;
