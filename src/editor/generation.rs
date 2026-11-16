//! Image-to-3D session draft; the native service owns the resident model.
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Quality {
    Draft,
    Standard,
    Detailed,
}

#[derive(Clone, Copy)]
enum ReturnTab {
    Scene(Room),
    Model(usize),
    Image(usize),
    Narrative(usize),
    Empty,
}

pub(super) struct Workspace {
    pub opened: bool,
    pub active: bool,
    previous: ReturnTab,
    pub source: Option<PathBuf>,
    pub quality: Quality,
    pub steps: u32,
    pub guidance: f32,
    pub resolution: u32,
    pub chunks: u32,
    pub seed: u64,
    pub random_seed: bool,
    pub simplify: bool,
    pub target_faces: u32,
    pub obj: bool,
    pub name: String,
    pub backend_ready: bool,
    pub engine_active: bool,
    pub engine_loading: bool,
    pub source_ready: bool,
    pub config_valid: bool,
    pub running: bool,
    pub has_result: bool,
    pub result_imported: bool,
    pub wireframe: bool,
}

impl Default for Workspace {
    fn default() -> Self {
        Self {
            opened: false,
            active: false,
            previous: ReturnTab::Empty,
            source: None,
            quality: Quality::Standard,
            steps: 30,
            guidance: 5.0,
            resolution: 380,
            chunks: 20_000,
            seed: 12345,
            random_seed: false,
            simplify: true,
            target_faces: 40_000,
            obj: false,
            name: "Новая модель".into(),
            backend_ready: false,
            engine_active: false,
            engine_loading: false,
            source_ready: false,
            config_valid: true,
            running: false,
            has_result: false,
            result_imported: false,
            wireframe: false,
        }
    }
}

impl Workspace {
    pub fn set_quality(&mut self, quality: Quality) {
        self.quality = quality;
        (self.steps, self.resolution) = match quality {
            Quality::Draft => (20, 256),
            Quality::Standard => (30, 380),
            Quality::Detailed => (50, 512),
        };
    }

    pub fn accepts(path: &std::path::Path) -> bool {
        path.extension().and_then(|v| v.to_str()).is_some_and(|v| {
            ["png", "jpg", "jpeg", "webp"]
                .iter()
                .any(|ext| v.eq_ignore_ascii_case(ext))
        })
    }
}

impl Editor {
    pub(super) fn open_generation(&mut self) {
        if !self.generation.active {
            self.generation.previous = if let Some(id) = self.viewing_image {
                ReturnTab::Image(id)
            } else if let Some(index) = self.viewing_model {
                ReturnTab::Model(index)
            } else if let Some(id) = self.narrative.active {
                ReturnTab::Narrative(id)
            } else if self.has_scene() {
                ReturnTab::Scene(self.room)
            } else {
                ReturnTab::Empty
            };
        }
        self.finish_camera_navigation();
        self.generation.opened = true;
        self.generation.active = true;
        self.drawer = false;
        self.viewing_image = None;
        self.viewing_model = None;
        self.narrative.active = None;
        self.pilot = false;
    }

    pub(super) fn close_generation(&mut self) {
        self.generation.opened = false;
        if !self.generation.active {
            return;
        }
        self.generation.active = false;
        match self.generation.previous {
            ReturnTab::Scene(room) if self.open_tabs.contains(&room) => {
                self.open_document(room);
            }
            ReturnTab::Model(index) if self.model_tabs.contains(&index) => {
                self.open_model_tab(index)
            }
            ReturnTab::Image(id) if self.image_tab(id).is_some() => self.activate_image_tab(id),
            ReturnTab::Narrative(id) if self.narrative.get(id).is_some() => {
                self.narrative.active = Some(id)
            }
            _ if self.has_scene() => {}
            _ => {
                self.viewing_model = self.model_tabs.last().copied();
                if self.viewing_model.is_none() {
                    self.viewing_image = self.image_tabs.last().map(|tab| tab.id);
                    if self.viewing_image.is_none() {
                        self.narrative.active = self.narrative.tabs.last().map(|d| d.id);
                    }
                }
            }
        }
    }
}
