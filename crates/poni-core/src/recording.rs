//! Recording a meeting's sound: the parts that are only arithmetic. The
//! Windows crate captures the microphone and what the PC plays (WASAPI
//! loopback) and encodes AAC (`services::recording`); this brings both to
//! one rate, lines them up as stereo (left: you, right: everyone else, so
//! a transcript can tell the two apart later), names the file, and says
//! when the disk is too full to start or carry on.

use crate::datetime::LocalDateTime;
use std::collections::VecDeque;

/// The file's sample rate.
pub const RATE: u32 = 48_000;
/// AAC at 96 kbit/s: about 43 MB an hour.
pub const AAC_BYTES_PER_SECOND: u32 = 12_000;
/// Free space a recording needs to start, and below which it stops itself.
pub const START_MIN_FREE: u64 = 500 * 1024 * 1024;
pub const STOP_MIN_FREE: u64 = 200 * 1024 * 1024;
/// What the other side may run ahead of the microphone before the oldest
/// of it is dropped (one second).
const BACKLOG: usize = RATE as usize;

/// Linear-interpolation resampler for one channel, carried across chunks.
#[derive(Clone, Debug)]
pub struct Resampler {
    /// Input samples per output sample.
    step: f64,
    /// Where the next output falls, in input samples from the start of the
    /// next chunk (-1: the last sample of the chunk before).
    pos: f64,
    prev: f32,
}

impl Resampler {
    pub fn new(from_rate: u32, to_rate: u32) -> Self {
        Self { step: from_rate.max(1) as f64 / to_rate.max(1) as f64, pos: 0.0, prev: 0.0 }
    }

    pub fn push(&mut self, input: &[f32], out: &mut Vec<f32>) {
        if input.is_empty() {
            return;
        }
        if self.step == 1.0 {
            out.extend_from_slice(input);
            return;
        }
        let n = input.len() as isize;
        loop {
            let i = self.pos.floor() as isize;
            if i + 1 >= n {
                break;
            }
            let a = if i < 0 { self.prev } else { input[i as usize] };
            let b = input[(i + 1) as usize];
            out.push(a + (b - a) * (self.pos - i as f64) as f32);
            self.pos += self.step;
        }
        self.pos -= n as f64;
        self.prev = input[input.len() - 1];
    }
}

/// One channel from a device's interleaved frames: the average of its channels.
pub fn downmix(frames: &[f32], channels: usize) -> Vec<f32> {
    let c = channels.max(1);
    frames.chunks_exact(c).map(|f| f.iter().sum::<f32>() / c as f32).collect()
}

/// The same for 16-bit samples, as -1..1.
pub fn downmix_i16(frames: &[i16], channels: usize) -> Vec<f32> {
    let c = channels.max(1);
    frames.chunks_exact(c).map(|f| f.iter().map(|s| *s as f32 / 32768.0).sum::<f32>() / c as f32).collect()
}

/// The two sides, waiting to be written as stereo.
#[derive(Debug, Default)]
pub struct Mixer {
    mic: VecDeque<f32>,
    others: VecDeque<f32>,
}

impl Mixer {
    pub fn push_mic(&mut self, s: &[f32]) {
        self.mic.extend(s);
    }

    /// What the PC plays. Loopback sends nothing while nothing plays, so
    /// the microphone keeps time; a backlog past a second is dropped.
    pub fn push_others(&mut self, s: &[f32]) {
        self.others.extend(s);
        while self.others.len() > BACKLOG {
            self.others.pop_front();
        }
    }

    pub fn mic_queued(&self) -> usize {
        self.mic.len()
    }

    /// `frames` stereo frames as interleaved 16-bit samples, left the
    /// microphone, right the PC; either side short of samples is silent.
    pub fn take(&mut self, frames: usize) -> Vec<i16> {
        let mut out = Vec::with_capacity(frames * 2);
        for _ in 0..frames {
            out.push(to_i16(self.mic.pop_front().unwrap_or(0.0)));
            out.push(to_i16(self.others.pop_front().unwrap_or(0.0)));
        }
        out
    }
}

fn to_i16(s: f32) -> i16 {
    (s.clamp(-1.0, 1.0) * 32767.0).round() as i16
}

