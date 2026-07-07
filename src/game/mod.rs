//! Playtest runtime: grid movement, events, NPCs, dialogue, encounters.

pub mod battle;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use eframe::egui;
use glam::Vec3;
use rand::RngExt;

use crate::audio::{self, Sfx, Track};
use crate::core::data::*;
use crate::gfx::camera::OrbitCamera;
use crate::gfx::mesh::tile_top_y;
use crate::gfx::renderer::PostSettings;
use crate::llm::{ChatRequest, ChatTurn, LlmEngine, LlmEvent};

pub const DIR_DOWN: u32 = 0;
pub const DIR_LEFT: u32 = 1;
pub const DIR_RIGHT: u32 = 2;
pub const DIR_UP: u32 = 3;

fn dir_delta(dir: u32) -> (i32, i32) {
    match dir {
        DIR_LEFT => (-1, 0),
        DIR_RIGHT => (1, 0),
        DIR_UP => (0, -1),
        _ => (0, 1),
    }
}

// ---------------------------------------------------------------------------
// Party
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct Member {
    pub actor_id: u32,
    pub name: String,
    #[allow(dead_code)] // shown in future menus; part of the party model
    pub class_name: String,
    pub sprite: u8,
    pub level: u32,
    pub exp: u32,
    pub max: Stats,
    pub hp: i32,
    pub mp: i32,
    pub skills: Vec<u32>,
    pub attack_element: Element,
}

pub fn exp_to_next(level: u32) -> u32 {
    30 + level * 22
}

pub fn member_from_actor(actor: &Actor, level: u32) -> Member {
    let l = level.max(1) as i32 - 1;
    let max = Stats {
        hp: actor.base.hp + actor.growth.hp * l,
        mp: actor.base.mp + actor.growth.mp * l,
        atk: actor.base.atk + actor.growth.atk * l,
        def: actor.base.def + actor.growth.def * l,
        mag: actor.base.mag + actor.growth.mag * l,
        spr: actor.base.spr + actor.growth.spr * l,
        spd: actor.base.spd + actor.growth.spd * l,
    };
    Member {
        actor_id: actor.id,
        name: actor.name.clone(),
        class_name: actor.class_name.clone(),
        sprite: actor.sprite,
        level: level.max(1),
        exp: 0,
        max,
        hp: max.hp,
        mp: max.mp,
        skills: actor
            .learnset
            .iter()
            .filter(|(lv, _)| *lv <= level.max(1))
            .map(|(_, id)| *id)
            .collect(),
        attack_element: actor.attack_element,
    }
}

// ---------------------------------------------------------------------------
// Dialogue
// ---------------------------------------------------------------------------

pub struct NpcChat {
    #[allow(dead_code)]
    pub event_id: u32,
    pub persona: NpcPersona,
    pub history: Vec<ChatTurn>,
    pub pending_req: Option<u64>,
    pub input: String,
}

pub struct Dialogue {
    pub speaker: String,
    pub text: String,
    /// Some = interactive LLM chat; None = plain message box.
    pub chat: Option<NpcChat>,
    pub streaming: bool,
    /// Seconds until the dialogue auto-closes; None = stays until dismissed.
    pub auto_close: Option<f32>,
}

// ---------------------------------------------------------------------------
// Game state
// ---------------------------------------------------------------------------

pub struct NpcRuntime {
    pub event_id: u32,
    pub x: i32,
    pub y: i32,
    pub home: (i32, i32),
    pub dir: u32,
    pub sprite: u8,
    pub wander: bool,
    pub move_t: f32,
    pub from: (i32, i32),
    pub wander_timer: f32,
}

pub struct Player {
    pub x: i32,
    pub y: i32,
    pub from: (i32, i32),
    pub move_t: f32,
    pub moving: bool,
    pub dir: u32,
    pub anim: f32,
}

pub struct Game {
    pub map_id: u32,
    pub map: Arc<MapData>,
    pub map_revision: u64,
    pub camera: OrbitCamera,
    pub player: Player,
    pub party: Vec<Member>,
    pub inventory: Vec<(u32, u32)>,
    pub npcs: Vec<NpcRuntime>,
    /// (map_id, event_id) of consumed one-shot events.
    pub done_events: HashSet<(u32, u32)>,
    pub fallback_cursor: HashMap<u32, usize>,
    pub dialogue: Option<Dialogue>,
    pub battle: Option<battle::Battle>,
    pub steps: u32,
    pub time: f32,
    pub game_over: bool,
    pub post: PostSettings,
}

