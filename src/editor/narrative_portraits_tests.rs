use super::*;

fn selected(app: &App, node: u32) -> usize {
    let Content::Dialogue(d) = &app
        .world()
        .resource::<Editor>()
        .narrative
        .current()
        .unwrap()
        .content
    else {
        panic!()
    };
    d.node(node).unwrap().portrait
}
fn history(app: &App) -> usize {
    app.world()
        .resource::<Editor>()
        .narrative
        .current()
        .unwrap()
        .undo
        .len()
}

#[test]
#[ignore = "GPU per-line portrait gallery, actual image clicks, history, save and preview"]
fn gpu_narrative_portrait_selection() {
    let output = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target/narrative-qa")
        .join(std::env::var("NARRATIVE_QA_PASS").unwrap_or("portraits-pass-1".into()));
    let lab = output.join(format!("portrait-project-{}", std::process::id()));
    fn copy_assets(from: &std::path::Path, to: &std::path::Path) {
        std::fs::create_dir_all(to).unwrap();
        for entry in std::fs::read_dir(from).unwrap() {
            let entry = entry.unwrap();
            let path = to.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                copy_assets(&entry.path(), &path);
            } else {
                std::fs::copy(entry.path(), path).unwrap();
            }
        }
    }
    let mut project = crate::project::EditorProject::load(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/legacy/project.json"),
    )
    .unwrap();
    copy_assets(&project.asset_root, &lab);
    for (name, tint) in [
        ("calm.png", [65, 120, 184]),
        ("alert.png", [177, 115, 60]),
        ("happy.png", [70, 150, 112]),
    ] {
        let image = image::RgbaImage::from_fn(180, 220, |x, y| {
            let dx = x as f32 - 90.;
            let dy = y as f32 - 105.;
            image::Rgba(if dx * dx / 3600. + dy * dy / 4900. < 1. {
                [tint[0], tint[1], tint[2], 255]
            } else {
                [14, 19, 30, 255]
            })
        });
        image.save(lab.join(name)).unwrap();
    }
    std::fs::create_dir_all(lab.join("NPC")).unwrap();
    let portraits = vec![
        Portrait {
            label: "Спокойствие".into(),
            path: "calm.png".into(),
        },
        Portrait {
            label: "Тревога".into(),
            path: "alert.png".into(),
        },
        Portrait {
            label: "Радость".into(),
            path: "happy.png".into(),
        },
    ];
    Content::Character(Character {
        version: VERSION,
        name: "Астра".into(),
        description: String::new(),
        portraits: portraits.clone(),
    })
    .save(&lab, std::path::Path::new("NPC/Астра.dcharacter"))
    .unwrap();
    Content::Character(Character {
        version: VERSION,
        name: "Без портретов".into(),
        description: String::new(),
        portraits: vec![],
    })
    .save(&lab, std::path::Path::new("NPC/Без портретов.dcharacter"))
    .unwrap();
    Content::Character(Character {
        version: VERSION,
        name: "Лира".into(),
        description: String::new(),
        portraits: vec![
            Portrait {
                label: "Нейтрально".into(),
                path: "happy.png".into(),
            },
            Portrait {
                label: "Улыбка".into(),
                path: "alert.png".into(),
            },
        ],
    })
    .save(&lab, std::path::Path::new("NPC/Лира.dcharacter"))
    .unwrap();
    project.asset_root = lab.clone();
    let (mut app, window, target) = fixture_project(project);
    app.world_mut().resource_mut::<Editor>().drawer = false;
    action(&mut app, NarrativeAction::Create(false));
    action(&mut app, NarrativeAction::Add(NodeKind::Line));
    action(&mut app, NarrativeAction::Add(NodeKind::Line));
    {
        let mut editor = app.world_mut().resource_mut::<Editor>();
        let doc = editor.narrative.current_mut().unwrap();
        let Content::Dialogue(d) = &mut doc.content else {
            panic!()
        };
        for id in [1, 2] {
            d.node_mut(id).unwrap().speaker = Some("NPC/Астра.dcharacter".into());
        }
        d.node_mut(0).unwrap().position = [15., 15.];
        d.node_mut(1).unwrap().position = [430., 15.];
        d.node_mut(2).unwrap().position = [850., 15.];
        doc.zoom = 0.85;
        doc.pan = Vec2::new(20., 25.);
    }
    app.world_mut().resource_mut::<NarrativeUi>().generation += 1;
    warm(&mut app, 45);
    let before = history(&app);
    // The button belongs to its own line even while another node is selected.
    action(&mut app, NarrativeAction::Select(2));
    click(&mut app, window, "Выбрать портрет ноды #1");
    assert_eq!(
        app.world().resource::<NarrativeUi>().picker,
        Some(Picker::Speaker)
    );
    capture_when(&mut app, &target, &output.join("01-gallery.png"), |_| true);
    action(&mut app, NarrativeAction::SetPortrait(999));
    assert_eq!(history(&app), before);
    assert_eq!(selected(&app, 1), 0);
    // The center of this tile is inside its image, not a separate text button.
    click(&mut app, window, "Выбрать портрет 2: Тревога");
    assert_eq!(selected(&app, 1), 1);
    assert_eq!(selected(&app, 2), 0);
    assert_eq!(history(&app), before + 1);
    assert!(app.world().resource::<NarrativeUi>().picker.is_none());
    capture_when(
        &mut app,
        &target,
        &output.join("02-line-portrait.png"),
        |_| true,
    );
    action(&mut app, NarrativeAction::Undo);
    assert_eq!(selected(&app, 1), 0);
    action(&mut app, NarrativeAction::Redo);
    assert_eq!(selected(&app, 1), 1);
    let before = history(&app);
    click(&mut app, window, "Выбрать портрет ноды #1");
    capture_when(&mut app, &target, &output.join("03-selected.png"), |_| true);
    click(&mut app, window, "Выбрать портрет 2: Тревога");
    assert_eq!(
        history(&app),
        before,
        "Choosing the same image cannot create an undo entry"
    );
    click(&mut app, window, "Выбрать портрет ноды #1");
    key(
        &mut app,
        window,
        KeyCode::Escape,
        bevy::input::keyboard::Key::Escape,
        None,
    );
    assert!(app.world().resource::<NarrativeUi>().picker.is_none());
    assert_eq!(selected(&app, 1), 1);
    // Character replacement and image selection share one anchored, undimmed picker.
    let before = history(&app);
    click(&mut app, window, "Выбрать говорящего ноды #1");
    let overlay = app.world().resource::<NarrativeUi>().overlay;
    assert_eq!(
        app.world().get::<Node>(overlay).unwrap().display,
        Display::None
    );
    let search = edit_text(&mut app, DraftField::Search, "без");
    assert_eq!(app.world().resource::<InputFocus>().get(), Some(search));
    click(&mut app, window, "Выбрать персонажа: Без портретов");
    assert_eq!(history(&app), before + 1);
    assert_eq!(selected(&app, 1), 0);
    assert!(app.world().resource::<NarrativeUi>().picker.is_none());
    assert!(
        app.world()
            .resource::<picker_fx::State>()
            .confirmation
            .is_some()
    );
    capture_when(
        &mut app,
        &target,
        &output.join("character-replaced.png"),
        |_| true,
    );
    action(&mut app, NarrativeAction::Undo);
    assert_eq!(selected(&app, 1), 1);
    action(&mut app, NarrativeAction::Redo);
    assert_eq!(selected(&app, 1), 0);
    action(&mut app, NarrativeAction::Undo);
    assert_eq!(selected(&app, 1), 1);
    // One opening supports both steps: choose another actor, then their image.
    let before = history(&app);
    click(&mut app, window, "Выбрать говорящего ноды #1");
    edit_text(&mut app, DraftField::Search, "Лира");
    click(&mut app, window, "Выбрать персонажа: Лира");
    assert_eq!(selected_speaker(&app), Some("NPC/Лира.dcharacter".into()));
    assert_eq!(
        app.world().resource::<NarrativeUi>().picker,
        Some(Picker::Portrait)
    );
    assert_eq!(selected(&app, 1), 0);
    assert_eq!(history(&app), before + 1);
    let focus = app.world().resource::<InputFocus>().get().unwrap();
    assert!(
        app.world().get::<Control>(focus).is_some_and(|c| matches!(
            c.action,
            Action::Narrative(NarrativeAction::SetPortrait(0))
        ))
    );
    capture_when(
        &mut app,
        &target,
        &output.join("character-then-avatar.png"),
        |_| true,
    );
    click(&mut app, window, "Выбрать портрет 2: Улыбка");
    assert_eq!(selected(&app, 1), 1);
    assert_eq!(selected(&app, 2), 0);
    assert!(app.world().resource::<NarrativeUi>().picker.is_none());
    action(&mut app, NarrativeAction::Undo);
    assert_eq!(selected_speaker(&app), Some("NPC/Лира.dcharacter".into()));
    assert_eq!(selected(&app, 1), 0);
    action(&mut app, NarrativeAction::Undo);
    assert_eq!(selected_speaker(&app), Some("NPC/Астра.dcharacter".into()));
    assert_eq!(selected(&app, 1), 1);
    // Enter picks the actor, a second Enter can accept its default portrait.
    click(&mut app, window, "Выбрать говорящего ноды #1");
    edit_text(&mut app, DraftField::Search, "Лира");
    key(
        &mut app,
        window,
        KeyCode::Enter,
        bevy::input::keyboard::Key::Enter,
        None,
    );
    assert_eq!(
        app.world().resource::<NarrativeUi>().picker,
        Some(Picker::Portrait)
    );
    key(
        &mut app,
        window,
        KeyCode::Enter,
        bevy::input::keyboard::Key::Enter,
        None,
    );
    assert!(app.world().resource::<NarrativeUi>().picker.is_none());
    assert_eq!(selected(&app, 1), 0);
    action(&mut app, NarrativeAction::Undo);
    assert_eq!(selected_speaker(&app), Some("NPC/Астра.dcharacter".into()));
    assert_eq!(selected(&app, 1), 1);
    action(&mut app, NarrativeAction::Save);
    {
        let editor = app.world().resource::<Editor>();
        let doc = editor.narrative.current().unwrap();
        let Content::Dialogue(saved) = Content::read(&editor.root, &doc.path).unwrap() else {
            panic!()
        };
        assert_eq!(saved.node(1).unwrap().portrait, 1);
        assert_eq!(saved.node(2).unwrap().portrait, 0);
        let Content::Character(saved) =
            Content::read(&editor.root, std::path::Path::new("NPC/Астра.dcharacter")).unwrap()
        else {
            panic!()
        };
        assert_eq!(
            saved.portraits, portraits,
            "Choosing a line portrait cannot change the character's primary image"
        );
    }
    action(&mut app, NarrativeAction::Preview);
    capture_when(&mut app, &target, &output.join("04-preview.png"), |_| true);
    {
        let world = app.world();
        let mut pending = vec![world.resource::<NarrativeUi>().modal];
        let mut images = Vec::new();
        while let Some(entity) = pending.pop() {
            if let Some(image) = world.get::<ImageNode>(entity) {
                if let Some(path) = world.resource::<AssetServer>().get_path(image.image.id()) {
                    images.push(path.path().to_path_buf());
                }
            }
            if let Some(children) = world.get::<Children>(entity) {
                pending.extend(children.iter());
            }
        }
        assert!(
            images.iter().any(|p| p.ends_with("alert.png")),
            "Preview must render the selected line image"
        );
        assert!(
            !images.iter().any(|p| p.ends_with("calm.png")),
            "Preview cannot fall back to the character's primary image"
        );
    }
    action(&mut app, NarrativeAction::Dismiss);
    // More images and a compact window must retain a usable, scrollable gallery.
    {
        let mut editor = app.world_mut().resource_mut::<Editor>();
        editor
            .narrative
            .open(&lab, "NPC/Астра.dcharacter".into())
            .unwrap();
        let Content::Character(c) = &mut editor.narrative.current_mut().unwrap().content else {
            panic!()
        };
        for i in 4..=9 {
            c.portraits.push(Portrait {
                label: format!("Выражение {i}"),
                path: if i == 9 {
                    "missing.png".into()
                } else {
                    "happy.png".into()
                },
            });
        }
        let id = editor
            .narrative
            .tabs
            .iter()
            .find(|d| matches!(d.content, Content::Dialogue(_)))
            .unwrap()
            .id;
        editor.narrative.active = Some(id);
    }
    resize_qa(&mut app, window, &target, 700, 620);
    action(&mut app, NarrativeAction::Portrait);
    warm(&mut app, 8);
    capture_when(
        &mut app,
        &target,
        &output.join("05-compact-gallery.png"),
        |_| true,
    );
    {
        let world = app.world_mut();
        let mut q = world.query::<(&AccessibleLabel, &ComputedNode, &mut ScrollPosition)>();
        for (label, size, mut scroll) in q.iter_mut(world) {
            if label.0 == "Портреты персонажа" {
                scroll.0.y = (size.content_size().y - size.size().y).max(0.);
            }
        }
    }
    warm(&mut app, 8);
    click(&mut app, window, "Выбрать портрет 9: Выражение 9");
    assert_eq!(selected(&app, 1), 8);
    // Empty characters offer their card rather than an unusable selector.
    {
        let mut editor = app.world_mut().resource_mut::<Editor>();
        let doc = editor.narrative.current_mut().unwrap();
        let Content::Dialogue(d) = &mut doc.content else {
            panic!()
        };
        d.node_mut(1).unwrap().speaker = Some("NPC/Без портретов.dcharacter".into());
        d.node_mut(1).unwrap().portrait = 0;
    }
    app.world_mut().resource_mut::<NarrativeUi>().generation += 1;
    action(&mut app, NarrativeAction::Portrait);
    capture_when(
        &mut app,
        &target,
        &output.join("06-no-portraits.png"),
        |_| true,
    );
    let before = history(&app);
    action(&mut app, NarrativeAction::SetPortrait(0));
    assert_eq!(history(&app), before);
    click(&mut app, window, "Открыть карточку персонажа");
    assert!(matches!(
        app.world()
            .resource::<Editor>()
            .narrative
            .current()
            .unwrap()
            .content,
        Content::Character(_)
    ));
}
