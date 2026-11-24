use super::*;
use crate::editor::selection_outline::render_tests::{capture_when, frame};
use edit::layers::{LayerAction, PanelSection, ThumbnailSize, ui::BlendReveal};

fn command(app: &mut App, action: LayerAction) {
    app.world_mut()
        .write_message(Action::ImageEdit(edit::EditAction::Layers(action)));
    for _ in 0..5 {
        frame(app);
    }
}
fn click(app: &mut App, window: Entity, wanted: impl Fn(LayerAction) -> bool) {
    click_action(
        app,
        window,
        |a| matches!(a,Action::ImageEdit(edit::EditAction::Layers(action)) if wanted(action)),
    );
    // A property action can rebuild the tree at the end of the input frame.
    // Let layout compute the new rows before the next pointer hit test.
    for _ in 0..3 {
        frame(app);
    }
}

fn input_visible(app: &mut App, label: &str) -> bool {
    let world = app.world_mut();
    world
        .query_filtered::<(&AccessibleLabel, &ComputedNode), With<EditableText>>()
        .iter(world)
        .find_map(|(name, node)| (name.0 == label).then_some(!node.is_empty()))
        .expect("Layer input")
}

fn blend_height(app: &mut App) -> Option<f32> {
    let world = app.world_mut();
    world
        .query_filtered::<&ComputedNode, With<BlendReveal>>()
        .iter(world)
        .next()
        .map(|node| node.size().y)
}

