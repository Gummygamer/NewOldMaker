//! LLM-assisted content generation: turn a short designer request into ready-made
//! database elements (heroes, monsters, skills, troops, items) or a whole map,
//! using whichever backend is configured under Database → LLM.
//!
//! The model is asked for strict JSON following a fixed schema; this module
//! builds those prompts, extracts the JSON from the (often chatty) reply, and
//! parses it *tolerantly* — missing fields fall back to sensible defaults,
//! unknown enum spellings are normalized, and cross-references (a hero's
//! learnset, an enemy's skills, a troop's members) are filtered down to ids that
//! actually exist so generated content is never dangling. Maps come back as a
//! compact fixed-legend ASCII grid that decodes into tiles.

use serde_json::Value;

use super::data::*;

/// What the designer asked the LLM to create.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GenTarget {
    Heroes,
    Monsters,
    Skills,
    Troops,
    Items,
    Map,
}

impl GenTarget {
    pub const ALL: [GenTarget; 6] = [
        GenTarget::Heroes,
        GenTarget::Monsters,
        GenTarget::Skills,
        GenTarget::Troops,
        GenTarget::Items,
        GenTarget::Map,
    ];

    pub fn label(self) -> &'static str {
        match self {
            GenTarget::Heroes => "Heroes",
            GenTarget::Monsters => "Monsters",
            GenTarget::Skills => "Skills",
            GenTarget::Troops => "Troops",
            GenTarget::Items => "Items",
            GenTarget::Map => "Map",
        }
    }

    pub fn is_map(self) -> bool {
        matches!(self, GenTarget::Map)
    }

    /// A rough per-request reply budget (tokens); maps and multiple elements
    /// need more room than a single item.
    pub fn max_tokens(self, count: u32) -> u32 {
        match self {
            GenTarget::Map => 1600,
            _ => (280 + count * 220).clamp(400, 1800),
        }
    }
}

/// The result of splicing generated content into the project.
pub struct GenApplied {
    /// A short human-readable summary to show the designer.
    pub summary: String,
    /// Set when a new map was added, so the editor can switch to it.
    pub new_map: Option<u32>,
}

// ---------------------------------------------------------------------------
// Prompts
// ---------------------------------------------------------------------------

/// Build the system prompt (schema + rules) for a generation target.
pub fn system_prompt(target: GenTarget, project: &ProjectData, language: Language) -> String {
    let mut s = String::from(
        "You are a game-design assistant for an Octopath-Traveler-style turn-based RPG maker. \
         You reply with STRICT JSON only — no prose, no explanation, no markdown fences. \
         Output must be parseable by a machine.\n\n",
    );
    s.push_str(&match target {
        GenTarget::Skills => skills_schema(),
        GenTarget::Monsters => enemies_schema(project),
        GenTarget::Heroes => actors_schema(project),
        GenTarget::Troops => troops_schema(project),
        GenTarget::Items => items_schema(),
        GenTarget::Map => map_schema(),
    });
    // Names and descriptions in the game's language; keep enum values English.
    if let Some(instr) = language.llm_instruction() {
        s.push_str(
            "\n\nWrite the \"name\", \"class_name\" and \"description\" text in the game's \
             language. ",
        );
        s.push_str(instr);
        s.push_str(
            " Keep every JSON key, every enum value (elements, targets, effects, kinds) and every \
             map legend character EXACTLY as written here, in English.",
        );
    }
    s
}

/// Build the user prompt: the designer's request plus how much to make.
pub fn user_prompt(target: GenTarget, count: u32, request: &str) -> String {
    let request = request.trim();
    let theme = if request.is_empty() {
        "Fit the classic fantasy-RPG theme.".to_string()
    } else {
        format!("Designer's request: {request}")
    };
    match target {
        GenTarget::Map => {
            format!("Design ONE map. {theme}\nReturn only the JSON object described above.")
        }
        _ => {
            let noun = target.label().to_lowercase();
            format!(
                "Generate {count} {noun}. {theme}\nMake them varied and balanced. \
                 Return only the JSON array described above."
            )
        }
    }
}

const ELEMENTS: &str = "Slash, Pierce, Blunt, Fire, Ice, Lightning, Light, Dark";

fn skills_schema() -> String {
    format!(
        "Produce a JSON array of skill objects. Each object has:\n\
         - \"name\": string\n\
         - \"element\": one of [{ELEMENTS}]\n\
         - \"power\": number, damage/heal multiplier vs a basic attack (1.0 = basic, \
           1.2-2.0 for strong hits, 0 for pure buffs)\n\
         - \"mp_cost\": integer 0-40\n\
         - \"target\": one of [OneEnemy, AllEnemies, OneAlly, AllAllies, Own]\n\
         - \"effect\": one of [Damage, Heal, BuffAttack, BreakDefense]\n\
         - \"effect_turns\": integer 1-5 (only for BuffAttack/BreakDefense)\n\
         - \"hits\": integer 1-8 (multi-hit skills)\n\
         - \"description\": one short sentence\n\
         Example: [{{\"name\":\"Frost Nova\",\"element\":\"Ice\",\"power\":1.4,\"mp_cost\":9,\
         \"target\":\"AllEnemies\",\"effect\":\"Damage\",\"effect_turns\":0,\"hits\":1,\
         \"description\":\"A burst of biting frost.\"}}]"
    )
}

