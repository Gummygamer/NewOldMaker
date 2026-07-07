//! The database window: actors, skills, items, enemies, troops, system, LLM.

use eframe::egui::{self, RichText};

use crate::core::data::*;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum DbTab {
    Actors,
    Skills,
    Items,
    Enemies,
    Troops,
    System,
    Llm,
}

pub fn database_window(ctx: &egui::Context, project: &mut ProjectData, open: &mut bool, tab: &mut DbTab) {
    egui::Window::new("🗄 Database")
        .open(open)
        .default_size([620.0, 520.0])
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(tab, DbTab::Actors, "Actors");
                ui.selectable_value(tab, DbTab::Skills, "Skills");
                ui.selectable_value(tab, DbTab::Items, "Items");
                ui.selectable_value(tab, DbTab::Enemies, "Enemies");
                ui.selectable_value(tab, DbTab::Troops, "Troops");
                ui.selectable_value(tab, DbTab::System, "System");
                ui.selectable_value(tab, DbTab::Llm, "LLM");
            });
            ui.separator();
            egui::ScrollArea::vertical().show(ui, |ui| match tab {
                DbTab::Actors => actors_tab(ui, project),
                DbTab::Skills => skills_tab(ui, project),
                DbTab::Items => items_tab(ui, project),
                DbTab::Enemies => enemies_tab(ui, project),
                DbTab::Troops => troops_tab(ui, project),
                DbTab::System => system_tab(ui, project),
                DbTab::Llm => llm_tab(ui, project),
            });
        });
}

fn stats_ui(ui: &mut egui::Ui, s: &mut Stats, growth: bool) {
    let speed = if growth { 0.05 } else { 0.5 };
    egui::Grid::new(ui.next_auto_id()).num_columns(7).show(ui, |ui| {
        for (label, v) in [
            ("HP", &mut s.hp),
            ("MP", &mut s.mp),
            ("ATK", &mut s.atk),
            ("DEF", &mut s.def),
            ("MAG", &mut s.mag),
            ("SPR", &mut s.spr),
            ("SPD", &mut s.spd),
        ] {
            ui.vertical(|ui| {
                ui.small(label);
                ui.add(egui::DragValue::new(v).speed(speed));
            });
        }
        ui.end_row();
    });
}

fn element_combo(ui: &mut egui::Ui, label: &str, e: &mut Element) {
    egui::ComboBox::from_label(label)
        .selected_text(format!("{} {}", e.icon(), e.name()))
        .show_ui(ui, |ui| {
            for el in ALL_ELEMENTS {
                ui.selectable_value(e, el, format!("{} {}", el.icon(), el.name()));
            }
        });
}

// ---------------------------------------------------------------------------

fn actors_tab(ui: &mut egui::Ui, project: &mut ProjectData) {
    let skills: Vec<(u32, String)> = project.skills.iter().map(|s| (s.id, s.name.clone())).collect();
    let mut remove = None;
    for (i, a) in project.actors.iter_mut().enumerate() {
        egui::CollapsingHeader::new(format!("{} · {} ({})", a.id, a.name, a.class_name))
            .id_salt(("actor", a.id))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Name");
                    ui.text_edit_singleline(&mut a.name);
                    ui.label("Class");
                    ui.text_edit_singleline(&mut a.class_name);
                });
                ui.horizontal(|ui| {
                    ui.label("Sprite");
                    let mut s = a.sprite as i32;
                    if ui.add(egui::Slider::new(&mut s, 0..=7)).changed() {
                        a.sprite = s as u8;
                    }
                });
                element_combo(ui, "Attack element", &mut a.attack_element);
                ui.label(RichText::new("Base stats (Lv.1)").strong());
                stats_ui(ui, &mut a.base, false);
                ui.label(RichText::new("Growth per level").strong());
                stats_ui(ui, &mut a.growth, true);
                ui.label(RichText::new("Learnset (level → skill)").strong());
                let mut remove_learn = None;
                for (j, (lv, sk)) in a.learnset.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        ui.label("Lv");
                        ui.add(egui::DragValue::new(lv).range(1..=99));
                        egui::ComboBox::from_id_salt(("learn", a.id, j))
                            .selected_text(
                                skills.iter().find(|(id, _)| id == sk).map(|(_, n)| n.as_str()).unwrap_or("?"),
                            )
                            .show_ui(ui, |ui| {
                                for (id, name) in &skills {
                                    ui.selectable_value(sk, *id, name);
                                }
                            });
                        if ui.small_button("🗑").clicked() {
                            remove_learn = Some(j);
                        }
                    });
                }
                if let Some(j) = remove_learn {
                    a.learnset.remove(j);
                }
                if ui.button("＋ Add skill").clicked() {
                    if let Some((id, _)) = skills.first() {
                        a.learnset.push((1, *id));
                    }
                }
                if ui.button("🗑 Delete actor").clicked() {
                    remove = Some(i);
                }
            });
    }
    if let Some(i) = remove {
        let id = project.actors[i].id;
        project.actors.remove(i);
        project.system.party.retain(|a| *a != id);
    }
    if ui.button("＋ New actor").clicked() {
        let id = project.actors.iter().map(|a| a.id).max().unwrap_or(0) + 1;
        project.actors.push(Actor {
            id,
            name: format!("Hero {id}"),
            class_name: "Adventurer".into(),
            sprite: (id % 8) as u8,
            base: Stats { hp: 90, mp: 20, atk: 10, def: 10, mag: 10, spr: 10, spd: 10 },
            growth: Stats { hp: 10, mp: 3, atk: 2, def: 2, mag: 2, spr: 2, spd: 1 },
            learnset: vec![],
            attack_element: Element::Slash,
        });
    }
}

