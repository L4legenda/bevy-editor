use super::*;

pub(in crate::editor) fn cube() -> Geometry {
    let positions = vec![
        Vec3::new(-1., -1., -1.),
        Vec3::new(1., -1., -1.),
        Vec3::new(1., 1., -1.),
        Vec3::new(-1., 1., -1.),
        Vec3::new(-1., -1., 1.),
        Vec3::new(1., -1., 1.),
        Vec3::new(1., 1., 1.),
        Vec3::new(-1., 1., 1.),
    ];
    let faces = [
        [0, 3, 2, 1],
        [4, 5, 6, 7],
        [0, 1, 5, 4],
        [3, 7, 6, 2],
        [0, 4, 7, 3],
        [1, 2, 6, 5],
    ]
    .into_iter()
    .map(|v| Face {
        corners: v
            .into_iter()
            .enumerate()
            .map(|(i, v)| {
                let mut c = Corner::new(v);
                c.uv = [Vec2::ZERO, Vec2::X, Vec2::ONE, Vec2::Y][i];
                c
            })
            .collect(),
        material: 0,
        smooth: false,
    })
    .collect();
    Geometry {
        positions,
        faces,
        ..default()
    }
}
fn closed(mesh: &Geometry) {
    mesh.validate().unwrap();
    assert!(
        mesh.edges().values().all(|faces| faces.len() == 2),
        "Operation introduced a hole or a non-manifold edge"
    );
}
fn face(index: usize) -> Selection {
    Selection {
        kind: Element::Face,
        faces: [index].into(),
        ..default()
    }
}

#[test]
fn model_mesh_extrude_and_inset_keep_closed_topology_and_materials() {
    for inset in [false, true] {
        let mut mesh = cube();
        let mut selection = face(3);
        if inset {
            mesh.inset(&mut selection, 0.2).unwrap();
        } else {
            mesh.extrude(&mut selection, 0.5).unwrap();
        }
        assert_eq!(mesh.faces.len(), 10);
        assert_eq!(mesh.positions.len(), 12);
        closed(&mesh);
        assert!(mesh.faces.iter().all(|f| f.material == 0));
        assert_eq!(selection.faces.len(), 1);
    }
}
#[test]
fn model_mesh_subdivision_and_loop_cut_share_neighbor_vertices() {
    let mut mesh = cube();
    let mut selection = face(3);
    mesh.subdivide(&mut selection).unwrap();
    assert_eq!(mesh.faces.len(), 9);
    closed(&mesh);
    let mut mesh = cube();
    mesh.loop_cut(Edge::new(0, 1)).unwrap();
    assert_eq!(mesh.faces.len(), 10);
    closed(&mesh);
}
#[test]
fn model_mesh_triangles_quads_and_concave_ngons_render_without_missing_faces() {
    let mut mesh = cube();
    let mut selection = Selection {
        kind: Element::Face,
        ..default()
    };
    selection.all(&mesh);
    mesh.triangulate(&mut selection);
    assert_eq!(mesh.faces.len(), 12);
    mesh.dissolve(&mut selection, true).unwrap();
    assert_eq!(mesh.faces.len(), 6);
    closed(&mesh);
    let positions = vec![
        Vec3::new(0., 0., 0.),
        Vec3::new(2., 0., 0.),
        Vec3::new(2., 2., 0.),
        Vec3::new(1., 1., 0.),
        Vec3::new(0., 2., 0.),
    ];
    let face = Face {
        corners: (0..5).map(Corner::new).collect(),
        material: 0,
        smooth: false,
    };
    let mesh = Geometry {
        positions,
        faces: vec![face],
        ..default()
    };
    assert_eq!(mesh.triangles(&mesh.faces[0]).len(), 3);
    assert_eq!(mesh.buffers()[&0].indices.len(), 9);
}
#[test]
fn model_mesh_delete_fill_bisect_and_vertex_bevel_are_manifold() {
    let mut mesh = cube();
    mesh.delete(&mut face(3), true);
    assert_eq!(mesh.faces.len(), 5);
    mesh.fill(&mut Selection::default()).unwrap();
    assert_eq!(mesh.faces.len(), 6);
    closed(&mesh);
    let mut mesh = cube();
    mesh.bisect(Vec3::X, 0.0).unwrap();
    assert_eq!(mesh.faces.len(), 10);
    closed(&mesh);
    let mut mesh = cube();
    let mut selection = Selection::default();
    selection.all(&mesh);
    mesh.bevel_vertices(&mut selection, 0.2).unwrap();
    assert_eq!(mesh.positions.len(), 24);
    assert_eq!(mesh.faces.len(), 14);
    closed(&mesh);
}
#[test]
fn model_mesh_subdivision_preserves_uv_seams_and_proportional_motion_is_local() {
    let mut mesh = cube();
    let mut selected = face(3);
    mesh.subdivide(&mut selected).unwrap();
    assert!(mesh.faces.iter().all(|f| {
        f.corners
            .iter()
            .all(|c| c.uv.cmpge(Vec2::ZERO).all() && c.uv.cmple(Vec2::ONE).all())
    }));
    let mut mesh = cube();
    let selection = Selection {
        vertices: [0].into(),
        ..default()
    };
    let before = mesh.positions.clone();
    mesh.transform(
        &selection,
        Mat4::from_translation(Vec3::Y),
        Some((1.0, true)),
    );
    assert_eq!(mesh.positions[0], before[0] + Vec3::Y);
    assert_eq!(mesh.positions[6], before[6]);
}

