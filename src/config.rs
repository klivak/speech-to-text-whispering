//! Завантаження/збереження налаштувань із config.json у теці застосунку.
//!
//! Файл лежить у `%APPDATA%\whisper-uk\config.json`. Якщо його немає —
//! створюється з дефолтами, і користувач лише вписує туди свій Groq-ключ.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Режим хоткея.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HotkeyMode {
    /// Натиснув — почав запис, натиснув ще раз — зупинив.
    Toggle,
    /// Тримаєш — пише, відпустив — зупинив (push-to-talk).
    PushToTalk,
}

/// Текст-підказка, що завжди пишеться у config.json (бо JSON не має коментарів).
fn help_default() -> Vec<String> {
    vec![
        "Як отримати ключ Groq (безкоштовно, картка не потрібна):".to_string(),
        "1) Відкрий https://console.groq.com/keys і залогінься (email або Google).".to_string(),
        "2) Натисни 'Create API Key', скопіюй ключ виду gsk_... (показується раз).".to_string(),
        "3) Встав його нижче у поле groq_api_key.".to_string(),
        "4) У меню трея обери 'Перезавантажити конфіг'.".to_string(),
    ]
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Інструкція по ключу (тільки для читання людиною; на роботу не впливає).
    #[serde(rename = "_як_отримати_ключ", default = "help_default")]
    pub help: Vec<String>,
    /// Ключ Groq виду `gsk_...`. Без нього транскрипція не працює.
    pub groq_api_key: String,
    /// Модель Whisper на Groq.
    pub model: String,
    /// Код мови ISO-639-1, напр. "uk". Порожній рядок = автовизначення.
    pub language: String,
    /// Глобальний хоткей, напр. "Ctrl+Alt+Space".
    pub hotkey: String,
    /// Режим хоткея: toggle або push_to_talk.
    pub mode: HotkeyMode,
    /// Автоматично вставляти текст у курсор (Ctrl+V) після транскрипції.
    pub auto_paste: bool,
    /// Автоматично копіювати розпізнаний текст у буфер обміну. За замовчуванням
    /// увімкнено — навіть якщо автовставка зірветься (бо фокус перехопило
    /// повідомлення/інше вікно), текст лишиться в буфері й його можна вставити вручну.
    #[serde(default = "default_true")]
    pub auto_copy: bool,
    /// Зберігати історію розпізнаного тексту у history.jsonl. За замовчуванням увімкнено.
    #[serde(default = "default_true")]
    pub save_history: bool,
    /// Короткий звуковий біп на старт/кінець запису.
    #[serde(default = "default_true")]
    pub sound_feedback: bool,
    /// Показувати кругле кольорове коло по центру екрана під час запису/обробки.
    #[serde(default = "default_true")]
    pub show_overlay: bool,
    /// Скільки мс дозаписувати «хвіст» після відпускання клавіші (push-to-talk),
    /// щоб не зрізати кінець слова. 0 = зупиняти миттєво.
    #[serde(default = "default_release_tail")]
    pub release_tail_ms: u64,
}

/// Дефолт для булевих полів, які мають бути увімкнені (для `serde(default)`).
fn default_true() -> bool {
    true
}

/// Дефолтний «хвіст» дозапису після відпускання клавіші.
fn default_release_tail() -> u64 {
    400
}

impl Default for Config {
    fn default() -> Self {
        Self {
            help: help_default(),
            groq_api_key: String::new(),
            model: "whisper-large-v3".to_string(),
            language: "uk".to_string(),
            hotkey: "Ctrl+Alt+Space".to_string(),
            mode: HotkeyMode::Toggle,
            auto_paste: true,
            auto_copy: true,
            save_history: true,
            sound_feedback: true,
            show_overlay: true,
            release_tail_ms: 400,
        }
    }
}

impl Config {
    /// Шлях до config.json (`%APPDATA%\whisper-uk\config.json`).
    pub fn path() -> PathBuf {
        let mut dir = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
        dir.push("whisper-uk");
        dir.push("config.json");
        dir
    }

    /// Завантажує конфіг; якщо файлу немає — створює з дефолтами.
    pub fn load_or_create() -> Self {
        let path = Self::path();
        match std::fs::read_to_string(&path) {
            Ok(text) => match serde_json::from_str::<Config>(&text) {
                Ok(cfg) => cfg,
                Err(e) => {
                    eprintln!("config.json пошкоджено ({e}); беру дефолти");
                    Config::default()
                }
            },
            Err(_) => {
                let cfg = Config::default();
                cfg.save();
                cfg
            }
        }
    }

    /// Зберігає поточний конфіг у файл (створює теку за потреби).
    pub fn save(&self) {
        let path = Self::path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match serde_json::to_string_pretty(self) {
            Ok(text) => {
                if let Err(e) = std::fs::write(&path, text) {
                    eprintln!("Не вдалося зберегти config.json: {e}");
                }
            }
            Err(e) => eprintln!("Серіалізація config.json не вдалась: {e}"),
        }
    }
}