fn skills_tab(ui: &mut egui::Ui, project: &mut ProjectData) {
    let mut remove = None;
    for (i, s) in project.skills.iter_mut().enumerate() {
        egui::CollapsingHeader::new(format!("{} · {} {}", s.id, s.element.icon(), s.name))
            .id_salt(("skill", s.id))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Name");
                    ui.text_edit_singleline(&mut s.name);
                });
                element_combo(ui, "Element", &mut s.element);
                ui.horizontal(|ui| {
                    ui.label("Power ×");
                    ui.add(egui::DragValue::new(&mut s.power).speed(0.05).range(0.0..=8.0));
                    ui.label("MP");
                    ui.add(egui::DragValue::new(&mut s.mp_cost).range(0..=99));
                    ui.label("Hits");
                    ui.add(egui::DragValue::new(&mut s.hits).range(1..=8));
                });
                egui::ComboBox::from_label("Target")
                    .selected_text(format!("{:?}", s.target))
                    .show_ui(ui, |ui| {
                        for t in [
                            SkillTarget::OneEnemy,
                            SkillTarget::AllEnemies,
                            SkillTarget::OneAlly,
                            SkillTarget::AllAllies,
                            SkillTarget::Own,
                        ] {
                            ui.selectable_value(&mut s.target, t, format!("{t:?}"));
                        }
                    });
                let mut effect_idx = match s.effect {
                    SkillEffect::Damage => 0,
                    SkillEffect::Heal => 1,
                    SkillEffect::BuffAttack(_) => 2,
                    SkillEffect::BreakDefense(_) => 3,
                };
                egui::ComboBox::from_label("Effect")
                    .selected_text(["Damage", "Heal", "Buff attack", "Break defense"][effect_idx])
                    .show_ui(ui, |ui| {
                        for (j, name) in ["Damage", "Heal", "Buff attack", "Break defense"].iter().enumerate() {
                            if ui.selectable_value(&mut effect_idx, j, *name).changed() {
                                s.effect = match j {
                                    0 => SkillEffect::Damage,
                                    1 => SkillEffect::Heal,
                                    2 => SkillEffect::BuffAttack(3),
                                    _ => SkillEffect::BreakDefense(3),
                                };
                            }
                        }
                    });
                ui.label("Description");
                ui.text_edit_singleline(&mut s.description);
                if ui.button("🗑 Delete skill").clicked() {
                    remove = Some(i);
                }
            });
    }
    if let Some(i) = remove {
        project.skills.remove(i);
    }
    if ui.button("＋ New skill").clicked() {
        let id = project.skills.iter().map(|s| s.id).max().unwrap_or(0) + 1;
        project.skills.push(Skill {
            id,
            name: format!("Skill {id}"),
            element: Element::Slash,
            power: 1.2,
            mp_cost: 5,
            target: SkillTarget::OneEnemy,
            effect: SkillEffect::Damage,
            hits: 1,
            description: String::new(),
        });
    }
}

