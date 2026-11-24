//! Lit swatches use the same fill projection, baked inputs and compositing as the workspace.
use super::engine::projection;
use super::*;
use crate::editor::{
    model_document::{MeshData, Object},
    model_mesh::Geometry,
    model_paint_data::{CHANNEL_COUNT, mask::BakedInputs, solid},
    model_primitives::Primitive,
};
use image::{Rgba, RgbaImage};
use std::sync::OnceLock;

pub(super) const SIZE: u32 = 128;
const FRAME: u32 = 144;

struct Sample {
    mesh: MeshData,
    baked: Arc<BakedInputs>,
    pixels: Vec<Option<Fragment>>,
    view: Vec3,
}

struct Fragment {
    depth: f32,
    uv: Vec2,
    normal: Vec3,
    tangent: Vec3,
    handedness: f32,
}

fn sample() -> Result<&'static Sample, String> {
    static SAMPLE: OnceLock<Result<Sample, String>> = OnceLock::new();
    SAMPLE
        .get_or_init(build_sample)
        .as_ref()
        .map_err(Clone::clone)
}

fn build_sample() -> Result<Sample, String> {
    let mut mesh = Geometry::default();
    for (primitive, position, rotation, uv_origin, uv_extent) in [
        (
            Primitive::UvSphere,
            Vec3::Y * 0.12,
            Quat::IDENTITY,
            Vec2::ZERO,
            Vec2::splat(0.69),
        ),
        (
            Primitive::Torus,
            Vec3::Y * 0.12,
            Quat::from_rotation_z(0.33),
            Vec2::new(0.02, 0.73),
            Vec2::new(0.94, 0.22),
        ),
        (
            Primitive::Cylinder,
            Vec3::NEG_Y * 0.63,
            Quat::IDENTITY,
            Vec2::new(0.73, 0.02),
            Vec2::new(0.24, 0.67),
        ),
    ] {
        let mut parameters = primitive.defaults();
        parameters.radius = 0.61;
        parameters.major_radius = 0.605;
        parameters.minor_radius = 0.055;
        parameters.size.y = 0.22;
        let piece = primitive.geometry(&parameters)?;
        let offset = mesh.positions.len();
        mesh.positions
            .extend(piece.positions.into_iter().map(|p| position + rotation * p));
        mesh.faces.extend(piece.faces.into_iter().map(|mut face| {
            // The primitive shares UV space between its caps and barrel.
            // A baked sample needs each of these surfaces on its own island.
            let (uv_origin, uv_extent) = if primitive == Primitive::Cylinder {
                let normal = face.corners[0].source_normal.unwrap_or(Vec3::ZERO);
                let section = if normal.y > 0.9 {
                    0.
                } else if normal.y < -0.9 {
                    0.70
                } else {
                    0.35
                };
                (
                    uv_origin + uv_extent * Vec2::Y * section,
                    uv_extent * Vec2::new(1., 0.30),
                )
            } else {
                (uv_origin, uv_extent)
            };
            for corner in &mut face.corners {
                corner.vertex += offset;
                corner.uv = uv_origin + corner.uv * uv_extent;
                corner.source_normal = corner.source_normal.map(|n| rotation * n);
            }
            face
        }));
    }
    let mesh = MeshData::from(mesh);
    let object = Object {
        id: 0,
        source_node: None,
        name: "Material sample".into(),
        geometry: mesh.clone(),
        transform: Transform::IDENTITY,
        materials: Vec::new(),
        hidden: false,
    };
    let baked = bake::shelf_maps(object, SIZE)?;
    let view = Vec3::new(0.36, 0.37, 1.).normalize();
    let pixels = raster(&mesh, view);
    Ok(Sample {
        mesh,
        baked,
        pixels,
        view,
    })
}

