//! The starter project every new NewOldMaker project begins from:
//! a meadow village map, a cave map, four heroes, and a small bestiary.
//!
//! Every player-facing string is localized to the chosen [`Language`] so a new
//! project starts fully in that language — spell names, items, enemies, map
//! and location names, signs and NPC lines included. The LLM-steering persona
//! fields (role/personality/knowledge/constraints) stay English on purpose:
//! the model is nudged into the target language via [`Language::llm_instruction`].

use super::data::*;

pub fn default_project(language: Language) -> ProjectData {
    ProjectData {
        format_version: FORMAT_VERSION,
        name: language.pick("Untitled Tale", "Conto Sem Título").into(),
        maps: vec![meadow_map(language), cave_map(language)],
        actors: default_actors(language),
        skills: default_skills(language),
        items: default_items(language),
        enemies: default_enemies(language),
        troops: default_troops(language),
        system: SystemData {
            title: language.pick("Untitled Tale", "Conto Sem Título").into(),
            start_map: 1,
            start_x: 12,
            start_y: 16,
            party: vec![1, 2, 3, 4],
            start_items: vec![(1, 5), (2, 3), (3, 1)],
            language,
        },
        llm: LlmSettings::default(),
    }
}

// ---------------------------------------------------------------------------
// Maps
// ---------------------------------------------------------------------------

