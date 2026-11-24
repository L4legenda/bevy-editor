use super::*;
use super::bevel::Miter;

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

#[test]
fn extruding_either_end_of_a_loose_edge_extends_away_from_its_neighbor() {
    for (vertex, expected) in [(0, -Vec3::X * 0.5), (1, Vec3::X * 1.5)] {
        let mut mesh = Geometry {
            positions: vec![Vec3::ZERO, Vec3::X],
            loose_edges: [Edge::new(0, 1)].into(),
            ..default()
        };
        let mut selection = Selection {
            kind: Element::Vertex,
            vertices: [vertex].into(),
            ..default()
        };

        mesh.extrude(&mut selection, 0.5).unwrap();

        assert_eq!(mesh.positions[2], expected);
        assert_eq!(mesh.loose_edges.len(), 2);
        assert!(mesh.loose_edges.contains(&Edge::new(vertex, 2)));
        assert_eq!(selection.vertices, [2].into());
    }
}

#[test]
fn extruding_connected_loose_edges_keeps_both_new_faces() {
    let mut mesh = Geometry {
        positions: vec![Vec3::X, Vec3::ZERO, Vec3::Y],
        loose_edges: [Edge::new(0, 1), Edge::new(1, 2)].into(),
        ..default()
    };
    let mut selection = Selection {
        kind: Element::Edge,
        edges: mesh.loose_edges.clone(),
        ..default()
    };

    mesh.extrude(&mut selection, 0.5).unwrap();

    mesh.validate().unwrap();
    assert_eq!(mesh.positions.len(), 6);
    assert_eq!(mesh.faces.len(), 2);
    assert_eq!(selection.edges, [Edge::new(3, 4), Edge::new(4, 5)].into());
    assert_eq!(mesh.edges()[&Edge::new(1, 4)].len(), 2);
}

#[test]
fn extruding_vertices_keeps_connected_strips_and_extends_isolated_points() {
    let mut mesh = Geometry {
        positions: vec![Vec3::ZERO, Vec3::X, Vec3::Y, Vec3::splat(2.0)],
        loose_edges: [Edge::new(0, 1), Edge::new(0, 2)].into(),
        ..default()
    };
    let mut selection = Selection {
        kind: Element::Vertex,
        vertices: [0, 1, 2, 3].into(),
        ..default()
    };

    mesh.extrude(&mut selection, 0.5).unwrap();

    mesh.validate().unwrap();
    assert_eq!(mesh.positions.len(), 8);
    assert_eq!(mesh.faces.len(), 2);
    assert_eq!(mesh.edges()[&Edge::new(0, 4)].len(), 2);
    assert_eq!(mesh.loose_edges, [Edge::new(3, 7)].into());
    assert_eq!(selection.kind, Element::Vertex);
    assert_eq!(selection.vertices, [4, 5, 6, 7].into());
}

#[test]
fn loop_cut_preview_matches_asymmetric_and_multiple_cuts() {
    for (count, position) in [(1, 0.25), (1, 0.75), (3, 0.5)] {
        let mut mesh = cube();
        mesh.uv_seams.insert(Edge::new(0, 1));
        let cut = LoopCut::find(&mesh, Edge::new(0, 1)).unwrap();
        let options = LoopCutOptions { count, position, ..default() };
        let preview = cut.preview(&mesh, options);
        let original = mesh.positions.len();
        let selected = cut.apply(&mut mesh, options).unwrap();
        assert_eq!(selected.len(), 4 * count);
        assert_eq!(mesh.positions.len(), original + 4 * count);
        assert_eq!(mesh.uv_seams.len(), count + 1);
        for [a,b] in preview {
            assert!(selected.iter().any(|edge| {
                let [x,y] = [mesh.positions[edge.0], mesh.positions[edge.1]];
                (a == x && b == y) || (a == y && b == x)
            }));
        }
        if count == 1 {
            assert!(mesh.positions[original..].iter().all(|p| p.x == -1.0 + 2.0 * position));
        }
        closed(&mesh);
    }
}

