use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::{
    atomic::{AtomicBool, AtomicU32, Ordering},
    Arc, Mutex,
};

pub const RATE: u32 = 24_000;
pub const MAX_SECONDS: usize = 480;

pub fn devices() -> Result<Vec<String>, String> {
    cpal::default_host()
        .input_devices()
        .map_err(|e| e.to_string())?
        .map(|d| d.name().map_err(|e| e.to_string()))
        .collect()
}

/// State shared between the capture callback and the recording loop.
#[derive(Clone)]
pub struct Shared {
    samples: Arc<Mutex<Vec<i16>>>,
    pub peak: Arc<AtomicU32>,
    pub failed: Arc<AtomicBool>,
    pub full: Arc<AtomicBool>,
}

impl Shared {
    fn take(&self) -> Vec<i16> {
        std::mem::take(&mut *self.samples.lock().unwrap_or_else(|e| e.into_inner()))
    }
}

pub struct Capture {
    _stream: cpal::Stream,
    pub shared: Shared,
}

impl Capture {
    pub fn start(name: &str) -> Result<Self, String> {
        let host = cpal::default_host();
        let device = if name.is_empty() {
            host.default_input_device()
        } else {
            host.input_devices()
                .map_err(|e| e.to_string())?
                .find(|d| d.name().ok().as_deref() == Some(name))
        }
        .ok_or("Microphone unavailable. Refresh devices and choose a microphone.")?;
        let supported = device
            .default_input_config()
            .map_err(|e| format!("Cannot open microphone: {e}"))?;
        let config: cpal::StreamConfig = supported.clone().into();
        let shared = Shared {
            samples: Arc::new(Mutex::new(Vec::with_capacity(RATE as usize * 30))),
            peak: Arc::default(),
            failed: Arc::default(),
            full: Arc::default(),
        };
        let stream = match supported.sample_format() {
            cpal::SampleFormat::F32 => input::<f32>(&device, &config, shared.clone()),
            cpal::SampleFormat::I16 => input::<i16>(&device, &config, shared.clone()),
            cpal::SampleFormat::U16 => input::<u16>(&device, &config, shared.clone()),
            _ => return Err("Unsupported microphone sample format".into()),
        }?;
        stream
            .play()
            .map_err(|e| format!("Cannot start microphone: {e}"))?;
        Ok(Self {
            _stream: stream,
            shared,
        })
    }
    /// Stop the input stream before draining its final samples.
    pub fn finish(self) -> Vec<i16> {
        let Self { _stream, shared } = self;
        drop(_stream);
        shared.take()
    }
    pub fn take(&self) -> Vec<i16> {
        self.shared.take()
    }
}

fn input<T: cpal::SizedSample + cpal::Sample>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    shared: Shared,
) -> Result<cpal::Stream, String>
where
    f32: cpal::FromSample<T>,
{
    let Shared {
        samples,
        peak,
        failed,
        full,
    } = shared;
    let channels = config.channels as usize;
    let source_rate = config.sample_rate.0;
    let mut phase: u64 = 0;
    let mut sum = 0.0f32;
    let mut count = 0u32;
    device
        .build_input_stream(
            config,
            move |data: &[T], _| {
                let mut out = samples.lock().unwrap_or_else(|e| e.into_inner());
                let mut max = 0.0f32;
                for frame in data.chunks_exact(channels) {
                    let value =
                        frame.iter().map(|x| x.to_sample::<f32>()).sum::<f32>() / channels as f32;
                    max = max.max(value.abs());
                    sum += value;
                    count += 1;
                    phase += RATE as u64;
                    while phase >= source_rate as u64 {
                        phase -= source_rate as u64;
                        if out.len() < RATE as usize * MAX_SECONDS {
                            out.push(((sum / count as f32).clamp(-1.0, 1.0) * 32767.0) as i16);
                        } else {
                            full.store(true, Ordering::Relaxed);
                        }
                    }
                    if phase < RATE as u64 {
                        sum = 0.0;
                        count = 0;
                    }
                }
                peak.store((max.clamp(0.0, 1.0) * 100.0) as u32, Ordering::Relaxed);
            },
            move |_| {
                failed.store(true, Ordering::Relaxed);
            },
            None,
        )
        .map_err(|e| format!("Microphone permission or device error: {e}"))
}

pub fn playback(samples: Arc<Vec<i16>>, cancelled: Arc<AtomicBool>) -> Result<(), String> {
    let device = cpal::default_host()
        .default_output_device()
        .ok_or("No output device")?;
    let supported = device.default_output_config().map_err(|e| e.to_string())?;
    let config: cpal::StreamConfig = supported.clone().into();
    let done = Arc::new(AtomicBool::new(false));
    let stream = match supported.sample_format() {
        cpal::SampleFormat::F32 => output::<f32>(&device, &config, samples, done.clone()),
        cpal::SampleFormat::I16 => output::<i16>(&device, &config, samples, done.clone()),
        cpal::SampleFormat::U16 => output::<u16>(&device, &config, samples, done.clone()),
        _ => return Err("Unsupported speaker format".into()),
    }?;
    stream.play().map_err(|e| e.to_string())?;
    while !done.load(Ordering::Relaxed) && !cancelled.load(Ordering::Relaxed) {
        std::thread::sleep(std::time::Duration::from_millis(30));
    }
    Ok(())
}
fn output<T: cpal::SizedSample + cpal::FromSample<f32>>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    samples: Arc<Vec<i16>>,
    done: Arc<AtomicBool>,
) -> Result<cpal::Stream, String> {
    let channels = config.channels as usize;
    let rate = config.sample_rate.0 as u64;
    let mut cursor = 0u64;
    let failed = done.clone();
    device
        .build_output_stream(
            config,
            move |data: &mut [T], _| {
                for frame in data.chunks_mut(channels) {
                    let index = (cursor * RATE as u64 / rate) as usize;
                    let sample = samples.get(index).copied().unwrap_or(0) as f32 / 32768.0;
                    if index >= samples.len() {
                        done.store(true, Ordering::Relaxed);
                    }
                    frame.fill(T::from_sample(sample));
                    cursor += 1;
                }
            },
            move |_| {
                failed.store(true, Ordering::Relaxed);
            },
            None,
        )
        .map_err(|e| e.to_string())
}
