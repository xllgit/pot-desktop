// Mouse selection translate, works like Easydict:
// select text (drag / double click / triple click / shift+click) -> show icon -> hover icon to translate
use crate::config::get;
use crate::window::text_translate;
use crate::APP;
use device_query::{DeviceQuery, DeviceState, Keycode};
use log::{info, warn};
use once_cell::sync::Lazy;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};
use tauri::{Manager, Window, WindowBuilder};

const ICON_LABEL: &str = "selection_icon";
// Icon size (logical), the icon image fills the whole window
const ICON_SIZE: f64 = 26.0;
// Distance (logical) between mouse and icon
const ICON_OFFSET: f64 = 12.0;
// Icon is dismissed when mouse moves out of this radius (logical) around the icon
const ICON_EXPANDED_RADIUS: f64 = 120.0;
// Minimum mouse movement while pressed to be considered a drag selection
const DRAG_THRESHOLD: i32 = 5;
// Max interval between clicks to be considered a multi click
const MULTI_CLICK_INTERVAL: Duration = Duration::from_millis(500);
const DRAG_DELAY: Duration = Duration::from_millis(50);
const DOUBLE_CLICK_DELAY: Duration = Duration::from_millis(200);
const TRIPLE_CLICK_DELAY: Duration = Duration::from_millis(100);
const SHIFT_CLICK_DELAY: Duration = Duration::from_millis(100);
const POLL_INTERVAL: Duration = Duration::from_millis(15);
const DISABLED_POLL_INTERVAL: Duration = Duration::from_millis(500);

// Icon area in mouse coordinates
#[derive(Clone, Copy)]
struct IconArea {
    center: (i32, i32),
    half_size: i32,
    radius: i32,
}

impl IconArea {
    fn contains(&self, (x, y): (i32, i32)) -> bool {
        (x - self.center.0).abs() <= self.half_size && (y - self.center.1).abs() <= self.half_size
    }
    fn in_expanded_area(&self, (x, y): (i32, i32)) -> bool {
        let dx = (x - self.center.0) as i64;
        let dy = (y - self.center.1) as i64;
        dx * dx + dy * dy <= (self.radius as i64) * (self.radius as i64)
    }
}

// Text selected when the icon was shown
static SELECTED_TEXT: Lazy<Mutex<String>> = Lazy::new(|| Mutex::new(String::new()));
// Some while the icon is visible
static ICON_AREA: Lazy<Mutex<Option<IconArea>>> = Lazy::new(|| Mutex::new(None));
// Increased on every new trigger, used to cancel pending selection reads (e.g. double -> triple click)
static SELECTION_GENERATION: AtomicU64 = AtomicU64::new(0);

fn is_enabled() -> bool {
    match get("selection_icon") {
        Some(v) => v.as_bool().unwrap_or(false),
        None => false,
    }
}

fn get_icon_window() -> Window {
    let app_handle = APP.get().unwrap();
    match app_handle.get_window(ICON_LABEL) {
        Some(v) => v,
        None => {
            info!("Creating selection icon window");
            WindowBuilder::new(
                app_handle,
                ICON_LABEL,
                tauri::WindowUrl::App("index.html".into()),
            )
            .title("Selection Icon")
            .additional_browser_args("--disable-web-security")
            .inner_size(ICON_SIZE, ICON_SIZE)
            .resizable(false)
            .decorations(false)
            .transparent(true)
            .always_on_top(true)
            .skip_taskbar(true)
            .focused(false)
            .visible(false)
            .build()
            .unwrap()
        }
    }
}

fn hide_icon() {
    let mut area = ICON_AREA.lock().unwrap();
    if area.is_none() {
        return;
    }
    *area = None;
    if let Some(window) = APP.get().unwrap().get_window(ICON_LABEL) {
        hide_window(&window);
    }
}

