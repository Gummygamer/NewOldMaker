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
        s.push_str(&format!("Personality and speaking style: {}\n", p.personality.trim()));
    }
    if !p.knowledge.trim().is_empty() {
        s.push_str(&format!("Things you know: {}\n", p.knowledge.trim()));
    }
    if !p.constraints.trim().is_empty() {
        s.push_str(&format!("Hard rules you must follow: {}\n", p.constraints.trim()));
    }
    s.push_str(
        "Stay in character. Speak only as this character would, in plain spoken dialogue \
         (no narration, no quotation marks, no stage directions). Keep replies to one to three short sentences.",
    );
    s
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
                tx.send(LlmEvent::Status(LlmStatus::Error(format!("llama init: {e}")))).ok();
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
                            tx.send(LlmEvent::Status(LlmStatus::Error(format!("load: {e}")))).ok();
                        }
                    }
                }
                WorkerMsg::Chat(req) => {
                    let Some(m) = &model else {
                        tx.send(LlmEvent::Error { id: req.id, msg: "no model loaded".into() }).ok();
                        continue;
                    };
                    if let Err(e) = run_chat(&backend, m, &settings, &req, &tx) {
                        tx.send(LlmEvent::Error { id: req.id, msg: e.to_string() }).ok();
                    }
                }
            }
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
        let mut chat = vec![LlamaChatMessage::new("system".into(), super::system_prompt(req))?];
        if req.history.is_empty() {
            chat.push(LlamaChatMessage::new(
                "user".into(),
                format!("{} walks up to you. Greet them in character.", req.player_name),
            )?);
        }
        for turn in &req.history {
            let role = if turn.from_player { "user" } else { "assistant" };
            chat.push(LlamaChatMessage::new(role.into(), turn.text.clone())?);
        }
        let template = model
            .chat_template(None)
            .unwrap_or_else(|_| LlamaChatTemplate::new("chatml").expect("chatml template"));
        let prompt = model.apply_chat_template(&template, &chat, true)?;

        let mut ctx_params = LlamaContextParams::default()
            .with_n_ctx(NonZeroU32::new(settings.context_tokens.max(512)))
            .with_n_batch(N_BATCH);
        if settings.threads > 0 {
            ctx_params = ctx_params.with_n_threads(settings.threads as i32);
        }
        let mut ctx = model.new_context(backend, ctx_params)?;

        let mut tokens = model.str_to_token(&prompt, AddBos::Never)?;
        // Keep the prompt inside the context window, preserving the start (system prompt).
        let budget = (settings.context_tokens.max(512) as usize).saturating_sub(settings.max_reply_tokens as usize + 8);
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

        let mut generated = String::new();
        // Streaming decoder: tokens can split multi-byte UTF-8 sequences.
        let mut decoder = encoding_rs::UTF_8.new_decoder();
        for _ in 0..req.max_tokens {
            let token = sampler.sample(&ctx, -1);
            sampler.accept(token);
            if model.is_eog_token(token) {
                break;
            }
            let piece = model.token_to_piece(token, &mut decoder, false, None).unwrap_or_default();
            generated.push_str(&piece);
            // NPC lines are short; stop at a blank line.
            if generated.contains("\n\n") {
                break;
            }
            if !piece.is_empty() {
                tx.send(LlmEvent::Token { id: req.id, text: piece })?;
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
        assert!(reply.trim().len() > 5, "expected a real reply, got: {reply:?}");
    }
}