impl Game {
    pub fn new(project: &ProjectData) -> Self {
        let map_id = project.system.start_map;
        let map: Arc<MapData> = Arc::new(
            project.map(map_id).or_else(|| project.maps.first()).expect("maps").clone(),
        );
        let party = project
            .system
            .party
            .iter()
            .filter_map(|id| project.actor(*id))
            .take(4)
            .map(|a| member_from_actor(a, 1))
            .collect();
        let mut camera = OrbitCamera::default();
        camera.dist = 11.0;
        camera.pitch = 0.86;
        let mut game = Game {
            map_id: map.id,
            map_revision: crate::app::next_revision(),
            camera,
            player: Player {
                x: project.system.start_x,
                y: project.system.start_y,
                from: (project.system.start_x, project.system.start_y),
                move_t: 1.0,
                moving: false,
                dir: DIR_DOWN,
                anim: 0.0,
            },
            party,
            inventory: project.system.start_items.clone(),
            npcs: Vec::new(),
            done_events: HashSet::new(),
            fallback_cursor: HashMap::new(),
            dialogue: None,
            battle: None,
            steps: 0,
            time: 0.0,
            game_over: false,
            post: PostSettings::default(),
            map,
        };
        game.spawn_npcs();
        audio::music(Track::for_map(&game.map));
        game
    }

    fn spawn_npcs(&mut self) {
        self.npcs.clear();
        for ev in &self.map.events {
            if let EventKind::Npc { sprite, wander, .. } = &ev.kind {
                self.npcs.push(NpcRuntime {
                    event_id: ev.id,
                    x: ev.x,
                    y: ev.y,
                    home: (ev.x, ev.y),
                    dir: DIR_DOWN,
                    sprite: *sprite,
                    wander: *wander,
                    move_t: 1.0,
                    from: (ev.x, ev.y),
                    wander_timer: 1.0,
                });
            }
        }
    }

    pub fn transfer(&mut self, project: &ProjectData, map_id: u32, x: i32, y: i32) {
        if let Some(map) = project.map(map_id) {
            self.map = Arc::new(map.clone());
            self.map_id = map_id;
            self.map_revision = crate::app::next_revision();
            self.player.x = x;
            self.player.y = y;
            self.player.from = (x, y);
            self.player.move_t = 1.0;
            self.player.moving = false;
            self.spawn_npcs();
            audio::music(Track::for_map(&self.map));
        }
    }

    pub fn player_world_pos(&self) -> Vec3 {
        let t = self.player.move_t.clamp(0.0, 1.0);
        let (fx, fy) = self.player.from;
        let x = fx as f32 + (self.player.x - fx) as f32 * t + 0.5;
        let z = fy as f32 + (self.player.y - fy) as f32 * t + 0.5;
        let y0 = tile_top_y(&self.map, fx, fy);
        let y1 = tile_top_y(&self.map, self.player.x, self.player.y);
        Vec3::new(x, y0 + (y1 - y0) * t, z)
    }

    pub fn npc_world_pos(&self, npc: &NpcRuntime) -> Vec3 {
        let t = npc.move_t.clamp(0.0, 1.0);
        let (fx, fy) = npc.from;
        let x = fx as f32 + (npc.x - fx) as f32 * t + 0.5;
        let z = fy as f32 + (npc.y - fy) as f32 * t + 0.5;
        let y0 = tile_top_y(&self.map, fx, fy);
        let y1 = tile_top_y(&self.map, npc.x, npc.y);
        Vec3::new(x, y0 + (y1 - y0) * t, z)
    }

    fn walkable(&self, x: i32, y: i32, from_x: i32, from_y: i32) -> bool {
        if !self.map.in_bounds(x, y) {
            return false;
        }
        let t = self.map.tile(x, y);
        if !Terrain::from_u8(t.terrain).walkable() || Prop::from_u8(t.prop).blocks() {
            return false;
        }
        let from = self.map.tile(from_x, from_y);
        if (t.height as i32 - from.height as i32).abs() > 1 {
            return false;
        }
        if let Some(ev) = self.map.event_at(x, y) {
            if ev.kind.blocks() && !self.done_events.contains(&(self.map_id, ev.id)) {
                return false;
            }
        }
        for npc in &self.npcs {
            if npc.x == x && npc.y == y {
                return false;
            }
        }
        true
    }

    // -----------------------------------------------------------------
    // Per-frame update (returns requested transfer, handled by caller)
    // -----------------------------------------------------------------

