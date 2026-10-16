//! Handwriting recognition.
//!
//! On Windows this uses Windows Ink, which is part of the OS, works offline and reads the
//! languages whose handwriting recognizers the user has installed. Elsewhere it is not
//! available yet.
//!
//! TODO(macOS, Linux, web): a local recognition model (check the weights' license).

/// One stroke as points, y pointing down. Any unit: strokes are rescaled to a typical
/// handwriting size before recognition.
pub type Stroke = Vec<(f64, f64)>;

/// Whether [`recognize`] can work on this platform.
pub fn is_available() -> bool {
    cfg!(windows)
}

/// Recognizes the strokes as one line of text on a background thread, then calls `done`
/// there with the best guess.
pub fn recognize(strokes: Vec<Stroke>, done: impl Done) {
    imp::recognize(strokes, done);
}

/// What [`recognize`] calls with its result. It runs on another thread where there are
/// threads, so it must be `Send` there; the web has none.
#[cfg(not(target_arch = "wasm32"))]
pub trait Done: FnOnce(Result<String, String>) + Send + 'static {}
#[cfg(not(target_arch = "wasm32"))]
impl<F: FnOnce(Result<String, String>) + Send + 'static> Done for F {}

#[cfg(target_arch = "wasm32")]
pub trait Done: FnOnce(Result<String, String>) + 'static {}
#[cfg(target_arch = "wasm32")]
impl<F: FnOnce(Result<String, String>) + 'static> Done for F {}

/// Line height recognizers expect, in their device-independent pixels.
#[cfg_attr(not(windows), allow(dead_code))]
const TARGET_HEIGHT: f64 = 60.0;

/// Moves the strokes to the origin and scales them to [`TARGET_HEIGHT`].
#[cfg_attr(not(windows), allow(dead_code))]
fn normalize(strokes: &[Stroke]) -> Vec<Stroke> {
    let points = strokes.iter().flatten();
    let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for &(x, y) in points {
        (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
    }
    // Lines and dots have no height; size them by their width instead.
    let extent = (y1 - y0).max((x1 - x0) / 4.0);
    let scale = if extent > 0.0 {
        TARGET_HEIGHT / extent
    } else {
        1.0
    };
    strokes
        .iter()
        .map(|s| {
            s.iter()
                .map(|&(x, y)| ((x - x0) * scale, (y - y0) * scale))
                .collect()
        })
        .collect()
}

#[cfg(windows)]
mod imp {
    use windows::Foundation::Point;
    use windows::UI::Input::Inking::{
        InkPoint, InkRecognitionTarget, InkRecognizerContainer, InkStrokeBuilder,
        InkStrokeContainer,
    };
    use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize};
    use windows_collections::IIterable;
    use windows_numerics::Matrix3x2;

    use super::{Stroke, normalize};

    pub fn recognize(strokes: Vec<Stroke>, done: impl super::Done) {
        std::thread::spawn(move || {
            let result = recognize_blocking(&strokes).map_err(|err| err.message());
            done(result);
        });
    }

    fn recognize_blocking(strokes: &[Stroke]) -> windows::core::Result<String> {
        #[allow(unsafe_code)]
        // SAFETY: called once at the start of a thread we own, before any WinRT use; the
        // thread joins the multithreaded apartment, which waiting on async results needs.
        let _ = unsafe { RoInitialize(RO_INIT_MULTITHREADED) };

        let builder = InkStrokeBuilder::new()?;
        let container = InkStrokeContainer::new()?;
        for stroke in normalize(strokes) {
            let points = stroke
                .iter()
                .map(|&(x, y)| {
                    let position = Point {
                        X: x as f32,
                        Y: y as f32,
                    };
                    InkPoint::CreateInkPoint(position, 0.5).ok()
                })
                .collect::<Vec<_>>();
            let stroke = builder
                .CreateStrokeFromInkPoints(&IIterable::from(points), Matrix3x2::identity())?;
            container.AddStroke(&stroke)?;
        }
        let recognizer = InkRecognizerContainer::new()?;
        let results = recognizer
            .RecognizeAsync(&container, InkRecognitionTarget::All)?
            .join()?;
        let mut words = Vec::new();
        for i in 0..results.Size()? {
            let candidates = results.GetAt(i)?.GetTextCandidates()?;
            if candidates.Size()? > 0 {
                words.push(candidates.GetAt(0)?.to_string());
            }
        }
        Ok(words.join(" "))
    }
}

#[cfg(not(windows))]
mod imp {
    use super::Stroke;

    pub fn recognize(_strokes: Vec<Stroke>, done: impl super::Done) {
        done(Err(
            "handwriting recognition is not available on this platform yet".into(),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_scales_to_line_height() {
        let strokes = vec![vec![(100.0, 50.0), (110.0, 70.0)], vec![(120.0, 60.0)]];
        let n = normalize(&strokes);
        assert_eq!(n[0][0], (0.0, 0.0));
        assert_eq!(n[0][1], (30.0, TARGET_HEIGHT));
        assert_eq!(n[1][0], (60.0, 30.0));
    }

    /// Needs Windows with a handwriting recognizer; writes "1" as a vertical line.
    #[cfg(windows)]
    #[test]
    #[ignore = "uses the system recognizer"]
    fn recognizes_a_vertical_line() {
        let (tx, rx) = std::sync::mpsc::channel();
        recognize(
            vec![vec![(10.0, 0.0), (10.0, 50.0), (10.0, 100.0)]],
            move |r| {
                tx.send(r).unwrap();
            },
        );
        let text = rx.recv().unwrap().unwrap();
        assert!(!text.is_empty(), "{text:?}");
    }
}
