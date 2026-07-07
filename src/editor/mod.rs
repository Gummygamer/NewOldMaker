//! Map editor: HD-2D viewport with direct painting, tool palettes, event
//! placement, map settings.

pub mod database;

use std::sync::Arc;

use eframe::egui::{self, Color32, RichText};
use glam::Vec3;

use crate::core::data::*;
use crate::gfx::camera::OrbitCamera;
use crate::gfx::mesh::{pick_tile, tile_top_y, SPRITE_HORIZONTAL, SPRITE_UNLIT};
use crate::gfx::renderer::{Hd2dCallback, PostSettings};
use crate::gfx::scene;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Terrain,
    Height,
    Prop,
    Event,
}

pub struct UndoEntry {
    map_id: u32,
    tiles: Vec<Tile>,
    events: Vec<EventData>,
}

pub struct EditorState {
    pub current_map: u32,
    pub camera: OrbitCamera,
    pub tool: Tool,
    pub sel_terrain: Terrain,
    pub sel_prop: Prop,
    pub hovered: Option<(i32, i32)>,
    pub selected_event: Option<u32>,
    pub map_arc: Arc<MapData>,
    pub map_revision: u64,
    pub post: PostSettings,
    pub preview_fx: bool,
    pub show_database: bool,
    pub db_tab: database::DbTab,
    pub undo: Vec<UndoEntry>,
    stroke_active: bool,
    last_tile: Option<(i32, i32)>,
    /// Pending "new event" popup at tile.
    new_event_at: Option<(i32, i32)>,
}

impl EditorState {
    pub fn new(project: &ProjectData) -> Self {
        let map_id = project.system.start_map;
        let map = project.map(map_id).or_else(|| project.maps.first()).expect("project has maps");
        let mut camera = OrbitCamera::default();
        camera.target = Vec3::new(map.width as f32 / 2.0, 0.5, map.height as f32 / 2.0);
        camera.dist = (map.width.max(map.height) as f32) * 0.9;
        EditorState {
            current_map: map.id,
            camera,
            tool: Tool::Terrain,
            sel_terrain: Terrain::Grass,
            sel_prop: Prop::Tree,
            hovered: None,
            selected_event: None,
            map_arc: Arc::new(map.clone()),
            map_revision: crate::app::next_revision(),
            post: PostSettings::default(),
            preview_fx: true,
            show_database: false,
            db_tab: database::DbTab::Actors,
            undo: Vec::new(),
            stroke_active: false,
            last_tile: None,
            new_event_at: None,
        }
    }

    pub fn sync_map(&mut self, project: &ProjectData) {
        if let Some(map) = project.map(self.current_map) {
            self.map_arc = Arc::new(map.clone());
        } else if let Some(map) = project.maps.first() {
            self.current_map = map.id;
            self.map_arc = Arc::new(map.clone());
        }
        self.map_revision = crate::app::next_revision();
    }

    pub fn switch_map(&mut self, project: &ProjectData, id: u32) {
        if project.map(id).is_some() {
            self.current_map = id;
            self.selected_event = None;
            self.sync_map(project);
            let map = self.map_arc.clone();
            self.camera.target = Vec3::new(map.width as f32 / 2.0, 0.5, map.height as f32 / 2.0);
        }
    }

    fn push_undo(&mut self, project: &ProjectData) {
        if let Some(map) = project.map(self.current_map) {
            self.undo.push(UndoEntry {
                map_id: map.id,
                tiles: map.tiles.clone(),
                events: map.events.clone(),
            });
            if self.undo.len() > 48 {
                self.undo.remove(0);
            }
        }
    }

    pub fn apply_undo(&mut self, project: &mut ProjectData) {
        if let Some(entry) = self.undo.pop() {
            if let Some(map) = project.map_mut(entry.map_id) {
                map.tiles = entry.tiles;
                map.events = entry.events;
            }
            if entry.map_id != self.current_map {
                self.current_map = entry.map_id;
            }
            self.sync_map(project);
        }
    }
}

