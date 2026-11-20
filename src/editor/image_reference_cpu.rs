//! Software fallback for a reference camera that cannot produce pixels.
use bevy::{
    math::Affine3A,
    mesh::{PrimitiveTopology, VertexAttributeValues},
    prelude::*,
};
use photocraft_color::convert::{linear_to_srgb, srgb_to_linear};

const SIDE: u32 = 512;

#[derive(bevy::ecs::system::SystemParam)]
pub(in crate::editor) struct Geometry<'w, 's> {
    instances: Query<
        'w,
        's,
        (
            &'static Mesh3d,
            Option<&'static MeshMaterial3d<StandardMaterial>>,
            &'static GlobalTransform,
            &'static InheritedVisibility,
        ),
    >,
    meshes: Res<'w, Assets<Mesh>>,
    materials: Res<'w, Assets<StandardMaterial>>,
}

pub(super) struct Part {
    mesh: Mesh,
    transform: Affine3A,
    material: StandardMaterial,
    texture: Option<Image>,
}

impl Geometry<'_, '_> {
    pub(super) fn snapshot(
        &self,
        descendants: impl Iterator<Item = Entity>,
        images: &Assets<Image>,
    ) -> Vec<Part> {
        descendants
            .filter_map(|entity| {
                let (handle, material, transform, visibility) = self.instances.get(entity).ok()?;
                if !visibility.get() {
                    return None;
                }
                let mesh = self.meshes.get(&handle.0)?;
                let material = material.and_then(|handle| self.materials.get(&handle.0));
                Some(Part {
                    mesh: mesh.clone(),
                    transform: transform.affine(),
                    material: material.cloned().unwrap_or_default(),
                    texture: material
                        .and_then(|material| material.base_color_texture.as_ref())
                        .and_then(|handle| images.get(handle))
                        .cloned(),
                })
            })
            .collect()
    }
}

pub(super) fn render(
    parts: Vec<Part>,
    view: Transform,
    projection: Projection,
) -> image::RgbaImage {
    let view_projection = projection.get_clip_from_view() * view.to_matrix().inverse();
    let light = (*view.back() * 0.55 - *view.right() * 0.55 + *view.up()).normalize();
    let mut image = image::RgbaImage::new(SIDE, SIDE);
    let mut depth = vec![f32::NEG_INFINITY; (SIDE * SIDE) as usize];
    for part in parts {
        let Ok(attribute) = part.mesh.try_attribute(Mesh::ATTRIBUTE_POSITION) else {
            continue;
        };
        let Some(positions) = attribute.as_float3() else {
            continue;
        };
        let uv_attribute = if part.material.base_color_channel == bevy::mesh::UvChannel::Uv1 {
            Mesh::ATTRIBUTE_UV_1
        } else {
            Mesh::ATTRIBUTE_UV_0
        };
        let uvs = match part.mesh.try_attribute(uv_attribute) {
            Ok(VertexAttributeValues::Float32x2(values)) => values.as_slice(),
            _ => &[],
        };
        let vertices: Vec<_> = positions
            .iter()
            .enumerate()
            .map(|(index, &position)| {
                let world = part.transform.transform_point3(Vec3::from_array(position));
                let clip = view_projection * world.extend(1.0);
                let ndc = clip.truncate() / clip.w;
                Vertex {
                    world,
                    screen: (ndc.truncate() * Vec2::new(0.5, -0.5) + Vec2::splat(0.5))
                        * SIDE as f32,
                    depth: ndc.z,
                    reciprocal_w: clip.w.recip(),
                    uv: uvs.get(index).copied().map_or(Vec2::ZERO, Vec2::from_array),
                }
            })
            .collect();
        let indices: Vec<_> = match part.mesh.try_indices_option() {
            Ok(Some(indices)) => indices.iter().collect(),
            Ok(None) => (0..positions.len()).collect(),
            Err(_) => continue,
        };
        let texture = part
            .texture
            .and_then(|texture| texture.try_into_dynamic().ok())
            .map(|texture| texture.into_rgba8());
        let mut draw = |indices: [usize; 3]| {
            let [Some(a), Some(b), Some(c)] = indices.map(|i| vertices.get(i)) else {
                return;
            };
            let normal = (b.world - a.world)
                .cross(c.world - a.world)
                .normalize_or_zero();
            let facing = normal.dot(view.translation - a.world);
            if (part.material.cull_mode == Some(bevy::render::render_resource::Face::Back)
                && facing < 0.0)
                || (part.material.cull_mode == Some(bevy::render::render_resource::Face::Front)
                    && facing > 0.0)
            {
                return;
            }
            let normal = if facing < 0.0 { -normal } else { normal };
            let shade = if part.material.unlit {
                1.0
            } else {
                0.36 + 0.64 * normal.dot(light).max(0.0)
            };
            let paint = Paint {
                material: &part.material,
                texture: texture.as_ref(),
                shade,
            };
            triangle([a, b, c], paint, &mut image, &mut depth);
        };
        match part.mesh.primitive_topology() {
            PrimitiveTopology::TriangleList => {
                for &face in indices.as_chunks::<3>().0 {
                    draw(face);
                }
            }
            PrimitiveTopology::TriangleStrip => {
                for (index, &[a, b, c]) in indices.array_windows().enumerate() {
                    draw(if index % 2 == 0 { [a, b, c] } else { [b, a, c] });
                }
            }
            _ => {}
        }
    }
    image
}

