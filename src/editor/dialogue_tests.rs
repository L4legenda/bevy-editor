use super::*;

fn lab() -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "bevy-narrative-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&root).unwrap();
    root
}

#[test]
fn branching_roundtrip_preserves_speakers_portraits_and_cycles() {
    let root = lab();
    let c = create(&root, Path::new(""), true).unwrap();
    let mut character = Character {
        version: VERSION,
        name: "Астра".into(),
        description: "Навигатор".into(),
        portraits: vec![
            Portrait {
                uid: None,
                label: "Спокойствие".into(),
                path: "portraits/calm.png".into(),
            },
            Portrait {
                uid: None,
                label: "Тревога".into(),
                path: "portraits/worried.png".into(),
            },
        ],
    };
    Content::Character(character.clone())
        .save(&root, &c)
        .unwrap();
    assert_eq!(
        Content::read(&root, &c).unwrap(),
        Content::Character(character.clone())
    );
    character.name = "Новое имя".into();
    Content::Character(character.clone())
        .save(&root, &c)
        .unwrap();
    assert_eq!(
        Content::read(&root, &c).unwrap(),
        Content::Character(character)
    );
    let path = create(&root, Path::new(""), false).unwrap();
    let mut d = Dialogue::new("Прибытие".into());
    d.extensions.insert(
        "clinic".into(),
        serde_json::json!({"lines": {"1": {"id": "arrival", "sound": "audio/arrival.ogg"}}}),
    );
    let first = d.add(NodeKind::Line, 0);
    let n = d.node_mut(first).unwrap();
    n.speaker = Some(c.to_string_lossy().into_owned());
    n.portrait = 1;
    n.text = "Сигнал принят?".into();
    let end = d.add(NodeKind::End, first);
    d.node_mut(first).unwrap().choices.push(Choice {
        text: "Повторить".into(),
        target: Some(first),
        condition: Default::default(),
        dialogue: None,
    });
    assert!(d.warnings().is_empty());
    Content::Dialogue(d.clone()).save(&root, &path).unwrap();
    assert_eq!(
        Content::read(&root, &path).unwrap(),
        Content::Dialogue(d.clone())
    );
    d.delete(end);
    assert!(d.node(first).unwrap().choices[0].target.is_none());
    d.delete(0);
    assert!(d.node(0).is_some());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn tabs_reuse_paths_retain_unsaved_edits_and_undo_independently() {
    let root = lab();
    let a = create(&root, Path::new(""), false).unwrap();
    let b = create(&root, Path::new(""), true).unwrap();
    let mut workspace = Workspace::default();
    workspace.open(&root, a.clone()).unwrap();
    let id = workspace.active.unwrap();
    let doc = workspace.current_mut().unwrap();
    doc.checkpoint();
    if let Content::Dialogue(d) = &mut doc.content {
        d.add(NodeKind::Line, 0);
    }
    assert!(doc.dirty());
    workspace.open(&root, b).unwrap();
    workspace.open(&root, a).unwrap();
    assert_eq!(workspace.tabs.len(), 2);
    assert_eq!(workspace.active, Some(id));
    let doc = workspace.current_mut().unwrap();
    doc.undo();
    assert!(!doc.dirty());
    doc.redo();
    assert!(doc.dirty());
    workspace.close(id);
    assert!(matches!(
        workspace.current().unwrap().content,
        Content::Character(_)
    ));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn malformed_documents_are_rejected_without_changing_active_tab() {
    let root = lab();
    let path = create(&root, Path::new(""), false).unwrap();
    let mut w = Workspace::default();
    w.open(&root, path.clone()).unwrap();
    let active = w.active;
    let mut d = Dialogue::new("Bad".into());
    d.nodes[0].choices[0].target = Some(999);
    fs::write(root.join("bad.dialog"), serde_json::to_vec(&d).unwrap()).unwrap();
    assert!(w.open(&root, "bad.dialog".into()).is_err());
    assert_eq!(w.active, active);
    assert!(Content::read(&root, Path::new("../escape.dialog")).is_err());
    assert!(
        Content::Character(Character {
            version: VERSION,
            name: "Unsafe".into(),
            description: String::new(),
            portraits: vec![Portrait {
                uid: None,
                label: "x".into(),
                path: "../private.png".into()
            }]
        })
        .save(&root, &PathBuf::from("unsafe.dcharacter"))
        .is_err()
    );
    let same = create(&root, Path::new(""), false).unwrap();
    assert_ne!(same, path);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn validation_finds_unreachable_and_incomplete_branches_without_rejecting_drafts() {
    let mut d = Dialogue::new("Draft".into());
    let first = d.add(NodeKind::Line, 0);
    d.node_mut(0).unwrap().choices[0].target = None;
    assert!(d.validate().is_ok());
    assert!(d.warnings().iter().any(|w| w.contains("недоступна")));
    d.nodes.push(d.node(first).unwrap().clone());
    assert!(d.validate().is_err());
}

#[test]
fn renaming_character_preserves_saved_links_unsaved_text_history_and_metadata() {
    let root = lab();
    fs::create_dir_all(root.join("people")).unwrap();
    fs::create_dir_all(root.join("chapters")).unwrap();
    let source = create(&root, Path::new("people"), true).unwrap();
    fs::write(
        root.join(source.with_extension("dcharacter.meta")),
        b"character metadata",
    )
    .unwrap();
    let mut dialogue = Dialogue::new("Контакт".into());
    let line = dialogue.add(NodeKind::Line, 0);
    dialogue.node_mut(line).unwrap().speaker = Some(source.to_string_lossy().replace('\\', "/"));
    let mut raw = serde_json::to_value(&dialogue).unwrap();
    raw["custom_field"] = serde_json::json!({"keep": true});
    for name in ["open.dialog", "closed.dialog"] {
        fs::write(
            root.join("chapters").join(name),
            serde_json::to_vec_pretty(&raw).unwrap(),
        )
        .unwrap();
    }
    let mut workspace = Workspace::default();
    workspace.open(&root, source.clone()).unwrap();
    let character_id = workspace.active.unwrap();
    let previous_name = workspace.current().unwrap().content.title().to_owned();
    workspace.current_mut().unwrap().checkpoint();
    if let Content::Character(c) = &mut workspace.current_mut().unwrap().content {
        c.name = "Астра".into();
    }
    workspace
        .open(&root, "chapters/open.dialog".into())
        .unwrap();
    let graph_id = workspace.active.unwrap();
    let graph = workspace.current_mut().unwrap();
    graph.checkpoint();
    if let Content::Dialogue(d) = &mut graph.content {
        d.node_mut(line).unwrap().text = "Несохранённая реплика".into();
    }
    graph.redo.push(graph.saved.clone());
    let (_, destination) = workspace
        .rename(&root, character_id, "Астра")
        .unwrap()
        .unwrap();
    assert_eq!(destination, PathBuf::from("people/Астра.dcharacter"));
    assert!(!root.join(&source).exists());
    assert_eq!(
        fs::read(root.join(destination.with_extension("dcharacter.meta"))).unwrap(),
        b"character metadata"
    );
    let graph = workspace.get(graph_id).unwrap();
    assert!(graph.dirty());
    for content in std::iter::once(&graph.content)
        .chain(std::iter::once(&graph.saved))
        .chain(&graph.undo)
        .chain(&graph.redo)
    {
        if let Content::Dialogue(d) = content {
            assert_eq!(
                d.node(line).unwrap().speaker.as_deref(),
                Some("people/Астра.dcharacter")
            );
        }
    }
    if let Content::Dialogue(d) = &graph.content {
        assert_eq!(d.node(line).unwrap().text, "Несохранённая реплика");
    }
    for name in ["open.dialog", "closed.dialog"] {
        let saved: serde_json::Value =
            serde_json::from_slice(&fs::read(root.join("chapters").join(name)).unwrap()).unwrap();
        assert_eq!(saved["nodes"][1]["speaker"], "people/Астра.dcharacter");
        assert_eq!(saved["nodes"][1]["text"], "Новая реплика");
        assert_eq!(saved["custom_field"]["keep"], true);
    }
    // An unsaved name remains an ordinary document edit; undo can move it back.
    assert!(workspace.get(character_id).unwrap().dirty());
    workspace
        .tabs
        .iter_mut()
        .find(|d| d.id == character_id)
        .unwrap()
        .undo();
    workspace
        .rename(&root, character_id, &previous_name)
        .unwrap();
    assert!(root.join(&source).exists());
    assert_eq!(
        crate::asset_paths::Redirects::read(&root)
            .unwrap()
            .resolve(&destination)
            .unwrap(),
        source
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn narrative_rename_keeps_extensions_and_never_overwrites_colliding_names() {
    let root = lab();
    let first = create(&root, Path::new(""), true).unwrap();
    let second = create(&root, Path::new(""), true).unwrap();
    let graph = create(&root, Path::new(""), false).unwrap();
    let mut workspace = Workspace::default();
    workspace.open(&root, first).unwrap();
    let first_id = workspace.active.unwrap();
    workspace.open(&root, second).unwrap();
    let second_id = workspace.active.unwrap();
    workspace.open(&root, graph).unwrap();
    let graph_id = workspace.active.unwrap();
    workspace.rename(&root, first_id, "Астра").unwrap();
    let original = fs::read(root.join("Астра.dcharacter")).unwrap();
    workspace.rename(&root, second_id, "Астра").unwrap();
    assert_eq!(
        workspace.get(second_id).unwrap().path,
        PathBuf::from("Астра (2).dcharacter")
    );
    assert_eq!(fs::read(root.join("Астра.dcharacter")).unwrap(), original);
    assert!(
        workspace
            .rename(&root, second_id, "Астра")
            .unwrap()
            .is_none()
    );
    workspace.rename(&root, graph_id, "Сигнал.dialog").unwrap();
    assert_eq!(
        workspace.get(graph_id).unwrap().path,
        PathBuf::from("Сигнал.dialog")
    );
    let retained = workspace.get(second_id).unwrap().path.clone();
    let redirects = fs::read(root.join(crate::asset_paths::FILE)).unwrap();
    for name in ["", "  ", "../escape", "A/B", "CON", "Bad?"] {
        assert!(workspace.rename(&root, second_id, name).is_err(), "{name}");
        assert_eq!(workspace.get(second_id).unwrap().path, retained);
        assert!(root.join(&retained).exists());
        assert_eq!(
            fs::read(root.join(crate::asset_paths::FILE)).unwrap(),
            redirects
        );
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn creating_more_characters_does_not_reuse_aliases_reserved_by_previous_renames() {
    let root = lab();
    let first = create(&root, Path::new(""), true).unwrap();
    let mut workspace = Workspace::default();
    workspace.open(&root, first.clone()).unwrap();
    let first_id = workspace.active.unwrap();
    workspace.rename(&root, first_id, "Первый").unwrap();
    let second = create(&root, Path::new(""), true).unwrap();
    assert_ne!(first, second);
    workspace.open(&root, second).unwrap();
    let second_id = workspace.active.unwrap();
    workspace.rename(&root, second_id, "Второй").unwrap();
    workspace
        .rename(
            &root,
            second_id,
            first.file_stem().unwrap().to_str().unwrap(),
        )
        .unwrap();
    assert_ne!(workspace.get(second_id).unwrap().path, first);
    assert_eq!(
        crate::asset_paths::Redirects::read(&root)
            .unwrap()
            .resolve(first)
            .unwrap(),
        PathBuf::from("Первый.dcharacter")
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn readonly_reference_rejects_rename_without_moving_character_or_live_paths() {
    let root = lab();
    let source = create(&root, Path::new(""), true).unwrap();
    let mut dialogue = Dialogue::new("Reference".into());
    dialogue.nodes[0].speaker = Some(source.to_string_lossy().into_owned());
    Content::Dialogue(dialogue)
        .save(&root, Path::new("locked.dialog"))
        .unwrap();
    let file = root.join("locked.dialog");
    let permissions = fs::metadata(&file).unwrap().permissions();
    let mut readonly = permissions.clone();
    readonly.set_readonly(true);
    fs::set_permissions(&file, readonly).unwrap();
    let mut workspace = Workspace::default();
    workspace.open(&root, source.clone()).unwrap();
    let id = workspace.active.unwrap();
    let result = workspace.rename(&root, id, "Астра");
    fs::set_permissions(&file, permissions).unwrap();
    assert!(result.is_err());
    assert_eq!(workspace.get(id).unwrap().path, source);
    assert!(root.join(&source).exists());
    assert!(!root.join("Астра.dcharacter").exists());
    assert!(!root.join(crate::asset_paths::FILE).exists());
    fs::remove_dir_all(root).unwrap();
}

#[cfg(windows)]
#[test]
fn case_only_narrative_rename_preserves_links() {
    let root = lab();
    let source = create(&root, Path::new(""), true).unwrap();
    let mut workspace = Workspace::default();
    workspace.open(&root, source).unwrap();
    let id = workspace.active.unwrap();
    workspace.rename(&root, id, "Астра").unwrap();
    workspace.rename(&root, id, "АСТРА").unwrap();
    assert_eq!(
        workspace.get(id).unwrap().path,
        PathBuf::from("АСТРА.dcharacter")
    );
    assert!(
        fs::read_dir(&root)
            .unwrap()
            .any(|entry| entry.unwrap().file_name() == "АСТРА.dcharacter")
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn legacy_dialogue_link_loads_as_a_connected_jump_node() {
    let root = lab();
    let file = Path::new("legacy.dialog");
    let mut original = Dialogue::new("Сигнал".into());
    original.nodes[0].choices[0].dialogue = Some("Дальше.dialog".into());
    Content::Dialogue(original).save(&root, file).unwrap();
    let Content::Dialogue(migrated) = Content::read(&root, file).unwrap() else {
        unreachable!();
    };
    let choice = &migrated.nodes[0].choices[0];
    assert!(choice.dialogue.is_none());
    let jump = migrated.node(choice.target.unwrap()).unwrap();
    assert_eq!(jump.kind, NodeKind::Jump);
    assert_eq!(jump.dialogue.as_deref(), Some("Дальше.dialog"));
    assert!(migrated.warnings().is_empty());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn asset_registry_preserves_portrait_after_old_filename_is_reused() {
    let root = lab();
    fs::write(root.join("face#1.png"), b"portrait").unwrap();
    let character = create(&root, Path::new(""), true).unwrap();
    let mut scan = crate::asset_registry::scan::Scanner::new(&root).unwrap();
    scan.run(None).unwrap();
    Content::Character(Character {
        version: VERSION,
        name: "Nina".into(),
        description: String::new(),
        portraits: vec![Portrait {
            uid: None,
            label: "Portrait".into(),
            path: "face#1.png".into(),
        }],
    })
    .save(&root, &character)
    .unwrap();
    let bytes = fs::read(root.join(&character)).unwrap();
    let saved: Character = serde_json::from_slice(&bytes).unwrap();
    assert!(saved.portraits[0].uid.is_some());
    fs::rename(root.join("face#1.png"), root.join("new#2.png")).unwrap();
    fs::write(root.join("face#1.png"), b"other face").unwrap();
    scan.run(None).unwrap();
    let Content::Character(loaded) = Content::read(&root, &character).unwrap() else {
        panic!()
    };
    assert_eq!(loaded.portraits[0].path, "new#2.png");
    assert_eq!(loaded.portraits[0].uid, saved.portraits[0].uid);
    assert_eq!(fs::read(root.join(&character)).unwrap(), bytes);
    fs::remove_dir_all(root).unwrap();
}
