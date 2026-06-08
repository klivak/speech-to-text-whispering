//! Історія розпізнаного тексту.
//!
//! Кожен успішний результат дописується окремим рядком JSON (формат JSONL) у
//! `%APPDATA%\whisper-uk\history.jsonl`. JSONL зручний: легко дописувати в кінець
//! без перечитування всього файлу й легко парсити рядок за рядком.

use serde::Serialize;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// Один запис історії (серіалізується в один рядок JSONL).
#[derive(Serialize)]
struct Entry<'a> {
    /// Час у вигляді "YYYY-MM-DD HH:MM:SS UTC" — людиночитний.
    time: String,
    /// Час у мілісекундах від епохи (для сортування/обробки).
    ts_ms: u128,
    /// Розпізнаний текст.
    text: &'a str,
}

/// Шлях до history.jsonl (`%APPDATA%\whisper-uk\history.jsonl`).
pub fn path() -> PathBuf {
    let mut dir = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
    dir.push("whisper-uk");
    dir.push("history.jsonl");
    dir
}

/// Дописує один запис у кінець файлу. Помилки лише логуються — історія не критична.
pub fn append(text: &str) {
    if text.trim().is_empty() {
        return;
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let entry = Entry {
        time: format_utc(now.as_secs()),
        ts_ms: now.as_millis(),
        text,
    };
    let line = match serde_json::to_string(&entry) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("history: серіалізація не вдалась: {e}");
            return;
        }
    };
    let path = path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match OpenOptions::new().create(true).append(true).open(&path) {
        Ok(mut f) => {
            if let Err(e) = writeln!(f, "{line}") {
                eprintln!("history: запис не вдався: {e}");
            }
        }
        Err(e) => eprintln!("history: відкриття файлу не вдалось: {e}"),
    }
}

/// Форматує секунди від епохи як "YYYY-MM-DD HH:MM:SS UTC" (алгоритм цивільної дати,
/// без зовнішніх залежностей; Howard Hinnant `civil_from_days`).
fn format_utc(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (h, mi, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);

    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let y = if m <= 2 { y + 1 } else { y };

    format!("{y:04}-{m:02}-{d:02} {h:02}:{mi:02}:{s:02} UTC")
}
