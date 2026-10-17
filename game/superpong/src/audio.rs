//! Console sounds (SPEC §12): the C-140's four tones, synthesised at startup
//! and played on a single channel.
//!
//! The model has two halves, fitted once to the reference video (sounds.md
//! §11). The chip's sound pin is a logic square wave: a bit of the vertical
//! line counter, switched on for whole frames, turning the speaker
//! transistor on and off ([`drive`]). What the video heard is that drive
//! through the console's small speaker and the room, modelled as three
//! analog sections shared by all four sounds ([`speaker`]). The sections run
//! at [`OVERSAMPLE`] times the output rate on an exactly area-sampled drive,
//! and each output sample is the mean of its sub-samples, so the result is
//! band-limited and bit-for-bit deterministic.
//!
//! Which tone an event plays is the pure function [`Tone::for_event`];
//! [`Sounds`] only loads and plays, scaled by the master volume
//! ([`volume_gain`], SPEC §13.6). Never panics: if audio is unavailable
//! macroquad's own calls just no-op (see research notes on the "audio"
//! feature / no sound card).

use std::f64::consts::PI;
use std::path::{Path, PathBuf};

use macroquad::audio::{load_sound_from_bytes, play_sound, stop_sound, PlaySoundParams, Sound};
use superpong_core::{SoundEvent, LS_LINES, PLAY_LINES};

/// Output sample rate. quad-snd mixes at 44.1 kHz and resamples anything
/// else nearest-neighbour, so the sounds are made at its rate.
pub const SAMPLE_RATE: u32 = 44_100;

/// Sub-samples per output sample for the drive and the speaker sections.
const OVERSAMPLE: usize = 16;

/// The console's line rate: 3.595295 MHz over 228 clocks per line
/// (SPEC §2.3, §2.4).
const LINE_HZ: f64 = 3_595_295.0 / 228.0;

/// How long each sound runs on after its drive ends, for the speaker's
/// ringing, and the raised-cosine fade over the last part of that.
const TAIL_S: f64 = 0.080;
const FADE_S: f64 = 0.010;

/// Output scale: puts the loudest sound, the Bounce, at about −0.9 dBFS.
const OUTPUT_GAIN: f64 = 0.24;

/// Top of the master volume scale (SPEC §13.6); 0 is silent.
pub const VOLUME_MAX: u8 = 10;
/// Volume at start-up: a little under full, so there is room to turn it up
/// on a quiet speaker.
pub const DEFAULT_VOLUME: u8 = 8;

/// Playback gain of master volume `level` (`0..=`[`VOLUME_MAX`]): the
/// square of its fraction of the scale, so the steps sound roughly even
/// (level 5 is −12 dB, level 1 −40 dB) where a straight line would crowd
/// all the audible change into the bottom steps.
pub fn volume_gain(level: u8) -> f32 {
    let f = f32::from(level.min(VOLUME_MAX)) / f32::from(VOLUME_MAX);
    f * f
}

/// One analog section, `(b2·s² + b1·s + b0) / (a2·s² + a1·s + a0)`, as
/// `[b2, b1, b0, a2, a1, a0]` with s in rad/s.
type Analog = [f64; 6];

/// Angular frequency of `hz`.
fn w(hz: f64) -> f64 {
    2.0 * PI * hz
}

/// The speaker and room, as heard in the reference video (sounds.md §11.2,
/// refit in §11.6). One model for all four sounds; no sound has a gain of
/// its own.
fn speaker() -> [Analog; 3] {
    let (cone, mode, anti, top) = (w(455.0), w(997.0), w(1008.0), w(2400.0));
    [
        // The cone's resonance: a 2nd-order high-pass at 455 Hz, Q 5.3. It
        // gives the Bounce its ringing and leaves little of the 246 Hz
        // fundamental.
        [1.0, 0.0, 0.0, 1.0, cone / 5.3, cone * cone],
        // A mode at 997 Hz (Q 48) under an anti-resonance at 1008 Hz
        // (Q 250). Only the Serve has energy here: it rings on at the mode
        // for about a frame after the chip stops, and while the chip drives
        // it the pair pulls its pitch down to the 981 Hz the video hears.
        [1.0, anti / 250.0, anti * anti, 1.0, mode / 48.0, mode * mode],
        // A resonant low-pass at 2.4 kHz, Q 2.1: the small cone's top end,
        // which lifts the upper harmonics before it cuts them.
        [0.0, 0.0, top * top, 1.0, top / 2.1, top * top],
    ]
}

