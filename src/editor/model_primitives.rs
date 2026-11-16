//! Standard mesh assets created from the file browser.
use super::{model_document, model_mesh::Geometry};
use bevy::{mesh::MeshBuilder, prelude::*};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Primitive {
    Plane,
    Cube,
    Circle,
    UvSphere,
    IcoSphere,
    Cylinder,
    Cone,
    Torus,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Parameter {
    SizeX,
    SizeY,
    SizeZ,
    Radius,
    TopRadius,
    Segments,
    Rings,
    Subdivisions,
    MajorRadius,
    MinorRadius,
}

impl Parameter {
    pub const ALL: [Self; 10] = [
        Self::SizeX,
        Self::SizeY,
        Self::SizeZ,
        Self::Radius,
        Self::TopRadius,
        Self::Segments,
        Self::Rings,
        Self::Subdivisions,
        Self::MajorRadius,
        Self::MinorRadius,
    ];

    pub fn label(self, kind: Primitive) -> &'static str {
        match self {
            Self::SizeX => "Размер X, м",
            Self::SizeY if matches!(kind, Primitive::Cylinder | Primitive::Cone) => "Высота, м",
            Self::SizeY => "Размер Y, м",
            Self::SizeZ => "Размер Z, м",
            Self::Radius if kind == Primitive::Cone => "Радиус основания, м",
            Self::Radius => "Радиус, м",
            Self::TopRadius => "Верхний радиус, м",
            Self::Segments if kind == Primitive::UvSphere => "Меридианы",
            Self::Segments if kind == Primitive::Torus => "Сегменты кольца",
            Self::Segments => "Сегменты",
            Self::Rings if kind == Primitive::Torus => "Сегменты сечения",
            Self::Rings => "Кольца",
            Self::Subdivisions => "Подразделения",
            Self::MajorRadius => "Большой радиус, м",
            Self::MinorRadius => "Малый радиус, м",
        }
    }

    pub fn integer(self) -> bool {
        matches!(self, Self::Segments | Self::Rings | Self::Subdivisions)
    }

    pub fn range(self, kind: Primitive) -> (f32, f32) {
        match self {
            Self::Segments => (3.0, 256.0),
            Self::Rings => (if kind == Primitive::Torus { 3.0 } else { 2.0 }, 256.0),
            Self::Subdivisions => (1.0, 5.0),
            Self::TopRadius => (0.0, 10000.0),
            _ => (0.0001, 10000.0),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Parameters {
    pub size: Vec3,
    pub radius: f32,
    pub top_radius: f32,
    pub segments: u32,
    pub rings: u32,
    pub subdivisions: u32,
    pub major_radius: f32,
    pub minor_radius: f32,
}

impl Parameters {
    pub const MAX_CELLS: u32 = 16_384;

    pub fn get(&self, field: Parameter) -> f32 {
        match field {
            Parameter::SizeX => self.size.x,
            Parameter::SizeY => self.size.y,
            Parameter::SizeZ => self.size.z,
            Parameter::Radius => self.radius,
            Parameter::TopRadius => self.top_radius,
            Parameter::Segments => self.segments as f32,
            Parameter::Rings => self.rings as f32,
            Parameter::Subdivisions => self.subdivisions as f32,
            Parameter::MajorRadius => self.major_radius,
            Parameter::MinorRadius => self.minor_radius,
        }
    }

    pub fn set(&mut self, field: Parameter, value: f32) {
        match field {
            Parameter::SizeX => self.size.x = value,
            Parameter::SizeY => self.size.y = value,
            Parameter::SizeZ => self.size.z = value,
            Parameter::Radius => self.radius = value,
            Parameter::TopRadius => self.top_radius = value,
            Parameter::Segments => self.segments = value as u32,
            Parameter::Rings => self.rings = value as u32,
            Parameter::Subdivisions => self.subdivisions = value as u32,
            Parameter::MajorRadius => self.major_radius = value,
            Parameter::MinorRadius => self.minor_radius = value,
        }
    }

    pub fn validate(&self, kind: Primitive) -> Result<(), String> {
        for &field in kind.fields() {
            let value = self.get(field);
            let (min, max) = field.range(kind);
            if !value.is_finite() || value < min || value > max {
                return Err(format!(
                    "{}: допустимо от {min} до {max}.",
                    field.label(kind)
                ));
            }
        }
        if kind == Primitive::Torus && self.major_radius <= self.minor_radius {
            return Err("Большой радиус тора должен быть больше малого радиуса.".into());
        }
        if matches!(kind, Primitive::UvSphere | Primitive::Torus)
            && u64::from(self.segments) * u64::from(self.rings) > u64::from(Self::MAX_CELLS)
        {
            return Err("Слишком высокая детализация: уменьшите число сегментов или колец.".into());
        }
        Ok(())
    }
}

impl Primitive {
    pub const ALL: [Self; 8] = [
        Self::Plane,
        Self::Cube,
        Self::Circle,
        Self::UvSphere,
        Self::IcoSphere,
        Self::Cylinder,
        Self::Cone,
        Self::Torus,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Plane => "Плоскость",
            Self::Cube => "Куб",
            Self::Circle => "Круг",
            Self::UvSphere => "UV-сфера",
            Self::IcoSphere => "Икосфера",
            Self::Cylinder => "Цилиндр",
            Self::Cone => "Конус",
            Self::Torus => "Тор",
        }
    }

    pub fn fields(self) -> &'static [Parameter] {
        use Parameter::*;
        match self {
            Self::Plane => &[SizeX, SizeZ],
            Self::Cube => &[SizeX, SizeY, SizeZ],
            Self::Circle => &[Radius, Segments],
            Self::UvSphere => &[Radius, Segments, Rings],
            Self::IcoSphere => &[Radius, Subdivisions],
            Self::Cylinder => &[Radius, SizeY, Segments],
            Self::Cone => &[Radius, TopRadius, SizeY, Segments],
            Self::Torus => &[MajorRadius, MinorRadius, Segments, Rings],
        }
    }

    pub fn defaults(self) -> Parameters {
        Parameters {
            size: Vec3::splat(2.0),
            radius: 1.0,
            top_radius: 0.0,
            segments: 32,
            rings: if self == Self::Torus { 24 } else { 16 },
            subdivisions: 2,
            major_radius: 1.0,
            minor_radius: 0.25,
        }
    }

    /// Shared surface for the live preview and the editable asset.
    pub fn mesh(self, p: &Parameters) -> Result<Mesh, String> {
        p.validate(self)?;
        let mut mesh = match self {
            Self::Plane => Plane3d::default().mesh().size(p.size.x, p.size.z).build(),
            Self::Cube => Mesh::from(Cuboid::from_size(p.size)),
            Self::Circle => Circle::new(p.radius).mesh().resolution(p.segments).build(),
            Self::UvSphere => Sphere::new(p.radius).mesh().uv(p.segments, p.rings),
            Self::IcoSphere => Sphere::new(p.radius)
                .mesh()
                .ico((1 << (p.subdivisions - 1)) - 1)
                .map_err(|e| e.to_string())?,
            Self::Cylinder => Cylinder::new(p.radius, p.size.y)
                .mesh()
                .resolution(p.segments)
                .build(),
            Self::Cone if p.top_radius == 0.0 => Cone::new(p.radius, p.size.y)
                .mesh()
                .resolution(p.segments)
                .build(),
            Self::Cone => ConicalFrustum {
                radius_top: p.top_radius,
                radius_bottom: p.radius,
                height: p.size.y,
            }
            .mesh()
            .resolution(p.segments)
            .build(),
            Self::Torus => Torus::new(
                p.major_radius - p.minor_radius,
                p.major_radius + p.minor_radius,
            )
            .mesh()
            .major_resolution(p.segments as usize)
            .minor_resolution(p.rings as usize)
            .build(),
        };
        if self == Self::Circle {
            mesh.rotate_by(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2));
        }
        if self == Self::IcoSphere {
            mesh.duplicate_vertices();
            mesh.compute_flat_normals();
        }
        Ok(mesh)
    }

    pub fn geometry(self, p: &Parameters) -> Result<Geometry, String> {
        let mesh = self.mesh(p)?;
        let mut geometry = Geometry::default();
        model_document::from_mesh(&mesh, &mut geometry, 0)?;
        if self == Self::IcoSphere {
            for face in &mut geometry.faces {
                face.smooth = false;
                for corner in &mut face.corners {
                    corner.source_normal = None;
                }
            }
        }
        geometry.validate()?;
        Ok(geometry)
    }
}