// ---------------------------------------------------------------------------
// Viewport
// ---------------------------------------------------------------------------

pub fn viewport(ui: &mut egui::Ui, project: &mut ProjectData, ed: &mut EditorState, time: f32) {
    let avail = ui.available_size();
    let (rect, response) = ui.allocate_exact_size(avail, egui::Sense::click_and_drag());
    let ppp = ui.ctx().pixels_per_point();
    let viewport_px = [
        (rect.width() * ppp).round().max(8.0) as u32,
        (rect.height() * ppp).round().max(8.0) as u32,
    ];
    let aspect = viewport_px[0] as f32 / viewport_px[1].max(1) as f32;

    // ---- Camera controls ----
    if response.dragged_by(egui::PointerButton::Secondary) {
        let d = response.drag_delta();
        ed.camera.yaw -= d.x * 0.008;
        ed.camera.pitch = (ed.camera.pitch + d.y * 0.006).clamp(0.30, 1.45);
    }
    if response.dragged_by(egui::PointerButton::Middle) {
        let d = response.drag_delta();
        let right = ed.camera.billboard_right();
        let fwd = Vec3::new(-ed.camera.yaw.sin(), 0.0, -ed.camera.yaw.cos());
        let scale = ed.camera.dist * 0.0016;
        ed.camera.target -= right * d.x * scale;
        ed.camera.target -= fwd * -d.y * scale;
    }
    if response.hovered() {
        let scroll = ui.input(|i| i.smooth_scroll_delta.y);
        if scroll.abs() > 0.0 {
            ed.camera.dist = (ed.camera.dist * (1.0 - scroll * 0.0015)).clamp(4.0, 80.0);
        }
    }

    // ---- Picking ----
    ed.hovered = None;
    if let Some(pointer) = response.hover_pos() {
        let ndc_x = ((pointer.x - rect.left()) / rect.width()) * 2.0 - 1.0;
        let ndc_y = 1.0 - ((pointer.y - rect.top()) / rect.height()) * 2.0;
        let (origin, dir) = ed.camera.screen_ray(ndc_x, ndc_y, aspect);
        ed.hovered = pick_tile(&ed.map_arc, origin, dir);
    }

    // ---- Tool application ----
    let ctrl = ui.input(|i| i.modifiers.ctrl);
    let primary_down = response.dragged_by(egui::PointerButton::Primary)
        || response.clicked_by(egui::PointerButton::Primary);
    if primary_down {
        if !ed.stroke_active {
            ed.stroke_active = true;
            ed.last_tile = None;
            ed.push_undo(project);
        }
        if let Some((tx, ty)) = ed.hovered {
            apply_tool(project, ed, tx, ty, ctrl);
        }
    } else {
        ed.stroke_active = false;
        ed.last_tile = None;
    }

    // ---- Build the frame ----
    let map = ed.map_arc.clone();
    let atlas = crate::app::atlas();
    let mut cutout = Vec::with_capacity(256);
    let mut blend = Vec::with_capacity(32);
    let mut lights = Vec::new();
    scene::map_prop_sprites(&map, atlas, &mut cutout, &mut lights);

    // Event markers.
    for ev in &map.events {
        let top = tile_top_y(&map, ev.x, ev.y);
        let pos = Vec3::new(ev.x as f32 + 0.5, top, ev.y as f32 + 0.5);
        let color = match &ev.kind {
            EventKind::Npc { sprite, .. } => {
                scene::char_sprites(atlas, *sprite as usize, 0, 1, pos, [1.0; 4], &mut cutout, &mut blend);
                [0.3, 0.9, 0.4, 0.4]
            }
            EventKind::Sign { .. } => {
                cutout.push(scene::sprite(pos, scene::PROP_SIZE, atlas.props[Prop::Signpost as usize], [1.0; 4], 0));
                [0.8, 0.6, 0.3, 0.4]
            }
            EventKind::Chest { .. } => {
                cutout.push(scene::sprite(pos, scene::PROP_SIZE, atlas.props[Prop::Barrel as usize], [1.3, 1.1, 0.5, 1.0], 0));
                [1.0, 0.85, 0.2, 0.45]
            }
            EventKind::Transfer { .. } => [0.2, 0.8, 1.0, 0.5],
            EventKind::BattleTrigger { .. } => [1.0, 0.25, 0.2, 0.5],
            EventKind::HealPoint => {
                cutout.push(scene::sprite(pos, scene::PROP_SIZE, atlas.props[Prop::Crystal as usize], [0.6, 1.3, 0.7, 1.0], 0));
                [0.3, 1.0, 0.6, 0.45]
            }
        };
        let selected = ed.selected_event == Some(ev.id);
        let tint = if selected { [1.0, 1.0, 1.0, 0.75] } else { color };
        blend.push(scene::sprite(
            pos + Vec3::Y * 0.04,
            [0.95, 0.95],
            atlas.white,
            tint,
            SPRITE_HORIZONTAL | SPRITE_UNLIT,
        ));
    }

    // Hover cursor.
    if let Some((tx, ty)) = ed.hovered {
        let top = tile_top_y(&map, tx, ty);
        blend.push(scene::sprite(
            Vec3::new(tx as f32 + 0.5, top + 0.06, ty as f32 + 0.5),
            [1.0, 1.0],
            atlas.white,
            [1.0, 1.0, 0.5, 0.28],
            SPRITE_HORIZONTAL | SPRITE_UNLIT,
        ));
    }

    let mut post = ed.post;
    if !ed.preview_fx {
        post.dof_strength = 0.0;
        post.vignette = 0.0;
        post.bloom_strength = 0.0;
    }
    let input = scene::frame_input(
        map,
        ed.map_revision,
        &ed.camera,
        viewport_px,
        time,
        lights,
        cutout,
        blend,
        post,
    );
    ui.painter().add(eframe::egui_wgpu::Callback::new_paint_callback(
        rect,
        Hd2dCallback { input: Arc::new(input) },
    ));

    // Status line overlay.
    let status = match ed.hovered {
        Some((x, y)) => {
            let t = ed.map_arc.tile(x, y);
            format!(
                "({}, {})  {}  h{}  |  LMB apply · Ctrl+LMB inverse · RMB orbit · MMB pan · wheel zoom",
                x,
                y,
                Terrain::from_u8(t.terrain).name(),
                t.height
            )
        }
        None => "RMB orbit · MMB pan · wheel zoom".to_string(),
    };
    ui.painter().text(
        rect.left_bottom() + egui::vec2(8.0, -8.0),
        egui::Align2::LEFT_BOTTOM,
        status,
        egui::FontId::monospace(12.0),
        Color32::from_white_alpha(180),
    );

    // New-event popup.
    new_event_popup(ui, project, ed);
}

