# NewOldMaker

A desktop-native game engine for building **HD-2D turn-based RPGs** in the
style of Octopath Traveler — RPG Maker's workflow, a diorama renderer, and
**local-LLM-driven NPC conversations** that run fully offline.

Written in Rust: wgpu (Vulkan/Metal/DX12) for rendering, egui for the editor
UI, llama.cpp (via `llama-cpp-2`) for NPC dialogue. No external assets
required — all placeholder pixel art is generated procedurally at startup.

![engine](https://img.shields.io/badge/rust-wgpu%20%2B%20egui-orange)

## Features

**HD-2D renderer**
- 3D tile terrain with per-tile elevation, cliff strata, animated water/lava
- Billboarded pixel-art sprites with soft blob shadows
- HDR pipeline: bloom, tilt-shift depth-of-field, vignette, Reinhard tonemap
- Per-map ambience: sun/ambient/fog colors, fog density, darkness (caves!),
  point lights from torches and crystals (up to 32)

**Editor**
- Paint terrain, raise/lower elevation, and place props *directly in the 3D
  viewport* (LMB apply, Ctrl+LMB inverse, RMB orbit, MMB pan, wheel zoom)
- Events: NPCs, signs, transfers, chests, battle triggers, heal points
- Database: actors (stats/growth/learnsets), skills (elements, multi-hit,
  buffs), items, enemies (weaknesses + shields), troops, system settings
- Undo (Ctrl+Z), autosaveable single-file JSON project format

**Playtest (F5)**
- Grid movement with elevation rules, wandering NPCs, random encounters
- Octopath-style battles: speed order, **Boost Points** (bank 1/turn, spend
  up to 3), elemental **weaknesses**, **shield points and Break**
- EXP, level-ups, items, flee, game over

**LLM NPC dialogue**
- Each NPC has a *persona*: role, personality, knowledge, hard constraints,
  and scripted fallback lines
- Two interchangeable backends drive the chat (Database → LLM → Backend):
  - **Local** — a llama.cpp GGUF model, streamed token by token entirely
    on-CPU and offline
  - **NVIDIA NIM** — a hosted, OpenAI-compatible cloud endpoint, driven by
    your NVIDIA API key (no local model needed)
- With a backend configured, talking to an NPC opens a free-form chat — type
  anything and the NPC answers in character, streamed as it generates
- Without a backend (or with `use_llm` off) NPCs cycle their fallback lines

## Building

```sh
cargo build --release              # full engine (local llama.cpp + NIM)
cargo build --release --no-default-features --features nim   # NIM only, no llama.cpp
cargo build --release --no-default-features   # no LLM at all (fallback lines)
cargo run --release
```

Requirements: Rust 1.85+, a Vulkan/Metal/DX12-capable GPU, and for the `llm`
feature a C/C++ toolchain + cmake + libclang (used to build llama.cpp). The
`nim` feature only needs network access at runtime.

> Note: `.cargo/config.toml` sets `BINDGEN_EXTRA_CLANG_ARGS` to gcc's include
> dir because this machine has libclang without clang's resource headers.
> Remove/adjust it on systems where plain clang is installed.

## Getting a dialogue model

```sh
./scripts/get-model.sh    # Qwen2.5-0.5B-Instruct Q4_K_M (~400 MB)
```

Then in the engine: **Database → LLM → Model (.gguf)** and pick the file.
The menu bar shows `LLM: <model>` in green when it's ready. Any small
instruct-tuned GGUF works; 0.3–1.5 B models are the sweet spot for
low-latency village chatter.

## Using NVIDIA NIM instead

To let NPCs talk through NVIDIA's hosted models rather than a local one, set
**Database → LLM → Backend** to *NVIDIA NIM* and fill in:

- **API key** — your `nvapi-…` key. Leave it blank to read the
  `NVIDIA_API_KEY` environment variable instead (handy for not saving the key
  into the project file).
- **Model** — e.g. `meta/llama-3.1-8b-instruct`.
- **Base URL** — defaults to `https://integrate.api.nvidia.com/v1`; point it
  at a self-hosted NIM container if you run one.

The endpoint is OpenAI-compatible, so replies stream in exactly as they do for
the local backend, through the same in-character prompt and markup filter.

## Quick tour

1. `cargo run --release` — opens the sample project: a riverside village and
   the Cave of Whispers.
2. Paint some terrain, raise a hill, drop a few torches (they cast light).
3. Open an NPC with the **Events** tool and read Marta's persona — knowledge,
   constraints, fallback lines.
4. Press **F5**. Walk with WASD, talk with **Z**, fight slimes (they're weak
   to swords and fire — break their shields), take the north exit to the cave.
5. If a model is loaded, ask Marta about the cave, the crystal, her husband —
   she'll improvise within her persona.

## Project format

One pretty-printed JSON file (`*.nom.json`) containing maps (terrain,
elevation, props, events), the full database, system settings, and LLM
settings. Versioned via `format_version`.

## Tests

```sh
cargo test --no-default-features
```

Covers project save/load roundtrips, battle math (weakness → shield chip →
break, boost spending, BP banking, healing rules, a full simulated battle),
and a headless GPU smoke test that compiles every shader/pipeline and renders
a frame.

## Architecture

```
src/
  core/      data model, defaults (sample project), JSON I/O
  gfx/       procedural pixel-art atlas, camera, terrain mesh builder,
             wgpu HD-2D renderer + WGSL shaders (scene, bloom, tilt-shift)
  editor/    3D-viewport map editor, database window
  game/      playtest runtime (movement, events, encounters), battle system
  llm/       llama.cpp worker thread, persona prompts, token streaming
  app.rs     mode switching, menus, dialogue/battle overlays
```