fn enemies_schema(project: &ProjectData) -> String {
    format!(
        "Produce a JSON array of enemy objects. Each object has:\n\
         - \"name\": string\n\
         - \"sprite\": integer 0-3\n\
         - \"hp\",\"mp\",\"atk\",\"def\",\"mag\",\"spr\",\"spd\": integer stats \
           (weak foes ~40-80 HP, bosses 250+)\n\
         - \"exp\": integer reward\n\
         - \"shields\": integer 1-8 (shield points that must be broken)\n\
         - \"weaknesses\": array of 1-3 elements from [{ELEMENTS}]\n\
         - \"skill_ids\": array of skill ids this enemy may cast (may be empty)\n\
         {}\n\
         Example: [{{\"name\":\"Frost Wolf\",\"sprite\":1,\"hp\":70,\"mp\":10,\"atk\":14,\
         \"def\":8,\"mag\":6,\"spr\":6,\"spd\":13,\"exp\":24,\"shields\":2,\
         \"weaknesses\":[\"Fire\",\"Slash\"],\"skill_ids\":[]}}]",
        skill_reference(project)
    )
}

fn actors_schema(project: &ProjectData) -> String {
    format!(
        "Produce a JSON array of playable hero objects. Each object has:\n\
         - \"name\": string\n\
         - \"class_name\": string (e.g. Knight, Mage, Cleric)\n\
         - \"sprite\": integer 0-7\n\
         - \"attack_element\": one of [{ELEMENTS}]\n\
         - \"base\": object of Lv.1 stats {{\"hp\",\"mp\",\"atk\",\"def\",\"mag\",\"spr\",\"spd\"}} \
           (fighters ~110 HP, mages ~75 HP with high mag)\n\
         - \"growth\": object of the same stats gained each level (smaller numbers)\n\
         - \"learnset\": array of {{\"level\":int, \"skill_id\":int}} using existing skill ids\n\
         {}\n\
         Example: [{{\"name\":\"Kael\",\"class_name\":\"Knight\",\"sprite\":0,\
         \"attack_element\":\"Slash\",\"base\":{{\"hp\":118,\"mp\":12,\"atk\":16,\"def\":14,\
         \"mag\":6,\"spr\":8,\"spd\":9}},\"growth\":{{\"hp\":14,\"mp\":2,\"atk\":3,\"def\":3,\
         \"mag\":1,\"spr\":1,\"spd\":1}},\"learnset\":[{{\"level\":1,\"skill_id\":1}}]}}]",
        skill_reference(project)
    )
}

fn troops_schema(project: &ProjectData) -> String {
    format!(
        "Produce a JSON array of troop (enemy group) objects. Each object has:\n\
         - \"name\": string\n\
         - \"member_ids\": array of 1-4 enemy ids drawn from the existing enemies below\n\
         {}\n\
         Example: [{{\"name\":\"Wolf Pack\",\"member_ids\":[2,2,3]}}]",
        enemy_reference(project)
    )
}

fn items_schema() -> String {
    "Produce a JSON array of consumable item objects. Each object has:\n\
     - \"name\": string\n\
     - \"kind\": one of [HealHp, HealMp, Revive]\n\
     - \"power\": integer (HP or MP restored, or revive HP)\n\
     - \"description\": one short sentence\n\
     Example: [{\"name\":\"Grand Elixir\",\"kind\":\"HealHp\",\"power\":150,\
     \"description\":\"Restores 150 HP.\"}]"
        .to_string()
}

