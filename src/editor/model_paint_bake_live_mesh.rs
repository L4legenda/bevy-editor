//! Raster and ray data reused between previews of the same texture set.
use super::*;
use crate::editor::model_paint_data::custom::Recipe;
use std::sync::Mutex;

struct UvTriangle {
    face: usize,
    vertices: [usize; 3],
    uv: [Vec2; 3],
    normals: [Vec3; 3],
}

#[derive(Clone, Copy)]
struct Texel {
    index: usize,
    surface: Option<RaySurface>,
    width: f32,
}

type Rasters = Vec<(u32, bool, Arc<[Texel]>)>;
type Padding = Vec<(u32, Arc<padding::Offsets>)>;

struct Gpu {
    rays: rays_gpu::Batch,
    projection: Option<rays_gpu::ProjectBatch>,
}

struct PreviewProjection<'a> {
    batch: &'a mut rays_gpu::ProjectBatch,
    source: &'a project::Projection,
    parts: &'a project::HighRelief,
    probes: Vec<aa::Probe>,
    pending: Vec<usize>,
}

impl PreviewProjection<'_> {
    fn flush(&mut self, texels: &mut [Texel], progress: &Progress) -> Result<(), String> {
        let hits = self.batch.run(self.source, &self.probes, progress)?;
        for (index, hit) in self.pending.drain(..).zip(hits) {
            texels[index].surface =
                hit.map(|hit| RaySurface::projected(self.source.scene(), self.parts, hit));
        }
        self.probes.clear();
        Ok(())
    }
}

pub(in crate::editor::ui::model_viewer::paint::bake) struct Scene {
    geometry: PreparedScene,
    high_parts: Option<Arc<project::HighRelief>>,
    target: usize,
    triangles: Vec<UvTriangle>,
    average_normals: bool,
    rasters: Mutex<Rasters>,
    padding: Mutex<Padding>,
    gpu: Mutex<Option<Gpu>>,
}

impl Scene {
    pub fn prepare(
        objects: Vec<Object>,
        target: usize,
        slot: usize,
        settings: &ProjectionSettings,
        progress: &Progress,
    ) -> Result<Self, String> {
        let object = objects
            .iter()
            .find(|object| object.id == target && !object.hidden)
            .ok_or("Объект скрыт")?;
        let geometry = prepare_scene(&objects, object, settings, true, progress)?;
        let high_parts = geometry
            .projection
            .as_ref()
            .map(|projector| {
                let params = relief::Parameters::new(geometry.bounds, geometry.scale, settings.surface);
                projector.build_relief(params, false, progress)
            })
            .transpose()?;
        let offset = object.uv_offset(slot);
        let mut triangles = Vec::new();
        for (face, polygon) in object
            .geometry
            .faces
            .iter()
            .enumerate()
            .filter(|(_, face)| face.material == slot)
        {
            progress.check()?;
            for triangle in object.geometry.triangles(polygon) {
                let corners = triangle.map(|index| &polygon.corners[index]);
                triangles.push(UvTriangle {
                    face,
                    vertices: corners.map(|corner| corner.vertex),
                    uv: corners.map(|corner| corner.uv - offset),
                    normals: triangle.map(|index| geometry.corner_normals[face][index]),
                });
            }
        }
        if triangles.is_empty() {
            return Err("В наборе нет граней".into());
        }
        Ok(Self {
            geometry,
            high_parts,
            average_normals: settings.average_normals,
            target,
            triangles,
            rasters: Mutex::default(),
            padding: Mutex::default(),
            gpu: Mutex::default(),
        })
    }

