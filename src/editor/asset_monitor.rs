//! Keep native filesystem discovery off the Bevy update thread.
use super::{Editor, browser::Browser, file_moves};
use crate::asset_registry::{
    self,
    scan::{self, Changes, Scanner},
};
use bevy::prelude::*;
use notify::{
    EventKind, RecursiveMode, Watcher,
    event::{ModifyKind, RenameMode},
};
use std::{
    collections::BTreeSet,
    fs, io,
    path::{Path, PathBuf},
    sync::{Mutex, mpsc},
    time::{Duration, Instant},
};

struct BrowserSnapshot {
    browser: Browser,
    project: crate::EditorProject,
    catalog_len: usize,
    scene_len: usize,
    revision: u64,
}
enum Work {
    Files(Vec<PathBuf>),
    Rename(String, String),
    Full,
    Locate(String),
    Selected(String, io::Result<Option<String>>),
    Stop,
}
enum ResultEvent {
    Indexed(Changes),
    Error(String),
    PickerDone,
}
#[derive(Resource)]
pub(super) struct Monitor {
    tx: mpsc::Sender<Work>,
    rx: Mutex<mpsc::Receiver<ResultEvent>>,
    panel: Option<Entity>,
    missing: Vec<(String, String)>,
    current: usize,
    picking: bool,
    refresh_needed: bool,
    build: Option<bevy::tasks::Task<BrowserSnapshot>>,
}
impl Drop for Monitor {
    fn drop(&mut self) {
        let _ = self.tx.send(Work::Stop);
    }
}
impl Monitor {
    pub(super) fn start(
        root: PathBuf,
        initial: BTreeSet<PathBuf>,
        priority: BTreeSet<PathBuf>,
    ) -> Self {
        let (tx, work) = mpsc::channel();
        let (reply, rx) = mpsc::channel();
        let events = tx.clone();
        std::thread::spawn(move || {
            if let Err(e) = worker(&root, initial, priority, events, work, &reply) {
                let _ = reply.send(ResultEvent::Error(e.to_string()));
            }
        });
        Self {
            tx,
            rx: Mutex::new(rx),
            panel: None,
            missing: vec![],
            current: 0,
            picking: false,
            refresh_needed: false,
            build: None,
        }
    }
}
fn worker(
    root: &Path,
    initial: BTreeSet<PathBuf>,
    priority: BTreeSet<PathBuf>,
    events: mpsc::Sender<Work>,
    work: mpsc::Receiver<Work>,
    reply: &mpsc::Sender<ResultEvent>,
) -> io::Result<()> {
    let base = root.to_owned();
    let picker_events = events.clone();
    #[cfg(windows)]
    let mut rename_from: Option<PathBuf> = None;
    let mut watcher =
        notify::recommended_watcher(move |event: notify::Result<notify::Event>| match event {
            Ok(e) if e.need_rescan() => {
                let _ = events.send(Work::Full);
            }
            Ok(e) if !matches!(e.kind, EventKind::Access(_)) => {
                let paths: Vec<_> = e
                    .paths
                    .iter()
                    .filter_map(|p| p.strip_prefix(&base).ok())
                    .filter(|p| scan::relevant(p))
                    .map(Path::to_owned)
                    .collect();
                let paired = match (&e.kind, paths.as_slice()) {
                    (EventKind::Modify(ModifyKind::Name(RenameMode::Both)), [from, to]) => {
                        Some((from.clone(), to.clone()))
                    }
                    #[cfg(windows)]
                    (EventKind::Modify(ModifyKind::Name(RenameMode::From)), [from]) => {
                        rename_from = Some(from.clone());
                        None
                    }
                    #[cfg(windows)]
                    (EventKind::Modify(ModifyKind::Name(RenameMode::To)), [to]) => rename_from
                        .take()
                        .filter(|from| from.parent() == to.parent())
                        .map(|from| (from, to.clone())),
                    _ => {
                        #[cfg(windows)]
                        {
                            rename_from = None;
                        }
                        None
                    }
                };
                if let Some((from, to)) = paired {
                    let portable =
                        |p: PathBuf| p.to_string_lossy().replace(std::path::MAIN_SEPARATOR, "/");
                    let _ = events.send(Work::Rename(portable(from), portable(to)));
                }
                if !paths.is_empty() {
                    let _ = events.send(Work::Files(paths));
                }
            }
            Err(_) => {
                let _ = events.send(Work::Full);
            }
            _ => {}
        })
        .ok();
    let watching = watcher
        .as_mut()
        .is_some_and(|w| w.watch(root, RecursiveMode::Recursive).is_ok());
    let mut scanner = Scanner::new(root)?;
    let interval = if watching {
        Duration::from_secs(120)
    } else {
        Duration::from_secs(3)
    };
    let mut next = Instant::now() + interval;
    let report = |result: io::Result<Changes>| match result {
        Ok(changes) => {
            for (from, to) in &changes.companions {
                if let Err(e) = file_moves::recover_companions(root, Path::new(from), Path::new(to))
                {
                    let _ = reply.send(ResultEvent::Error(format!("{from} → {to}: {e}")));
                }
            }
            let _ = reply.send(ResultEvent::Indexed(changes));
            true
        }
        Err(e) => {
            let _ = reply.send(ResultEvent::Error(e.to_string()));
            false
        }
    };
    // On first import, publish the active scene's models before hashing unrelated textures.
    if !root.join(asset_registry::FILE).exists() && !priority.is_empty() {
        report(scanner.first_import(&priority));
    }
    if !report(scanner.initial(&initial)) {
        next = Instant::now() + Duration::from_secs(1);
    }
    loop {
        let first = match work.recv_timeout(next.saturating_duration_since(Instant::now())) {
            Ok(msg) => msg,
            Err(mpsc::RecvTimeoutError::Timeout) => Work::Full,
            Err(_) => break,
        };
        let mut paths = BTreeSet::new();
        let mut moves = vec![];
        let mut full = Instant::now() >= next;
        let mut messages = vec![first];
        // Let both halves of Explorer's move and atomic saves reach the queue.
        if matches!(messages[0], Work::Files(_) | Work::Rename(..)) {
            std::thread::sleep(Duration::from_millis(180));
        }
        messages.extend(work.try_iter());
        for msg in messages {
            match msg {
                Work::Stop => return Ok(()),
                Work::Files(files) => paths.extend(files),
                Work::Rename(from, to) => {
                    paths.insert(PathBuf::from(&from));
                    paths.insert(PathBuf::from(&to));
                    moves.push((from, to));
                }
                Work::Full => full = true,
                Work::Locate(uid) => {
                    let root = root.to_owned();
                    let sender = picker_events.clone();
                    std::thread::spawn(move || {
                        let _ = sender.send(Work::Selected(uid, choose_file(&root)));
                    });
                }
                Work::Selected(uid, result) => {
                    match result {
                        Ok(Some(path)) => {
                            if report(
                                scanner.run(Some(&[PathBuf::from(&path)].into_iter().collect())),
                            ) {
                                report(scanner.relink(&uid, &path));
                            }
                        }
                        Ok(None) => {}
                        Err(e) => {
                            let _ = reply.send(ResultEvent::Error(e.to_string()));
                        }
                    }
                    let _ = reply.send(ResultEvent::PickerDone);
                }
            }
        }
        if full || !paths.is_empty() {
            if full {
                next = Instant::now() + interval;
            }
            if !report(scanner.events(if full { None } else { Some(&paths) }, &moves)) {
                next = Instant::now() + Duration::from_secs(1);
            }
        }
    }
    Ok(())
}
fn choose_file(root: &Path) -> io::Result<Option<String>> {
    let Some(path) = choose_file_path(root)? else {
        return Ok(None);
    };
    let relative = path
        .strip_prefix(root.canonicalize()?)
        .map_err(|_| io::Error::other("Выберите файл внутри папки ассетов"))?;
    Ok(Some(
        relative
            .to_str()
            .ok_or_else(|| io::Error::other("Invalid asset filename"))?
            .replace('\\', "/"),
    ))
}

