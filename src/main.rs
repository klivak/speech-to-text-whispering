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

mod audio;
#[cfg(windows)]
mod autostart;
mod config;
mod groq;
mod hotkey;
mod paste;
mod stats;

use audio::Recording;
use config::{Config, HotkeyMode};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use stats::Stats;
use std::path::Path;
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tray_icon::{
    menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu},
    Icon, TrayIconBuilder,
};

/// Готові варіанти хоткеїв для швидкого вибору в меню.
const HOTKEY_PRESETS: &[&str] = &[
    "Ctrl+Alt+Space",
    "Ctrl+Shift+Space",
    "Ctrl+Shift+D",
    "Alt+Backquote",
    "F9",
    "F8",
];

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
    TranscribeDone {
        result: Result<String, String>,
        audio_ms: u64,
        limits: groq::Limits,
    },
}

fn main() {
    let mut cfg = Config::load_or_create();
    let mut usage = Stats::load();

    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let proxy = event_loop.create_proxy();

    // --- Реєстрація глобального хоткея ---
    let hk_manager = GlobalHotKeyManager::new().expect("GlobalHotKeyManager");
    // Якщо хоткей із конфігу хибний — відкочуємось на дефолтний, не падаючи.
    let mut current_hotkey = hotkey::parse(&cfg.hotkey).unwrap_or_else(|e| {
        eprintln!("Хибний хоткей '{}' ({e}); беру Ctrl+Alt+Space", cfg.hotkey);
        cfg.hotkey = "Ctrl+Alt+Space".to_string();
        hotkey::parse(&cfg.hotkey).expect("дефолтний хоткей валідний")
    });
    // Реєстрація може не вдатись (хоткей зайнятий іншою програмою) — не панікуємо:
    // застосунок усе одно запуститься, користувач обере інший хоткей у меню.
    if hk_manager.register(current_hotkey).is_err() {
        eprintln!(
            "Хоткей {} зайнятий іншою програмою — обери інший у меню «Хоткей»",
            cfg.hotkey
        );
    }

    // --- Меню трея ---
    let menu = Menu::new();

    // Рядок-підказка: що саме натискати/тримати (оновлюється при зміні).
    let action_hint = MenuItem::new(action_hint_text(&cfg), false, None);

    // Підменю вибору хоткея з готовими комбінаціями.
    let hotkey_menu = Submenu::new("Хоткей", true);
    let hotkey_items: Vec<CheckMenuItem> = HOTKEY_PRESETS
        .iter()
        .map(|&spec| CheckMenuItem::new(spec, true, spec.eq_ignore_ascii_case(&cfg.hotkey), None))
        .collect();
    for it in &hotkey_items {
        hotkey_menu.append(it).expect("hotkey item");
    }

    let mode_toggle =
        CheckMenuItem::new("Режим: Toggle", true, cfg.mode == HotkeyMode::Toggle, None);
    let mode_ptt = CheckMenuItem::new(
        "Режим: Push-to-talk",
        true,
        cfg.mode == HotkeyMode::PushToTalk,
        None,
    );

    let stats_summary = MenuItem::new(usage.summary(), false, None);
    let limits_summary = MenuItem::new(usage.limits_summary(), false, None);
    let open_limits = MenuItem::new("Ліміти Groq (онлайн)…", true, None);
    let open_stats = MenuItem::new("Відкрити статистику", true, None);

    #[cfg(windows)]
    let autostart_item =
        CheckMenuItem::new("Запускати з Windows", true, autostart::is_enabled(), None);
    let get_key = MenuItem::new("Як отримати ключ Groq…", true, None);
    let open_cfg = MenuItem::new("Відкрити config.json", true, None);
    let reload_cfg = MenuItem::new("Перезавантажити конфіг", true, None);
    let quit = MenuItem::new("Вийти", true, None);

    menu.append_items(&[
        &action_hint,
        &PredefinedMenuItem::separator(),
        &hotkey_menu,
        &mode_toggle,
        &mode_ptt,
        &PredefinedMenuItem::separator(),
        &stats_summary,
        &limits_summary,
        &open_limits,
        &open_stats,
        &PredefinedMenuItem::separator(),
        &get_key,
        &open_cfg,
        &reload_cfg,
    ])
    .expect("build menu");
    #[cfg(windows)]
    menu.append(&autostart_item).expect("autostart item");
    menu.append_items(&[&PredefinedMenuItem::separator(), &quit])
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
                                Ok((wav, audio_ms)) => {
                                    let cfg2 = cfg.clone();
                                    let proxy = proxy.clone();
                                    std::thread::spawn(move || {
                                        let (result, limits) = match groq::transcribe(&cfg2, wav) {
                                            Ok((text, limits)) => {
                                                let res = if text.is_empty() {
                                                    Ok(text)
                                                } else {
                                                    paste::copy_to_clipboard(&text)
                                                        .and_then(|()| {
                                                            if cfg2.auto_paste {
                                                                paste::paste_at_cursor()?;
                                                            }
                                                            Ok(text)
                                                        })
                                                };
                                                (res, limits)
                                            }
                                            Err(e) => (Err(e), groq::Limits::default()),
                                        };
                                        let _ = proxy.send_event(UserEvent::TranscribeDone {
                                            result,
                                            audio_ms,
                                            limits,
                                        });
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

                UserEvent::TranscribeDone {
                    result,
                    audio_ms,
                    limits,
                } => {
                    match &result {
                        Ok(text) => {
                            eprintln!("OK: {text}");
                            state = State::Idle;
                        }
                        Err(e) => {
                            eprintln!("Транскрипція: {e}");
                            state = State::Error;
                        }
                    }
                    // Оновлюємо статистику й залишок лімітів.
                    usage.record(&result, audio_ms, &limits);
                    usage.save();
                    stats_summary.set_text(usage.summary());
                    limits_summary.set_text(usage.limits_summary());
                    set_state(&keep_tray, &icons, state, &cfg);
                }

                UserEvent::Menu(id) => {
                    let mut handled = false;

                    // 1) Вибір хоткея з підменю.
                    for (i, it) in hotkey_items.iter().enumerate() {
                        if id == *it.id() {
                            let spec = HOTKEY_PRESETS[i];
                            if let Ok(new_hk) = hotkey::parse(spec) {
                                let _ = hk_manager.unregister(current_hotkey);
                                if hk_manager.register(new_hk).is_ok() {
                                    current_hotkey = new_hk;
                                    cfg.hotkey = spec.to_string();
                                    cfg.save();
                                } else {
                                    // Відкат, якщо нова комбінація зайнята.
                                    let _ = hk_manager.register(current_hotkey);
                                }
                            }
                            sync_hotkey_checks(&hotkey_items, &cfg.hotkey);
                            action_hint.set_text(action_hint_text(&cfg));
                            set_state(&keep_tray, &icons, state, &cfg);
                            handled = true;
                            break;
                        }
                    }

                    // 2) Автозапуск (тільки Windows).
                    #[cfg(windows)]
                    if !handled && id == *autostart_item.id() {
                        match autostart::toggle() {
                            Ok(enabled) => autostart_item.set_checked(enabled),
                            Err(e) => {
                                eprintln!("Автозапуск: {e}");
                                autostart_item.set_checked(autostart::is_enabled());
                            }
                        }
                        handled = true;
                    }

                    if handled {
                        return;
                    }

                    if id == *quit.id() {
                        *control_flow = ControlFlow::Exit;
                    } else if id == *mode_toggle.id() {
                        cfg.mode = HotkeyMode::Toggle;
                        mode_toggle.set_checked(true);
                        mode_ptt.set_checked(false);
                        cfg.save();
                        action_hint.set_text(action_hint_text(&cfg));
                        set_state(&keep_tray, &icons, state, &cfg);
                    } else if id == *mode_ptt.id() {
                        cfg.mode = HotkeyMode::PushToTalk;
                        mode_ptt.set_checked(true);
                        mode_toggle.set_checked(false);
                        cfg.save();
                        action_hint.set_text(action_hint_text(&cfg));
                        set_state(&keep_tray, &icons, state, &cfg);
                    } else if id == *open_limits.id() {
                        open_url("https://console.groq.com/settings/limits");
                    } else if id == *open_stats.id() {
                        open_path(&Stats::path());
                    } else if id == *get_key.id() {
                        open_url("https://console.groq.com/keys");
                    } else if id == *open_cfg.id() {
                        open_path(&Config::path());
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
                        sync_hotkey_checks(&hotkey_items, &cfg.hotkey);
                        action_hint.set_text(action_hint_text(&cfg));
                        state = State::Idle;
                        set_state(&keep_tray, &icons, state, &cfg);
                    }
                }
            }
        }
    });
}

