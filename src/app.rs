//! Top-level application: mode switching (edit ⇄ playtest), menu bar,
//! project file management, play-mode UI (dialogue, battle, game over).

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};

use eframe::egui;
use glam::Vec3;

use crate::core::aigen;
use crate::core::data::*;
use crate::core::defaults::default_project;
use crate::core::io;
use crate::editor::{self, EditorState, GenJob};
use crate::game::Game;
use crate::gfx::mesh::SPRITE_HORIZONTAL;
use crate::gfx::pixelart::{build_atlas, Atlas, CHAR_FRAMES};
use crate::gfx::renderer::{Hd2dCallback, Hd2dRenderer};
use crate::gfx::scene;
use crate::llm::{ChatTurn, GenRequest, LlmEngine, LlmEvent, LlmStatus};

static ATLAS: OnceLock<Arc<Atlas>> = OnceLock::new();
static REVISION: AtomicU64 = AtomicU64::new(1);

pub fn atlas() -> &'static Atlas {
    ATLAS.get().expect("atlas initialized in App::new")
}

pub fn next_revision() -> u64 {
    REVISION.fetch_add(1, Ordering::Relaxed)
}

pub struct App {
    project: ProjectData,
    project_path: Option<PathBuf>,
    editor: EditorState,
    game: Option<Game>,
    llm: LlmEngine,
    start: std::time::Instant,
    toast: Option<(String, f32)>,
    /// Live audio stream; kept alive for the lifetime of the app.
    _audio: crate::audio::AudioStream,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let atlas = ATLAS.get_or_init(|| Arc::new(build_atlas())).clone();
        let rs = cc
            .wgpu_render_state
            .as_ref()
            .expect("NewOldMaker requires the wgpu backend");
        let renderer = Hd2dRenderer::new(&rs.device, &rs.queue, rs.target_format, atlas);
        rs.renderer.write().callback_resources.insert(renderer);

