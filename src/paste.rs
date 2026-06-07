//! Робота з буфером обміну та автовставкою тексту в активне поле.

use enigo::{Direction, Enigo, Key, Keyboard, Settings};

/// Клавіша «V» для Ctrl+V. На Windows шлемо віртуальний код VK_V (0x56) —
/// він не залежить від активної розкладки (інакше на кирилиці `Key::Unicode('v')`
/// падає: латинської 'v' у розкладці немає). На інших ОС лишаємо Unicode.
#[cfg(target_os = "windows")]
const KEY_V: Key = Key::Other(0x56);
#[cfg(not(target_os = "windows"))]
const KEY_V: Key = Key::Unicode('v');

/// Копіює текст у буфер обміну.
pub fn copy_to_clipboard(text: &str) -> Result<(), String> {
    let mut clipboard = arboard::Clipboard::new().map_err(|e| format!("clipboard: {e}"))?;
    clipboard
        .set_text(text.to_string())
        .map_err(|e| format!("set_text: {e}"))
}

/// Емулює Ctrl+V, щоб вставити вміст буфера в активне поле.
pub fn paste_at_cursor() -> Result<(), String> {
    let mut enigo = Enigo::new(&Settings::default()).map_err(|e| format!("enigo: {e}"))?;
    // Невелика пауза, щоб ОС встигла оновити буфер перед вставкою.
    std::thread::sleep(std::time::Duration::from_millis(120));
    enigo
        .key(Key::Control, Direction::Press)
        .map_err(|e| format!("ctrl down: {e}"))?;
    enigo
        .key(KEY_V, Direction::Click)
        .map_err(|e| format!("v: {e}"))?;
    enigo
        .key(Key::Control, Direction::Release)
        .map_err(|e| format!("ctrl up: {e}"))?;
    Ok(())
}