fn apply_tool(project: &mut ProjectData, ed: &mut EditorState, tx: i32, ty: i32, ctrl: bool) {
    let repeat_same_tile = ed.last_tile == Some((tx, ty));
    let Some(map) = project.map_mut(ed.current_map) else { return };
    let mut changed = false;
    match ed.tool {
        Tool::Terrain => {
            if let Some(t) = map.tile_mut(tx, ty) {
                let new = ed.sel_terrain as u8;
                if t.terrain != new {
                    t.terrain = new;
                    if Terrain::from_u8(new).liquid() {
                        t.prop = 0;
                    }
                    changed = true;
                }
            }
        }
        Tool::Height => {
            if !repeat_same_tile {
                if let Some(t) = map.tile_mut(tx, ty) {
                    let h = t.height as i32 + if ctrl { -1 } else { 1 };
                    t.height = h.clamp(0, MAX_TILE_HEIGHT as i32) as u8;
                    changed = true;
                }
            }
        }
        Tool::Prop => {
            if let Some(t) = map.tile_mut(tx, ty) {
                let new = if ctrl { 0 } else { ed.sel_prop as u8 };
                if t.prop != new && !Terrain::from_u8(t.terrain).liquid() {
                    t.prop = new;
                    changed = true;
                }
            }
        }
        Tool::Event => {
            if !repeat_same_tile && !ed.stroke_was_handled() {
                if let Some(ev) = map.event_at(tx, ty) {
                    ed.selected_event = Some(ev.id);
                } else {
                    ed.new_event_at = Some((tx, ty));
                }
            }
        }
    }
    ed.last_tile = Some((tx, ty));
    if changed {
        ed.sync_map(project);
    }
}