// (x, y) is the mouse position reported by the OS:
// physical pixels on Windows/Linux, logical points on macOS
fn show_icon(x: i32, y: i32) {
    let window = get_icon_window();
    #[cfg(target_os = "macos")]
    let scale = 1.0;
    #[cfg(not(target_os = "macos"))]
    let scale = window.scale_factor().unwrap_or(1.0);

    let offset = (ICON_OFFSET * scale) as i32;
    #[cfg(target_os = "macos")]
    let _ = window.set_position(tauri::LogicalPosition::new(x + offset, y + offset));
    #[cfg(not(target_os = "macos"))]
    let _ = window.set_position(tauri::PhysicalPosition::new(x + offset, y + offset));
    show_without_focus(&window, (ICON_SIZE * scale) as i32);

    let half_size = (ICON_SIZE * scale / 2.0) as i32;
    *ICON_AREA.lock().unwrap() = Some(IconArea {
        center: (x + offset + half_size, y + offset + half_size),
        half_size,
        radius: (ICON_EXPANDED_RADIUS * scale) as i32,
    });
}

// Showing the icon must not steal focus from the application where text is selected
// `size` is in physical pixels. On Windows the size is set here because the window keeps
// caption styles and is enlarged to the system minimum width when created
#[cfg(target_os = "windows")]
fn show_without_focus(window: &Window, size: i32) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{
        SetWindowPos, ShowWindow, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SW_SHOWNOACTIVATE,
    };
    match window.hwnd() {
        Ok(hwnd) => unsafe {
            let hwnd = HWND(hwnd.0 as _);
            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            let _ = SetWindowPos(
                hwnd,
                HWND_TOPMOST,
                0,
                0,
                size,
                size,
                SWP_NOMOVE | SWP_NOACTIVATE,
            );
        },
        Err(_) => {
            let _ = window.show();
        }
    }
}

#[cfg(not(target_os = "windows"))]
fn show_without_focus(window: &Window, _size: i32) {
    let _ = window.show();
    let _ = window.set_always_on_top(true);
}

// The icon is shown with native calls on Windows, so tao still thinks it is hidden
// and window.hide() does nothing. Hide it natively as well
#[cfg(target_os = "windows")]
fn hide_window(window: &Window) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_HIDE};
    match window.hwnd() {
        Ok(hwnd) => unsafe {
            let _ = ShowWindow(HWND(hwnd.0 as _), SW_HIDE);
        },
        Err(_) => {
            let _ = window.hide();
        }
    }
}

#[cfg(not(target_os = "windows"))]
fn hide_window(window: &Window) {
    let _ = window.hide();
}

// Ignore selections made inside pot's own windows
fn is_pot_focused() -> bool {
    APP.get()
        .unwrap()
        .windows()
        .values()
        .any(|w| w.is_focused().unwrap_or(false))
}

// Read selected text after `delay` and show the icon at `coords`
fn trigger_selection(delay: Duration, coords: (i32, i32)) {
    let generation = SELECTION_GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    thread::spawn(move || {
        // Wait for the target application to finish updating the selection
        thread::sleep(delay);
        if SELECTION_GENERATION.load(Ordering::SeqCst) != generation || is_pot_focused() {
            return;
        }
        let text = selection::get_text();
        if text.trim().is_empty() || SELECTION_GENERATION.load(Ordering::SeqCst) != generation {
            info!("No selected text found");
            return;
        }
        info!("Selected text found, showing selection icon at {:?}", coords);
        *SELECTED_TEXT.lock().unwrap() = text;
        show_icon(coords.0, coords.1);
    });
}

fn is_modifier(key: &Keycode) -> bool {
    let name = format!("{:?}", key);
    ["Shift", "Control", "Alt", "Meta", "Command", "Option", "Fn", "CapsLock"]
        .iter()
        .any(|m| name.contains(m))
}

fn is_copy_shortcut(keys: &[Keycode]) -> bool {
    keys.contains(&Keycode::C) && keys.iter().any(|k| is_modifier(k))
}

