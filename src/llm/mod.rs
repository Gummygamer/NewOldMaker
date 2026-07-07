//! Local LLM NPC dialogue. A worker thread owns the llama.cpp model; the UI
//! sends `ChatRequest`s and receives streamed tokens via channels, so the
//! frame loop never blocks. Built without the `llm` feature, everything
//! degrades to the personas' scripted fallback lines.

use std::sync::mpsc::{Receiver, Sender, TryRecvError};

use crate::core::data::{LlmSettings, NpcPersona};

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
}

pub struct LlmEngine {
    to_worker: Option<Sender<WorkerMsg>>,
    from_worker: Option<Receiver<LlmEvent>>,
    pub status: LlmStatus,
    next_id: u64,
    configured_path: String,
}

impl LlmEngine {
    pub fn new() -> Self {
        LlmEngine {
            to_worker: None,
            from_worker: None,
            status: LlmStatus::Off,
            next_id: 1,
            configured_path: String::new(),
        }
    }

    /// Ensure the worker matches the project settings (loads/reloads the model).
    pub fn configure(&mut self, settings: &LlmSettings) {
        if settings.model_path == self.configured_path {
            return;
        }
        self.configured_path = settings.model_path.clone();
        if settings.model_path.is_empty() {
            self.to_worker = None;
            self.from_worker = None;
            self.status = LlmStatus::Off;
            return;
        }
        #[cfg(feature = "llm")]
        {
            let (tx_req, rx_req) = std::sync::mpsc::channel::<WorkerMsg>();
            let (tx_ev, rx_ev) = std::sync::mpsc::channel::<LlmEvent>();
            std::thread::Builder::new()
                .name("nom-llm".into())
                .spawn(move || backend::worker(rx_req, tx_ev))
                .expect("spawn llm worker");
            tx_req.send(WorkerMsg::Load(settings.clone())).ok();
            self.to_worker = Some(tx_req);
            self.from_worker = Some(rx_ev);
            self.status = LlmStatus::Loading;
        }
        #[cfg(not(feature = "llm"))]
        {
            self.status = LlmStatus::Error("engine built without the `llm` feature".into());
        }
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
         (no narration, no quotation marks, no stage directions). Keep replies to one to three short sentences.",
    );
    s
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
            let Some(lt) = self.pending.find('<') else {
                let text = std::mem::take(&mut self.pending);
                self.consume_text(&text, &mut out);
                break;
            };
            let before = self.pending[..lt].to_string();
            self.consume_text(&before, &mut out);
            let rest = self.pending[lt..].to_string();
            if let Some((inner, len)) = match_tag(&rest) {
                self.handle_tag(&inner);
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
        // A trailing partial tag never completed — it was real text after all.
        let leftover = std::mem::take(&mut self.pending);
        if !leftover.starts_with("<|") {
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

    fn handle_tag(&mut self, inner: &str) {
        self.harmony = true;
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

/// If `s` starts with a complete `<|…|>` control tag, return its inner text and
/// the byte length consumed.
fn match_tag(s: &str) -> Option<(String, usize)> {
    let rest = s.strip_prefix("<|")?;
    let close = rest.find("|>")?;
    // Reject anything with a newline inside — that's not a real control tag.
    let inner = &rest[..close];
    if inner.contains('\n') {
        return None;
    }
    Some((inner.to_string(), 2 + close + 2))
}

/// Could `s` still grow into a `<|…|>` tag once more pieces arrive?
fn is_partial_tag(s: &str) -> bool {
    "<|".starts_with(s) || (s.starts_with("<|") && !s.contains("|>"))
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
        let mut chat = vec![LlamaChatMessage::new(
            "system".into(),
            super::system_prompt(req),
        )?];
        if req.history.is_empty() {
            chat.push(LlamaChatMessage::new(
                "user".into(),
                format!(
                    "{} walks up to you. Greet them in character.",
                    req.player_name
                ),
            )?);
        }
        for turn in &req.history {
            let role = if turn.from_player {
                "user"
            } else {
                "assistant"
            };
            chat.push(LlamaChatMessage::new(role.into(), turn.text.clone())?);
        }
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
}

#[cfg(all(test, feature = "llm"))]
mod tests {
    use super::*;

    /// End-to-end: load the real GGUF model (if present in ./models), run a
    /// persona chat, and require a streamed in-character reply.
    /// Ignored by default — run with `cargo test llm_end_to_end -- --ignored`.
    #[test]
    #[ignore]
    fn llm_end_to_end() {
        let model = std::path::Path::new("models/qwen2.5-0.5b-instruct-q4_k_m.gguf");
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
    }
}