        let project = default_project(Language::default());
        let editor = EditorState::new(&project);
        App {
            project,
            project_path: None,
            editor,
            game: None,
            llm: LlmEngine::new(),
            start: std::time::Instant::now(),
            toast: None,
            _audio: crate::audio::init(),
        }
    }

    /// Stop the current playtest and fade the music back to silence.
    fn stop_game(&mut self) {
        self.game = None;
        crate::audio::music(crate::audio::Track::Silence);
    }

    fn toast(&mut self, msg: impl Into<String>) {
        self.toast = Some((msg.into(), 2.5));
    }

    // -----------------------------------------------------------------
    // File operations
    // -----------------------------------------------------------------

    fn save_project(&mut self, save_as: bool) {
        let path = if save_as || self.project_path.is_none() {
            rfd::FileDialog::new()
                .add_filter("NewOldMaker project", &["json"])
                .set_file_name(format!("{}.nom.json", self.project.name.replace(' ', "_")))
                .save_file()
        } else {
            self.project_path.clone()
        };
        if let Some(path) = path {
            match io::save_project(&self.project, &path) {
                Ok(()) => {
                    self.project_path = Some(path);
                    self.toast("Project saved");
                }
                Err(e) => self.toast(format!("Save failed: {e}")),
            }
        }
    }

    fn open_project(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("NewOldMaker project", &["json"])
            .pick_file()
        {
            match io::load_project(&path) {
                Ok(p) => {
                    self.project = p;
                    self.project_path = Some(path);
                    self.editor = EditorState::new(&self.project);
                    self.stop_game();
                    self.toast("Project loaded");
                }
                Err(e) => self.toast(format!("Open failed: {e}")),
            }
        }
    }

    fn new_project(&mut self) {
        // Keep the language the user currently has selected.
        self.project = default_project(self.project.system.language);
        self.project_path = None;
        self.editor = EditorState::new(&self.project);
        self.stop_game();
    }

    /// When the language changes and the project is still the untouched starter
    /// template, regenerate it in the new language so the default content
    /// (spell names, items, enemies, maps…) follows the language too. Any
    /// project the user has actually edited is left alone — only the language
    /// field (already flipped by the picker) changes there.
    fn relocalize_default_content(&mut self, old_lang: Language, new_lang: Language) {
        // Never swap content out from under a running playtest.
        if self.game.is_some() {
            return;
        }
        // Compare against the pristine default for the previous language.
        // Serializing both sides avoids needing `PartialEq` on every data type;
        // forcing the language field equal isolates the comparison to content.
        let mut baseline = self.project.clone();
        baseline.system.language = old_lang;
        let is_pristine = serde_json::to_string(&baseline).ok()
            == serde_json::to_string(&default_project(old_lang)).ok();
        if is_pristine {
            self.project = default_project(new_lang);
            self.editor = EditorState::new(&self.project);
        }
    }

    // -----------------------------------------------------------------
    // Menu bar
    // -----------------------------------------------------------------

    fn menu_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("🗺 NewOldMaker").strong());
            ui.separator();
            ui.menu_button("File", |ui| {
                if ui.button("New project").clicked() {
                    self.new_project();
                    ui.close();
                }
                if ui.button("Open…").clicked() {
                    self.open_project();
                    ui.close();
                }
                if ui.button("Save            Ctrl+S").clicked() {
                    self.save_project(false);
                    ui.close();
                }
                if ui.button("Save As…").clicked() {
                    self.save_project(true);
                    ui.close();
                }
            });
            if ui.button("🗄 Database").clicked() {
                self.editor.show_database = !self.editor.show_database;
            }
            if self.game.is_none() {
                if ui.button("↶ Undo").clicked() {
                    self.editor.apply_undo(&mut self.project);
                }
                if ui
                    .button(
                        egui::RichText::new("▶ Playtest  (F5)")
                            .color(egui::Color32::from_rgb(120, 220, 120)),
                    )
                    .clicked()
                {
                    self.game = Some(Game::new(&self.project));
                }
            } else if ui
                .button(
                    egui::RichText::new("⏹ Stop  (F5)")
                        .color(egui::Color32::from_rgb(240, 120, 100)),
                )
                .clicked()
            {
                self.stop_game();
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let (icon, hint) = if crate::audio::is_muted() {
                    ("🔇", "Sound off")
                } else {
                    ("🔊", "Sound on")
                };
                if ui.button(icon).on_hover_text(hint).clicked() {
                    crate::audio::toggle_muted();
                }
                ui.separator();
                let old_lang = self.project.system.language;
                let lang = &mut self.project.system.language;
                egui::ComboBox::from_id_salt("game-language")
                    .selected_text(lang.name())
                    .show_ui(ui, |ui| {
                        for l in ALL_LANGUAGES {
                            ui.selectable_value(lang, l, l.name());
                        }
                    })
                    .response
                    .on_hover_text("Game language");
                let new_lang = self.project.system.language;
                if new_lang != old_lang {
                    self.relocalize_default_content(old_lang, new_lang);
                }
                ui.separator();
                match &self.llm.status {
                    LlmStatus::Off => ui.weak("LLM: off"),
                    LlmStatus::Loading => ui.colored_label(egui::Color32::YELLOW, "LLM: loading…"),
                    LlmStatus::Ready(name) => ui.colored_label(
                        egui::Color32::from_rgb(120, 220, 120),
                        format!("LLM: {name}"),
                    ),
                    LlmStatus::Error(e) => ui
                        .colored_label(egui::Color32::from_rgb(240, 120, 100), format!("LLM: {e}")),
                };
                ui.label(format!(
                    "· {}",
                    self.project_path
                        .as_ref()
                        .map(|p| p.display().to_string())
                        .unwrap_or_else(|| "unsaved".into())
                ));
            });
        });
    }

    // -----------------------------------------------------------------
    // Play mode
    // -----------------------------------------------------------------

    fn play_view(&mut self, ui: &mut egui::Ui, dt: f32) {
        let ctx = ui.ctx().clone();
        let lang = self.project.system.language;
        let Some(game) = &mut self.game else { return };
        game.update(&self.project, &mut self.llm, &ctx, dt);

        let avail = ui.available_size();
        let (rect, _response) = ui.allocate_exact_size(avail, egui::Sense::hover());
        let ppp = ctx.pixels_per_point();
        let viewport_px = [
            (rect.width() * ppp).round().max(8.0) as u32,
            (rect.height() * ppp).round().max(8.0) as u32,
        ];
        let atlas = atlas();
        let mut cutout = Vec::with_capacity(256);
        let mut blend = Vec::with_capacity(32);
        let mut lights = Vec::new();
        scene::map_prop_sprites(&game.map, atlas, &mut cutout, &mut lights);

        // Visible events (signs, unopened chests, heal points).
        for ev in &game.map.events {
            if game.done_events.contains(&(game.map_id, ev.id)) {
                continue;
            }
            let top = crate::gfx::mesh::tile_top_y(&game.map, ev.x, ev.y);
            let pos = Vec3::new(ev.x as f32 + 0.5, top, ev.y as f32 + 0.5);
            match &ev.kind {
                EventKind::Sign { .. } => {
                    cutout.push(scene::sprite(
                        pos,
                        scene::PROP_SIZE,
                        atlas.props[Prop::Signpost as usize],
                        [1.0; 4],
                        0,
                    ));
                }
                EventKind::Chest { .. } => {
                    cutout.push(scene::sprite(
                        pos,
                        scene::PROP_SIZE,
                        atlas.props[Prop::Barrel as usize],
                        [1.3, 1.1, 0.5, 1.0],
                        0,
                    ));
                }
                EventKind::HealPoint => {
                    cutout.push(scene::sprite(
                        pos,
                        scene::PROP_SIZE,
                        atlas.props[Prop::Crystal as usize],
                        [0.6, 1.3, 0.7, 1.0],
                        0,
                    ));
                    lights.push(crate::gfx::renderer::LightSpec {
                        pos: pos + Vec3::Y * 1.0,
                        radius: 3.5,
                        color: [0.3, 1.0, 0.5],
                    });
                }
                _ => {}
            }
        }

        let camera;
        if let Some(battle) = &game.battle {
            camera = battle.camera;
            for (_, f, tint, dir_frame) in battle.fighter_visuals(game.time) {
                if f.is_player {
                    let dir = dir_frame / CHAR_FRAMES;
                    let frame = dir_frame % CHAR_FRAMES;
                    scene::char_sprites(
                        atlas,
                        f.sprite as usize,
                        dir,
                        frame,
                        f.pos,
                        tint,
                        &mut cutout,
                        &mut blend,
                    );
                } else {
                    let uv = atlas.enemies[f.sprite as usize % atlas.enemies.len()];
                    cutout.push(scene::sprite(
                        f.pos + Vec3::Y * 0.02,
                        [1.5, 1.5],
                        uv,
                        tint,
                        0,
                    ));
                    blend.push(scene::sprite(
                        f.pos + Vec3::Y * 0.015,
                        [1.1, 0.6],
                        atlas.shadow,
                        [1.0, 1.0, 1.0, 0.8],
                        SPRITE_HORIZONTAL | crate::gfx::mesh::SPRITE_UNLIT,
                    ));
                }
            }
        } else {
            camera = game.camera;
            // NPCs.
            for npc in &game.npcs {
                let pos = game.npc_world_pos(npc);
                let frame = if npc.move_t < 1.0 {
                    ((game.time * 8.0) as u32) % CHAR_FRAMES
                } else {
                    1
                };
                scene::char_sprites(
                    atlas,
                    npc.sprite as usize,
                    npc.dir,
                    frame,
                    pos,
                    [1.0; 4],
                    &mut cutout,
                    &mut blend,
                );
            }
            // Player.
            let frame = if game.player.moving {
                ((game.player.anim * 9.0) as u32) % CHAR_FRAMES
            } else {
                1
            };
            scene::char_sprites(
                atlas,
                game.party.first().map(|m| m.sprite as usize).unwrap_or(0),
                game.player.dir,
                frame,
                game.player_world_pos(),
                [1.0; 4],
                &mut cutout,
                &mut blend,
            );
        }

        let aspect = viewport_px[0] as f32 / viewport_px[1].max(1) as f32;
        let view_proj = camera.view_proj(aspect);
        let input = scene::frame_input(
            game.map.clone(),
            game.map_revision,
            &camera,
            viewport_px,
            game.time,
            lights,
            cutout,
            blend,
            game.post,
        );
        ui.painter()
            .add(eframe::egui_wgpu::Callback::new_paint_callback(
                rect,
                Hd2dCallback {
                    input: Arc::new(input),
                },
            ));

        // Damage popups projected onto the viewport.
        if let Some(battle) = &game.battle {
            for p in &battle.popups {
                let ndc = view_proj.project_point3(p.pos);
                if ndc.z > 0.0 && ndc.z < 1.0 {
                    let sx = rect.left() + (ndc.x + 1.0) * 0.5 * rect.width();
                    let sy = rect.top() + (1.0 - ndc.y) * 0.5 * rect.height();
                    let alpha = ((1.2 - p.age) / 1.2).clamp(0.0, 1.0);
                    let col = egui::Color32::from_rgba_unmultiplied(
                        (p.color[0] * 255.0) as u8,
                        (p.color[1] * 255.0) as u8,
                        (p.color[2] * 255.0) as u8,
                        (alpha * 255.0) as u8,
                    );
                    ui.painter().text(
                        egui::pos2(sx, sy),
                        egui::Align2::CENTER_CENTER,
                        &p.text,
                        egui::FontId::proportional(20.0),
                        col,
                    );
                }
            }
        }

        // Controls hint.
        if game.battle.is_none() && game.dialogue.is_none() {
            ui.painter().text(
                rect.left_bottom() + egui::vec2(8.0, -8.0),
                egui::Align2::LEFT_BOTTOM,
                lang.controls_hint(),
                egui::FontId::monospace(12.0),
                egui::Color32::from_white_alpha(160),
            );
        }

        // Overlays.
        if let Some(battle) = &mut game.battle {
            battle.ui(&ctx, &self.project);
        }
        self.dialogue_ui(&ctx);
        self.game_over_ui(&ctx);
    }

    fn dialogue_ui(&mut self, ctx: &egui::Context) {
        let lang = self.project.system.language;
        let Some(game) = &mut self.game else { return };
        let Some(dialogue) = &mut game.dialogue else {
            return;
        };
        let mut close = false;
        let mut send: Option<String> = None;

        egui::Window::new("dialogue")
            .title_bar(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_BOTTOM, [0.0, -24.0])
            .min_width(480.0)
            .show(ctx, |ui| {
                ui.set_width(480.0);
                if !dialogue.speaker.is_empty() {
                    ui.label(
                        egui::RichText::new(&dialogue.speaker)
                            .strong()
                            .color(egui::Color32::from_rgb(255, 210, 120)),
                    );
                }
                let text = if dialogue.text.is_empty() && dialogue.streaming {
                    "…".to_string()
                } else {
                    format!(
                        "{}{}",
                        dialogue.text,
                        if dialogue.streaming { " ▌" } else { "" }
                    )
                };
                ui.label(egui::RichText::new(text).size(15.0));
                ui.add_space(6.0);
                if let Some(chat) = &mut dialogue.chat {
                    ui.horizontal(|ui| {
                        let editing = egui::TextEdit::singleline(&mut chat.input)
                            .hint_text(lang.say_something())
                            .desired_width(360.0)
                            .show(ui);
                        let enter = editing.response.lost_focus()
                            && ui.input(|i| i.key_pressed(egui::Key::Enter));
                        let clicked = ui
                            .add_enabled(
                                !dialogue.streaming && !chat.input.trim().is_empty(),
                                egui::Button::new(lang.send()),
                            )
                            .clicked();
                        if (enter || clicked)
                            && !dialogue.streaming
                            && !chat.input.trim().is_empty()
                        {
                            send = Some(chat.input.trim().to_string());
                            chat.input.clear();
                        }
                        if ui.button(lang.leave()).clicked() {
                            close = true;
                        }
                        editing.response.request_focus();
                    });
                } else {
                    ui.small(lang.close_hint());
                    if ctx.input(|i| {
                        i.key_pressed(egui::Key::Z)
                            || i.key_pressed(egui::Key::Enter)
                            || i.key_pressed(egui::Key::Space)
                            || i.key_pressed(egui::Key::Escape)
                    }) {
                        close = true;
                    }
                }
            });

        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            close = true;
        }

        if let Some(line) = send {
            let location = game.map.name.clone();
            let player_name = game
                .party
                .first()
                .map(|m| m.name.clone())
                .unwrap_or("the hero".into());
            if let Some(d) = &mut game.dialogue {
                if let Some(chat) = &mut d.chat {
                    // NPC's streamed reply was appended to history on Done; add the player's line.
                    chat.history.push(ChatTurn {
                        from_player: true,
                        text: line,
                    });
                    let req = crate::llm::ChatRequest {
                        id: 0,
                        persona: chat.persona.clone(),
                        game_title: self.project.system.title.clone(),
                        location,
                        player_name,
                        history: chat.history.clone(),
                        max_tokens: self.project.llm.max_reply_tokens,
                        temperature: self.project.llm.temperature,
                        language: self.project.system.language,
                    };
                    d.text.clear();
                    chat.pending_req = self.llm.request(req);
                    d.streaming = chat.pending_req.is_some();
                    if chat.pending_req.is_none() {
                        d.text = "…".into();
                    }
                }
            }
        } else if close {
            game.dialogue = None;
            crate::audio::sfx(crate::audio::Sfx::Cancel);
        }
    }

    fn game_over_ui(&mut self, ctx: &egui::Context) {
        let lang = self.project.system.language;
        let Some(game) = &self.game else { return };
        if !game.game_over {
            return;
        }
        let mut stop = false;
        egui::Window::new(lang.game_over())
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.heading(lang.party_fallen());
                if ui.button(lang.return_to_editor()).clicked() {
                    stop = true;
                }
            });
        if stop {
            self.stop_game();
        }
    }

    // -----------------------------------------------------------------
    // AI content generation (Database → ✨ AI)
    // -----------------------------------------------------------------

    /// Feed streamed LLM events into the active generation job; when it finishes,
    /// parse the JSON and splice the new content into the project.
    fn process_gen_events(&mut self, events: Vec<LlmEvent>) {
        let Some(job) = self.editor.genai.job.as_mut() else {
            return;
        };
        let mut done = false;
        let mut errored = None;
        for ev in events {
            match ev {
                LlmEvent::Token { id, text } if id == job.id => job.buffer.push_str(&text),
                LlmEvent::Done { id } if id == job.id => done = true,
                LlmEvent::Error { id, msg } if id == job.id => errored = Some(msg),
                _ => {}
            }
        }
        if let Some(msg) = errored {
            self.editor.genai.status = Some(format!("Generation failed: {msg}"));
            self.editor.genai.job = None;
            return;
        }
        if !done {
            return;
        }
        let job = self.editor.genai.job.take().expect("job present");
        match aigen::apply(job.target, &mut self.project, &job.buffer) {
            Ok(applied) => {
                if let Some(id) = applied.new_map {
                    self.editor.switch_map(&self.project, id);
                } else {
                    self.editor.sync_map(&self.project);
                }
                self.editor.genai.status = Some(applied.summary.clone());
                self.toast(applied.summary);
            }
            Err(e) => self.editor.genai.status = Some(e),
        }
    }

    /// If the AI tab requested a generation this frame, build the prompt and
    /// dispatch it to the LLM worker.
    fn start_gen_if_requested(&mut self) {
        if !self.editor.genai.submit {
            return;
        }
        self.editor.genai.submit = false;
        if self.editor.genai.job.is_some() {
            return;
        }
        let target = self.editor.genai.target;
        let count = self.editor.genai.count.clamp(1, 12);
        let language = self.project.system.language;
        let req = GenRequest {
            id: 0,
            system: aigen::system_prompt(target, &self.project, language),
            prompt: aigen::user_prompt(target, count, &self.editor.genai.prompt),
            max_tokens: target.max_tokens(count),
            // Structured output wants less randomness than chatty dialogue.
            temperature: (self.project.llm.temperature * 0.6).clamp(0.1, 0.9),
        };
        match self.llm.generate(req) {
            Some(id) => {
                self.editor.genai.job = Some(GenJob {
                    id,
                    target,
                    buffer: String::new(),
                });
                self.editor.genai.status = Some(format!("Generating {}…", target.label()));
            }
            None => {
                self.editor.genai.status =
                    Some("LLM is not ready. Configure a backend in the LLM tab.".into());
            }
        }
    }
}