    fn raster(
        &self,
        size: u32,
        progress: &Progress,
        batch: Option<&mut rays_gpu::ProjectBatch>,
    ) -> Result<Arc<[Texel]>, String> {
        let scene = &self.geometry;
        let high = scene.projection.as_ref().zip(self.high_parts.as_ref());
        let mut gpu = batch
            .zip(high)
            .map(|(batch, (source, parts))| PreviewProjection {
                batch,
                source,
                parts,
                probes: Vec::new(),
                pending: Vec::new(),
            });
        let projected_on_gpu = gpu.is_some();
        if let Some((_, _, raster)) = self
            .rasters
            .lock()
            .unwrap()
            .iter()
            .find(|(res, on_gpu, _)| *res == size && *on_gpu == projected_on_gpu)
        {
            return Ok(raster.clone());
        }
        let extent = Vec2::splat(size as f32);
        let mut covered = vec![false; (size * size) as usize];
        let mut texels = Vec::new();
        let pattern = aa::SamplePattern::new(1);
        for triangle in &self.triangles {
            progress.check()?;
            let geo = scene.geometric[triangle.face];
            let uv = triangle.uv.map(|uv| uv * extent);
            let raster = raster::Triangle::new(triangle.face, triangle.vertices, uv, size);
            let points = triangle.vertices.map(|vertex| scene.positions[vertex]);
            let cage = scene
                .cage
                .as_ref()
                .map(|cage| cage.triangle(triangle.vertices));
            let frame = scene
                .projection
                .as_ref()
                .and_then(|_| project::tangent_axes(points, uv))
                .zip(raster.barycentric)
                .map(|(axes, barycentric)| aa::ProjectTriangle {
                    barycentric,
                    points,
                    normals: triangle.normals,
                    geometric: geo,
                    axes,
                    directions: triangle_ray_directions(
                        triangle.vertices,
                        triangle.normals,
                        self.average_normals.then_some(scene.projection_normals.as_slice()),
                        geo,
                    ),
                    cage: cage.as_ref(),
                });
            let width = two_texel_width(points, uv);
            for y in raster.lo.y..raster.hi.y {
                progress.check()?;
                for x in raster.lo.x..raster.hi.x {
                    let index = (y * size + x) as usize;
                    if covered[index] {
                        continue;
                    }
                    let center = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
                    let Some(w) = raster.weights(center)
                    else {
                        continue;
                    };
                    covered[index] = true;
                    let surface = if let Some((projection, parts)) = high {
                        if let Some(gpu) = gpu.as_mut() {
                            if let Some(frame) = &frame {
                                gpu.pending.push(texels.len());
                                gpu.probes.push(frame.center_probe(projection, w));
                            }
                            None
                        } else {
                            frame
                                .as_ref()
                                .and_then(|frame| frame.sample(projection, center, w, &pattern))
                                .map(|sample| {
                                    RaySurface::projected(projection.scene(), parts, sample.center)
                                })
                        }
                    } else if let Some(parts) = &scene.parts {
                        let part = parts.faces[triangle.face];
                        let [a, b, c] = triangle.normals;
                        Some(RaySurface {
                            point: points[0] * w.x + points[1] * w.y + points[2] * w.z,
                            normal: (a * w.x + b * w.y + c * w.z).try_normalize().unwrap_or(geo),
                            geometric: geo,
                            object: self.target,
                            face: triangle.face,
                            part,
                            bounds: parts.bounds[part],
                        })
                    } else {
                        None
                    };
                    texels.push(Texel {
                        index,
                        surface,
                        width,
                    });
                    if let Some(gpu) = gpu.as_mut()
                        && gpu.probes.len() == gpu.batch.capacity
                    {
                        gpu.flush(&mut texels, progress)?;
                    }
                }
            }
        }
        if texels.is_empty() {
            return Err("UV-развёртка набора не покрывает карту".into());
        }
        if let Some(gpu) = gpu.as_mut() {
            gpu.flush(&mut texels, progress)?;
        }
        let raster: Arc<[Texel]> = texels.into();
        let mut cache = self.rasters.lock().unwrap();
        if cache.len() >= 2 {
            cache.remove(0);
        }
        cache.push((size, projected_on_gpu, raster.clone()));
        Ok(raster)
    }

    #[cfg(test)]
    pub fn has_gpu_raster(&self, size: u32) -> bool {
        self.rasters
            .lock()
            .unwrap()
            .iter()
            .any(|(res, gpu, _)| *res == size && *gpu)
    }

    #[cfg(test)]
    pub fn shade(
        &self,
        recipe: &Recipe,
        size: u32,
        quality: super::super::Quality,
        progress: &Progress,
    ) -> Result<Vec<u8>, String> {
        self.shade_on_device(recipe, size, quality, progress, None)
            .map(|(pixels, _)| pixels)
    }

