use super::{Document, MaterialSlot, Object, udim};
use crate::editor::model_mesh::{Corner, Face, Geometry};
use crate::editor::model_primitives::Primitive;
use bevy::prelude::*;
use serde_json::{Value, json};
use std::{
    collections::{HashMap, VecDeque},
    sync::Arc,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::editor) enum Command {
    Toggle,
    Primitive(Shape),
    Import,
    Undo,
    Redo,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "mcp", derive(serde::Deserialize, serde::Serialize, rmcp::schemars::JsonSchema))]
#[cfg_attr(feature = "mcp", serde(rename_all = "snake_case"))]
#[cfg_attr(feature = "mcp", schemars(crate = "rmcp::schemars"))]
pub(in crate::editor) enum Shape {
    RoundedCube,
    Sphere,
    Torus,
    Cylinder,
    Plane,
    MaterialBall,
}

impl Shape {
    pub const ALL: [Self; 6] = [
        Self::RoundedCube,
        Self::Sphere,
        Self::Torus,
        Self::Cylinder,
        Self::Plane,
        Self::MaterialBall,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::RoundedCube => "Скруглённый куб",
            Self::Sphere => "Сфера",
            Self::Torus => "Тор",
            Self::Cylinder => "Цилиндр",
            Self::Plane => "Плоскость",
            Self::MaterialBall => "Шар для материалов",
        }
    }

    fn geometry(self) -> Result<Geometry, String> {
        let primitive = match self {
            Self::RoundedCube => return Ok(rounded_cube()),
            Self::MaterialBall => return material_ball(),
            Self::Sphere => Primitive::UvSphere,
            Self::Torus => Primitive::Torus,
            Self::Cylinder => Primitive::Cylinder,
            Self::Plane => Primitive::Plane,
        };
        let mut geometry = primitive.geometry(&primitive.defaults())?;
        if primitive == Primitive::Cylinder {
            for face in &mut geometry.faces {
                let normal_y = face.corners[0].source_normal.map_or(0., |normal| normal.y);
                let (origin, scale) = if normal_y.abs() > 0.5 {
                    (
                        Vec2::new(if normal_y > 0. { 0.03 } else { 0.52 }, 0.58),
                        Vec2::splat(0.40),
                    )
                } else {
                    (Vec2::splat(0.02), Vec2::new(0.96, 0.52))
                };
                for corner in &mut face.corners {
                    corner.uv = origin + scale * corner.uv;
                    corner.uv1 = corner.uv;
                }
            }
        }
        Ok(geometry)
    }
}

pub(in crate::editor) fn primitive(shape: Shape) -> Result<Vec<Object>, String> {
    let geometry = shape.geometry()?;
    geometry.validate()?;
    Ok(vec![Object {
        id: 0,
        source_node: None,
        name: shape.label().into(),
        geometry: geometry.into(),
        transform: Transform::IDENTITY,
        materials: vec![MaterialSlot {
            material: default(),
            source: json!({}),
            paint: None,
            authored: None,
            bake_fixes: default(),
        }],
        hidden: false,
    }])
}

fn rounded_cube() -> Geometry {
    const STEPS: i32 = 24;
    const HALF: i32 = STEPS / 2;
    let mut geometry = Geometry::default();
    let mut vertices = HashMap::new();
    let core = Vec3::splat(0.78);
    for (chart, (normal, horizontal)) in [
        (IVec3::X, IVec3::NEG_Z),
        (IVec3::NEG_X, IVec3::Z),
        (IVec3::Y, IVec3::X),
        (IVec3::NEG_Y, IVec3::X),
        (IVec3::Z, IVec3::X),
        (IVec3::NEG_Z, IVec3::NEG_X),
    ]
    .into_iter()
    .enumerate()
    {
        let vertical = normal.cross(horizontal);
        let chart = Vec2::new((chart % 3) as f32, (chart / 3) as f32);
        let mut corner = |x: i32, y: i32, geometry: &mut Geometry| {
            let key = normal * HALF + horizontal * x + vertical * y;
            let point = key.as_vec3() / HALF as f32;
            let base = point.clamp(-core, core);
            let direction = (point - base).normalize();
            let vertex = *vertices.entry(key).or_insert_with(|| {
                let vertex = geometry.positions.len();
                geometry.positions.push(base + direction * 0.22);
                vertex
            });
            let uv = Vec2::new((x + HALF) as f32, (HALF - y) as f32) / STEPS as f32;
            let uv = (chart + 0.03 + 0.94 * uv) / Vec2::new(3., 2.);
            Corner {
                uv,
                uv1: uv,
                source_normal: Some(direction),
                ..Corner::new(vertex)
            }
        };
        for y in -HALF..HALF {
            for x in -HALF..HALF {
                let corners = [(x, y), (x + 1, y), (x + 1, y + 1), (x, y + 1)]
                    .map(|(x, y)| corner(x, y, &mut geometry))
                    .to_vec();
                geometry.faces.push(Face {
                    corners,
                    material: 0,
                    smooth: true,
                });
            }
        }
    }
    geometry
}