fn raster(mesh: &Geometry, view: Vec3) -> Vec<Option<Fragment>> {
    let right = Vec3::Y.cross(view).normalize();
    let up = view.cross(right);
    let size = FRAME as f32;
    let project = |position: Vec3| {
        Vec2::new(position.dot(right), -position.dot(up)) * (size / 1.72)
            + Vec2::new(size * 0.5, size * 0.49)
    };
    let mut output: Vec<Option<Fragment>> = (0..FRAME * FRAME).map(|_| None).collect();
    let Some(buffers) = mesh.buffers().remove(&0) else {
        return output;
    };
    for indices in buffers.indices.chunks_exact(3) {
        let indices = [indices[0], indices[1], indices[2]].map(|i| i as usize);
        let points = indices.map(|i| Vec3::from(buffers.positions[i]));
        let normals = indices.map(|i| Vec3::from(buffers.normals[i]));
        let uvs = indices.map(|i| Vec2::from(buffers.uv[i]));
        let screen = points.map(project);
        let min = screen[0]
            .min(screen[1])
            .min(screen[2])
            .floor()
            .max(Vec2::ZERO)
            .as_uvec2();
        let max = screen[0]
            .max(screen[1])
            .max(screen[2])
            .ceil()
            .min(Vec2::splat(size))
            .as_uvec2();
        let [du, dv] = [uvs[1] - uvs[0], uvs[2] - uvs[0]];
        let [a, b] = [points[1] - points[0], points[2] - points[0]];
        let determinant = du.perp_dot(dv).signum();
        let tangent = (a * dv.y - b * du.y) * determinant;
        let bitangent = (b * du.x - a * dv.x) * determinant;
        let depths = Vec3::from_array(points.map(|p| p.dot(view)));
        for y in min.y..max.y {
            for x in min.x..max.x {
                let Some(w) =
                    projection::weights(Vec2::new(x as f32 + 0.5, y as f32 + 0.5), screen)
                        .filter(|w| w.min_element() >= -1.0e-5)
                else {
                    continue;
                };
                let pixel = &mut output[(y * FRAME + x) as usize];
                let depth = w.dot(depths);
                if pixel
                    .as_ref()
                    .is_some_and(|previous| previous.depth >= depth)
                {
                    continue;
                }
                let normal =
                    (normals[0] * w.x + normals[1] * w.y + normals[2] * w.z).normalize_or_zero();
                let tangent = (tangent - normal * tangent.dot(normal))
                    .try_normalize()
                    .unwrap_or_else(|| normal.any_orthonormal_vector());
                *pixel = Some(Fragment {
                    depth,
                    uv: uvs[0] * w.x + uvs[1] * w.y + uvs[2] * w.z,
                    normal,
                    tangent,
                    handedness: normal.cross(tangent).dot(bitangent).signum(),
                });
            }
        }
    }
    output
}

pub(super) fn render(layers: Vec<Layer>) -> Result<RgbaImage, String> {
    let sample = sample()?;
    let mut surface = Surface {
        size: SIZE,
        base: [
            [160, 160, 160, 255],
            [140, 140, 140, 255],
            [0, 0, 0, 255],
            [128, 128, 128, 255],
            [255, 255, 255, 255],
            [0, 0, 0, 255],
            [128, 128, 255, 255],
            [128, 128, 128, 255],
        ]
        .map(|rgba| solid(SIZE, rgba)),
        stack: Stack {
            layers,
            selected: 0,
        },
        material: serde_json::Value::Null,
        baked: Some(sample.baked.clone()),
    };
    surface.bind_generators(&sample.mesh, 0, Vec2::ZERO, Transform::IDENTITY);
    surface.bind_texture_fills(&sample.mesh, 0, Vec2::ZERO, Transform::IDENTITY);
    let mut maps = surface.composites();
    maps[6] = surface.normal_map(&maps);
    let mut image = RgbaImage::new(FRAME, FRAME);
    for (pixel, fragment) in image.pixels_mut().zip(&sample.pixels) {
        if let Some(fragment) = fragment {
            *pixel = shade(fragment, sample.view, &maps);
        }
    }
    Ok(image)
}