#[test]
fn loop_cut_even_is_measured_from_the_selected_side_and_can_correct_uvs() {
    let mut original = cube();
    original.positions[1].x = 2.0;
    original.positions[5].x = 2.0;
    original.faces[0].material = 3;
    let cut = LoopCut::find(&original, Edge::new(0,1)).unwrap();
    let options = LoopCutOptions { position: 0.5, even: true, ..default() };
    let mut left = original.clone();
    let mut right = original.clone();
    let mut uncorrected = original.clone();
    cut.apply(&mut left, options).unwrap();
    cut.apply(&mut right, LoopCutOptions { flipped: true, ..options }).unwrap();
    cut.apply(&mut uncorrected, LoopCutOptions { correct_uv: false, ..options }).unwrap();
    assert!(left.positions[8..].iter().all(|p| (p.x-0.5).abs() < 1.0e-6));
    assert!(right.positions[8..].iter().any(|p|(p.x+0.5).abs() < 1.0e-6));
    assert_eq!(left.positions, uncorrected.positions);
    assert_ne!(left.faces, uncorrected.faces);
    let top = left.positions.iter().position(|point| *point == Vec3::new(0.5, 1.0, -1.0)).unwrap();
    let uv_at_top = |mesh: &Geometry| {
        mesh.faces.iter().filter(|face| face.material == 3)
            .flat_map(|face| &face.corners).find(|corner| corner.vertex == top).unwrap().uv
    };
    let checker = |uv: Vec2| ((uv.x * 4.0).floor() as i32 + (uv.y * 4.0).floor() as i32) % 2;
    assert_eq!(uv_at_top(&left), Vec2::new(1.0, 0.75));
    assert_eq!(checker(uv_at_top(&left)), checker(Vec2::new(1.0, 0.75)));
    assert_ne!(checker(uv_at_top(&left)), checker(uv_at_top(&uncorrected)));
    closed(&left);
    closed(&right);
}

#[test]
fn loop_cut_ends_at_a_triangle_without_detaching_the_boundary() {
    let mut mesh = cube();
    mesh.faces.truncate(1);
    mesh.positions.push(Vec3::new(0.0, 2.0, -1.0));
    mesh.faces.push(Face {
        corners: [2,3,8].into_iter().map(Corner::new).collect(),
        material: 1, smooth: false,
    });
    mesh.compact();
    let cut = LoopCut::find(&mesh, Edge::new(0,1)).unwrap();
    let edges = cut.apply(&mut mesh, LoopCutOptions {count:3,..default()}).unwrap();
    assert_eq!(edges.len(),3);
    assert_eq!(mesh.faces.len(),5);
    assert_eq!(mesh.faces.iter().find(|f|f.material==1).unwrap().corners.len(),6);
    mesh.validate().unwrap();
}

#[test]
fn knife_reuses_a_cut_point_between_adjacent_faces() {
    let mut mesh = cube();
    let common = Vec3::new(0.0,-1.0,-1.0);
    let cuts = mesh.apply_knife(&[
        KnifeSegment { face: 0, ends: [Vec3::new(0.0,1.0,-1.0), common] },
        KnifeSegment { face: 2, ends: [common,Vec3::new(0.0,-1.0,1.0)] },
    ]).unwrap();
    assert_eq!(cuts.len(),2);
    let shared = mesh.positions.iter().position(|p| *p==common).unwrap();
    assert!(cuts.iter().all(|edge|edge.0==shared || edge.1==shared));
    closed(&mesh);
}

#[test]
fn knife_keeps_the_outline_of_a_concave_face() {
    let point = |x,y| Vec3::new(x,y,0.0);
    let mut mesh = Geometry {
        positions: [(-2.0,-1.0),(2.0,-1.0),(2.0,1.0),(1.0,1.0),(1.0,0.0),(-1.0,0.0),(-1.0,1.0),(-2.0,1.0)]
            .map(|(x,y)| point(x,y)).into(),
        faces: vec![Face { corners: (0..8).map(Corner::new).collect(), material: 4, smooth: false }],
        ..default()
    };
    let area = mesh.normal(&mesh.faces[0]).z;
    let cuts = mesh.apply_knife(&[
        KnifeSegment { face: 0, ends: [point(-2.0,0.5),point(-1.0,0.5)] },
        KnifeSegment { face: 0, ends: [point(1.0,0.5),point(2.0,0.5)] },
    ]).unwrap();
    assert_eq!(cuts.len(),2);
    assert_eq!(mesh.faces.len(),3);
    assert!(mesh.faces.iter().all(|face| face.material == 4));
    assert_eq!(mesh.faces.iter().map(|face| mesh.normal(face).z).sum::<f32>(), area);
}