#[test]
fn model_mesh_full_extrusion_has_no_ghost_vertices_and_uv_seams_survive_dissolve() {
    let mut mesh = cube();
    let mut selection = Selection {
        kind: Element::Face,
        ..default()
    };
    selection.all(&mesh);
    mesh.extrude(&mut selection, 0.2).unwrap();
    assert_eq!(
        mesh.positions.len(),
        8,
        "A closed-region extrusion must not leave unreferenced vertices in the viewport"
    );
    closed(&mesh);
    mesh.triangulate(&mut selection);
    let original = mesh.faces.len();
    // Deliberately make every face-corner chart discontinuous.
    for (i, f) in mesh.faces.iter_mut().enumerate() {
        for c in &mut f.corners {
            c.uv += Vec2::splat(i as f32 * 2.0);
        }
    }
    assert!(mesh.dissolve(&mut selection, true).is_err());
    assert_eq!(mesh.faces.len(), original);
    for scale in [0.00001, 10000.0] {
        let mut mesh = cube();
        for p in &mut mesh.positions {
            *p *= scale;
        }
        closed(&mesh);
    }
}

fn split_quad() -> Geometry {
    let positions = vec![Vec3::ZERO, Vec3::X, Vec3::X + Vec3::Y, Vec3::Y];
    let uv = [Vec2::ZERO, Vec2::X, Vec2::new(0.7, 0.8), Vec2::Y];
    Geometry {
        positions,
        loose_edges: default(),
        faces: [[0, 1, 2], [0, 2, 3]]
            .into_iter()
            .map(|vertices| Face {
                corners: vertices
                    .into_iter()
                    .map(|vertex| Corner {
                        vertex,
                        uv: uv[vertex],
                        uv1: uv[vertex] * 0.5,
                        color: Vec4::new(vertex as f32 / 4.0, 0.5, 0.7, 1.0),
                        source_normal: Some(Vec3::Z),
                    })
                    .collect(),
                material: 0,
                smooth: false,
            })
            .collect(),
    }
}

fn rendered_triangles(mesh: &Geometry) -> Vec<(usize, [[u32; 14]; 3])> {
    let mut triangles = Vec::new();
    for (material, buffers) in mesh.buffers() {
        for indices in buffers.indices.chunks_exact(3) {
            let mut triangle: [[u32; 14]; 3] = std::array::from_fn(|corner| {
                let i = indices[corner] as usize;
                buffers.positions[i]
                    .into_iter()
                    .chain(buffers.normals[i])
                    .chain(buffers.uv[i])
                    .chain(buffers.uv1[i])
                    .chain(buffers.colors[i])
                    .map(f32::to_bits)
                    .collect::<Vec<_>>()
                    .try_into()
                    .unwrap()
            });
            let first = (0..3).min_by_key(|&i| triangle[i]).unwrap();
            triangle.rotate_left(first);
            triangles.push((material, triangle));
        }
    }
    triangles.sort_unstable();
    triangles
}

#[test]
fn model_mesh_restored_quad_preserves_the_rendered_triangles_and_attributes() {
    let mut mesh = split_quad();
    let before = rendered_triangles(&mesh);
    let mut selection = face(0);
    selection.all(&mesh);
    mesh.dissolve(&mut selection, true).unwrap();
    mesh.validate().unwrap();
    assert_eq!(mesh.faces.len(), 1);
    assert_eq!(mesh.faces[0].corners.len(), 4);
    assert_eq!(mesh.edges().len(), 4);
    assert_eq!(rendered_triangles(&mesh), before);
    assert_eq!(selection.faces, [0].into());
    mesh.extrude(&mut selection, 0.5).unwrap();
    assert_eq!(
        mesh.faces.len(),
        5,
        "Extrusion must operate on the whole quad"
    );
}

#[test]
fn model_mesh_quad_join_preserves_seams_creases_and_concave_boundaries() {
    for boundary in 0..8 {
        let mut mesh = split_quad();
        match boundary {
            0 => mesh.faces[1].material = 1,
            1 => mesh.faces[1].corners[0].uv += Vec2::X,
            2 => mesh.faces[1].corners[0].uv1 += Vec2::Y,
            3 => mesh.faces[1].corners[0].color.x += 0.1,
            4 => mesh.faces[1].corners[0].source_normal = Some(Vec3::X),
            5 => mesh.faces[1].smooth = true,
            6 => mesh.positions[3].z = 1.5,
            _ => mesh.positions[2] = Vec3::new(0.25, 0.25, 0.0),
        }
        mesh.validate().unwrap();
        let before = mesh.clone();
        let mut selection = face(0);
        selection.all(&mesh);
        assert!(
            mesh.dissolve(&mut selection, true).is_err(),
            "Boundary {boundary}"
        );
        assert_eq!(
            mesh, before,
            "Boundary {boundary} must remain editable as triangles"
        );
    }
}

#[test]
fn model_mesh_bent_quad_joins_flat_shaded_triangles_without_flattening_vertices() {
    let mut mesh = split_quad();
    mesh.positions[3].z = 0.25;
    let positions = mesh.positions.clone();
    let normals: Vec<_> = mesh
        .faces
        .iter()
        .map(|face| mesh.normal(face).normalize())
        .collect();
    for (face, normal) in mesh.faces.iter_mut().zip(normals) {
        for corner in &mut face.corners {
            corner.source_normal = Some(normal);
        }
    }
    let mut selection = face(0);
    selection.all(&mesh);
    mesh.dissolve(&mut selection, true).unwrap();
    mesh.validate().unwrap();
    assert_eq!(
        mesh.positions, positions,
        "Reconstructing a quad must preserve the edited shape"
    );
    assert_eq!(mesh.faces.len(), 1);
    assert_eq!(mesh.faces[0].corners.len(), 4);
    assert_eq!(mesh.edges().len(), 4);
    assert!(
        mesh.faces[0]
            .corners
            .iter()
            .all(|c| c.source_normal.is_none())
    );
}