impl EditorState {
    fn stroke_was_handled(&self) -> bool {
        // Event tool acts once per click, not per drag.
        self.last_tile.is_some()
    }
}

fn new_event_popup(ui: &mut egui::Ui, project: &mut ProjectData, ed: &mut EditorState) {
    let Some((tx, ty)) = ed.new_event_at else { return };
    let mut open = true;
    let mut created: Option<EventKind> = None;
    egui::Window::new("New Event")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .open(&mut open)
        .show(ui.ctx(), |ui| {
            ui.label(format!("Create at ({tx}, {ty}):"));
            ui.horizontal_wrapped(|ui| {
                if ui.button("🧑 NPC").clicked() {
                    created = Some(EventKind::Npc {
                        sprite: 7,
                        persona: NpcPersona::default(),
                        wander: false,
                    });
                }
                if ui.button("🪧 Sign").clicked() {
                    created = Some(EventKind::Sign { text: "…".into() });
                }
                if ui.button("🚪 Transfer").clicked() {
                    created = Some(EventKind::Transfer {
                        target_map: project.maps.first().map(|m| m.id).unwrap_or(1),
                        target_x: 0,
                        target_y: 0,
                    });
                }
                if ui.button("🧰 Chest").clicked() {
                    created = Some(EventKind::Chest {
                        item_id: project.items.first().map(|i| i.id).unwrap_or(1),
                    });
                }
                if ui.button("⚔ Battle").clicked() {
                    created = Some(EventKind::BattleTrigger {
                        troop_id: project.troops.first().map(|t| t.id).unwrap_or(1),
                        once: true,
                    });
                }
                if ui.button("✚ Heal").clicked() {
                    created = Some(EventKind::HealPoint);
                }
            });
        });
    if let Some(kind) = created {
        if let Some(map) = project.map_mut(ed.current_map) {
            let id = ProjectData::next_event_id(map);
            map.events.push(EventData {
                id,
                name: format!("EV{id:03}"),
                x: tx,
                y: ty,
                kind,
            });
            ed.selected_event = Some(id);
        }
        ed.sync_map(project);
        ed.new_event_at = None;
    } else if !open {
        ed.new_event_at = None;
    }
}

// ---------------------------------------------------------------------------
// Left panel: tools & palettes & maps
// ---------------------------------------------------------------------------

