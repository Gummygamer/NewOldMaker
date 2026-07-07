//! Project data model: everything a game made with NewOldMaker consists of.
//! Serialized as a single pretty-printed JSON file (`*.nom.json`).

use serde::{Deserialize, Serialize};

pub use crate::core::i18n::{Language, ALL_LANGUAGES};

// ---------------------------------------------------------------------------
// Terrain / props (built-in sets backed by the procedural atlas)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[repr(u8)]
pub enum Terrain {
    Grass = 0,
    Dirt = 1,
    Stone = 2,
    Sand = 3,
    Water = 4,
    WoodFloor = 5,
    StoneBrick = 6,
    Snow = 7,
    CaveFloor = 8,
    Lava = 9,
}

pub const TERRAIN_COUNT: usize = 10;
pub const ALL_TERRAINS: [Terrain; TERRAIN_COUNT] = [
    Terrain::Grass,
    Terrain::Dirt,
    Terrain::Stone,
    Terrain::Sand,
    Terrain::Water,
    Terrain::WoodFloor,
    Terrain::StoneBrick,
    Terrain::Snow,
    Terrain::CaveFloor,
    Terrain::Lava,
];

impl Terrain {
    pub fn name(self) -> &'static str {
        match self {
            Terrain::Grass => "Grass",
            Terrain::Dirt => "Dirt",
            Terrain::Stone => "Stone",
            Terrain::Sand => "Sand",
            Terrain::Water => "Water",
            Terrain::WoodFloor => "Wood Floor",
            Terrain::StoneBrick => "Stone Brick",
            Terrain::Snow => "Snow",
            Terrain::CaveFloor => "Cave Floor",
            Terrain::Lava => "Lava",
        }
    }
    pub fn walkable(self) -> bool {
        !matches!(self, Terrain::Water | Terrain::Lava)
    }
    /// Liquid tiles render lowered and animated.
    pub fn liquid(self) -> bool {
        matches!(self, Terrain::Water | Terrain::Lava)
    }
    pub fn from_u8(v: u8) -> Terrain {
        ALL_TERRAINS[(v as usize).min(TERRAIN_COUNT - 1)]
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[repr(u8)]
pub enum Prop {
    None = 0,
    Tree = 1,
    Pine = 2,
    Rock = 3,
    Bush = 4,
    Flowers = 5,
    Torch = 6,
    Signpost = 7,
    Barrel = 8,
    Crystal = 9,
    Stump = 10,
    Cactus = 11,
}

pub const PROP_COUNT: usize = 12;
pub const ALL_PROPS: [Prop; PROP_COUNT] = [
    Prop::None,
    Prop::Tree,
    Prop::Pine,
    Prop::Rock,
    Prop::Bush,
    Prop::Flowers,
    Prop::Torch,
    Prop::Signpost,
    Prop::Barrel,
    Prop::Crystal,
    Prop::Stump,
    Prop::Cactus,
];

impl Prop {
    pub fn name(self) -> &'static str {
        match self {
            Prop::None => "Erase",
            Prop::Tree => "Tree",
            Prop::Pine => "Pine",
            Prop::Rock => "Rock",
            Prop::Bush => "Bush",
            Prop::Flowers => "Flowers",
            Prop::Torch => "Torch",
            Prop::Signpost => "Signpost",
            Prop::Barrel => "Barrel",
            Prop::Crystal => "Crystal",
            Prop::Stump => "Stump",
            Prop::Cactus => "Cactus",
        }
    }
    pub fn blocks(self) -> bool {
        !matches!(self, Prop::None | Prop::Flowers)
    }
    /// Emits a point light in the HD-2D scene.
    pub fn light(self) -> Option<[f32; 3]> {
        match self {
            Prop::Torch => Some([1.0, 0.55, 0.2]),
            Prop::Crystal => Some([0.3, 0.6, 1.0]),
            _ => None,
        }
    }
    pub fn from_u8(v: u8) -> Prop {
        ALL_PROPS[(v as usize).min(PROP_COUNT - 1)]
    }
}

// ---------------------------------------------------------------------------
// Map
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Tile {
    pub terrain: u8,
    pub height: u8,
    pub prop: u8,
}

impl Default for Tile {
    fn default() -> Self {
        Tile {
            terrain: Terrain::Grass as u8,
            height: 1,
            prop: 0,
        }
    }
}

pub const MAX_TILE_HEIGHT: u8 = 8;