/// One of the four sounds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Bounce,
    Point,
    Serve,
    AttractTick,
}

impl Tone {
    /// Every tone, in table order; [`Tone::index`] is the position here.
    pub const ALL: [Tone; 4] = [Tone::Bounce, Tone::Point, Tone::Serve, Tone::AttractTick];

    fn index(self) -> usize {
        self as usize
    }

    /// The tone an event plays (SPEC §12.3): every reflection is the Bounce.
    pub fn for_event(event: SoundEvent) -> Tone {
        match event {
            SoundEvent::Bounce(_) => Tone::Bounce,
            SoundEvent::Point => Tone::Point,
            SoundEvent::Serve => Tone::Serve,
            SoundEvent::AttractTick => Tone::AttractTick,
        }
    }

    /// File name for `--export-sounds`.
    pub fn file_name(self) -> &'static str {
        match self {
            Tone::Bounce => "bounce.wav",
            Tone::Point => "point.wav",
            Tone::Serve => "serve.wav",
            Tone::AttractTick => "attract_tick.wav",
        }
    }

    /// Lines per half-period: the tone is a bit of the line counter, so the
    /// pitches are the line rate over 16, 32 and 64 (985.6, 492.8 and
    /// 246.4 Hz).
    fn half_period_lines(self) -> u32 {
        match self {
            Tone::Serve => 8,
            Tone::Bounce => 16,
            Tone::Point | Tone::AttractTick => 32,
        }
    }

    /// Lines per frame while the tone plays: the light show's frame is a
    /// line shorter than a game's (SPEC §2.4).
    fn frame_lines(self) -> u32 {
        match self {
            Tone::AttractTick => LS_LINES as u32,
            _ => PLAY_LINES as u32,
        }
    }

    /// Frames the chip drives the speaker for.
    fn frames(self) -> u32 {
        match self {
            Tone::AttractTick => 5,
            _ => 1,
        }
    }

    /// The drive's length in seconds.
    fn gate_s(self) -> f64 {
        f64::from(self.frames() * self.frame_lines()) / LINE_HZ
    }
}

/// The chip's sound pin for `tone`, as level changes `(line, step)`: the
/// pin turns on at line 0 of a frame and follows its counter bit (on in the
/// first half-period), and the counter restarts every frame, so a
/// multi-frame tone has one long half-period at each frame boundary. Levels
/// are 0 (transistor off, the rest state) and 1.
fn drive(tone: Tone) -> Vec<(u32, f64)> {
    let mut steps = Vec::new();
    let mut level = 0.0;
    for frame in 0..tone.frames() {
        for line in 0..tone.frame_lines() {
            let on = if (line / tone.half_period_lines()).is_multiple_of(2) { 1.0 } else { 0.0 };
            if on != level {
                steps.push((frame * tone.frame_lines() + line, on - level));
                level = on;
            }
        }
    }
    if level != 0.0 {
        steps.push((tone.frames() * tone.frame_lines(), -level));
    }
    steps
}

/// A digital biquad (transposed direct form II), from an [`Analog`] section
/// by the bilinear transform at `fs`.
struct Biquad {
    b: [f64; 3],
    a: [f64; 2],
    s: [f64; 2],
}

impl Biquad {
    fn bilinear([b2, b1, b0, a2, a1, a0]: Analog, fs: f64) -> Self {
        let k = 2.0 * fs;
        let kk = k * k;
        let n = a2 * kk + a1 * k + a0;
        Self {
            b: [(b2 * kk + b1 * k + b0) / n, (2.0 * b0 - 2.0 * b2 * kk) / n, (b2 * kk - b1 * k + b0) / n],
            a: [(2.0 * a0 - 2.0 * a2 * kk) / n, (a2 * kk - a1 * k + a0) / n],
            s: [0.0; 2],
        }
    }

    fn step(&mut self, x: f64) -> f64 {
        let y = self.b[0] * x + self.s[0];
        self.s[0] = self.b[1] * x - self.a[0] * y + self.s[1];
        self.s[1] = self.b[2] * x - self.a[1] * y;
        y
    }
}

