//! LLM-driven NPC dialogue. A worker thread owns the model (a local llama.cpp
//! GGUF, or an NVIDIA NIM cloud endpoint); the UI sends `ChatRequest`s and
//! receives streamed tokens via channels, so the frame loop never blocks. Built
//! without a backend feature, everything degrades to the personas' scripted
//! fallback lines.

use std::sync::mpsc::{Receiver, Sender, TryRecvError};

use crate::core::data::{Language, LlmBackend, LlmSettings, NpcPersona};

#[derive(Clone, Debug, PartialEq)]
pub enum LlmStatus {
    /// No model configured (or engine built without the `llm` feature).
    Off,
    Loading,
    Ready(String),
    Error(String),
}

#[derive(Clone, Debug)]
pub struct ChatTurn {
    pub from_player: bool,
    pub text: String,
}

#[derive(Clone, Debug)]
pub struct ChatRequest {
    pub id: u64,
    pub persona: NpcPersona,
    pub game_title: String,
    pub location: String,
    pub player_name: String,
    /// Full conversation so far, oldest first. Empty = NPC greets the player.
    pub history: Vec<ChatTurn>,
    pub max_tokens: u32,
    pub temperature: f32,
    /// Language the NPC should reply in.
    pub language: Language,
}

/// A one-shot content-generation request (maps, database elements). Unlike a
/// [`ChatRequest`], the reply is *not* run through [`ReplyFilter`] — the caller
/// wants the raw text (JSON) — and a bigger token budget is used. The streamed
/// pieces arrive as [`LlmEvent::Token`] and completion as [`LlmEvent::Done`],
/// keyed by `id`, exactly like a chat; the caller accumulates them itself.
#[derive(Clone, Debug)]
pub struct GenRequest {
    pub id: u64,
    /// System prompt: the schema and rules the model must follow.
    pub system: String,
    /// User prompt: the designer's request plus what to produce.
    pub prompt: String,
    pub max_tokens: u32,
    pub temperature: f32,
}

#[derive(Clone, Debug)]
pub enum LlmEvent {
    Status(LlmStatus),
    Token { id: u64, text: String },
    Done { id: u64 },
    Error { id: u64, msg: String },
}

enum WorkerMsg {
    Load(LlmSettings),
    Chat(Box<ChatRequest>),
    Generate(Box<GenRequest>),
}

pub struct LlmEngine {
    to_worker: Option<Sender<WorkerMsg>>,
    from_worker: Option<Receiver<LlmEvent>>,
    pub status: LlmStatus,
    next_id: u64,
    /// Signature of the settings the current worker was built for; a change
    /// tears the worker down and spins up a fresh one for the new backend.
    configured_sig: String,
}

impl LlmEngine {
    pub fn new() -> Self {
        LlmEngine {
            to_worker: None,
            from_worker: None,
            status: LlmStatus::Off,
            next_id: 1,
            configured_sig: String::new(),
        }
    }

    /// Ensure the worker matches the project settings (loads/reloads the
    /// backend when the model, endpoint, or key changes).
    pub fn configure(&mut self, settings: &LlmSettings) {
        let sig = settings.worker_signature();
        if sig == self.configured_sig {
            return;
        }
        self.configured_sig = sig;
        // Tear down any existing worker; each (re)configuration spawns fresh.
        self.to_worker = None;
        self.from_worker = None;
        self.status = LlmStatus::Off;

        if !settings.is_configured() {
            return;
        }

        match settings.backend {
            LlmBackend::Local => {
                #[cfg(feature = "llm")]
                self.spawn(backend::worker, settings);
                #[cfg(not(feature = "llm"))]
                {
                    self.status = LlmStatus::Error("engine built without the `llm` feature".into());
                }
            }
            LlmBackend::Nim => {
                #[cfg(feature = "nim")]
                self.spawn(nim::worker, settings);
                #[cfg(not(feature = "nim"))]
                {
                    self.status = LlmStatus::Error("engine built without the `nim` feature".into());
                }
            }
        }
    }

    /// Spawn a backend worker and hand it the initial settings to load.
    #[cfg(any(feature = "llm", feature = "nim"))]
    fn spawn(&mut self, worker: fn(Receiver<WorkerMsg>, Sender<LlmEvent>), settings: &LlmSettings) {
        let (tx_req, rx_req) = std::sync::mpsc::channel::<WorkerMsg>();
        let (tx_ev, rx_ev) = std::sync::mpsc::channel::<LlmEvent>();
        std::thread::Builder::new()
            .name("nom-llm".into())
            .spawn(move || worker(rx_req, tx_ev))
            .expect("spawn llm worker");
        tx_req.send(WorkerMsg::Load(settings.clone())).ok();
        self.to_worker = Some(tx_req);
        self.from_worker = Some(rx_ev);
        self.status = LlmStatus::Loading;
    }

    pub fn ready(&self) -> bool {
        matches!(self.status, LlmStatus::Ready(_))
    }

    /// Queue a chat request; returns its id (tokens arrive via `poll`).
    pub fn request(&mut self, mut req: ChatRequest) -> Option<u64> {
        let tx = self.to_worker.as_ref()?;
        if !self.ready() {
            return None;
        }
        let id = self.next_id;
        self.next_id += 1;
        req.id = id;
        tx.send(WorkerMsg::Chat(Box::new(req))).ok()?;
        Some(id)
    }

    /// Queue a content-generation request; returns its id (raw text arrives via
    /// `poll` as [`LlmEvent::Token`]s, terminated by [`LlmEvent::Done`]).
    pub fn generate(&mut self, mut req: GenRequest) -> Option<u64> {
        let tx = self.to_worker.as_ref()?;
        if !self.ready() {
            return None;
        }
        let id = self.next_id;
        self.next_id += 1;
        req.id = id;
        tx.send(WorkerMsg::Generate(Box::new(req))).ok()?;
        Some(id)
    }

    /// Drain pending events; call once per frame.
    pub fn poll(&mut self) -> Vec<LlmEvent> {
        let mut events = Vec::new();
        if let Some(rx) = &self.from_worker {
            loop {
                match rx.try_recv() {
                    Ok(ev) => {
                        if let LlmEvent::Status(s) = &ev {
                            self.status = s.clone();
                        }
                        events.push(ev);
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        self.status = LlmStatus::Error("LLM worker died".into());
                        self.to_worker = None;
                        self.from_worker = None;
                        break;
                    }
                }
            }
        }
        events
    }
}

/// Build the system prompt that keeps the model in character.
pub fn system_prompt(req: &ChatRequest) -> String {
    let p = &req.persona;
    let mut s = format!(
        "You are {}, {}, a character in the RPG \"{}\". The player, {}, is talking to you in {}.\n",
        p.name, p.role, req.game_title, req.player_name, req.location
    );
    if !p.personality.trim().is_empty() {
        s.push_str(&format!(
            "Personality and speaking style: {}\n",
            p.personality.trim()
        ));
    }
    if !p.knowledge.trim().is_empty() {
        s.push_str(&format!("Things you know: {}\n", p.knowledge.trim()));
    }
    if !p.constraints.trim().is_empty() {
        s.push_str(&format!(
            "Hard rules you must follow: {}\n",
            p.constraints.trim()
        ));
    }
    s.push_str(
        "Stay in character. Speak only as this character would, in plain spoken dialogue \
         (no narration, no quotation marks, no stage directions). Never write HTML or XML \
         tags, markup, code blocks, JSON, or tool calls, and never reveal your private \
         thoughts — only the words you say aloud. You have no tools; just talk. \
         Keep replies to one to three short sentences.",
    );
    if let Some(instruction) = req.language.llm_instruction() {
        s.push(' ');
        s.push_str(instruction);
    }
    s
}