pub(super) fn choose_file_path(root: &Path) -> io::Result<Option<PathBuf>> {
    use std::process::Command;
    #[cfg(windows)]
    let output = {
        use std::os::windows::process::CommandExt;
        Command::new("powershell.exe").creation_flags(0x08000000).current_dir(root).args(["-NoProfile","-STA","-Command","[Console]::OutputEncoding=[Text.Encoding]::UTF8; Add-Type -AssemblyName System.Windows.Forms; $d=New-Object Windows.Forms.OpenFileDialog; $d.InitialDirectory=(Get-Location).Path; if($d.ShowDialog() -eq 'OK'){[Console]::Write($d.FileName)}; $d.Dispose()"]).output()?
    };
    #[cfg(target_os = "macos")]
    let output = Command::new("osascript")
        .args([
            "-e",
            "try\nreturn POSIX path of (choose file)\non error number -128\nreturn \"\"\nend try",
        ])
        .output()?;
    #[cfg(not(any(windows, target_os = "macos")))]
    let output = Command::new("zenity")
        .current_dir(root)
        .arg("--file-selection")
        .output()
        .or_else(|e| {
            if e.kind() == io::ErrorKind::NotFound {
                Command::new("kdialog")
                    .current_dir(root)
                    .args(["--getopenfilename", "."])
                    .output()
            } else {
                Err(e)
            }
        })?;
    if !output.status.success() {
        if output.status.code() == Some(1) && output.stderr.is_empty() {
            return Ok(None);
        }
        return Err(io::Error::other(
            String::from_utf8_lossy(&output.stderr).into_owned(),
        ));
    }
    let text = String::from_utf8(output.stdout).map_err(io::Error::other)?;
    if text.trim().is_empty() {
        return Ok(None);
    }
    Ok(Some(fs::canonicalize(text.trim_end_matches(['\r', '\n']))?))
}

