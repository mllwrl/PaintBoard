//! Short sounds plugins can play (`paintboard.ui.playSound`), made here from sine tones,
//! so there are no sound files to ship.

/// Samples per second of the sounds.
pub const RATE: u32 = 22_050;

/// The sounds there are, by the names plugins use.
pub const NAMES: [&str; 4] = ["chime", "beep", "done", "alarm"];

/// A tone: its pitch in Hz, when it starts and how long it rings, in seconds.
struct Note(f64, f64, f64);

/// The sound called `name`: mono samples at [`RATE`].
pub fn sound(name: &str) -> Option<Vec<i16>> {
    let notes: &[Note] = match name {
        // Two bell-like notes falling a third, E6 then C6.
        "chime" => &[Note(1318.5, 0.0, 0.9), Note(1046.5, 0.22, 1.2)],
        "beep" => &[Note(880.0, 0.0, 0.18)],
        // C, E, G rising: something finished.
        "done" => &[
            Note(523.25, 0.0, 0.3),
            Note(659.25, 0.12, 0.3),
            Note(783.99, 0.24, 0.6),
        ],
        // Three quick high beeps.
        "alarm" => &[
            Note(987.77, 0.0, 0.12),
            Note(987.77, 0.2, 0.12),
            Note(987.77, 0.4, 0.12),
        ],
        _ => return None,
    };
    let rate = f64::from(RATE);
    let end = notes.iter().map(|n| n.1 + n.2).fold(0.0, f64::max);
    let mut samples = vec![0.0f64; (end * rate).ceil() as usize];
    for &Note(pitch, start, length) in notes {
        let first = (start * rate) as usize;
        let count = (length * rate) as usize;
        for (i, sample) in samples.iter_mut().skip(first).take(count).enumerate() {
            let t = i as f64 / rate;
            // A quick start, a ring dying away, and a short fade so it ends without a click.
            let attack = (t / 0.005).min(1.0);
            let decay = (-3.0 * t / length).exp();
            let fade = ((length - t) / 0.01).clamp(0.0, 1.0);
            let phase = std::f64::consts::TAU * pitch * t;
            let tone = phase.sin() + 0.25 * (2.0 * phase).sin();
            *sample += 0.25 * attack * decay * fade * tone;
        }
    }
    Some(
        samples
            .into_iter()
            .map(|s| (s.clamp(-1.0, 1.0) * f64::from(i16::MAX)) as i16)
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sounds_are_short_and_clean() {
        for name in NAMES {
            let samples = sound(name).unwrap();
            let seconds = samples.len() as f64 / f64::from(RATE);
            assert!((0.1..2.0).contains(&seconds), "{name}: {seconds} s");
            let loudest = samples.iter().map(|s| s.unsigned_abs()).max().unwrap();
            assert!(
                loudest > 3000 && loudest < i16::MAX as u16,
                "{name}: {loudest}"
            );
            // Silent at both ends: no clicks.
            assert!(samples[0].unsigned_abs() < 100, "{name}");
            assert!(samples.last().unwrap().unsigned_abs() < 100, "{name}");
        }
        assert_eq!(sound("trumpet"), None);
    }
}