/// Assemble the ordered chat messages `(role, content)` sent to whichever
/// backend serves the reply. An empty history means the NPC greets first.
/// Roles are the OpenAI/ChatML trio (`system`/`user`/`assistant`), which both
/// llama.cpp's chat templating and NIM's OpenAI-compatible API understand.
pub fn build_chat(req: &ChatRequest) -> Vec<(&'static str, String)> {
    let mut msgs = vec![("system", system_prompt(req))];
    if req.history.is_empty() {
        msgs.push((
            "user",
            format!(
                "{} walks up to you. Greet them in character.",
                req.player_name
            ),
        ));
    }
    for turn in &req.history {
        let role = if turn.from_player {
            "user"
        } else {
            "assistant"
        };
        msgs.push((role, turn.text.clone()));
    }
    msgs
}

/// Assemble the two-message (system + user) chat for a [`GenRequest`], in the
/// same `(role, content)` shape [`build_chat`] produces.
pub fn build_gen_chat(req: &GenRequest) -> Vec<(&'static str, String)> {
    vec![
        ("system", req.system.clone()),
        ("user", req.prompt.clone()),
    ]
}

// ---------------------------------------------------------------------------
// Reply filtering
// ---------------------------------------------------------------------------

/// Strips the control markup that "reasoning" chat models emit around their
/// answers. Many recent instruct models (the OpenAI *harmony* format and its
/// look-alikes) wrap output in channels, e.g.
///
/// ```text
/// <|channel|>analysis<|message|>The player greeted me...<|end|>
/// <|start|>assistant<|channel|>final<|message|>Well met, traveller!<|return|>
/// ```
///
/// Gemma 4 speaks a dialect with *asymmetric* tags: `<|name>` opens a channel
/// and `<name|>` closes it, with the user-facing reply as bare text outside
/// any channel:
///
/// ```text
/// <|channel>thought
/// The player greeted me...
/// <channel|>Well met, traveller!
/// ```
///
/// Some models instead degrade into HTML/XML-style markup — DeepSeek-style
/// `<think>...</think>` blocks, or free-form tags like
/// `<div style="thought" />` repeated line after line:
///
/// ```text
/// <div style="thought" />
/// <div style="thought" />
/// ```
///
/// Without filtering, those tags and the private "thinking" channel leak into
/// the chat bubble. This is a small streaming state machine: feed it token
/// pieces as they arrive and it returns only the user-facing text, hiding
/// reasoning channels and swallowing the control tags — even when a tag is
/// split across two token pieces.
///
/// It is deliberately tolerant: models that emit no channel markup at all have
/// their text passed straight through, and a channel with an unknown or empty
/// name is treated as visible (better to show a stray reply than to mute it).
#[derive(Default)]
pub struct ReplyFilter {
    /// Text not yet decided on — may hold the front of a tag split across pieces.
    pending: String,
    mode: Mode,
    /// Whether the current channel's body should be shown to the player.
    visible: bool,
    /// Have we ever seen a control tag? Until we do, plain text is shown.
    harmony: bool,
    /// Channel name currently being read (between `<|channel|>` and its body).
    name: String,
    /// Name of the HTML-ish tag whose body is being hidden (e.g. `div` for
    /// `<div class="thought">…</div>`), so its closer restores visibility.
    hidden_tag: String,
    /// Inside a ``` fenced code block. Models that degrade into emitting a
    /// tool-call JSON blob wrap it in a fence; dialogue never contains one, so
    /// everything between the fences is swallowed.
    fence: bool,
    /// All user-facing text emitted so far (used for stop conditions / fallback).
    shown: String,
    /// Body of the most recent channel, kept as a last-resort fallback when no
    /// visible channel is ever produced.
    last_channel: String,
}

#[derive(Default, PartialEq)]
enum Mode {
    /// Emitting (or hiding) channel body text.
    #[default]
    Body,
    /// Reading a channel name after `<|channel|>`, up to its body.
    ReadingName,
    /// Between `<|start|>` and the next tag (a role name we ignore).
    SkipToTag,
}

/// Channel names whose content is the model's private reasoning, not dialogue.
fn is_reasoning_channel(name: &str) -> bool {
    matches!(
        name.trim().to_ascii_lowercase().as_str(),
        "analysis"
            | "think"
            | "thinking"
            | "thought"
            | "thoughts"
            | "reasoning"
            | "reflection"
            | "commentary"
            | "critic"
            | "plan"
    )
}

/// Tag names that mark a bare `<name>` tag as markup rather than dialogue:
/// reasoning channels plus the HTML and turn-boundary tags models fall back
/// to. Kept to a fixed list so dialogue like "press <Enter>" is never eaten.
/// The one open-ended family is `start_of_*` / `end_of_*`: models improvise
/// boundary tokens in that shape (`<end_of_turn>`, `<end_of_action>`, …) and
/// no plausible dialogue does.
fn is_markup_name(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    is_reasoning_channel(&name)
        || name.starts_with("start_of_")
        || name.starts_with("end_of_")
        || matches!(
            name.as_str(),
            "div"
                | "span"
                | "p"
                | "br"
                | "hr"
                | "b"
                | "i"
                | "em"
                | "strong"
                | "u"
                | "sub"
                | "sup"
                | "code"
                | "pre"
                | "details"
                | "summary"
                | "im_start"
                | "im_end"
        )
}

/// True if any word in the tag (name or attribute values) names a reasoning
/// channel, e.g. `div style="thought"`.
fn has_reasoning_word(inner: &str) -> bool {
    inner
        .split(|c: char| !is_name_char(c))
        .any(is_reasoning_channel)
}

impl ReplyFilter {
    pub fn new() -> Self {
        ReplyFilter {
            visible: true,
            ..Default::default()
        }
    }

    /// The user-facing text emitted so far.
    pub fn shown(&self) -> &str {
        &self.shown
    }

    /// Feed a token piece; returns any newly revealed user-facing text.
    pub fn push(&mut self, piece: &str) -> String {
        self.pending.push_str(piece);
        let mut out = String::new();
        loop {
            // The next thing worth stopping on: a code fence (always), plus a
            // control tag when we're not already inside a fence.
            let backtick = self.pending.find('`');
            let lt = if self.fence {
                None
            } else {
                self.pending.find('<')
            };
            let Some(pos) = min_opt(backtick, lt) else {
                let text = std::mem::take(&mut self.pending);
                self.consume_text(&text, &mut out);
                break;
            };
            let before = self.pending[..pos].to_string();
            self.consume_text(&before, &mut out);
            let rest = self.pending[pos..].to_string();
            if rest.starts_with('`') {
                // A run of three or more backticks opens or closes a fence.
                let run = rest.chars().take_while(|&c| c == '`').count();
                if run >= 3 {
                    self.fence = !self.fence;
                    self.pending = rest[run..].to_string();
                } else if run == rest.len() {
                    // One or two trailing backticks: might grow into a fence.
                    self.pending = rest;
                    break;
                } else {
                    // One or two backticks mid-text: ordinary dialogue.
                    self.consume_text(&rest[..run], &mut out);
                    self.pending = rest[run..].to_string();
                }
            } else if let Some((inner, shape, len)) = match_tag(&rest) {
                self.handle_tag(&inner, shape);
                self.pending = rest[len..].to_string();
            } else if is_partial_tag(&rest) {
                // Might still become a tag once more pieces arrive; hold it.
                self.pending = rest;
                break;
            } else {
                // A lone '<' that isn't the start of a tag: ordinary text.
                self.consume_text("<", &mut out);
                self.pending = rest[1..].to_string();
            }
        }
        out
    }

