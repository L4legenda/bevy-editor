//! Discoverable per-line portrait selection from the speaking character's images.
use super::*;

/// Invalid and repeated choices cannot add history entries or corrupt the saved index.
pub(super) fn select(editor: &mut Editor, index: usize) -> Option<bool> {
    let doc = editor.narrative.current()?;
    let Content::Dialogue(d) = &doc.content else {
        return None;
    };
    let n = d.node(doc.selected).filter(|n| n.kind == NodeKind::Line)?;
    let c = character(editor, n.speaker.as_deref()?)?;
    c.portraits.get(index)?;
    if n.portrait == index {
        return Some(false);
    }
    let doc = editor.narrative.current_mut()?;
    doc.checkpoint();
    if let Content::Dialogue(d) = &mut doc.content {
        d.node_mut(doc.selected)?.portrait = index;
    }
    Some(true)
}