#[test]
fn knife_subdivides_only_the_target_face_and_keeps_boundary_watertight() {
    let mut mesh = cube();
    mesh.faces[0].material = 5;
    mesh.uv_seams.insert(Edge::new(0, 3));
    let cuts = mesh.apply_knife(&[KnifeSegment {
        face: 0, ends: [Vec3::new(-1.0, 0.0, -1.0), Vec3::new(1.0, 0.0, -1.0)],
    }]).unwrap();
    assert_eq!(cuts.len(), 1);
    assert_eq!(mesh.faces.len(), 7);
    assert_eq!(mesh.faces.iter().filter(|face| face.material == 5).count(), 2);
    assert_eq!(mesh.uv_seams.len(), 2);
    closed(&mesh);
}

#[test]
fn knife_resolves_crossing_paths_and_closed_internal_contours() {
    let mut mesh = cube();
    let point = |x,y| Vec3::new(x,y,-1.0);
    let cuts = mesh.apply_knife(&[
        KnifeSegment { face: 0, ends: [point(-0.6,0.0),point(0.6,0.0)] },
        KnifeSegment { face: 0, ends: [point(0.0,-0.6),point(0.0,0.6)] },
    ]).unwrap();
    assert_eq!(cuts.len(), 4);
    assert_eq!(cuts.iter().filter(|edge| {
        [edge.0,edge.1].iter().any(|&v|mesh.positions[v].abs_diff_eq(point(0.0,0.0),1.0e-5))
    }).count(),4);
    closed(&mesh);

    let mut mesh = cube();
    let path = [point(-0.5,-0.5),point(0.5,-0.5),point(0.5,0.5),point(-0.5,0.5)];
    let segments: Vec<_> = path.iter().zip(path.iter().cycle().skip(1))
        .map(|(&a,&b)| KnifeSegment { face: 0, ends: [a,b] }).collect();
    assert_eq!(mesh.apply_knife(&segments).unwrap().len(),4);
    closed(&mesh);
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
fn edge_loop_and_slide_follow_consistent_rails_on_a_cut_cube() {
    let mut mesh = cube();
    let cut = mesh.loop_cut(Edge::new(0, 1)).unwrap();
    assert_eq!(cut.len(), 4);
    assert_eq!(mesh.edge_loops(&[*cut.first().unwrap()].into()), cut);
    let before = mesh.positions.clone();
    let selected: BTreeSet<_> = cut.iter().flat_map(|e| [e.0, e.1]).collect();
    mesh.slide_edges(&cut, 0.25).unwrap();
    let delta = mesh.positions[*selected.first().unwrap()] - before[*selected.first().unwrap()];
    assert_eq!(delta.x.abs(), 0.25);
    for (i, (&position, old)) in mesh.positions.iter().zip(before).enumerate() {
        assert_eq!(position - old, if selected.contains(&i) { delta } else { Vec3::ZERO });
    }
    closed(&mesh);
}

#[test]
fn transform_pivot_can_differ_from_the_selected_vertices_center() {
    use crate::editor::model_document::{Document, Mode, Object, Pivot, glb::Glb};

    let asset = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/legacy/assets/models/block.glb");
    let mut doc = Document::new(
        Glb::load(&asset).unwrap(),
        vec![Object {
            id: 0,
            source_node: None,
            name: "Куб".into(),
            geometry: cube().into(),
            transform: Transform::from_xyz(10.0, 0.0, 0.0),
            materials: Vec::new(),
            hidden: false,
        }],
    );
    doc.mode = Mode::Edit;
    doc.selection.vertices = [0, 1, 2].into();
    let center = Vec3::new(10.0 + 1.0 / 3.0, -1.0 / 3.0, -1.0);
    assert!(doc.pivot().unwrap().abs_diff_eq(center, 0.00001));
    doc.pivot_mode = Pivot::Bounds;
    assert_eq!(doc.pivot(), Some(Vec3::new(10.0, 0.0, -1.0)));
    doc.pivot_mode = Pivot::ObjectOrigin;
    assert_eq!(doc.pivot(), Some(Vec3::X * 10.0));
    doc.pivot_mode = Pivot::WorldOrigin;
    assert_eq!(doc.pivot(), Some(Vec3::ZERO));
    assert!(doc.selection_center().unwrap().abs_diff_eq(center, 0.00001));
    doc.selection.clear();
    assert_eq!(doc.pivot(), None);
}

#[test]
fn edge_bevel_bridges_a_single_edge_and_caps_corners_of_complete_selection() {
    let mut mesh = cube();
    mesh.faces[4].material = 7;
    let selected = [Edge::new(0, 1)].into();
    let bevel = mesh.bevel_edges(&selected, 0.25).unwrap();
    assert_eq!(bevel.len(), 1);
    assert_eq!(mesh.faces[4].material, 7);
    let strip = &mesh.faces[*bevel.first().unwrap()];
    assert_eq!(strip.corners.len(), 4);
    assert!((mesh.positions[strip.corners[0].vertex].y + 0.75).abs() < 0.00001);
    closed(&mesh);

    let mut mesh = cube();
    let selected = mesh.edges().into_keys().collect();
    let bevel = mesh.bevel_edges(&selected, 0.25).unwrap();
    assert_eq!(bevel.len(), 12 + 8);
    assert_eq!(mesh.positions.len(), 24);
    closed(&mesh);
}

#[test]
fn edge_bevel_width_modes_and_profile_build_closed_meshes() {
    let edge = Edge::new(0, 1);
    let selected = [edge].into();
    for (mode, amount) in [
        (WidthType::Offset, 0.25),
        (WidthType::Absolute, 0.25),
        (WidthType::Percent, 12.5),
        (WidthType::Width, 0.25 * std::f32::consts::SQRT_2),
        (WidthType::Depth, 0.25 / std::f32::consts::SQRT_2),
    ] {
        let mut mesh = cube();
        let bevel = mesh.bevel_edges_with(&selected, BevelOptions {
            amount, width_type: mode, ..default()
        }).unwrap();
        let corner = &mesh.faces[*bevel.first().unwrap()].corners[0];
        assert!((mesh.positions[corner.vertex].y + 0.75).abs() < 0.00001, "{mode:?}");
        closed(&mesh);
    }
    let mut rounded = cube();
    let mut flat = rounded.clone();
    let base = BevelOptions { amount: 0.25, segments: 4, ..default() };
    let rounded_faces = rounded.bevel_edges_with(&selected, base).unwrap();
    let flat_faces = flat.bevel_edges_with(&selected, BevelOptions { profile: 0.25, ..base }).unwrap();
    assert_eq!(rounded_faces.len(), 6);
    assert_eq!(flat_faces.len(), 4);
    assert_ne!(rounded.positions, flat.positions);
    closed(&rounded);
    closed(&flat);
    let mut cube = cube();
    let all = cube.edges().into_keys().collect();
    cube.bevel_edges_with(&all, base).unwrap();
    closed(&cube);
}

#[test]
fn edge_bevel_samples_custom_profile_and_solves_shared_rail_offsets() {
    let mut mesh = cube();
    let options = BevelOptions {
        amount: 0.25,
        segments: 4,
        custom_profile: Some([
            Vec2::new(0.75, 0.08),
            Vec2::new(0.5, 0.18),
            Vec2::new(0.25, 0.3),
        ]),
        ..default()
    };
    let bevel = mesh.bevel_edges_with(&[Edge::new(0, 1)].into(), options).unwrap();
    let strip = &mesh.faces[bevel.first().copied().unwrap() + 1];
    let midpoint = mesh.positions[strip.corners[2].vertex];
    let displacement = midpoint - Vec3::splat(-1.0);
    let steps = Vec2::new(displacement.y, displacement.z);
    let expected = Vec2::new(0.5, 0.18) * options.amount;
    assert!(steps.abs_diff_eq(expected, 0.00001) || steps.yx().abs_diff_eq(expected, 0.00001));
    closed(&mesh);

    let mut slid = cube();
    for p in &mut slid.positions {
        p.x += 0.4 * (p.z + 1.0);
    }
    let mut even = slid.clone();
    let selected = [Edge::new(0, 1), Edge::new(0, 3)].into();
    slid.bevel_edges_with(&selected, BevelOptions { amount: 0.25, ..default() }).unwrap();
    even.bevel_edges_with(&selected, BevelOptions {
        amount: 0.25, loop_slide: false, ..default()
    }).unwrap();
    assert_ne!(even.positions, slid.positions);
    closed(&slid);
    closed(&even);
}

#[test]
fn edge_bevel_builds_inner_arcs_and_outer_patches_and_arcs() {
    let options = BevelOptions { amount: 0.1, segments: 3, ..default() };
    let mut sharp = cube();
    let mut arc = sharp.clone();
    let selected = [Edge::new(0, 1), Edge::new(0, 3)].into();
    sharp.bevel_edges_with(&selected, options).unwrap();
    arc.bevel_edges_with(&selected, BevelOptions { inner_miter: Miter::Arc, ..options }).unwrap();
    assert_eq!(arc.faces[0].corners.len(), sharp.faces[0].corners.len() + 3);
    closed(&arc);

    let outline = [(0., 0.), (2., 0.), (2., 1.), (1., 1.), (1., 2.), (0., 2.)];
    let positions = [0., 1.].into_iter().flat_map(|z| {
        outline.into_iter().map(move |(x, y)| Vec3::new(x, y, z))
    }).collect();
    let face = |vertices: Vec<usize>| Face {
        corners: vertices.into_iter().map(Corner::new).collect(),
        material: 0,
        smooth: false,
    };
    let mut faces = vec![face((0..6).rev().collect()), face((6..12).collect())];
    for a in 0..6 {
        let b = (a + 1) % 6;
        faces.push(face(vec![a, b, b + 6, a + 6]));
    }
    let shape = Geometry { positions, faces, ..default() };
    closed(&shape);
    let selected = [Edge::new(8, 9), Edge::new(9, 10)].into();
    let mut results = Vec::new();
    for miter in [Miter::Sharp, Miter::Patch, Miter::Arc] {
        let mut mesh = shape.clone();
        mesh.bevel_edges_with(&selected, BevelOptions { outer_miter: miter, ..options }).unwrap();
        closed(&mesh);
        results.push(mesh.faces[1].corners.len());
    }
    assert_eq!(results, [6, 8, 9]);
}

#[test]
fn edge_bevel_grid_and_cutoff_use_different_closed_junctions() {
    let mut grid = cube();
    let mut cutoff = grid.clone();
    let selected = grid.edges().into_keys().collect();
    let options = BevelOptions { amount: 0.2, segments: 3, ..default() };
    grid.bevel_edges_with(&selected, options).unwrap();
    cutoff.bevel_edges_with(&selected, BevelOptions {
        intersection: Intersection::Cutoff,
        ..options
    }).unwrap();
    let cap_start = 6 + 12 * 3;
    assert_eq!(grid.faces[cap_start..].iter().filter(|f| f.corners.len() == 4).count(), 8 * 9);
    assert_eq!(cutoff.faces[cap_start..].iter().filter(|f| f.corners.len() == 6).count(), 8 * 3);
    assert_eq!(cutoff.faces[cap_start..].iter().filter(|f| f.corners.len() == 3).count(), 8);
    closed(&grid);
    closed(&cutoff);
}

#[test]
fn edge_bevel_rounds_an_interior_edge_on_an_open_mesh() {
    let mut mesh = cube();
    mesh.faces = [mesh.faces[0].clone(), mesh.faces[2].clone()].into();
    mesh.compact();
    let affected = mesh.bevel_edges_with(&[Edge::new(0, 1)].into(), BevelOptions {
        amount: 0.25,
        segments: 3,
        ..default()
    }).unwrap();
    assert_eq!(affected.len(), 3);
    let adjacency = mesh.edges();
    assert!(adjacency.values().all(|faces| (1..=2).contains(&faces.len())));
    assert_eq!(adjacency.values().filter(|faces| faces.len() == 1).count(), 12);
    mesh.validate().unwrap();
}

#[test]
fn edge_bevel_reaches_the_far_side_of_a_cube() {
    let selected = [Edge::new(0, 1)].into();
    for segments in [1, 3] {
        let mut mesh = cube();
        let affected = mesh.bevel_edges_with(&selected, BevelOptions {
            amount: 3.0,
            segments,
            ..default()
        }).unwrap();
        let vertices: Vec<_> = affected.iter()
            .flat_map(|&i| mesh.faces[i].corners.iter().map(|c| mesh.positions[c.vertex]))
            .collect();
        assert!(vertices.iter().all(|p| p.y <= 1.0 && p.z <= 1.0));
        assert!(vertices.iter().any(|p| p.y == 1.0));
        assert!(vertices.iter().any(|p| p.z == 1.0));
        assert_eq!(mesh.faces.len(), if segments == 1 { 5 } else { 9 });
        closed(&mesh);
    }
}

#[test]
fn edge_bevel_stops_where_two_bevels_meet() {
    let mut mesh = cube();
    let selected = [Edge::new(0, 1), Edge::new(2, 3)].into();
    let affected = mesh.bevel_edges_with(&selected, BevelOptions {
        amount: 3.0,
        ..default()
    }).unwrap();
    let endpoints: Vec<_> = affected.iter()
        .flat_map(|&i| mesh.faces[i].corners.iter().map(|c| mesh.positions[c.vertex]))
        .filter(|p| p.z == -1.0)
        .collect();
    assert!(!endpoints.is_empty());
    assert!(endpoints.iter().all(|p| p.y.abs() < 0.000001));
    assert_eq!(mesh.faces.len(), 7);
    closed(&mesh);
}

#[test]
fn vertex_bevel_builds_profile_caps_and_handles_an_open_boundary() {
    let mut mesh = cube();
    mesh.uv_seams.insert(Edge::new(0, 1));
    let affected = mesh.bevel_vertices_with(&[0].into(), BevelOptions {
        amount: 12.5,
        width_type: WidthType::Percent,
        segments: 3,
        preserve_uv_seams: true,
        material: Some(1),
        ..default()
    }).unwrap();
    assert_eq!(affected.len(), 10);
    assert!(affected.iter().all(|&fi| mesh.faces[fi].material == 1));
    assert_eq!(mesh.uv_seams.len(), 1);
    assert!(mesh.positions.iter().any(|point| point.abs_diff_eq(
        Vec3::new(-0.75, -1.0, -1.0), 0.00001
    )));
    closed(&mesh);

    let mut plane = cube();
    plane.faces.truncate(1);
    plane.compact();
    let affected = plane.bevel_vertices_with(&[0].into(), BevelOptions {
        amount: 0.2,
        segments: 4,
        ..default()
    }).unwrap();
    assert_eq!(affected, [0].into());
    assert_eq!(plane.faces[0].corners.len(), 8);
    assert_eq!(plane.triangles(&plane.faces[0]).len(), 6);
    plane.validate().unwrap();
}

#[test]
fn edge_bevel_applies_material_and_preserves_seams_and_split_normals() {
    let mut mesh = cube();
    let edge = Edge::new(0, 1);
    mesh.uv_seams.insert(edge);
    for fi in [0, 2] {
        let normal = mesh.normal(&mesh.faces[fi]).normalize();
        for corner in &mut mesh.faces[fi].corners {
            corner.source_normal = Some(normal);
        }
    }
    let affected = mesh.bevel_edges_with(&[edge].into(), BevelOptions {
        amount: 0.2,
        segments: 3,
        preserve_uv_seams: true,
        preserve_sharp: true,
        harden_normals: true,
        material: Some(3),
        ..default()
    }).unwrap();
    assert!(affected.iter().all(|&fi| mesh.faces[fi].material == 3));
    assert_eq!(mesh.uv_seams.len(), 2);
    for fi in [0, 2] {
        let face = &mesh.faces[fi];
        let normal = mesh.normal(face).normalize();
        assert!(face.corners.iter().all(|corner| corner.source_normal == Some(normal)));
        let (a, b) = face.edges().find(|&(a, b)| mesh.uv_seams.contains(&Edge::new(a, b))).unwrap();
        let strip = affected.iter().map(|&j| &mesh.faces[j])
            .find(|f| f.edges().any(|pair| pair == (b, a))).unwrap();
        let strip_normal = strip.corners.iter().find(|c| c.vertex == a).unwrap().source_normal.unwrap();
        assert!(strip_normal.dot(normal) < 0.999);
    }
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
        uv_seams: default(),
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
