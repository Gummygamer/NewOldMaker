//! The sound-effect catalogue: each `Sfx` variant is synthesized into a
//! `SfxVoice` from a handful of frequency-swept grains. No samples on disk.

use super::synth::{grain, Grain, SfxVoice, Wave};
use super::Sfx;

const SQ: Wave = Wave::Square(0.5);
const TRI: Wave = Wave::Triangle;
const SINE: Wave = Wave::Sine;
const NOISE: Wave = Wave::Noise;

impl Sfx {
    /// Build the voice that renders this effect.
    pub fn voice(self) -> SfxVoice {
        let g: Vec<Grain> = match self {
            Sfx::Confirm => vec![
                grain(0.0, 0.07, 660.0, 660.0, 0.28, SQ, 6.0),
                grain(0.05, 0.10, 990.0, 990.0, 0.28, SQ, 6.0),
            ],
            Sfx::Cancel => vec![grain(0.0, 0.12, 440.0, 220.0, 0.28, SQ, 7.0)],
            Sfx::Step => vec![
                grain(0.0, 0.06, 150.0, 90.0, 0.12, SINE, 22.0),
                grain(0.0, 0.05, 400.0, 200.0, 0.05, NOISE, 30.0),
            ],
            Sfx::Chest => vec![
                grain(0.0, 0.10, 523.0, 523.0, 0.24, TRI, 4.0),
                grain(0.09, 0.10, 659.0, 659.0, 0.24, TRI, 4.0),
                grain(0.18, 0.10, 784.0, 784.0, 0.24, TRI, 4.0),
                grain(0.27, 0.24, 1046.0, 1046.0, 0.26, TRI, 3.0),
            ],
            Sfx::Heal => vec![
                grain(0.0, 0.45, 700.0, 1400.0, 0.20, SINE, 2.2),
                grain(0.08, 0.40, 1050.0, 2100.0, 0.10, SINE, 2.6),
            ],
            Sfx::Encounter => vec![
                grain(0.0, 0.30, 200.0, 1400.0, 0.28, SQ, 1.5),
                grain(0.30, 0.35, 1400.0, 180.0, 0.30, Wave::Saw, 2.0),
                grain(0.30, 0.30, 400.0, 100.0, 0.30, NOISE, 4.0),
            ],
            Sfx::Hit => vec![
                grain(0.0, 0.10, 220.0, 70.0, 0.34, SQ, 10.0),
                grain(0.0, 0.08, 800.0, 200.0, 0.26, NOISE, 16.0),
            ],
            Sfx::Weakness => vec![
                grain(0.0, 0.10, 260.0, 80.0, 0.30, SQ, 10.0),
                grain(0.0, 0.09, 1000.0, 300.0, 0.24, NOISE, 14.0),
                grain(0.02, 0.16, 1600.0, 1900.0, 0.22, SQ, 5.0),
            ],
            Sfx::Break => vec![
                grain(0.0, 0.55, 2000.0, 120.0, 0.34, NOISE, 3.0),
                grain(0.0, 0.20, 300.0, 80.0, 0.32, SQ, 6.0),
                grain(0.05, 0.30, 900.0, 250.0, 0.20, Wave::Saw, 4.0),
            ],
            Sfx::LevelUp => vec![
                grain(0.0, 0.10, 523.0, 523.0, 0.26, SQ, 4.0),
                grain(0.10, 0.10, 659.0, 659.0, 0.26, SQ, 4.0),
                grain(0.20, 0.10, 784.0, 784.0, 0.26, SQ, 4.0),
                grain(0.30, 0.12, 1046.0, 1046.0, 0.26, SQ, 4.0),
                grain(0.42, 0.30, 1318.0, 1318.0, 0.28, SQ, 2.5),
            ],
            Sfx::Victory => vec![
                grain(0.0, 0.14, 784.0, 784.0, 0.28, SQ, 3.0),
                grain(0.14, 0.14, 784.0, 784.0, 0.28, SQ, 3.0),
                grain(0.28, 0.40, 1046.0, 1046.0, 0.30, SQ, 1.8),
                grain(0.0, 0.68, 392.0, 392.0, 0.16, TRI, 1.2),
            ],
            Sfx::Defeat => vec![
                grain(0.0, 0.50, 440.0, 330.0, 0.26, TRI, 1.5),
                grain(0.30, 0.60, 330.0, 165.0, 0.26, TRI, 1.4),
                grain(0.30, 0.60, 220.0, 110.0, 0.18, SINE, 1.4),
            ],
            Sfx::Flee => vec![
                grain(0.0, 0.22, 300.0, 1200.0, 0.20, SQ, 2.5),
                grain(0.0, 0.22, 600.0, 2400.0, 0.10, NOISE, 6.0),
            ],
        };
        SfxVoice::new(g)
    }
}