/// `tone` as heard through the speaker: mono 16-bit PCM at [`SAMPLE_RATE`],
/// starting from silence one sample before the drive and ending at silence
/// [`TAIL_S`] after it.
pub fn synthesise(tone: Tone) -> Vec<i16> {
    let sr = f64::from(SAMPLE_RATE);
    let fs = sr * OVERSAMPLE as f64;
    let n = ((tone.gate_s() + TAIL_S) * sr).round() as usize;
    // Each step's time in sub-samples, one output sample in.
    let steps: Vec<(f64, f64)> = drive(tone)
        .into_iter()
        .map(|(line, step)| (f64::from(line) / LINE_HZ * fs + OVERSAMPLE as f64, step))
        .collect();
    let mut sections = speaker().map(|s| Biquad::bilinear(s, fs));

    let mut out = Vec::with_capacity(n);
    let (mut level, mut next) = (0.0, 0);
    for m in 0..n {
        let mut acc = 0.0;
        for j in 0..OVERSAMPLE {
            let i = m * OVERSAMPLE + j;
            // The drive's mean over sub-sample i: a step inside it counts
            // for the part of the sub-sample after it.
            let mut x = level;
            while let Some(&(at, step)) = steps.get(next) {
                if at >= (i + 1) as f64 {
                    break;
                }
                x += step * ((i + 1) as f64 - at);
                level += step;
                next += 1;
            }
            for section in &mut sections {
                x = section.step(x);
            }
            acc += x;
        }
        out.push(acc / OVERSAMPLE as f64);
    }
    // The ringing has decayed by at least 40 dB here (the Serve; 57-71 dB the
    // others); the fade only makes the end
    // exactly silent.
    let fade = (FADE_S * sr).round() as usize;
    for i in 0..fade {
        out[n - fade + i] *= 0.5 * (1.0 + (PI * (i + 1) as f64 / fade as f64).cos());
    }
    out.into_iter().map(|y| (y * OUTPUT_GAIN * 32767.0).round().clamp(-32768.0, 32767.0) as i16).collect()
}

/// A RIFF/WAVE file of mono 16-bit PCM at [`SAMPLE_RATE`].
fn wav_bytes(samples: &[i16]) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut b = Vec::with_capacity(44 + samples.len() * 2);
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + data_len).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes()); // PCM
    b.extend_from_slice(&1u16.to_le_bytes()); // mono
    b.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    b.extend_from_slice(&(SAMPLE_RATE * 2).to_le_bytes()); // byte rate
    b.extend_from_slice(&2u16.to_le_bytes()); // block align
    b.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    b.extend_from_slice(b"data");
    b.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        b.extend_from_slice(&s.to_le_bytes());
    }
    b
}

/// `--export-sounds`: writes the four sounds as WAV files into `dir`
/// (created if missing) and returns their paths.
pub fn export(dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    std::fs::create_dir_all(dir)?;
    Tone::ALL
        .into_iter()
        .map(|tone| {
            let path = dir.join(tone.file_name());
            std::fs::write(&path, wav_bytes(&synthesise(tone)))?;
            Ok(path)
        })
        .collect()
}

/// The loaded sounds, plus the single speaker channel.
pub struct Sounds {
    /// Indexed by [`Tone::index`]; `None` for a sound the backend failed to
    /// load.
    sounds: [Option<Sound>; Tone::ALL.len()],
    /// The sound most recently started, which the next one cuts off.
    current: Option<Sound>,
}

impl Sounds {
    /// Synthesises and loads the four sounds. A load failure (audio backend
    /// unavailable) leaves that sound as `None`; playing it is then simply a
    /// no-op rather than a panic.
    pub async fn load() -> Self {
        let mut sounds: [Option<Sound>; Tone::ALL.len()] = Default::default();
        for tone in Tone::ALL {
            sounds[tone.index()] = load_sound_from_bytes(&wav_bytes(&synthesise(tone))).await.ok();
        }
        Self { sounds, current: None }
    }

    /// Plays `events` in order on the one channel (SPEC §12.2) at `gain`
    /// (see [`volume_gain`]): each sound first stops the one playing, so when
    /// several arrive in one frame the last is the one heard. A gain of 0
    /// (muted, or volume 0) plays nothing.
    pub fn play(&mut self, events: &[SoundEvent], gain: f32) {
        for &event in events {
            self.beep(Tone::for_event(event), gain);
        }
    }

    /// Plays one `tone` on the channel at `gain`, as [`Sounds::play`] does
    /// for an event: the menu's volume feedback and the countdown's beeps.
    pub fn beep(&mut self, tone: Tone, gain: f32) {
        if gain <= 0.0 {
            return;
        }
        let Some(sound) = &self.sounds[tone.index()] else { return };
        cut_in(&mut self.current, sound, stop_sound, |new| {
            play_sound(new, PlaySoundParams { looped: false, volume: gain })
        });
    }