// ---------------------------------------------------------------------------
// eframe::App
// ---------------------------------------------------------------------------

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        ctx.request_repaint(); // live viewport
        let dt = ctx.input(|i| i.stable_dt).min(0.1);
        let time = self.start.elapsed().as_secs_f32();

        self.llm.configure(&self.project.llm);
        if self.game.is_none() {
            // Keep the LLM status fresh and drive any in-flight AI generation.
            let events = self.llm.poll();
            self.process_gen_events(events);
        }

        // Global shortcuts.
        if ctx.input(|i| i.key_pressed(egui::Key::F5)) {
            if self.game.is_some() {
                self.stop_game();
            } else {
                self.game = Some(Game::new(&self.project));
            }
        }
        if ctx.input(|i| i.modifiers.ctrl && i.key_pressed(egui::Key::S)) {
            self.save_project(false);
        }
        if self.game.is_none() && ctx.input(|i| i.modifiers.ctrl && i.key_pressed(egui::Key::Z)) {
            self.editor.apply_undo(&mut self.project);
        }

        egui::Panel::top("menu").show(ui, |ui| {
            self.menu_bar(ui);
        });

        if self.game.is_some() {
            egui::CentralPanel::no_frame().show(ui, |ui| {
                self.play_view(ui, dt);
            });
        } else {
            egui::Panel::left("tools")
                .default_size(230.0)
                .show(ui, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        editor::left_panel(ui, &mut self.project, &mut self.editor);
                    });
                });
            egui::Panel::right("inspector")
                .default_size(280.0)
                .show(ui, |ui| {
                    editor::right_panel(ui, &mut self.project, &mut self.editor);
                });
            egui::CentralPanel::no_frame().show(ui, |ui| {
                editor::viewport(ui, &mut self.project, &mut self.editor, time);
            });
            if self.editor.show_database {
                let mut open = true;
                let mut tab = self.editor.db_tab;
                let llm_ready = self.llm.ready();
                editor::database::database_window(
                    &ctx,
                    &mut self.project,
                    &mut open,
                    &mut tab,
                    &mut self.editor.genai,
                    llm_ready,
                );
                self.editor.db_tab = tab;
                if !open {
                    self.editor.show_database = false;
                }
                // Database edits can touch the current map's troops etc.
                // (tiles are untouched, so no resync needed here)
            }
            // Dispatch an AI generation request queued by the database's AI tab.
            self.start_gen_if_requested();
        }

        // Toast.
        if let Some((msg, ttl)) = &mut self.toast {
            *ttl -= dt;
            let msg = msg.clone();
            let ttl = *ttl;
            egui::Area::new(egui::Id::new("toast"))
                .anchor(egui::Align2::CENTER_TOP, [0.0, 40.0])
                .show(&ctx, |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.label(msg);
                    });
                });
            if ttl <= 0.0 {
                self.toast = None;
            }
        }
    }
}
