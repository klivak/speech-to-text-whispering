//! whisper-uk — мінімальний голосовий ввід українською.
//!
//! Тиснеш глобальний хоткей → говориш → текст транскрибується через Groq
//! (`whisper-large-v3`) і вставляється в курсор. Усе живе в треї, без вікон.
//!
//! Архітектура (один процес, один UI-потік):
//!   • tao EventLoop — головний потік, тримає трей і реагує на події;
//!   • два фонові потоки лише *пересилають* події хоткея/меню в цикл;
//!   • транскрипція виконується в окремому потоці, щоб не блокувати UI,
//!     а результат повертається назад через EventLoopProxy.

// Ховаємо консольне вікно у release-збірці (у debug лишаємо для логів).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(windows)]
mod autostart;
mod audio;
mod config;
mod groq;
mod hotkey;
mod paste;

use audio::Recording;
use config::{Config, HotkeyMode};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tray_icon::{
    menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem},
    Icon, TrayIconBuilder,
};

/// Стан застосунку (визначає колір іконки трея).
#[derive(Clone, Copy, PartialEq)]
enum State {
    Idle,
    Recording,
    Transcribing,
    Error,
}

/// Події, що приходять у головний цикл.
enum UserEvent {
    Hotkey(GlobalHotKeyEvent),
    Menu(tray_icon::menu::MenuId),
    TranscribeDone(Result<String, String>),
}

fn main() {
    let mut cfg = Config::load_or_create();

    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let proxy = event_loop.create_proxy();

    // --- Реєстрація глобального хоткея ---
    let hk_manager = GlobalHotKeyManager::new().expect("GlobalHotKeyManager");
    let mut current_hotkey =
        hotkey::parse(&cfg.hotkey).unwrap_or_else(|e| panic!("Хибний хоткей: {e}"));
    hk_manager.register(current_hotkey).expect("register hotkey");

    // --- Меню трея ---
    let menu = Menu::new();
    let mode_toggle =
        CheckMenuItem::new("Режим: Toggle", true, cfg.mode == HotkeyMode::Toggle, None);
    let mode_ptt = CheckMenuItem::new(
        "Режим: Push-to-talk",
        true,
        cfg.mode == HotkeyMode::PushToTalk,
        None,
    );
    #[cfg(windows)]
    let autostart_item = CheckMenuItem::new(
        "Запускати з Windows",
        true,
        autostart::is_enabled(),
        None,
    );
    let get_key = MenuItem::new("Як отримати ключ Groq…", true, None);
    let open_cfg = MenuItem::new("Відкрити config.json", true, None);
    let reload_cfg = MenuItem::new("Перезавантажити конфіг", true, None);
    let quit = MenuItem::new("Вийти", true, None);
    menu.append_items(&[
        &mode_toggle,
        &mode_ptt,
        &PredefinedMenuItem::separator(),
        &get_key,
        &open_cfg,
        &reload_cfg,
    ])
    .expect("build menu");
    #[cfg(windows)]
    menu.append(&autostart_item).expect("autostart item");
    menu.append_items(&[
        &PredefinedMenuItem::separator(),
        &quit,
    ])
    .expect("build menu");

    let icons = Icons::new();
    let tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip(tooltip(State::Idle, &cfg))
        .with_icon(icons.idle.clone())
        .build()
        .expect("tray");

    // --- Фонові потоки: пересилають події в цикл (без активного опитування) ---
    {
        let proxy = proxy.clone();
        std::thread::spawn(move || {
            let rx = GlobalHotKeyEvent::receiver();
            while let Ok(ev) = rx.recv() {
                if proxy.send_event(UserEvent::Hotkey(ev)).is_err() {
                    break;
                }
            }
        });
    }
    {
        let proxy = proxy.clone();
        std::thread::spawn(move || {
            let rx = MenuEvent::receiver();
            while let Ok(ev) = rx.recv() {
                if proxy.send_event(UserEvent::Menu(ev.id)).is_err() {
                    break;
                }
            }
        });
    }

    // Тримаємо tray живим увесь час роботи циклу (дроп ховає іконку).
    let keep_tray = tray;

    let mut state = State::Idle;
    let mut recording: Option<Recording> = None;

    let set_state = |tray: &tray_icon::TrayIcon, icons: &Icons, st: State, cfg: &Config| {
        let icon = match st {
            State::Idle => &icons.idle,
            State::Recording => &icons.recording,
            State::Transcribing => &icons.transcribing,
            State::Error => &icons.error,
        };
        let _ = tray.set_icon(Some(icon.clone()));
        let _ = tray.set_tooltip(Some(tooltip(st, cfg)));
    };

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;

        if let tao::event::Event::UserEvent(ev) = event {
            match ev {
                UserEvent::Hotkey(hk) => {
                    let should_start = hk.state == HotKeyState::Pressed && recording.is_none();
                    let should_stop = match cfg.mode {
                        HotkeyMode::Toggle => {
                            hk.state == HotKeyState::Pressed && recording.is_some()
                        }
                        HotkeyMode::PushToTalk => {
                            hk.state == HotKeyState::Released && recording.is_some()
                        }
                    };

                    if should_stop {
                        if let Some(rec) = recording.take() {
                            state = State::Transcribing;
                            set_state(&keep_tray, &icons, state, &cfg);
                            match rec.stop_to_wav() {
                                Ok(wav) => {
                                    let cfg2 = cfg.clone();
                                    let proxy = proxy.clone();
                                    std::thread::spawn(move || {
                                        let res = groq::transcribe(&cfg2, wav).and_then(|text| {
                                            if text.is_empty() {
                                                return Ok(text);
                                            }
                                            paste::copy_to_clipboard(&text)?;
                                            if cfg2.auto_paste {
                                                paste::paste_at_cursor()?;
                                            }
                                            Ok(text)
                                        });
                                        let _ =
                                            proxy.send_event(UserEvent::TranscribeDone(res));
                                    });
                                }
                                Err(e) => {
                                    eprintln!("Кодування WAV: {e}");
                                    state = State::Error;
                                    set_state(&keep_tray, &icons, state, &cfg);
                                }
                            }
                        }
                    } else if should_start {
                        match Recording::start() {
                            Ok(rec) => {
                                recording = Some(rec);
                                state = State::Recording;
                                set_state(&keep_tray, &icons, state, &cfg);
                            }
                            Err(e) => {
                                eprintln!("Старт запису: {e}");
                                state = State::Error;
                                set_state(&keep_tray, &icons, state, &cfg);
                            }
                        }
                    }
                }

                UserEvent::TranscribeDone(res) => {
                    match res {
                        Ok(text) => {
                            eprintln!("OK: {text}");
                            state = State::Idle;
                        }
                        Err(e) => {
                            eprintln!("Транскрипція: {e}");
                            state = State::Error;
                        }
                    }
                    set_state(&keep_tray, &icons, state, &cfg);
                }

                UserEvent::Menu(id) => {
                    #[cfg(windows)]
                    if id == *autostart_item.id() {
                        match autostart::toggle() {
                            Ok(enabled) => autostart_item.set_checked(enabled),
                            Err(e) => {
                                eprintln!("Автозапуск: {e}");
                                autostart_item.set_checked(autostart::is_enabled());
                            }
                        }
                    }

                    if id == *quit.id() {
                        *control_flow = ControlFlow::Exit;
                    } else if id == *mode_toggle.id() {
                        cfg.mode = HotkeyMode::Toggle;
                        mode_toggle.set_checked(true);
                        mode_ptt.set_checked(false);
                        cfg.save();
                        set_state(&keep_tray, &icons, state, &cfg);
                    } else if id == *mode_ptt.id() {
                        cfg.mode = HotkeyMode::PushToTalk;
                        mode_ptt.set_checked(true);
                        mode_toggle.set_checked(false);
                        cfg.save();
                        set_state(&keep_tray, &icons, state, &cfg);
                    } else if id == *get_key.id() {
                        open_url("https://console.groq.com/keys");
                    } else if id == *open_cfg.id() {
                        open_config_file();
                    } else if id == *reload_cfg.id() {
                        let new_cfg = Config::load_or_create();
                        // Перереєстровуємо хоткей, якщо змінився.
                        if let Ok(new_hk) = hotkey::parse(&new_cfg.hotkey) {
                            if new_hk != current_hotkey {
                                let _ = hk_manager.unregister(current_hotkey);
                                if hk_manager.register(new_hk).is_ok() {
                                    current_hotkey = new_hk;
                                }
                            }
                        }
                        cfg = new_cfg;
                        mode_toggle.set_checked(cfg.mode == HotkeyMode::Toggle);
                        mode_ptt.set_checked(cfg.mode == HotkeyMode::PushToTalk);
                        state = State::Idle;
                        set_state(&keep_tray, &icons, state, &cfg);
                    }
                }
            }
        }
    });
}