    /// Flush any buffered text at end of generation. Falls back to the last
    /// channel's content if nothing user-facing was ever shown, so the NPC is
    /// never left silent.
    pub fn finish(&mut self) -> String {
        let mut out = String::new();
        // A trailing partial tag never completed. A '<' followed by a pipe, an
        // attribute, or a known markup name is almost certainly truncated
        // markup; anything else was real text.
        let leftover = std::mem::take(&mut self.pending);
        let markup = leftover.starts_with('<') && {
            let body = leftover[1..].strip_prefix('/').unwrap_or(&leftover[1..]);
            let name_end = body.find(|c: char| !is_name_char(c)).unwrap_or(body.len());
            leftover.contains('|') || leftover.contains('=') || is_markup_name(&body[..name_end])
        };
        // A trailing run of backticks is a fence marker that never completed;
        // drop it rather than show stray backticks.
        let backticks = !leftover.is_empty() && leftover.chars().all(|c| c == '`');
        if !markup && !backticks {
            self.consume_text(&leftover, &mut out);
        }
        if self.shown.trim().is_empty() {
            let fallback = self.last_channel.trim();
            if !fallback.is_empty() {
                out.push_str(fallback);
                self.shown.push_str(fallback);
            }
        }
        out
    }

    fn consume_text(&mut self, text: &str, out: &mut String) {
        if self.fence {
            // Inside a fenced code block: never user-facing, and kept out of
            // the fallback channel so a pure tool-call reply stays hidden.
            return;
        }
        match self.mode {
            Mode::SkipToTag => {}
            Mode::ReadingName => {
                // Some models terminate the channel name with a newline rather
                // than a `<|message|>` tag; honour both.
                if let Some(nl) = text.find('\n') {
                    self.name.push_str(&text[..nl]);
                    self.begin_body();
                    let rest = text[nl + 1..].to_string();
                    self.consume_text(&rest, out);
                } else {
                    self.name.push_str(text);
                }
            }
            Mode::Body => {
                self.last_channel.push_str(text);
                if self.visible {
                    // Trim leading whitespace so the bubble doesn't open blank.
                    let text = if self.shown.is_empty() {
                        text.trim_start()
                    } else {
                        text
                    };
                    self.shown.push_str(text);
                    out.push_str(text);
                }
            }
        }
    }

    /// Finish reading a channel name and start its body.
    fn begin_body(&mut self) {
        self.visible = !is_reasoning_channel(&self.name);
        self.last_channel.clear();
        self.mode = Mode::Body;
    }

