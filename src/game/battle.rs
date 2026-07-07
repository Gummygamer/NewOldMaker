//! Octopath-style turn-based battles: speed order, Boost Points, elemental
//! weaknesses, shield points and Break.

use eframe::egui;
use glam::Vec3;
use rand::RngExt;

use crate::audio::{self, Sfx};
use crate::core::data::*;
use crate::game::{exp_to_next, Member, DIR_LEFT};
use crate::gfx::camera::OrbitCamera;
use crate::gfx::mesh::tile_top_y;

pub const MAX_BP: i32 = 5;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum OutcomeKind {
    Victory,
    Defeat,
    Fled,
}

pub enum Outcome {
    Victory { exp: u32, messages: Vec<String> },
    Defeat,
    Fled,
}

pub struct Fighter {
    pub name: String,
    pub is_player: bool,
    pub sprite: u8,
    pub member_idx: Option<usize>,
    pub max: Stats,
    pub hp: i32,
    pub mp: i32,
    pub attack_element: Element,
    pub skills: Vec<u32>,
    pub weaknesses: Vec<Element>,
    pub bp: i32,
    pub shields_max: u8,
    pub shields: u8,
    /// Turns of Break remaining (enemy skips its turn, takes +50% damage).
    pub broken: u8,
    pub buff_atk: u8,
    pub debuff_def: u8,
    pub defending: bool,
    pub exp: u32,
    pub pos: Vec3,
    pub flash: f32,
}