/// Ambience knobs that give each map its HD-2D mood.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MapAmbience {
    pub sun_color: [f32; 3],
    pub ambient_color: [f32; 3],
    pub fog_color: [f32; 3],
    pub fog_density: f32,
    pub bloom_strength: f32,
    /// 0 = outdoor daylight, 1 = dark interior/cave (sun off, lights matter).
    pub darkness: f32,
}

impl Default for MapAmbience {
    fn default() -> Self {
        MapAmbience {
            sun_color: [1.0, 0.96, 0.88],
            ambient_color: [0.45, 0.5, 0.6],
            fog_color: [0.65, 0.75, 0.9],
            fog_density: 0.012,
            bloom_strength: 0.6,
            darkness: 0.0,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MapData {
    pub id: u32,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub tiles: Vec<Tile>,
    pub events: Vec<EventData>,
    pub ambience: MapAmbience,
    /// Troop ids that can be encountered here. Empty = no random battles.
    pub encounter_troops: Vec<u32>,
    /// Average steps between random encounters (0 = disabled).
    pub encounter_steps: u32,
}

impl MapData {
    pub fn new(id: u32, name: &str, width: u32, height: u32) -> Self {
        MapData {
            id,
            name: name.to_string(),
            width,
            height,
            tiles: vec![Tile::default(); (width * height) as usize],
            events: Vec::new(),
            ambience: MapAmbience::default(),
            encounter_troops: Vec::new(),
            encounter_steps: 0,
        }
    }
    pub fn in_bounds(&self, x: i32, y: i32) -> bool {
        x >= 0 && y >= 0 && (x as u32) < self.width && (y as u32) < self.height
    }
    pub fn tile(&self, x: i32, y: i32) -> Tile {
        if self.in_bounds(x, y) {
            self.tiles[(y as u32 * self.width + x as u32) as usize]
        } else {
            Tile {
                terrain: Terrain::Water as u8,
                height: 0,
                prop: 0,
            }
        }
    }
    pub fn tile_mut(&mut self, x: i32, y: i32) -> Option<&mut Tile> {
        if self.in_bounds(x, y) {
            let w = self.width;
            Some(&mut self.tiles[(y as u32 * w + x as u32) as usize])
        } else {
            None
        }
    }
    pub fn event_at(&self, x: i32, y: i32) -> Option<&EventData> {
        self.events.iter().find(|e| e.x == x && e.y == y)
    }
}

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EventData {
    pub id: u32,
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub kind: EventKind,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum EventKind {
    /// A talking NPC. Dialogue comes from the local LLM when available,
    /// otherwise cycles through the persona's fallback lines.
    Npc {
        sprite: u8,
        persona: NpcPersona,
        wander: bool,
    },
    /// A readable sign.
    Sign { text: String },
    /// Steps onto this tile move the player elsewhere.
    Transfer {
        target_map: u32,
        target_x: i32,
        target_y: i32,
    },
    /// One-shot item pickup.
    Chest { item_id: u32 },
    /// Touching this tile starts a fixed battle. `once` = disappears after victory.
    BattleTrigger { troop_id: u32, once: bool },
    /// Full-party heal when interacted with (inn/statue style).
    HealPoint,
}

impl EventKind {
    pub fn label(&self) -> &'static str {
        match self {
            EventKind::Npc { .. } => "NPC",
            EventKind::Sign { .. } => "Sign",
            EventKind::Transfer { .. } => "Transfer",
            EventKind::Chest { .. } => "Chest",
            EventKind::BattleTrigger { .. } => "Battle",
            EventKind::HealPoint => "Heal Point",
        }
    }
    pub fn blocks(&self) -> bool {
        !matches!(
            self,
            EventKind::Transfer { .. } | EventKind::BattleTrigger { .. }
        )
    }
}

/// Everything the local LLM needs to speak as this character.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NpcPersona {
    /// Displayed name, e.g. "Old Marta".
    pub name: String,
    /// One line: who they are. e.g. "the village apothecary".
    pub role: String,
    /// Free-text personality & speaking style.
    pub personality: String,
    /// World facts this NPC knows and may share.
    pub knowledge: String,
    /// Hard rules, e.g. "never reveals the cellar key location".
    pub constraints: String,
    /// Used verbatim when no LLM model is loaded (cycled in order).
    pub fallback_lines: Vec<String>,
    /// Allow the LLM to drive this NPC (else always fallback lines).
    pub use_llm: bool,
}

impl Default for NpcPersona {
    fn default() -> Self {
        NpcPersona {
            name: "Villager".into(),
            role: "a villager".into(),
            personality: "Friendly and a little nosy.".into(),
            knowledge: String::new(),
            constraints: String::new(),
            fallback_lines: vec!["Nice weather we're having.".into()],
            use_llm: true,
        }
    }
}

// ---------------------------------------------------------------------------
// Battle database
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Hash)]
pub enum Element {
    Slash,
    Pierce,
    Blunt,
    Fire,
    Ice,
    Lightning,
    Light,
    Dark,
}