fn map_schema() -> String {
    "Produce ONE JSON object describing a map painted as ASCII grids. Fields:\n\
     - \"name\": string\n\
     - \"width\": integer 12-40\n\
     - \"height\": integer 12-40\n\
     - \"terrain\": array of \"height\" strings, each \"width\" characters wide, using the \
       TERRAIN legend below\n\
     - \"props\": array of the same size using the PROP legend ('.' = nothing) — optional\n\
     - \"heights\": array of the same size of digits 0-8 for tile elevation — optional\n\
     - \"ambience\": optional {\"darkness\":0-1, \"fog_density\":0-0.12, \"bloom\":0-2}\n\
     - \"encounter_steps\": optional integer (0 = no random battles, ~12 typical)\n\n\
     TERRAIN legend: '.'=Grass ','=Dirt 'o'=Stone 's'=Sand '~'=Water '='=WoodFloor \
     '#'=StoneBrick '^'=Snow 'c'=CaveFloor '!'=Lava\n\
     PROP legend: '.'=none 'T'=Tree 'P'=Pine 'R'=Rock 'H'=Bush 'F'=Flowers 'L'=Torch \
     'G'=Signpost 'K'=Barrel 'Y'=Crystal 'U'=Stump 'X'=Cactus\n\
     Make the layout readable: paths, a clearing or building, water or cliffs. For a cave \
     use 'o'/'c' terrain with high darkness and 'Y' crystals or 'L' torches for light.\n\
     Example (tiny): {\"name\":\"Quiet Glade\",\"width\":12,\"height\":6,\
     \"terrain\":[\"............\",\"...~~~~.....\",\"...~~~~.....\",\"............\",\
     \"...====.....\",\"............\"],\
     \"props\":[\"T..........T\",\"............\",\"............\",\".....F......\",\
     \"............\",\"T..........T\"],\"encounter_steps\":12}"
        .to_string()
}

/// A compact "id=Name" listing of existing skills for cross-references.
fn skill_reference(project: &ProjectData) -> String {
    if project.skills.is_empty() {
        return "There are no existing skills yet, so use an empty \"skill_ids\"/\"learnset\"."
            .to_string();
    }
    let list = project
        .skills
        .iter()
        .map(|s| format!("{}={}", s.id, s.name))
        .collect::<Vec<_>>()
        .join(", ");
    format!("Existing skills you may reference by id: {list}.")
}

/// A compact "id=Name" listing of existing enemies for troop members.
fn enemy_reference(project: &ProjectData) -> String {
    if project.enemies.is_empty() {
        return "There are no enemies yet; generate some Monsters first.".to_string();
    }
    let list = project
        .enemies
        .iter()
        .map(|e| format!("{}={}", e.id, e.name))
        .collect::<Vec<_>>()
        .join(", ");
    format!("Existing enemies (id=name): {list}.")
}

// ---------------------------------------------------------------------------
// Apply: parse the model output and splice it into the project
// ---------------------------------------------------------------------------

/// Parse the model's raw reply and add the generated content to `project`.
pub fn apply(
    target: GenTarget,
    project: &mut ProjectData,
    raw: &str,
) -> Result<GenApplied, String> {
    let json = extract_json(raw)
        .ok_or_else(|| "The model did not return any JSON. Try again or rephrase.".to_string())?;
    let value: Value =
        serde_json::from_str(&json).map_err(|e| format!("Could not parse the model's JSON: {e}"))?;
    match target {
        GenTarget::Skills => apply_skills(project, &value),
        GenTarget::Monsters => apply_enemies(project, &value),
        GenTarget::Heroes => apply_actors(project, &value),
        GenTarget::Troops => apply_troops(project, &value),
        GenTarget::Items => apply_items(project, &value),
        GenTarget::Map => apply_map(project, &value),
    }
}

fn apply_skills(project: &mut ProjectData, v: &Value) -> Result<GenApplied, String> {
    let items = as_items(v);
    if items.is_empty() {
        return Err("No skills found in the model output.".into());
    }
    let base = project.skills.iter().map(|s| s.id).max().unwrap_or(0) + 1;
    let mut names = Vec::new();
    for (offset, it) in items.iter().enumerate() {
        let id = base + offset as u32;
        let element = parse_element(&text(it, &["element"], "")).unwrap_or(Element::Slash);
        let turns = inum(it, &["effect_turns", "turns", "duration"], 3).clamp(1, 9) as u8;
        let effect = match parse_effect(&text(it, &["effect"], "damage")) {
            EffKind::Damage => SkillEffect::Damage,
            EffKind::Heal => SkillEffect::Heal,
            EffKind::Buff => SkillEffect::BuffAttack(turns),
            EffKind::Break => SkillEffect::BreakDefense(turns),
        };
        let name = text(it, &["name"], &format!("Skill {id}"));
        project.skills.push(Skill {
            id,
            name: name.clone(),
            element,
            power: fnum(it, &["power", "power_multiplier", "multiplier"], 1.2).clamp(0.0, 8.0) as f32,
            mp_cost: inum(it, &["mp_cost", "mp", "cost"], 5).clamp(0, 99) as u32,
            target: parse_target(&text(it, &["target"], "")).unwrap_or(SkillTarget::OneEnemy),
            effect,
            hits: inum(it, &["hits", "num_hits"], 1).clamp(1, 8) as u8,
            description: text(it, &["description", "desc"], ""),
        });
        names.push(name);
    }
    Ok(GenApplied {
        summary: added_summary("skill", &names),
        new_map: None,
    })
}