fn material_ball() -> Result<Geometry, String> {
    let mut result = Geometry::default();
    let mut add = |mesh: Geometry, transform: Transform, uv_min: Vec2, uv_size: Vec2| {
        let offset = result.positions.len();
        result.positions.extend(
            mesh.positions
                .into_iter()
                .map(|p| transform.transform_point(p)),
        );
        result.faces.extend(mesh.faces.into_iter().map(|mut face| {
            for corner in &mut face.corners {
                corner.vertex += offset;
                corner.uv = uv_min + uv_size * corner.uv;
                corner.uv1 = corner.uv;
                corner.source_normal = corner.source_normal.map(|normal| {
                    (transform.rotation * (normal / transform.scale)).normalize_or_zero()
                });
            }
            face
        }));
    };
    let sphere = Primitive::UvSphere;
    let mut sphere_parameters = sphere.defaults();
    sphere_parameters.radius = 0.9;
    add(
        sphere.geometry(&sphere_parameters)?,
        Transform::from_xyz(0., 0.15, 0.),
        Vec2::new(0.02, 0.02),
        Vec2::new(0.60, 0.62),
    );
    add(
        rounded_cube(),
        Transform::from_xyz(0., -0.87, 0.).with_scale(Vec3::new(0.73, 0.14, 0.73)),
        Vec2::new(0.02, 0.67),
        Vec2::new(0.60, 0.30),
    );
    let torus = Primitive::Torus;
    let mut ring_parameters = torus.defaults();
    ring_parameters.major_radius = 0.87;
    ring_parameters.minor_radius = 0.06;
    let ring = torus.geometry(&ring_parameters)?;
    for (index, rotation) in [Quat::from_rotation_z(0.56), Quat::from_rotation_x(-0.9)]
        .into_iter()
        .enumerate()
    {
        add(
            ring.clone(),
            Transform::from_xyz(0., 0.15, 0.).with_rotation(rotation),
            Vec2::new(0.65, 0.02 + index as f32 * 0.30),
            Vec2::new(0.32, 0.27),
        );
    }
    let mut bolt_parameters = sphere.defaults();
    bolt_parameters.radius = 0.1;
    bolt_parameters.segments = 16;
    bolt_parameters.rings = 8;
    let bolt = sphere.geometry(&bolt_parameters)?;
    for (index, (x, z)) in [(-0.54, -0.54), (-0.54, 0.54), (0.54, -0.54), (0.54, 0.54)]
        .into_iter()
        .enumerate()
    {
        add(
            bolt.clone(),
            Transform::from_xyz(x, -0.74, z).with_scale(Vec3::new(1., 0.47, 1.)),
            Vec2::new(
                0.65 + (index % 2) as f32 * 0.17,
                0.64 + (index / 2) as f32 * 0.17,
            ),
            Vec2::splat(0.15),
        );
    }
    Ok(result)
}

#[derive(PartialEq, Eq, Hash)]
struct SetName {
    name: String,
    tile: u32,
}

fn name(slot: &MaterialSlot, root: &Value) -> SetName {
    SetName {
        name: slot.name(root).unwrap_or_default().trim().to_lowercase(),
        tile: slot.udim().map_or(1001, |tile| tile.number()),
    }
}

