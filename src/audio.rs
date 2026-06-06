//! Запис мікрофона через cpal у моно-буфер f32 та кодування у WAV (16-bit PCM).

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::{Arc, Mutex};

/// Активний запис: тримає cpal-стрім живим і накопичує семпли.
pub struct Recording {
    stream: cpal::Stream,
    samples: Arc<Mutex<Vec<f32>>>,
    sample_rate: u32,
}

impl Recording {
    /// Стартує запис із дефолтного мікрофона. Семпли зводяться в моно.
    pub fn start() -> Result<Self, String> {
        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .ok_or_else(|| "Не знайдено мікрофон".to_string())?;
        let config = device
            .default_input_config()
            .map_err(|e| format!("default_input_config: {e}"))?;

        let channels = config.channels() as usize;
        let sample_rate = config.sample_rate().0;
        let samples = Arc::new(Mutex::new(Vec::<f32>::new()));
        let buf = samples.clone();

        let err_fn = |e| eprintln!("Помилка аудіо-стріму: {e}");

        // cpal віддає різні формати; нас цікавить f32, i16, u16.
        let stream = match config.sample_format() {
            cpal::SampleFormat::F32 => device.build_input_stream(
                &config.into(),
                move |data: &[f32], _: &_| push_mono(&buf, data, channels, |s| s),
                err_fn,
                None,
            ),
            cpal::SampleFormat::I16 => device.build_input_stream(
                &config.into(),
                move |data: &[i16], _: &_| {
                    push_mono(&buf, data, channels, |s| s as f32 / i16::MAX as f32)
                },
                err_fn,
                None,
            ),
            cpal::SampleFormat::U16 => device.build_input_stream(
                &config.into(),
                move |data: &[u16], _: &_| {
                    push_mono(&buf, data, channels, |s| {
                        (s as f32 - u16::MAX as f32 / 2.0) / (u16::MAX as f32 / 2.0)
                    })
                },
                err_fn,
                None,
            ),
            other => return Err(format!("Непідтримуваний формат семплів: {other:?}")),
        }
        .map_err(|e| format!("build_input_stream: {e}"))?;

        stream.play().map_err(|e| format!("stream.play: {e}"))?;

        Ok(Self {
            stream,
            samples,
            sample_rate,
        })
    }

    /// Зупиняє запис і повертає WAV-байти (16-bit PCM, моно).
    pub fn stop_to_wav(self) -> Result<Vec<u8>, String> {
        drop(self.stream); // зупиняє захоплення
        let samples = self
            .samples
            .lock()
            .map_err(|_| "mutex poisoned".to_string())?
            .clone();
        encode_wav(&samples, self.sample_rate)
    }
}

/// Зводить кадри в моно (усереднює канали) і дописує у спільний буфер.
fn push_mono<T: Copy>(
    buf: &Arc<Mutex<Vec<f32>>>,
    data: &[T],
    channels: usize,
    to_f32: impl Fn(T) -> f32,
) {
    if channels == 0 {
        return;
    }
    let mut guard = match buf.lock() {
        Ok(g) => g,
        Err(_) => return,
    };
    for frame in data.chunks(channels) {
        let sum: f32 = frame.iter().map(|&s| to_f32(s)).sum();
        guard.push(sum / channels as f32);
    }
}

/// Кодує моно f32-семпли у WAV (16-bit PCM).
fn encode_wav(samples: &[f32], sample_rate: u32) -> Result<Vec<u8>, String> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut cursor = std::io::Cursor::new(Vec::<u8>::new());
    {
        let mut writer =
            hound::WavWriter::new(&mut cursor, spec).map_err(|e| format!("WavWriter: {e}"))?;
        for &s in samples {
            let v = (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
            writer
                .write_sample(v)
                .map_err(|e| format!("write_sample: {e}"))?;
        }
        writer.finalize().map_err(|e| format!("finalize: {e}"))?;
    }
    Ok(cursor.into_inner())
}