    /// Stops the sound playing, if any: the pause cuts the speaker off.
    pub fn stop(&mut self) {
        if let Some(sound) = self.current.take() {
            stop_sound(&sound);
        }
    }
}

/// The one speaker channel (SPEC §12.2): starts `next`, after stopping
/// `current`, the sound this channel started last, and remembers `next` as
/// the new current one. `stop` and `play` are the backend's calls, passed in
/// so the order can be tested without an audio device.
fn cut_in<S: Clone>(current: &mut Option<S>, next: &S, mut stop: impl FnMut(&S), mut play: impl FnMut(&S)) {
    if let Some(old) = current.take() {
        stop(&old);
    }
    play(next);
    *current = Some(next.clone());
}

#[cfg(test)]
mod tests {
    use super::*;
    use superpong_core::BounceKind;

    /// The four sounds, synthesised once.
    fn all() -> Vec<Vec<i16>> {
        Tone::ALL.into_iter().map(synthesise).collect()
    }

    /// Output samples covering the drive, from its start (one sample in).
    fn gate(tone: Tone, pcm: &[i16]) -> Vec<f64> {
        let n = (tone.gate_s() * f64::from(SAMPLE_RATE)) as usize;
        pcm[1..1 + n].iter().map(|&s| f64::from(s)).collect()
    }

    fn rms(x: &[f64]) -> f64 {
        (x.iter().map(|v| v * v).sum::<f64>() / x.len() as f64).sqrt()
    }

    /// Frequency from the rising zero crossings of `x` at `rate`, skipping
    /// the first `skip` samples (the start-up transient).
    fn zero_crossing_hz(x: &[f64], rate: f64, skip: usize) -> f64 {
        let ups: Vec<usize> = (skip + 1..x.len()).filter(|&i| x[i - 1] < 0.0 && x[i] >= 0.0).collect();
        let (first, last) = (ups[0], ups[ups.len() - 1]);
        (ups.len() - 1) as f64 * rate / (last - first) as f64
    }

    #[test]
    fn every_reflection_plays_the_one_bounce() {
        for kind in [BounceKind::Paddle, BounceKind::Wall, BounceKind::Border] {
            assert_eq!(Tone::for_event(SoundEvent::Bounce(kind)), Tone::Bounce);
        }
        assert_eq!(Tone::for_event(SoundEvent::Point), Tone::Point);
        assert_eq!(Tone::for_event(SoundEvent::Serve), Tone::Serve);
        assert_eq!(Tone::for_event(SoundEvent::AttractTick), Tone::AttractTick);
    }

    #[test]
    fn tone_indices_follow_table_order() {
        for (i, tone) in Tone::ALL.into_iter().enumerate() {
            assert_eq!(tone.index(), i, "{tone:?}");
        }
    }

    #[test]
    fn the_drive_has_the_line_counter_pitches() {
        // The chip pin, sampled at the line rate: its rising zero crossings
        // (with the rest level at the midpoint) give the pitch exactly.
        for (tone, hz) in
            [(Tone::Serve, LINE_HZ / 16.0), (Tone::Bounce, LINE_HZ / 32.0), (Tone::Point, LINE_HZ / 64.0)]
        {
            let mut level = 0.0;
            let mut steps = drive(tone).into_iter().peekable();
            let pin: Vec<f64> = (0..tone.frames() * tone.frame_lines())
                .map(|line| {
                    while let Some((_, step)) = steps.next_if(|&(at, _)| at <= line) {
                        level += step;
                    }
                    level - 0.5
                })
                .collect();
            let f = zero_crossing_hz(&pin, LINE_HZ, 0);
            assert!((f / hz - 1.0).abs() < 1e-9, "{tone:?}: {f} Hz, want {hz}");
        }
    }

    #[test]
    fn the_light_show_tick_restarts_its_phase_every_frame() {
        // 262 lines = 4 periods of 64 + 6 lines: each frame ends with 6
        // lines on, and the next starts on again, so each of the 4 frame
        // boundaries makes one on-half 38 lines long; the 5th frame's 6
        // lines end the tone.
        let lengths: Vec<u32> = drive(Tone::AttractTick).windows(2).map(|p| p[1].0 - p[0].0).collect();
        assert_eq!(lengths.iter().filter(|&&l| l == 38).count(), 4);
        assert_eq!(lengths.iter().filter(|&&l| l == 32).count(), lengths.len() - 5);
        assert_eq!(lengths.last(), Some(&6));
        assert_eq!(drive(Tone::AttractTick).last(), Some(&(5 * 262, -1.0)));
    }