impl Fighter {
    pub fn alive(&self) -> bool {
        self.hp > 0
    }
    fn atk_stat(&self, physical: bool) -> f32 {
        let base = if physical { self.max.atk } else { self.max.mag } as f32;
        if self.buff_atk > 0 {
            base * 1.3
        } else {
            base
        }
    }
    fn def_stat(&self, physical: bool) -> f32 {
        let base = if physical { self.max.def } else { self.max.spr } as f32;
        let mut v = base;
        if self.debuff_def > 0 {
            v *= 0.7;
        }
        if self.broken > 0 {
            v *= 0.75;
        }
        v
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum ActionKind {
    Attack,
    Skill(u32),
    Item(u32),
    Defend,
    Flee,
}

#[derive(Clone, Copy)]
pub enum Menu {
    Root,
    Skills,
    Items,
    Targets { action: ActionKind, allies: bool },
}

pub enum Phase {
    Intro(f32),
    Choose {
        fighter: usize,
        menu: Menu,
        boost: i32,
    },
    Anim {
        timer: f32,
    },
    Finished(OutcomeKind, f32),
}

pub struct Popup {
    pub pos: Vec3,
    pub text: String,
    pub color: [f32; 3],
    pub age: f32,
}

pub struct Battle {
    pub fighters: Vec<Fighter>,
    pub order: Vec<usize>,
    pub order_pos: usize,
    pub round: u32,
    pub phase: Phase,
    pub log: Vec<String>,
    pub popups: Vec<Popup>,
    pub camera: OrbitCamera,
    pub inventory: Vec<(u32, u32)>,
    troop_name: String,
    avg_enemy_spd: f32,
}

impl Battle {
    pub fn new(project: &ProjectData, game: &crate::game::Game, troop_id: u32) -> Option<Battle> {
        let troop = project.troop(troop_id)?;
        let map = &game.map;
        let center = game.player_world_pos();
        // Keep the arena roughly on the map.
        let cx = center.x.clamp(3.0, map.width as f32 - 3.0);
        let cz = center.z.clamp(2.0, map.height as f32 - 2.0);
        let ground = |x: f32, z: f32| {
            tile_top_y(
                map,
                (x.floor() as i32).clamp(0, map.width as i32 - 1),
                (z.floor() as i32).clamp(0, map.height as i32 - 1),
            )
        };

        let mut fighters = Vec::new();
        let n_e = troop.members.len().max(1) as f32;
        for (i, enemy_id) in troop.members.iter().enumerate() {
            let Some(e) = project.enemy(*enemy_id) else {
                continue;
            };
            let z = cz + (i as f32 - (n_e - 1.0) / 2.0) * 1.6;
            let x = cx - 2.4 - (i as f32 % 2.0) * 0.7;
            fighters.push(Fighter {
                name: e.name.clone(),
                is_player: false,
                sprite: e.sprite,
                member_idx: None,
                max: e.stats,
                hp: e.stats.hp,
                mp: e.stats.mp,
                attack_element: Element::Blunt,
                skills: e.skills.clone(),
                weaknesses: e.weaknesses.clone(),
                bp: 0,
                shields_max: e.shields,
                shields: e.shields,
                broken: 0,
                buff_atk: 0,
                debuff_def: 0,
                defending: false,
                exp: e.exp,
                pos: Vec3::new(x, ground(x, z), z),
                flash: 0.0,
            });
        }
        if fighters.is_empty() {
            return None;
        }
        let n_p = game.party.len().max(1) as f32;
        for (i, m) in game.party.iter().enumerate() {
            let z = cz + (i as f32 - (n_p - 1.0) / 2.0) * 1.3;
            let x = cx + 2.4 + (i as f32 % 2.0) * 0.6;
            fighters.push(Fighter {
                name: m.name.clone(),
                is_player: true,
                sprite: m.sprite,
                member_idx: Some(i),
                max: m.max,
                hp: m.hp,
                mp: m.mp,
                attack_element: m.attack_element,
                skills: m.skills.clone(),
                weaknesses: Vec::new(),
                bp: 1,
                shields_max: 0,
                shields: 0,
                broken: 0,
                buff_atk: 0,
                debuff_def: 0,
                defending: false,
                exp: 0,
                pos: Vec3::new(x, ground(x, z), z),
                flash: 0.0,
            });
        }

        let avg_enemy_spd = fighters
            .iter()
            .filter(|f| !f.is_player)
            .map(|f| f.max.spd as f32)
            .sum::<f32>()
            / n_e;

        let mut camera = OrbitCamera::default();
        camera.target = Vec3::new(cx, ground(cx, cz) + 0.9, cz);
        camera.yaw = std::f32::consts::FRAC_PI_2 - 0.38;
        camera.pitch = 0.34;
        camera.dist = 8.6;

        let mut b = Battle {
            fighters,
            order: Vec::new(),
            order_pos: 0,
            round: 1,
            phase: Phase::Intro(0.8),
            log: vec![project.system.language.attacks(&troop.name)],
            popups: Vec::new(),
            camera,
            inventory: game.inventory.clone(),
            troop_name: troop.name.clone(),
            avg_enemy_spd,
        };
        b.compute_order();
        Some(b)
    }

    fn compute_order(&mut self) {
        let mut idx: Vec<usize> = (0..self.fighters.len())
            .filter(|i| self.fighters[*i].alive())
            .collect();
        idx.sort_by_key(|i| -(self.fighters[*i].max.spd));
        self.order = idx;
        self.order_pos = 0;
    }

    fn outcome_check(&self) -> Option<OutcomeKind> {
        if !self.fighters.iter().any(|f| !f.is_player && f.alive()) {
            Some(OutcomeKind::Victory)
        } else if !self.fighters.iter().any(|f| f.is_player && f.alive()) {
            Some(OutcomeKind::Defeat)
        } else {
            None
        }
    }

    /// Advance to the next turn (or next round). Sets `phase`.
    fn next_turn(&mut self, project: &ProjectData) {
        if let Some(kind) = self.outcome_check() {
            self.phase = Phase::Finished(kind, 1.2);
            return;
        }
        loop {
            if self.order_pos >= self.order.len() {
                self.end_round();
            }
            let fi = self.order[self.order_pos];
            self.order_pos += 1;
            if !self.fighters[fi].alive() {
                continue;
            }
            if self.fighters[fi].is_player {
                self.fighters[fi].defending = false;
                self.phase = Phase::Choose {
                    fighter: fi,
                    menu: Menu::Root,
                    boost: 0,
                };
                return;
            }
            // Enemy turn.
            if self.fighters[fi].broken > 0 {
                self.log_line(project.system.language.is_broken(&self.fighters[fi].name));
                self.phase = Phase::Anim { timer: 0.45 };
                return;
            }
            self.enemy_act(project, fi);
            self.phase = Phase::Anim { timer: 0.7 };
            return;
        }
    }

    fn end_round(&mut self) {
        self.round += 1;
        for f in &mut self.fighters {
            if f.is_player {
                if f.alive() {
                    f.bp = (f.bp + 1).min(MAX_BP);
                }
            } else if f.broken > 0 {
                f.broken -= 1;
                if f.broken == 0 {
                    f.shields = f.shields_max;
                }
            }
            if f.buff_atk > 0 {
                f.buff_atk -= 1;
            }
            if f.debuff_def > 0 {
                f.debuff_def -= 1;
            }
        }
        self.compute_order();
    }

    fn log_line(&mut self, s: String) {
        self.log.push(s);
        if self.log.len() > 4 {
            self.log.remove(0);
        }
    }

    fn popup(&mut self, at: Vec3, text: String, color: [f32; 3]) {
        self.popups.push(Popup {
            pos: at + Vec3::Y * 1.4,
            text,
            color,
            age: 0.0,
        });
    }

    // -----------------------------------------------------------------
    // Executing actions
    // -----------------------------------------------------------------

    fn deal_damage(
        &mut self,
        project: &ProjectData,
        src: usize,
        dst: usize,
        element: Element,
        power: f32,
        hits: u32,
        boost: i32,
    ) {
        let lang = project.system.language;
        let mut rng = rand::rng();
        let physical = element.physical();
        let mut hit_any = false;
        let mut weak_any = false;
        let mut broke = false;
        for _ in 0..hits {
            if !self.fighters[dst].alive() {
                break;
            }
            hit_any = true;
            let a = self.fighters[src].atk_stat(physical);
            let d = self.fighters[dst].def_stat(physical);
            let mut dmg = (a * 2.2 * power - d * 1.15).max(1.0);
            dmg *= 1.0 + 0.55 * boost as f32;
            let weak = self.fighters[dst].weaknesses.contains(&element);
            if weak {
                dmg *= 1.3;
                weak_any = true;
            }
            if self.fighters[dst].broken > 0 {
                dmg *= 1.5;
            }
            if self.fighters[dst].defending {
                dmg *= 0.5;
            }
            dmg *= rng.random_range(0.9..1.1);
            let dmg = dmg.round() as i32;
            self.fighters[dst].hp = (self.fighters[dst].hp - dmg).max(0);
            self.fighters[dst].flash = 0.25;
            let color = if weak {
                [1.0, 0.85, 0.2]
            } else {
                [1.0, 1.0, 1.0]
            };
            let pos = self.fighters[dst].pos;
            self.popup(pos, format!("{dmg}"), color);

            // Shields chip on weakness hits.
            if weak
                && !self.fighters[dst].is_player
                && self.fighters[dst].broken == 0
                && self.fighters[dst].shields_max > 0
            {
                let s = self.fighters[dst].shields.saturating_sub(1);
                self.fighters[dst].shields = s;
                if s == 0 {
                    self.fighters[dst].broken = 2;
                    broke = true;
                    let pos = self.fighters[dst].pos;
                    self.popup(
                        pos + Vec3::Y * 0.5,
                        lang.break_popup().into(),
                        [1.0, 0.4, 0.2],
                    );
                    let name = self.fighters[dst].name.clone();
                    self.log_line(lang.guard_broken(&name));
                }
            }
            if !self.fighters[dst].alive() {
                let name = self.fighters[dst].name.clone();
                self.log_line(lang.is_defeated(&name));
            }
        }
        // One sound per action: Break trumps a weakness hit, which trumps a
        // plain hit.
        if broke {
            audio::sfx(Sfx::Break);
        } else if weak_any {
            audio::sfx(Sfx::Weakness);
        } else if hit_any {
            audio::sfx(Sfx::Hit);
        }
    }

    fn heal(&mut self, dst: usize, amount: i32) {
        let f = &mut self.fighters[dst];
        let was_dead = !f.alive();
        if was_dead {
            return; // healing doesn't revive
        }
        f.hp = (f.hp + amount).min(f.max.hp);
        let pos = f.pos;
        self.popup(pos, format!("+{amount}"), [0.4, 1.0, 0.5]);
        audio::sfx(Sfx::Heal);
    }

    pub fn execute_player_action(
        &mut self,
        project: &ProjectData,
        fighter: usize,
        action: ActionKind,
        target: Option<usize>,
        boost: i32,
    ) {
        audio::sfx(Sfx::Confirm);
        let lang = project.system.language;
        let boost = boost.min(self.fighters[fighter].bp).max(0);
        self.fighters[fighter].bp -= boost;
        let name = self.fighters[fighter].name.clone();
        match action {
            ActionKind::Attack => {
                if let Some(t) = target {
                    let el = self.fighters[fighter].attack_element;
                    let hits = 1 + boost as u32;
                    self.log_line(lang.attacks(&name));
                    self.deal_damage(project, fighter, t, el, 1.0, hits, 0);
                }
            }
            ActionKind::Skill(id) => {
                if let Some(skill) = project.skill(id).cloned() {
                    self.fighters[fighter].mp -= skill.mp_cost as i32;
                    self.log_line(lang.uses(&name, &skill.name));
                    self.apply_skill(project, fighter, &skill, target, boost);
                }
            }
            ActionKind::Item(id) => {
                if let Some(item) = project.item(id).cloned() {
                    if let Some(slot) = self.inventory.iter_mut().find(|(i, _)| *i == id) {
                        if slot.1 > 0 {
                            slot.1 -= 1;
                            self.log_line(lang.uses(&name, &item.name));
                            if let Some(t) = target {
                                match item.kind {
                                    ItemKind::HealHp => self.heal(t, item.power),
                                    ItemKind::HealMp => {
                                        let f = &mut self.fighters[t];
                                        f.mp = (f.mp + item.power).min(f.max.mp);
                                        let pos = f.pos;
                                        self.popup(pos, lang.mp_gain(item.power), [0.4, 0.7, 1.0]);
                                    }
                                    ItemKind::Revive => {
                                        let f = &mut self.fighters[t];
                                        if !f.alive() {
                                            f.hp = item.power.min(f.max.hp);
                                            let pos = f.pos;
                                            self.popup(pos, lang.revived().into(), [1.0, 0.9, 0.4]);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            ActionKind::Defend => {
                self.fighters[fighter].defending = true;
                // Defending banks the boost back.
                self.fighters[fighter].bp = (self.fighters[fighter].bp + boost + 0).min(MAX_BP);
                self.log_line(lang.guards(&name));
            }
            ActionKind::Flee => {
                let party_spd: f32 = self
                    .fighters
                    .iter()
                    .filter(|f| f.is_player && f.alive())
                    .map(|f| f.max.spd as f32)
                    .sum::<f32>()
                    / self
                        .fighters
                        .iter()
                        .filter(|f| f.is_player && f.alive())
                        .count()
                        .max(1) as f32;
                let chance = (0.5 + (party_spd - self.avg_enemy_spd) * 0.02).clamp(0.15, 0.95);
                if rand::rng().random_range(0.0..1.0) < chance {
                    self.log_line(lang.got_away().into());
                    self.phase = Phase::Finished(OutcomeKind::Fled, 0.7);
                    return;
                }
                self.log_line(lang.couldnt_escape().into());
            }
        }
        self.phase = Phase::Anim { timer: 0.7 };
    }

    fn apply_skill(
        &mut self,
        project: &ProjectData,
        src: usize,
        skill: &Skill,
        target: Option<usize>,
        boost: i32,
    ) {
        let src_is_player = self.fighters[src].is_player;
        let enemies: Vec<usize> = (0..self.fighters.len())
            .filter(|i| self.fighters[*i].is_player != src_is_player && self.fighters[*i].alive())
            .collect();
        let allies: Vec<usize> = (0..self.fighters.len())
            .filter(|i| self.fighters[*i].is_player == src_is_player && self.fighters[*i].alive())
            .collect();
        let targets: Vec<usize> = match skill.target {
            SkillTarget::OneEnemy => target.into_iter().collect(),
            SkillTarget::AllEnemies => enemies,
            SkillTarget::OneAlly => target.into_iter().collect(),
            SkillTarget::AllAllies => allies,
            SkillTarget::Own => vec![src],
        };
        for t in targets {
            match skill.effect {
                SkillEffect::Damage => {
                    self.deal_damage(
                        project,
                        src,
                        t,
                        skill.element,
                        skill.power,
                        skill.hits as u32,
                        boost,
                    );
                }
                SkillEffect::Heal => {
                    let m = self.fighters[src].atk_stat(false);
                    let amount =
                        (m * 1.8 * skill.power * (1.0 + 0.5 * boost as f32)).round() as i32;
                    self.heal(t, amount);
                }
                SkillEffect::BuffAttack(turns) => {
                    self.fighters[t].buff_atk = turns + boost as u8;
                    let pos = self.fighters[t].pos;
                    self.popup(pos, "ATK ↑".into(), [1.0, 0.7, 0.3]);
                }
                SkillEffect::BreakDefense(turns) => {
                    if skill.power > 0.0 {
                        self.deal_damage(
                            project,
                            src,
                            t,
                            skill.element,
                            skill.power,
                            skill.hits as u32,
                            boost,
                        );
                    }
                    self.fighters[t].debuff_def = turns + boost as u8;
                    let pos = self.fighters[t].pos;
                    self.popup(pos, "DEF ↓".into(), [0.8, 0.5, 1.0]);
                }
            }
        }
    }

    fn enemy_act(&mut self, project: &ProjectData, fi: usize) {
        let lang = project.system.language;
        let mut rng = rand::rng();
        let players: Vec<usize> = (0..self.fighters.len())
            .filter(|i| self.fighters[*i].is_player && self.fighters[*i].alive())
            .collect();
        if players.is_empty() {
            return;
        }
        let target = players[rng.random_range(0..players.len())];
        let name = self.fighters[fi].name.clone();
        let usable: Vec<u32> = self.fighters[fi]
            .skills
            .iter()
            .copied()
            .filter(|id| {
                project
                    .skill(*id)
                    .map(|s| s.mp_cost as i32 <= self.fighters[fi].mp)
                    .unwrap_or(false)
            })
            .collect();
        if !usable.is_empty() && rng.random_range(0..100) < 55 {
            let id = usable[rng.random_range(0..usable.len())];
            if let Some(skill) = project.skill(id).cloned() {
                self.fighters[fi].mp -= skill.mp_cost as i32;
                self.log_line(lang.uses(&name, &skill.name));
                self.apply_skill(project, fi, &skill, Some(target), 0);
                return;
            }
        }
        self.log_line(lang.attacks(&name));
        let el = self.fighters[fi].attack_element;
        self.deal_damage(project, fi, target, el, 1.0, 1, 0);
    }

    // -----------------------------------------------------------------
    // Frame update
    // -----------------------------------------------------------------

    pub fn update(&mut self, project: &ProjectData, dt: f32) -> Option<OutcomeKind> {
        for f in &mut self.fighters {
            f.flash = (f.flash - dt).max(0.0);
        }
        for p in &mut self.popups {
            p.age += dt;
            p.pos.y += dt * 0.8;
        }
        self.popups.retain(|p| p.age < 1.2);

        match &mut self.phase {
            Phase::Intro(t) => {
                *t -= dt;
                if *t <= 0.0 {
                    self.next_turn(project);
                }
                None
            }
            Phase::Choose { .. } => None, // waiting on UI
            Phase::Anim { timer } => {
                *timer -= dt;
                if *timer <= 0.0 {
                    self.next_turn(project);
                }
                None
            }
            Phase::Finished(kind, t) => {
                *t -= dt;
                if *t <= 0.0 && self.popups.is_empty() {
                    Some(*kind)
                } else {
                    None
                }
            }
        }
    }

    /// Write battle results back into the party; returns the final outcome.
    pub fn settle(
        &self,
        party: &mut [Member],
        project: &ProjectData,
        kind: OutcomeKind,
        inventory: &mut Vec<(u32, u32)>,
    ) -> Outcome {
        *inventory = self.inventory.clone();
        for f in &self.fighters {
            if let Some(i) = f.member_idx {
                if let Some(m) = party.get_mut(i) {
                    m.hp = f.hp.max(if kind == OutcomeKind::Defeat { 0 } else { 1 });
                    m.mp = f.mp.max(0);
                }
            }
        }
        match kind {
            OutcomeKind::Victory => {
                let exp: u32 = self
                    .fighters
                    .iter()
                    .filter(|f| !f.is_player)
                    .map(|f| f.exp)
                    .sum();
                let mut messages = Vec::new();
                for m in party.iter_mut() {
                    if m.hp <= 0 {
                        continue;
                    }
                    m.exp += exp;
                    while m.exp >= exp_to_next(m.level) {
                        m.exp -= exp_to_next(m.level);
                        m.level += 1;
                        if let Some(actor) = project.actor(m.actor_id) {
                            let fresh = crate::game::member_from_actor(actor, m.level);
                            let hp_gain = fresh.max.hp - m.max.hp;
                            m.max = fresh.max;
                            m.skills = fresh.skills;
                            m.hp = (m.hp + hp_gain.max(0)).min(m.max.hp);
                            m.mp = m.mp.min(m.max.mp);
                        }
                        messages.push(project.system.language.reached_level(&m.name, m.level));
                    }
                }
                Outcome::Victory { exp, messages }
            }
            OutcomeKind::Defeat => Outcome::Defeat,
            OutcomeKind::Fled => Outcome::Fled,
        }
    }

    // -----------------------------------------------------------------
    // egui battle UI (menus, party status, enemy list)
    // -----------------------------------------------------------------

    pub fn ui(&mut self, ctx: &egui::Context, project: &ProjectData) {
        let lang = project.system.language;
        // Enemy roster (top left).
        egui::Window::new("enemies")
            .title_bar(false)
            .resizable(false)
            .anchor(egui::Align2::LEFT_TOP, [12.0, 12.0])
            .show(ctx, |ui| {
                ui.label(egui::RichText::new(&self.troop_name).strong());
                for f in self.fighters.iter().filter(|f| !f.is_player) {
                    if !f.alive() {
                        ui.weak(format!("✝ {}", f.name));
                        continue;
                    }
                    let shields = if f.broken > 0 {
                        lang.break_popup().to_string()
                    } else if f.shields_max > 0 {
                        format!("🛡{}", f.shields)
                    } else {
                        String::new()
                    };
                    ui.horizontal(|ui| {
                        ui.label(&f.name);
                        if f.broken > 0 {
                            ui.colored_label(egui::Color32::ORANGE, shields);
                        } else {
                            ui.label(shields);
                        }
                    });
                    let frac = f.hp as f32 / f.max.hp.max(1) as f32;
                    ui.add(
                        egui::ProgressBar::new(frac)
                            .desired_width(140.0)
                            .desired_height(6.0),
                    );
                }
                ui.small(lang.round(self.round));
                for line in &self.log {
                    ui.small(line);
                }
            });

        // Party status + action menu (bottom).
        let mut chosen: Option<(usize, ActionKind, Option<usize>, i32)> = None;
        egui::Window::new("battle-panel")
            .title_bar(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_BOTTOM, [0.0, -12.0])
            .show(ctx, |ui| {
                ui.set_width(720.0);
                ui.set_min_height(150.0);
                ui.columns(2, |cols| {
                    // Party status.
                    cols[0].vertical(|ui| {
                        let active = match self.phase {
                            Phase::Choose { fighter, .. } => Some(fighter),
                            _ => None,
                        };
                        for (i, f) in self.fighters.iter().enumerate().filter(|(_, f)| f.is_player) {
                            ui.horizontal(|ui| {
                                let name = if Some(i) == active {
                                    egui::RichText::new(format!("▶ {}", f.name)).strong()
                                } else if !f.alive() {
                                    egui::RichText::new(format!("✝ {}", f.name)).weak()
                                } else {
                                    egui::RichText::new(&f.name)
                                };
                                ui.label(name);
                                ui.label(format!("HP {}/{}", f.hp.max(0), f.max.hp));
                                ui.label(format!("MP {}", f.mp.max(0)));
                                let bp: String = (0..MAX_BP).map(|b| if b < f.bp { '●' } else { '○' }).collect();
                                ui.colored_label(egui::Color32::from_rgb(255, 200, 80), bp);
                            });
                        }
                    });
                    // Action menu.
                    cols[1].vertical(|ui| {
                        if let Phase::Choose { fighter, menu, boost } = &mut self.phase {
                            let fi = *fighter;
                            let f = &self.fighters[fi];
                            ui.horizontal(|ui| {
                                ui.label(egui::RichText::new(lang.choose_action(&f.name)).strong());
                                ui.label(egui::RichText::new(format!("{}: {boost}", lang.boost())).color(egui::Color32::from_rgb(255, 200, 80)));
                            });
                            ui.horizontal(|ui| {
                                for b in 0..=3.min(f.bp) {
                                    if ui.selectable_label(*boost == b, format!("+{b}")).clicked() {
                                        *boost = b;
                                    }
                                }
                            });
                            match menu {
                                Menu::Root => {
                                    ui.horizontal_wrapped(|ui| {
                                        if ui.button(lang.attack()).clicked() {
                                            *menu = Menu::Targets { action: ActionKind::Attack, allies: false };
                                        }
                                        if ui.button(lang.skills()).clicked() {
                                            *menu = Menu::Skills;
                                        }
                                        if ui.button(lang.items()).clicked() {
                                            *menu = Menu::Items;
                                        }
                                        if ui.button(lang.defend()).clicked() {
                                            chosen = Some((fi, ActionKind::Defend, None, *boost));
                                        }
                                        if ui.button(lang.flee()).clicked() {
                                            chosen = Some((fi, ActionKind::Flee, None, 0));
                                        }
                                    });
                                }
                                Menu::Skills => {
                                    let skills: Vec<Skill> = f.skills.iter().filter_map(|id| project.skill(*id).cloned()).collect();
                                    egui::ScrollArea::vertical().max_height(80.0).show(ui, |ui| {
                                        for s in &skills {
                                            let can = s.mp_cost as i32 <= f.mp;
                                            let label = format!("{} {} ({} MP) — {}", s.element.icon(), s.name, s.mp_cost, s.description);
                                            if ui.add_enabled(can, egui::Button::new(label)).clicked() {
                                                match s.target {
                                                    SkillTarget::OneEnemy => {
                                                        *menu = Menu::Targets { action: ActionKind::Skill(s.id), allies: false };
                                                    }
                                                    SkillTarget::OneAlly => {
                                                        *menu = Menu::Targets { action: ActionKind::Skill(s.id), allies: true };
                                                    }
                                                    _ => {
                                                        chosen = Some((fi, ActionKind::Skill(s.id), None, *boost));
                                                    }
                                                }
                                            }
                                        }
                                    });
                                    if ui.small_button(lang.back()).clicked() {
                                        *menu = Menu::Root;
                                    }
                                }
                                Menu::Items => {
                                    let items: Vec<(u32, u32, String)> = self
                                        .inventory
                                        .iter()
                                        .filter(|(_, n)| *n > 0)
                                        .filter_map(|(id, n)| project.item(*id).map(|it| (*id, *n, it.name.clone())))
                                        .collect();
                                    for (id, n, name) in items {
                                        if ui.button(format!("{name} ×{n}")).clicked() {
                                            *menu = Menu::Targets { action: ActionKind::Item(id), allies: true };
                                        }
                                    }
                                    if ui.small_button(lang.back()).clicked() {
                                        *menu = Menu::Root;
                                    }
                                }
                                Menu::Targets { action, allies } => {
                                    ui.label(lang.target());
                                    let action = *action;
                                    let allies = *allies;
                                    let targets: Vec<(usize, String)> = self
                                        .fighters
                                        .iter()
                                        .enumerate()
                                        .filter(|(_, t)| t.is_player == allies)
                                        .filter(|(_, t)| {
                                            // Revive targets the fallen; everything else the living.
                                            if matches!(action, ActionKind::Item(id) if project.item(id).map(|i| i.kind == ItemKind::Revive).unwrap_or(false)) {
                                                !t.alive()
                                            } else {
                                                t.alive()
                                            }
                                        })
                                        .map(|(i, t)| (i, t.name.clone()))
                                        .collect();
                                    ui.horizontal_wrapped(|ui| {
                                        for (i, name) in targets {
                                            if ui.button(name).clicked() {
                                                chosen = Some((fi, action, Some(i), *boost));
                                            }
                                        }
                                    });
                                    if ui.small_button(lang.back()).clicked() {
                                        *menu = Menu::Root;
                                    }
                                }
                            }
                        } else if let Phase::Finished(kind, _) = &self.phase {
                            let msg = match kind {
                                OutcomeKind::Victory => lang.victory(),
                                OutcomeKind::Defeat => lang.party_fallen(),
                                OutcomeKind::Fled => lang.escaped(),
                            };
                            ui.heading(msg);
                        } else {
                            ui.label("…");
                        }
                    });
                });
            });

        if let Some((fi, action, target, boost)) = chosen {
            self.execute_player_action(project, fi, action, target, boost);
        }
    }

    /// Sprite/tint info per fighter for the scene builder.
    pub fn fighter_visuals(&self, time: f32) -> Vec<(usize, &Fighter, [f32; 4], u32)> {
        let mut out = Vec::new();
        for (i, f) in self.fighters.iter().enumerate() {
            if !f.alive() && !f.is_player {
                continue;
            }
            let mut tint = [1.0, 1.0, 1.0, 1.0];
            if f.flash > 0.0 {
                let k = 1.0 + f.flash * 6.0;
                tint = [k, k * 0.6, k * 0.5, 1.0];
            } else if f.broken > 0 {
                tint = [0.6, 0.6, 0.75, 1.0];
            } else if !f.alive() {
                tint = [0.35, 0.3, 0.4, 1.0];
            }
            let frame = if f.is_player {
                ((time * 2.0) as u32 + i as u32) % 3
            } else {
                0
            };
            out.push((
                i,
                f,
                tint,
                if f.is_player {
                    DIR_LEFT * 3 + frame
                } else {
                    frame
                },
            ));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::defaults::default_project;
    use crate::game::Game;

    fn setup(troop: u32) -> (ProjectData, Battle) {
        let project = default_project(Language::default());
        let game = Game::new(&project);
        let battle = Battle::new(&project, &game, troop).expect("troop exists");
        (project, battle)
    }

    fn first_alive_enemy(b: &Battle) -> usize {
        (0..b.fighters.len())
            .find(|i| !b.fighters[*i].is_player && b.fighters[*i].alive())
            .unwrap()
    }

    #[test]
    fn weakness_hits_chip_shields_and_break() {
        let (project, mut b) = setup(1); // slimes: shields 2, weak to Slash/Fire
        let e = first_alive_enemy(&b);
        let p = (0..b.fighters.len())
            .find(|i| b.fighters[*i].is_player)
            .unwrap();
        assert_eq!(b.fighters[e].shields, 2);
        // Slash attack (weakness) chips one shield per hit.
        b.deal_damage(&project, p, e, Element::Slash, 0.01, 1, 0);
        assert_eq!(b.fighters[e].shields, 1);
        b.deal_damage(&project, p, e, Element::Fire, 0.01, 1, 0);
        assert_eq!(b.fighters[e].shields, 0);
        assert!(b.fighters[e].broken > 0, "shields at 0 must break");
        // Non-weakness element must not chip shields on a fresh enemy.
        let (project2, mut b2) = setup(1);
        let e2 = first_alive_enemy(&b2);
        let p2 = (0..b2.fighters.len())
            .find(|i| b2.fighters[*i].is_player)
            .unwrap();
        b2.deal_damage(&project2, p2, e2, Element::Ice, 0.01, 1, 0);
        assert_eq!(b2.fighters[e2].shields, 2);
    }

    #[test]
    fn broken_enemies_take_bonus_damage() {
        let (project, mut b) = setup(4); // golem, high HP
        let e = first_alive_enemy(&b);
        let p = (0..b.fighters.len())
            .find(|i| b.fighters[*i].is_player)
            .unwrap();
        // Sample damage many times before/after break to smooth variance.
        let sample = |b: &mut Battle, project: &ProjectData| -> f32 {
            let mut total = 0.0;
            for _ in 0..40 {
                let before = b.fighters[e].hp;
                b.deal_damage(project, p, e, Element::Ice, 1.0, 1, 0);
                total += (before - b.fighters[e].hp) as f32;
                b.fighters[e].hp = b.fighters[e].max.hp; // refill
            }
            total / 40.0
        };
        let normal = sample(&mut b, &project);
        b.fighters[e].broken = 2;
        let broken = sample(&mut b, &project);
        assert!(
            broken > normal * 1.3,
            "break should amplify damage: {normal} -> {broken}"
        );
    }

    #[test]
    fn boost_spends_bp_and_amplifies() {
        let (project, mut b) = setup(4);
        let e = first_alive_enemy(&b);
        let p = (0..b.fighters.len())
            .find(|i| b.fighters[*i].is_player)
            .unwrap();
        b.fighters[p].bp = 3;
        b.phase = Phase::Choose {
            fighter: p,
            menu: Menu::Root,
            boost: 0,
        };
        let before = b.fighters[e].hp;
        b.execute_player_action(&project, p, ActionKind::Attack, Some(e), 3);
        assert_eq!(b.fighters[p].bp, 0, "boost must spend BP");
        assert!(b.fighters[e].hp < before);
        assert!(matches!(b.phase, Phase::Anim { .. }));
    }

    #[test]
    fn bp_banks_each_round() {
        let (_project, mut b) = setup(1);
        let p = (0..b.fighters.len())
            .find(|i| b.fighters[*i].is_player)
            .unwrap();
        let start = b.fighters[p].bp;
        b.end_round();
        assert_eq!(b.fighters[p].bp, start + 1);
        for _ in 0..20 {
            b.end_round();
        }
        assert_eq!(b.fighters[p].bp, MAX_BP, "BP caps at {MAX_BP}");
    }

    #[test]
    fn full_battle_reaches_victory_and_grants_exp() {
        let (project, mut b) = setup(1);
        let mut party: Vec<Member> = default_project(Language::default())
            .system
            .party
            .iter()
            .filter_map(|id| project.actor(*id))
            .map(|a| crate::game::member_from_actor(a, 1))
            .collect();
        let mut inventory = vec![(1u32, 5u32)];
        let mut kind = None;
        // Drive the battle: players always basic-attack the first alive enemy.
        for _ in 0..20_000 {
            if let Phase::Choose { fighter, .. } = b.phase {
                let target = (0..b.fighters.len())
                    .find(|i| !b.fighters[*i].is_player && b.fighters[*i].alive());
                if let Some(t) = target {
                    b.execute_player_action(&project, fighter, ActionKind::Attack, Some(t), 1);
                } else {
                    break;
                }
            }
            if let Some(k) = b.update(&project, 0.1) {
                kind = Some(k);
                break;
            }
        }
        let kind = kind.expect("battle should end");
        // Level-1 party vs 2 slimes must not lose.
        assert_eq!(kind, OutcomeKind::Victory);
        let before_exp: u32 = party.iter().map(|m| m.exp).sum();
        let outcome = b.settle(&mut party, &project, kind, &mut inventory);
        match outcome {
            Outcome::Victory { exp, .. } => {
                assert_eq!(exp, 24, "two slimes give 12 exp each");
                assert!(party.iter().any(|m| m.exp > 0 || m.level > 1));
                let _ = before_exp;
            }
            _ => panic!("expected victory outcome"),
        }
    }

    #[test]
    fn healing_never_exceeds_max_and_skips_dead() {
        let (_project, mut b) = setup(1);
        let p = (0..b.fighters.len())
            .find(|i| b.fighters[*i].is_player)
            .unwrap();
        b.fighters[p].hp = b.fighters[p].max.hp - 5;
        b.heal(p, 9999);
        assert_eq!(b.fighters[p].hp, b.fighters[p].max.hp);
        b.fighters[p].hp = 0;
        b.heal(p, 9999);
        assert_eq!(b.fighters[p].hp, 0, "heal must not revive the dead");
    }
}
