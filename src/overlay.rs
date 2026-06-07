//! Екранний індикатор: кругле «футуристичне» коло по центру екрана, що
//! плавно змінює колір залежно від стану (запис / розпізнавання).
//!
//! Реалізовано як прозоре layered-вікно (per-pixel alpha) поверх усіх вікон,
//! click-through (не перехоплює кліки) і без активації. Малюється радіальним
//! градієнтом у DIB та виводиться через `UpdateLayeredWindow`. Анімація —
//! по таймеру в окремому потоці з власним циклом повідомлень. Лише Windows.

#![cfg(windows)]

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU8, Ordering};
use std::sync::{mpsc, Mutex};
use std::time::Instant;

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, POINT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, ReleaseDC, SelectObject,
    AC_SRC_ALPHA, AC_SRC_OVER, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, BLENDFUNCTION, DIB_RGB_COLORS,
    HBITMAP, HGDIOBJ,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, GetSystemMetrics, KillTimer,
    PostMessageW, PostQuitMessage, RegisterClassW, SetTimer, ShowWindow, TranslateMessage,
    UpdateLayeredWindow, CW_USEDEFAULT, MSG, SM_CXSCREEN, SM_CYSCREEN, SW_HIDE, SW_SHOWNOACTIVATE,
    ULW_ALPHA, WM_APP, WM_DESTROY, WM_TIMER, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};

/// Розмір вікна-індикатора (px). Коло вписане в цей квадрат.
const SIZE_PX: i32 = 220;
/// Ідентифікатор таймера анімації.
const TIMER_ID: usize = 1;
/// Інтервал кадру (мс) ≈ 30 fps.
const FRAME_MS: u32 = 33;
/// Крок зміни прозорості за кадр (з 1000). 150 ≈ ~230 мс на повний fade.
const FADE_STEP: u32 = 150;

// Команди стану — передаються у віконну процедуру через WM_APP (wparam).
const ST_HIDE: u8 = 0;
const ST_RECORDING: u8 = 1;
const ST_TRANSCRIBING: u8 = 2;

// Поточний стан і фаза анімації. Оскільки індикатор єдиний — тримаємо в
// статиках, щоб віконна процедура не залежала від userdata-вказівників.
static STATE: AtomicU8 = AtomicU8::new(ST_HIDE);
static PHASE: AtomicU32 = AtomicU32::new(0);
/// Бажана видимість: ST_HIDE = згасаємо до 0, інакше = проявляємось до 1000.
static TARGET: AtomicU8 = AtomicU8::new(ST_HIDE);
/// Поточна загальна прозорість 0..1000 (для fade-in/out).
static FADE: AtomicU32 = AtomicU32::new(0);
/// Чи крутиться таймер анімації (щоб не запускати його двічі).
static RUNNING: AtomicBool = AtomicBool::new(false);
/// Момент старту запису — для відліку реального часу запису в центрі кола.
/// `Some` лише поки триває запис (не під час розпізнавання).
static REC_START: Mutex<Option<Instant>> = Mutex::new(None);

/// Хендл для керування індикатором із головного потоку.
pub struct Overlay {
    /// HWND як ціле (HWND не Send; PostMessageW потокобезпечний).
    hwnd: Option<isize>,
}

impl Overlay {
    /// Запускає потік індикатора й повертає хендл. За помилки — «порожній»
    /// хендл, виклики якого нічого не роблять (індикатор просто не з'явиться).
    pub fn spawn() -> Self {
        let (tx, rx) = mpsc::channel::<isize>();
        std::thread::spawn(move || unsafe { run(tx) });
        Overlay {
            hwnd: rx.recv().ok(),
        }
    }

    fn post(&self, code: u8) {
        if let Some(h) = self.hwnd {
            unsafe {
                let _ = PostMessageW(HWND(h as *mut _), WM_APP, WPARAM(code as usize), LPARAM(0));
            }
        }
    }

    /// Показати індикатор у стані «запис».
    pub fn recording(&self) {
        self.post(ST_RECORDING);
    }
    /// Показати індикатор у стані «розпізнавання».
    pub fn transcribing(&self) {
        self.post(ST_TRANSCRIBING);
    }
    /// Сховати індикатор.
    pub fn hide(&self) {
        self.post(ST_HIDE);
    }
}

