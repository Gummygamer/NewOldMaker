//! The starter project every new NewOldMaker project begins from:
//! a meadow village map, a cave map, four heroes, and a small bestiary.

use super::data::*;

pub fn default_project() -> ProjectData {
    ProjectData {
        format_version: FORMAT_VERSION,
        name: "Untitled Tale".into(),
        maps: vec![meadow_map(), cave_map()],
        actors: default_actors(),
        skills: default_skills(),
        items: default_items(),
        enemies: default_enemies(),
        troops: default_troops(),
        system: SystemData {
            title: "Untitled Tale".into(),
            start_map: 1,
            start_x: 12,
            start_y: 16,
            party: vec![1, 2, 3, 4],
            start_items: vec![(1, 5), (2, 3), (3, 1)],
        },
        llm: LlmSettings::default(),
    }
}

// ---------------------------------------------------------------------------
// Maps
// ---------------------------------------------------------------------------

fn meadow_map() -> MapData {
    let mut m = MapData::new(1, "Riverside Meadow", 28, 24);
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
        (2, 3), (5, 2), (21, 2), (24, 4), (3, 14), (4, 20), (22, 15), (24, 19),
        (20, 21), (2, 8), (25, 8), (6, 22),
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
                    name: "Old Marta".into(),
                    role: "the village apothecary of Riverside".into(),
                    personality: "Warm, talkative grandmother; speaks in short folksy sentences; calls everyone 'dearie'.".into(),
                    knowledge: "The cave north of the plateau is full of slimes and bats. A blue crystal deep inside is said to grant wishes. The bridge was built by her late husband Tomas.".into(),
                    constraints: "Never talks about politics. If asked about the crystal's power she only says 'some wishes are better left unwished'.".into(),
                    fallback_lines: vec![
                        "Ah, dearie, mind the river — the current is stronger than it looks.".into(),
                        "The cave up north? Full of slimes. Take a torch, dearie.".into(),
                    ],
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
                    fallback_lines: vec![
                        "Hmph. Slimes burn easy. Bats can't stand the light.".into(),
                        "My sword arm's done. Yours isn't. Go on then.".into(),
                    ],
                    use_llm: true,
                },
                wander: false,
            },
        },
        EventData {
            id: 3,
            name: "Village Sign".into(),
            x: 13,
            y: 14,
            kind: EventKind::Sign { text: "Riverside Village — pop. 27.\nCave of Whispers: north past the plateau.".into() },
        },
        EventData {
            id: 4,
            name: "To the Cave".into(),
            x: 13,
            y: 0,
            kind: EventKind::Transfer { target_map: 2, target_x: 10, target_y: 18 },
        },
        EventData {
            id: 5,
            name: "Chest".into(),
            x: 25,
            y: 2,
            kind: EventKind::Chest { item_id: 1 },
        },
        EventData {
            id: 6,
            name: "Shrine".into(),
            x: 9,
            y: 18,
            kind: EventKind::HealPoint,
        },
    ];
    m
}

fn cave_map() -> MapData {
    let mut m = MapData::new(2, "Cave of Whispers", 20, 20);
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
            name: "Back to Meadow".into(),
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
                    name: "Echo".into(),
                    role: "a ghostly spirit bound to the wishing crystal".into(),
                    personality: "Cryptic, melancholic, speaks in riddles and half-finished sentences.".into(),
                    knowledge: "It guards the great crystal. The golem below wakes when greed enters the cave. It remembers every adventurer who never left.".into(),
                    constraints: "Never states plainly what the crystal does. Never leaves the chamber.".into(),
                    fallback_lines: vec![
                        "...another warm one comes... the crystal hums when you lie...".into(),
                        "...the golem sleeps on greed... tread light, tread light...".into(),
                    ],
                    use_llm: true,
                },
                wander: false,
            },
        },
        EventData {
            id: 3,
            name: "Golem".into(),
            x: 10,
            y: 6,
            kind: EventKind::BattleTrigger { troop_id: 4, once: true },
        },
        EventData {
            id: 4,
            name: "Chest".into(),
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
    Stats { hp, mp, atk, def, mag, spr, spd }
}

fn default_actors() -> Vec<Actor> {
    vec![
        Actor {
            id: 1,
            name: "Aldric".into(),
            class_name: "Warrior".into(),
            sprite: 0,
            base: stats(120, 12, 16, 14, 6, 8, 9),
            growth: stats(14, 2, 3, 3, 1, 1, 1),
            learnset: vec![(1, 1), (1, 2), (4, 3)],
            attack_element: Element::Slash,
        },
        Actor {
            id: 2,
            name: "Lyra".into(),
            class_name: "Mage".into(),
            sprite: 1,
            base: stats(78, 34, 7, 8, 18, 12, 10),
            growth: stats(8, 5, 1, 1, 3, 2, 1),
            learnset: vec![(1, 4), (1, 5), (3, 6), (6, 7)],
            attack_element: Element::Blunt,
        },
        Actor {
            id: 3,
            name: "Serah".into(),
            class_name: "Cleric".into(),
            sprite: 2,
            base: stats(92, 30, 9, 10, 14, 16, 8),
            growth: stats(10, 4, 1, 2, 2, 3, 1),
            learnset: vec![(1, 8), (1, 9), (5, 10)],
            attack_element: Element::Blunt,
        },
        Actor {
            id: 4,
            name: "Finn".into(),
            class_name: "Thief".into(),
            sprite: 3,
            base: stats(88, 16, 13, 9, 8, 8, 16),
            growth: stats(9, 2, 2, 1, 1, 1, 3),
            learnset: vec![(1, 11), (2, 12), (5, 13)],
            attack_element: Element::Pierce,
        },
    ]
}