pub fn left_panel(ui: &mut egui::Ui, project: &mut ProjectData, ed: &mut EditorState) {
    ui.heading("Tools");
    ui.horizontal_wrapped(|ui| {
        ui.selectable_value(&mut ed.tool, Tool::Terrain, "🖌 Terrain");
        ui.selectable_value(&mut ed.tool, Tool::Height, "⛰ Height");
        ui.selectable_value(&mut ed.tool, Tool::Prop, "🌲 Props");
        ui.selectable_value(&mut ed.tool, Tool::Event, "★ Events");
    });
    ui.separator();

    match ed.tool {
        Tool::Terrain => {
            ui.label(RichText::new("Terrain palette").strong());
            egui::Grid::new("terrain-palette").num_columns(2).show(ui, |ui| {
                for (i, t) in ALL_TERRAINS.iter().enumerate() {
                    let base = terrain_swatch(*t);
                    let selected = ed.sel_terrain == *t;
                    let label = RichText::new(format!("■ {}", t.name())).color(base);
                    if ui.selectable_label(selected, label).clicked() {
                        ed.sel_terrain = *t;
                    }
                    if i % 2 == 1 {
                        ui.end_row();
                    }
                }
            });
        }
        Tool::Height => {
            ui.label("Click: raise · Ctrl+Click: lower");
            ui.label(format!("Max height: {MAX_TILE_HEIGHT}"));
        }
        Tool::Prop => {
            ui.label(RichText::new("Props").strong());
            egui::Grid::new("prop-palette").num_columns(2).show(ui, |ui| {
                for (i, p) in ALL_PROPS.iter().enumerate() {
                    if ui.selectable_label(ed.sel_prop == *p, p.name()).clicked() {
                        ed.sel_prop = *p;
                    }
                    if i % 2 == 1 {
                        ui.end_row();
                    }
                }
            });
            ui.small("Ctrl+Click erases.");
        }
        Tool::Event => {
            ui.label("Click an empty tile to create an event; click an event to select it.");
        }
    }

    ui.separator();
    ui.heading("Maps");
    let mut switch_to = None;
    let mut delete_map = None;
    for m in &project.maps {
        ui.horizontal(|ui| {
            if ui.selectable_label(ed.current_map == m.id, format!("{} · {}", m.id, m.name)).clicked() {
                switch_to = Some(m.id);
            }
            if project.maps.len() > 1 && ui.small_button("🗑").on_hover_text("Delete map").clicked() {
                delete_map = Some(m.id);
            }
        });
    }
    if let Some(id) = switch_to {
        ed.switch_map(project, id);
    }
    if let Some(id) = delete_map {
        project.maps.retain(|m| m.id != id);
        if ed.current_map == id {
            let first = project.maps.first().map(|m| m.id).unwrap_or(1);
            ed.switch_map(project, first);
        }
    }
    if ui.button("＋ New map").clicked() {
        let id = project.next_map_id();
        project.maps.push(MapData::new(id, &format!("Map {id}"), 24, 24));
        ed.switch_map(project, id);
    }
}

fn terrain_swatch(t: Terrain) -> Color32 {
    let c = match t {
        Terrain::Grass => [88, 148, 68],
        Terrain::Dirt => [124, 94, 62],
        Terrain::Stone => [116, 116, 124],
        Terrain::Sand => [212, 188, 128],
        Terrain::Water => [52, 96, 168],
        Terrain::WoodFloor => [150, 110, 68],
        Terrain::StoneBrick => [136, 132, 128],
        Terrain::Snow => [228, 234, 244],
        Terrain::CaveFloor => [98, 88, 110],
        Terrain::Lava => [220, 90, 40],
    };
    Color32::from_rgb(c[0], c[1], c[2])
}

// ---------------------------------------------------------------------------
// Right panel: inspector (map settings + selected event)
// ---------------------------------------------------------------------------

pub fn right_panel(ui: &mut egui::Ui, project: &mut ProjectData, ed: &mut EditorState) {
    egui::ScrollArea::vertical().show(ui, |ui| {
        selected_event_ui(ui, project, ed);
        ui.separator();
        map_settings_ui(ui, project, ed);
        ui.separator();
        post_settings_ui(ui, ed);
    });
}

