//! Procedural audio: real-time synthesized chiptune music and sound effects.
//!
//! Like the rest of NewOldMaker, this ships no assets — everything is generated
//! on the fly from oscillators (see [`synth`]). A single background thread owned
//! by `cpal` mixes a looping music track (chosen per map / for battles) with a
//! pool of one-shot sound-effect voices. The rest of the engine talks to it
//! through the free functions [`music`], [`sfx`], and the mute controls, which
//! post to a small shared control block; when the `audio` feature is off these
//! all compile to no-ops.

use crate::core::data::MapData;

// -----------------------------------------------------------------------------
// Public value types (always available, even without the `audio` feature)
// -----------------------------------------------------------------------------

/// A looping background music track.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Track {
    Silence,
    Field,
    Town,
    Cave,
    Battle,
}

impl Track {
    /// Pick the field track that best fits a map's mood.
    pub fn for_map(map: &MapData) -> Track {
        if map.ambience.darkness > 0.45 {
            return Track::Cave;
        }
        let n = map.name.to_lowercase();
        if n.contains("town") || n.contains("village") || n.contains("inn") || n.contains("home") {
            Track::Town
        } else {
            Track::Field
        }
    }
}

/// Every one-shot sound the game can play.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Sfx {
    /// Menu accept / talk to an NPC / read a sign.
    Confirm,
    /// Menu back / cancel.
    Cancel,
    /// A footstep as the player moves onto a tile.
    Step,
    /// Opening a chest / picking up an item.
    Chest,
    /// A healing pulse (heal points, curative skills/items).
    Heal,
    /// The sting when a battle begins.
    Encounter,
    /// A physical hit landing.
    Hit,
    /// A hit against an elemental weakness (brighter, metallic).
    Weakness,
    /// An enemy's shields shattering (Break).
    Break,
    /// A character gaining a level.
    LevelUp,
    /// Battle won.
    Victory,
    /// The party is defeated.
    Defeat,
    /// Successfully fleeing a battle.
    Flee,
}

// -----------------------------------------------------------------------------
// Real engine (feature = "audio")
// -----------------------------------------------------------------------------

#[cfg(feature = "audio")]
mod music;
#[cfg(feature = "audio")]
mod sfx;
#[cfg(feature = "audio")]
mod synth;

#[cfg(feature = "audio")]
mod engine {
    use std::sync::{Arc, Mutex, OnceLock};

    use super::{Sfx, Track};
    use super::music::MusicSeq;
    use super::synth::{Rng, SfxVoice};

    /// Control block shared between the game (writer) and the audio callback
    /// (reader). Kept tiny so the lock is held only briefly.
    struct Control {
        master: f32,
        muted: bool,
        target: Track,
        pending: Vec<Sfx>,
    }

    impl Default for Control {
        fn default() -> Self {
            Control { master: 0.55, muted: false, target: Track::Silence, pending: Vec::new() }
        }
    }

    static HANDLE: OnceLock<Arc<Mutex<Control>>> = OnceLock::new();

    /// The realtime mixer, owned by the audio callback.
    struct Mixer {
        dt: f32,
        rng: Rng,
        music: MusicSeq,
        music_gain: f32,
        target: Track,
        sfx: Vec<SfxVoice>,
        master: f32,
        muted: bool,
    }

    impl Mixer {
        fn new(sample_rate: f32) -> Self {
            Mixer {
                dt: 1.0 / sample_rate,
                rng: Rng(0x1234_5678),
                music: MusicSeq::new(),
                music_gain: 0.0,
                target: Track::Silence,
                sfx: Vec::new(),
                master: 0.55,
                muted: false,
            }
        }

        fn trigger_sfx(&mut self, s: Sfx) {
            if self.sfx.len() < 24 {
                self.sfx.push(s.voice());
            }
        }

        fn prune(&mut self) {
            self.sfx.retain(|v| !v.done());
        }

        #[inline]
        fn next_sample(&mut self) -> f32 {
            // Crossfade the music voice when the target track changes.
            let fade = self.dt / 0.30;
            if self.music.track != self.target {
                self.music_gain -= fade;
                if self.music_gain <= 0.0 {
                    self.music_gain = 0.0;
                    self.music.set_track(self.target);
                }
            } else if self.music_gain < 1.0 {
                self.music_gain = (self.music_gain + fade).min(1.0);
            }

            let mut s = self.music.render(self.dt, &mut self.rng) * self.music_gain * 0.6;
            for v in &mut self.sfx {
                s += v.render(self.dt, &mut self.rng);
            }
            s *= self.master;
            if self.muted {
                s = 0.0;
            }
            // Soft limiter to keep peaks in range when voices stack up.
            (s * 0.8).tanh()
        }
    }