#[derive(Component, Copy, Clone)]
pub(super) enum RepairButton {
    Locate,
    Next,
}
pub(super) fn update(
    mut commands: Commands,
    mut monitor: ResMut<Monitor>,
    mut editor: ResMut<Editor>,
    mut browser: ResMut<Browser>,
    interface: Res<super::ui::Interface>,
    buttons: Query<&RepairButton, Added<bevy::ui::Pressed>>,
    assets: Option<Res<AssetServer>>,
    mut thumbs: Option<ResMut<super::thumbnails::Thumbnails>>,
    mut images: Option<ResMut<Assets<Image>>>,
) {
    let received: Vec<_> = monitor.rx.lock().unwrap().try_iter().collect();
    let mut rebuild = false;
    for message in received {
        match message {
            ResultEvent::Indexed(changes) => {
                if changes.refresh {
                    let old_scene = editor
                        .has_scene()
                        .then(|| editor.definition().asset.clone());
                    let moves: std::collections::HashMap<&Path, &Path> = changes
                        .moves
                        .iter()
                        .map(|(a, b)| (Path::new(a), Path::new(b)))
                        .collect();
                    if !moves.is_empty() {
                        file_moves::remap_open(&mut editor, |p| {
                            moves.get(p).map(|x| x.to_path_buf())
                        });
                    }
                    for (from, to) in &changes.moves {
                        if let Some(assets) = &assets
                            && assets.get_handle_untyped(from.clone()).is_some()
                        {
                            assets.reload(from.clone());
                        }
                        if let Some(thumbs) = thumbs.as_mut() {
                            thumbs.invalidate_asset(from);
                            thumbs.invalidate_asset(to);
                        }
                    }
                    if old_scene.is_some_and(|old| old != editor.definition().asset) {
                        editor.reload = true;
                    }
                    monitor.refresh_needed = true;
                }
                if let Ok(index) = asset_registry::index(&editor.root) {
                    for model in &mut editor.project.catalog {
                        index.bind(&mut model.asset, &mut model.uid);
                    }
                    for scene in &mut editor.project.scenes {
                        if !scene.asset.is_empty() {
                            index.bind(&mut scene.asset, &mut scene.uid);
                        }
                    }
                    let next: Vec<_> = index
                        .missing()
                        .map(|(u, p)| (u.to_owned(), p.to_owned()))
                        .collect();
                    if next != monitor.missing {
                        monitor.missing = next;
                        monitor.current = 0;
                        rebuild = true;
                    }
                }
            }
            ResultEvent::Error(e) => editor.status = format!("Реестр ассетов · {e}"),
            ResultEvent::PickerDone => monitor.picking = false,
        }
    }
    use bevy::tasks::{IoTaskPool, block_on, poll_once};
    if let Some(snapshot) = monitor
        .build
        .as_mut()
        .and_then(|task| block_on(poll_once(task)))
    {
        monitor.build = None;
        if !monitor.refresh_needed
            && browser.scan_revision == snapshot.revision
            && editor.project.catalog.len() == snapshot.catalog_len
            && editor.project.scenes.len() == snapshot.scene_len
        {
            editor.project.catalog.extend(
                snapshot
                    .project
                    .catalog
                    .into_iter()
                    .skip(snapshot.catalog_len),
            );
            editor
                .project
                .scenes
                .extend(snapshot.project.scenes.into_iter().skip(snapshot.scene_len));
            browser.adopt(snapshot.browser);
            if let (Some(thumbs), Some(images)) = (thumbs.as_mut(), images.as_mut()) {
                thumbs.extend(images, editor.project.catalog.len());
            }
        } else {
            monitor.refresh_needed = true;
        }
    }
    if monitor.refresh_needed && monitor.build.is_none() {
        monitor.refresh_needed = false;
        let mut project = editor.project.clone();
        let catalog_len = project.catalog.len();
        let scene_len = project.scenes.len();
        let revision = browser.scan_revision;
        monitor.build = Some(IoTaskPool::get().spawn(async move {
            let browser = Browser::new(&mut project);
            BrowserSnapshot {
                browser,
                project,
                catalog_len,
                scene_len,
                revision,
            }
        }));
    }
    for action in &buttons {
        if monitor.missing.is_empty() || monitor.picking {
            continue;
        }
        match action {
            RepairButton::Next => {
                monitor.current = (monitor.current + 1) % monitor.missing.len();
                rebuild = true;
            }
            RepairButton::Locate => {
                if monitor
                    .tx
                    .send(Work::Locate(monitor.missing[monitor.current].0.clone()))
                    .is_ok()
                {
                    monitor.picking = true;
                }
            }
        }
    }
    if rebuild {
        if let Some(panel) = monitor.panel.take() {
            commands.entity(panel).try_despawn();
        }
        if let Some((_, path)) = monitor.missing.get(monitor.current) {
            let mut panel = commands.spawn((
                Node {
                    position_type: PositionType::Absolute,
                    right: px(12),
                    bottom: px(40),
                    padding: UiRect::all(px(8)),
                    column_gap: px(8),
                    align_items: AlignItems::Center,
                    ..default()
                },
                BackgroundColor(Color::srgb(0.13, 0.14, 0.17)),
                GlobalZIndex(90),
            ));
            panel.with_children(|p| {
                p.spawn((
                    Text::new(format!("Файл не найден · {path}")),
                    interface.text_font(14.),
                ));
                for (label, action) in [
                    ("Указать файл", RepairButton::Locate),
                    ("Следующий", RepairButton::Next),
                ] {
                    if matches!(action, RepairButton::Next) && monitor.missing.len() < 2 {
                        continue;
                    }
                    p.spawn((
                        bevy::ui_widgets::Button,
                        action,
                        Node {
                            padding: UiRect::axes(px(8), px(4)),
                            ..default()
                        },
                        BackgroundColor(Color::srgb(0.22, 0.27, 0.34)),
                    ))
                    .with_child((Text::new(label), interface.text_font(14.)));
                }
            });
            monitor.panel = Some(panel.id());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::{
        selection_outline::render_tests::frame,
        tests::lighting_ui_render::fixture_project_with_pipeline_compilation,
    };
    #[test]
    #[ignore = "renders a real viewport while indexing 10,000 new textures"]
    fn viewport_frame_times_during_asset_registry_scan() {
        let root = std::env::temp_dir().join(format!(
            "bevy-registry-frames-{}",
            bevy::asset::uuid::Uuid::new_v4()
        ));
        let project = crate::project_manager::create_test_project(&root).unwrap();
        let assets = project.asset_root.clone();
        let staging = root.join("staged");
        fs::create_dir(&staging).unwrap();
        for i in 0u32..10000 {
            fs::write(staging.join(format!("{i}.png")), i.to_le_bytes()).unwrap();
        }
        let (mut app, _, _) = fixture_project_with_pipeline_compilation(project, false);
        let mut baseline = vec![];
        for _ in 0..40 {
            let before = Instant::now();
            frame(&mut app);
            baseline.push(before.elapsed());
        }
        let start = Instant::now();
        fs::rename(staging, assets.join("textures")).unwrap();
        let mut frames = vec![];
        while !app
            .world()
            .resource::<Browser>()
            .files
            .iter()
            .any(|f| f.path == Path::new("textures/9999.png"))
        {
            let before = Instant::now();
            frame(&mut app);
            frames.push(before.elapsed());
            assert!(
                start.elapsed() < Duration::from_secs(90),
                "{}",
                app.world().resource::<Editor>().status
            );
        }
        baseline.sort();
        frames.sort();
        eprintln!(
            "Viewport: baseline P95 {:?}, indexing P50 {:?}, P95 {:?}, maximum {:?}, {} frames over {:?}",
            baseline[38],
            frames[frames.len() / 2],
            frames[frames.len() * 95 / 100],
            frames.last().unwrap(),
            frames.len(),
            start.elapsed()
        );
        assert!(frames.len() > 1);
        drop(app);
        fs::remove_dir_all(root).unwrap();
    }
}