fn shade(fragment: &Fragment, view: Vec3, maps: &[Vec<u8>; CHANNEL_COUNT]) -> Rgba<u8> {
    let pos = (fragment.uv * SIZE as f32 - Vec2::splat(0.5))
        .clamp(Vec2::ZERO, Vec2::splat((SIZE - 1) as f32));
    let p = pos.as_uvec2();
    let q = (p + UVec2::ONE).min(UVec2::splat(SIZE - 1));
    let fraction = pos - p.as_vec2();
    let sample = |channel: usize, component: usize| {
        let load = |x, y| f32::from(maps[channel][((y * SIZE + x) * 4) as usize + component]);
        let upper = load(p.x, p.y).lerp(load(q.x, p.y), fraction.x);
        let lower = load(p.x, q.y).lerp(load(q.x, q.y), fraction.x);
        upper.lerp(lower, fraction.y) / 255.
    };
    let rgb = |channel: usize| Vec3::from_array(std::array::from_fn(|i| sample(channel, i)));
    let scalar = |channel: usize| sample(channel, 0);
    let base = rgb(0).powf(2.2);
    let metal = scalar(2);
    let rough = scalar(1);
    let detail = rgb(6) * 2. - Vec3::ONE;
    let bitangent = fragment.normal.cross(fragment.tangent) * fragment.handedness;
    let normal = (fragment.normal * detail.z + fragment.tangent * detail.x + bitangent * detail.y)
        .normalize_or_zero();
    let key = Vec3::new(-0.55, 0.78, 0.74).normalize();
    let fill = Vec3::new(0.85, 0.2, -0.6).normalize();
    let shine = |light: Vec3| {
        normal
            .dot((light + view).normalize())
            .max(0.)
            .powf(4. + 65. * (1. - rough).powi(3))
    };
    let diffuse = 0.24 + 0.80 * normal.dot(key).max(0.) + 0.20 * normal.dot(fill).max(0.);
    let specular = Vec3::splat(scalar(7) * 0.08).lerp(base, metal);
    let lit = base * diffuse * (1. - metal * 0.50)
        + specular * (shine(key) + 0.37 * shine(fill)) * (1.15 - 0.55 * rough)
        + rgb(5).powf(2.2);
    let [r, g, b] = lit
        .clamp(Vec3::ZERO, Vec3::ONE)
        .powf(1. / 2.2)
        .to_array()
        .map(|v| (v * 255.).round() as u8);
    Rgba([r, g, b, (scalar(4) * 255.).round() as u8])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smart_shelf_preview_displays_baked_mask_and_material_layers() {
        let fill = |color| {
            let mut layer = Layer::new("Fill".into(), LayerKind::Fill);
            layer.channels[0] = Some(solid(SIZE, color));
            layer
        };
        let mut base = fill([16, 55, 204, 255]);
        let mut top = fill([237, 50, 18, 255]);
        base.depth = 1;
        top.depth = 1;
        let mut mask = Mask::new(solid(SIZE, [255, 0, 0, 255]), Vec::new());
        mask.affine = true;
        mask.generator = Some(Generator::new(GeneratorKind::EdgeWear));
        let with_mask = {
            let mut top = top.clone();
            top.mask = Some(Arc::new(mask));
            top
        };
        let folder = Layer::new("Material".into(), LayerKind::Folder);
        let blue = render(vec![base.clone(), folder.clone()]).unwrap();
        let orange = render(vec![base.clone(), top, folder.clone()]).unwrap();
        let composite = render(vec![base, with_mask, folder]).unwrap();
        let count = |other: &RgbaImage| {
            composite
                .pixels()
                .zip(other.pixels())
                .filter(|(a, b)| a.0[3] > 0 && (0..3).any(|i| a.0[i].abs_diff(b.0[i]) >= 15))
                .count()
        };
        assert!(
            count(&blue) > 64,
            "the baked generator must reveal the orange coating"
        );
        assert!(
            count(&orange) > 64,
            "the blue material must show through the mask"
        );
        assert_eq!(composite.get_pixel(0, 0).0[3], 0);
        if let Some(path) = std::env::var_os("BEVY_SMART_SWATCH_QA") {
            let recipe = SMART_MATERIALS
                .iter()
                .find(|recipe| recipe.name == "Brass Aged")
                .unwrap();
            render(recipe.prepare(SIZE).unwrap())
                .unwrap()
                .save(path)
                .unwrap();
        }
    }
}
