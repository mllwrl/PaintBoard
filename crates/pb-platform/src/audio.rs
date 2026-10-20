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

        /// Whether the microphone was refused or failed after starting (in the browser,
        /// which asks first): the recording has no sound.
        pub fn refused(&self) -> bool {
            false
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
    //! Web Audio: the microphone through `getUserMedia`, sound played from an
    //! `AudioBuffer`.

    use std::cell::{Cell, RefCell};
    use std::rc::Rc;
    use std::sync::Arc;

    use wasm_bindgen::closure::Closure;
    use wasm_bindgen::{JsCast, JsValue};
    use web_sys::{
        AudioBuffer, AudioBufferSourceNode, AudioContext, AudioProcessingEvent, MediaStream,
        MediaStreamConstraints, MediaStreamTrack, ScriptProcessorNode,
    };

    use super::Clip;

    /// Samples per chunk the page hands over while recording.
    const CHUNK: u32 = 4096;

    type OnAudio = Closure<dyn FnMut(AudioProcessingEvent)>;

    fn js(err: JsValue) -> String {
        err.as_string().unwrap_or_else(|| format!("{err:?}"))
    }

    /// What recording has so far; the page fills it as sound comes.
    #[derive(Default)]
    struct Taken {
        samples: Vec<f32>,
        /// The microphone, once the browser was allowed to use it.
        stream: Option<MediaStream>,
        processor: Option<ScriptProcessorNode>,
        on_audio: Option<OnAudio>,
        refused: bool,
    }

    /// Records the default microphone until finished. The browser asks whether the page
    /// may use it; the sound starts when it is allowed, after silence for the wait, so it
    /// stays in step with the view.
    pub struct Recorder {
        context: AudioContext,
        taken: Rc<RefCell<Taken>>,
    }

    impl Recorder {
        pub fn start() -> Result<Self, String> {
            let window = web_sys::window().ok_or("No window")?;
            let devices = window
                .navigator()
                .media_devices()
                .map_err(|_| "No microphone access in this browser")?;
            let constraints = MediaStreamConstraints::new();
            constraints.set_audio(&JsValue::TRUE);
            let asked = devices
                .get_user_media_with_constraints(&constraints)
                .map_err(js)?;
            let context = AudioContext::new().map_err(js)?;
            let started = context.current_time();
            let taken = Rc::new(RefCell::new(Taken::default()));
            let (ctx, into) = (context.clone(), taken.clone());
            wasm_bindgen_futures::spawn_local(async move {
                let stream = match wasm_bindgen_futures::JsFuture::from(asked).await {
                    Ok(stream) => stream.unchecked_into::<MediaStream>(),
                    Err(err) => {
                        log::warn!("microphone: {}", js(err));
                        into.borrow_mut().refused = true;
                        return;
                    }
                };
                if let Err(err) = listen(&ctx, &stream, &into, started) {
                    log::warn!("microphone: {err}");
                    into.borrow_mut().refused = true;
                }
                into.borrow_mut().stream = Some(stream);
            });
            Ok(Self { context, taken })
        }

        /// Whether the microphone was refused or failed: the recording has no sound.
        pub fn refused(&self) -> bool {
            self.taken.borrow().refused
        }

        /// Stops recording and hands over what was recorded.
        pub fn finish(self) -> Clip {
            let mut taken = self.taken.borrow_mut();
            if let Some(processor) = taken.processor.take() {
                processor.set_onaudioprocess(None);
                let _ = processor.disconnect();
            }
            taken.on_audio = None;
            if let Some(stream) = taken.stream.take() {
                for track in stream.get_tracks().iter() {
                    track.unchecked_into::<MediaStreamTrack>().stop();
                }
            }
            let _ = self.context.close();
            Clip {
                rate: self.context.sample_rate() as u32,
                samples: std::mem::take(&mut taken.samples),
            }
        }
    }

    /// Feeds the microphone's sound into `taken`, after silence for the time it took to
    /// be allowed (since `started`, on `context`'s clock).
    fn listen(
        context: &AudioContext,
        stream: &MediaStream,
        taken: &Rc<RefCell<Taken>>,
        started: f64,
    ) -> Result<(), String> {
        let source = context.create_media_stream_source(stream).map_err(js)?;
        let processor = context
            .create_script_processor_with_buffer_size_and_number_of_input_channels_and_number_of_output_channels(
                CHUNK, 1, 1,
            )
            .map_err(js)?;
        let rate = f64::from(context.sample_rate());
        let waited = ((context.current_time() - started).max(0.0) * rate) as usize;
        taken.borrow_mut().samples.resize(waited, 0.0);
        let into = taken.clone();
        let on_audio = OnAudio::new(move |event: AudioProcessingEvent| {
            if let Ok(buffer) = event.input_buffer()
                && let Ok(chunk) = buffer.get_channel_data(0)
            {
                into.borrow_mut().samples.extend_from_slice(&chunk);
            }
        });
        processor.set_onaudioprocess(Some(on_audio.as_ref().unchecked_ref()));
        source.connect_with_audio_node(&processor).map_err(js)?;
        // The processor only runs while connected on; it writes nothing, so it is silent.
        processor
            .connect_with_audio_node(&context.destination())
            .map_err(js)?;
        let mut taken = taken.borrow_mut();
        taken.processor = Some(processor);
        taken.on_audio = Some(on_audio);
        Ok(())
    }

    /// Plays mono samples, from a position, pausable.
    pub struct Player {
        context: AudioContext,
        buffer: AudioBuffer,
        node: RefCell<Option<AudioBufferSourceNode>>,
        /// Seconds into the sound, and when that was on the context's clock.
        from: Cell<(f64, f64)>,
    }

    impl Player {
        /// Starts playing `samples` (at `rate` per second) from `from` seconds in.
        pub fn start(samples: Arc<[i16]>, rate: u32, from: f64) -> Result<Self, String> {
            if samples.is_empty() {
                return Err("No sound".into());
            }
            let context = AudioContext::new().map_err(js)?;
            let buffer = context
                .create_buffer(1, samples.len() as u32, rate as f32)
                .map_err(js)?;
            let floats: Vec<f32> = samples.iter().map(|&s| f32::from(s) / 32768.0).collect();
            buffer.copy_to_channel(&floats, 0).map_err(js)?;
            // Pages may only make sound after a click; this follows one.
            let _ = context.resume();
            let player = Self {
                context,
                buffer,
                node: RefCell::new(None),
                from: Cell::new((0.0, 0.0)),
            };
            player.seek(from);
            Ok(player)
        }

        /// Seconds played so far.
        pub fn position(&self) -> f64 {
            let (at, since) = self.from.get();
            (at + self.context.current_time() - since).clamp(0.0, self.buffer.duration())
        }

        pub fn seek(&self, seconds: f64) {
            let at = seconds.clamp(0.0, self.buffer.duration());
            if let Some(old) = self.node.borrow_mut().take() {
                #[allow(deprecated)]
                let _ = old.stop();
                let _ = old.disconnect();
            }
            self.from.set((at, self.context.current_time()));
            let Ok(node) = self.context.create_buffer_source() else {
                return;
            };
            node.set_buffer(Some(&self.buffer));
            let started = node
                .connect_with_audio_node(&self.context.destination())
                .and_then(|_| node.start_with_when_and_grain_offset(0.0, at));
            match started {
                Ok(()) => *self.node.borrow_mut() = Some(node),
                Err(err) => log::warn!("talktrack sound: {}", js(err)),
            }
        }

        /// Pauses the context's clock together with the sound, or runs both on.
        pub fn set_playing(&self, playing: bool) {
            let _ = if playing {
                self.context.resume()
            } else {
                self.context.suspend()
            };
        }
    }

    impl Drop for Player {
        fn drop(&mut self) {
            let _ = self.context.close();
        }
    }
}