#[test]
#[ignore = "requires GPU adapter; exercises the native layer tree, property binding, mask selection and panel settings"]
fn gpu_image_layers_tree_properties_masks_and_save() {
    let fixture = Fixture::new();
    let (mut app, window, target) = lighting_ui_render::fixture();
    app.world_mut()
        .resource_scope(|world, mut browser: Mut<Browser>| {
            let mut editor = world.resource_mut::<Editor>();
            editor.root = fixture.0.clone();
            editor.project.asset_root = fixture.0.clone();
            browser.refresh(&mut editor.project);
            editor.drawer = false;
        });
    for _ in 0..45 {
        frame(&mut app);
    }
    let image = file(&app, "b-transparent.png");
    app.world_mut().write_message(Action::ViewImage(image));
    for _ in 0..25 {
        frame(&mut app);
    }
    app.world_mut()
        .write_message(Action::ImageEdit(edit::EditAction::Edit));
    for _ in 0..120 {
        frame(&mut app);
        if app
            .world()
            .resource::<ImageViewer>()
            .edit
            .editing(app.world().resource::<Editor>().viewing_image)
        {
            break;
        }
    }
    for _ in 0..12 {
        frame(&mut app);
    }
    let id = app.world().resource::<Editor>().viewing_image.unwrap();
    let output = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/image-layers-qa");
    std::fs::create_dir_all(&output).unwrap();
    let revision = app
        .world()
        .resource::<ImageViewer>()
        .edit
        .document(Some(id))
        .unwrap()
        .revision;
    assert!(!input_visible(&mut app, "Найти слой"));
    capture_when(
        &mut app,
        &target,
        &output.join("00-compact-panel.png"),
        |_| true,
    );
    assert_eq!(
        app.world()
            .resource::<ImageViewer>()
            .edit
            .document(Some(id))
            .unwrap()
            .revision,
        revision
    );
    let layer = app
        .world()
        .resource::<ImageViewer>()
        .edit
        .document(Some(id))
        .unwrap()
        .active_id()
        .unwrap();
    click(
        &mut app,
        window,
        |a| matches!(a, LayerAction::Blend(id) if id == layer),
    );
    let opening_height = blend_height(&mut app).unwrap();
    for _ in 0..14 {
        frame(&mut app);
    }
    let expanded_height = blend_height(&mut app).unwrap();
    assert!(opening_height > 0.0 && opening_height < expanded_height);
    let slider_position = {
        let world = app.world_mut();
        let (_, node, transform) = world
            .query::<(&AccessibleLabel, &ComputedNode, &UiGlobalTransform)>()
            .iter(world)
            .find(|(label, _, _)| label.0 == "Непрозрачность слоя")
            .unwrap();
        (transform.affine().translation - Vec2::X * node.size().x * 0.25)
            * node.inverse_scale_factor()
    };
    use crate::editor::tests::lighting_ui_render::{mouse, move_pointer};
    move_pointer(&mut app, window, slider_position);
    mouse(&mut app, window, bevy::input::ButtonState::Pressed);
    mouse(&mut app, window, bevy::input::ButtonState::Released);
    for _ in 0..4 {
        frame(&mut app);
    }
    let opacity = app
        .world()
        .resource::<ImageViewer>()
        .edit
        .document(Some(id))
        .unwrap()
        .document
        .layer(layer)
        .unwrap()
        .opacity;
    assert!((0.15..0.35).contains(&opacity), "opacity: {opacity}");
    capture_when(
        &mut app,
        &target,
        &output.join("00-layer-blend.png"),
        |_| true,
    );
    use photocraft_color::BlendMode;
    click(
        &mut app,
        window,
        |a| matches!(a, LayerAction::SetBlend(id, BlendMode::Multiply) if id == layer),
    );
    let edit = &app.world().resource::<ImageViewer>().edit;
    assert_eq!(edit.layers.blend_menu, Some(layer));
    assert_eq!(
        edit.document(Some(id))
            .unwrap()
            .document
            .layer(layer)
            .unwrap()
            .blend,
        BlendMode::Multiply
    );
    let (slider, value) = {
        let world = app.world_mut();
        let (entity, _, value) = world
            .query::<(Entity, &AccessibleLabel, &bevy::ui_widgets::SliderValue)>()
            .iter(world)
            .find(|(_, label, _)| label.0 == "Непрозрачность слоя")
            .unwrap();
        (entity, value.0)
    };
    click(
        &mut app,
        window,
        |a| matches!(a, LayerAction::Blend(id) if id == layer),
    );
    assert!(
        app.world()
            .resource::<ImageViewer>()
            .edit
            .layers
            .blend_menu
            .is_none()
    );
    let closing_height = blend_height(&mut app).unwrap();
    assert!(closing_height > 0.0 && closing_height < expanded_height);
    assert_eq!(
        app.world()
            .get::<bevy::ui_widgets::SliderValue>(slider)
            .unwrap()
            .0,
        value
    );
    assert_eq!(app.world().get::<Pickable>(slider), Some(&Pickable::IGNORE));
    for _ in 0..12 {
        frame(&mut app);
    }
    assert!(blend_height(&mut app).is_none());
    for _ in 0..2 {
        app.world_mut()
            .write_message(Action::ImageEdit(edit::EditAction::Undo));
        frame(&mut app);
    }
    click_control(
        &mut app,
        window,
        |a| matches!(a, Action::ImageEdit(edit::EditAction::SelectLayer(0))),
        MouseButton::Right,
    );
    for _ in 0..12 {
        frame(&mut app);
    }
    click(&mut app, window, |a| {
        matches!(a, LayerAction::Category("Параметры слоя"))
    });
    assert!(
        app.world_mut()
            .query::<(&Control, &ComputedNode)>()
            .iter(app.world())
            .any(|(control, node)| matches!(
                control.action,
                Action::ImageEdit(edit::EditAction::SelectLayer(0))
            ) && !node.is_empty())
    );
    capture_when(
        &mut app,
        &target,
        &output.join("00-context-settings.png"),
        |_| true,
    );
    click(&mut app, window, |a| {
        matches!(a, LayerAction::Lock("pixels"))
    });
    assert!(
        app.world()
            .resource::<ImageViewer>()
            .edit
            .document(Some(id))
            .unwrap()
            .active_layer()
            .unwrap()
            .locks
            .pixels
    );
    command(&mut app, LayerAction::Cancel);
    app.world_mut()
        .write_message(Action::ImageEdit(edit::EditAction::Undo));
    for _ in 0..5 {
        frame(&mut app);
    }
    click(&mut app, window, |a| {
        matches!(a, LayerAction::PanelSection(PanelSection::Search))
    });
    assert!(input_visible(&mut app, "Найти слой"));
    {
        let world = app.world_mut();
        let entity = world
            .query_filtered::<Entity, With<edit::layers::ui::LayerSearch>>()
            .single(world)
            .unwrap();
        world
            .get_mut::<EditableText>(entity)
            .unwrap()
            .editor_mut()
            .set_text("слой_которого_нет");
        world
            .resource_mut::<InputFocus>()
            .set(entity, bevy::input_focus::FocusCause::Auto);
    }
    for _ in 0..5 {
        frame(&mut app);
    }
    {
        let world = app.world_mut();
        assert!(
            world
                .query::<&Text>()
                .iter(world)
                .any(|text| text.0 == "Слои не найдены")
        );
    }
    click(&mut app, window, |a| {
        matches!(a, LayerAction::PanelSection(PanelSection::Search))
    });
    assert!(
        app.world().resource::<ImageViewer>().edit.active(
            edit::EditAction::Layers(LayerAction::PanelSection(PanelSection::Search)),
            Some(id)
        ),
        "The closed search button must indicate an active filter"
    );
    click(&mut app, window, |a| {
        matches!(a, LayerAction::PanelSection(PanelSection::Search))
    });
    click(&mut app, window, |a| {
        matches!(a, LayerAction::Category("Вид панели"))
    });
    assert!(app.world().resource::<ImageViewer>().edit.layers.menu);
    click(&mut app, window, |a| matches!(a, LayerAction::Cancel));
    click(&mut app, window, |a| matches!(a, LayerAction::ResetFilter));
    click(&mut app, window, |a| {
        matches!(a, LayerAction::PanelSection(PanelSection::Search))
    });
    assert!(!input_visible(&mut app, "Найти слой"));
    assert!(
        !app.world()
            .resource::<ImageViewer>()
            .edit
            .document(Some(id))
            .unwrap()
            .dirty(),
        "View-only sections and filtering must not dirty the document"
    );
    click_action(&mut app, window, |a| {
        matches!(a, Action::ImageEdit(edit::EditAction::AddLayer))
    });
    let raster = app
        .world()
        .resource::<ImageViewer>()
        .edit
        .document(Some(id))
        .unwrap()
        .active_id()
        .unwrap();
    command(&mut app, LayerAction::Command("layer.new.group"));
    assert!(
        app.world()
            .resource::<ImageViewer>()
            .edit
            .layers
            .form
            .is_some()
    );
    click(&mut app, window, |a| matches!(a, LayerAction::Apply));
    let group = app
        .world()
        .resource::<ImageViewer>()
        .edit
        .document(Some(id))
        .unwrap()
        .active_id()
        .unwrap();
    assert_ne!(raster, group);
    command(
        &mut app,
        LayerAction::Move(raster.0, group.0, "into", false),
    );
    command(&mut app, LayerAction::Command("layer.layerMask.revealAll"));
    command(
        &mut app,
        LayerAction::Command("layer.layerStyle.dropShadow"),
    );
    assert!(
        app.world().resource::<ImageViewer>().edit.error.is_none(),
        "{:?}",
        app.world().resource::<ImageViewer>().edit.error
    );
    assert!(
        app.world()
            .resource::<ImageViewer>()
            .edit
            .layers
            .form
            .is_some()
    );
    let field = app
        .world()
        .resource::<ImageViewer>()
        .edit
        .layers
        .form
        .as_ref()
        .unwrap()
        .fields
        .iter()
        .position(|f| f.key == "opacity")
        .unwrap();
    command(&mut app, LayerAction::FieldNumber(field, 63.0));
    assert_eq!(
        app.world()
            .resource::<ImageViewer>()
            .edit
            .layers
            .form
            .as_ref()
            .unwrap()
            .fields[field]
            .text,
        "63"
    );
    capture_when(
        &mut app,
        &target,
        &output.join("01-shadow-properties.png"),
        |_| true,
    );
    click(&mut app, window, |a| matches!(a, LayerAction::Apply));
    {
        let viewer = app.world().resource::<ImageViewer>();
        assert!(viewer.edit.error.is_none(), "{:?}", viewer.edit.error);
        let doc = viewer.edit.document(Some(id)).unwrap();
        let layer = doc.document.layer(raster).unwrap();
        let photocraft_doc::Effect::DropShadow(shadow) = &layer.effects.items[0] else {
            panic!("missing shadow")
        };
        assert!((shadow.common.opacity - 0.63).abs() < 0.0001);
        assert!(
            doc.document
                .layer(group)
                .unwrap()
                .children()
                .unwrap()
                .iter()
                .any(|l| l.id == raster)
        );
    }
    // Scroll the native panel to reveal its complete outliner and mask thumbnail.
    {
        let width = app.world().get::<Window>(window).unwrap().width();
        let world = app.world_mut();
        for (node,transform,mut offset) in world.query_filtered::<(&ComputedNode,&UiGlobalTransform,&mut ScrollPosition),With<Scroller>>().iter_mut(world) {
            if transform.affine().translation.x>width*0.65 && node.size().y>100.0 {offset.y=((node.content_size().y-node.size().y)*node.inverse_scale_factor()).max(0.0);}
        }
    }
    for _ in 0..6 {
        frame(&mut app);
    }
    let active = app
        .world()
        .resource::<ImageViewer>()
        .edit
        .document(Some(id))
        .unwrap()
        .active;
    click(
        &mut app,
        window,
        |a| matches!(a,LayerAction::Mask(index) if index==active),
    );
    assert!(
        app.world()
            .resource::<ImageViewer>()
            .edit
            .document(Some(id))
            .unwrap()
            .mask_target
    );
    capture_when(&mut app, &target, &output.join("02-layer-tree.png"), |_| {
        true
    });
    let revision = app
        .world()
        .resource::<ImageViewer>()
        .edit
        .document(Some(id))
        .unwrap()
        .revision;
    command(&mut app, LayerAction::MaskPreview(false));
    assert_eq!(
        app.world()
            .resource::<ImageViewer>()
            .edit
            .document(Some(id))
            .unwrap()
            .revision,
        revision
    );
    capture_when(
        &mut app,
        &target,
        &output.join("03-mask-preview.png"),
        |_| true,
    );
    command(&mut app, LayerAction::MaskPreview(false));
    command(&mut app, LayerAction::Category("Вид панели"));
    click(&mut app, window, |a| matches!(a, LayerAction::OnlySelected));
    command(&mut app, LayerAction::Thumbnail(ThumbnailSize::Large));
    capture_when(
        &mut app,
        &target,
        &output.join("04-panel-options.png"),
        |_| true,
    );
    command(&mut app, LayerAction::Cancel);
    command(&mut app, LayerAction::ResetFilter);
    app.world_mut()
        .write_message(Action::ImageEdit(edit::EditAction::Save));
    for _ in 0..240 {
        frame(&mut app);
        if !app
            .world()
            .resource::<ImageViewer>()
            .edit
            .document(Some(id))
            .unwrap()
            .dirty()
        {
            break;
        }
    }
    assert!(
        !app.world()
            .resource::<ImageViewer>()
            .edit
            .document(Some(id))
            .unwrap()
            .dirty(),
        "{:?}",
        app.world().resource::<ImageViewer>().edit.error
    );
    let native = photocraft_format::load_path(&fixture.0.join("b-transparent.pcraft")).unwrap();
    assert_eq!(native.layer_count(), 3);
    assert!(native.layer(raster).unwrap().mask.is_some());
    command(&mut app, LayerAction::Select(0, false, false));
    command(&mut app, LayerAction::Command("filter.other.highPass"));
    click(&mut app, window, |a| matches!(a, LayerAction::Apply));
    let active = {
        let viewer = app.world().resource::<ImageViewer>();
        assert!(viewer.edit.error.is_none(), "{:?}", viewer.edit.error);
        let doc = viewer.edit.document(Some(id)).unwrap();
        let photocraft_doc::LayerContent::Smart(smart) = &doc.active_layer().unwrap().content
        else {
            panic!("Filter must become an editable smart filter")
        };
        assert_eq!(smart.smart_filters.len(), 1);
        doc.active
    };
    click(
        &mut app,
        window,
        |a| matches!(a,LayerAction::FilterMask(index) if index==active),
    );
    {
        let viewer = app.world().resource::<ImageViewer>();
        assert!(viewer.edit.error.is_none(), "{:?}", viewer.edit.error);
        let doc = viewer.edit.document(Some(id)).unwrap();
        assert!(doc.mask_view.is_some_and(|view| view.filter));
        assert!(doc.paintable());
    }
    command(&mut app, LayerAction::Category("Параметры слоя"));
    click(&mut app, window, |a| matches!(a, LayerAction::Properties));
    assert_eq!(
        app.world()
            .resource::<ImageViewer>()
            .edit
            .layers
            .form
            .as_ref()
            .unwrap()
            .command,
        "editor.filterMaskOptions"
    );
    click(&mut app, window, |a| matches!(a, LayerAction::Apply));
    assert!(app.world().resource::<ImageViewer>().edit.error.is_none());
    println!("Layer UI captures: {}", output.display());
}
