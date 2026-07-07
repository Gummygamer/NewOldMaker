//! Procedural chiptune sequencer. Each `Track` is described by a small
//! `TrackSpec` (tempo, key, scale, chord progression, timbres); the sequencer
//! generates a looping four-voice arrangement (pad, bass, arpeggio, lead) from
//! it in real time. The lead melody is deterministic — derived by hashing the
//! step index — so a track loops seamlessly without storing a score.

use super::synth::{midi_to_freq, Note, Rng, Wave};
use super::Track;

const MAJOR: &[i32] = &[0, 2, 4, 5, 7, 9, 11];
const MINOR: &[i32] = &[0, 2, 3, 5, 7, 8, 10];
const DORIAN: &[i32] = &[0, 2, 3, 5, 7, 9, 10];

struct TrackSpec {
    seed: u32,
    bpm: f32,
    /// MIDI note of the tonic the arrangement centres on.
    root: f32,
    scale: &'static [i32],
    /// Chord roots per bar, as scale degrees (0 = I, 3 = IV, 4 = V, 5 = vi …).
    progression: &'static [i32],
    lead_wave: Wave,
    bass_wave: Wave,
    /// Chance a given eighth-note slot is a rest rather than a lead note.
    rest_prob: f32,
    /// Bass plays on every eighth note (driving) rather than every quarter.
    bass_eighths: bool,
    arp: bool,
    /// Overall loudness trim for the whole track.
    gain: f32,
}

impl TrackSpec {
    fn for_track(track: Track) -> Option<TrackSpec> {
        Some(match track {
            Track::Silence => return None,
            // Bright, bouncy overworld pop: I–vi–IV–V in C major.
            Track::Field => TrackSpec {
                seed: 0x1111,
                bpm: 128.0,
                root: 60.0,
                scale: MAJOR,
                progression: &[0, 5, 3, 4],
                lead_wave: Wave::Square(0.5),
                bass_wave: Wave::Triangle,
                rest_prob: 0.28,
                bass_eighths: false,
                arp: true,
                gain: 0.9,
            },
            // Gentle, warm village tune in F major.
            Track::Town => TrackSpec {
                seed: 0x2222,
                bpm: 100.0,
                root: 65.0,
                scale: MAJOR,
                progression: &[0, 3, 1, 4],
                lead_wave: Wave::Triangle,
                bass_wave: Wave::Triangle,
                rest_prob: 0.4,
                bass_eighths: false,
                arp: true,
                gain: 0.85,
            },
            // Sparse, brooding minor for caves/dungeons.
            Track::Cave => TrackSpec {
                seed: 0x3333,
                bpm: 82.0,
                root: 57.0,
                scale: MINOR,
                progression: &[0, 5, 3, 4],
                lead_wave: Wave::Triangle,
                bass_wave: Wave::Sine,
                rest_prob: 0.55,
                bass_eighths: false,
                arp: false,
                gain: 0.9,
            },
            // Fast, driving battle theme (dorian gives it bite).
            Track::Battle => TrackSpec {
                seed: 0x4444,
                bpm: 152.0,
                root: 57.0,
                scale: DORIAN,
                progression: &[0, 0, 5, 4],
                lead_wave: Wave::Square(0.4),
                bass_wave: Wave::Square(0.5),
                rest_prob: 0.18,
                bass_eighths: true,
                arp: true,
                gain: 1.0,
            },
        })
    }
}

/// Deterministic hash → `0.0..1.0`, used to pick melody notes per step.
fn frand(a: u32, b: u32) -> f32 {
    let mut x = a.wrapping_mul(0x9E3779B1) ^ b.wrapping_mul(0x85EBCA77);
    x ^= x >> 15;
    x = x.wrapping_mul(0x2C1B3C6D);
    x ^= x >> 12;
    (x & 0xFFFFFF) as f32 / 0xFFFFFF as f32
}

pub struct MusicSeq {
    pub track: Track,
    spec: Option<TrackSpec>,
    step: u32,
    acc: f32,
    step_dur: f32,
    lead: Note,
    bass: Note,
    arp: Note,
    pad: Note,
}