/// Рядок-підказка: дієслово (натисни/тримай) + поточний хоткей.
fn action_hint_text(cfg: &Config) -> String {
    let verb = match cfg.mode {
        HotkeyMode::Toggle => "Натисни",
        HotkeyMode::PushToTalk => "Тримай",
    };
    format!("▶ {verb}: {}", cfg.hotkey)
}

/// Ставить галочку лише на тому пресеті, що збігається з поточним хоткеєм.
fn sync_hotkey_checks(items: &[CheckMenuItem], current: &str) {
    for (it, &spec) in items.iter().zip(HOTKEY_PRESETS) {
        it.set_checked(spec.eq_ignore_ascii_case(current));
    }
}

/// Підказка (tooltip) для іконки трея.
fn tooltip(state: State, cfg: &Config) -> String {
    let st = match state {
        State::Idle => "очікує",
        State::Recording => "🔴 запис…",
        State::Transcribing => "⏳ розпізнавання…",
        State::Error => "⚠ помилка (див. config / ключ)",
    };
    format!("whisper-uk — {st}\n{}", action_hint_text(cfg))
}

/// Відкриває файл/шлях у застосунку за замовчуванням.
fn open_path(path: &Path) {
    open_target(&path.to_string_lossy());
}

/// Відкриває URL у браузері за замовчуванням.
fn open_url(url: &str) {
    open_target(url);
}

/// Відкриває файл або URL засобами ОС.
#[cfg(windows)]
fn open_target(target: &str) {
    use windows::core::{HSTRING, PCWSTR};
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    let op = HSTRING::from("open");
    let file = HSTRING::from(target);
    // ShellExecuteW сам обирає застосунок за замовчуванням; не запускає cmd.exe.
    unsafe {
        ShellExecuteW(
            None,
            &op,
            &file,
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        );
    }
}

/// Відкриває файл або URL засобами ОС.
#[cfg(not(windows))]
fn open_target(target: &str) {
    let _ = std::process::Command::new("xdg-open").arg(target).spawn();
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
            idle: solid_icon(120, 120, 120),        // сірий — очікує
            recording: solid_icon(220, 40, 40),     // червоний — запис
            transcribing: solid_icon(230, 180, 30), // жовтий — розпізнавання
            error: solid_icon(150, 30, 160),        // фіолетовий — помилка
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