    #[test]
    fn bounce_and_serve_sound_at_their_pitch() {
        // Through the speaker the Point's and tick's 246 Hz fundamental is
        // weaker than their 3rd harmonic, so only these two are checked by
        // zero crossings here. The cone's 455 Hz resonance pulls the
        // Bounce's crossings a little low (489 Hz), and the ~1 kHz mode
        // pulls the Serve's below its 985.6 Hz drive (978 Hz), as in the
        // video (488 and 981 Hz read, sounds.md §11.6).
        let pcm = all();
        let rate = f64::from(SAMPLE_RATE);
        let skip = (0.002 * rate) as usize;
        let bounce = zero_crossing_hz(&gate(Tone::Bounce, &pcm[0]), rate, skip);
        assert!((480.0..492.0).contains(&bounce), "Bounce at {bounce} Hz");
        let serve = zero_crossing_hz(&gate(Tone::Serve, &pcm[2]), rate, skip);
        assert!((970.0..LINE_HZ / 16.0).contains(&serve), "Serve at {serve} Hz");
    }

    #[test]
    fn the_serve_rings_at_the_mode_after_its_drive() {
        // 1-20 ms after the chip stops, the Serve is the free ringing of the
        // ~1 kHz mode: 997 Hz, as the video's 997-999 Hz (sounds.md §11.6).
        let pcm = synthesise(Tone::Serve);
        let rate = f64::from(SAMPLE_RATE);
        let end = 1 + (Tone::Serve.gate_s() * rate) as usize;
        let tail: Vec<f64> = pcm[end + (0.001 * rate) as usize..end + (0.020 * rate) as usize]
            .iter()
            .map(|&s| f64::from(s))
            .collect();
        let f = zero_crossing_hz(&tail, rate, 0);
        assert!((992.0..1002.0).contains(&f), "Serve rings at {f} Hz");
    }

    #[test]
    fn each_sound_is_its_drive_plus_the_tail() {
        for (tone, pcm) in Tone::ALL.into_iter().zip(all()) {
            let want = ((tone.gate_s() + TAIL_S) * f64::from(SAMPLE_RATE)).round() as usize;
            assert_eq!(pcm.len(), want, "{tone:?}");
        }
        assert!((Tone::Bounce.gate_s() * 1000.0 - 16.68).abs() < 0.01);
        assert!((Tone::AttractTick.gate_s() * 1000.0 - 83.08).abs() < 0.01);
    }

    #[test]
    fn the_sound_stops_with_the_drive_apart_from_the_ringing() {
        // 10 ms after the drive every sound is at least 6 dB below its level
        // during the drive, and 40 ms after at least 25 dB (the Serve's
        // ~1 kHz mode rings longest, sounds.md §11.6).
        for (tone, pcm) in Tone::ALL.into_iter().zip(all()) {
            let level = rms(&gate(tone, &pcm));
            let at = |s: f64| (tone.gate_s() + s) * f64::from(SAMPLE_RATE);
            let window = |s: f64| -> Vec<f64> {
                pcm[at(s) as usize..at(s + 0.005) as usize].iter().map(|&v| f64::from(v)).collect()
            };
            assert!(20.0 * (rms(&window(0.010)) / level).log10() < -6.0, "{tone:?} rings too long");
            assert!(20.0 * (rms(&window(0.040)) / level).log10() < -25.0, "{tone:?} rings too long");
        }
    }

    #[test]
    fn levels_relative_to_the_bounce_are_the_models() {
        // RMS over the drive, dB re the Bounce (sounds.md §11.3). These come
        // from the one speaker model, not from per-sound gains.
        let pcm = all();
        let bounce = rms(&gate(Tone::Bounce, &pcm[0]));
        for (tone, want) in [(Tone::Point, -13.5), (Tone::Serve, -8.9), (Tone::AttractTick, -13.7)] {
            let db = 20.0 * (rms(&gate(tone, &pcm[tone.index()])) / bounce).log10();
            assert!((db - want).abs() < 0.3, "{tone:?} at {db:.2} dB, want {want}");
        }
    }