impl MusicSeq {
    pub fn new() -> Self {
        MusicSeq {
            track: Track::Silence,
            spec: None,
            step: 0,
            acc: 0.0,
            step_dur: 0.1,
            lead: Note::new(Wave::Square(0.5)),
            bass: Note::new(Wave::Triangle),
            arp: Note::new(Wave::Square(0.5)),
            pad: Note::new(Wave::Sine),
        }
    }

    pub fn set_track(&mut self, track: Track) {
        if track == self.track {
            return;
        }
        self.track = track;
        self.spec = TrackSpec::for_track(track);
        self.step = 0;
        self.acc = 0.0;
        if let Some(spec) = &self.spec {
            self.step_dur = 60.0 / spec.bpm / 4.0; // one sixteenth note
            self.lead.wave = spec.lead_wave;
            self.bass.wave = spec.bass_wave;
        }
    }

    /// Semitone offset for a scale degree that may span several octaves.
    fn degree_semitone(scale: &[i32], degree: i32) -> f32 {
        let n = scale.len() as i32;
        let oct = degree.div_euclid(n);
        let idx = degree.rem_euclid(n) as usize;
        (scale[idx] + 12 * oct) as f32
    }

    fn on_step(&mut self) {
        let Some(spec) = &self.spec else { return };
        let bars = spec.progression.len() as u32;
        let loop_len = 16 * bars;
        let s = self.step % loop_len;
        let bar = s / 16;
        let beat = s % 16; // 0..15 sixteenths within the bar
        let chord = spec.progression[bar as usize];
        let freq = |degree: i32| midi_to_freq(spec.root + Self::degree_semitone(spec.scale, degree));

        // Pad: soft chord root sustaining the whole bar.
        if beat == 0 {
            self.pad.trigger(freq(chord), 0.06 * spec.gain, self.step_dur * 15.5);
        }

        // Bass: chord root (with a fifth on the off-beats), an octave-ish down.
        let bass_hit = if spec.bass_eighths { beat % 2 == 0 } else { beat % 4 == 0 };
        if bass_hit {
            let degree = if beat % 8 == 0 { chord } else { chord + 4 };
            let gate = if spec.bass_eighths { self.step_dur * 1.6 } else { self.step_dur * 3.2 };
            self.bass.trigger(freq(degree - 7), 0.16 * spec.gain, gate);
        }

        // Arpeggio: cycle chord tones an octave up on every sixteenth.
        if spec.arp {
            let tones = [0, 2, 4, 2];
            let degree = chord + 7 + tones[(beat % 4) as usize];
            self.arp.trigger(freq(degree), 0.05 * spec.gain, self.step_dur * 0.85);
        }

        // Lead: a melody on eighth notes, chord tones on strong beats and
        // passing scale tones on the weak ones, with rests for phrasing.
        if beat % 2 == 0 {
            let r = frand(spec.seed, s);
            if r > spec.rest_prob {
                let strong = beat % 4 == 0;
                let pick = frand(spec.seed ^ 0xABCD, s);
                let degree = if strong {
                    chord + 7 + [0, 2, 4, 7][(pick * 4.0) as usize % 4]
                } else {
                    chord + 7 + [1, 2, 3, 5, 6][(pick * 5.0) as usize % 5]
                };
                let gate = self.step_dur * if strong { 1.7 } else { 1.3 };
                self.lead.trigger(freq(degree), 0.15 * spec.gain, gate);
            }
        }

        self.step += 1;
    }

    #[inline]
    pub fn render(&mut self, dt: f32, rng: &mut Rng) -> f32 {
        if self.spec.is_none() {
            return 0.0;
        }
        self.acc += dt;
        while self.acc >= self.step_dur {
            self.acc -= self.step_dur;
            self.on_step();
        }
        self.pad.render(dt, rng)
            + self.bass.render(dt, rng)
            + self.arp.render(dt, rng)
            + self.lead.render(dt, rng)
    }
}