    pub fn update(
        &mut self,
        project: &ProjectData,
        llm: &mut LlmEngine,
        ctx: &egui::Context,
        dt: f32,
    ) {
        self.time += dt;

        // Drain LLM events into the open dialogue.
        for ev in llm.poll() {
            if let Some(d) = &mut self.dialogue {
                if let Some(chat) = &mut d.chat {
                    match ev {
                        LlmEvent::Token { id, text } if Some(id) == chat.pending_req => {
                            d.text.push_str(&text);
                        }
                        LlmEvent::Done { id } if Some(id) == chat.pending_req => {
                            chat.pending_req = None;
                            d.streaming = false;
                            if d.text.trim().is_empty() {
                                // The reply was all markup/reasoning and got
                                // filtered away; fall back to a scripted line.
                                d.text = chat
                                    .persona
                                    .fallback_lines
                                    .first()
                                    .cloned()
                                    .unwrap_or_else(|| "…".into());
                            }
                            chat.history.push(ChatTurn { from_player: false, text: d.text.clone() });
                        }
                        LlmEvent::Error { id, msg } if Some(id) == chat.pending_req => {
                            chat.pending_req = None;
                            d.streaming = false;
                            if d.text.is_empty() {
                                d.text = format!("({msg})");
                            }
                        }
                        _ => {}
                    }
                }
            }
        }

        if self.battle.is_some() {
            let done = {
                let b = self.battle.as_mut().unwrap();
                b.update(project, dt)
            };
            if let Some(kind) = done {
                let battle = self.battle.take().unwrap();
                let outcome = battle.settle(&mut self.party, project, kind, &mut self.inventory);
                self.finish_battle(outcome);
            }
            return;
        }

        if let Some(d) = &mut self.dialogue {
            if let Some(t) = &mut d.auto_close {
                *t -= dt;
                if *t <= 0.0 {
                    self.dialogue = None;
                }
            }
            // Dialogue UI handles its own input; Esc closes (checked there).
            return;
        }

        // Movement input.
        let (l, r, u, dn, interact) = ctx.input(|i| {
            (
                i.key_down(egui::Key::ArrowLeft) || i.key_down(egui::Key::A),
                i.key_down(egui::Key::ArrowRight) || i.key_down(egui::Key::D),
                i.key_down(egui::Key::ArrowUp) || i.key_down(egui::Key::W),
                i.key_down(egui::Key::ArrowDown) || i.key_down(egui::Key::S),
                i.key_pressed(egui::Key::Z)
                    || i.key_pressed(egui::Key::Enter)
                    || i.key_pressed(egui::Key::Space),
            )
        });

        // Advance current step.
        if self.player.moving {
            self.player.move_t += dt * 4.4;
            self.player.anim += dt;
            if self.player.move_t >= 1.0 {
                self.player.moving = false;
                self.player.move_t = 1.0;
                self.on_step_complete(project);
            }
        }
        if !self.player.moving && self.dialogue.is_none() && self.battle.is_none() {
            let want = if l {
                Some(DIR_LEFT)
            } else if r {
                Some(DIR_RIGHT)
            } else if u {
                Some(DIR_UP)
            } else if dn {
                Some(DIR_DOWN)
            } else {
                None
            };
            if let Some(dir) = want {
                self.player.dir = dir;
                let (dx, dy) = dir_delta(dir);
                let (nx, ny) = (self.player.x + dx, self.player.y + dy);
                // Touch triggers fire even when the tile blocks walking.
                if self.walkable(nx, ny, self.player.x, self.player.y) {
                    self.player.from = (self.player.x, self.player.y);
                    self.player.x = nx;
                    self.player.y = ny;
                    self.player.move_t = 0.0;
                    self.player.moving = true;
                } else {
                    self.player.anim = 0.35; // idle-facing frame
                }
            } else {
                self.player.anim = 0.35;
            }
            if interact {
                self.interact(project, llm);
            }
        }

        // NPC wandering.
        self.update_npcs(dt);

        // Camera follows.
        let target = self.player_world_pos() + Vec3::Y * 0.6;
        self.camera.target = self.camera.target.lerp(target, (dt * 6.0).min(1.0));
        // Q/E rotate.
        let (q, e) = ctx.input(|i| (i.key_down(egui::Key::Q), i.key_down(egui::Key::E)));
        if q {
            self.camera.yaw += dt * 1.6;
        }
        if e {
            self.camera.yaw -= dt * 1.6;
        }
    }