/// Підказка (tooltip) для іконки трея.
fn tooltip(state: State, cfg: &Config) -> String {
    let st = match state {
        State::Idle => "очікує",
        State::Recording => "🔴 запис…",
        State::Transcribing => "⏳ розпізнавання…",
        State::Error => "⚠ помилка (див. config / ключ)",
    };
    let mode = match cfg.mode {
        HotkeyMode::Toggle => "toggle",
        HotkeyMode::PushToTalk => "push-to-talk",
    };
    format!("whisper-uk — {st}\nХоткей: {} ({mode})", cfg.hotkey)
}

/// Відкриває config.json у редакторі за замовчуванням.
fn open_config_file() {
    let path = Config::path();
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("cmd")
            .args(["/C", "start", "", &path.to_string_lossy()])
            .spawn();
    }
    #[cfg(not(windows))]
    {
        let _ = std::process::Command::new("xdg-open").arg(&path).spawn();
    }
}

/// Відкриває URL у браузері за замовчуванням.
fn open_url(url: &str) {
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("cmd")
            .args(["/C", "start", "", url])
            .spawn();
    }
    #[cfg(not(windows))]
    {
        let _ = std::process::Command::new("xdg-open").arg(url).spawn();
    }
}

/// Набір однотонних іконок трея для різних станів.
struct Icons {
    idle: Icon,
    recording: Icon,
    transcribing: Icon,
    error: Icon,
}

impl Icons {
    fn new() -> Self {
        Self {
            idle: solid_icon(120, 120, 120),         // сірий — очікує
            recording: solid_icon(220, 40, 40),      // червоний — запис
            transcribing: solid_icon(230, 180, 30),  // жовтий — розпізнавання
            error: solid_icon(150, 30, 160),         // фіолетовий — помилка
        }
    }
}

/// Генерує іконку 32×32 суцільного кольору (без зовнішніх файлів).
fn solid_icon(r: u8, g: u8, b: u8) -> Icon {
    const SIZE: u32 = 32;
    let mut rgba = Vec::with_capacity((SIZE * SIZE * 4) as usize);
    for _ in 0..(SIZE * SIZE) {
        rgba.extend_from_slice(&[r, g, b, 255]);
    }
    Icon::from_rgba(rgba, SIZE, SIZE).expect("icon")
}