    /// A live audio stream. Dropping it stops playback, so `App` keeps it alive.
    pub struct AudioStream(#[allow(dead_code)] Option<cpal::Stream>);

    pub fn init() -> AudioStream {
        AudioStream(build().unwrap_or_else(|e| {
            eprintln!("audio disabled: {e}");
            None
        }))
    }

    fn build() -> anyhow::Result<Option<cpal::Stream>> {
        use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

        let host = cpal::default_host();
        let Some(device) = host.default_output_device() else {
            return Ok(None); // headless / no sound card
        };
        let supported = device.default_output_config()?;
        let sample_format = supported.sample_format();
        let config: cpal::StreamConfig = supported.config();
        let sample_rate = config.sample_rate as f32;
        let channels = config.channels as usize;

        let shared = Arc::new(Mutex::new(Control::default()));
        let _ = HANDLE.set(shared.clone());

        let mut mixer = Mixer::new(sample_rate);
        let shared_cb = shared.clone();
        let err_fn = |e| eprintln!("audio stream error: {e}");

        macro_rules! run {
            ($t:ty) => {
                device.build_output_stream(
                    config,
                    move |data: &mut [$t], _: &cpal::OutputCallbackInfo| {
                        {
                            let mut c = shared_cb.lock().unwrap();
                            mixer.master = c.master;
                            mixer.muted = c.muted;
                            mixer.target = c.target;
                            for s in c.pending.drain(..) {
                                mixer.trigger_sfx(s);
                            }
                        }
                        for frame in data.chunks_mut(channels) {
                            let v = mixer.next_sample();
                            let sample = <$t as cpal::Sample>::from_sample(v);
                            for out in frame.iter_mut() {
                                *out = sample;
                            }
                        }
                        mixer.prune();
                    },
                    err_fn,
                    None,
                )?
            };
        }

        let stream = match sample_format {
            cpal::SampleFormat::F32 => run!(f32),
            cpal::SampleFormat::I16 => run!(i16),
            cpal::SampleFormat::U16 => run!(u16),
            other => {
                eprintln!("audio disabled: unsupported sample format {other:?}");
                return Ok(None);
            }
        };
        stream.play()?;
        Ok(Some(stream))
    }

    fn with_control(f: impl FnOnce(&mut Control)) {
        if let Some(h) = HANDLE.get() {
            if let Ok(mut c) = h.lock() {
                f(&mut c);
            }
        }
    }

    pub fn music(track: Track) {
        with_control(|c| c.target = track);
    }

    pub fn sfx(s: Sfx) {
        with_control(|c| {
            if c.pending.len() < 32 {
                c.pending.push(s);
            }
        });
    }

    pub fn toggle_muted() -> bool {
        let mut now = false;
        with_control(|c| {
            c.muted = !c.muted;
            now = c.muted;
        });
        now
    }

    pub fn is_muted() -> bool {
        HANDLE
            .get()
            .and_then(|h| h.lock().ok().map(|c| c.muted))
            .unwrap_or(false)
    }
}

#[cfg(feature = "audio")]
pub use engine::{init, is_muted, music, sfx, toggle_muted, AudioStream};

// -----------------------------------------------------------------------------
// No-op stubs (feature = "audio" disabled)
// -----------------------------------------------------------------------------

#[cfg(not(feature = "audio"))]
mod stub {
    use super::{Sfx, Track};

    /// Placeholder handle; there is nothing to keep alive without the feature.
    pub struct AudioStream;

    pub fn init() -> AudioStream {
        AudioStream
    }
    pub fn music(_track: Track) {}
    pub fn sfx(_s: Sfx) {}
    pub fn toggle_muted() -> bool {
        false
    }
    pub fn is_muted() -> bool {
        false
    }
}

#[cfg(not(feature = "audio"))]
pub use stub::{init, is_muted, music, sfx, toggle_muted, AudioStream};

// -----------------------------------------------------------------------------
// Tests: exercise the DSP without a sound device.
// -----------------------------------------------------------------------------

#[cfg(all(test, feature = "audio"))]
mod tests {
    use super::music::MusicSeq;
    use super::synth::Rng;
    use super::{Sfx, Track};

    const SR: f32 = 44_100.0;

    #[test]
    fn tracks_render_finite_non_silent_audio() {
        for track in [Track::Field, Track::Town, Track::Cave, Track::Battle] {
            let mut seq = MusicSeq::new();
            seq.set_track(track);
            let mut rng = Rng(1);
            let dt = 1.0 / SR;
            let mut energy = 0.0f32;
            for _ in 0..(SR as u32 * 2) {
                let s = seq.render(dt, &mut rng);
                assert!(s.is_finite(), "{track:?} produced a non-finite sample");
                assert!(s.abs() < 8.0, "{track:?} sample out of range: {s}");
                energy += s * s;
            }
            assert!(energy > 0.0, "{track:?} produced only silence");
        }
    }

    #[test]
    fn sfx_voices_are_finite_and_terminate() {
        let all = [
            Sfx::Confirm, Sfx::Cancel, Sfx::Step, Sfx::Chest, Sfx::Heal,
            Sfx::Encounter, Sfx::Hit, Sfx::Weakness, Sfx::Break, Sfx::LevelUp,
            Sfx::Victory, Sfx::Defeat, Sfx::Flee,
        ];
        let dt = 1.0 / SR;
        for s in all {
            let mut voice = s.voice();
            let mut rng = Rng(7);
            let mut energy = 0.0f32;
            let mut frames = 0u32;
            while !voice.done() {
                let x = voice.render(dt, &mut rng);
                assert!(x.is_finite(), "{s:?} produced a non-finite sample");
                energy += x * x;
                frames += 1;
                assert!(frames < SR as u32 * 4, "{s:?} never terminates");
            }
            assert!(energy > 0.0, "{s:?} was silent");
        }
    }
}