fn apply_enemies(project: &mut ProjectData, v: &Value) -> Result<GenApplied, String> {
    let items = as_items(v);
    if items.is_empty() {
        return Err("No enemies found in the model output.".into());
    }
    let skill_ids: Vec<u32> = project.skills.iter().map(|s| s.id).collect();
    let base = project.enemies.iter().map(|e| e.id).max().unwrap_or(0) + 1;
    let mut names = Vec::new();
    for (offset, it) in items.iter().enumerate() {
        let id = base + offset as u32;
        let stats = read_stats(it, stats7(60, 0, 11, 8, 6, 6, 8));
        let name = text(it, &["name"], &format!("Enemy {id}"));
        project.enemies.push(Enemy {
            id,
            name: name.clone(),
            sprite: (inum(it, &["sprite"], 0).rem_euclid(4)) as u8,
            stats,
            exp: inum(it, &["exp", "experience"], 15).clamp(0, 99999) as u32,
            shields: inum(it, &["shields", "shield"], 2).clamp(0, 15) as u8,
            weaknesses: parse_elements(get(it, &["weaknesses", "weakness"])),
            skills: parse_id_list(get(it, &["skill_ids", "skills"]), &skill_ids),
        });
        names.push(name);
    }
    Ok(GenApplied {
        summary: added_summary("enemy", &names),
        new_map: None,
    })
}

fn apply_actors(project: &mut ProjectData, v: &Value) -> Result<GenApplied, String> {
    let items = as_items(v);
    if items.is_empty() {
        return Err("No heroes found in the model output.".into());
    }
    let skill_ids: Vec<u32> = project.skills.iter().map(|s| s.id).collect();
    let first_id = project.actors.iter().map(|a| a.id).max().unwrap_or(0) + 1;
    let mut names = Vec::new();
    for (offset, it) in items.iter().enumerate() {
        let id = first_id + offset as u32;
        // base/growth may be nested objects, or stats spread on the top object.
        let base = read_stats(
            get(it, &["base", "base_stats", "stats"]).unwrap_or(it),
            stats7(95, 20, 12, 11, 10, 10, 10),
        );
        let growth = read_stats(
            get(it, &["growth", "growth_stats"]).unwrap_or(&Value::Null),
            stats7(11, 3, 2, 2, 2, 2, 1),
        );
        let name = text(it, &["name"], &format!("Hero {id}"));
        project.actors.push(Actor {
            id,
            name: name.clone(),
            class_name: text(it, &["class_name", "class", "job"], "Adventurer"),
            sprite: (inum(it, &["sprite"], (id % 8) as i32).rem_euclid(8)) as u8,
            base,
            growth,
            learnset: parse_learnset(get(it, &["learnset", "skills"]), &skill_ids),
            attack_element: parse_element(&text(it, &["attack_element", "element"], ""))
                .unwrap_or(Element::Slash),
        });
        names.push(name);
    }
    Ok(GenApplied {
        summary: added_summary("hero", &names),
        new_map: None,
    })
}

fn apply_troops(project: &mut ProjectData, v: &Value) -> Result<GenApplied, String> {
    let items = as_items(v);
    if items.is_empty() {
        return Err("No troops found in the model output.".into());
    }
    if project.enemies.is_empty() {
        return Err("There are no enemies to build troops from — generate Monsters first.".into());
    }
    let enemy_ids: Vec<u32> = project.enemies.iter().map(|e| e.id).collect();
    let mut next = project.troops.iter().map(|t| t.id).max().unwrap_or(0) + 1;
    let mut names = Vec::new();
    for it in &items {
        let mut members = parse_id_list(get(it, &["member_ids", "members", "enemies"]), &enemy_ids);
        members.truncate(4);
        if members.is_empty() {
            // Skip troops that reference no valid enemy rather than add an empty one.
            continue;
        }
        let name = text(it, &["name"], &format!("Troop {next}"));
        project.troops.push(Troop {
            id: next,
            name: name.clone(),
            members,
        });
        names.push(name);
        next += 1;
    }
    if names.is_empty() {
        return Err("None of the generated troops referenced a valid enemy id.".into());
    }
    Ok(GenApplied {
        summary: added_summary("troop", &names),
        new_map: None,
    })
}

fn apply_items(project: &mut ProjectData, v: &Value) -> Result<GenApplied, String> {
    let items = as_items(v);
    if items.is_empty() {
        return Err("No items found in the model output.".into());
    }
    let base = project.items.iter().map(|i| i.id).max().unwrap_or(0) + 1;
    let mut names = Vec::new();
    for (offset, it) in items.iter().enumerate() {
        let id = base + offset as u32;
        let name = text(it, &["name"], &format!("Item {id}"));
        project.items.push(Item {
            id,
            name: name.clone(),
            kind: parse_kind(&text(it, &["kind", "type"], "")),
            power: inum(it, &["power", "amount"], 50).clamp(0, 9999),
            description: text(it, &["description", "desc"], ""),
        });
        names.push(name);
    }
    Ok(GenApplied {
        summary: added_summary("item", &names),
        new_map: None,
    })
}