    #[test]
    fn volume_runs_from_silent_to_full_scale() {
        assert_eq!(volume_gain(0), 0.0);
        assert_eq!(volume_gain(VOLUME_MAX), 1.0);
        assert_eq!(volume_gain(VOLUME_MAX + 5), 1.0, "clamped");
        let gains: Vec<f32> = (0..=VOLUME_MAX).map(volume_gain).collect();
        assert!(gains.windows(2).all(|p| p[0] < p[1]), "{gains:?}");
        let db = |level| 20.0 * volume_gain(level).log10();
        assert!((db(5) + 12.04).abs() < 0.01 && (db(1) + 40.0).abs() < 0.01);
        const { assert!(DEFAULT_VOLUME > 0 && DEFAULT_VOLUME <= VOLUME_MAX) };
    }

    #[test]
    fn synthesis_is_deterministic() {
        assert_eq!(all(), all());
    }

    #[test]
    fn nothing_clips() {
        for (tone, pcm) in Tone::ALL.into_iter().zip(all()) {
            let peak = pcm.iter().map(|s| s.unsigned_abs()).max().unwrap();
            assert!(peak < 31_000, "{tone:?} peaks at {peak}");
            assert!(peak > 5_000, "{tone:?} is nearly silent");
        }
    }

    #[test]
    fn sounds_start_and_end_without_a_click() {
        for (tone, pcm) in Tone::ALL.into_iter().zip(all()) {
            assert_eq!(pcm[0], 0, "{tone:?} first sample");
            assert_eq!(*pcm.last().unwrap(), 0, "{tone:?} last sample");
            // The last 10 ms (the fade) move by at most a few LSBs per sample:
            // the Serve's 1 kHz ringing is only about 40 dB down there.
            let tail = &pcm[pcm.len() - 441..];
            let step = tail.windows(2).map(|p| (i32::from(p[1]) - i32::from(p[0])).abs()).max().unwrap();
            assert!(step <= 16, "{tone:?} steps {step} in its fade");
        }
    }

    /// Reads a little-endian integer from the WAV bytes.
    fn le(bytes: &[u8], at: usize, len: usize) -> usize {
        bytes[at..at + len].iter().rev().fold(0, |acc, &b| (acc << 8) | b as usize)
    }

    #[test]
    fn wav_bytes_are_pcm16_mono_44k() {
        let pcm = synthesise(Tone::Bounce);
        let b = wav_bytes(&pcm);
        assert_eq!(&b[0..4], b"RIFF");
        assert_eq!(le(&b, 4, 4) + 8, b.len());
        assert_eq!(&b[8..16], b"WAVEfmt ");
        assert_eq!(le(&b, 16, 4), 16);
        assert_eq!(le(&b, 20, 2), 1, "PCM");
        assert_eq!(le(&b, 22, 2), 1, "mono");
        assert_eq!(le(&b, 24, 4), 44_100, "sample rate");
        assert_eq!(le(&b, 28, 4), 88_200, "byte rate");
        assert_eq!(le(&b, 32, 2), 2, "block align");
        assert_eq!(le(&b, 34, 2), 16, "bits per sample");
        assert_eq!(&b[36..40], b"data");
        assert_eq!(le(&b, 40, 4), pcm.len() * 2);
        assert_eq!(le(&b, 44, 2) as u16 as i16, pcm[0]);
        assert_eq!(b.len(), 44 + pcm.len() * 2);
    }

    /// A recording speaker: the calls `cut_in` makes, in order.
    fn cut_in_log(sequence: &[&'static str]) -> Vec<String> {
        let log = std::cell::RefCell::new(Vec::new());
        let mut current = None;
        for name in sequence {
            cut_in(
                &mut current,
                name,
                |old| log.borrow_mut().push(format!("stop {old}")),
                |new| log.borrow_mut().push(format!("play {new}")),
            );
            assert_eq!(current, Some(*name));
        }
        log.into_inner()
    }

    #[test]
    fn the_first_sound_just_plays() {
        assert_eq!(cut_in_log(&["bounce"]), ["play bounce"]);
    }

    #[test]
    fn each_sound_stops_the_previous_one_before_it_plays() {
        assert_eq!(
            cut_in_log(&["bounce", "point", "serve"]),
            ["play bounce", "stop bounce", "play point", "stop point", "play serve"],
        );
    }

    #[test]
    fn a_sound_cuts_off_even_a_repeat_of_itself() {
        assert_eq!(cut_in_log(&["tick", "tick"]), ["play tick", "stop tick", "play tick"]);
    }
}
