//! Звуковий фідбек: короткі біпи на старт/кінець запису.
//!
//! Біп грається в окремому потоці, бо `Beep` блокує на час звучання,
//! а ми не хочемо підвішувати UI-потік.

/// Висхідний тон — почали запис.
pub fn play_start() {
    beep(880, 120);
}

/// Нисхідний тон — зупинили запис.
pub fn play_stop() {
    beep(523, 120);
}

#[cfg(windows)]
fn beep(freq: u32, dur_ms: u32) {
    std::thread::spawn(move || {
        use windows::Win32::System::Diagnostics::Debug::Beep;
        unsafe {
            let _ = Beep(freq, dur_ms);
        }
    });
}

#[cfg(not(windows))]
fn beep(_freq: u32, _dur_ms: u32) {}