fn map_settings_ui(ui: &mut egui::Ui, project: &mut ProjectData, ed: &mut EditorState) {
    ui.heading("Map");
    let troops: Vec<(u32, String)> = project.troops.iter().map(|t| (t.id, t.name.clone())).collect();
    let Some(map) = project.map_mut(ed.current_map) else { return };
    let mut changed = false;

    ui.horizontal(|ui| {
        ui.label("Name");
        changed |= ui.text_edit_singleline(&mut map.name).changed();
    });

    ui.collapsing("Ambience (HD-2D mood)", |ui| {
        let a = &mut map.ambience;
        changed |= ui.add(egui::Slider::new(&mut a.darkness, 0.0..=1.0).text("Darkness")).changed();
        changed |= ui.add(egui::Slider::new(&mut a.fog_density, 0.0..=0.12).text("Fog density")).changed();
        changed |= ui.add(egui::Slider::new(&mut a.bloom_strength, 0.0..=2.0).text("Bloom")).changed();
        changed |= color_edit(ui, "Sun", &mut a.sun_color);
        changed |= color_edit(ui, "Ambient", &mut a.ambient_color);
        changed |= color_edit(ui, "Fog", &mut a.fog_color);
    });

    ui.collapsing("Random encounters", |ui| {
        let mut steps = map.encounter_steps as i32;
        if ui.add(egui::Slider::new(&mut steps, 0..=60).text("Avg steps (0=off)")).changed() {
            map.encounter_steps = steps as u32;
            changed = true;
        }
        for (id, name) in &troops {
            let mut on = map.encounter_troops.contains(id);
            if ui.checkbox(&mut on, name).changed() {
                if on {
                    map.encounter_troops.push(*id);
                } else {
                    map.encounter_troops.retain(|t| t != id);
                }
                changed = true;
            }
        }
    });

    if changed {
        ed.sync_map(project);
    }
}

fn post_settings_ui(ui: &mut egui::Ui, ed: &mut EditorState) {
    ui.heading("HD-2D Post FX");
    ui.checkbox(&mut ed.preview_fx, "Preview in editor");
    let p = &mut ed.post;
    ui.add(egui::Slider::new(&mut p.dof_strength, 0.0..=1.0).text("Tilt-shift"));
    ui.add(egui::Slider::new(&mut p.focus_y, 0.2..=0.8).text("Focus height"));
    ui.add(egui::Slider::new(&mut p.bloom_threshold, 0.2..=1.5).text("Bloom threshold"));
    ui.add(egui::Slider::new(&mut p.vignette, 0.0..=1.0).text("Vignette"));
    ui.add(egui::Slider::new(&mut p.exposure, 0.5..=2.0).text("Exposure"));
    ui.add(egui::Slider::new(&mut p.saturation, 0.5..=1.6).text("Saturation"));
}

fn color_edit(ui: &mut egui::Ui, label: &str, c: &mut [f32; 3]) -> bool {
    let mut rgb = [c[0], c[1], c[2]];
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label(label);
        if ui.color_edit_button_rgb(&mut rgb).changed() {
            *c = rgb;
            changed = true;
        }
    });
    changed
}

