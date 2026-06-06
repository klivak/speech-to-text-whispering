//! Статистика використання: скільки запитів до Groq, успішних/невдалих,
//! скільки слів/символів розпізнано, скільки секунд аудіо відправлено.
//!
//! Зберігається у `%APPDATA%\whisper-uk\stats.json`. Лічильник «сьогодні»
//! рахується по UTC-добі (як і ліміти Groq), щоб орієнтуватись на rate-limit.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Stats {
    /// Усього запитів до Groq (успішних + невдалих).
    pub requests: u64,
    /// Успішні транскрипції.
    pub ok: u64,
    /// Невдалі (помилка мережі / ключа / ліміту).
    pub failed: u64,
    /// Сумарно розпізнано слів.
    pub total_words: u64,
    /// Сумарно розпізнано символів.
    pub total_chars: u64,
    /// Сумарно відправлено аудіо, мс.
    pub total_audio_ms: u64,
    /// Номер UTC-доби останнього запиту (секунди/86400), для денного лічильника.
    pub today_day: u64,
    /// Запитів за поточну UTC-добу.
    pub today_requests: u64,
}

/// Номер поточної UTC-доби (днів від епохи).
fn current_day() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() / 86_400)
        .unwrap_or(0)
}

impl Stats {
    pub fn path() -> PathBuf {
        let mut dir = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
        dir.push("whisper-uk");
        dir.push("stats.json");
        dir
    }

    pub fn load() -> Self {
        std::fs::read_to_string(Self::path())
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        let path = Self::path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(text) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(path, text);
        }
    }

    /// Враховує один завершений виклик до Groq.
    pub fn record(&mut self, result: &Result<String, String>, audio_ms: u64) {
        let day = current_day();
        if day != self.today_day {
            self.today_day = day;
            self.today_requests = 0;
        }
        self.requests += 1;
        self.today_requests += 1;
        match result {
            Ok(text) => {
                self.ok += 1;
                self.total_chars += text.chars().count() as u64;
                self.total_words += text.split_whitespace().count() as u64;
                self.total_audio_ms += audio_ms;
            }
            Err(_) => self.failed += 1,
        }
    }

    /// Короткий рядок для пункту меню.
    pub fn summary(&self) -> String {
        format!(
            "📊 {} запитів · {} слів · сьогодні {}",
            self.requests, self.total_words, self.today_requests
        )
    }
}
