//! Portable narrative documents. Paths are relative to the project's asset root.
use super::*;
use serde::{Deserialize, Serialize};
#[path = "dialogue_text.rs"]
pub(super) mod text;
use std::{
    collections::HashSet,
    fs,
    io::{self, Write},
    path::Path,
};
pub(super) use text::{TextAlign, TextFormat};

#[path = "dialogue_conditions.rs"]
pub(super) mod conditions;
pub(super) use conditions::{Condition, OTHERWISE};

pub(super) const VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(super) struct Dialogue {
    pub version: u32,
    pub title: String,
    pub nodes: Vec<DialogueNode>,
    #[serde(flatten)]
    pub extensions: std::collections::BTreeMap<String, serde_json::Value>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum NodeKind {
    Start,
    Line,
    End,
    Jump,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(super) struct DialogueNode {
    pub id: u32,
    pub kind: NodeKind,
    pub position: [f32; 2],
    #[serde(default)]
    pub speaker: Option<String>,
    #[serde(default)]
    pub portrait: usize,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dialogue: Option<String>,
    #[serde(default, skip_serializing_if = "TextFormat::is_default")]
    pub text_format: TextFormat,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub text_spans: Vec<text::TextSpan>,
    pub choices: Vec<Choice>,
    #[serde(default, skip_serializing_if = "Condition::is_empty")]
    pub condition: Condition,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub otherwise: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(super) struct Choice {
    pub text: String,
    #[serde(default, skip_serializing_if = "Condition::is_empty")]
    pub condition: Condition,
    pub target: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dialogue: Option<String>,
}

impl DialogueNode {
    pub fn has_otherwise(&self) -> bool {
        self.kind == NodeKind::Jump || self.kind == NodeKind::Line && !self.condition.is_empty()
    }

    pub fn output_exists(&self, index: usize) -> bool {
        if index == OTHERWISE {
            self.has_otherwise()
        } else {
            self.choices.get(index).is_some()
        }
    }

    pub fn output_target(&self, index: usize) -> Option<u32> {
        if index == OTHERWISE {
            self.otherwise
        } else {
            self.choices.get(index)?.target
        }
    }

    pub fn output_connected(&self, index: usize) -> bool {
        self.output_target(index).is_some()
            || self.choices.get(index).is_some_and(Choice::connected)
    }

    pub fn output_targets(&self) -> impl Iterator<Item = (usize, u32)> + '_ {
        self.choices
            .iter()
            .enumerate()
            .filter_map(|(i, c)| c.target.map(|target| (i, target)))
            .chain(self.otherwise.map(|target| (OTHERWISE, target)))
    }

    pub fn set_output(&mut self, index: usize, target: Option<u32>) {
        if index == OTHERWISE {
            self.otherwise = target;
        } else if let Some(choice) = self.choices.get_mut(index) {
            choice.target = target;
            choice.dialogue = None;
        }
    }
}

impl Choice {
    pub fn connected(&self) -> bool {
        self.target.is_some() || self.dialogue.is_some()
    }
}

impl Dialogue {
    pub fn new(title: String) -> Self {
        Self {
            version: VERSION,
            title,
            extensions: Default::default(),
            nodes: vec![DialogueNode {
                id: 0,
                kind: NodeKind::Start,
                position: [64.0, 180.0],
                speaker: None,
                portrait: 0,
                text: String::new(),
                dialogue: None,
                text_format: TextFormat::default(),
                text_spans: Vec::new(),
                condition: Condition::default(),
                otherwise: None,
                choices: vec![Choice {
                    text: "Начать".into(),
                    target: None,
                    condition: Default::default(),
                    dialogue: None,
                }],
            }],
        }
    }
    pub fn node(&self, id: u32) -> Option<&DialogueNode> {
        self.nodes.iter().find(|n| n.id == id)
    }
    pub fn node_mut(&mut self, id: u32) -> Option<&mut DialogueNode> {
        self.nodes.iter_mut().find(|n| n.id == id)
    }
    pub fn add(&mut self, kind: NodeKind, from: u32) -> u32 {
        let id = self
            .nodes
            .iter()
            .map(|n| n.id)
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .unwrap_or_else(|| (1..u32::MAX).find(|id| self.node(*id).is_none()).unwrap());
        let source = self.node(from);
        let mut position = source.map_or([380.0, 180.0], |n| {
            [
                n.position[0] + if kind == NodeKind::Jump { 440.0 } else { 320.0 },
                n.position[1],
            ]
        });
        while self.nodes.iter().any(|n| {
            (n.position[0] - position[0]).abs() < if kind == NodeKind::Jump { 380.0 } else { 260.0 }
                && (n.position[1] - position[1]).abs() < 200.0
        }) {
            position[1] += 230.0;
        }
        if kind != NodeKind::Jump
            && let Some(source) = self.node_mut(from)
        {
            if let Some(choice) = source.choices.iter_mut().find(|c| !c.connected()) {
                choice.target = Some(id);
            }
        }
        self.nodes.push(DialogueNode {
            id,
            kind,
            position,
            speaker: None,
            portrait: 0,
            text: if kind == NodeKind::End {
                "Конец разговора"
            } else {
                "Новая реплика"
            }
            .into(),
            dialogue: None,
            text_format: TextFormat::default(),
            text_spans: Vec::new(),
            condition: Condition::default(),
            otherwise: None,
            choices: if kind == NodeKind::Line {
                vec![Choice {
                    text: "Продолжить".into(),
                    target: None,
                    condition: Default::default(),
                    dialogue: None,
                }]
            } else {
                vec![]
            },
        });
        id
    }
    fn migrate_jumps(&mut self) {
        let links: Vec<_> = self
            .nodes
            .iter()
            .flat_map(|node| {
                node.choices.iter().enumerate().filter_map(|(i, choice)| {
                    choice.dialogue.clone().map(|path| (node.id, i, path))
                })
            })
            .take(2000usize.saturating_sub(self.nodes.len()))
            .collect();
        for (from, index, path) in links {
            let id = self.add(NodeKind::Jump, from);
            self.node_mut(id).unwrap().dialogue = Some(path);
            let choice = &mut self.node_mut(from).unwrap().choices[index];
            choice.dialogue = None;
            choice.target = Some(id);
        }
    }

    pub fn delete(&mut self, id: u32) {
        if self.node(id).is_none_or(|n| n.kind == NodeKind::Start) {
            return;
        }
        self.nodes.retain(|n| n.id != id);
        for node in &mut self.nodes {
            if node.otherwise == Some(id) {
                node.otherwise = None;
            }
            for choice in &mut node.choices {
                if choice.target == Some(id) {
                    choice.target = None;
                }
            }
        }
    }
    /// Cycles are legal; incomplete and unreachable branches remain editable.
    pub fn warnings(&self) -> Vec<String> {
        let mut result = Vec::new();
        let mut reachable = HashSet::new();
        let mut pending: Vec<_> = self
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Start)
            .map(|n| n.id)
            .collect();
        while let Some(id) = pending.pop() {
            if !reachable.insert(id) {
                continue;
            }
            if let Some(n) = self.node(id) {
                pending.extend(n.output_targets().map(|(_, target)| target));
            }
        }
        for n in &self.nodes {
            if !reachable.contains(&n.id) {
                result.push(format!("Реплика #{} недоступна из начала", n.id));
            }
            if n.kind == NodeKind::Jump && n.dialogue.is_none() {
                result.push(format!("Переход #{}: выберите диалог", n.id));
            }
            if matches!(n.kind, NodeKind::Start | NodeKind::Line) && n.choices.is_empty() {
                result.push(format!("Узел #{}: добавьте переход или завершение", n.id));
            }
            if n.kind == NodeKind::Line
                && !n.choices.is_empty()
                && n.choices.iter().all(|c| !c.condition.is_empty())
            {
                result.push(format!(
                    "Реплика #{}: оставьте запасной ответ без условия",
                    n.id
                ));
            }
            if n.kind == NodeKind::Line && n.has_otherwise() && n.otherwise.is_none() {
                result.push(format!("Реплика #{}: подключите выход «Иначе»", n.id));
            }
            for (i, c) in n.choices.iter().enumerate() {
                if !c.connected() {
                    result.push(format!("Узел #{} · ответ {}: нет перехода", n.id, i + 1));
                }
            }
        }
        result
    }
    fn validate(&self) -> Result<(), String> {
        if self.version != VERSION {
            return Err("Неподдерживаемая версия .dialog".into());
        }
        if self.nodes.len() > 2000 {
            return Err("В диалоге больше 2000 узлов".into());
        }
        if self
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Start)
            .count()
            != 1
        {
            return Err("В диалоге должен быть один узел начала".into());
        }
        let ids: HashSet<_> = self.nodes.iter().map(|n| n.id).collect();
        if ids.len() != self.nodes.len() {
            return Err("Повторяющиеся идентификаторы узлов".into());
        }
        for n in &self.nodes {
            if n.position
                .iter()
                .any(|v| !v.is_finite() || v.abs() > 1_000_000.0)
            {
                return Err("Некорректное положение узла".into());
            }
            if n.choices
                .iter()
                .any(|c| c.target.is_some_and(|id| !ids.contains(&id)))
            {
                return Err("Переход ссылается на отсутствующий узел".into());
            }
            if n.choices.iter().any(|c| {
                c.dialogue
                    .as_ref()
                    .is_some_and(|path| c.target.is_some() || !valid_dialogue_path(path))
            }) {
                return Err("Переход должен указывать на один файл .dialog внутри ассетов".into());
            }
            if n.otherwise
                .is_some_and(|id| !n.has_otherwise() || !ids.contains(&id))
            {
                return Err("Некорректная запасная связь «Иначе»".into());
            }
            if n.dialogue
                .as_ref()
                .is_some_and(|path| n.kind != NodeKind::Jump || !valid_dialogue_path(path))
            {
                return Err("Некорректный файл перехода".into());
            }
            if matches!(n.kind, NodeKind::End | NodeKind::Jump) && !n.choices.is_empty() {
                return Err("Узел без выходов не может иметь ответов".into());
            }
            if n.speaker.as_ref().is_some_and(|p| !safe_relative(p)) {
                return Err("Путь персонажа должен быть относительно ассетов".into());
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(super) struct Character {
    pub version: u32,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub portraits: Vec<Portrait>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(super) struct Portrait {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uid: Option<String>,
    pub label: String,
    pub path: String,
}

fn valid_dialogue_path(path: &str) -> bool {
    safe_relative(path)
        && Path::new(path)
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("dialog"))
}

pub(super) fn safe_relative(path: &str) -> bool {
    !path.is_empty()
        && !path.contains(':')
        && !path.starts_with(['/', '\\'])
        && path
            .split(['/', '\\'])
            .all(|s| !s.is_empty() && s != ".." && s != ".")
}

#[derive(Clone, Debug, PartialEq)]
pub(super) enum Content {
    Dialogue(Dialogue),
    Character(Character),
}
impl Content {
    pub fn title(&self) -> &str {
        match self {
            Self::Dialogue(d) => &d.title,
            Self::Character(c) => &c.name,
        }
    }

    pub fn remap_character(&mut self, from: &Path, to: &Path) {
        if let Self::Character(character) = self {
            for portrait in &mut character.portraits {
                if Path::new(&portrait.path) == from {
                    portrait.path = to.to_string_lossy().replace('\\', "/");
                }
            }
        }
        if let Self::Dialogue(dialogue) = self {
            let from = from.to_string_lossy().replace('\\', "/");
            let to = to.to_string_lossy().replace('\\', "/");
            for node in &mut dialogue.nodes {
                if node
                    .speaker
                    .as_ref()
                    .is_some_and(|path| same_asset_path(path, &from))
                {
                    node.speaker = Some(to.clone());
                }
            }
        }
    }
    pub fn read(root: &Path, relative: &Path) -> Result<Self, String> {
        let path = checked_path(root, relative)?;
        if fs::metadata(&path).map_err(|e| e.to_string())?.len() > 8 * 1024 * 1024 {
            return Err("Документ превышает 8 МБ".into());
        }
        let bytes = fs::read(path).map_err(|e| e.to_string())?;
        let mut content = if relative
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("dialog"))
        {
            Self::Dialogue(serde_json::from_slice(&bytes).map_err(|e| e.to_string())?)
        } else if relative
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("dcharacter"))
        {
            Self::Character(serde_json::from_slice(&bytes).map_err(|e| e.to_string())?)
        } else {
            return Err("Ожидается .dialog или .dcharacter".into());
        };
        content.validate()?;
        if let Self::Character(c) = &mut content {
            let index = crate::asset_registry::index(root).map_err(|e| e.to_string())?;
            for portrait in &mut c.portraits {
                index.read_ref(&mut portrait.path, &mut portrait.uid);
            }
        }
        if let Self::Dialogue(d) = &mut content {
            d.migrate_jumps();
        }
        Ok(content)
    }
    fn validate(&self) -> Result<(), String> {
        match self {
            Self::Dialogue(d) => d.validate(),
            Self::Character(c) => {
                if c.version != VERSION {
                    return Err("Неподдерживаемая версия .dcharacter".into());
                }
                if c.portraits.iter().any(|p| !safe_relative(&p.path)) {
                    return Err("Портрет должен находиться внутри ассетов проекта".into());
                }
                Ok(())
            }
        }
    }
    pub fn save(&self, root: &Path, relative: &Path) -> Result<(), String> {
        self.validate()?;
        let path = checked_path(root, relative)?;
        let bytes = match self {
            Self::Dialogue(d) => serde_json::to_vec_pretty(d),
            Self::Character(c) => {
                let mut c = c.clone();
                let index = crate::asset_registry::index(root).map_err(|e| e.to_string())?;
                for portrait in &mut c.portraits {
                    index.save_ref(&mut portrait.path, &mut portrait.uid);
                }
                serde_json::to_vec_pretty(&c)
            }
        }
        .map_err(|e| e.to_string())?;
        let temp = path.with_extension(format!(
            "{}.tmp",
            relative.extension().unwrap_or_default().to_string_lossy()
        ));
        let mut owned_temp = false;
        let result = (|| -> io::Result<()> {
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temp)?;
            owned_temp = true;
            file.write_all(&bytes)?;
            file.sync_all()?;
            drop(file);
            fs::rename(&temp, &path)
        })();
        if result.is_err() && owned_temp {
            let _ = fs::remove_file(&temp);
        }
        result.map_err(|e| e.to_string())
    }
}
fn checked_path(root: &Path, relative: &Path) -> Result<PathBuf, String> {
    if !safe_relative(&relative.to_string_lossy()) {
        return Err("Недопустимый путь документа".into());
    }
    let root = root.canonicalize().map_err(|e| e.to_string())?;
    let path = root.join(relative);
    let parent = path
        .parent()
        .ok_or("Нет папки документа")?
        .canonicalize()
        .map_err(|e| e.to_string())?;
    if !parent.starts_with(&root)
        || (path.exists()
            && !path
                .canonicalize()
                .map_err(|e| e.to_string())?
                .starts_with(&root))
    {
        return Err("Документ находится вне проекта".into());
    }
    Ok(path)
}

pub(super) struct Document {
    pub id: usize,
    pub path: PathBuf,
    pub filename_title: String,
    pub content: Content,
    pub saved: Content,
    pub undo: Vec<Content>,
    pub redo: Vec<Content>,
    pub selected: u32,
    pub pan: Vec2,
    pub zoom: f32,
}
impl Document {
    pub fn dirty(&self) -> bool {
        self.content != self.saved
    }
    pub fn checkpoint(&mut self) {
        self.undo.push(self.content.clone());
        if self.undo.len() > HISTORY_LIMIT {
            self.undo.remove(0);
        }
        self.redo.clear();
    }
    pub fn undo(&mut self) {
        if let Some(c) = self.undo.pop() {
            self.redo.push(std::mem::replace(&mut self.content, c));
        }
    }
    pub fn redo(&mut self) {
        if let Some(c) = self.redo.pop() {
            self.undo.push(std::mem::replace(&mut self.content, c));
        }
    }
}
#[derive(Default)]
pub(super) struct Workspace {
    pub active: Option<usize>,
    pub tabs: Vec<Document>,
    next_id: usize,
}
impl Workspace {
    pub fn get(&self, id: usize) -> Option<&Document> {
        self.tabs.iter().find(|d| d.id == id)
    }
    pub fn current(&self) -> Option<&Document> {
        self.get(self.active?)
    }
    pub fn current_mut(&mut self) -> Option<&mut Document> {
        let id = self.active?;
        self.tabs.iter_mut().find(|d| d.id == id)
    }
    pub fn open(&mut self, root: &Path, path: PathBuf) -> Result<(), String> {
        if let Some(d) = self.tabs.iter().find(|d| d.path == path) {
            self.active = Some(d.id);
            return Ok(());
        }
        let content = Content::read(root, &path)?;
        let selected = match &content {
            Content::Dialogue(d) => {
                d.nodes
                    .iter()
                    .find(|n| n.kind == NodeKind::Start)
                    .unwrap()
                    .id
            }
            _ => 0,
        };
        let id = self.next_id;
        self.next_id += 1;
        self.tabs.push(Document {
            id,
            path,
            filename_title: content.title().to_owned(),
            saved: content.clone(),
            content,
            undo: vec![],
            redo: vec![],
            selected,
            pan: Vec2::new(24.0, 24.0),
            zoom: 1.0,
        });
        self.active = Some(id);
        Ok(())
    }
    pub fn close(&mut self, id: usize) {
        if let Some(i) = self.tabs.iter().position(|d| d.id == id) {
            self.tabs.remove(i);
            if self.active == Some(id) {
                self.active = self
                    .tabs
                    .get(i.min(self.tabs.len().saturating_sub(1)))
                    .map(|d| d.id);
            }
        }
    }

    /// Changing an authored name moves the file without saving other edits.
    /// All live history snapshots follow the same physical character resource.
    pub fn rename(
        &mut self,
        root: &Path,
        id: usize,
        title: &str,
    ) -> Result<Option<(PathBuf, PathBuf)>, String> {
        let doc = self.get(id).ok_or("Документ уже закрыт")?;
        let from = doc.path.clone();
        let extension = from.extension().unwrap_or_default().to_string_lossy();
        let mut stem = title.trim();
        let suffix = format!(".{extension}");
        if stem.to_lowercase().ends_with(&suffix.to_lowercase()) {
            stem = &stem[..stem.len() - suffix.len()];
        }
        if stem.is_empty() {
            return Err("Введите имя персонажа или название диалога".into());
        }
        super::browser_files::validate_name(stem).map_err(str::to_owned)?;
        let parent = from.parent().unwrap_or(Path::new(""));
        let source = checked_path(root, &from)?;
        let redirects = crate::asset_paths::Redirects::read(root).map_err(|e| e.to_string())?;
        let mut destination = None;
        for index in 1..10000 {
            let filename = if index == 1 {
                format!("{stem}.{extension}")
            } else {
                format!("{stem} ({index}).{extension}")
            };
            let path = parent.join(filename);
            if path == from {
                destination = Some(path);
                break;
            }
            let resolved = redirects.resolve(&path).map_err(|e| e.to_string())?;
            if resolved != path && resolved != from {
                continue;
            }
            if self.tabs.iter().any(|d| {
                d.id != id && same_asset_path(&d.path.to_string_lossy(), &path.to_string_lossy())
            }) {
                continue;
            }
            let absolute = checked_path(root, &path)?;
            if absolute.exists() {
                // Windows preserves casing but addresses the same file through either spelling.
                if source.canonicalize().ok() == absolute.canonicalize().ok() {
                    destination = Some(path);
                    break;
                }
                continue;
            }
            let meta = path.with_extension(format!("{extension}.meta"));
            if root.join(meta).exists() {
                continue;
            }
            destination = Some(path);
            break;
        }
        let to = destination.ok_or("Не удалось подобрать свободное имя файла")?;
        if from == to {
            self.tabs
                .iter_mut()
                .find(|d| d.id == id)
                .unwrap()
                .filename_title = title.to_owned();
            return Ok(None);
        }
        super::file_moves::rename_narrative(root, &from, &to).map_err(|e| e.to_string())?;
        for doc in &mut self.tabs {
            if doc.id == id {
                doc.path = to.clone();
                doc.filename_title = title.to_owned();
            }
            for content in std::iter::once(&mut doc.content)
                .chain(std::iter::once(&mut doc.saved))
                .chain(&mut doc.undo)
                .chain(&mut doc.redo)
            {
                content.remap_character(&from, &to);
            }
        }
        Ok(Some((from, to)))
    }
}

pub(super) fn same_asset_path(a: &str, b: &str) -> bool {
    let a = a.replace('\\', "/");
    let b = b.replace('\\', "/");
    if cfg!(windows) {
        a.to_lowercase() == b.to_lowercase()
    } else {
        a == b
    }
}

pub(super) fn create(root: &Path, parent: &Path, character: bool) -> Result<PathBuf, String> {
    let redirects = crate::asset_paths::Redirects::read(root).map_err(|e| e.to_string())?;
    let (base, ext) = if character {
        ("Новый персонаж", "dcharacter")
    } else {
        ("Новый диалог", "dialog")
    };
    for i in 1..10000 {
        let stem = if i == 1 {
            base.to_string()
        } else {
            format!("{base} {i}")
        };
        let path = parent.join(format!("{stem}.{ext}"));
        if redirects.resolve(&path).map_err(|e| e.to_string())? != path {
            continue;
        }
        let absolute = checked_path(root, &path)?;
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(absolute)
        {
            Ok(mut file) => {
                let bytes = if character {
                    serde_json::to_vec_pretty(&Character {
                        version: VERSION,
                        name: stem,
                        description: String::new(),
                        portraits: vec![],
                    })
                } else {
                    serde_json::to_vec_pretty(&Dialogue::new(stem))
                }
                .map_err(|e| e.to_string())?;
                file.write_all(&bytes)
                    .and_then(|_| file.sync_all())
                    .map_err(|e| e.to_string())?;
                return Ok(path);
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.to_string()),
        }
    }
    Err("Не удалось подобрать свободное имя".into())
}

#[cfg(test)]
#[path = "dialogue_tests.rs"]
mod tests;