fn meadow_map(lang: Language) -> MapData {
    let mut m = MapData::new(
        1,
        lang.pick("Riverside Meadow", "Campina à Beira-Rio"),
        28,
        24,
    );
    m.encounter_troops = vec![1, 2];
    m.encounter_steps = 14;

    // Gentle hills: raise a plateau in the north.
    for y in 0..24i32 {
        for x in 0..28i32 {
            let t = m.tile_mut(x, y).unwrap();
            if y < 6 {
                t.height = 3;
                t.terrain = Terrain::Grass as u8;
            } else if y < 8 {
                t.height = 2;
                t.terrain = Terrain::Dirt as u8;
            }
        }
    }
    // River running east-west with a stone bridge.
    for x in 0..28i32 {
        for y in 10..12i32 {
            let t = m.tile_mut(x, y).unwrap();
            t.terrain = Terrain::Water as u8;
            t.height = 0;
            t.prop = 0;
        }
    }
    for y in 10..12i32 {
        let t = m.tile_mut(13, y).unwrap();
        t.terrain = Terrain::StoneBrick as u8;
        t.height = 1;
    }
    // Village plaza in the south.
    for y in 14..20i32 {
        for x in 8..18i32 {
            let t = m.tile_mut(x, y).unwrap();
            t.terrain = Terrain::WoodFloor as u8;
        }
    }
    // Scatter trees & decor.
    let trees = [
        (2, 3),
        (5, 2),
        (21, 2),
        (24, 4),
        (3, 14),
        (4, 20),
        (22, 15),
        (24, 19),
        (20, 21),
        (2, 8),
        (25, 8),
        (6, 22),
    ];
    for (x, y) in trees {
        m.tile_mut(x, y).unwrap().prop = Prop::Tree as u8;
    }
    for (x, y) in [(7, 4), (18, 3), (23, 21)] {
        m.tile_mut(x, y).unwrap().prop = Prop::Pine as u8;
    }
    for (x, y) in [(10, 13), (16, 13), (10, 20), (16, 20)] {
        m.tile_mut(x, y).unwrap().prop = Prop::Torch as u8;
    }
    for (x, y) in [(6, 9), (20, 13), (19, 6)] {
        m.tile_mut(x, y).unwrap().prop = Prop::Rock as u8;
    }
    for (x, y) in [(9, 16), (3, 6), (24, 12), (11, 22)] {
        m.tile_mut(x, y).unwrap().prop = Prop::Flowers as u8;
    }
    m.tile_mut(17, 15).unwrap().prop = Prop::Barrel as u8;

    m.events = vec![
        EventData {
            id: 1,
            name: "Marta".into(),
            x: 11,
            y: 15,
            kind: EventKind::Npc {
                sprite: 4,
                persona: NpcPersona {
                    name: lang.pick("Old Marta", "Velha Marta").into(),
                    role: "the village apothecary of Riverside".into(),
                    personality: "Warm, talkative grandmother; speaks in short folksy sentences; calls everyone 'dearie'.".into(),
                    knowledge: "The cave north of the plateau is full of slimes and bats. A blue crystal deep inside is said to grant wishes. The bridge was built by her late husband Tomas.".into(),
                    constraints: "Never talks about politics. If asked about the crystal's power she only says 'some wishes are better left unwished'.".into(),
                    fallback_lines: match lang {
                        Language::English => vec![
                            "Ah, dearie, mind the river — the current is stronger than it looks.".into(),
                            "The cave up north? Full of slimes. Take a torch, dearie.".into(),
                        ],
                        Language::Portuguese => vec![
                            "Ah, querida, cuidado com o rio — a correnteza é mais forte do que parece.".into(),
                            "A caverna ao norte? Cheia de slimes. Leve uma tocha, querida.".into(),
                        ],
                    },
                    use_llm: true,
                },
                wander: true,
            },
        },
        EventData {
            id: 2,
            name: "Bram".into(),
            x: 15,
            y: 18,
            kind: EventKind::Npc {
                sprite: 5,
                persona: NpcPersona {
                    name: "Bram".into(),
                    role: "a retired sellsword who guards the village".into(),
                    personality: "Gruff, few words, secretly kind. Complains about his knees.".into(),
                    knowledge: "Slimes are weak to fire and swords. Bats hate light magic. He once fought a stone golem and lost.".into(),
                    constraints: "Refuses to leave the village or join the party.".into(),
                    fallback_lines: match lang {
                        Language::English => vec![
                            "Hmph. Slimes burn easy. Bats can't stand the light.".into(),
                            "My sword arm's done. Yours isn't. Go on then.".into(),
                        ],
                        Language::Portuguese => vec![
                            "Hmpf. Slimes queimam fácil. Morcegos não suportam a luz.".into(),
                            "Meu braço de espada já era. O seu não. Vá em frente, então.".into(),
                        ],
                    },
                    use_llm: true,
                },
                wander: false,
            },
        },
        EventData {
            id: 3,
            name: lang.pick("Village Sign", "Placa da Vila").into(),
            x: 13,
            y: 14,
            kind: EventKind::Sign {
                text: lang.pick(
                    "Riverside Village — pop. 27.\nCave of Whispers: north past the plateau.",
                    "Vila à Beira-Rio — pop. 27.\nCaverna dos Sussurros: ao norte, além do planalto.",
                ).into(),
            },
        },
        EventData {
            id: 4,
            name: lang.pick("To the Cave", "Para a Caverna").into(),
            x: 13,
            y: 0,
            kind: EventKind::Transfer { target_map: 2, target_x: 10, target_y: 18 },
        },
        EventData {
            id: 5,
            name: lang.pick("Chest", "Baú").into(),
            x: 25,
            y: 2,
            kind: EventKind::Chest { item_id: 1 },
        },
        EventData {
            id: 6,
            name: lang.pick("Shrine", "Santuário").into(),
            x: 9,
            y: 18,
            kind: EventKind::HealPoint,
        },
    ];
    m
}