fn apply_map(project: &mut ProjectData, v: &Value) -> Result<GenApplied, String> {
    let id = project.next_map_id();
    let m = decode_map(id, v)?;
    let (name, w, h) = (m.name.clone(), m.width, m.height);
    project.maps.push(m);
    Ok(GenApplied {
        summary: format!("Added map \u{201c}{name}\u{201d} ({w}\u{00d7}{h})."),
        new_map: Some(id),
    })
}

// ---------------------------------------------------------------------------
// Map decoding
// ---------------------------------------------------------------------------

fn terrain_from_char(c: char) -> Option<Terrain> {
    Some(match c {
        '.' => Terrain::Grass,
        ',' => Terrain::Dirt,
        'o' => Terrain::Stone,
        's' => Terrain::Sand,
        '~' => Terrain::Water,
        '=' => Terrain::WoodFloor,
        '#' => Terrain::StoneBrick,
        '^' => Terrain::Snow,
        'c' => Terrain::CaveFloor,
        '!' => Terrain::Lava,
        _ => return None,
    })
}

fn prop_from_char(c: char) -> Option<Prop> {
    Some(match c {
        'T' => Prop::Tree,
        'P' => Prop::Pine,
        'R' => Prop::Rock,
        'H' => Prop::Bush,
        'F' => Prop::Flowers,
        'L' => Prop::Torch,
        'G' => Prop::Signpost,
        'K' => Prop::Barrel,
        'Y' => Prop::Crystal,
        'U' => Prop::Stump,
        'X' => Prop::Cactus,
        _ => return None,
    })
}

fn decode_map(id: u32, v: &Value) -> Result<MapData, String> {
    let name = text(v, &["name", "title"], "Generated Map");
    let terrain_rows = str_rows(get(v, &["terrain", "tiles", "ground"]));
    let prop_rows = str_rows(get(v, &["props", "objects", "decor"]));
    let height_rows = str_rows(get(v, &["heights", "elevation"]));

    let mut width = inum(v, &["width", "w", "cols"], 0);
    let mut height = inum(v, &["height", "h", "rows"], 0);
    if width <= 0 {
        width = terrain_rows
            .iter()
            .map(|r| r.chars().count())
            .max()
            .unwrap_or(20) as i32;
    }
    if height <= 0 {
        height = terrain_rows.len().max(1) as i32;
    }
    let width = width.clamp(4, 48) as u32;
    let height = height.clamp(4, 48) as u32;
    if terrain_rows.is_empty() {
        return Err("The map had no terrain rows.".into());
    }

    let mut m = MapData::new(id, &name, width, height);
    for y in 0..height as i32 {
        let trow: Vec<char> = row_chars(&terrain_rows, y);
        let prow: Vec<char> = row_chars(&prop_rows, y);
        let hrow: Vec<char> = row_chars(&height_rows, y);
        for x in 0..width as i32 {
            let terrain = trow
                .get(x as usize)
                .and_then(|c| terrain_from_char(*c))
                .unwrap_or(Terrain::Grass);
            let t = m.tile_mut(x, y).unwrap();
            t.terrain = terrain as u8;
            t.height = hrow
                .get(x as usize)
                .and_then(|c| c.to_digit(10))
                .map(|d| (d as u8).min(MAX_TILE_HEIGHT))
                .unwrap_or(1);
            if terrain.liquid() {
                t.height = 0;
                t.prop = 0;
            } else if let Some(p) = prow.get(x as usize).and_then(|c| prop_from_char(*c)) {
                t.prop = p as u8;
            }
        }
    }

    if let Some(a) = v.get("ambience").filter(|a| a.is_object()) {
        let amb = &mut m.ambience;
        amb.darkness = fnum(a, &["darkness"], amb.darkness as f64).clamp(0.0, 1.0) as f32;
        amb.fog_density = fnum(a, &["fog_density", "fog"], amb.fog_density as f64).clamp(0.0, 0.2)
            as f32;
        amb.bloom_strength =
            fnum(a, &["bloom", "bloom_strength"], amb.bloom_strength as f64).clamp(0.0, 3.0) as f32;
        if let Some(c) = color(a, &["sun_color", "sun"]) {
            amb.sun_color = c;
        }
        if let Some(c) = color(a, &["ambient_color", "ambient"]) {
            amb.ambient_color = c;
        }
        if let Some(c) = color(a, &["fog_color"]) {
            amb.fog_color = c;
        }
    }
    m.encounter_steps = inum(v, &["encounter_steps", "encounter_rate"], 0).clamp(0, 240) as u32;
    Ok(m)
}