struct Vertex {
    world: Vec3,
    screen: Vec2,
    depth: f32,
    reciprocal_w: f32,
    uv: Vec2,
}

struct Paint<'a> {
    material: &'a StandardMaterial,
    texture: Option<&'a image::RgbaImage>,
    shade: f32,
}

impl Paint<'_> {
    fn sample(&self, uv: Vec2) -> Option<image::Rgba<u8>> {
        let base = self.material.base_color.to_linear();
        let color = self.texture.map_or([1.0; 4], |texture| {
            let x = (uv.x.rem_euclid(1.0) * texture.width() as f32) as u32;
            let y = (uv.y.rem_euclid(1.0) * texture.height() as f32) as u32;
            let [r, g, b, a] = texture
                .get_pixel(x.min(texture.width() - 1), y.min(texture.height() - 1))
                .0
                .map(|value| value as f32 / 255.0);
            [srgb_to_linear(r), srgb_to_linear(g), srgb_to_linear(b), a]
        });
        let alpha = base.alpha * color[3];
        let coverage = match self.material.alpha_mode {
            AlphaMode::Opaque => 1.0,
            AlphaMode::Mask(threshold) if alpha < threshold => return None,
            AlphaMode::Mask(_) => 1.0,
            _ => alpha,
        };
        let emission = self.material.emissive;
        let [r, g, b] = [
            base.red * color[0] * self.shade + emission.red,
            base.green * color[1] * self.shade + emission.green,
            base.blue * color[2] * self.shade + emission.blue,
        ]
        .map(|channel| (linear_to_srgb(channel.clamp(0.0, 1.0)) * 255.0).round() as u8);
        (coverage > 0.0).then_some(image::Rgba([r, g, b, (coverage * 255.0).round() as u8]))
    }
}

fn triangle(vertices: [&Vertex; 3], paint: Paint, image: &mut image::RgbaImage, depth: &mut [f32]) {
    if vertices
        .iter()
        .any(|v| !v.screen.is_finite() || v.reciprocal_w <= 0.0)
    {
        return;
    }
    let [a, b, c] = vertices.map(|v| v.screen);
    let area = (b - a).perp_dot(c - a);
    if area.abs() < 1.0e-6 {
        return;
    }
    let min = a.min(b).min(c).floor().max(Vec2::ZERO).as_uvec2();
    let max = a
        .max(b)
        .max(c)
        .ceil()
        .clamp(Vec2::ZERO, Vec2::splat(SIDE as f32))
        .as_uvec2();
    for y in min.y..max.y {
        for x in min.x..max.x {
            let delta = Vec2::new(x as f32 + 0.5, y as f32 + 0.5) - a;
            let wb = delta.perp_dot(c - a) / area;
            let wc = (b - a).perp_dot(delta) / area;
            let wa = 1.0 - wb - wc;
            if wa < 0.0 || wb < 0.0 || wc < 0.0 {
                continue;
            }
            let weights = Vec3::new(wa, wb, wc);
            let z = weights.dot(Vec3::from_array(vertices.map(|v| v.depth)));
            let index = (y * SIDE + x) as usize;
            if z <= depth[index] {
                continue;
            }
            let uv_weights = weights * Vec3::from_array(vertices.map(|v| v.reciprocal_w));
            let uv = (vertices[0].uv * uv_weights.x
                + vertices[1].uv * uv_weights.y
                + vertices[2].uv * uv_weights.z)
                / uv_weights.element_sum();
            if let Some(pixel) = paint.sample(uv) {
                image.put_pixel(x, y, pixel);
                depth[index] = z;
            }
        }
    }
}