    pub fn shade_on_device(
        &self,
        recipe: &Recipe,
        size: u32,
        quality: super::super::Quality,
        progress: &Progress,
        device: Option<&RayDevice>,
    ) -> Result<(Vec<u8>, bool), String> {
        let scene = &self.geometry;
        let prepared = custom::Prepared::new(recipe, scene.bounds);
        let sampling = quality.sampling();
        let rays = if let Some(projection) = &scene.projection {
            Rays {
                scene: projection.scene(),
                scale: scene.scale,
                sampling,
                maps: MapSelection::ALL,
                resolution: size,
                flow_normals: Some(custom::FlowNormals::High(projection)),
            }
        } else {
            Rays {
                scene: scene.low_scene.as_ref().expect("low-poly bake scene"),
                scale: scene.scale,
                sampling,
                maps: MapSelection::ALL,
                resolution: size,
                flow_normals: Some(custom::FlowNormals::Mesh(&scene.triangle_normals)),
            }
        };
        let mut pixels = vec![[0; 4]; (size * size) as usize];
        let mut covered = vec![0; pixels.len()];
        let mut gpu_raster = None;
        if let Some(device) = device {
            match self.shade_gpu(device, &rays, &prepared, &mut pixels, progress) {
                Ok(raster) => gpu_raster = raster,
                Err(error) => {
                    progress.check()?;
                    warn!("GPU preview failed; retrying on the CPU: {error}");
                }
            }
        }
        let used_gpu = gpu_raster.is_some();
        let raster = if let Some(raster) = gpu_raster {
            raster
        } else {
            let raster = self.raster(size, progress, None)?;
            for texel in raster.iter() {
                progress.check()?;
                let value = texel.surface.map_or(0, |surface| {
                    prepared.sample(&rays, surface, texel.width, &|triangle| {
                        if let Some(parts) = &self.high_parts {
                            parts.part(triangle.face) != surface.part
                        } else if let Some(parts) = &scene.parts {
                            triangle.object != surface.object
                                || parts.faces[triangle.face] != surface.part
                        } else {
                            false
                        }
                    })
                });
                pixels[texel.index] = [value, value, value, 255];
            }
            raster
        };
        for texel in raster.iter() {
            covered[texel.index] = 1;
        }
        let offsets = if let Some(device) = device {
            self.padding_on_device(device, size, &covered, progress)?
        } else {
            None
        };
        let copy_texel = |i: usize, neighbor: usize| pixels[i] = pixels[neighbor];
        if let Some(offsets) = offsets {
            offsets.apply(&mut covered, progress, copy_texel)?;
        } else {
            pad_uv(size as usize, &mut covered, progress, copy_texel)?;
        }
        Ok((pixels.into_flattened(), used_gpu))
    }

    fn padding_on_device(
        &self,
        device: &RayDevice,
        size: u32,
        covered: &[u8],
        progress: &Progress,
    ) -> Result<Option<Arc<padding::Offsets>>, String> {
        let mut cache = self.padding.lock().unwrap();
        if let Some((_, offsets)) = cache.iter().find(|(resolution, _)| *resolution == size) {
            return Ok(Some(offsets.clone()));
        }
        let offsets = padding::offsets(device, size as usize, covered, progress)?.map(Arc::new);
        if let Some(offsets) = &offsets {
            if cache.len() >= 2 {
                cache.remove(0);
            }
            cache.push((size, offsets.clone()));
        }
        Ok(offsets)
    }

    #[cfg(test)]
    pub fn has_gpu_padding(&self, size: u32) -> bool {
        self.padding
            .lock()
            .unwrap()
            .iter()
            .any(|(resolution, _)| *resolution == size)
    }

    fn shade_gpu(
        &self,
        device: &RayDevice,
        rays: &Rays,
        recipe: &custom::Prepared,
        pixels: &mut [[u8; 4]],
        progress: &Progress,
    ) -> Result<Option<Arc<[Texel]>>, String> {
        let mut gpu = self.gpu.lock().unwrap();
        progress.check()?;
        let recipes = std::slice::from_ref(recipe);
        if gpu.is_none() {
            let labels = if let (Some(parts), Some(projection)) =
                (&self.high_parts, &self.geometry.projection)
            {
                rays_gpu::PartsOfScene::High(parts, projection)
            } else {
                rays_gpu::PartsOfScene::Low(self.geometry.parts.as_ref().unwrap(), self.target)
            };
            *gpu = rays_gpu::Batch::new(device, rays, labels, recipes, false, None).map(|batch| Gpu {
                projection: self.high_parts.as_ref().and_then(|_| batch.project_batch()),
                rays: batch,
            });
        }
        let Some(Gpu {
            rays: batch,
            projection,
        }) = gpu.as_mut()
        else {
            return Ok(None);
        };
        let result = (|| {
            let raster = self.raster(rays.resolution, progress, projection.as_mut())?;
            batch.configure(recipes, rays);
            for texel in raster.iter() {
                progress.check()?;
                pixels[texel.index] = [0, 0, 0, 255];
                if let Some(surface) = texel.surface {
                    batch.push(surface, texel.index, texel.width);
                    if batch.full() {
                        batch.flush_preview(pixels, progress)?;
                    }
                }
            }
            batch.flush_preview(pixels, progress)?;
            Ok(Some(raster))
        })();
        if result.is_err() {
            // An interrupted batch can still own pending inputs or a mapping.
            *gpu = None;
        }
        result
    }
}