fn cave_map(lang: Language) -> MapData {
    let mut m = MapData::new(
        2,
        lang.pick("Cave of Whispers", "Caverna dos Sussurros"),
        20,
        20,
    );
    m.encounter_troops = vec![2, 3];
    m.encounter_steps = 10;
    m.ambience = MapAmbience {
        sun_color: [0.25, 0.28, 0.38],
        ambient_color: [0.12, 0.13, 0.2],
        fog_color: [0.02, 0.02, 0.05],
        fog_density: 0.05,
        bloom_strength: 0.9,
        darkness: 0.85,
    };

    // Solid rock walls everywhere, carve rooms/corridors.
    for y in 0..20i32 {
        for x in 0..20i32 {
            let t = m.tile_mut(x, y).unwrap();
            t.terrain = Terrain::Stone as u8;
            t.height = 4;
        }
    }
    let mut carve = |x0: i32, y0: i32, x1: i32, y1: i32| {
        for y in y0..=y1 {
            for x in x0..=x1 {
                if let Some(t) = m.tile_mut(x, y) {
                    t.terrain = Terrain::CaveFloor as u8;
                    t.height = 1;
                }
            }
        }
    };
    carve(8, 14, 12, 19); // entrance hall
    carve(9, 8, 11, 14); // corridor
    carve(4, 4, 16, 8); // main chamber
    carve(4, 8, 6, 12); // west nook
    carve(14, 8, 16, 12); // east nook

    for (x, y) in [(9, 18), (11, 18), (9, 9), (5, 5), (15, 5)] {
        m.tile_mut(x, y).unwrap().prop = Prop::Torch as u8;
    }
    for (x, y) in [(5, 11), (15, 11), (10, 5)] {
        m.tile_mut(x, y).unwrap().prop = Prop::Crystal as u8;
    }
    for (x, y) in [(6, 6), (13, 7), (12, 16)] {
        m.tile_mut(x, y).unwrap().prop = Prop::Rock as u8;
    }

    m.events = vec![
        EventData {
            id: 1,
            name: lang.pick("Back to Meadow", "Voltar à Campina").into(),
            x: 10,
            y: 19,
            kind: EventKind::Transfer { target_map: 1, target_x: 13, target_y: 1 },
        },
        EventData {
            id: 2,
            name: "Echo".into(),
            x: 10,
            y: 4,
            kind: EventKind::Npc {
                sprite: 6,
                persona: NpcPersona {
                    name: lang.pick("Echo", "Eco").into(),
                    role: "a ghostly spirit bound to the wishing crystal".into(),
                    personality: "Cryptic, melancholic, speaks in riddles and half-finished sentences.".into(),
                    knowledge: "It guards the great crystal. The golem below wakes when greed enters the cave. It remembers every adventurer who never left.".into(),
                    constraints: "Never states plainly what the crystal does. Never leaves the chamber.".into(),
                    fallback_lines: match lang {
                        Language::English => vec![
                            "...another warm one comes... the crystal hums when you lie...".into(),
                            "...the golem sleeps on greed... tread light, tread light...".into(),
                        ],
                        Language::Portuguese => vec![
                            "...mais um ser quente se aproxima... o cristal vibra quando você mente...".into(),
                            "...o golem dorme sobre a ganância... pise leve, pise leve...".into(),
                        ],
                    },
                    use_llm: true,
                },
                wander: false,
            },
        },
        EventData {
            id: 3,
            name: lang.pick("Golem", "Golem").into(),
            x: 10,
            y: 6,
            kind: EventKind::BattleTrigger { troop_id: 4, once: true },
        },
        EventData {
            id: 4,
            name: lang.pick("Chest", "Baú").into(),
            x: 4,
            y: 12,
            kind: EventKind::Chest { item_id: 3 },
        },
    ];
    m
}

// ---------------------------------------------------------------------------
// Database
// ---------------------------------------------------------------------------

fn stats(hp: i32, mp: i32, atk: i32, def: i32, mag: i32, spr: i32, spd: i32) -> Stats {
    Stats {
        hp,
        mp,
        atk,
        def,
        mag,
        spr,
        spd,
    }
}

fn default_actors(lang: Language) -> Vec<Actor> {
    vec![
        Actor {
            id: 1,
            name: "Aldric".into(),
            class_name: lang.pick("Warrior", "Guerreiro").into(),
            sprite: 0,
            base: stats(120, 12, 16, 14, 6, 8, 9),
            growth: stats(14, 2, 3, 3, 1, 1, 1),
            learnset: vec![(1, 1), (1, 2), (4, 3)],
            attack_element: Element::Slash,
        },
        Actor {
            id: 2,
            name: "Lyra".into(),
            class_name: lang.pick("Mage", "Maga").into(),
            sprite: 1,
            base: stats(78, 34, 7, 8, 18, 12, 10),
            growth: stats(8, 5, 1, 1, 3, 2, 1),
            learnset: vec![(1, 4), (1, 5), (3, 6), (6, 7)],
            attack_element: Element::Blunt,
        },
        Actor {
            id: 3,
            name: "Serah".into(),
            class_name: lang.pick("Cleric", "Clériga").into(),
            sprite: 2,
            base: stats(92, 30, 9, 10, 14, 16, 8),
            growth: stats(10, 4, 1, 2, 2, 3, 1),
            learnset: vec![(1, 8), (1, 9), (5, 10)],
            attack_element: Element::Blunt,
        },
        Actor {
            id: 4,
            name: "Finn".into(),
            class_name: lang.pick("Thief", "Ladino").into(),
            sprite: 3,
            base: stats(88, 16, 13, 9, 8, 8, 16),
            growth: stats(9, 2, 2, 1, 1, 1, 3),
            learnset: vec![(1, 11), (2, 12), (5, 13)],
            attack_element: Element::Pierce,
        },
    ]
}

