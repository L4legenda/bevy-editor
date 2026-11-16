use super::*;

#[test]
fn background_transform_matches_native_pixels_and_retains_one_history_step() {
    let fixture = Fixture::new();
    let mut doc = PhotoDocument::load(&fixture.0).unwrap();
    doc.layer_command("select.rect", json!({"x":4,"y":4,"width":16,"height":12}))
        .unwrap();
    let original = doc.composite().pixels;
    let history = doc.undo.len();
    let frame = doc.transform_frame().unwrap();
    let mut edit = doc.begin_move();
    let quad = [[7.0, 6.0], [26.0, 7.0], [23.0, 24.0], [6.0, 22.0]];
    let (document, pixels, changed) = doc.transform_job(&edit, frame, quad, false)().unwrap();
    assert_eq!(
        doc.composite().pixels,
        original,
        "A worker must not mutate the live document"
    );
    doc.preview_transform(&mut edit, frame, quad, false)
        .unwrap();
    let expected = doc.composite().pixels;
    assert_eq!(pixels.pixels, expected);
    doc.accept_transform(&mut edit, document, changed);
    assert_eq!(doc.composite().pixels, expected);
    assert_eq!(doc.undo.len(), history + 1);
    doc.undo();
    assert_eq!(doc.composite().pixels, original);
    doc.redo();
    assert_eq!(doc.composite().pixels, expected);
    doc.cancel_move(&mut edit);
    assert_eq!(doc.composite().pixels, original);
    assert_eq!(doc.undo.len(), history);
}

fn at(doc: &PhotoDocument, x: i32, y: i32) -> [u8; 4] {
    let image = doc.composite_region(Rect::new(x, y, x + 1, y + 1));
    image.pixels[..4].try_into().unwrap()
}
fn selected(doc: &PhotoDocument, x: i32, y: i32) -> f32 {
    doc.document
        .selection
        .as_ref()
        .map_or(0.0, |s| s.sample_channel(x, y, 0))
}

#[test]
fn photocraft_selections_combine_and_constrain_paint_and_bucket_fill() {
    let fixture = Fixture::new();
    let mut doc = PhotoDocument::load(&fixture.0).unwrap();
    doc.layer_command("select.rect", json!({"x":4,"y":4,"width":10,"height":8}))
        .unwrap();
    assert!(selected(&doc, 5, 5) > 0.9);
    assert_eq!(selected(&doc, 2, 2), 0.0);
    doc.layer_command(
        "select.rect",
        json!({"x":10,"y":4,"width":10,"height":8,"mode":"add"}),
    )
    .unwrap();
    assert!(selected(&doc, 18, 5) > 0.9);
    doc.layer_command(
        "select.rect",
        json!({"x":8,"y":4,"width":4,"height":8,"mode":"subtract"}),
    )
    .unwrap();
    assert_eq!(selected(&doc, 9, 5), 0.0);
    doc.layer_command(
        "select.rect",
        json!({"x":2,"y":2,"width":6,"height":6,"mode":"intersect"}),
    )
    .unwrap();
    assert!(selected(&doc, 5, 5) > 0.9);
    assert_eq!(selected(&doc, 18, 5), 0.0);
    let before = at(&doc, 1, 1);
    doc.fill(5, 5, [1.0, 0.0, 0.0, 1.0]);
    assert_eq!(at(&doc, 5, 5), [255, 0, 0, 255]);
    assert_eq!(at(&doc, 1, 1), before);
    doc.layer_command("select.inverse", json!({})).unwrap();
    assert_eq!(selected(&doc, 5, 5), 0.0);
    assert!(selected(&doc, 1, 1) > 0.9);
    doc.layer_command("select.deselect", json!({})).unwrap();
    assert!(doc.document.selection.is_none());
    doc.undo();
    assert!(doc.document.selection.is_some());
    doc.redo();
    assert!(doc.document.selection.is_none());
}

