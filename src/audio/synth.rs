//! Low-level synthesis primitives: oscillators, a tiny noise RNG, an
//! envelope-gated musical note, and a multi-grain sound-effect voice. All
//! sound in NewOldMaker is generated from these — there are no audio assets.

use std::f32::consts::TAU;

/// Oscillator waveforms. `Square` carries its pulse-width (duty) in `0..1`.
#[derive(Clone, Copy)]
pub enum Wave {
    Square(f32),
    Triangle,
    Saw,
    Sine,
    Noise,
}

/// Fast xorshift PRNG, used for noise waveforms.
pub struct Rng(pub u32);

impl Rng {
    #[inline]
    pub fn next_u32(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        x
    }
    /// White noise in `-1.0..=1.0`.
    #[inline]
    pub fn white(&mut self) -> f32 {
        (self.next_u32() as f32 / u32::MAX as f32) * 2.0 - 1.0
    }
}

/// Sample an oscillator. `phase` is the cycle position in `0..1`.
#[inline]
pub fn osc(wave: Wave, phase: f32, rng: &mut Rng) -> f32 {
    match wave {
        Wave::Square(duty) => {
            if phase < duty {
                1.0
            } else {
                -1.0
            }
        }
        Wave::Triangle => 4.0 * (phase - 0.5).abs() - 1.0,
        Wave::Saw => 2.0 * phase - 1.0,
        Wave::Sine => (phase * TAU).sin(),
        Wave::Noise => rng.white(),
    }
}

/// Convert a MIDI note number to frequency in Hz (A4 = 69 = 440 Hz).
#[inline]
pub fn midi_to_freq(midi: f32) -> f32 {
    440.0 * 2f32.powf((midi - 69.0) / 12.0)
}

// ---------------------------------------------------------------------------
// Musical note voice (one oscillator with an AD-S-R gate)
// ---------------------------------------------------------------------------

/// A monophonic voice: retriggering overwrites whatever was playing. Used by
/// the music sequencer for lead / bass / arpeggio / pad lines.
pub struct Note {
    pub wave: Wave,
    freq: f32,
    level: f32,
    phase: f32,
    t: f32,
    gate: f32,
    atk: f32,
    rel: f32,
    active: bool,
}

impl Note {
    pub fn new(wave: Wave) -> Self {
        Note {
            wave,
            freq: 0.0,
            level: 0.0,
            phase: 0.0,
            t: 0.0,
            gate: 0.0,
            atk: 0.006,
            rel: 0.08,
            active: false,
        }
    }

    /// Start a new note. `gate` is how long the key is held (seconds).
    pub fn trigger(&mut self, freq: f32, level: f32, gate: f32) {
        self.freq = freq;
        self.level = level;
        self.gate = gate;
        self.t = 0.0;
        self.active = true;
        // Phase is intentionally *not* reset, to avoid clicks between notes.
    }

    #[inline]
    pub fn render(&mut self, dt: f32, rng: &mut Rng) -> f32 {
        if !self.active {
            return 0.0;
        }
        self.t += dt;
        // Attack ramp, gentle exponential decay toward a sustain floor, then
        // a linear release once the key lifts.
        let sustain = |x: f32| 0.25 + 0.75 * (-2.2 * (x - self.atk)).exp();
        let amp = if self.t < self.atk {
            self.t / self.atk
        } else if self.t < self.gate {
            sustain(self.t)
        } else {
            let r = (self.t - self.gate) / self.rel;
            if r >= 1.0 {
                self.active = false;
                return 0.0;
            }
            sustain(self.gate) * (1.0 - r)
        };
        self.phase = (self.phase + self.freq * dt).fract();
        osc(self.wave, self.phase, rng) * amp * self.level
    }
}

// ---------------------------------------------------------------------------
// Sound-effect voice (a bag of frequency-swept grains)
// ---------------------------------------------------------------------------

/// One layer of a sound effect: a frequency sweep with an exponential decay.
#[derive(Clone, Copy)]
pub struct Grain {
    pub delay: f32,
    pub dur: f32,
    pub f0: f32,
    pub f1: f32,
    pub level: f32,
    pub wave: Wave,
    /// Exponential decay rate; larger = punchier.
    pub decay: f32,
}

/// Convenience constructor for a grain with a short default attack.
pub fn grain(delay: f32, dur: f32, f0: f32, f1: f32, level: f32, wave: Wave, decay: f32) -> Grain {
    Grain { delay, dur, f0, f1, level, wave, decay }
}

pub struct SfxVoice {
    grains: Vec<Grain>,
    phase: Vec<f32>,
    t: f32,
    life: f32,
}

impl SfxVoice {
    pub fn new(grains: Vec<Grain>) -> Self {
        let life = grains
            .iter()
            .map(|g| g.delay + g.dur)
            .fold(0.0f32, f32::max);
        let phase = vec![0.0; grains.len()];
        SfxVoice { grains, phase, t: 0.0, life }
    }

    #[inline]
    pub fn done(&self) -> bool {
        self.t > self.life
    }

    #[inline]
    pub fn render(&mut self, dt: f32, rng: &mut Rng) -> f32 {
        self.t += dt;
        let mut s = 0.0;
        for (i, g) in self.grains.iter().enumerate() {
            let u = self.t - g.delay;
            if u < 0.0 || u > g.dur {
                continue;
            }
            let freq = g.f0 + (g.f1 - g.f0) * (u / g.dur);
            self.phase[i] = (self.phase[i] + freq * dt).fract();
            let atk = 0.004;
            let env = if u < atk {
                u / atk
            } else {
                (-g.decay * (u - atk)).exp()
            };
            s += osc(g.wave, self.phase[i], rng) * env * g.level;
        }
        s
    }
}