/// Reserve every named match before allowing positional matches to take any source set.
fn assignments(old: &[SetName], new: &[SetName], preferred: Option<usize>) -> Vec<Option<usize>> {
    if let Some(index) = preferred.filter(|&index| index < old.len() && new.len() == 1) {
        return vec![Some(index)];
    }
    let mut candidates: HashMap<&SetName, VecDeque<usize>> = HashMap::new();
    for (index, key) in old
        .iter()
        .enumerate()
        .filter(|(_, key)| !key.name.is_empty())
    {
        candidates.entry(key).or_default().push_back(index);
    }
    let mut used = vec![false; old.len()];
    let mut matches: Vec<_> = new
        .iter()
        .map(|key| {
            let index = candidates.get_mut(key)?.pop_front()?;
            used[index] = true;
            Some(index)
        })
        .collect();
    let mut remaining = used
        .iter()
        .enumerate()
        .filter_map(|(index, used)| (!used).then_some(index));
    for missing in matches.iter_mut().filter(|entry| entry.is_none()) {
        *missing = remaining.next();
    }
    matches
}

fn transfer(incoming: &mut MaterialSlot, previous: &MaterialSlot) {
    let tile = incoming.udim();
    let incoming_name = incoming.name(&Value::Null).map(str::to_owned);
    *incoming = previous.clone();
    if let Some(extras) = incoming.source["extras"].as_object_mut() {
        extras.remove("bevy_editor_udim");
    }
    if let Some(tile) = tile {
        if !incoming.source["extras"].is_object() {
            incoming.source["extras"] = json!({});
        }
        incoming.source["extras"]["bevy_editor_udim"] = json!(tile.number());
    }
    if let Some(name) = incoming_name {
        incoming.source["name"] = json!(name);
    }
    incoming.bake_fixes = default();
    if let Some(paint) = &mut incoming.paint {
        Arc::make_mut(paint).baked = None;
    }
}

fn create_material(slot: &mut MaterialSlot, materials: &mut Assets<StandardMaterial>) {
    let Some(authored) = &slot.authored else {
        return;
    };
    let pbr = &authored["pbrMetallicRoughness"];
    let [r, g, b, a] =
        serde_json::from_value::<[f32; 4]>(pbr["baseColorFactor"].clone()).unwrap_or([1.; 4]);
    let [er, eg, eb] =
        serde_json::from_value::<[f32; 3]>(authored["emissiveFactor"].clone()).unwrap_or([0.; 3]);
    slot.material = materials.add(StandardMaterial {
        base_color: Color::linear_rgba(r, g, b, a),
        metallic: pbr["metallicFactor"].as_f64().unwrap_or(0.) as f32,
        perceptual_roughness: pbr["roughnessFactor"].as_f64().unwrap_or(0.75) as f32,
        emissive: LinearRgba::rgb(er, eg, eb),
        cull_mode: None,
        alpha_mode: if a < 1. {
            AlphaMode::Blend
        } else {
            AlphaMode::Opaque
        },
        ..default()
    });
}

pub(in crate::editor) struct Report {
    pub transferred: usize,
    pub omitted: usize,
}

impl Document {
    pub fn replace_paint_mesh(
        &mut self,
        mut objects: Vec<Object>,
        use_active_set: bool,
        detect_tiles: bool,
        bake_on_load: bool,
        materials: &mut Assets<StandardMaterial>,
    ) -> Result<Report, String> {
        let mut before = self.snapshot();
        before.mesh_replacement = true;
        let first_id = self
            .objects
            .iter()
            .map(|object| object.id)
            .max()
            .unwrap_or(0)
            + 1;
        for (index, object) in objects.iter_mut().enumerate() {
            object.id = first_id + index;
            object.source_node = None;
            udim::import(object, detect_tiles);
        }
        let old: Vec<_> = before
            .objects
            .iter()
            .flat_map(|object| &object.materials)
            .collect();
        let old_names: Vec<_> = old
            .iter()
            .map(|slot| name(slot, &self.source.json))
            .collect();
        let new_names: Vec<_> = objects
            .iter()
            .flat_map(|object| &object.materials)
            .map(|slot| name(slot, &Value::Null))
            .collect();
        if new_names.is_empty() {
            return Err("У новой сетки нет наборов текстур".into());
        }
        let active = self.paint_set_index();
        let matches = assignments(&old_names, &new_names, use_active_set.then_some(active));
        let new_active = matches
            .iter()
            .position(|&matched| matched == Some(active))
            .unwrap_or(0);
        let transferred = matches.iter().flatten().count();
        for (incoming, matched) in objects
            .iter_mut()
            .flat_map(|object| &mut object.materials)
            .zip(matches)
        {
            if let Some(index) = matched {
                transfer(incoming, old[index]);
            } else {
                create_material(incoming, materials);
            }
        }
        self.objects = objects;
        if let Err(error) = self.validate() {
            self.restore(before);
            return Err(error);
        }
        self.bake_settings.cage = None;
        self.bake_settings.high_poly = None;
        self.cage_edits = default();
        self.auto_bake_pending = bake_on_load;
        self.paint_set = Some(new_active);
        if let Some((object, _)) = self.paint_target(new_active) {
            self.select_object(object.id, false);
        }
        let omitted = old.len() - transferred;
        self.checkpoint(before);
        Ok(Report {
            transferred,
            omitted,
        })
    }

