//! Відправка WAV у Groq Cloud (OpenAI-сумісний endpoint транскрипції).

use crate::config::Config;

const ENDPOINT: &str = "https://api.groq.com/openai/v1/audio/transcriptions";

/// Транскрибує WAV-байти й повертає розпізнаний текст.
pub fn transcribe(cfg: &Config, wav: Vec<u8>) -> Result<String, String> {
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
    let body = resp.text().unwrap_or_default();
    if !status.is_success() {
        return Err(format!("Groq повернув {status}: {body}"));
    }
    Ok(body.trim().to_string())
}