/// "2026-10-30 10.00 Standup.m4a": when it started and what (the meeting's
/// title, else "Recording"), safe as a Windows file name.
pub fn file_name(title: Option<&str>, at: &LocalDateTime) -> String {
    let clean: String = title
        .unwrap_or("")
        .chars()
        .map(|c| if matches!(c, '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|') || c.is_control() { ' ' } else { c })
        .collect();
    let clean: String = clean.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(60).collect();
    let clean = clean.trim_end_matches(['.', ' ']);
    let what = if clean.is_empty() { "Recording" } else { clean };
    format!("{:04}-{:02}-{:02} {:02}.{:02} {what}.m4a", at.year, at.month, at.day, at.hour, at.minute)
}

/// Whether a recording may start with `free` bytes on the disk.
pub fn can_start(free: u64) -> bool {
    free >= START_MIN_FREE
}

/// Whether a recording under way should stop with `free` bytes left.
pub fn must_stop(free: u64) -> bool {
    free < STOP_MIN_FREE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_rate_passes_through_and_others_resample_smoothly() {
        let mut same = Resampler::new(48_000, 48_000);
        let mut out = Vec::new();
        same.push(&[0.1, 0.2, 0.3], &mut out);
        assert_eq!(out, [0.1, 0.2, 0.3]);

        // 96 kHz to 48: every other sample, across chunk edges.
        let mut half = Resampler::new(96_000, 48_000);
        let input: Vec<f32> = (0..10).map(|i| i as f32).collect();
        let mut out = Vec::new();
        half.push(&input[..5], &mut out);
        half.push(&input[5..], &mut out);
        assert_eq!(out, [0.0, 2.0, 4.0, 6.0, 8.0]);

        // 44.1 kHz to 48: a ramp stays a ramp, and the count follows the ratio.
        let mut up = Resampler::new(44_100, 48_000);
        let ramp: Vec<f32> = (0..4410).map(|i| i as f32 / 4410.0).collect();
        let mut out = Vec::new();
        for chunk in ramp.chunks(441) {
            up.push(chunk, &mut out);
        }
        assert!((out.len() as i64 - 4800).abs() <= 2, "{}", out.len());
        assert!(out.windows(2).all(|w| w[1] >= w[0]), "no jumps at the chunk edges");
    }

    #[test]
    fn devices_are_brought_to_one_channel() {
        assert_eq!(downmix(&[0.2, 0.4, -1.0, 1.0], 2), [0.3, 0.0]);
        assert_eq!(downmix(&[0.5, 0.25], 1), [0.5, 0.25]);
        assert_eq!(downmix_i16(&[16384, 16384], 2), [0.5]);
    }

    #[test]
    fn the_mixer_keeps_time_by_the_microphone() {
        let mut m = Mixer::default();
        m.push_mic(&[0.5, -0.5, 1.5]);
        m.push_others(&[0.25]);
        assert_eq!(m.mic_queued(), 3);
        // Left you, right the others; the others go quiet when they run out,
        // and loud samples are held to full scale.
        assert_eq!(m.take(3), [16384, 8192, -16384, 0, 32767, 0]);
        // A backlog of the other side never grows past a second.
        m.push_others(&vec![0.0; BACKLOG + 500]);
        assert_eq!(m.others.len(), BACKLOG);
    }

    #[test]
    fn files_are_named_by_when_and_what() {
        let at = LocalDateTime { year: 2026, month: 10, day: 30, hour: 9, minute: 5, second: 0 };
        assert_eq!(file_name(Some("Standup"), &at), "2026-10-30 09.05 Standup.m4a");
        assert_eq!(file_name(Some("Q4: plan / budget?"), &at), "2026-10-30 09.05 Q4 plan budget.m4a");
        assert_eq!(file_name(Some("  ...  "), &at), "2026-10-30 09.05 Recording.m4a");
        assert_eq!(file_name(None, &at), "2026-10-30 09.05 Recording.m4a");
        assert!(can_start(START_MIN_FREE) && !can_start(START_MIN_FREE - 1));
        assert!(must_stop(STOP_MIN_FREE - 1) && !must_stop(STOP_MIN_FREE));
    }
}