fn default_skills(lang: Language) -> Vec<Skill> {
    let s = |id: u32,
             name: &str,
             element: Element,
             power: f32,
             mp: u32,
             target: SkillTarget,
             effect: SkillEffect,
             hits: u8,
             d: &str| Skill {
        id,
        name: name.into(),
        element,
        power,
        mp_cost: mp,
        target,
        effect,
        hits,
        description: d.into(),
    };
    let t = |en, pt| lang.pick(en, pt);
    vec![
        s(
            1,
            t("Cleave", "Talho"),
            Element::Slash,
            1.4,
            4,
            SkillTarget::OneEnemy,
            SkillEffect::Damage,
            1,
            t("A heavy sword blow.", "Um golpe pesado de espada."),
        ),
        s(
            2,
            t("Cross Strike", "Golpe Cruzado"),
            Element::Slash,
            0.6,
            6,
            SkillTarget::OneEnemy,
            SkillEffect::Damage,
            2,
            t("Two quick slashes.", "Dois cortes rápidos."),
        ),
        s(
            3,
            t("Warcry", "Grito de Guerra"),
            Element::Blunt,
            0.0,
            5,
            SkillTarget::AllAllies,
            SkillEffect::BuffAttack(3),
            1,
            t(
                "Raise the party's attack for 3 turns.",
                "Aumenta o ataque do grupo por 3 turnos.",
            ),
        ),
        s(
            4,
            t("Fireball", "Bola de Fogo"),
            Element::Fire,
            1.5,
            6,
            SkillTarget::OneEnemy,
            SkillEffect::Damage,
            1,
            t("Hurl a ball of flame.", "Lança uma bola de chamas."),
        ),
        s(
            5,
            t("Icicle", "Sincelo"),
            Element::Ice,
            1.3,
            5,
            SkillTarget::OneEnemy,
            SkillEffect::Damage,
            1,
            t("A spear of ice.", "Uma lança de gelo."),
        ),
        s(
            6,
            t("Thunder", "Trovão"),
            Element::Lightning,
            1.2,
            8,
            SkillTarget::AllEnemies,
            SkillEffect::Damage,
            1,
            t(
                "Lightning strikes every foe.",
                "Um raio atinge todos os inimigos.",
            ),
        ),
        s(
            7,
            t("Inferno", "Inferno"),
            Element::Fire,
            1.9,
            14,
            SkillTarget::AllEnemies,
            SkillEffect::Damage,
            1,
            t(
                "Engulf all foes in fire.",
                "Envolve todos os inimigos em fogo.",
            ),
        ),
        s(
            8,
            t("Mend", "Curar"),
            Element::Light,
            1.6,
            5,
            SkillTarget::OneAlly,
            SkillEffect::Heal,
            1,
            t("Restore one ally's HP.", "Restaura o HP de um aliado."),
        ),
        s(
            9,
            t("Ray", "Raio de Luz"),
            Element::Light,
            1.2,
            5,
            SkillTarget::OneEnemy,
            SkillEffect::Damage,
            1,
            t("A beam of holy light.", "Um feixe de luz sagrada."),
        ),
        s(
            10,
            t("Blessing", "Bênção"),
            Element::Light,
            1.0,
            12,
            SkillTarget::AllAllies,
            SkillEffect::Heal,
            1,
            t(
                "Restore the whole party's HP.",
                "Restaura o HP de todo o grupo.",
            ),
        ),
        s(
            11,
            t("Dagger Dance", "Dança das Adagas"),
            Element::Pierce,
            0.45,
            4,
            SkillTarget::OneEnemy,
            SkillEffect::Damage,
            3,
            t("Three lightning-fast stabs.", "Três estocadas velozes."),
        ),
        s(
            12,
            t("Armor Crush", "Quebra-Armadura"),
            Element::Blunt,
            0.9,
            6,
            SkillTarget::OneEnemy,
            SkillEffect::BreakDefense(3),
            1,
            t(
                "Damage and lower defense for 3 turns.",
                "Causa dano e reduz a defesa por 3 turnos.",
            ),
        ),
        s(
            13,
            t("Shadow Fang", "Presa Sombria"),
            Element::Dark,
            1.5,
            8,
            SkillTarget::OneEnemy,
            SkillEffect::Damage,
            1,
            t("A strike from the shadows.", "Um golpe vindo das sombras."),
        ),
        s(
            14,
            t("Gnaw", "Mordida"),
            Element::Pierce,
            1.0,
            0,
            SkillTarget::OneEnemy,
            SkillEffect::Damage,
            1,
            t("Bite.", "Mordida."),
        ),
        s(
            15,
            t("Screech", "Guincho"),
            Element::Dark,
            0.8,
            0,
            SkillTarget::AllEnemies,
            SkillEffect::Damage,
            1,
            t("An ear-splitting cry.", "Um grito ensurdecedor."),
        ),
        s(
            16,
            t("Boulder Fist", "Punho de Pedra"),
            Element::Blunt,
            1.6,
            0,
            SkillTarget::OneEnemy,
            SkillEffect::Damage,
            1,
            t("A crushing stone fist.", "Um punho de pedra esmagador."),
        ),
    ]
}

