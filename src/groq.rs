//! Відправка WAV у Groq Cloud (OpenAI-сумісний endpoint транскрипції).

use crate::config::Config;
use serde::{Deserialize, Serialize};

const ENDPOINT: &str = "https://api.groq.com/openai/v1/audio/transcriptions";

/// Залишок rate-limit, зчитаний із заголовків відповіді Groq.
/// Денний ліміт (RPD) Groq у заголовках не віддає — його рахуємо самі у `Stats`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Limits {
    /// `x-ratelimit-remaining-requests` — лишилось запитів у вікні.
    pub remaining_requests: Option<String>,
    /// `x-ratelimit-limit-requests` — стеля запитів у вікні.
    pub limit_requests: Option<String>,
    /// `x-ratelimit-remaining-audio-seconds` — лишилось секунд аудіо.
    pub remaining_audio_seconds: Option<String>,
    /// `x-ratelimit-limit-audio-seconds` — стеля секунд аудіо.
    pub limit_audio_seconds: Option<String>,
    /// `x-ratelimit-reset-audio-seconds` — коли скинеться ліміт аудіо.
    pub reset_audio_seconds: Option<String>,
}

impl Limits {
    fn from_headers(h: &reqwest::header::HeaderMap) -> Self {
        let get = |name: &str| {
            h.get(name)
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string())
        };
        Self {
            remaining_requests: get("x-ratelimit-remaining-requests"),
            limit_requests: get("x-ratelimit-limit-requests"),
            remaining_audio_seconds: get("x-ratelimit-remaining-audio-seconds"),
            limit_audio_seconds: get("x-ratelimit-limit-audio-seconds"),
            reset_audio_seconds: get("x-ratelimit-reset-audio-seconds"),
        }
    }

    /// Чи є хоч якісь дані про ліміт.
    pub fn is_some(&self) -> bool {
        self.remaining_requests.is_some() || self.remaining_audio_seconds.is_some()
    }
}

/// Транскрибує WAV-байти й повертає розпізнаний текст + залишок лімітів.
pub fn transcribe(cfg: &Config, wav: Vec<u8>) -> Result<(String, Limits), String> {
    if cfg.groq_api_key.trim().is_empty() {
        return Err("Не вказано Groq API key у config.json".to_string());
    }

    let part = reqwest::blocking::multipart::Part::bytes(wav)
        .file_name("audio.wav")
        .mime_str("audio/wav")
        .map_err(|e| format!("mime: {e}"))?;

    let mut form = reqwest::blocking::multipart::Form::new()
        .part("file", part)
        .text("model", cfg.model.clone())
        .text("response_format", "text");

    // Порожня мова = автовизначення; інакше явно вказуємо (точніше для uk).
    if !cfg.language.trim().is_empty() {
        form = form.text("language", cfg.language.clone());
    }

    let client = reqwest::blocking::Client::new();
    let resp = client
        .post(ENDPOINT)
        .bearer_auth(&cfg.groq_api_key)
        .multipart(form)
        .send()
        .map_err(|e| format!("Запит до Groq не вдався: {e}"))?;

    let status = resp.status();
    let limits = Limits::from_headers(resp.headers());
    let body = resp.text().unwrap_or_default();
    if !status.is_success() {
        return Err(format!("Groq повернув {status}: {body}"));
    }
    Ok((body.trim().to_string(), limits))
}