fn row_chars(rows: &[String], y: i32) -> Vec<char> {
    rows.get(y as usize)
        .map(|s| s.chars().collect())
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// JSON extraction & tolerant field helpers
// ---------------------------------------------------------------------------

/// Pull the JSON body out of a raw model reply: drop any `<think>` reasoning,
/// unwrap a ``` code fence if present, then return the first balanced
/// object/array.
pub fn extract_json(raw: &str) -> Option<String> {
    let mut s = raw.to_string();
    // Reasoning models emit a <think>…</think> preamble; keep only what follows.
    if let Some(pos) = s.to_ascii_lowercase().rfind("</think>") {
        s = s[pos + "</think>".len()..].to_string();
    }
    if let Some(inner) = fenced(&s) {
        s = inner;
    }
    slice_balanced(&s)
}

/// If the text contains a ``` code fence, return the content between the first
/// pair of fences (skipping an optional language tag on the opening line).
fn fenced(s: &str) -> Option<String> {
    let start = s.find("```")?;
    let after = &s[start + 3..];
    // Skip an optional language tag up to the end of the opening line.
    let body_start = after.find('\n').map(|nl| nl + 1).unwrap_or(0);
    let body = &after[body_start..];
    let end = body.find("```").unwrap_or(body.len());
    Some(body[..end].to_string())
}

/// From the first `{` or `[`, return the substring up to its matching close,
/// respecting string literals and escapes.
fn slice_balanced(s: &str) -> Option<String> {
    let start = s.find(['{', '['])?;
    let open = s.as_bytes()[start] as char;
    let close = if open == '{' { '}' } else { ']' };
    let mut depth = 0i32;
    let mut in_str = false;
    let mut esc = false;
    for (i, c) in s.char_indices().filter(|(i, _)| *i >= start) {
        if in_str {
            if esc {
                esc = false;
            } else if c == '\\' {
                esc = true;
            } else if c == '"' {
                in_str = false;
            }
            continue;
        }
        match c {
            '"' => in_str = true,
            ch if ch == open => depth += 1,
            ch if ch == close => {
                depth -= 1;
                if depth == 0 {
                    return Some(s[start..=i].to_string());
                }
            }
            _ => {}
        }
    }
    None
}

/// The array of elements to parse: the value if it's an array, else the first
/// array-valued field of an object, else the object itself as a single item.
fn as_items(v: &Value) -> Vec<Value> {
    if let Some(a) = v.as_array() {
        return a.clone();
    }
    if let Some(o) = v.as_object() {
        for val in o.values() {
            if let Some(a) = val.as_array() {
                return a.clone();
            }
        }
        return vec![v.clone()];
    }
    vec![]
}

/// First present, non-null field among `keys`.
fn get<'a>(v: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    for k in keys {
        if let Some(x) = v.get(k)
            && !x.is_null()
        {
            return Some(x);
        }
    }
    None
}

/// A number field (accepts numeric strings too).
fn fnum(v: &Value, keys: &[&str], default: f64) -> f64 {
    get(v, keys)
        .and_then(|x| {
            x.as_f64()
                .or_else(|| x.as_str().and_then(|s| s.trim().parse().ok()))
        })
        .unwrap_or(default)
}

fn inum(v: &Value, keys: &[&str], default: i32) -> i32 {
    fnum(v, keys, default as f64).round() as i32
}

/// A string field, trimmed; falls back to `default` when missing/empty.
fn text(v: &Value, keys: &[&str], default: &str) -> String {
    get(v, keys)
        .and_then(|x| x.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| default.to_string())
}

