//! Робота з буфером обміну та автовставкою тексту в активне поле.

use enigo::{Direction, Enigo, Key, Keyboard, Settings};

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
        .key(Key::Unicode('v'), Direction::Click)
        .map_err(|e| format!("v: {e}"))?;
    enigo
        .key(Key::Control, Direction::Release)
        .map_err(|e| format!("ctrl up: {e}"))?;
    Ok(())
}
