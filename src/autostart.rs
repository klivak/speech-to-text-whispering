//! Автозапуск разом із Windows через ключ реєстру
//! `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`.
//!
//! Це надійніший спосіб, ніж ярлик у теці shell:startup: запис вказує на
//! поточний шлях до `whisper-uk.exe` і не залежить від ярликів, які легко
//! ламаються при переміщенні файлу.
//!
//! Реалізовано через прямий доступ до реєстру (winreg), а не через запуск
//! `reg.exe`: безконсольний застосунок не завжди може запустити консольну
//! утиліту (помилка 0xc0000142 — STATUS_DLL_INIT_FAILED). Тільки Windows.

#![cfg(windows)]

use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE};
use winreg::RegKey;

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const VALUE_NAME: &str = "whisper-uk";

/// Поточний шлях до exe у лапках (надійно для шляхів із пробілами).
fn exe_value() -> Result<String, String> {
    let exe = std::env::current_exe().map_err(|e| format!("current_exe: {e}"))?;
    Ok(format!("\"{}\"", exe.to_string_lossy()))
}

/// Чи увімкнено автозапуск (чи є наш запис у Run).
pub fn is_enabled() -> bool {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let Ok(run) = hkcu.open_subkey_with_flags(RUN_KEY, KEY_READ) else {
        return false;
    };
    run.get_value::<String, _>(VALUE_NAME).is_ok()
}

/// Вмикає автозапуск: прописує поточний шлях до exe в Run.
pub fn enable() -> Result<(), String> {
    let value = exe_value()?;
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (run, _) = hkcu
        .create_subkey_with_flags(RUN_KEY, KEY_WRITE)
        .map_err(|e| format!("open Run key: {e}"))?;
    run.set_value(VALUE_NAME, &value)
        .map_err(|e| format!("set value: {e}"))
}

/// Вимикає автозапуск: видаляє наш запис із Run.
pub fn disable() -> Result<(), String> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let run = hkcu
        .open_subkey_with_flags(RUN_KEY, KEY_WRITE)
        .map_err(|e| format!("open Run key: {e}"))?;
    match run.delete_value(VALUE_NAME) {
        Ok(()) => Ok(()),
        // Значення вже відсутнє — вважаємо успіхом.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("delete value: {e}")),
    }
}

/// Перемикає стан автозапуску; повертає новий стан (true = увімкнено).
pub fn toggle() -> Result<bool, String> {
    if is_enabled() {
        disable()?;
        Ok(false)
    } else {
        enable()?;
        Ok(true)
    }
}