fn selected_event_ui(ui: &mut egui::Ui, project: &mut ProjectData, ed: &mut EditorState) {
    ui.heading("Event");
    let Some(sel) = ed.selected_event else {
        ui.small("No event selected. Use the Events tool.");
        return;
    };
    let map_id = ed.current_map;
    let mut changed = false;
    let mut delete = false;
    {
        let maps_list: Vec<(u32, String)> = project.maps.iter().map(|m| (m.id, m.name.clone())).collect();
        let items_list: Vec<(u32, String)> = project.items.iter().map(|i| (i.id, i.name.clone())).collect();
        let troops_list: Vec<(u32, String)> = project.troops.iter().map(|t| (t.id, t.name.clone())).collect();
        let Some(map) = project.map_mut(map_id) else { return };
        let Some(ev) = map.events.iter_mut().find(|e| e.id == sel) else {
            ed.selected_event = None;
            return;
        };

        ui.horizontal(|ui| {
            ui.label(format!("#{} {}", ev.id, ev.kind.label()));
            if ui.small_button("🗑 Delete").clicked() {
                delete = true;
            }
        });
        ui.horizontal(|ui| {
            ui.label("Name");
            changed |= ui.text_edit_singleline(&mut ev.name).changed();
        });
        ui.label(format!("At ({}, {})", ev.x, ev.y));

        match &mut ev.kind {
            EventKind::Npc { sprite, persona, wander } => {
                ui.horizontal(|ui| {
                    ui.label("Sprite");
                    let mut s = *sprite as i32;
                    if ui.add(egui::Slider::new(&mut s, 0..=7)).changed() {
                        *sprite = s as u8;
                        changed = true;
                    }
                });
                changed |= ui.checkbox(wander, "Wanders around").changed();
                ui.separator();
                ui.label(RichText::new("Persona (drives the local LLM)").strong());
                changed |= ui.checkbox(&mut persona.use_llm, "LLM dialogue enabled").changed();
                ui.label("Display name");
                changed |= ui.text_edit_singleline(&mut persona.name).changed();
                ui.label("Role (one line)");
                changed |= ui.text_edit_singleline(&mut persona.role).changed();
                ui.label("Personality & speech style");
                changed |= ui.text_edit_multiline(&mut persona.personality).changed();
                ui.label("Knowledge (facts they can share)");
                changed |= ui.text_edit_multiline(&mut persona.knowledge).changed();
                ui.label("Constraints (hard rules)");
                changed |= ui.text_edit_multiline(&mut persona.constraints).changed();
                ui.label("Fallback lines (no-LLM mode, one per line)");
                let mut lines = persona.fallback_lines.join("\n");
                if ui.text_edit_multiline(&mut lines).changed() {
                    persona.fallback_lines = lines.lines().map(|l| l.to_string()).collect();
                    changed = true;
                }
            }
            EventKind::Sign { text } => {
                ui.label("Text");
                changed |= ui.text_edit_multiline(text).changed();
            }
            EventKind::Transfer { target_map, target_x, target_y } => {
                egui::ComboBox::from_label("Target map")
                    .selected_text(
                        maps_list
                            .iter()
                            .find(|(id, _)| id == target_map)
                            .map(|(_, n)| n.clone())
                            .unwrap_or_else(|| "?".into()),
                    )
                    .show_ui(ui, |ui| {
                        for (id, name) in &maps_list {
                            changed |= ui.selectable_value(target_map, *id, name).changed();
                        }
                    });
                ui.horizontal(|ui| {
                    ui.label("X");
                    changed |= ui.add(egui::DragValue::new(target_x)).changed();
                    ui.label("Y");
                    changed |= ui.add(egui::DragValue::new(target_y)).changed();
                });
            }
            EventKind::Chest { item_id } => {
                egui::ComboBox::from_label("Item")
                    .selected_text(
                        items_list
                            .iter()
                            .find(|(id, _)| id == item_id)
                            .map(|(_, n)| n.clone())
                            .unwrap_or_else(|| "?".into()),
                    )
                    .show_ui(ui, |ui| {
                        for (id, name) in &items_list {
                            changed |= ui.selectable_value(item_id, *id, name).changed();
                        }
                    });
            }
            EventKind::BattleTrigger { troop_id, once } => {
                egui::ComboBox::from_label("Troop")
                    .selected_text(
                        troops_list
                            .iter()
                            .find(|(id, _)| id == troop_id)
                            .map(|(_, n)| n.clone())
                            .unwrap_or_else(|| "?".into()),
                    )
                    .show_ui(ui, |ui| {
                        for (id, name) in &troops_list {
                            changed |= ui.selectable_value(troop_id, *id, name).changed();
                        }
                    });
                changed |= ui.checkbox(once, "Disappears after victory").changed();
            }
            EventKind::HealPoint => {
                ui.small("Fully heals the party when used.");
            }
        }

        if delete {
            let id = ev.id;
            map.events.retain(|e| e.id != id);
            ed.selected_event = None;
            changed = true;
        }
    }
    if changed {
        ed.sync_map(project);
    }
}