fn stats7(hp: i32, mp: i32, atk: i32, def: i32, mag: i32, spr: i32, spd: i32) -> Stats {
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

fn read_stats(v: &Value, def: Stats) -> Stats {
    Stats {
        hp: inum(v, &["hp", "health"], def.hp),
        mp: inum(v, &["mp", "mana"], def.mp),
        atk: inum(v, &["atk", "attack"], def.atk),
        def: inum(v, &["def", "defense"], def.def),
        mag: inum(v, &["mag", "magic"], def.mag),
        spr: inum(v, &["spr", "spirit"], def.spr),
        spd: inum(v, &["spd", "speed"], def.spd),
    }
}

fn color(v: &Value, keys: &[&str]) -> Option<[f32; 3]> {
    let arr = get(v, keys)?.as_array()?;
    if arr.len() < 3 {
        return None;
    }
    Some([
        arr[0].as_f64().unwrap_or(1.0) as f32,
        arr[1].as_f64().unwrap_or(1.0) as f32,
        arr[2].as_f64().unwrap_or(1.0) as f32,
    ])
}

/// Read an array of row strings (each grid line) from a JSON value.
fn str_rows(v: Option<&Value>) -> Vec<String> {
    v.and_then(|x| x.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|r| r.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default()
}

/// Parse a list of ids, keeping only those that appear in `valid`.
fn parse_id_list(v: Option<&Value>, valid: &[u32]) -> Vec<u32> {
    let Some(arr) = v.and_then(|x| x.as_array()) else {
        return Vec::new();
    };
    arr.iter()
        .filter_map(|x| {
            let id = x
                .as_u64()
                .map(|n| n as u32)
                .or_else(|| x.as_str().and_then(|s| s.trim().parse().ok()))?;
            valid.contains(&id).then_some(id)
        })
        .collect()
}

/// Parse a learnset: `[{level, skill_id}]`, dropping unknown skill ids.
fn parse_learnset(v: Option<&Value>, valid: &[u32]) -> Vec<(u32, u32)> {
    let Some(arr) = v.and_then(|x| x.as_array()) else {
        return Vec::new();
    };
    arr.iter()
        .filter_map(|entry| {
            let level = inum(entry, &["level", "lv"], 1).clamp(1, 99) as u32;
            let skill = inum(entry, &["skill_id", "skill", "id"], 0) as u32;
            valid.contains(&skill).then_some((level, skill))
        })
        .collect()
}

fn parse_elements(v: Option<&Value>) -> Vec<Element> {
    let Some(arr) = v.and_then(|x| x.as_array()) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for x in arr {
        if let Some(e) = x.as_str().and_then(parse_element)
            && !out.contains(&e)
        {
            out.push(e);
        }
    }
    out
}

/// Normalize a string to lowercase alphanumerics for tolerant enum matching.
fn norm(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

fn parse_element(s: &str) -> Option<Element> {
    Some(match norm(s).as_str() {
        "slash" | "cut" | "sword" => Element::Slash,
        "pierce" | "stab" | "thrust" => Element::Pierce,
        "blunt" | "strike" | "bash" | "physical" => Element::Blunt,
        "fire" | "flame" | "burn" => Element::Fire,
        "ice" | "frost" | "cold" | "water" => Element::Ice,
        "lightning" | "thunder" | "electric" | "shock" | "bolt" => Element::Lightning,
        "light" | "holy" | "radiant" => Element::Light,
        "dark" | "shadow" | "void" | "curse" => Element::Dark,
        _ => return None,
    })
}

fn parse_target(s: &str) -> Option<SkillTarget> {
    Some(match norm(s).as_str() {
        "oneenemy" | "enemy" | "singleenemy" | "single" => SkillTarget::OneEnemy,
        "allenemies" | "allenemy" | "enemies" | "aoe" => SkillTarget::AllEnemies,
        "oneally" | "ally" | "singleally" => SkillTarget::OneAlly,
        "allallies" | "allies" | "party" => SkillTarget::AllAllies,
        "own" | "self" | "user" | "caster" => SkillTarget::Own,
        _ => return None,
    })
}

enum EffKind {
    Damage,
    Heal,
    Buff,
    Break,
}

fn parse_effect(s: &str) -> EffKind {
    match norm(s).as_str() {
        "heal" | "healing" | "restore" | "cure" => EffKind::Heal,
        "buffattack" | "buff" | "attackup" | "atkup" | "raiseattack" => EffKind::Buff,
        "breakdefense" | "break" | "defensedown" | "defdown" | "debuff" | "lowerdefense" => {
            EffKind::Break
        }
        _ => EffKind::Damage,
    }
}

fn parse_kind(s: &str) -> ItemKind {
    match norm(s).as_str() {
        "healmp" | "mp" | "mana" | "ether" => ItemKind::HealMp,
        "revive" | "resurrect" | "phoenix" | "life" => ItemKind::Revive,
        _ => ItemKind::HealHp,
    }
}

/// "Added N X(s): a, b, c." with a sensible plural.
fn added_summary(noun: &str, names: &[String]) -> String {
    let plural = if names.len() == 1 {
        noun.to_string()
    } else {
        format!("{noun}s")
    };
    format!("Added {} {}: {}.", names.len(), plural, names.join(", "))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::defaults::default_project;

    #[test]
    fn extract_json_from_code_fence() {
        let raw = "Sure! Here you go:\n```json\n[{\"name\":\"Zap\"}]\n```\nHope that helps.";
        assert_eq!(extract_json(raw).as_deref(), Some("[{\"name\":\"Zap\"}]"));
    }

    #[test]
    fn extract_json_after_think_block() {
        let raw = "<think>Let me design a skill. Fire seems good.</think>\n[{\"name\":\"Ember\"}]";
        assert_eq!(extract_json(raw).as_deref(), Some("[{\"name\":\"Ember\"}]"));
    }

    #[test]
    fn extract_json_bare_object_with_nested_brackets() {
        let raw = "noise {\"a\":[1,2,{\"b\":3}]} trailing";
        assert_eq!(
            extract_json(raw).as_deref(),
            Some("{\"a\":[1,2,{\"b\":3}]}")
        );
    }

    #[test]
    fn skills_are_added_with_continuing_ids() {
        let mut p = default_project(Language::English);
        let start_max = p.skills.iter().map(|s| s.id).max().unwrap();
        let raw = r#"[
            {"name":"Frost Nova","element":"frost","power":1.4,"mp_cost":9,
             "target":"all enemies","effect":"Damage","hits":1,"description":"Chill."},
            {"name":"Rally","element":"Light","power":0,"mp_cost":6,
             "target":"AllAllies","effect":"BuffAttack","effect_turns":3,"hits":1,"description":"Cheer."}
        ]"#;
        let applied = apply(GenTarget::Skills, &mut p, raw).unwrap();
        assert!(applied.new_map.is_none());
        let nova = p.skills.iter().find(|s| s.name == "Frost Nova").unwrap();
        assert_eq!(nova.id, start_max + 1);
        assert_eq!(nova.element, Element::Ice); // "frost" normalized
        assert_eq!(nova.target, SkillTarget::AllEnemies);
        let rally = p.skills.iter().find(|s| s.name == "Rally").unwrap();
        assert_eq!(rally.id, start_max + 2);
        assert!(matches!(rally.effect, SkillEffect::BuffAttack(3)));
    }

    #[test]
    fn enemy_skill_refs_are_filtered_to_existing() {
        let mut p = default_project(Language::English);
        let valid = p.skills[0].id;
        let raw = format!(
            r#"[{{"name":"Wraith","sprite":9,"hp":80,"atk":12,"weaknesses":["Light","bogus"],
                "skill_ids":[{valid}, 99999]}}]"#
        );
        apply(GenTarget::Monsters, &mut p, &raw).unwrap();
        let e = p.enemies.iter().find(|e| e.name == "Wraith").unwrap();
        assert_eq!(e.sprite, 1); // 9 % 4
        assert_eq!(e.skills, vec![valid]); // 99999 dropped
        assert_eq!(e.weaknesses, vec![Element::Light]); // "bogus" dropped
    }

    #[test]
    fn troops_drop_invalid_members_and_error_when_all_invalid() {
        let mut p = default_project(Language::English);
        let real = p.enemies[0].id;
        let ok = r#"[{"name":"Ambush","member_ids":[999, 88888]},
                     {"name":"Real Fight","member_ids":[999, VALID, VALID]}]"#
            .replace("VALID", &real.to_string());
        let applied = apply(GenTarget::Troops, &mut p, &ok).unwrap();
        // "Ambush" had no valid members and is skipped; only "Real Fight" survives.
        assert!(applied.summary.contains("Real Fight"));
        assert!(!applied.summary.contains("Ambush"));
        let t = p.troops.iter().find(|t| t.name == "Real Fight").unwrap();
        assert_eq!(t.members, vec![real, real]);
    }

    #[test]
    fn map_decodes_from_ascii_grids() {
        let mut p = default_project(Language::English);
        let raw = r#"{
            "name":"Test Isle","width":4,"height":4,
            "terrain":["....","~~~~","cccc","...."],
            "props":["T..T","....","..Y.","...."],
            "heights":["2222","0000","1111","5555"],
            "ambience":{"darkness":0.8,"fog_density":0.05},
            "encounter_steps":10
        }"#;
        let applied = apply(GenTarget::Map, &mut p, raw).unwrap();
        let id = applied.new_map.unwrap();
        let m = p.map(id).unwrap();
        assert_eq!((m.width, m.height), (4, 4));
        assert_eq!(m.name, "Test Isle");
        // Row 0: grass at height 2, trees at the corners.
        assert_eq!(m.tile(0, 0).terrain, Terrain::Grass as u8);
        assert_eq!(m.tile(0, 0).height, 2);
        assert_eq!(m.tile(0, 0).prop, Prop::Tree as u8);
        // Row 1: water is forced to height 0 with no prop.
        assert_eq!(m.tile(1, 1).terrain, Terrain::Water as u8);
        assert_eq!(m.tile(1, 1).height, 0);
        // Row 2: cave floor with a crystal prop.
        assert_eq!(m.tile(2, 2).terrain, Terrain::CaveFloor as u8);
        assert_eq!(m.tile(2, 2).prop, Prop::Crystal as u8);
        assert_eq!(m.ambience.darkness, 0.8);
        assert_eq!(m.encounter_steps, 10);
    }

    #[test]
    fn map_infers_dimensions_from_rows() {
        let mut p = default_project(Language::English);
        let raw = r#"{"terrain":["......","......","......","......","......","......"]}"#;
        let id = apply(GenTarget::Map, &mut p, raw).unwrap().new_map.unwrap();
        let m = p.map(id).unwrap();
        assert_eq!((m.width, m.height), (6, 6));
    }
}
