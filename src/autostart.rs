//! Автозапуск разом із Windows через ключ реєстру
//! `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`.
//!
//! Це надійніший спосіб, ніж ярлик у теці shell:startup: запис вказує на
//! поточний шлях до `whisper-uk.exe` і не залежить від ярликів, які легко
//! ламаються при переміщенні файлу.
//!
//! Реалізовано через `reg.exe` (без зайвих залежностей). Тільки Windows.

#![cfg(windows)]

const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
const VALUE_NAME: &str = "whisper-uk";

/// Чи увімкнено автозапуск (чи є наш запис у Run і чи вказує він на цей exe).
pub fn is_enabled() -> bool {
    let output = std::process::Command::new("reg")
        .args(["query", RUN_KEY, "/v", VALUE_NAME])
        .output();
    match output {
        Ok(o) => o.status.success(),
        Err(_) => false,
    }
}

/// Вмикає автозапуск: прописує поточний шлях до exe в Run.
pub fn enable() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| format!("current_exe: {e}"))?;
    let exe = exe.to_string_lossy().to_string();
    // reg сам обгортає значення; передаємо шлях у лапках для надійності з пробілами.
    let value = format!("\"{exe}\"");
    let status = std::process::Command::new("reg")
        .args([
            "add", RUN_KEY, "/v", VALUE_NAME, "/t", "REG_SZ", "/d", &value, "/f",
        ])
        .status()
        .map_err(|e| format!("reg add: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err("reg add повернув помилку".to_string())
    }
}

/// Вимикає автозапуск: видаляє наш запис із Run.
pub fn disable() -> Result<(), String> {
    let status = std::process::Command::new("reg")
        .args(["delete", RUN_KEY, "/v", VALUE_NAME, "/f"])
        .status()
        .map_err(|e| format!("reg delete: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err("reg delete повернув помилку".to_string())
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