fn default_items(lang: Language) -> Vec<Item> {
    vec![
        Item {
            id: 1,
            name: lang.pick("Healing Herb", "Erva Curativa").into(),
            kind: ItemKind::HealHp,
            power: 60,
            description: lang.pick("Restores 60 HP.", "Restaura 60 de HP.").into(),
        },
        Item {
            id: 2,
            name: lang.pick("Mana Drop", "Gota de Mana").into(),
            kind: ItemKind::HealMp,
            power: 25,
            description: lang.pick("Restores 25 MP.", "Restaura 25 de MP.").into(),
        },
        Item {
            id: 3,
            name: lang.pick("Phoenix Ash", "Cinzas de Fênix").into(),
            kind: ItemKind::Revive,
            power: 50,
            description: lang
                .pick(
                    "Revives a fallen ally with 50 HP.",
                    "Revive um aliado caído com 50 de HP.",
                )
                .into(),
        },
    ]
}

fn default_enemies(lang: Language) -> Vec<Enemy> {
    vec![
        Enemy {
            id: 1,
            name: lang.pick("Meadow Slime", "Slime da Campina").into(),
            sprite: 0,
            stats: stats(55, 0, 9, 6, 4, 4, 6),
            exp: 12,
            shields: 2,
            weaknesses: vec![Element::Slash, Element::Fire],
            skills: vec![],
        },
        Enemy {
            id: 2,
            name: lang.pick("Cave Bat", "Morcego da Caverna").into(),
            sprite: 1,
            stats: stats(42, 10, 11, 4, 8, 5, 14),
            exp: 15,
            shields: 2,
            weaknesses: vec![Element::Pierce, Element::Light],
            skills: vec![15],
        },
        Enemy {
            id: 3,
            name: lang.pick("Mud Crawler", "Rastejante de Lama").into(),
            sprite: 2,
            stats: stats(80, 0, 13, 10, 5, 6, 5),
            exp: 22,
            shields: 3,
            weaknesses: vec![Element::Ice, Element::Lightning],
            skills: vec![14],
        },
        Enemy {
            id: 4,
            name: lang.pick("Stone Golem", "Golem de Pedra").into(),
            sprite: 3,
            stats: stats(320, 20, 18, 16, 10, 10, 4),
            exp: 120,
            shields: 5,
            weaknesses: vec![Element::Blunt, Element::Lightning, Element::Dark],
            skills: vec![16],
        },
    ]
}

fn default_troops(lang: Language) -> Vec<Troop> {
    vec![
        Troop {
            id: 1,
            name: lang.pick("Slimes ×2", "Slimes ×2").into(),
            members: vec![1, 1],
        },
        Troop {
            id: 2,
            name: lang.pick("Slime & Bat", "Slime e Morcego").into(),
            members: vec![1, 2],
        },
        Troop {
            id: 3,
            name: lang.pick("Cave Pack", "Bando da Caverna").into(),
            members: vec![2, 2, 3],
        },
        Troop {
            id: 4,
            name: lang.pick("Stone Golem", "Golem de Pedra").into(),
            members: vec![4],
        },
    ]
}