    pub fn can_restore_paint_mesh(&self, redo: bool) -> bool {
        let history = if redo { &self.redo } else { &self.undo };
        history
            .last()
            .is_some_and(|snapshot| snapshot.mesh_replacement)
    }

    pub fn paint_sets_available_on_mesh_undo(&self) -> usize {
        self.undo
            .last()
            .filter(|snapshot| snapshot.mesh_replacement)
            .map_or(0, |before| {
                let count = |objects: &[Object]| {
                    objects
                        .iter()
                        .map(|object| object.materials.len())
                        .sum::<usize>()
                };
                count(&before.objects).saturating_sub(count(&self.objects))
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_named_tiles_before_using_remaining_sets_in_order() {
        let set = |name: &str, tile| SetName {
            name: name.into(),
            tile,
        };
        let old = [set("metal", 1001), set("wood", 1001), set("metal", 1013)];
        let new = [
            set("stone", 1001),
            set("metal", 1013),
            set("metal", 1001),
            set("extra", 1001),
        ];
        assert_eq!(
            assignments(&old, &new, None),
            [Some(1), Some(2), Some(0), None]
        );
        assert_eq!(assignments(&old, &[set("", 1001)], Some(2)), [Some(2)]);
    }

    #[test]
    fn every_replacement_primitive_has_uvs_in_its_texture_set() {
        for shape in Shape::ALL {
            let objects = primitive(shape).unwrap();
            let geometry = &objects[0].geometry;
            assert!(!geometry.faces.is_empty());
            assert!(
                geometry.faces.iter().all(|face| {
                    face.corners.iter().all(|corner| {
                        corner.uv.cmpge(Vec2::ZERO).all() && corner.uv.cmple(Vec2::ONE).all()
                    })
                }),
                "{shape:?}"
            );
            assert!(
                geometry.faces.iter().any(|face| {
                    let [a, b, c] = [0, 1, 2].map(|index| face.corners[index].uv);
                    (b - a).perp_dot(c - a).abs() > 1e-8
                }),
                "{shape:?}"
            );
        }
    }

    #[test]
    fn replacement_import_uses_material_names_and_separates_udim_tiles() {
        use crate::editor::model_import_mesh::{AutoUnwrap, replacement};
        let directory = std::env::temp_dir().join(format!(
            "bevy-paint-replacement-{}",
            bevy::asset::uuid::Uuid::new_v4()
        ));
        std::fs::create_dir(&directory).unwrap();
        let path = directory.join("replacement.obj");
        std::fs::write(directory.join("mesh.mtl"), "newmtl Brass\nKd 0.8 0.5 0.2\n").unwrap();
        std::fs::write(
            &path,
            "mtllib mesh.mtl\no Panel\n\
            v 0 0 0\nv 1 0 0\nv 0 1 0\nv 1 1 0\n\
            vt 0.1 0.1\nvt 0.9 0.1\nvt 0.1 0.9\n\
            vt 2.1 1.1\nvt 2.9 1.1\nvt 2.9 1.9\n\
            usemtl Brass\nf 1/1 2/2 3/3\nf 3/4 2/5 4/6\n",
        )
        .unwrap();
        let mut objects = replacement(&path, AutoUnwrap::Missing).unwrap();
        assert_eq!(objects.len(), 1);
        udim::import(&mut objects[0], true);
        let names: Vec<_> = objects[0]
            .materials
            .iter()
            .map(|slot| name(slot, &Value::Null))
            .collect();
        assert_eq!(
            names.iter().map(|name| name.tile).collect::<Vec<_>>(),
            [1001, 1013]
        );
        assert!(names.iter().all(|name| name.name == "brass"));
        std::fs::remove_dir_all(directory).unwrap();
    }
}
