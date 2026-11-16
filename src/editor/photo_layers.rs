//! The upstream layer engine shares the editor's document and undo timeline.
use super::*;
use std::sync::Arc;

pub(super) struct Parent {
    snapshot: Snapshot,
    saved: u64,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
}
impl Parent {
    pub fn dirty(&self) -> bool {
        self.snapshot.revision != self.saved
    }
}

impl PhotoDocument {
    pub fn ensure_filter_mask(&mut self) -> Result<(), String> {
        let id = self.active_id().ok_or("Выберите смарт-объект")?;
        let Some(Layer {
            content: LayerContent::Smart(smart),
            ..
        }) = self.active_layer()
        else {
            return Err("Выберите смарт-объект".into());
        };
        if smart.smart_filters.is_empty() {
            return Err("Сначала добавьте смарт-фильтр".into());
        }
        if smart.filter_mask.is_some() {
            return Ok(());
        }
        if self.document.effective_locks(id).all {
            return Err("Слой заблокирован".into());
        }
        self.checkpoint();
        if let Some(Layer {
            content: LayerContent::Smart(smart),
            ..
        }) = self.active_layer_mut()
        {
            smart.filter_mask = Some(photocraft_doc::LayerMask::reveal_all());
        }
        self.changed();
        Ok(())
    }
    fn edit_filter_mask(
        &mut self,
        params: &serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let id = params
            .get("layer")
            .and_then(serde_json::Value::as_u64)
            .map(LayerId)
            .or_else(|| self.active_id())
            .ok_or("Выберите смарт-объект")?;
        if self.document.effective_locks(id).all {
            return Err("Слой заблокирован".into());
        }
        let mut mask = self
            .document
            .layer(id)
            .and_then(|layer| photocraft_engine::commands::mask_view_cmds::viewed_mask(layer, true))
            .cloned()
            .ok_or("У слоя нет маски смарт-фильтров")?;
        let original = mask.clone();
        for (key, target) in [("enabled", &mut mask.enabled), ("linked", &mut mask.linked)] {
            if let Some(value) = params.get(key) {
                *target = value
                    .as_bool()
                    .ok_or_else(|| format!("Неверное значение {key}"))?;
            }
        }
        for (key, target, max, scale) in [
            ("density", &mut mask.density, 100.0, 0.01),
            ("feather", &mut mask.feather, 1000.0, 1.0),
        ] {
            if let Some(value) = params.get(key) {
                let value = value
                    .as_f64()
                    .filter(|v| v.is_finite() && (0.0..=max).contains(v))
                    .ok_or_else(|| format!("{key}: допустимо от 0 до {max}"))?;
                *target = (value * scale) as f32;
            }
        }
        if mask != original {
            let before = self.snapshot();
            if let Some(Layer {
                content: LayerContent::Smart(smart),
                ..
            }) = self.document.layer_mut(id)
            {
                smart.filter_mask = Some(mask);
            }
            if let Err(error) =
                photocraft_engine::commands::smart_cmds::refresh_after_filter_mask_edit(
                    &mut self.document,
                    id,
                )
            {
                self.document = before.document;
                return Err(error.to_string());
            }
            self.checkpoint_from(before);
            self.changed();
        }
        Ok(serde_json::json!({"layer":id.0}))
    }
    pub fn layer(&self, index: usize) -> Option<&Layer> {
        self.document
            .walk()
            .into_iter()
            .nth(index)
            .map(|(_, _, layer)| layer)
    }
    pub fn active_layer(&self) -> Option<&Layer> {
        self.layer(self.active)
    }
    pub fn active_id(&self) -> Option<LayerId> {
        self.active_layer().map(|l| l.id)
    }
    pub fn active_layer_mut(&mut self) -> Option<&mut Layer> {
        let id = self.active_id()?;
        self.document.layer_mut(id)
    }
    pub fn paint_surface(&self) -> Option<&Surface> {
        let layer = self.active_layer()?;
        if self.mask_target {
            photocraft_engine::commands::mask_view_cmds::viewed_mask(layer, self.filter_target)
                .map(|m| &m.surface)
        } else {
            layer.surface()
        }
    }
    pub fn paint_surface_mut(&mut self) -> Option<&mut Surface> {
        let mask = self.mask_target;
        let filter = self.filter_target;
        let layer = self.active_layer_mut()?;
        if mask {
            if filter && let LayerContent::Smart(s) = &mut layer.content {
                return s.filter_mask.as_mut().map(|m| &mut m.surface);
            }
            layer.mask.as_mut().map(|m| &mut m.surface)
        } else {
            layer.surface_mut()
        }
    }
    pub fn select(&mut self, index: usize, toggle: bool, range: bool) {
        let Some(id) = self.layer(index).map(|l| l.id) else {
            return;
        };
        if range {
            let lo = index.min(self.active);
            let hi = index.max(self.active);
            self.selected = self
                .document
                .walk()
                .into_iter()
                .skip(lo)
                .take(hi - lo + 1)
                .map(|(_, _, l)| l.id)
                .collect();
        } else if toggle {
            if self.selected.contains(&id) {
                self.selected.retain(|&l| l != id);
            } else {
                self.selected.push(id);
            }
        } else {
            self.selected = vec![id];
        }
        self.active = index;
        if toggle && !self.selected.contains(&id) {
            self.active = self
                .selected
                .last()
                .and_then(|last| {
                    self.document
                        .walk()
                        .iter()
                        .position(|(_, _, l)| l.id == *last)
                })
                .unwrap_or(usize::MAX);
        }
        self.mask_target = false;
        self.filter_target = false;
        if self.mask_view.take().is_some() {
            self.view_revision += 1;
            self.damage = None;
        }
    }
    /// Commands retain their native validation, masks, styles and multi-layer semantics.
    /// The temporary engine history is discarded: painting and layer commands use one timeline.
    pub fn layer_command(
        &mut self,
        command: &str,
        mut params: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        if command == "editor.filterMaskOptions" {
            return self.edit_filter_mask(&params);
        }
        if command == "layer.smartObjects.saveContents" && !self.parents.is_empty() {
            self.finish_smart()?;
            return Ok(serde_json::json!({"updated":true}));
        }
        let active = self.active_id();
        let auto_copy_name = command == "layer.duplicate" && params.get("name").is_none();
        if params.get("name").is_none()
            && matches!(
                command,
                "layer.new.layer"
                    | "layer.new.group"
                    | "layer.groupLayers"
                    | "layer.new.groupFromLayers"
            )
        {
            let prefix = if command == "layer.new.layer" {
                "Слой"
            } else {
                "Группа"
            };
            let names = self
                .document
                .walk()
                .into_iter()
                .map(|(_, _, l)| l.name.as_str())
                .collect::<std::collections::HashSet<_>>();
            if let Some(name) = (1..=names.len() + 2)
                .map(|n| format!("{prefix} {n}"))
                .find(|n| !names.contains(n.as_str()))
            {
                params["name"] = serde_json::json!(name);
            }
        }
        let before = self.snapshot();
        let engine_container = self
            .engine
            .take()
            .unwrap_or_else(|| std::sync::Mutex::new(Box::new(photocraft_engine::Session::new())));
        let mut engine = engine_container.lock().map_err(|e| e.to_string())?;
        engine.tools.foreground = self.tool_colors.0;
        engine.tools.background = self.tool_colors.1;
        let state = photocraft_engine::DocState::new(self.document.clone(), None);
        if let Some(current) = engine.active_mut() {
            current.doc = state.doc;
            current.history = state.history;
            current.coalesce = None;
            current.revision += 1;
        } else {
            engine.add_document(self.document.clone(), None);
        }
        if let Some(current) = engine.active_mut() {
            current.channel_view.layer_mask = self.mask_view;
        }
        engine
            .restore_layer_selection(active, &self.selected, active)
            .map_err(|e| e.to_string())?;
        let original = engine.active().unwrap().doc.clone();
        let smart_filter =
            command.starts_with("filter.") && command != "filter.convertForSmartFilters";
        let conversion = if smart_filter
            && engine
                .active()
                .and_then(|st| st.active_layer.and_then(|id| st.doc.layer(id)))
                .is_some_and(|l| !matches!(l.content, LayerContent::Smart(_)))
        {
            engine.execute(
                "layer.smartObjects.convertToSmartObject",
                serde_json::json!({}),
            )
        } else {
            Ok(serde_json::Value::Null)
        };
        if conversion.is_ok()
            && smart_filter
            && let Some(id) = engine.active().and_then(|s| s.active_layer)
        {
            if params.get("layer").and_then(serde_json::Value::as_u64) == active.map(|id| id.0) {
                params["layer"] = serde_json::json!(id.0);
            }
        }
        let global_angle = if matches!(
            command,
            "layer.layerStyle.dropShadow"
                | "layer.layerStyle.innerShadow"
                | "layer.layerStyle.bevelEmboss"
        ) && params
            .get("useGlobalLight")
            .and_then(serde_json::Value::as_bool)
            != Some(false)
        {
            params.get("angle").and_then(serde_json::Value::as_f64)
        } else if command == "layer.layerStyle.replace" {
            params
                .get("globalLight")
                .and_then(serde_json::Value::as_f64)
                .or_else(|| {
                    params["effects"].as_array().and_then(|entries| {
                        entries.iter().rev().find_map(|entry| {
                            let angle = entry["params"]["angle"].as_f64()?;
                            let effect: photocraft_doc::Effect =
                                serde_json::from_value(entry["fx"].clone()).ok()?;
                            let uses_global = match effect {
                                photocraft_doc::Effect::DropShadow(s)
                                | photocraft_doc::Effect::InnerShadow(s) => s.use_global_light,
                                photocraft_doc::Effect::BevelEmboss(b) => b.use_global_light,
                                _ => false,
                            };
                            entry["params"]
                                .get("useGlobalLight")
                                .and_then(serde_json::Value::as_bool)
                                .unwrap_or(uses_global)
                                .then_some(angle)
                        })
                    })
                })
        } else {
            None
        };
        let result = conversion
            .and_then(|_| {
                global_angle.map_or(Ok(serde_json::Value::Null), |angle| {
                    engine.execute(
                        "layer.layerStyle.globalLight",
                        serde_json::json!({"angle":angle}),
                    )
                })
            })
            .and_then(|_| engine.execute(command, params))
            .map_err(|e| e.to_string());
        let result = result.and_then(|value| {
            if let Some(state) = engine.active() {
                validate(&state.doc)?;
            }
            Ok(value)
        });
        if result.is_err() && engine.active().is_some_and(|s| s.doc.id != original.id) {
            if let Some(child) = engine.active_index() {
                if let Some(state) = engine.active_mut() {
                    state.saved_revision = state.revision;
                }
                engine.close(child);
                if let Some(parent) = engine
                    .documents()
                    .iter()
                    .position(|s| s.doc.id == original.id)
                {
                    engine.set_active(parent);
                }
            }
        }
        if result.is_ok()
            && let Some(state) = engine.active()
        {
            if original.id != state.doc.id {
                self.parents.push(Parent {
                    snapshot: before,
                    saved: self.saved_revision,
                    undo: std::mem::take(&mut self.undo),
                    redo: std::mem::take(&mut self.redo),
                });
                self.document = (*state.doc).clone();
                self.changed();
                self.saved_revision = self.revision;
            } else if !Arc::ptr_eq(&original, &state.doc)
                && photocraft_engine::commands::find(command).is_some_and(|s| !s.journal)
            {
                self.document = (*state.doc).clone();
                self.view_revision += 1;
                self.damage = None;
            } else if !Arc::ptr_eq(&original, &state.doc) {
                self.checkpoint_from(before);
                self.document = (*state.doc).clone();
                self.changed();
            }
            self.selected = state.selected_layers.clone();
            if auto_copy_name {
                let originals = original
                    .walk()
                    .into_iter()
                    .map(|(_, _, l)| l.name.as_str())
                    .collect::<std::collections::HashSet<_>>();
                let copies = self
                    .document
                    .walk()
                    .into_iter()
                    .filter(|(_, _, l)| {
                        original.layer(l.id).is_none() && !originals.contains(l.name.as_str())
                    })
                    .map(|(_, _, l)| (l.id, l.name.clone()))
                    .collect::<Vec<_>>();
                for (id, name) in copies {
                    if let Some((base, suffix)) = name.rsplit_once(" copy")
                        && (suffix.is_empty() || suffix.trim().bytes().all(|c| c.is_ascii_digit()))
                    {
                        let name = self.copy_name(base);
                        if let Some(layer) = self.document.layer_mut(id) {
                            layer.name = name;
                        }
                    }
                }
            }
            self.active = state
                .active_layer
                .and_then(|id| {
                    self.document
                        .walk()
                        .into_iter()
                        .position(|(_, _, l)| l.id == id)
                })
                .unwrap_or(0);
            self.mask_target &= self.active_layer().is_some_and(|layer| {
                photocraft_engine::commands::mask_view_cmds::viewed_mask(layer, self.filter_target)
                    .is_some()
            });
            let view = photocraft_engine::commands::mask_view_cmds::current(state);
            if self.mask_view != view {
                self.mask_view = view;
                self.view_revision += 1;
                self.damage = None;
            }
            if let Some(view) = self.mask_view {
                self.mask_target = true;
                self.filter_target = view.filter;
            }
        }
        drop(engine);
        self.engine = Some(engine_container);
        result
    }
    pub fn set_tool_colors(&mut self, foreground: [f32; 4], background: [f32; 4]) {
        self.tool_colors = (foreground, background);
    }
    pub fn copied_style(&mut self) -> Option<photocraft_engine::CopiedStyle> {
        self.engine
            .as_mut()?
            .get_mut()
            .ok()?
            .style_clipboard
            .clone()
    }
    pub fn set_copied_style(&mut self, style: Option<photocraft_engine::CopiedStyle>) {
        let engine = self.engine.get_or_insert_with(|| {
            std::sync::Mutex::new(Box::new(photocraft_engine::Session::new()))
        });
        if let Ok(engine) = engine.get_mut() {
            engine.style_clipboard = style;
        }
    }
    pub fn copy_layers(&self) -> Vec<Layer> {
        self.document
            .walk()
            .into_iter()
            .filter(|(path, _, layer)| {
                self.selected.contains(&layer.id)
                    && !(1..path.len()).any(|n| {
                        self.document
                            .layer_at(&path[..n])
                            .is_some_and(|l| self.selected.contains(&l.id))
                    })
            })
            .map(|(_, _, layer)| layer.clone())
            .collect()
    }
    pub fn paste_layers(&mut self, layers: &[Layer]) -> Result<(), String> {
        fn count(layer: &Layer) -> usize {
            1 + layer.children().map_or(0, |c| c.iter().map(count).sum())
        }
        if layers.is_empty() {
            return Err("Сначала скопируйте слои".into());
        }
        if self.document.layer_count() + layers.iter().map(count).sum::<usize>() > MAX_LAYERS {
            return Err("Слишком много слоёв".into());
        }
        let mut next_link = self
            .document
            .walk()
            .iter()
            .filter_map(|(_, _, l)| l.link_group)
            .max()
            .unwrap_or(0)
            + 1;
        let mut links = std::collections::HashMap::new();
        fn remap(
            layer: &mut Layer,
            links: &mut std::collections::HashMap<u64, u64>,
            next: &mut u64,
        ) {
            if let Some(old) = layer.link_group {
                layer.link_group = Some(*links.entry(old).or_insert_with(|| {
                    let id = *next;
                    *next += 1;
                    id
                }));
            }
            if let Some(children) = layer.children_mut() {
                for child in children {
                    remap(child, links, next);
                }
            }
        }
        self.checkpoint();
        let mut above = self.active_id();
        let mut selected = Vec::new();
        for layer in layers {
            let mut copy = layer.duplicate();
            copy.name = self.copy_name(&copy.name);
            remap(&mut copy, &mut links, &mut next_link);
            let id = self.document.insert_above(above, copy);
            above = Some(id);
            selected.push(id);
        }
        self.selected = selected;
        self.active = above
            .and_then(|id| self.document.walk().iter().position(|(_, _, l)| l.id == id))
            .unwrap_or(0);
        self.mask_target = false;
        self.filter_target = false;
        self.changed();
        Ok(())
    }
    pub fn movable(&self) -> bool {
        self.active_id().is_some_and(|id| {
            let locks = self.document.effective_locks(id);
            !locks.all && !locks.position
        })
    }
    fn copy_name(&self, name: &str) -> String {
        let base = name
            .rsplit_once(" — копия")
            .filter(|(_, suffix)| {
                suffix.is_empty() || suffix.trim().bytes().all(|c| c.is_ascii_digit())
            })
            .map_or(name, |(base, _)| base);
        let names = self
            .document
            .walk()
            .into_iter()
            .map(|(_, _, l)| l.name.as_str())
            .collect::<std::collections::HashSet<_>>();
        (1..=names.len() + 2)
            .map(|n| {
                if n == 1 {
                    format!("{base} — копия")
                } else {
                    format!("{base} — копия {n}")
                }
            })
            .find(|n| !names.contains(n.as_str()))
            .unwrap_or_else(|| format!("{base} — копия"))
    }
    pub fn preview(&self) -> Rgba8Image {
        self.preview_region(Rect::from_size(self.size()))
    }
    pub fn preview_region(&self, rect: Rect) -> Rgba8Image {
        use photocraft_engine::commands::mask_view_cmds::{MaskViewMode, viewed_mask};
        let rect = rect.intersect(&Rect::from_size(self.size()));
        let Some(view) = self.mask_view else {
            return self.composite_region(rect);
        };
        let Some(mask) = self
            .document
            .layer(view.layer)
            .and_then(|l| viewed_mask(l, view.filter))
        else {
            return self.composite_region(rect);
        };
        let mut image = if view.mode == MaskViewMode::Gray {
            Rgba8Image::new(rect.width(), rect.height())
        } else {
            self.composite_region(rect)
        };
        for (index, pixel) in image.pixels.chunks_exact_mut(4).enumerate() {
            let x = rect.x0 + (index % rect.width() as usize) as i32;
            let y = rect.y0 + (index / rect.width() as usize) as i32;
            let value = mask.surface.sample_channel(x, y, 0).clamp(0.0, 1.0);
            if view.mode == MaskViewMode::Gray {
                let gray = (value * 255.0).round() as u8;
                pixel.copy_from_slice(&[gray, gray, gray, 255]);
            } else {
                let alpha = (1.0 - value) * 0.5;
                pixel[0] = (pixel[0] as f32 * (1.0 - alpha) + 255.0 * alpha).round() as u8;
                pixel[1] = (pixel[1] as f32 * (1.0 - alpha)).round() as u8;
                pixel[2] = (pixel[2] as f32 * (1.0 - alpha)).round() as u8;
            }
        }
        image
    }
    pub fn editing_smart(&self) -> bool {
        !self.parents.is_empty()
    }
    pub fn finish_smart(&mut self) -> Result<(), String> {
        let Some(parent) = self.parents.last() else {
            return Ok(());
        };
        let parent_id = parent.snapshot.document.id;
        let modified = self.revision != self.saved_revision;
        let engine = self
            .engine
            .as_mut()
            .ok_or("Редактор содержимого недоступен")?
            .get_mut()
            .map_err(|e| e.to_string())?;
        let child = engine.active_index().ok_or("Нет открытого содержимого")?;
        if let Some(current) = engine.active_mut() {
            current.doc = Arc::new(self.document.clone());
            if !modified {
                current.saved_revision = current.revision;
            }
        }
        if modified {
            engine
                .execute("layer.smartObjects.saveContents", serde_json::json!({}))
                .map_err(|e| e.to_string())?;
        }
        engine.close(child);
        let index = engine
            .documents()
            .iter()
            .position(|d| d.doc.id == parent_id)
            .ok_or("Исходное изображение закрыто")?;
        engine.set_active(index);
        let document = (*engine.active().unwrap().doc).clone();
        let parent = self.parents.pop().unwrap();
        self.undo = parent.undo;
        self.redo = parent.redo;
        self.saved_revision = parent.saved;
        self.restore(parent.snapshot.clone());
        if modified {
            self.checkpoint_from(parent.snapshot);
            self.document = document;
            self.changed();
        }
        Ok(())
    }
    pub fn begin_move(&self) -> LayerMove {
        LayerMove {
            before: self.snapshot(),
            evicted: if self.undo.len() == HISTORY {
                self.undo.first().cloned()
            } else {
                None
            },
            redo: self.redo.clone(),
            applied: false,
        }
    }
    pub fn cancel_move(&mut self, movement: &mut LayerMove) {
        if movement.applied
            && self
                .undo
                .last()
                .is_some_and(|s| s.revision == movement.before.revision)
        {
            self.undo.pop();
            if let Some(evicted) = &movement.evicted {
                self.undo.insert(0, evicted.clone());
            }
        }
        self.restore(movement.before.clone());
        self.redo = movement.redo.clone();
        movement.applied = false;
    }
    pub fn translate_live(
        &mut self,
        movement: &mut LayerMove,
        dx: i32,
        dy: i32,
    ) -> Result<(), String> {
        self.cancel_move(movement);
        if dx == 0 && dy == 0 {
            return Ok(());
        }
        self.layer_command(
            if self.document.selection.is_some() { "edit.transform" } else { "layer.translate" },
            serde_json::json!({"dx":dx,"dy":dy,"target":if self.mask_target && !self.filter_target{"mask"}else{"pixels"}}),
        )?;
        movement.applied = self.revision != movement.before.revision;
        Ok(())
    }
}