    fn update_npcs(&mut self, dt: f32) {
        let mut rng = rand::rng();
        let player = (self.player.x, self.player.y);
        for i in 0..self.npcs.len() {
            let npc = &mut self.npcs[i];
            if npc.move_t < 1.0 {
                npc.move_t = (npc.move_t + dt * 3.0).min(1.0);
                continue;
            }
            if !npc.wander {
                continue;
            }
            npc.wander_timer -= dt;
            if npc.wander_timer > 0.0 {
                continue;
            }
            let npc_pos = (npc.x, npc.y, npc.home, npc.event_id);
            npc.wander_timer = rng.random_range(1.2..3.5);
            let dir = rng.random_range(0..4u32);
            let (dx, dy) = dir_delta(dir);
            let (nx, ny) = (npc_pos.0 + dx, npc_pos.1 + dy);
            let near_home = (nx - npc_pos.2.0).abs() <= 3 && (ny - npc_pos.2.1).abs() <= 3;
            let not_player = (nx, ny) != player;
            let no_event = self
                .map
                .event_at(nx, ny)
                .map(|e| e.id == npc_pos.3)
                .unwrap_or(true);
            if near_home && not_player && no_event && self.walkable_for_npc(nx, ny, npc_pos.0, npc_pos.1, i) {
                let npc = &mut self.npcs[i];
                npc.from = (npc.x, npc.y);
                npc.x = nx;
                npc.y = ny;
                npc.dir = dir;
                npc.move_t = 0.0;
            } else {
                self.npcs[i].dir = dir;
            }
        }
    }

    fn walkable_for_npc(&self, x: i32, y: i32, fx: i32, fy: i32, npc_idx: usize) -> bool {
        if !self.map.in_bounds(x, y) {
            return false;
        }
        let t = self.map.tile(x, y);
        if !Terrain::from_u8(t.terrain).walkable() || Prop::from_u8(t.prop).blocks() {
            return false;
        }
        if (t.height as i32 - self.map.tile(fx, fy).height as i32).abs() > 1 {
            return false;
        }
        for (j, npc) in self.npcs.iter().enumerate() {
            if j != npc_idx && npc.x == x && npc.y == y {
                return false;
            }
        }
        true
    }

    fn on_step_complete(&mut self, project: &ProjectData) {
        self.steps += 1;
        audio::sfx(Sfx::Step);
        // Touch events on the tile we arrived at.
        let ev = self.map.event_at(self.player.x, self.player.y).cloned();
        if let Some(ev) = ev {
            match &ev.kind {
                EventKind::Transfer { target_map, target_x, target_y } => {
                    self.transfer(project, *target_map, *target_x, *target_y);
                    return;
                }
                EventKind::BattleTrigger { troop_id, once } => {
                    if !self.done_events.contains(&(self.map_id, ev.id)) {
                        if *once {
                            self.done_events.insert((self.map_id, ev.id));
                        }
                        self.start_battle(project, *troop_id);
                        return;
                    }
                }
                _ => {}
            }
        }
        // Random encounters.
        if self.map.encounter_steps > 0 && !self.map.encounter_troops.is_empty() {
            let mut rng = rand::rng();
            if rng.random_range(0..self.map.encounter_steps.max(1)) == 0 {
                let troop = self.map.encounter_troops
                    [rng.random_range(0..self.map.encounter_troops.len())];
                self.start_battle(project, troop);
            }
        }
    }