/// Тіло потоку: реєструє клас, створює вікно, віддає HWND і крутить цикл подій.
unsafe fn run(tx: mpsc::Sender<isize>) {
    let hinstance: HINSTANCE = match GetModuleHandleW(None) {
        Ok(h) => h.into(),
        Err(_) => return,
    };
    let class_name = w!("WhisperUkOverlay");

    let wc = WNDCLASSW {
        lpfnWndProc: Some(wndproc),
        hInstance: hinstance,
        lpszClassName: class_name,
        ..Default::default()
    };
    RegisterClassW(&wc);

    let hwnd = match CreateWindowExW(
        WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
        class_name,
        PCWSTR::null(),
        WS_POPUP,
        CW_USEDEFAULT,
        CW_USEDEFAULT,
        SIZE_PX,
        SIZE_PX,
        None,
        None,
        hinstance,
        None,
    ) {
        Ok(h) => h,
        Err(_) => return,
    };

    let _ = tx.send(hwnd.0 as isize);

    let mut msg = MSG::default();
    while GetMessageW(&mut msg, None, 0, 0).as_bool() {
        let _ = TranslateMessage(&msg);
        DispatchMessageW(&msg);
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_APP => {
            let code = wparam.0 as u8;
            TARGET.store(code, Ordering::Relaxed);
            if code != ST_HIDE {
                // Новий активний стан: задаємо колір і показуємо вікно (fade-in
                // далі веде таймер). PHASE не скидаємо при зміні стану на льоту.
                if FADE.load(Ordering::Relaxed) == 0 {
                    PHASE.store(0, Ordering::Relaxed);
                }
                STATE.store(code, Ordering::Relaxed);
                // Таймер відліку йде лише під час запису; на розпізнаванні — стоп.
                if let Ok(mut g) = REC_START.lock() {
                    *g = if code == ST_RECORDING {
                        Some(Instant::now())
                    } else {
                        None
                    };
                }
                redraw(hwnd);
                let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            }
            // У будь-якому разі вмикаємо таймер: і для fade-in, і для fade-out.
            if !RUNNING.swap(true, Ordering::Relaxed) {
                SetTimer(hwnd, TIMER_ID, FRAME_MS, None);
            }
            LRESULT(0)
        }
        WM_TIMER => {
            PHASE.fetch_add(1, Ordering::Relaxed);

            // Рухаємо прозорість до цілі (1000 = видимо, 0 = сховано).
            let goal: u32 = if TARGET.load(Ordering::Relaxed) == ST_HIDE {
                0
            } else {
                1000
            };
            let cur = FADE.load(Ordering::Relaxed);
            let next = if cur < goal {
                (cur + FADE_STEP).min(goal)
            } else {
                cur.saturating_sub(FADE_STEP).max(goal)
            };
            FADE.store(next, Ordering::Relaxed);

            if next == 0 && goal == 0 {
                // Згасли повністю — зупиняємо таймер і ховаємо вікно.
                let _ = KillTimer(hwnd, TIMER_ID);
                RUNNING.store(false, Ordering::Relaxed);
                let _ = ShowWindow(hwnd, SW_HIDE);
            } else {
                redraw(hwnd);
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

/// Перемальовує коло й виводить його через UpdateLayeredWindow.
unsafe fn redraw(hwnd: HWND) {
    let state = STATE.load(Ordering::Relaxed);
    if state == ST_HIDE {
        return;
    }
    let phase = PHASE.load(Ordering::Relaxed) as f32;

    let hdc_screen = GetDC(None);
    let hdc_mem = CreateCompatibleDC(hdc_screen);

    // Top-down 32-bit DIB (від'ємна висота = зверху вниз).
    let bi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: SIZE_PX,
            biHeight: -SIZE_PX,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
    let hbmp: HBITMAP = match CreateDIBSection(hdc_mem, &bi, DIB_RGB_COLORS, &mut bits, None, 0) {
        Ok(h) => h,
        Err(_) => {
            let _ = DeleteDC(hdc_mem);
            ReleaseDC(None, hdc_screen);
            return;
        }
    };
    let old = SelectObject(hdc_mem, HGDIOBJ(hbmp.0));

    let fade = FADE.load(Ordering::Relaxed) as f32 / 1000.0;
    // Скільки секунд триває запис (-1 = таймер не показуємо).
    let secs: i32 = if state == ST_RECORDING {
        REC_START
            .lock()
            .ok()
            .and_then(|g| *g)
            .map(|t| t.elapsed().as_secs() as i32)
            .unwrap_or(-1)
    } else {
        -1
    };
    let px = std::slice::from_raw_parts_mut(bits as *mut u32, (SIZE_PX * SIZE_PX) as usize);
    draw(px, state, phase, fade, secs);

    // Центр первинного монітора.
    let sw = GetSystemMetrics(SM_CXSCREEN);
    let sh = GetSystemMetrics(SM_CYSCREEN);
    let dst = POINT {
        x: (sw - SIZE_PX) / 2,
        y: (sh - SIZE_PX) / 2,
    };
    let size = SIZE {
        cx: SIZE_PX,
        cy: SIZE_PX,
    };
    let src = POINT { x: 0, y: 0 };
    let blend = BLENDFUNCTION {
        BlendOp: AC_SRC_OVER as u8,
        BlendFlags: 0,
        SourceConstantAlpha: 255,
        AlphaFormat: AC_SRC_ALPHA as u8,
    };
    let _ = UpdateLayeredWindow(
        hwnd,
        hdc_screen,
        Some(&dst),
        Some(&size),
        hdc_mem,
        Some(&src),
        COLORREF(0),
        Some(&blend),
        ULW_ALPHA,
    );

    SelectObject(hdc_mem, old);
    let _ = DeleteObject(HGDIOBJ(hbmp.0));
    let _ = DeleteDC(hdc_mem);
    ReleaseDC(None, hdc_screen);
}

/// Малює радіальне кільце з пульсацією у буфер премультиплікованого BGRA.
fn draw(px: &mut [u32], state: u8, phase: f32, fade: f32, secs: i32) {
    let s = SIZE_PX as f32;
    let c = s / 2.0;
    let radius = c - 6.0;

    // Базовий колір за станом.
    let (br, bg, bb) = match state {
        ST_RECORDING => (255.0_f32, 70.0, 90.0), // червоно-рожевий
        ST_TRANSCRIBING => (255.0, 190.0, 60.0), // бурштиновий
        _ => (120.0, 200.0, 255.0),              // блакитний (запас)
    };

    // М'яка пульсація яскравості/альфи.
    let pulse = 0.5 + 0.5 * (phase * 0.18).sin();
    // Кут «голови» для спінера у стані розпізнавання.
    let head = phase * 0.14;

    for y in 0..SIZE_PX {
        for x in 0..SIZE_PX {
            let dx = x as f32 + 0.5 - c;
            let dy = y as f32 + 0.5 - c;
            let dist = (dx * dx + dy * dy).sqrt() / radius;

            // Кільце: пік альфи біля dist≈0.8, м'які краї.
            let ring = (1.0 - ((dist - 0.8).abs() / 0.16)).clamp(0.0, 1.0);
            // Внутрішнє світіння до центру.
            let glow = (1.0 - dist).clamp(0.0, 1.0).powf(2.5) * 0.30;
            let mut alpha = ring.powf(1.4).max(glow);

            // Яскравий рухомий сегмент під час розпізнавання (ефект спінера).
            let mut bright = 0.55 + 0.45 * pulse;
            if state == ST_TRANSCRIBING {
                let ang = dy.atan2(dx);
                let mut d = (ang - head).abs() % (2.0 * std::f32::consts::PI);
                if d > std::f32::consts::PI {
                    d = 2.0 * std::f32::consts::PI - d;
                }
                let arc = (1.0 - d / 1.1).clamp(0.0, 1.0);
                alpha = alpha.max(ring * arc);
                bright += arc * 0.6;
            } else {
                alpha *= 0.6 + 0.4 * pulse;
            }

            if dist > 1.05 {
                alpha = 0.0;
            }
            // Загальна прозорість fade-in/out.
            alpha = (alpha * fade).clamp(0.0, 1.0);
            bright = bright.clamp(0.0, 1.2);

            // Колір із підсвіткою (тягнемо до білого на яскравих ділянках).
            let white = (bright - 1.0).clamp(0.0, 1.0);
            let r = (br * bright.min(1.0) + (255.0 - br) * white).min(255.0);
            let g = (bg * bright.min(1.0) + (255.0 - bg) * white).min(255.0);
            let b = (bb * bright.min(1.0) + (255.0 - bb) * white).min(255.0);

            // Премультиплікація на альфу (вимога AC_SRC_ALPHA).
            let a8 = (alpha * 255.0) as u32;
            let r8 = (r * alpha) as u32;
            let g8 = (g * alpha) as u32;
            let b8 = (b * alpha) as u32;
            px[(y * SIZE_PX + x) as usize] = (a8 << 24) | (r8 << 16) | (g8 << 8) | b8;
        }
    }

    // Таймер M:SS у центрі (лише під час реального запису).
    if secs >= 0 {
        draw_timer(px, secs, fade * (0.7 + 0.3 * pulse));
    }
}

/// Малює час M:SS семисегментними цифрами по центру кола.
fn draw_timer(px: &mut [u32], secs: i32, alpha: f32) {
    let m = (secs / 60).min(9); // хвилини 0..9 (довше навряд чи)
    let ss = secs % 60;
    let glyphs = [m, -1 /* двокрапка */, ss / 10, ss % 10];

    // Геометрія цифр.
    let dw = 22_i32;
    let dh = 42_i32;
    let t = 5_i32;
    let gap = 8_i32;
    let colon_w = 10_i32;

    // Повна ширина рядка для центрування.
    let mut total = 0;
    for g in glyphs {
        total += if g == -1 { colon_w } else { dw } + gap;
    }
    total -= gap;

    let mut x = SIZE_PX / 2 - total / 2;
    let y0 = SIZE_PX / 2 - dh / 2;
    // Світлий бірюзовий колір для контрасту з кільцем.
    let color = (210u32, 245, 255);

    for g in glyphs {
        if g == -1 {
            // Двокрапка: дві крапки.
            fill_rect(px, x + colon_w / 2 - t / 2, y0 + dh / 3, t, t, color, alpha);
            fill_rect(
                px,
                x + colon_w / 2 - t / 2,
                y0 + 2 * dh / 3,
                t,
                t,
                color,
                alpha,
            );
            x += colon_w + gap;
        } else {
            draw_digit(px, x, y0, dw, dh, t, g as usize, color, alpha);
            x += dw + gap;
        }
    }
}

/// Малює одну семисегментну цифру в прямокутнику (x0,y0,dw,dh).
#[allow(clippy::too_many_arguments)]
fn draw_digit(
    px: &mut [u32],
    x0: i32,
    y0: i32,
    dw: i32,
    dh: i32,
    t: i32,
    digit: usize,
    color: (u32, u32, u32),
    alpha: f32,
) {
    // Маски сегментів [a,b,c,d,e,f,g] для цифр 0..9.
    const SEG: [[bool; 7]; 10] = [
        [true, true, true, true, true, true, false],     // 0
        [false, true, true, false, false, false, false], // 1
        [true, true, false, true, true, false, true],    // 2
        [true, true, true, true, false, false, true],    // 3
        [false, true, true, false, false, true, true],   // 4
        [true, false, true, true, false, true, true],    // 5
        [true, false, true, true, true, true, true],     // 6
        [true, true, true, false, false, false, false],  // 7
        [true, true, true, true, true, true, true],      // 8
        [true, true, true, true, false, true, true],     // 9
    ];
    let s = SEG[digit % 10];
    let mid = y0 + dh / 2;
    // a — верх
    if s[0] {
        fill_rect(px, x0 + t, y0, dw - 2 * t, t, color, alpha);
    }
    // b — правий верхній
    if s[1] {
        fill_rect(px, x0 + dw - t, y0 + t, t, mid - (y0 + t), color, alpha);
    }
    // c — правий нижній
    if s[2] {
        fill_rect(px, x0 + dw - t, mid, t, (y0 + dh - t) - mid, color, alpha);
    }
    // d — низ
    if s[3] {
        fill_rect(px, x0 + t, y0 + dh - t, dw - 2 * t, t, color, alpha);
    }
    // e — лівий нижній
    if s[4] {
        fill_rect(px, x0, mid, t, (y0 + dh - t) - mid, color, alpha);
    }
    // f — лівий верхній
    if s[5] {
        fill_rect(px, x0, y0 + t, t, mid - (y0 + t), color, alpha);
    }
    // g — середина
    if s[6] {
        fill_rect(px, x0 + t, mid - t / 2, dw - 2 * t, t, color, alpha);
    }
}

/// Заливає прямокутник кольором із заданою альфою (премультиплікація BGRA).
fn fill_rect(px: &mut [u32], x: i32, y: i32, w: i32, h: i32, color: (u32, u32, u32), alpha: f32) {
    let a = alpha.clamp(0.0, 1.0);
    let (r, g, b) = color;
    let a8 = (a * 255.0) as u32;
    let r8 = (r as f32 * a) as u32;
    let g8 = (g as f32 * a) as u32;
    let b8 = (b as f32 * a) as u32;
    let packed = (a8 << 24) | (r8 << 16) | (g8 << 8) | b8;
    for yy in y..(y + h) {
        if yy < 0 || yy >= SIZE_PX {
            continue;
        }
        for xx in x..(x + w) {
            if xx < 0 || xx >= SIZE_PX {
                continue;
            }
            px[(yy * SIZE_PX + xx) as usize] = packed;
        }
    }
}
