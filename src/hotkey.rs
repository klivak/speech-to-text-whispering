//! Парсинг рядка хоткея виду "Ctrl+Alt+Space" у global_hotkey::HotKey.

use global_hotkey::hotkey::{Code, HotKey, Modifiers};

/// Перетворює рядок ("Ctrl+Alt+Space") на HotKey.
///
/// Підтримує модифікатори Ctrl/Control, Alt, Shift, Super/Win/Meta та
/// поширені клавіші (літери, цифри, F1–F12, Space, Enter тощо).
pub fn parse(spec: &str) -> Result<HotKey, String> {
    let mut mods = Modifiers::empty();
    let mut code: Option<Code> = None;

    for raw in spec.split('+') {
        let token = raw.trim();
        if token.is_empty() {
            continue;
        }
        match token.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => mods |= Modifiers::CONTROL,
            "alt" | "option" => mods |= Modifiers::ALT,
            "shift" => mods |= Modifiers::SHIFT,
            "super" | "win" | "meta" | "cmd" | "command" => mods |= Modifiers::META,
            other => {
                code = Some(parse_code(other)?);
            }
        }
    }

    let code = code.ok_or_else(|| format!("У хоткеї '{spec}' немає основної клавіші"))?;
    Ok(HotKey::new(Some(mods), code))
}

fn parse_code(key: &str) -> Result<Code, String> {
    let c = match key {
        "space" => Code::Space,
        "enter" | "return" => Code::Enter,
        "tab" => Code::Tab,
        "esc" | "escape" => Code::Escape,
        "backquote" | "grave" | "`" => Code::Backquote,
        "a" => Code::KeyA, "b" => Code::KeyB, "c" => Code::KeyC, "d" => Code::KeyD,
        "e" => Code::KeyE, "f" => Code::KeyF, "g" => Code::KeyG, "h" => Code::KeyH,
        "i" => Code::KeyI, "j" => Code::KeyJ, "k" => Code::KeyK, "l" => Code::KeyL,
        "m" => Code::KeyM, "n" => Code::KeyN, "o" => Code::KeyO, "p" => Code::KeyP,
        "q" => Code::KeyQ, "r" => Code::KeyR, "s" => Code::KeyS, "t" => Code::KeyT,
        "u" => Code::KeyU, "v" => Code::KeyV, "w" => Code::KeyW, "x" => Code::KeyX,
        "y" => Code::KeyY, "z" => Code::KeyZ,
        "0" => Code::Digit0, "1" => Code::Digit1, "2" => Code::Digit2, "3" => Code::Digit3,
        "4" => Code::Digit4, "5" => Code::Digit5, "6" => Code::Digit6, "7" => Code::Digit7,
        "8" => Code::Digit8, "9" => Code::Digit9,
        "f1" => Code::F1, "f2" => Code::F2, "f3" => Code::F3, "f4" => Code::F4,
        "f5" => Code::F5, "f6" => Code::F6, "f7" => Code::F7, "f8" => Code::F8,
        "f9" => Code::F9, "f10" => Code::F10, "f11" => Code::F11, "f12" => Code::F12,
        other => return Err(format!("Невідома клавіша: '{other}'")),
    };
    Ok(c)
}