fn items_tab(ui: &mut egui::Ui, project: &mut ProjectData) {
    let mut remove = None;
    for (i, item) in project.items.iter_mut().enumerate() {
        egui::CollapsingHeader::new(format!("{} · {}", item.id, item.name))
            .id_salt(("item", item.id))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Name");
                    ui.text_edit_singleline(&mut item.name);
                });
                egui::ComboBox::from_label("Kind")
                    .selected_text(format!("{:?}", item.kind))
                    .show_ui(ui, |ui| {
                        for k in [ItemKind::HealHp, ItemKind::HealMp, ItemKind::Revive] {
                            ui.selectable_value(&mut item.kind, k, format!("{k:?}"));
                        }
                    });
                ui.horizontal(|ui| {
                    ui.label("Power");
                    ui.add(egui::DragValue::new(&mut item.power).range(0..=999));
                });
                ui.label("Description");
                ui.text_edit_singleline(&mut item.description);
                if ui.button("🗑 Delete item").clicked() {
                    remove = Some(i);
                }
            });
    }
    if let Some(i) = remove {
        project.items.remove(i);
    }
    if ui.button("＋ New item").clicked() {
        let id = project.items.iter().map(|i| i.id).max().unwrap_or(0) + 1;
        project.items.push(Item {
            id,
            name: format!("Item {id}"),
            kind: ItemKind::HealHp,
            power: 50,
            description: String::new(),
        });
    }
}

fn enemies_tab(ui: &mut egui::Ui, project: &mut ProjectData) {
    let skills: Vec<(u32, String)> = project.skills.iter().map(|s| (s.id, s.name.clone())).collect();
    let mut remove = None;
    for (i, e) in project.enemies.iter_mut().enumerate() {
        egui::CollapsingHeader::new(format!("{} · {}", e.id, e.name))
            .id_salt(("enemy", e.id))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Name");
                    ui.text_edit_singleline(&mut e.name);
                    ui.label("Sprite");
                    let mut s = e.sprite as i32;
                    if ui.add(egui::Slider::new(&mut s, 0..=3)).changed() {
                        e.sprite = s as u8;
                    }
                });
                stats_ui(ui, &mut e.stats, false);
                ui.horizontal(|ui| {
                    ui.label("EXP");
                    ui.add(egui::DragValue::new(&mut e.exp).range(0..=9999));
                    ui.label("Shields");
                    ui.add(egui::DragValue::new(&mut e.shields).range(0..=15));
                });
                ui.label(RichText::new("Weaknesses (Octopath-style)").strong());
                ui.horizontal_wrapped(|ui| {
                    for el in ALL_ELEMENTS {
                        let mut on = e.weaknesses.contains(&el);
                        if ui.toggle_value(&mut on, format!("{} {}", el.icon(), el.name())).changed() {
                            if on {
                                e.weaknesses.push(el);
                            } else {
                                e.weaknesses.retain(|w| *w != el);
                            }
                        }
                    }
                });
                ui.label(RichText::new("Skills").strong());
                ui.horizontal_wrapped(|ui| {
                    for (id, name) in &skills {
                        let mut on = e.skills.contains(id);
                        if ui.toggle_value(&mut on, name).changed() {
                            if on {
                                e.skills.push(*id);
                            } else {
                                e.skills.retain(|s| s != id);
                            }
                        }
                    }
                });
                if ui.button("🗑 Delete enemy").clicked() {
                    remove = Some(i);
                }
            });
    }
    if let Some(i) = remove {
        project.enemies.remove(i);
    }
    if ui.button("＋ New enemy").clicked() {
        let id = project.enemies.iter().map(|e| e.id).max().unwrap_or(0) + 1;
        project.enemies.push(Enemy {
            id,
            name: format!("Enemy {id}"),
            sprite: 0,
            stats: Stats { hp: 60, mp: 0, atk: 10, def: 8, mag: 6, spr: 6, spd: 8 },
            exp: 15,
            shields: 2,
            weaknesses: vec![Element::Fire],
            skills: vec![],
        });
    }
}

