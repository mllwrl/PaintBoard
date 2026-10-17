//! Talktrack sound: voice resampled to 16 kHz mono and packed with IMA ADPCM (4 bits a
//! sample, about 0.5 MB a minute). Stored as an asset:
//!
//! `PBAU` · version 1 (u8) · sample rate (u32 LE) · sample count (u32 LE) · ADPCM nibbles,
//! low nibble first.

/// What voice is kept at: enough for speech.
pub const RATE: u32 = 16_000;

const MAGIC: &[u8; 4] = b"PBAU";
const VERSION: u8 = 1;
const HEADER: usize = 4 + 1 + 4 + 4;

const STEPS: [i32; 89] = [
    7, 8, 9, 10, 11, 12, 13, 14, 16, 17, 19, 21, 23, 25, 28, 31, 34, 37, 41, 45, 50, 55, 60, 66,
    73, 80, 88, 97, 107, 118, 130, 143, 157, 173, 190, 209, 230, 253, 279, 307, 337, 371, 408, 449,
    494, 544, 598, 658, 724, 796, 876, 963, 1060, 1166, 1282, 1411, 1552, 1707, 1878, 2066, 2272,
    2499, 2749, 3024, 3327, 3660, 4026, 4428, 4871, 5358, 5894, 6484, 7132, 7845, 8630, 9493,
    10442, 11487, 12635, 13899, 15289, 16818, 18500, 20350, 22385, 24623, 27086, 29794, 32767,
];

const INDEX_CHANGE: [i32; 16] = [-1, -1, -1, -1, 2, 4, 6, 8, -1, -1, -1, -1, 2, 4, 6, 8];

/// The coder's state, the same on both sides.
#[derive(Default)]
struct State {
    predicted: i32,
    index: i32,
}

impl State {
    /// Applies `code` and returns the sample it stands for.
    fn decode(&mut self, code: u8) -> i16 {
        let step = STEPS[self.index as usize];
        let mut diff = step >> 3;
        if code & 4 != 0 {
            diff += step;
        }
        if code & 2 != 0 {
            diff += step >> 1;
        }
        if code & 1 != 0 {
            diff += step >> 2;
        }
        if code & 8 != 0 {
            diff = -diff;
        }
        self.predicted = (self.predicted + diff).clamp(-32768, 32767);
        self.index = (self.index + INDEX_CHANGE[usize::from(code)]).clamp(0, 88);
        self.predicted as i16
    }

    fn encode(&mut self, sample: i16) -> u8 {
        let step = STEPS[self.index as usize];
        let mut diff = i32::from(sample) - self.predicted;
        let mut code = 0;
        if diff < 0 {
            code = 8;
            diff = -diff;
        }
        let mut part = step;
        for bit in [4, 2, 1] {
            if diff >= part {
                code |= bit;
                diff -= part;
            }
            part >>= 1;
        }
        // Track exactly what the decoder will see.
        self.decode(code);
        code
    }
}

/// Packs `samples` (mono, -1..1, `rate` per second) for storing.
pub fn encode(samples: &[f32], rate: u32) -> Vec<u8> {
    let voice = resample(samples, rate, RATE);
    let mut bytes = Vec::with_capacity(HEADER + voice.len().div_ceil(2));
    bytes.extend_from_slice(MAGIC);
    bytes.push(VERSION);
    bytes.extend_from_slice(&RATE.to_le_bytes());
    bytes.extend_from_slice(&(voice.len() as u32).to_le_bytes());
    let mut state = State::default();
    for pair in voice.chunks(2) {
        let low = state.encode(to_i16(pair[0]));
        let high = pair.get(1).map_or(0, |&s| state.encode(to_i16(s)));
        bytes.push(low | (high << 4));
    }
    bytes
}

/// Unpacks stored sound: its samples and their rate. `None` if it is not ours.
pub fn decode(bytes: &[u8]) -> Option<(Vec<i16>, u32)> {
    if bytes.len() < HEADER || &bytes[..4] != MAGIC || bytes[4] != VERSION {
        return None;
    }
    let rate = u32::from_le_bytes(bytes[5..9].try_into().ok()?);
    let count = u32::from_le_bytes(bytes[9..13].try_into().ok()?) as usize;
    let data = &bytes[HEADER..];
    if data.len() < count.div_ceil(2) || rate == 0 {
        return None;
    }
    let mut state = State::default();
    let mut out = Vec::with_capacity(count);
    for &byte in data {
        for code in [byte & 0x0F, byte >> 4] {
            if out.len() < count {
                out.push(state.decode(code));
            }
        }
    }
    Some((out, rate))
}

/// How long stored sound lasts, in seconds, without unpacking it.
pub fn duration(bytes: &[u8]) -> Option<f64> {
    let rate = u32::from_le_bytes(bytes.get(5..9)?.try_into().ok()?);
    let count = u32::from_le_bytes(bytes.get(9..13)?.try_into().ok()?);
    (rate > 0).then(|| f64::from(count) / f64::from(rate))
}

fn to_i16(sample: f32) -> i16 {
    (sample.clamp(-1.0, 1.0) * 32767.0).round() as i16
}

/// `samples` at rate `to`: averaged over each output sample's span when going down (which
/// keeps most aliasing out of speech), linearly interpolated when going up.
fn resample(samples: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || from == 0 || samples.is_empty() {
        return samples.to_vec();
    }
    let ratio = f64::from(from) / f64::from(to);
    let len = (samples.len() as f64 / ratio).floor() as usize;
    (0..len)
        .map(|i| {
            let start = i as f64 * ratio;
            if ratio > 1.0 {
                let a = start as usize;
                let b = (((i + 1) as f64 * ratio) as usize).clamp(a + 1, samples.len());
                samples[a..b].iter().sum::<f32>() / (b - a) as f32
            } else {
                let a = start as usize;
                let t = (start - a as f64) as f32;
                let next = samples.get(a + 1).copied().unwrap_or(samples[a]);
                samples[a] * (1.0 - t) + next * t
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn voice_survives_packing() {
        // One second of a 440 Hz tone recorded at 48 kHz.
        let tone: Vec<f32> = (0..48_000)
            .map(|i| (i as f32 * 440.0 * std::f32::consts::TAU / 48_000.0).sin() * 0.5)
            .collect();
        let bytes = encode(&tone, 48_000);
        assert!(
            bytes.len() < 8_100,
            "about 8 KB a second, got {}",
            bytes.len()
        );
        assert_eq!(duration(&bytes), Some(1.0));
        let (back, rate) = decode(&bytes).unwrap();
        assert_eq!((back.len(), rate), (16_000, RATE));
        // Close to the tone at 16 kHz once the coder has settled.
        let error: f64 = (1_000..16_000)
            .map(|i| {
                let want = (i as f32 * 440.0 * std::f32::consts::TAU / 16_000.0).sin() * 0.5;
                f64::from(f32::from(back[i]) / 32767.0 - want).abs()
            })
            .sum::<f64>()
            / 15_000.0;
        assert!(error < 0.03, "mean error {error}");
    }

    #[test]
    fn foreign_bytes_are_refused() {
        assert_eq!(decode(b"RIFF....WAVE"), None);
        assert_eq!(decode(b""), None);
    }
}