fn default_skills() -> Vec<Skill> {
    let s = |id: u32, name: &str, element: Element, power: f32, mp: u32, target: SkillTarget, effect: SkillEffect, hits: u8, d: &str| Skill {
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
    vec![
        s(1, "Cleave", Element::Slash, 1.4, 4, SkillTarget::OneEnemy, SkillEffect::Damage, 1, "A heavy sword blow."),
        s(2, "Cross Strike", Element::Slash, 0.6, 6, SkillTarget::OneEnemy, SkillEffect::Damage, 2, "Two quick slashes."),
        s(3, "Warcry", Element::Blunt, 0.0, 5, SkillTarget::AllAllies, SkillEffect::BuffAttack(3), 1, "Raise the party's attack for 3 turns."),
        s(4, "Fireball", Element::Fire, 1.5, 6, SkillTarget::OneEnemy, SkillEffect::Damage, 1, "Hurl a ball of flame."),
        s(5, "Icicle", Element::Ice, 1.3, 5, SkillTarget::OneEnemy, SkillEffect::Damage, 1, "A spear of ice."),
        s(6, "Thunder", Element::Lightning, 1.2, 8, SkillTarget::AllEnemies, SkillEffect::Damage, 1, "Lightning strikes every foe."),
        s(7, "Inferno", Element::Fire, 1.9, 14, SkillTarget::AllEnemies, SkillEffect::Damage, 1, "Engulf all foes in fire."),
        s(8, "Mend", Element::Light, 1.6, 5, SkillTarget::OneAlly, SkillEffect::Heal, 1, "Restore one ally's HP."),
        s(9, "Ray", Element::Light, 1.2, 5, SkillTarget::OneEnemy, SkillEffect::Damage, 1, "A beam of holy light."),
        s(10, "Blessing", Element::Light, 1.0, 12, SkillTarget::AllAllies, SkillEffect::Heal, 1, "Restore the whole party's HP."),
        s(11, "Dagger Dance", Element::Pierce, 0.45, 4, SkillTarget::OneEnemy, SkillEffect::Damage, 3, "Three lightning-fast stabs."),
        s(12, "Armor Crush", Element::Blunt, 0.9, 6, SkillTarget::OneEnemy, SkillEffect::BreakDefense(3), 1, "Damage and lower defense for 3 turns."),
        s(13, "Shadow Fang", Element::Dark, 1.5, 8, SkillTarget::OneEnemy, SkillEffect::Damage, 1, "A strike from the shadows."),
        s(14, "Gnaw", Element::Pierce, 1.0, 0, SkillTarget::OneEnemy, SkillEffect::Damage, 1, "Bite."),
        s(15, "Screech", Element::Dark, 0.8, 0, SkillTarget::AllEnemies, SkillEffect::Damage, 1, "An ear-splitting cry."),
        s(16, "Boulder Fist", Element::Blunt, 1.6, 0, SkillTarget::OneEnemy, SkillEffect::Damage, 1, "A crushing stone fist."),
    ]
}

fn default_items() -> Vec<Item> {
    vec![
        Item { id: 1, name: "Healing Herb".into(), kind: ItemKind::HealHp, power: 60, description: "Restores 60 HP.".into() },
        Item { id: 2, name: "Mana Drop".into(), kind: ItemKind::HealMp, power: 25, description: "Restores 25 MP.".into() },
        Item { id: 3, name: "Phoenix Ash".into(), kind: ItemKind::Revive, power: 50, description: "Revives a fallen ally with 50 HP.".into() },
    ]
}

fn default_enemies() -> Vec<Enemy> {
    vec![
        Enemy {
            id: 1,
            name: "Meadow Slime".into(),
            sprite: 0,
            stats: stats(55, 0, 9, 6, 4, 4, 6),
            exp: 12,
            shields: 2,
            weaknesses: vec![Element::Slash, Element::Fire],
            skills: vec![],
        },
        Enemy {
            id: 2,
            name: "Cave Bat".into(),
            sprite: 1,
            stats: stats(42, 10, 11, 4, 8, 5, 14),
            exp: 15,
            shields: 2,
            weaknesses: vec![Element::Pierce, Element::Light],
            skills: vec![15],
        },
        Enemy {
            id: 3,
            name: "Mud Crawler".into(),
            sprite: 2,
            stats: stats(80, 0, 13, 10, 5, 6, 5),
            exp: 22,
            shields: 3,
            weaknesses: vec![Element::Ice, Element::Lightning],
            skills: vec![14],
        },
        Enemy {
            id: 4,
            name: "Stone Golem".into(),
            sprite: 3,
            stats: stats(320, 20, 18, 16, 10, 10, 4),
            exp: 120,
            shields: 5,
            weaknesses: vec![Element::Blunt, Element::Lightning, Element::Dark],
            skills: vec![16],
        },
    ]
}

fn default_troops() -> Vec<Troop> {
    vec![
        Troop { id: 1, name: "Slimes ×2".into(), members: vec![1, 1] },
        Troop { id: 2, name: "Slime & Bat".into(), members: vec![1, 2] },
        Troop { id: 3, name: "Cave Pack".into(), members: vec![2, 2, 3] },
        Troop { id: 4, name: "Stone Golem".into(), members: vec![4] },
    ]
}