pub const ALL_ELEMENTS: [Element; 8] = [
    Element::Slash,
    Element::Pierce,
    Element::Blunt,
    Element::Fire,
    Element::Ice,
    Element::Lightning,
    Element::Light,
    Element::Dark,
];

impl Element {
    pub fn name(self) -> &'static str {
        match self {
            Element::Slash => "Slash",
            Element::Pierce => "Pierce",
            Element::Blunt => "Blunt",
            Element::Fire => "Fire",
            Element::Ice => "Ice",
            Element::Lightning => "Lightning",
            Element::Light => "Light",
            Element::Dark => "Dark",
        }
    }
    pub fn icon(self) -> &'static str {
        match self {
            Element::Slash => "🗡",
            Element::Pierce => "🏹",
            Element::Blunt => "🔨",
            Element::Fire => "🔥",
            Element::Ice => "❄",
            Element::Lightning => "⚡",
            Element::Light => "✦",
            Element::Dark => "🌑",
        }
    }
    /// Physical elements scale with ATK/DEF; the rest with MAG/SPR.
    pub fn physical(self) -> bool {
        matches!(self, Element::Slash | Element::Pierce | Element::Blunt)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum SkillTarget {
    OneEnemy,
    AllEnemies,
    OneAlly,
    AllAllies,
    Own,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum SkillEffect {
    Damage,
    Heal,
    /// Raise target ATK/MAG for N turns.
    BuffAttack(u8),
    /// Lower target DEF/SPR for N turns.
    BreakDefense(u8),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Skill {
    pub id: u32,
    pub name: String,
    pub element: Element,
    /// Damage/heal multiplier relative to a basic attack (1.0 = basic).
    pub power: f32,
    pub mp_cost: u32,
    pub target: SkillTarget,
    pub effect: SkillEffect,
    /// Number of hits (each checks the shield once). Octopath-style multi-hits.
    pub hits: u8,
    pub description: String,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, Default)]
pub struct Stats {
    pub hp: i32,
    pub mp: i32,
    pub atk: i32,
    pub def: i32,
    pub mag: i32,
    pub spr: i32,
    pub spd: i32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Actor {
    pub id: u32,
    pub name: String,
    pub class_name: String,
    /// Index into the generated character sprite sheets.
    pub sprite: u8,
    pub base: Stats,
    /// Added per level-up.
    pub growth: Stats,
    /// (level, skill_id) — learned when reaching that level.
    pub learnset: Vec<(u32, u32)>,
    /// Element of the basic attack.
    pub attack_element: Element,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Enemy {
    pub id: u32,
    pub name: String,
    /// Index into the generated enemy sprite set.
    pub sprite: u8,
    pub stats: Stats,
    pub exp: u32,
    /// Shield points (Octopath-style). 0 = cannot be broken.
    pub shields: u8,
    pub weaknesses: Vec<Element>,
    /// Skills it may use (basic attack always available).
    pub skills: Vec<u32>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Troop {
    pub id: u32,
    pub name: String,
    /// 1..=4 enemy ids.
    pub members: Vec<u32>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum ItemKind {
    HealHp,
    HealMp,
    Revive,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Item {
    pub id: u32,
    pub name: String,
    pub kind: ItemKind,
    pub power: i32,
    pub description: String,
}

// ---------------------------------------------------------------------------
// System / LLM settings / project root
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SystemData {
    pub title: String,
    pub start_map: u32,
    pub start_x: i32,
    pub start_y: i32,
    /// Actor ids in the starting party (max 4).
    pub party: Vec<u32>,
    pub start_items: Vec<(u32, u32)>,
    /// Language the game is played in (in-game text + NPC dialogue).
    #[serde(default)]
    pub language: Language,
}

/// Which engine drives NPC dialogue.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum LlmBackend {
    /// Local llama.cpp model loaded from a GGUF file (fully offline).
    #[default]
    Local,
    /// NVIDIA NIM cloud endpoint (OpenAI-compatible), driven by an API key.
    Nim,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LlmSettings {
    /// Which backend serves NPC dialogue.
    #[serde(default)]
    pub backend: LlmBackend,
    /// Path to a GGUF model file (local backend). Empty = LLM disabled.
    pub model_path: String,
    pub context_tokens: u32,
    pub max_reply_tokens: u32,
    pub temperature: f32,
    /// Number of CPU threads (0 = auto), local backend only.
    pub threads: u32,
    /// NVIDIA API key for the NIM backend. Empty = fall back to the
    /// `NVIDIA_API_KEY` environment variable.
    #[serde(default)]
    pub nim_api_key: String,
    /// NIM model id, e.g. "meta/llama-3.1-8b-instruct".
    #[serde(default = "default_nim_model")]
    pub nim_model: String,
    /// NIM OpenAI-compatible base URL (no trailing slash).
    #[serde(default = "default_nim_base_url")]
    pub nim_base_url: String,
}

fn default_nim_model() -> String {
    "meta/llama-3.1-8b-instruct".into()
}

fn default_nim_base_url() -> String {
    "https://integrate.api.nvidia.com/v1".into()
}

impl Default for LlmSettings {
    fn default() -> Self {
        LlmSettings {
            backend: LlmBackend::Local,
            model_path: String::new(),
            context_tokens: 2048,
            max_reply_tokens: 96,
            temperature: 0.8,
            threads: 0,
            nim_api_key: String::new(),
            nim_model: default_nim_model(),
            nim_base_url: default_nim_base_url(),
        }
    }
}

impl LlmSettings {
    /// A signature that changes whenever the active worker must be rebuilt.
    /// Switching backend, model, endpoint, or key reloads; per-chat knobs
    /// (temperature, reply length) are carried on each request instead.
    pub fn worker_signature(&self) -> String {
        match self.backend {
            LlmBackend::Local => format!("local\u{1}{}", self.model_path),
            LlmBackend::Nim => format!(
                "nim\u{1}{}\u{1}{}\u{1}{}",
                self.nim_base_url, self.nim_model, self.nim_api_key
            ),
        }
    }

    /// Whether the active backend has enough configuration to run at all.
    pub fn is_configured(&self) -> bool {
        match self.backend {
            LlmBackend::Local => !self.model_path.is_empty(),
            LlmBackend::Nim => {
                !self.nim_model.is_empty()
                    && (!self.nim_api_key.is_empty() || std::env::var("NVIDIA_API_KEY").is_ok())
            }
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProjectData {
    pub format_version: u32,
    pub name: String,
    pub maps: Vec<MapData>,
    pub actors: Vec<Actor>,
    pub skills: Vec<Skill>,
    pub items: Vec<Item>,
    pub enemies: Vec<Enemy>,
    pub troops: Vec<Troop>,
    pub system: SystemData,
    pub llm: LlmSettings,
}

pub const FORMAT_VERSION: u32 = 1;

impl ProjectData {
    pub fn map(&self, id: u32) -> Option<&MapData> {
        self.maps.iter().find(|m| m.id == id)
    }
    pub fn map_mut(&mut self, id: u32) -> Option<&mut MapData> {
        self.maps.iter_mut().find(|m| m.id == id)
    }
    pub fn actor(&self, id: u32) -> Option<&Actor> {
        self.actors.iter().find(|a| a.id == id)
    }
    pub fn skill(&self, id: u32) -> Option<&Skill> {
        self.skills.iter().find(|s| s.id == id)
    }
    pub fn item(&self, id: u32) -> Option<&Item> {
        self.items.iter().find(|i| i.id == id)
    }
    pub fn enemy(&self, id: u32) -> Option<&Enemy> {
        self.enemies.iter().find(|e| e.id == id)
    }
    pub fn troop(&self, id: u32) -> Option<&Troop> {
        self.troops.iter().find(|t| t.id == id)
    }
    pub fn next_map_id(&self) -> u32 {
        self.maps.iter().map(|m| m.id).max().unwrap_or(0) + 1
    }
    pub fn next_event_id(map: &MapData) -> u32 {
        map.events.iter().map(|e| e.id).max().unwrap_or(0) + 1
    }
}