pub fn start_selection_icon_monitor() {
    thread::spawn(|| {
        // DeviceState::new panics when no display is available (e.g. Wayland without XWayland)
        let device_state =
            match std::panic::catch_unwind(std::panic::AssertUnwindSafe(DeviceState::new)) {
                Ok(v) => v,
                Err(_) => {
                    warn!("Failed to init mouse monitor, selection icon disabled");
                    return;
                }
            };
        info!("Selection icon monitor started");
        let mut was_pressed = false;
        let mut press_coords = (0, 0);
        let mut press_on_icon = false;
        let mut click_count = 0;
        let mut last_click: Option<(Instant, (i32, i32))> = None;
        let mut prev_keys: Vec<Keycode> = Vec::new();
        loop {
            if !is_enabled() {
                hide_icon();
                was_pressed = false;
                last_click = None;
                thread::sleep(DISABLED_POLL_INTERVAL);
                continue;
            }
            let mouse = device_state.get_mouse();
            let keys = device_state.get_keys();
            let coords = mouse.coords;
            // button_pressed[1]: left, [2]: right, [3]: middle
            let left_pressed = mouse.button_pressed.get(1).copied().unwrap_or(false);
            let other_pressed = mouse.button_pressed.iter().skip(2).any(|v| *v);
            let icon_area = *ICON_AREA.lock().unwrap();

            // Dismiss icon
            if let Some(area) = icon_area {
                let new_key_pressed = keys
                    .iter()
                    .any(|k| !prev_keys.contains(k) && !is_modifier(k));
                if !area.in_expanded_area(coords)
                    || ((left_pressed && !was_pressed) || other_pressed) && !area.contains(coords)
                    || new_key_pressed && !is_copy_shortcut(&keys)
                {
                    hide_icon();
                }
            }

            // Detect selection
            if left_pressed && !was_pressed {
                press_coords = coords;
                press_on_icon = icon_area.map_or(false, |area| area.contains(coords));
            }
            if !left_pressed && was_pressed && !press_on_icon {
                let (x, y) = coords;
                let is_drag = (x - press_coords.0).abs() > DRAG_THRESHOLD
                    || (y - press_coords.1).abs() > DRAG_THRESHOLD;
                let now = Instant::now();
                if is_drag {
                    click_count = 0;
                    last_click = None;
                    trigger_selection(DRAG_DELAY, coords);
                } else {
                    let is_multi_click = match last_click {
                        Some((time, (lx, ly))) => {
                            now.duration_since(time) < MULTI_CLICK_INTERVAL
                                && (x - lx).abs() <= DRAG_THRESHOLD
                                && (y - ly).abs() <= DRAG_THRESHOLD
                        }
                        None => false,
                    };
                    click_count = if is_multi_click { click_count + 1 } else { 1 };
                    last_click = Some((now, (x, y)));
                    let shift_pressed =
                        keys.contains(&Keycode::LShift) || keys.contains(&Keycode::RShift);
                    match click_count {
                        2 => trigger_selection(DOUBLE_CLICK_DELAY, coords),
                        3 => trigger_selection(TRIPLE_CLICK_DELAY, coords),
                        1 if shift_pressed => trigger_selection(SHIFT_CLICK_DELAY, coords),
                        _ => {}
                    }
                }
            }
            was_pressed = left_pressed;
            prev_keys = keys;
            thread::sleep(POLL_INTERVAL);
        }
    });
}

// Called by the icon window when the mouse hovers over (or clicks) it
// Must be async, creating windows in sync commands deadlocks on Windows
#[tauri::command(async)]
pub fn selection_icon_translate() {
    info!("Selection icon hovered");
    hide_icon();
    let text = std::mem::take(&mut *SELECTED_TEXT.lock().unwrap());
    if !text.trim().is_empty() {
        text_translate(text);
    }
}
