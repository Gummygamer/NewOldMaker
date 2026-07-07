//! In-game localization. Only the text a *player* sees during playtest is
//! translated (dialogue box, battle UI/log, victory & game-over, controls
//! hint, pickup messages); the editor stays in English. NPC dialogue driven by
//! the LLM is nudged into the chosen language via [`Language::llm_instruction`].

use serde::{Deserialize, Serialize};

/// The language the finished game is played in.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum Language {
    #[default]
    English,
    Portuguese,
}

pub const ALL_LANGUAGES: [Language; 2] = [Language::English, Language::Portuguese];

impl Language {
    /// Endonym shown in the language picker.
    pub fn name(self) -> &'static str {
        match self {
            Language::English => "English",
            Language::Portuguese => "Português",
        }
    }

    /// Extra system-prompt line steering the LLM into this language, or `None`
    /// when the default (English) needs no nudging.
    pub fn llm_instruction(self) -> Option<&'static str> {
        match self {
            Language::English => None,
            Language::Portuguese => Some(
                "Always reply in Brazilian Portuguese (português do Brasil), \
                 regardless of the language the player writes in.",
            ),
        }
    }

    // -- Fixed strings (pick per language) --------------------------------

    /// `en` when English, `pt` otherwise. Keeps the call sites terse.
    fn pick(self, en: &'static str, pt: &'static str) -> &'static str {
        match self {
            Language::English => en,
            Language::Portuguese => pt,
        }
    }

    pub fn controls_hint(self) -> &'static str {
        self.pick(
            "WASD/arrows move · Z/Space interact · Q/E rotate · F5 stop",
            "WASD/setas mover · Z/Espaço interagir · Q/E girar · F5 parar",
        )
    }
    pub fn say_something(self) -> &'static str {
        self.pick("Say something…", "Diga algo…")
    }
    pub fn send(self) -> &'static str {
        self.pick("Send", "Enviar")
    }
    pub fn leave(self) -> &'static str {
        self.pick("Leave", "Sair")
    }
    pub fn close_hint(self) -> &'static str {
        self.pick("Z / Esc to close", "Z / Esc para fechar")
    }
    pub fn game_over(self) -> &'static str {
        self.pick("Game Over", "Fim de Jogo")
    }
    pub fn party_fallen(self) -> &'static str {
        self.pick("The party has fallen…", "O grupo foi derrotado…")
    }
    pub fn return_to_editor(self) -> &'static str {
        self.pick("Return to editor", "Voltar ao editor")
    }
    pub fn target(self) -> &'static str {
        self.pick("Target:", "Alvo:")
    }
    pub fn back(self) -> &'static str {
        self.pick("← Back", "← Voltar")
    }
    pub fn attack(self) -> &'static str {
        self.pick("⚔ Attack", "⚔ Atacar")
    }
    pub fn skills(self) -> &'static str {
        self.pick("✨ Skills", "✨ Habilidades")
    }
    pub fn items(self) -> &'static str {
        self.pick("🎒 Items", "🎒 Itens")
    }
    pub fn defend(self) -> &'static str {
        self.pick("🛡 Defend", "🛡 Defender")
    }
    pub fn flee(self) -> &'static str {
        self.pick("🏃 Flee", "🏃 Fugir")
    }
    pub fn victory(self) -> &'static str {
        self.pick("Victory!", "Vitória!")
    }
    pub fn escaped(self) -> &'static str {
        self.pick("Escaped!", "Escapou!")
    }
    pub fn break_popup(self) -> &'static str {
        self.pick("BREAK!", "QUEBRA!")
    }
    pub fn revived(self) -> &'static str {
        self.pick("Revived!", "Revivido!")
    }
    pub fn got_away(self) -> &'static str {
        self.pick("Got away safely!", "Escapou em segurança!")
    }
    pub fn couldnt_escape(self) -> &'static str {
        self.pick("Couldn't escape!", "Não conseguiu escapar!")
    }
    pub fn party_refreshed(self) -> &'static str {
        self.pick("The party feels refreshed!", "O grupo se sente revigorado!")
    }
    pub fn boost(self) -> &'static str {
        self.pick("Boost", "Impulso")
    }

    // -- Formatted strings ------------------------------------------------

    pub fn attacks(self, name: &str) -> String {
        match self {
            Language::English => format!("{name} attacks!"),
            Language::Portuguese => format!("{name} ataca!"),
        }
    }
    pub fn is_broken(self, name: &str) -> String {
        match self {
            Language::English => format!("{name} is broken and can't move!"),
            Language::Portuguese => format!("{name} está quebrado e não pode agir!"),
        }
    }
    pub fn guard_broken(self, name: &str) -> String {
        match self {
            Language::English => format!("{name}'s guard is broken!"),
            Language::Portuguese => format!("A guarda de {name} foi quebrada!"),
        }
    }
    pub fn is_defeated(self, name: &str) -> String {
        match self {
            Language::English => format!("{name} is defeated!"),
            Language::Portuguese => format!("{name} foi derrotado!"),
        }
    }
    pub fn uses(self, name: &str, thing: &str) -> String {
        match self {
            Language::English => format!("{name} uses {thing}!"),
            Language::Portuguese => format!("{name} usa {thing}!"),
        }
    }
    pub fn guards(self, name: &str) -> String {
        match self {
            Language::English => format!("{name} guards."),
            Language::Portuguese => format!("{name} se defende."),
        }
    }
    pub fn reached_level(self, name: &str, level: u32) -> String {
        match self {
            Language::English => format!("{name} reached Lv.{level}!"),
            Language::Portuguese => format!("{name} alcançou o Nv.{level}!"),
        }
    }
    pub fn round(self, n: u32) -> String {
        match self {
            Language::English => format!("Round {n}"),
            Language::Portuguese => format!("Rodada {n}"),
        }
    }
    pub fn found(self, name: &str) -> String {
        match self {
            Language::English => format!("Found {name}!"),
            Language::Portuguese => format!("Encontrou {name}!"),
        }
    }
    pub fn victory_gained(self, exp: u32) -> String {
        match self {
            Language::English => format!("Victory! Gained {exp} EXP."),
            Language::Portuguese => format!("Vitória! Ganhou {exp} EXP."),
        }
    }
    pub fn choose_action(self, name: &str) -> String {
        match self {
            Language::English => format!("{name} — choose action"),
            Language::Portuguese => format!("{name} — escolha a ação"),
        }
    }
    pub fn mp_gain(self, v: i32) -> String {
        // "MP" is used as an abbreviation in both languages.
        format!("+{v} MP")
    }
}
