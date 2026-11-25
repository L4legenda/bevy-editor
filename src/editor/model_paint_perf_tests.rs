//! Reference rasterizer retained only to compare pixels and measured cost.
use super::*;
fn legacy_project(camera: CameraMap, p: Vec3) -> Option<Vec2> {
    let p = camera.pose.to_matrix().inverse().transform_point3(p);
    if !camera.ortho && p.z >= -1.0e-6 {
        return None;
    }
    let h = if camera.ortho {
        camera.height
    } else {
        2. * -p.z * (FOV * 0.5).tan()
    };
    Some(
        camera.rect.center()
            + Vec2::new(
                p.x / (h * 0.5 * camera.rect.width() / camera.rect.height()),
                -p.y / (h * 0.5),
            ) * camera.rect.size()
                * 0.5,
    )
}
#[test]
fn model_paint_noop_strokes_preserve_snapshot_identity() {
    let (mut work, map, folder) = super::tests::paint_fixture();
    let map = Arc::new(map);
    let brush = super::tests::brush();
    let original = data::solid(64, [220, 20, 30, 255]);
    work.surface.stack.layers[0].channels[0] = Some(original.clone());
    let result = batch(
        work,
        vec![
            Event::Begin(brush),
            Event::Points(vec![Stamp::from(Vec2::splat(256.))], map.clone()),
            Event::End,
        ],
        false,
    );
    assert!(result.history.is_empty());
    assert!(result.maps.is_none());
    assert!(Arc::ptr_eq(
        &original,
        result.work.surface.stack.layers[0].channels[0]
            .as_ref()
            .unwrap()
    ));
    let mut work = result.work;
    work.surface.stack.layers[0].channels[0] = None;
    let mut eraser = brush;
    eraser.erase = true;
    let result = batch(
        work,
        vec![
            Event::Begin(eraser),
            Event::Points(vec![Stamp::from(Vec2::splat(256.))], map),
            Event::End,
        ],
        false,
    );
    assert!(result.history.is_empty());
    assert!(result.maps.is_none());
    assert!(result.work.surface.stack.layers[0].channels[0].is_none());
    std::fs::remove_dir_all(folder).unwrap();
}
#[test]
fn model_paint_bounded_batches_preserve_samples_and_stroke_boundaries() {
    let (_, map, folder) = super::tests::paint_fixture();
    let map = Arc::new(map);
    let points = (0..100)
        .map(|i| Stamp::from(Vec2::splat(i as f32)))
        .collect::<Vec<_>>();
    let mut queue = VecDeque::from([
        Event::Begin(super::tests::brush()),
        Event::Points(points.clone(), map),
        Event::End,
    ]);
    let mut all = Vec::new();
    while !queue.is_empty() {
        let events = take_events(&mut queue);
        assert!(
            events
                .iter()
                .filter_map(|e| if let Event::Points(p, _) = e {
                    Some(p.len())
                } else {
                    None
                })
                .sum::<usize>()
                <= 32
        );
        all.extend(events);
    }
    assert!(matches!(all.first(), Some(Event::Begin(_))));
    assert!(matches!(all.last(), Some(Event::End)));
    let actual = all
        .into_iter()
        .filter_map(|e| {
            if let Event::Points(p, _) = e {
                Some(p)
            } else {
                None
            }
        })
        .flatten()
        .collect::<Vec<_>>();
    assert_eq!(actual, points);
    std::fs::remove_dir_all(folder).unwrap();
}
#[test]
#[ignore = "measures reference vs incremental raster/compositing on a 40k-triangle model at 1024px"]
fn model_paint_benchmark_40k_triangles() {
    use crate::editor::model_mesh::{Corner, Face, Geometry};
    let (mut work, mut map, folder) = super::tests::paint_fixture();
    let mut mesh = Geometry::default();
    let nx = 100;
    let ny = 200;
    for y in 0..ny {
        for x in 0..nx {
            let start = mesh.positions.len();
            let uv = [
                Vec2::new(x as f32 / nx as f32, y as f32 / ny as f32),
                Vec2::new((x + 1) as f32 / nx as f32, y as f32 / ny as f32),
                Vec2::new((x + 1) as f32 / nx as f32, (y + 1) as f32 / ny as f32),
                Vec2::new(x as f32 / nx as f32, (y + 1) as f32 / ny as f32),
            ];
            mesh.positions
                .extend(uv.map(|p| Vec3::new(p.x * 2. - 1., p.y * 2. - 1., 0.)));
            mesh.faces.push(Face {
                corners: (0..4)
                    .map(|i| {
                        let mut c = Corner::new(start + i);
                        c.uv = uv[i];
                        c
                    })
                    .collect(),
                material: 0,
                smooth: false,
            });
        }
    }
    map.objects[0].geometry = mesh.into();
    work.surface.size = 1024;
    work.surface.base = std::array::from_fn(|_| data::solid(1024, [255; 4]));
    let map = Arc::new(map);
    let prep = std::time::Instant::now();
    assert_eq!(map.geometry().triangles.len(), 40000);
    let prep_ms = prep.elapsed().as_secs_f64() * 1000.;
    let surface = work.surface.clone();
    let mut brush = super::tests::brush();
    brush.radius = 30.;
    brush.hardness = 0.6;
    brush.flow = 0.35;
    let points = (0..20)
        .map(|i| Vec2::new(220. + i as f32 * 3., 250.))
        .collect::<Vec<_>>();
    let mut legacy = batch(work, vec![Event::Begin(brush)], false).work;
    let start = std::time::Instant::now();
    assert!(legacy_points(&mut legacy, &points, &map));
    let flattened = legacy.surface.composites();
    let legacy_ms = start.elapsed().as_secs_f64() * 1000.;
    let start = std::time::Instant::now();
    let result = batch(
        Work {
            surface,
            preview: Default::default(),
            baked_preview: None,
            stroke: None,
        },
        vec![
            Event::Begin(brush),
            Event::Points(points.into_iter().map(Stamp::from).collect(), map),
            Event::End,
        ],
        false,
    );
    let new_ms = start.elapsed().as_secs_f64() * 1000.;
    let actual = result.work.surface.composite(0);
    assert!(
        actual
            .iter()
            .zip(&flattened[0])
            .all(|(a, b)| a.abs_diff(*b) <= 1),
        "incremental raster must match the old stroke"
    );
    let patches = &result.maps.as_ref().unwrap().textures;
    assert_eq!(
        patches.iter().flatten().count(),
        1,
        "color strokes must not upload unrelated channels"
    );
    let bytes = patches
        .iter()
        .flatten()
        .map(|p| p.rgba.len())
        .sum::<usize>();
    assert!(bytes < 1024 * 1024 * 4 / 3);
    eprintln!(
        "PAINT PERF: triangles=40000 texture=1024 stamps=20 prepare_ms={prep_ms:.2} legacy_ms={legacy_ms:.2} incremental_ms={new_ms:.2} speedup={:.2} old_upload_bytes={} new_upload_bytes={bytes}",
        legacy_ms / new_ms,
        9 * 1024 * 1024 * 4
    );
    std::fs::remove_dir_all(folder).unwrap();
}
#[test]
fn model_paint_incremental_composite_matches_full_for_all_channels() {
    let (mut work, _, folder) = super::tests::paint_fixture();
    let surface = &mut work.surface;
    let mut layer = Layer::new("Test".into(), LayerKind::Paint);
    layer.blend = data::Blend::Overlay;
    for c in 0..8 {
        let mut p = vec![0; 64 * 64 * 4];
        for (i, p) in p.chunks_exact_mut(4).enumerate() {
            p.copy_from_slice(&[(i % 251) as u8, (c * 20) as u8, 180, 170]);
        }
        layer.channels[c] = Some(Arc::new(p));
    }
    surface.stack.layers.push(layer);
    let rect = URect {
        min: UVec2::new(3, 5),
        max: UVec2::new(23, 31),
    };
    for c in 0..8 {
        let full = surface.composite(c);
        let region = surface.composite_rect(c, rect);
        let expected = (rect.min.y..rect.max.y)
            .flat_map(|y| {
                let start = (y * 64 + rect.min.x) as usize * 4;
                full[start..start + rect.width() as usize * 4].to_vec()
            })
            .collect::<Vec<_>>();
        assert_eq!(region, expected);
    }
    let all = surface.composites();
    let normal = surface.normal_map(&all);
    let mut dirty = Default::default();
    let dirty: &mut Dirty = &mut dirty;
    dirty[3] = Some(rect);
    let updates = composite::prepare(surface, *dirty, 4.);
    let patch = updates.textures[2].as_ref().unwrap();
    let expected = (patch.rect.min.y..patch.rect.max.y)
        .flat_map(|y| {
            let start = (y * 64 + patch.rect.min.x) as usize * 4;
            normal[start..start + patch.rect.width() as usize * 4].to_vec()
        })
        .collect::<Vec<_>>();
    assert_eq!(patch.rgba, expected);
    std::fs::remove_dir_all(folder).unwrap();
}
fn legacy_points(work: &mut Work, points: &[Vec2], map: &Mapping) -> bool {
    let Some(stroke) = work.stroke.as_mut() else {
        return false;
    };
    let brush = stroke.brush;
    let selected = work.surface.stack.selected;
    let Some(layer) = work
        .surface
        .stack
        .layers
        .get_mut(selected)
        .filter(|l| l.kind == LayerKind::Paint && l.visible)
    else {
        return false;
    };
    let size = work.surface.size as usize;
    let bytes =
        layer.channels[brush.channel].get_or_insert_with(|| Arc::new(vec![0; size * size * 4]));
    let pixels = Arc::make_mut(bytes);
    let mut changed = false;
    for &point in points {
        let face = if !map.uv {
            let (o, d) = map.camera.ray(point);
            let hit = map.geometry().bvh.cast(o, d, None);
            if !hit.is_some_and(|(object, _, _)| object == map.object) {
                continue;
            }
            hit.map(|(_, f, _)| f)
        } else if brush.fill {
            let uv = (point - map.camera.rect.min) / map.camera.rect.size();
            let face = map
                .geometry()
                .triangles
                .iter()
                .find(|t| weights(uv, t.uv).is_some_and(|b| b.min_element() >= 0.))
                .map(|t| t.face);
            if face.is_none() {
                continue;
            }
            face
        } else {
            None
        };
        let bounds = Rect::from_center_size(point, Vec2::splat(brush.radius * 2.));
        for triangle in &map.geometry().triangles {
            if brush.fill && face.is_some_and(|f| f != triangle.face) {
                continue;
            }
            let screen = if map.uv {
                triangle
                    .uv
                    .map(|uv| map.camera.rect.min + uv * map.camera.rect.size())
            } else {
                let p = triangle.points.map(|p| legacy_project(map.camera, p));
                let [Some(a), Some(b), Some(c)] = p else {
                    continue;
                };
                [a, b, c]
            };
            let smin = screen[0].min(screen[1]).min(screen[2]);
            let smax = screen[0].max(screen[1]).max(screen[2]);
            if !brush.fill
                && (smin.x > bounds.max.x
                    || smax.x < bounds.min.x
                    || smin.y > bounds.max.y
                    || smax.y < bounds.min.y)
            {
                continue;
            }
            let mut umin = triangle.uv[0].min(triangle.uv[1]).min(triangle.uv[2]);
            let mut umax = triangle.uv[0].max(triangle.uv[1]).max(triangle.uv[2]);
            if !brush.fill {
                let corners = [
                    bounds.min,
                    Vec2::new(bounds.max.x, bounds.min.y),
                    bounds.max,
                    Vec2::new(bounds.min.x, bounds.max.y),
                ];
                let inverse = map.camera.pose.to_matrix().inverse();
                let depth = triangle.points.map(|p| -inverse.transform_point3(p).z);
                let projected = corners.map(|p| {
                    weights(p, screen).and_then(|mut b| {
                        if !map.uv && !map.camera.ortho {
                            b /= Vec3::from_array(depth);
                            let sum = b.element_sum();
                            if sum.abs() < 1.0e-8 {
                                return None;
                            }
                            b /= sum;
                        }
                        Some(triangle.uv[0] * b.x + triangle.uv[1] * b.y + triangle.uv[2] * b.z)
                    })
                });
                if let [Some(a), Some(b), Some(c), Some(d)] = projected {
                    umin = umin.max(a.min(b).min(c).min(d));
                    umax = umax.min(a.max(b).max(c).max(d));
                }
            }
            let lo = (umin.clamp(Vec2::ZERO, Vec2::ONE) * size as f32)
                .floor()
                .as_uvec2();
            let hi = (umax.clamp(Vec2::ZERO, Vec2::ONE) * size as f32)
                .ceil()
                .as_uvec2()
                .min(UVec2::splat(size as u32));
            for y in lo.y..hi.y {
                for x in lo.x..hi.x {
                    let uv = (Vec2::new(x as f32, y as f32) + 0.5) / size as f32;
                    let Some(b) = weights(uv, triangle.uv).filter(|b| b.min_element() >= -1.0e-5)
                    else {
                        continue;
                    };
                    let world = mix3(triangle.points, b);
                    let projected = if map.uv {
                        map.camera.rect.min + uv * map.camera.rect.size()
                    } else {
                        let Some(p) = legacy_project(map.camera, world) else {
                            continue;
                        };
                        p
                    };
                    let fall = if brush.fill {
                        1.
                    } else {
                        data::coverage(projected.distance(point) / brush.radius, brush.hardness)
                    };
                    if fall <= 0. {
                        continue;
                    }
                    if !map.uv {
                        let (o, d) = map.camera.ray(projected);
                        let depth = (world - o).dot(d);
                        if !map
                            .geometry()
                            .bvh
                            .cast(o, d, None)
                            .is_some_and(|(id, _, near)| {
                                id == map.object && depth <= near + map.camera.epsilon
                            })
                        {
                            continue;
                        }
                    }
                    let i = y as usize * size + x as usize;
                    let c = stroke.coverage[i];
                    let mask = if brush.fill {
                        1.
                    } else {
                        c + (1. - c) * fall * brush.flow
                    };
                    if mask <= c {
                        continue;
                    }
                    stroke.coverage[i] = mask;
                    let base = stroke
                        .original
                        .as_ref()
                        .map_or([0; 4], |p| p[i * 4..i * 4 + 4].try_into().unwrap());
                    let rgba = data::over(base, brush.color, mask * brush.opacity, brush.erase);
                    if pixels[i * 4..i * 4 + 4] != rgba {
                        pixels[i * 4..i * 4 + 4].copy_from_slice(&rgba);
                        changed = true;
                    }
                }
            }
        }
    }
    stroke.changed |= changed;
    changed
}