    fn interact(&mut self, project: &ProjectData, llm: &mut LlmEngine) {
        let (dx, dy) = dir_delta(self.player.dir);
        let (tx, ty) = (self.player.x + dx, self.player.y + dy);
        // Wandering NPCs move off their static event tile, so resolve the event
        // by the NPC's live runtime position first; otherwise fall back to the
        // static tile for stationary events (signs, chests, heal points).
        let ev = self
            .npcs
            .iter()
            .find(|n| n.x == tx && n.y == ty)
            .and_then(|n| self.map.events.iter().find(|e| e.id == n.event_id))
            .or_else(|| self.map.event_at(tx, ty));
        let Some(ev) = ev.cloned() else { return };
        match &ev.kind {
            EventKind::Npc { persona, .. } => {
                // Face the player.
                if let Some(npc) = self.npcs.iter_mut().find(|n| n.event_id == ev.id) {
                    npc.dir = match self.player.dir {
                        DIR_UP => DIR_DOWN,
                        DIR_DOWN => DIR_UP,
                        DIR_LEFT => DIR_RIGHT,
                        _ => DIR_LEFT,
                    };
                }
                audio::sfx(Sfx::Confirm);
                self.open_npc_dialogue(project, llm, ev.id, persona.clone());
            }
            EventKind::Sign { text } => {
                audio::sfx(Sfx::Confirm);
                self.dialogue = Some(Dialogue {
                    speaker: ev.name.clone(),
                    text: text.clone(),
                    chat: None,
                    streaming: false,
                    auto_close: None,
                });
            }
            EventKind::Chest { item_id } => {
                if !self.done_events.contains(&(self.map_id, ev.id)) {
                    self.done_events.insert((self.map_id, ev.id));
                    audio::sfx(Sfx::Chest);
                    let name = project.item(*item_id).map(|i| i.name.clone()).unwrap_or("???".into());
                    if let Some(slot) = self.inventory.iter_mut().find(|(id, _)| id == item_id) {
                        slot.1 += 1;
                    } else {
                        self.inventory.push((*item_id, 1));
                    }
                    self.dialogue = Some(Dialogue {
                        speaker: "".into(),
                        text: format!("Found {name}!"),
                        chat: None,
                        streaming: false,
                        auto_close: None,
                    });
                }
            }
            EventKind::HealPoint => {
                audio::sfx(Sfx::Heal);
                for m in &mut self.party {
                    m.hp = m.max.hp;
                    m.mp = m.max.mp;
                }
                self.dialogue = Some(Dialogue {
                    speaker: "".into(),
                    text: "The party feels refreshed!".into(),
                    chat: None,
                    streaming: false,
                    auto_close: None,
                });
            }
            EventKind::BattleTrigger { troop_id, once } => {
                if !self.done_events.contains(&(self.map_id, ev.id)) {
                    if *once {
                        self.done_events.insert((self.map_id, ev.id));
                    }
                    self.start_battle(project, *troop_id);
                }
            }
            EventKind::Transfer { .. } => {}
        }
    }

    fn open_npc_dialogue(
        &mut self,
        project: &ProjectData,
        llm: &mut LlmEngine,
        event_id: u32,
        persona: NpcPersona,
    ) {
        let speaker = persona.name.clone();
        if persona.use_llm && llm.ready() {
            let mut chat = NpcChat {
                event_id,
                persona: persona.clone(),
                history: Vec::new(),
                pending_req: None,
                input: String::new(),
            };
            let req = self.make_chat_request(project, &chat);
            chat.pending_req = llm.request(req);
            let streaming = chat.pending_req.is_some();
            self.dialogue = Some(Dialogue {
                speaker: speaker.clone(),
                text: String::new(),
                chat: Some(chat),
                streaming,
                auto_close: None,
            });
            if streaming {
                return;
            }
        }
        // Fallback lines.
        let idx = self.fallback_cursor.entry(event_id).or_insert(0);
        let line = if persona.fallback_lines.is_empty() {
            "…".to_string()
        } else {
            persona.fallback_lines[*idx % persona.fallback_lines.len()].clone()
        };
        *idx += 1;
        self.dialogue = Some(Dialogue { speaker, text: line, chat: None, streaming: false, auto_close: None });
    }

    pub fn make_chat_request(&self, project: &ProjectData, chat: &NpcChat) -> ChatRequest {
        ChatRequest {
            id: 0,
            persona: chat.persona.clone(),
            game_title: project.system.title.clone(),
            location: self.map.name.clone(),
            player_name: self.party.first().map(|m| m.name.clone()).unwrap_or("the hero".into()),
            history: chat.history.clone(),
            max_tokens: project.llm.max_reply_tokens,
            temperature: project.llm.temperature,
        }
    }

    fn start_battle(&mut self, project: &ProjectData, troop_id: u32) {
        if let Some(b) = battle::Battle::new(project, self, troop_id) {
            audio::sfx(Sfx::Encounter);
            audio::music(Track::Battle);
            self.battle = Some(b);
        }
    }

    fn finish_battle(&mut self, outcome: battle::Outcome) {
        // Return the field music once combat ends (defeat keeps it silent).
        match outcome {
            battle::Outcome::Victory { exp, messages } => {
                audio::sfx(Sfx::Victory);
                if !messages.is_empty() {
                    audio::sfx(Sfx::LevelUp);
                }
                audio::music(Track::for_map(&self.map));
                self.dialogue = Some(Dialogue {
                    speaker: "".into(),
                    text: format!("Victory! Gained {exp} EXP.{}", if messages.is_empty() {
                        String::new()
                    } else {
                        format!("\n{}", messages.join("\n"))
                    }),
                    chat: None,
                    streaming: false,
                    auto_close: Some(3.0),
                });
            }
            battle::Outcome::Defeat => {
                audio::sfx(Sfx::Defeat);
                audio::music(Track::Silence);
                self.game_over = true;
            }
            battle::Outcome::Fled => {
                audio::sfx(Sfx::Flee);
                audio::music(Track::for_map(&self.map));
            }
        }
    }
}
