use super::*;

#[test]
fn unicode_find_replace_keeps_original_utf8_boundaries() {
    let value = "ПРИВЕТ, ёж! Привет, İ🙂!";
    let found = matches(value, "привет", false);
    assert_eq!(found.len(), 2);
    assert_eq!(&value[found[0].clone()], "ПРИВЕТ");
    assert_eq!(&value[found[1].clone()], "Привет");
    assert!(matches(value, "привет", true).is_empty());
    let expanded = matches(value, "i", false);
    assert_eq!(&value[expanded[0].clone()], "İ");
    assert_eq!(&value[matches(value, "🙂", false)[0].clone()], "🙂");
    assert!(matches(value, "", false).is_empty());
}

#[test]
fn lists_transform_only_selected_lines_and_roundtrip() {
    let draft = Draft {
        text: "До\nПервая\nВторая\nПосле".into(),
        format: TextFormat::default(),
        spans: Vec::new(),
        selection: 5..28,
    };
    let first = draft.text.find("Первая").unwrap();
    let end = draft.text.find("После").unwrap();
    let mut draft = draft;
    draft.selection = first..end;
    let numbered = toggle_list(&draft, true);
    assert_eq!(numbered.text, "До\n1. Первая\n2. Вторая\nПосле");
    let bullet = toggle_list(&numbered, false);
    assert_eq!(bullet.text, "До\n• Первая\n• Вторая\nПосле");
    assert_eq!(toggle_list(&bullet, false).text, draft.text);
}

#[test]
fn formatting_is_optional_and_survives_document_roundtrip() {
    let mut dialogue = Dialogue::new("Проверка".into());
    let id = dialogue.add(NodeKind::Line, 0);
    let plain = serde_json::to_value(&dialogue).unwrap();
    assert!(plain["nodes"][1].get("text_format").is_none());
    let mut legacy: Dialogue = serde_json::from_value(plain).unwrap();
    assert_eq!(legacy.node(id).unwrap().text_format, TextFormat::default());
    legacy.node_mut(id).unwrap().text_format = TextFormat {
        font_size: 26.0,
        bold: true,
        italic: true,
        underline: true,
        strikethrough: true,
        alignment: TextAlign::Center,
        line_height: 1.5,
        color: Some([127, 183, 255]),
    };
    let stored = serde_json::to_vec(&legacy).unwrap();
    assert_eq!(serde_json::from_slice::<Dialogue>(&stored).unwrap(), legacy);
}

#[test]
fn draft_history_groups_typing_and_keeps_formatting_separate() {
    let mut dialogue = Dialogue::new("История".into());
    let id = dialogue.add(NodeKind::Line, 0);
    let mut session = Session::new(0, dialogue.node(id).unwrap());
    let original = session.draft.clone();
    for (text, now) in [("Первое", 1.0), ("Первое слово", 1.2)] {
        let mut next = session.draft.clone();
        next.text = text.into();
        next.selection = text.len()..text.len();
        session.edit(next, Some(Input::Body), now);
    }
    assert_eq!(session.undo.len(), 1);
    let mut next = session.draft.clone();
    next.format.bold = true;
    session.edit(next, None, 1.3);
    session.history(false);
    assert_eq!(session.draft.text, "Первое слово");
    assert!(!session.draft.format.bold);
    session.history(false);
    assert!(session.draft.same_content(&original));
    session.history(true);
    session.history(true);
    assert!(session.draft.format.bold);
    assert_eq!(session.draft.text, "Первое слово");
}

#[test]
fn font_validation_rejects_invalid_values() {
    let format = TextFormat {
        font_size: f32::NAN,
        line_height: f32::INFINITY,
        ..default()
    }
    .normalized();
    assert_eq!(format, TextFormat::default());
}