fn troops_tab(ui: &mut egui::Ui, project: &mut ProjectData) {
    let enemies: Vec<(u32, String)> = project.enemies.iter().map(|e| (e.id, e.name.clone())).collect();
    let mut remove = None;
    for (i, t) in project.troops.iter_mut().enumerate() {
        egui::CollapsingHeader::new(format!("{} · {}", t.id, t.name))
            .id_salt(("troop", t.id))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Name");
                    ui.text_edit_singleline(&mut t.name);
                });
                let mut remove_member = None;
                for (j, m) in t.members.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        egui::ComboBox::from_id_salt(("troop-member", t.id, j))
                            .selected_text(
                                enemies.iter().find(|(id, _)| id == m).map(|(_, n)| n.as_str()).unwrap_or("?"),
                            )
                            .show_ui(ui, |ui| {
                                for (id, name) in &enemies {
                                    ui.selectable_value(m, *id, name);
                                }
                            });
                        if ui.small_button("🗑").clicked() {
                            remove_member = Some(j);
                        }
                    });
                }
                if let Some(j) = remove_member {
                    t.members.remove(j);
                }
                if t.members.len() < 4 && ui.button("＋ Add enemy").clicked() {
                    if let Some((id, _)) = enemies.first() {
                        t.members.push(*id);
                    }
                }
                if ui.button("🗑 Delete troop").clicked() {
                    remove = Some(i);
                }
            });
    }
    if let Some(i) = remove {
        project.troops.remove(i);
    }
    if ui.button("＋ New troop").clicked() {
        let id = project.troops.iter().map(|t| t.id).max().unwrap_or(0) + 1;
        let members = project.enemies.first().map(|e| vec![e.id]).unwrap_or_default();
        project.troops.push(Troop { id, name: format!("Troop {id}"), members });
    }
}

fn system_tab(ui: &mut egui::Ui, project: &mut ProjectData) {
    ui.horizontal(|ui| {
        ui.label("Game title");
        ui.text_edit_singleline(&mut project.system.title);
    });
    egui::ComboBox::from_label("Start map")
        .selected_text(
            project
                .maps
                .iter()
                .find(|m| m.id == project.system.start_map)
                .map(|m| m.name.clone())
                .unwrap_or_else(|| "?".into()),
        )
        .show_ui(ui, |ui| {
            let maps: Vec<(u32, String)> = project.maps.iter().map(|m| (m.id, m.name.clone())).collect();
            for (id, name) in maps {
                ui.selectable_value(&mut project.system.start_map, id, name);
            }
        });
    ui.horizontal(|ui| {
        ui.label("Start X");
        ui.add(egui::DragValue::new(&mut project.system.start_x));
        ui.label("Start Y");
        ui.add(egui::DragValue::new(&mut project.system.start_y));
    });
    ui.label(RichText::new("Starting party (max 4)").strong());
    let actors: Vec<(u32, String)> = project.actors.iter().map(|a| (a.id, a.name.clone())).collect();
    for (id, name) in &actors {
        let mut on = project.system.party.contains(id);
        if ui.checkbox(&mut on, name).changed() {
            if on && project.system.party.len() < 4 {
                project.system.party.push(*id);
            } else {
                project.system.party.retain(|a| a != id);
            }
        }
    }
}

fn llm_tab(ui: &mut egui::Ui, project: &mut ProjectData) {
    ui.label(RichText::new("Local LLM for NPC dialogue").strong());
    ui.small("Point this at a small instruct-tuned GGUF model (e.g. Qwen2.5-0.5B-Instruct Q4_K_M). NPCs with 'LLM dialogue enabled' will speak through it, fully offline.");
    ui.horizontal(|ui| {
        ui.label("Model (.gguf)");
        ui.text_edit_singleline(&mut project.llm.model_path);
        if ui.button("Browse…").clicked() {
            if let Some(path) = rfd::FileDialog::new().add_filter("GGUF model", &["gguf"]).pick_file() {
                project.llm.model_path = path.display().to_string();
            }
        }
    });
    let mut ctx = project.llm.context_tokens as i32;
    if ui.add(egui::Slider::new(&mut ctx, 512..=8192).text("Context tokens")).changed() {
        project.llm.context_tokens = ctx as u32;
    }
    let mut max = project.llm.max_reply_tokens as i32;
    if ui.add(egui::Slider::new(&mut max, 24..=256).text("Max reply tokens")).changed() {
        project.llm.max_reply_tokens = max as u32;
    }
    ui.add(egui::Slider::new(&mut project.llm.temperature, 0.1..=1.5).text("Temperature"));
    let mut threads = project.llm.threads as i32;
    if ui.add(egui::Slider::new(&mut threads, 0..=32).text("CPU threads (0 = auto)")).changed() {
        project.llm.threads = threads as u32;
    }
    #[cfg(not(feature = "llm"))]
    ui.colored_label(egui::Color32::YELLOW, "Engine built without the `llm` feature — fallback lines will be used.");
}
