//! Recording from the microphone and playing sound back, for talktracks.

pub use imp::{Player, Recorder};

/// Sound recorded from the microphone: mono samples in -1..1 at `rate` per second.
#[derive(Clone, Debug, Default)]
pub struct Clip {
    pub rate: u32,
    pub samples: Vec<f32>,
}

#[cfg(not(target_arch = "wasm32"))]
mod imp {
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};

    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use cpal::{FromSample, Sample, SampleFormat, SizedSample, StreamConfig};

    use super::Clip;

    /// Records the default microphone until finished.
    pub struct Recorder {
        stream: cpal::Stream,
        samples: Arc<Mutex<Vec<f32>>>,
        rate: u32,
    }

    impl Recorder {
        pub fn start() -> Result<Self, String> {
            let device = cpal::default_host()
                .default_input_device()
                .ok_or("No microphone found")?;
            let supported = device
                .default_input_config()
                .map_err(|e| format!("The microphone cannot be used: {e}"))?;
            let format = supported.sample_format();
            let config: StreamConfig = supported.into();
            let samples = Arc::new(Mutex::new(Vec::new()));
            let stream = match format {
                SampleFormat::F32 => input::<f32>(&device, &config, &samples),
                SampleFormat::I16 => input::<i16>(&device, &config, &samples),
                SampleFormat::I32 => input::<i32>(&device, &config, &samples),
                SampleFormat::U16 => input::<u16>(&device, &config, &samples),
                SampleFormat::U8 => input::<u8>(&device, &config, &samples),
                other => return Err(format!("Unsupported microphone format {other}")),
            }?;
            stream
                .play()
                .map_err(|e| format!("The microphone cannot be started: {e}"))?;
            Ok(Self {
                stream,
                samples,
                rate: config.sample_rate,
            })
        }

        /// Stops recording and hands over what was recorded.
        pub fn finish(self) -> Clip {
            drop(self.stream);
            let samples =
                std::mem::take(&mut *self.samples.lock().unwrap_or_else(|e| e.into_inner()));
            Clip {
                rate: self.rate,
                samples,
            }
        }
    }

    /// An input stream that mixes `T` samples down to mono into `out`.
    fn input<T>(
        device: &cpal::Device,
        config: &StreamConfig,
        out: &Arc<Mutex<Vec<f32>>>,
    ) -> Result<cpal::Stream, String>
    where
        T: SizedSample,
        f32: FromSample<T>,
    {
        let channels = usize::from(config.channels.max(1));
        let out = out.clone();
        device
            .build_input_stream(
                *config,
                move |data: &[T], _: &cpal::InputCallbackInfo| {
                    if let Ok(mut out) = out.lock() {
                        out.extend(data.chunks(channels).map(|frame| {
                            frame.iter().map(|s| f32::from_sample(*s)).sum::<f32>()
                                / frame.len() as f32
                        }));
                    }
                },
                |err| log::warn!("microphone: {err}"),
                None,
            )
            .map_err(|e| format!("The microphone cannot be opened: {e}"))
    }

    /// Where playback is, shared with the audio thread.
    struct Shared {
        /// Position in source samples, as `f64` bits.
        position: AtomicU64,
        playing: AtomicBool,
    }

    /// Plays mono samples on the default speakers, from a position, pausable.
    pub struct Player {
        _stream: cpal::Stream,
        shared: Arc<Shared>,
        rate: u32,
        len: usize,
    }

    impl Player {
        /// Starts playing `samples` (at `rate` per second) from `from` seconds in.
        pub fn start(samples: Arc<[i16]>, rate: u32, from: f64) -> Result<Self, String> {
            let device = cpal::default_host()
                .default_output_device()
                .ok_or("No speakers found")?;
            let supported = device
                .default_output_config()
                .map_err(|e| format!("The speakers cannot be used: {e}"))?;
            let format = supported.sample_format();
            let config: StreamConfig = supported.into();
            let shared = Arc::new(Shared {
                position: AtomicU64::new((from * f64::from(rate)).max(0.0).to_bits()),
                playing: AtomicBool::new(true),
            });
            let len = samples.len();
            let source = Source {
                samples,
                step: f64::from(rate) / f64::from(config.sample_rate.max(1)),
                shared: shared.clone(),
            };
            let stream = match format {
                SampleFormat::F32 => output::<f32>(&device, &config, source),
                SampleFormat::I16 => output::<i16>(&device, &config, source),
                SampleFormat::I32 => output::<i32>(&device, &config, source),
                SampleFormat::U16 => output::<u16>(&device, &config, source),
                SampleFormat::U8 => output::<u8>(&device, &config, source),
                other => return Err(format!("Unsupported speaker format {other}")),
            }?;
            stream
                .play()
                .map_err(|e| format!("The speakers cannot be started: {e}"))?;
            Ok(Self {
                _stream: stream,
                shared,
                rate,
                len,
            })
        }

        /// Seconds played so far.
        pub fn position(&self) -> f64 {
            f64::from_bits(self.shared.position.load(Ordering::Relaxed)) / f64::from(self.rate)
        }

        pub fn seek(&self, seconds: f64) {
            let at = (seconds * f64::from(self.rate)).clamp(0.0, self.len as f64);
            self.shared.position.store(at.to_bits(), Ordering::Relaxed);
        }

        pub fn set_playing(&self, playing: bool) {
            self.shared.playing.store(playing, Ordering::Relaxed);
        }
    }

    /// The samples being played and how far to step through them per output sample.
    struct Source {
        samples: Arc<[i16]>,
        step: f64,
        shared: Arc<Shared>,
    }

    impl Source {
        /// The next output sample, linearly interpolated; silence when paused or done.
        fn next(&mut self) -> f32 {
            if !self.shared.playing.load(Ordering::Relaxed) {
                return 0.0;
            }
            let at = f64::from_bits(self.shared.position.load(Ordering::Relaxed));
            let i = at as usize;
            let Some(&a) = self.samples.get(i) else {
                return 0.0;
            };
            let b = self.samples.get(i + 1).copied().unwrap_or(a);
            let t = (at - i as f64) as f32;
            self.shared
                .position
                .store((at + self.step).to_bits(), Ordering::Relaxed);
            (f32::from(a) * (1.0 - t) + f32::from(b) * t) / 32768.0
        }
    }

    fn output<T>(
        device: &cpal::Device,
        config: &StreamConfig,
        mut source: Source,
    ) -> Result<cpal::Stream, String>
    where
        T: SizedSample + FromSample<f32>,
    {
        let channels = usize::from(config.channels.max(1));
        device
            .build_output_stream(
                *config,
                move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
                    for frame in data.chunks_mut(channels) {
                        let value = T::from_sample(source.next());
                        frame.fill(value);
                    }
                },
                |err| log::warn!("speakers: {err}"),
                None,
            )
            .map_err(|e| format!("The speakers cannot be opened: {e}"))
    }
}

#[cfg(target_arch = "wasm32")]
mod imp {
    use std::sync::Arc;

    use super::Clip;

    const UNAVAILABLE: &str = "Sound is not available in the browser yet";

    pub struct Recorder;

    impl Recorder {
        pub fn start() -> Result<Self, String> {
            Err(UNAVAILABLE.into())
        }

        pub fn finish(self) -> Clip {
            Clip::default()
        }
    }

    pub struct Player;

    impl Player {
        pub fn start(_samples: Arc<[i16]>, _rate: u32, _from: f64) -> Result<Self, String> {
            Err(UNAVAILABLE.into())
        }

        pub fn position(&self) -> f64 {
            0.0
        }

        pub fn seek(&self, _seconds: f64) {}

        pub fn set_playing(&self, _playing: bool) {}
    }
}