#[test]
fn photocraft_ellipse_lasso_wand_quick_and_object_selection_use_the_native_engine() {
    for (command, params) in [
        (
            "select.rect",
            json!({"x":4,"y":4,"width":16,"height":16,"ellipse":true}),
        ),
        (
            "select.lasso",
            json!({"points":[[4,4],[20,4],[20,20],[4,20]],"polygonal":true}),
        ),
        ("select.magicWand", json!({"x":10,"y":10,"tolerance":0})),
        (
            "select.quick",
            json!({"points":[[10,10],[14,10]],"size":8,"mode":"replace"}),
        ),
        ("select.object", json!({"rect":[4,4,20,20]})),
    ] {
        let fixture = Fixture::new();
        let mut doc = PhotoDocument::load(&fixture.0).unwrap();
        doc.layer_command(command, params)
            .unwrap_or_else(|e| panic!("{command}: {e}"));
        // A uniform image can correctly have no detected foreground object.
        if command != "select.object" {
            assert!(selected(&doc, 10, 10) > 0.5, "{command}");
        }
    }
}

#[test]
fn selected_pixels_transform_from_the_original_and_undo_as_one_edit() {
    let fixture = Fixture::new();
    let mut doc = PhotoDocument::load(&fixture.0).unwrap();
    doc.layer_command("select.rect", json!({"x":4,"y":4,"width":8,"height":8}))
        .unwrap();
    let before = doc.composite().pixels;
    let undo = doc.undo.len();
    let frame = doc.transform_frame().unwrap();
    assert_eq!(frame, [4.0, 4.0, 12.0, 12.0]);
    let mut edit = doc.begin_move();
    for dx in [4.0, 8.0, 12.0] {
        doc.preview_transform(
            &mut edit,
            frame,
            [
                [4.0 + dx, 4.0],
                [12.0 + dx, 4.0],
                [12.0 + dx, 12.0],
                [4.0 + dx, 12.0],
            ],
            true,
        )
        .unwrap();
        assert_eq!(doc.undo.len(), undo + 1);
    }
    assert_eq!(at(&doc, 5, 5)[3], 0);
    assert_eq!(at(&doc, 17, 5), [30, 60, 90, 255]);
    assert_eq!(at(&doc, 1, 1), [30, 60, 90, 255]);
    assert!(selected(&doc, 17, 5) > 0.9);
    assert_eq!(selected(&doc, 5, 5), 0.0);
    doc.undo();
    assert_eq!(doc.composite().pixels, before);
    doc.redo();
    assert_eq!(at(&doc, 5, 5)[3], 0);
}

#[test]
fn cancelling_a_transform_restores_pixels_selection_dirty_state_and_redo() {
    let fixture = Fixture::new();
    let mut doc = PhotoDocument::load(&fixture.0).unwrap();
    let before = doc.composite().pixels;
    doc.layer_command("select.all", json!({})).unwrap();
    doc.undo();
    let redo = doc.redo.len();
    let revision = doc.revision;
    let undo = doc.undo.len();
    let frame = doc.transform_frame().unwrap();
    let mut edit = doc.begin_move();
    doc.preview_transform(
        &mut edit,
        frame,
        [[2.0, 2.0], [26.0, 2.0], [26.0, 18.0], [2.0, 18.0]],
        false,
    )
    .unwrap();
    doc.cancel_move(&mut edit);
    assert_eq!(doc.composite().pixels, before);
    assert_eq!(doc.revision, revision);
    assert_eq!(doc.redo.len(), redo);
    assert_eq!(doc.undo.len(), undo);
    assert!(!doc.dirty());
    doc.redo();
    assert!(doc.document.selection.is_some());
}

#[test]
fn locked_layers_and_excessive_transform_sizes_are_rejected_without_an_edit() {
    let fixture = Fixture::new();
    let mut doc = PhotoDocument::load(&fixture.0).unwrap();
    doc.active_layer_mut().unwrap().locks.position = true;
    assert!(doc.transform_frame().is_none());
    doc.active_layer_mut().unwrap().locks.position = false;
    let frame = doc.transform_frame().unwrap();
    let mut edit = doc.begin_move();
    let revision = doc.revision;
    assert!(
        doc.preview_transform(
            &mut edit,
            frame,
            [
                [0.0, 0.0],
                [32000.0, 0.0],
                [32000.0, 32000.0],
                [0.0, 32000.0]
            ],
            false
        )
        .is_err()
    );
    assert_eq!(doc.revision, revision);
    assert!(!doc.can_undo());
}