    fn handle_tag(&mut self, inner: &str, shape: TagShape) {
        self.harmony = true;
        match shape {
            TagShape::Close => {
                // Gemma-4 style `<name|>`: the channel is over, and whatever
                // follows is the user-facing reply (there is no explicit
                // "final" channel in this dialect).
                self.mode = Mode::Body;
                self.visible = true;
            }
            TagShape::BareSelfClose => {
                // Standalone markup like `<div style="thought" />`: swallow it.
            }
            TagShape::BareClose => {
                // `</think>`, or the closer matching the tag that opened the
                // hidden block (`</div>` after `<div class="thought">`): the
                // reasoning is over and what follows is the reply. Other
                // closers are markup noise; swallow.
                let name = inner
                    .split(|c: char| !is_name_char(c))
                    .next()
                    .unwrap_or("")
                    .to_ascii_lowercase();
                if has_reasoning_word(inner)
                    || (!self.hidden_tag.is_empty() && name == self.hidden_tag)
                {
                    self.hidden_tag.clear();
                    self.mode = Mode::Body;
                    self.visible = true;
                }
            }
            TagShape::BareOpen => {
                let name = inner
                    .split(|c: char| !is_name_char(c))
                    .next()
                    .unwrap_or("")
                    .to_ascii_lowercase();
                if name == "start_of_turn" || name == "im_start" {
                    // A role name follows (e.g. `<start_of_turn>model`), then
                    // the body — same as reading a channel name.
                    self.name.clear();
                    self.mode = Mode::ReadingName;
                } else if name == "end_of_turn" || name == "im_end" {
                    self.mode = Mode::Body;
                    self.visible = false;
                } else if has_reasoning_word(inner) {
                    // `<think>` or `<div class="thought">`: hide the body
                    // until the matching closer.
                    self.hidden_tag = name;
                    self.last_channel.clear();
                    self.mode = Mode::Body;
                    self.visible = false;
                }
                // Any other HTML-ish tag is markup noise; swallow it.
            }
            TagShape::Symmetric | TagShape::Open => {
                match inner.trim().to_ascii_lowercase().as_str() {
                    "channel" => {
                        self.name.clear();
                        self.mode = Mode::ReadingName;
                    }
                    "message" => self.begin_body(),
                    "start" => {
                        self.mode = Mode::SkipToTag;
                        self.visible = false;
                    }
                    "end" | "return" | "endoftext" | "eot" | "eom" => {
                        self.mode = Mode::Body;
                        self.visible = false;
                    }
                    _ => {
                        // Unknown tag: if we were mid-name, treat it as the name's end.
                        if self.mode == Mode::ReadingName {
                            self.begin_body();
                        }
                    }
                }
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum TagShape {
    /// Harmony `<|name|>`.
    Symmetric,
    /// Gemma-4 channel opener `<|name>`.
    Open,
    /// Gemma-4 channel closer `<name|>`.
    Close,
    /// HTML/XML-style opener `<name>` / `<name attrs>`.
    BareOpen,
    /// HTML/XML-style closer `</name>`.
    BareClose,
    /// HTML/XML-style self-closing tag `<name attrs />`.
    BareSelfClose,
}

/// True for the characters allowed in a bare `<name|>` closing tag. Kept
/// strict (word-like only) so ordinary dialogue containing '<' never matches.
fn is_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

/// The smaller of two optional byte offsets, treating `None` as "no match".
fn min_opt(a: Option<usize>, b: Option<usize>) -> Option<usize> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

/// If `s` starts with a complete control tag, return its inner text, its
/// shape, and the byte length consumed.
fn match_tag(s: &str) -> Option<(String, TagShape, usize)> {
    if let Some(rest) = s.strip_prefix("<|") {
        let close = rest.find('>')?;
        let inner = &rest[..close];
        // A newline or '<' inside means this is not a real control tag.
        if inner.contains('\n') || inner.contains('<') {
            return None;
        }
        return if let Some(name) = inner.strip_suffix('|') {
            Some((name.to_string(), TagShape::Symmetric, 2 + close + 1))
        } else if !inner.contains('|') {
            Some((inner.to_string(), TagShape::Open, 2 + close + 1))
        } else {
            None
        };
    }
    let rest = s.strip_prefix('<')?;
    if let Some(bar) = rest.find("|>") {
        let name = &rest[..bar];
        if !name.is_empty() && name.chars().all(is_name_char) {
            return Some((name.to_string(), TagShape::Close, 1 + bar + 2));
        }
    }
    // HTML/XML-style tags some models fall back to: `<think>`, `</think>`,
    // `<div style="thought" />`. Only accepted when the tag name is a known
    // markup name or the tag carries attributes (`=`), so dialogue containing
    // '<' stays untouched.
    let (body, closing) = match rest.strip_prefix('/') {
        Some(r) => (r, true),
        None => (rest, false),
    };
    if !body.starts_with(|c: char| c.is_ascii_alphabetic()) {
        return None;
    }
    let name_end = body.find(|c: char| !is_name_char(c)).unwrap_or(body.len());
    let name = &body[..name_end];
    let after = &body[name_end..];
    let gt = after.find('>')?;
    let attrs = &after[..gt];
    if attrs.contains('\n') || attrs.contains('<') || attrs.contains('|') {
        return None;
    }
    let len = 1 + usize::from(closing) + name_end + gt + 1;
    if closing {
        if attrs.is_empty() && is_markup_name(name) {
            return Some((name.to_string(), TagShape::BareClose, len));
        }
        return None;
    }
    if !is_markup_name(name) && !attrs.contains('=') {
        return None;
    }
    let shape = if attrs.trim_end().ends_with('/') {
        TagShape::BareSelfClose
    } else {
        TagShape::BareOpen
    };
    Some((format!("{name}{attrs}"), shape, len))
}

/// Could `s` (which starts with '<') still grow into a control tag once more
/// pieces arrive?
fn is_partial_tag(s: &str) -> bool {
    if s == "<" || s == "<|" {
        return true;
    }
    if let Some(rest) = s.strip_prefix("<|") {
        // Heading toward `<|name|>` or `<|name>`: no closer yet, and at most
        // one pipe, which must be trailing.
        return !rest.contains('>')
            && !rest.contains('\n')
            && !rest.contains('<')
            && (!rest.contains('|') || (rest.ends_with('|') && rest.matches('|').count() == 1));
    }
    // Heading toward `<name|>`: word-like name, optionally ending in the pipe.
    let rest = &s[1..];
    let name = rest.strip_suffix('|').unwrap_or(rest);
    if !name.is_empty() && !name.contains('|') && name.chars().all(is_name_char) {
        return true;
    }
    // Heading toward an HTML-ish tag: `</name`, `<name attrs…`, `<name attrs /`.
    // Capped so a false positive can't buffer text indefinitely.
    if s.len() > 160 {
        return false;
    }
    let body = rest.strip_prefix('/').unwrap_or(rest);
    if body.is_empty() {
        // Just "</" so far — the closer's name hasn't arrived yet.
        return rest.starts_with('/');
    }
    if !body.starts_with(|c: char| c.is_ascii_alphabetic()) {
        return false;
    }
    let name_end = body.find(|c: char| !is_name_char(c)).unwrap_or(body.len());
    let attrs = &body[name_end..];
    if attrs.contains('>') || attrs.contains('\n') || attrs.contains('<') || attrs.contains('|') {
        return false;
    }
    attrs.is_empty() || is_markup_name(&body[..name_end]) || attrs.contains('=')
}

// ---------------------------------------------------------------------------
// llama.cpp worker (feature = "llm")
// ---------------------------------------------------------------------------

#[cfg(feature = "llm")]
mod backend {
    use super::*;
    use llama_cpp_2::context::params::LlamaContextParams;
    use llama_cpp_2::llama_backend::LlamaBackend;
    use llama_cpp_2::llama_batch::LlamaBatch;
    use llama_cpp_2::model::params::LlamaModelParams;
    use llama_cpp_2::model::{AddBos, LlamaChatMessage, LlamaChatTemplate, LlamaModel};
    use llama_cpp_2::sampling::LlamaSampler;
    use std::num::NonZeroU32;

    const N_BATCH: u32 = 1024;

    pub(super) fn worker(rx: Receiver<WorkerMsg>, tx: Sender<LlmEvent>) {
        let backend = match LlamaBackend::init() {
            Ok(b) => b,
            Err(e) => {
                tx.send(LlmEvent::Status(LlmStatus::Error(format!(
                    "llama init: {e}"
                ))))
                .ok();
                return;
            }
        };
        let mut model: Option<LlamaModel> = None;
        let mut settings = LlmSettings::default();

        while let Ok(msg) = rx.recv() {
            match msg {
                WorkerMsg::Load(s) => {
                    tx.send(LlmEvent::Status(LlmStatus::Loading)).ok();
                    settings = s;
                    match LlamaModel::load_from_file(
                        &backend,
                        &settings.model_path,
                        &LlamaModelParams::default(),
                    ) {
                        Ok(m) => {
                            let name = std::path::Path::new(&settings.model_path)
                                .file_name()
                                .map(|f| f.to_string_lossy().to_string())
                                .unwrap_or_else(|| "model".into());
                            model = Some(m);
                            tx.send(LlmEvent::Status(LlmStatus::Ready(name))).ok();
                        }
                        Err(e) => {
                            tx.send(LlmEvent::Status(LlmStatus::Error(format!("load: {e}"))))
                                .ok();
                        }
                    }
                }
                WorkerMsg::Chat(req) => {
                    let Some(m) = &model else {
                        tx.send(LlmEvent::Error {
                            id: req.id,
                            msg: "no model loaded".into(),
                        })
                        .ok();
                        continue;
                    };
                    if let Err(e) = run_chat(&backend, m, &settings, &req, &tx) {
                        tx.send(LlmEvent::Error {
                            id: req.id,
                            msg: e.to_string(),
                        })
                        .ok();
                    }
                }
                WorkerMsg::Generate(req) => {
                    let Some(m) = &model else {
                        tx.send(LlmEvent::Error {
                            id: req.id,
                            msg: "no model loaded".into(),
                        })
                        .ok();
                        continue;
                    };
                    if let Err(e) = run_generate(&backend, m, &settings, &req, &tx) {
                        tx.send(LlmEvent::Error {
                            id: req.id,
                            msg: e.to_string(),
                        })
                        .ok();
                    }
                }
            }
        }
    }

    /// Render the assembled chat into a prompt string.
    ///
    /// We prefer the model's own chat template, but many GGUFs ship a Jinja
    /// template that llama.cpp's built-in applier can't interpret — it only
    /// understands a fixed set of formats and returns an FFI error (`ffi error
    /// -1`) for anything else. When that happens, fall back to a compatible
    /// built-in template (picked from the model architecture) so dialogue keeps
    /// working instead of surfacing the raw error in the chat bubble.
    fn render_prompt(model: &LlamaModel, chat: &[LlamaChatMessage]) -> anyhow::Result<String> {
        if let Ok(tmpl) = model.chat_template(None) {
            if let Ok(prompt) = model.apply_chat_template(&tmpl, chat, true) {
                return Ok(prompt);
            }
        }
        let fallback = LlamaChatTemplate::new(fallback_template_name(model))
            .expect("built-in template name is a valid c-string");
        Ok(model.apply_chat_template(&fallback, chat, true)?)
    }

    /// Pick a built-in llama.cpp chat template that matches the loaded model,
    /// for use when the model's own template can't be applied.
    fn fallback_template_name(model: &LlamaModel) -> &'static str {
        let arch = model
            .meta_val_str("general.architecture")
            .unwrap_or_default();
        match arch.as_str() {
            // Gemma 1/2/3 use <start_of_turn>…<end_of_turn> and no system role.
            a if a.starts_with("gemma") => "gemma",
            "phi3" => "phi3",
            "phi4" | "phimoe" => "phi4",
            // Qwen, Mistral, and most other instruct models tolerate ChatML.
            _ => "chatml",
        }
    }

    fn run_chat(
        backend: &LlamaBackend,
        model: &LlamaModel,
        settings: &LlmSettings,
        req: &ChatRequest,
        tx: &Sender<LlmEvent>,
    ) -> anyhow::Result<()> {
        // Assemble the chat and render it through the model's own template.
        let chat = super::build_chat(req)
            .into_iter()
            .map(|(role, content)| LlamaChatMessage::new(role.into(), content))
            .collect::<Result<Vec<_>, _>>()?;
        let prompt = render_prompt(model, &chat)?;

        let mut ctx_params = LlamaContextParams::default()
            .with_n_ctx(NonZeroU32::new(settings.context_tokens.max(512)))
            .with_n_batch(N_BATCH);
        if settings.threads > 0 {
            ctx_params = ctx_params.with_n_threads(settings.threads as i32);
        }
        let mut ctx = model.new_context(backend, ctx_params)?;

        let mut tokens = model.str_to_token(&prompt, AddBos::Never)?;
        // Keep the prompt inside the context window, preserving the start (system prompt).
        let budget = (settings.context_tokens.max(512) as usize)
            .saturating_sub(settings.max_reply_tokens as usize + 8);
        if tokens.len() > budget {
            let keep_head = budget / 2;
            let keep_tail = budget - keep_head;
            let tail_start = tokens.len() - keep_tail;
            let mut clipped = tokens[..keep_head].to_vec();
            clipped.extend_from_slice(&tokens[tail_start..]);
            tokens = clipped;
        }

        // Decode the prompt in n_batch-sized chunks.
        let mut batch = LlamaBatch::new(N_BATCH as usize, 1);
        let mut pos = 0i32;
        for chunk in tokens.chunks(N_BATCH as usize) {
            batch.clear();
            for (i, tok) in chunk.iter().enumerate() {
                let is_last = pos as usize + i + 1 == tokens.len();
                batch.add(*tok, pos + i as i32, &[0], is_last)?;
            }
            ctx.decode(&mut batch)?;
            pos += chunk.len() as i32;
        }

        let seed = (req.id as u32).wrapping_mul(2654435761).wrapping_add(1);
        let mut sampler = LlamaSampler::chain_simple([
            // Quantized small models can lock into repeating one line forever
            // (e.g. `<div style="thought" />` twelve times); penalize recent
            // tokens so the loop breaks.
            LlamaSampler::penalties(64, 1.15, 0.0, 0.0),
            LlamaSampler::temp(req.temperature.clamp(0.05, 2.0)),
            LlamaSampler::dist(seed),
        ]);

        // Filter reasoning-model control markup out of the streamed reply.
        let mut filter = ReplyFilter::new();
        // Streaming decoder: tokens can split multi-byte UTF-8 sequences.
        let mut decoder = encoding_rs::UTF_8.new_decoder();
        for _ in 0..req.max_tokens {
            let token = sampler.sample(&ctx, -1);
            sampler.accept(token);
            if model.is_eog_token(token) {
                break;
            }
            // `special: true` surfaces control tokens (e.g. `<|channel|>`) as
            // text so the filter can strip them, whether the model encodes them
            // as special tokens or as ordinary text.
            let piece = model
                .token_to_piece(token, &mut decoder, true, None)
                .unwrap_or_default();
            let visible = filter.push(&piece);
            if !visible.is_empty() {
                tx.send(LlmEvent::Token {
                    id: req.id,
                    text: visible,
                })?;
            }
            // NPC lines are short; stop at a blank line in the visible reply.
            if filter.shown().contains("\n\n") {
                break;
            }
            batch.clear();
            batch.add(token, pos, &[0], true)?;
            pos += 1;
            ctx.decode(&mut batch)?;
        }
        let tail = filter.finish();
        if !tail.is_empty() {
            tx.send(LlmEvent::Token {
                id: req.id,
                text: tail,
            })?;
        }
        tx.send(LlmEvent::Done { id: req.id })?;
        Ok(())
    }

    /// Run a content-generation request: the raw (unfiltered) text is streamed
    /// back token by token. A wider context and larger reply budget are used
    /// than for dialogue, and control tokens are decoded plainly so the JSON
    /// body arrives intact.
    fn run_generate(
        backend: &LlamaBackend,
        model: &LlamaModel,
        settings: &LlmSettings,
        req: &GenRequest,
        tx: &Sender<LlmEvent>,
    ) -> anyhow::Result<()> {
        let chat = super::build_gen_chat(req)
            .into_iter()
            .map(|(role, content)| LlamaChatMessage::new(role.into(), content))
            .collect::<Result<Vec<_>, _>>()?;
        let prompt = render_prompt(model, &chat)?;

        // Generation needs room for a long JSON reply, so widen the window
        // beyond the (dialogue-sized) project setting when necessary.
        let ctx_tokens = settings.context_tokens.max(4096);
        let mut ctx_params = LlamaContextParams::default()
            .with_n_ctx(NonZeroU32::new(ctx_tokens))
            .with_n_batch(N_BATCH);
        if settings.threads > 0 {
            ctx_params = ctx_params.with_n_threads(settings.threads as i32);
        }
        let mut ctx = model.new_context(backend, ctx_params)?;

        let mut tokens = model.str_to_token(&prompt, AddBos::Never)?;
        let budget = (ctx_tokens as usize).saturating_sub(req.max_tokens as usize + 8);
        if tokens.len() > budget {
            // Keep the head (schema/system prompt) and the tail (the request).
            let keep_head = budget / 2;
            let keep_tail = budget - keep_head;
            let tail_start = tokens.len() - keep_tail;
            let mut clipped = tokens[..keep_head].to_vec();
            clipped.extend_from_slice(&tokens[tail_start..]);
            tokens = clipped;
        }

        let mut batch = LlamaBatch::new(N_BATCH as usize, 1);
        let mut pos = 0i32;
        for chunk in tokens.chunks(N_BATCH as usize) {
            batch.clear();
            for (i, tok) in chunk.iter().enumerate() {
                let is_last = pos as usize + i + 1 == tokens.len();
                batch.add(*tok, pos + i as i32, &[0], is_last)?;
            }
            ctx.decode(&mut batch)?;
            pos += chunk.len() as i32;
        }

        let seed = (req.id as u32).wrapping_mul(2654435761).wrapping_add(7);
        let mut sampler = LlamaSampler::chain_simple([
            LlamaSampler::penalties(64, 1.1, 0.0, 0.0),
            LlamaSampler::temp(req.temperature.clamp(0.05, 2.0)),
            LlamaSampler::dist(seed),
        ]);

        let mut decoder = encoding_rs::UTF_8.new_decoder();
        for _ in 0..req.max_tokens {
            let token = sampler.sample(&ctx, -1);
            sampler.accept(token);
            if model.is_eog_token(token) {
                break;
            }
            // `special: false`: we want the plain JSON body, not template tokens.
            let piece = model
                .token_to_piece(token, &mut decoder, false, None)
                .unwrap_or_default();
            if !piece.is_empty() {
                tx.send(LlmEvent::Token {
                    id: req.id,
                    text: piece,
                })?;
            }
            batch.clear();
            batch.add(token, pos, &[0], true)?;
            pos += 1;
            ctx.decode(&mut batch)?;
        }
        tx.send(LlmEvent::Done { id: req.id })?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// NVIDIA NIM worker (feature = "nim")
// ---------------------------------------------------------------------------

/// Talks to an NVIDIA NIM endpoint, which speaks the OpenAI-compatible
/// `/chat/completions` protocol. There is no model to load — a hosted endpoint
/// is "ready" as soon as we have an API key — so `Load` just reports status and
/// each `Chat` streams a Server-Sent-Events response, reusing the same
/// [`ReplyFilter`] and [`system_prompt`] as the local backend.
#[cfg(feature = "nim")]
mod nim {
    use super::*;
    use std::io::{BufRead, BufReader};

    pub(super) fn worker(rx: Receiver<WorkerMsg>, tx: Sender<LlmEvent>) {
        let mut settings = LlmSettings::default();
        while let Ok(msg) = rx.recv() {
            match msg {
                WorkerMsg::Load(s) => {
                    settings = s;
                    match api_key(&settings) {
                        Some(_) => {
                            tx.send(LlmEvent::Status(LlmStatus::Ready(
                                settings.nim_model.clone(),
                            )))
                            .ok();
                        }
                        None => {
                            tx.send(LlmEvent::Status(LlmStatus::Error(
                                "no NVIDIA API key (set it in the LLM tab or the \
                                 NVIDIA_API_KEY environment variable)"
                                    .into(),
                            )))
                            .ok();
                        }
                    }
                }
                WorkerMsg::Chat(req) => {
                    if let Err(e) = run_chat(&settings, &req, &tx) {
                        tx.send(LlmEvent::Error {
                            id: req.id,
                            msg: e.to_string(),
                        })
                        .ok();
                    }
                }
                WorkerMsg::Generate(req) => {
                    if let Err(e) = run_generate(&settings, &req, &tx) {
                        tx.send(LlmEvent::Error {
                            id: req.id,
                            msg: e.to_string(),
                        })
                        .ok();
                    }
                }
            }
        }
    }

    /// The API key from settings, falling back to the `NVIDIA_API_KEY` env var.
    fn api_key(settings: &LlmSettings) -> Option<String> {
        if !settings.nim_api_key.is_empty() {
            return Some(settings.nim_api_key.clone());
        }
        std::env::var("NVIDIA_API_KEY")
            .ok()
            .filter(|k| !k.is_empty())
    }

    fn run_chat(
        settings: &LlmSettings,
        req: &ChatRequest,
        tx: &Sender<LlmEvent>,
    ) -> anyhow::Result<()> {
        let key = api_key(settings).ok_or_else(|| anyhow::anyhow!("no NVIDIA API key"))?;
        let url = format!(
            "{}/chat/completions",
            settings.nim_base_url.trim_end_matches('/')
        );

        // OpenAI-style request body with streaming enabled.
        let messages: Vec<serde_json::Value> = super::build_chat(req)
            .into_iter()
            .map(|(role, content)| serde_json::json!({ "role": role, "content": content }))
            .collect();
        let body = serde_json::json!({
            "model": settings.nim_model,
            "messages": messages,
            "temperature": req.temperature.clamp(0.05, 2.0),
            "max_tokens": req.max_tokens,
            "stream": true,
        });

        let resp = match ureq::post(&url)
            .set("Authorization", &format!("Bearer {key}"))
            .set("Accept", "text/event-stream")
            .send_json(body)
        {
            Ok(r) => r,
            // Surface the endpoint's own error message (bad key, unknown model…).
            Err(ureq::Error::Status(code, r)) => {
                let detail = r.into_string().unwrap_or_default();
                anyhow::bail!("NIM HTTP {code}: {}", detail.trim());
            }
            Err(e) => anyhow::bail!("NIM request failed: {e}"),
        };

        // Parse the SSE stream: `data: {json}` lines terminated by `data: [DONE]`.
        // Each chunk carries an incremental `choices[0].delta.content` piece.
        let mut filter = ReplyFilter::new();
        let reader = BufReader::new(resp.into_reader());
        for line in reader.lines() {
            let payload = match line?.strip_prefix("data:") {
                Some(p) => p.trim().to_string(),
                None => continue,
            };
            if payload.is_empty() {
                continue;
            }
            if payload == "[DONE]" {
                break;
            }
            let Ok(chunk) = serde_json::from_str::<serde_json::Value>(&payload) else {
                continue;
            };
            let delta = chunk["choices"][0]["delta"]["content"]
                .as_str()
                .unwrap_or("");
            if delta.is_empty() {
                continue;
            }
            let visible = filter.push(delta);
            if !visible.is_empty() {
                tx.send(LlmEvent::Token {
                    id: req.id,
                    text: visible,
                })?;
            }
            // NPC lines are short; stop at a blank line in the visible reply.
            if filter.shown().contains("\n\n") {
                break;
            }
        }
        let tail = filter.finish();
        if !tail.is_empty() {
            tx.send(LlmEvent::Token {
                id: req.id,
                text: tail,
            })?;
        }
        tx.send(LlmEvent::Done { id: req.id })?;
        Ok(())
    }

    /// Content generation over the same OpenAI-compatible streaming endpoint,
    /// but the delta pieces are forwarded raw (no [`ReplyFilter`]) so the JSON
    /// body is preserved.
    fn run_generate(
        settings: &LlmSettings,
        req: &GenRequest,
        tx: &Sender<LlmEvent>,
    ) -> anyhow::Result<()> {
        let key = api_key(settings).ok_or_else(|| anyhow::anyhow!("no NVIDIA API key"))?;
        let url = format!(
            "{}/chat/completions",
            settings.nim_base_url.trim_end_matches('/')
        );

        let messages: Vec<serde_json::Value> = super::build_gen_chat(req)
            .into_iter()
            .map(|(role, content)| serde_json::json!({ "role": role, "content": content }))
            .collect();
        let body = serde_json::json!({
            "model": settings.nim_model,
            "messages": messages,
            "temperature": req.temperature.clamp(0.05, 2.0),
            "max_tokens": req.max_tokens,
            "stream": true,
        });

        let resp = match ureq::post(&url)
            .set("Authorization", &format!("Bearer {key}"))
            .set("Accept", "text/event-stream")
            .send_json(body)
        {
            Ok(r) => r,
            Err(ureq::Error::Status(code, r)) => {
                let detail = r.into_string().unwrap_or_default();
                anyhow::bail!("NIM HTTP {code}: {}", detail.trim());
            }
            Err(e) => anyhow::bail!("NIM request failed: {e}"),
        };

        let reader = BufReader::new(resp.into_reader());
        for line in reader.lines() {
            let payload = match line?.strip_prefix("data:") {
                Some(p) => p.trim().to_string(),
                None => continue,
            };
            if payload.is_empty() {
                continue;
            }
            if payload == "[DONE]" {
                break;
            }
            let Ok(chunk) = serde_json::from_str::<serde_json::Value>(&payload) else {
                continue;
            };
            let delta = chunk["choices"][0]["delta"]["content"]
                .as_str()
                .unwrap_or("");
            if delta.is_empty() {
                continue;
            }
            tx.send(LlmEvent::Token {
                id: req.id,
                text: delta.to_string(),
            })?;
        }
        tx.send(LlmEvent::Done { id: req.id })?;
        Ok(())
    }
}

#[cfg(test)]
mod filter_tests {
    use super::*;

    /// Feed a whole reply through the filter one piece at a time and collect the
    /// user-facing result. `chunk` controls how finely the input is split, to
    /// exercise tags that straddle token boundaries.
    fn run(input: &str, chunk: usize) -> String {
        let mut f = ReplyFilter::new();
        let mut out = String::new();
        let bytes = input.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            // Advance on char boundaries so we never split a UTF-8 sequence.
            let mut end = (i + chunk.max(1)).min(bytes.len());
            while end < bytes.len() && !input.is_char_boundary(end) {
                end += 1;
            }
            out.push_str(&f.push(&input[i..end]));
            i = end;
        }
        out.push_str(&f.finish());
        out
    }

    #[test]
    fn plain_text_passes_through() {
        assert_eq!(run("Well met, traveller!", 3), "Well met, traveller!");
    }

    #[test]
    fn hides_analysis_shows_final() {
        let raw = "<|channel|>analysis<|message|>They greeted me, be warm.<|end|>\
                   <|start|>assistant<|channel|>final<|message|>Well met, traveller!<|return|>";
        assert_eq!(run(raw, 5), "Well met, traveller!");
    }

    #[test]
    fn tags_split_across_pieces() {
        let raw = "<|channel|>final<|message|>Hello there.<|return|>";
        // A chunk size of 1 splits every tag across many pieces.
        assert_eq!(run(raw, 1), "Hello there.");
    }

    #[test]
    fn newline_terminated_channel_name() {
        let raw = "<|channel|>thought\nMy knees ache.\n<|channel|>final\nI'm not going anywhere.";
        assert_eq!(run(raw, 4), "I'm not going anywhere.");
    }

    #[test]
    fn unlabeled_channel_is_visible() {
        let raw = "<|channel|>thought<|message|>hmm<|end|><|channel|><|message|>Here you go.";
        assert_eq!(run(raw, 7), "Here you go.");
    }

    #[test]
    fn falls_back_when_no_visible_channel() {
        let raw = "<|channel|>analysis<|message|>Only thinking here.<|end|>";
        assert_eq!(run(raw, 6), "Only thinking here.");
    }

    #[test]
    fn lone_angle_bracket_is_text() {
        assert_eq!(run("3 < 5 is true", 2), "3 < 5 is true");
    }

    /// Gemma-4's dialect: `<|channel>` opens, `<name|>` closes, and the reply
    /// is bare text after the closer (the leak seen in NPC chat bubbles).
    #[test]
    fn gemma4_asymmetric_tags_with_reasoning() {
        let raw =
            "<|channel>thought\nThe player approaches; be gruff.\n<channel|>Hey. What do you want?";
        for chunk in [1, 3, 64] {
            assert_eq!(run(raw, chunk), "Hey. What do you want?");
        }
    }

    #[test]
    fn gemma4_empty_thought_channel() {
        let raw = "<|channel>thought\n<channel|>Hey. What do you want?";
        assert_eq!(run(raw, 2), "Hey. What do you want?");
    }

    #[test]
    fn gemma4_reasoning_only_falls_back() {
        let raw = "<|channel>thought\nOnly musing here.\n<channel|>";
        assert_eq!(run(raw, 4), "Only musing here.");
    }

    #[test]
    fn angle_bracket_word_is_text() {
        assert_eq!(run("I <3 slimes", 2), "I <3 slimes");
    }

    /// The leak from the screenshot: the model degrades into repeated
    /// HTML-ish self-closing tags instead of channel markup.
    #[test]
    fn html_self_closing_thought_tags_are_swallowed() {
        let raw = "<div style=\"thought\" />\n<div style=\"thought\" />\nFine, follow me.";
        for chunk in [1, 4, 64] {
            assert_eq!(run(raw, chunk), "Fine, follow me.");
        }
    }

    /// DeepSeek-R1 style `<think>…</think>` reasoning block.
    #[test]
    fn think_block_is_hidden() {
        let raw = "<think>\nThe player is rude; stay calm.\n</think>\nWatch your tongue.";
        for chunk in [1, 5, 64] {
            assert_eq!(run(raw, chunk), "Watch your tongue.");
        }
    }

    #[test]
    fn think_only_reply_falls_back() {
        let raw = "<think>Just musing, no reply.</think>";
        assert_eq!(run(raw, 3), "Just musing, no reply.");
    }

    #[test]
    fn html_div_thought_block_is_hidden() {
        let raw = "<div class=\"thought\">Should I trust them?</div>Aye, come in.";
        assert_eq!(run(raw, 4), "Aye, come in.");
    }

    /// Gemma's real turn markers, in case they leak through detokenization.
    #[test]
    fn gemma_turn_tags_are_stripped() {
        let raw = "<start_of_turn>model\nGood morning to you.<end_of_turn>";
        assert_eq!(run(raw, 3), "Good morning to you.");
    }

    /// The leak from the screenshot: an improvised boundary tag mid-reply.
    /// Anything shaped like `<start_of_*>`/`<end_of_*>` is swallowed, while
    /// the dialogue on both sides of it stays.
    #[test]
    fn improvised_boundary_tags_are_swallowed() {
        let raw = "for real? I'm going to a store.\n<end_of_action>\nSo leave me be.";
        for chunk in [1, 4, 64] {
            assert_eq!(
                run(raw, chunk),
                "for real? I'm going to a store.\n\nSo leave me be."
            );
        }
        assert_eq!(run("<start_of_reply>Hello there.", 3), "Hello there.");
    }

    /// Unknown bare tags without attributes stay untouched — they may be
    /// legitimate dialogue.
    #[test]
    fn unknown_bare_tag_is_text() {
        assert_eq!(
            run("press <Enter> to continue", 2),
            "press <Enter> to continue"
        );
    }

    /// A truncated markup tag at the very end of generation is dropped
    /// (the newline before it is ordinary text and stays).
    #[test]
    fn truncated_html_tag_is_dropped() {
        assert_eq!(run("Move along.\n<div style=\"thou", 4), "Move along.\n");
    }

    /// The leak from the screenshot: the model tries to "use a tool" and emits
    /// a fenced JSON blob instead of dialogue. The whole fence is swallowed.
    #[test]
    fn fenced_json_tool_call_is_swallowed() {
        let raw = "```json\n{\n  \"action\": \"result\",\n  \"status\": \"unknown\"\n}\n```";
        for chunk in [1, 3, 7, 64] {
            assert_eq!(run(raw, chunk), "");
        }
    }

    /// Dialogue surrounding a fenced block survives; only the fence is hidden.
    #[test]
    fn text_around_fence_survives() {
        let raw = "Here you go.\n```json\n{\"x\": 1}\n```\nAnything else?";
        for chunk in [1, 4, 64] {
            assert_eq!(run(raw, chunk), "Here you go.\n\nAnything else?");
        }
    }

    /// One or two backticks are ordinary text (inline emphasis), not a fence.
    #[test]
    fn stray_backticks_are_text() {
        assert_eq!(run("it's a `test` word", 2), "it's a `test` word");
    }

    /// A fence that never closes swallows everything after it.
    #[test]
    fn unclosed_fence_is_swallowed() {
        assert_eq!(run("Sure thing.\n```\n{\"a\":1}", 5), "Sure thing.\n");
    }
}

#[cfg(all(test, feature = "llm"))]
mod tests {
    use super::*;

    /// Load the real GGUF model and wait for it to be ready, or return `None`
    /// (skip) when it isn't downloaded. Shared by the end-to-end tests.
    fn ready_engine() -> Option<LlmEngine> {
        let path = std::env::var("NOM_TEST_MODEL")
            .unwrap_or_else(|_| "models/qwen2.5-0.5b-instruct-q4_k_m.gguf".into());
        if !std::path::Path::new(&path).exists() {
            eprintln!("model not downloaded; skipping (run scripts/get-model.sh)");
            return None;
        }
        let mut engine = LlmEngine::new();
        engine.configure(&crate::core::data::LlmSettings {
            model_path: path,
            ..Default::default()
        });
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
        while !engine.ready() {
            engine.poll();
            if let LlmStatus::Error(e) = &engine.status {
                panic!("model failed to load: {e}");
            }
            assert!(std::time::Instant::now() < deadline, "model load timed out");
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        Some(engine)
    }

    /// Drive one generation request to completion and return the raw reply.
    fn run_gen(engine: &mut LlmEngine, req: GenRequest) -> String {
        let id = engine.generate(req).expect("generation accepted");
        let mut out = String::new();
        let mut done = false;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(240);
        while !done {
            for ev in engine.poll() {
                match ev {
                    LlmEvent::Token { id: i, text } if i == id => out.push_str(&text),
                    LlmEvent::Done { id: i } if i == id => done = true,
                    LlmEvent::Error { msg, .. } => panic!("generation failed: {msg}"),
                    _ => {}
                }
            }
            assert!(std::time::Instant::now() < deadline, "generation timed out");
            std::thread::sleep(std::time::Duration::from_millis(30));
        }
        out
    }

    /// End-to-end: ask the real model to author skills and a map, then parse the
    /// output through [`crate::core::aigen`] and require valid content.
    /// Ignored by default — run with `cargo test gen_end_to_end -- --ignored`.
    #[test]
    #[ignore]
    fn gen_end_to_end() {
        use crate::core::aigen::{self, GenTarget};
        let Some(mut engine) = ready_engine() else {
            return;
        };
        let mut project = crate::core::defaults::default_project(Language::English);

        // Skills: a batch of new entries appended after the defaults.
        let before = project.skills.len();
        let req = GenRequest {
            id: 0,
            system: aigen::system_prompt(GenTarget::Skills, &project, Language::English),
            prompt: aigen::user_prompt(GenTarget::Skills, 3, "a set of wind and earth spells"),
            max_tokens: GenTarget::Skills.max_tokens(3),
            temperature: 0.4,
        };
        let raw = run_gen(&mut engine, req);
        let applied = aigen::apply(GenTarget::Skills, &mut project, &raw)
            .unwrap_or_else(|e| panic!("skill JSON rejected: {e}\n---\n{raw}"));
        assert!(project.skills.len() > before, "no skills were added");
        assert!(applied.summary.starts_with("Added"));

        // Map: decodes into a real grid the editor can open.
        let req = GenRequest {
            id: 0,
            system: aigen::system_prompt(GenTarget::Map, &project, Language::English),
            prompt: aigen::user_prompt(GenTarget::Map, 1, "a small grassy clearing with a pond"),
            max_tokens: GenTarget::Map.max_tokens(1),
            temperature: 0.4,
        };
        let raw = run_gen(&mut engine, req);
        let applied = aigen::apply(GenTarget::Map, &mut project, &raw)
            .unwrap_or_else(|e| panic!("map JSON rejected: {e}\n---\n{raw}"));
        let id = applied.new_map.expect("a new map id");
        let m = project.map(id).expect("map present");
        assert_eq!(m.tiles.len(), (m.width * m.height) as usize);
        println!("generated map {}×{} '{}'", m.width, m.height, m.name);
    }

    /// End-to-end: load the real GGUF model (if present in ./models), run a
    /// persona chat, and require a streamed in-character reply.
    /// Ignored by default — run with `cargo test llm_end_to_end -- --ignored`.
    /// Set `NOM_TEST_MODEL=/path/to/model.gguf` to test another model.
    #[test]
    #[ignore]
    fn llm_end_to_end() {
        let path = std::env::var("NOM_TEST_MODEL")
            .unwrap_or_else(|_| "models/qwen2.5-0.5b-instruct-q4_k_m.gguf".into());
        let model = std::path::Path::new(&path);
        if !model.exists() {
            eprintln!("model not downloaded; skipping (run scripts/get-model.sh)");
            return;
        }
        let mut engine = LlmEngine::new();
        engine.configure(&crate::core::data::LlmSettings {
            model_path: model.display().to_string(),
            ..Default::default()
        });

        // Wait for the model to load.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
        while !engine.ready() {
            engine.poll();
            if let LlmStatus::Error(e) = &engine.status {
                panic!("model failed to load: {e}");
            }
            assert!(std::time::Instant::now() < deadline, "model load timed out");
            std::thread::sleep(std::time::Duration::from_millis(50));
        }

        let persona = crate::core::data::NpcPersona {
            name: "Old Marta".into(),
            role: "the village apothecary".into(),
            personality: "Warm, folksy, calls everyone 'dearie'.".into(),
            knowledge: "The cave north of the village is full of slimes.".into(),
            constraints: String::new(),
            fallback_lines: vec![],
            use_llm: true,
        };
        let id = engine
            .request(ChatRequest {
                id: 0,
                persona,
                game_title: "Untitled Tale".into(),
                location: "Riverside Meadow".into(),
                player_name: "Aldric".into(),
                history: vec![ChatTurn {
                    from_player: true,
                    text: "What's in the cave to the north?".into(),
                }],
                max_tokens: 64,
                temperature: 0.8,
                language: Language::default(),
            })
            .expect("request accepted");

        let mut reply = String::new();
        let mut done = false;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(180);
        while !done {
            for ev in engine.poll() {
                match ev {
                    LlmEvent::Token { id: i, text } if i == id => reply.push_str(&text),
                    LlmEvent::Done { id: i } if i == id => done = true,
                    LlmEvent::Error { msg, .. } => panic!("generation failed: {msg}"),
                    _ => {}
                }
            }
            assert!(std::time::Instant::now() < deadline, "generation timed out");
            std::thread::sleep(std::time::Duration::from_millis(30));
        }
        println!("NPC reply: {reply}");
        assert!(
            reply.trim().len() > 5,
            "expected a real reply, got: {reply:?}"
        );
        assert!(
            !reply.contains("<|") && !reply.contains("|>"),
            "control markup leaked into the reply: {reply:?}"
        );
    }
}

#[cfg(all(test, feature = "nim"))]
mod nim_tests {
    use super::*;
    use crate::core::data::LlmBackend;

    /// End-to-end: hit the real NVIDIA NIM endpoint and require a streamed
    /// in-character reply. Ignored by default (needs network + a key).
    /// Run with `cargo test nim_end_to_end -- --ignored`, with `NVIDIA_API_KEY`
    /// set (or `NOM_TEST_NIM_MODEL` to try another model).
    #[test]
    #[ignore]
    fn nim_end_to_end() {
        if std::env::var("NVIDIA_API_KEY")
            .map(|k| k.is_empty())
            .unwrap_or(true)
        {
            eprintln!("NVIDIA_API_KEY not set; skipping");
            return;
        }
        let model = std::env::var("NOM_TEST_NIM_MODEL")
            .unwrap_or_else(|_| "meta/llama-3.1-8b-instruct".into());

        let mut engine = LlmEngine::new();
        engine.configure(&crate::core::data::LlmSettings {
            backend: LlmBackend::Nim,
            nim_model: model,
            ..Default::default()
        });

        // A hosted endpoint reports ready as soon as the worker starts.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !engine.ready() {
            engine.poll();
            if let LlmStatus::Error(e) = &engine.status {
                panic!("NIM backend failed to configure: {e}");
            }
            assert!(std::time::Instant::now() < deadline, "configure timed out");
            std::thread::sleep(std::time::Duration::from_millis(20));
        }

        let persona = crate::core::data::NpcPersona {
            name: "Old Marta".into(),
            role: "the village apothecary".into(),
            personality: "Warm, folksy, calls everyone 'dearie'.".into(),
            knowledge: "The cave north of the village is full of slimes.".into(),
            constraints: String::new(),
            fallback_lines: vec![],
            use_llm: true,
        };
        let id = engine
            .request(ChatRequest {
                id: 0,
                persona,
                game_title: "Untitled Tale".into(),
                location: "Riverside Meadow".into(),
                player_name: "Aldric".into(),
                history: vec![ChatTurn {
                    from_player: true,
                    text: "What's in the cave to the north?".into(),
                }],
                max_tokens: 64,
                temperature: 0.8,
                language: Language::default(),
            })
            .expect("request accepted");

        let mut reply = String::new();
        let mut done = false;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        while !done {
            for ev in engine.poll() {
                match ev {
                    LlmEvent::Token { id: i, text } if i == id => reply.push_str(&text),
                    LlmEvent::Done { id: i } if i == id => done = true,
                    LlmEvent::Error { msg, .. } => panic!("generation failed: {msg}"),
                    _ => {}
                }
            }
            assert!(std::time::Instant::now() < deadline, "generation timed out");
            std::thread::sleep(std::time::Duration::from_millis(30));
        }
        println!("NIM NPC reply: {reply}");
        assert!(
            reply.trim().len() > 5,
            "expected a real reply, got: {reply:?}"
        );
        assert!(
            !reply.contains("<|") && !reply.contains("|>"),
            "control markup leaked into the reply: {reply:?}"
        );
    }
}