#[test]
fn character_formatting_is_limited_to_selection_and_roundtrips() {
    let mut dialogue = Dialogue::new("Фрагменты".into());
    let id = dialogue.add(NodeKind::Line, 0);
    dialogue.node_mut(id).unwrap().text = "До 🙂 после".into();
    let mut session = Session::new(0, dialogue.node(id).unwrap());
    let mut next = session.draft.clone();
    next.paint(|f| f.bold = true);
    assert!(next.same_content(&session.draft)); // caret never formats the entire reply
    let start = next.text.find('🙂').unwrap();
    next.selection = start..start + 4;
    next.paint(|f| {
        f.bold = true;
        f.font_size = 32.;
        f.color = Some([113, 209, 176]);
    });
    let runs = rich::runs(&next.text, &next.format, &next.spans);
    assert_eq!(runs.len(), 3);
    assert_eq!(&next.text[runs[1].start..runs[1].end], "🙂");
    assert!(!runs[0].format.bold && !runs[2].format.bold);
    session.edit(next, None, 1.);
    session.history(false);
    assert!(session.draft.spans.is_empty());
    session.history(true);
    assert_eq!(session.draft.spans.len(), 1);
    dialogue.node_mut(id).unwrap().text_spans = session.draft.spans;
    assert_eq!(
        serde_json::from_slice::<Dialogue>(&serde_json::to_vec(&dialogue).unwrap()).unwrap(),
        dialogue
    );
}

#[test]
fn typing_replacement_and_list_prefixes_preserve_unicode_styles() {
    let mut draft = Draft {
        text: "До\nслово\nПосле".into(),
        format: TextFormat::default(),
        spans: Vec::new(),
        selection: 0..0,
    };
    let start = draft.text.find("слово").unwrap();
    let end = start + "слово".len();
    draft.selection = start..end;
    draft.paint(|f| f.italic = true);
    let numbered = toggle_list(&draft, true);
    let styled = rich::runs(&numbered.text, &numbered.format, &numbered.spans)
        .into_iter()
        .find(|s| s.format.italic)
        .unwrap();
    assert!(numbered.text[styled.start..styled.end].contains("слово"));
    assert!(numbered.text.ends_with("\nПосле"));
    draft.replace(start + 2..start + 2, "🙂");
    assert!(
        draft
            .spans
            .iter()
            .any(|s| draft.text[s.start..s.end].contains('🙂') && s.format.italic)
    );
    let new = format!("Пролог\n{}", draft.text);
    let mapped = rich::remap(&draft.text, &new, &draft.format, &draft.spans);
    let run = &mapped[0];
    assert_eq!(&new[run.start..run.end], "с🙂лово");
    let malformed = vec![rich::TextSpan {
        start: 1,
        end: usize::MAX,
        format: TextFormat {
            bold: true,
            ..default()
        },
    }];
    for span in rich::compact("🙂я", &draft.format, &malformed) {
        assert!("🙂я".is_char_boundary(span.start) && "🙂я".is_char_boundary(span.end));
    }
}

#[test]
fn colour_dialog_is_transactional_and_keeps_its_original_selection() {
    use crate::editor::ui::image_viewer::edit::{ColorTarget, color::Picker};
    use color_menu::{PickerAction, apply_action};
    let mut dialogue = Dialogue::new("Цвет".into());
    let id = dialogue.add(NodeKind::Line, 0);
    let mut session = Session::new(0, dialogue.node(id).unwrap());
    session.color_selection = 0.."Новая".len();
    session.picker = Some(Picker::new(ColorTarget::Foreground, [1.; 4]));
    apply_action(&mut session, PickerAction::Draft([0., 1., 0., 1.]), 1.);
    assert!(!session.dirty());
    apply_action(&mut session, PickerAction::Cancel, 1.);
    assert!(!session.dirty());
    session.picker = Some(Picker::new(ColorTarget::Foreground, [0., 1., 0., 1.]));
    apply_action(&mut session, PickerAction::Confirm, 2.);
    assert_eq!(session.draft.spans.len(), 1);
    assert_eq!(session.draft.spans[0].end, "Новая".len());
    assert_eq!(session.draft.spans[0].format.color, Some([0, 255, 0]));
    assert_eq!(session.undo.len(), 1);
}
