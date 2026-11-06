//! 好友桌宠自动排列：把页面脚本注入官方 WebView，由脚本包装 index.render。
//! 页面脚本负责位置计算、编辑模式与 localStorage 持久化；本线程只做注入与开关同步。

use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU8, Ordering};
use std::time::{Duration, Instant};

use crate::features::cdp::Cdp;

#[link(name = "user32")]
extern "system" {
    fn GetAsyncKeyState(vkey: i32) -> i16;
}

const CORE: &str = include_str!("../../../web/injected/room-layout-core.js");
const SCRIPT: &str = include_str!("../../../web/injected/room-layout.js");

static ENABLED: AtomicBool = AtomicBool::new(false);
static EDIT: AtomicBool = AtomicBool::new(false);
static EDIT_HOTKEY: AtomicI32 = AtomicI32::new(0x74);
static SAVE_HOTKEY: AtomicI32 = AtomicI32::new(0x75);
static LAYOUT_MODE: AtomicU8 = AtomicU8::new(0);
static RESET_REQUESTED: AtomicBool = AtomicBool::new(false);

pub fn enabled() -> bool {
    ENABLED.load(Ordering::SeqCst)
}

pub fn edit() -> bool {
    EDIT.load(Ordering::SeqCst)
}

pub fn layout_mode() -> &'static str {
    if LAYOUT_MODE.load(Ordering::SeqCst) == 2 { "vertical" } else { "horizontal" }
}

pub fn set_layout_mode(mode: &str) {
    let value = if mode.trim().eq_ignore_ascii_case("vertical") { 2 } else { 1 };
    LAYOUT_MODE.store(value, Ordering::SeqCst);
}

pub fn request_reset() {
    RESET_REQUESTED.store(true, Ordering::SeqCst);
}

fn valid_key(vk: i32, fallback: i32) -> i32 {
    if vk > 0 && !matches!(vk, 0x76 | 0x77 | 0x78) { vk } else { fallback }
}

pub fn edit_hotkey() -> i32 {
    EDIT_HOTKEY.load(Ordering::SeqCst)
}

pub fn save_hotkey() -> i32 {
    SAVE_HOTKEY.load(Ordering::SeqCst)
}

pub fn set_edit_hotkey(vk: i32) -> i32 {
    let vk = valid_key(vk, 0x74);
    if vk == save_hotkey() {
        return edit_hotkey();
    }
    EDIT_HOTKEY.store(vk, Ordering::SeqCst);
    vk
}

pub fn set_save_hotkey(vk: i32) -> i32 {
    let vk = valid_key(vk, 0x75);
    if vk == edit_hotkey() {
        return save_hotkey();
    }
    SAVE_HOTKEY.store(vk, Ordering::SeqCst);
    vk
}

pub fn set_hotkeys(edit: i32, save: i32) {
    let edit = valid_key(edit, 0x74);
    let save = valid_key(save, 0x75);
    if edit == save {
        EDIT_HOTKEY.store(0x74, Ordering::SeqCst);
        SAVE_HOTKEY.store(0x75, Ordering::SeqCst);
    } else {
        EDIT_HOTKEY.store(edit, Ordering::SeqCst);
        SAVE_HOTKEY.store(save, Ordering::SeqCst);
    }
}

pub fn set_enabled(value: bool) {
    ENABLED.store(value, Ordering::SeqCst);
    if !value {
        EDIT.store(false, Ordering::SeqCst);
    }
}

fn eval(cdp: &mut Cdp, script: &str) {
    let _ = cdp.evaluate(script, false);
}

pub fn start_worker() {
    std::thread::spawn(|| {
        let mut last_disabled = false;
        let mut next_capture = Instant::now();
        loop {
            let enabled = ENABLED.load(Ordering::SeqCst);
            let edit = EDIT.load(Ordering::SeqCst);
            if let Ok(mut cdp) = Cdp::connect() {
                if enabled {
                    eval(&mut cdp, CORE);
                    eval(
                        &mut cdp,
                        if layout_mode() == "vertical" {
                            "window.__autotappoLayoutMode = 'vertical'"
                        } else {
                            "window.__autotappoLayoutMode = 'horizontal'"
                        },
                    );
                    eval(&mut cdp, SCRIPT);
                    eval(&mut cdp, "window.__autotappoArrange && window.__autotappoArrange.setEnabled(true)");
                    let has_room_api = cdp
                        .evaluate("!!window.__autotappoRoomApi", false)
                        .ok()
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    if !has_room_api {
                        let has_cats = cdp
                            .evaluate("!!document.querySelector('.room-cat-shell')", false)
                            .ok()
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false);
                        if has_cats && Instant::now() >= next_capture {
                            next_capture = Instant::now() + Duration::from_secs(5);
                            let _ = cdp.capture_room_api();
                        }
                    }
                    eval(
                        &mut cdp,
                        if layout_mode() == "vertical" {
                            "window.__autotappoArrange && window.__autotappoArrange.setMode('vertical')"
                        } else {
                            "window.__autotappoArrange && window.__autotappoArrange.setMode('horizontal')"
                        },
                    );
                    eval(
                        &mut cdp,
                        if edit {
                            "window.__autotappoArrange && window.__autotappoArrange.setEdit(true)"
                        } else {
                            "window.__autotappoArrange && window.__autotappoArrange.setEdit(false)"
                        },
                    );
                    if RESET_REQUESTED.swap(false, Ordering::SeqCst) {
                        eval(&mut cdp, "window.__autotappoArrange && window.__autotappoArrange.reset()");
                    }
                    last_disabled = false;
                } else if !last_disabled {
                    eval(&mut cdp, "window.__autotappoArrange && window.__autotappoArrange.setEnabled(false)");
                    last_disabled = true;
                }
            }
            std::thread::sleep(Duration::from_millis(1000));
        }
    });
    std::thread::spawn(hotkey_worker);
}

fn hotkey_worker() {
    let mut edit_down = false;
    let mut save_down = false;
    loop {
        let edit_pressed = unsafe { (GetAsyncKeyState(edit_hotkey()) as u16) & 0x8000 != 0 };
        let save_pressed = unsafe { (GetAsyncKeyState(save_hotkey()) as u16) & 0x8000 != 0 };
        if ENABLED.load(Ordering::SeqCst) {
            if edit_pressed && !edit_down {
                let next = !EDIT.load(Ordering::SeqCst);
                EDIT.store(next, Ordering::SeqCst);
            }
            if save_pressed && !save_down {
                EDIT.store(false, Ordering::SeqCst);
            }
        }
        edit_down = edit_pressed;
        save_down = save_pressed;
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disable_resets_edit() {
        set_enabled(true);
        assert!(ENABLED.load(Ordering::SeqCst));
        EDIT.store(true, Ordering::SeqCst);
        set_enabled(false);
        assert!(!ENABLED.load(Ordering::SeqCst));
        assert!(!EDIT.load(Ordering::SeqCst));
    }

    #[test]
    fn hotkeys_reject_reserved_and_duplicate_keys() {
        set_hotkeys(0x74, 0x75);
        assert_eq!(edit_hotkey(), 0x74);
        assert_eq!(save_hotkey(), 0x75);
        assert_eq!(set_edit_hotkey(0x77), 0x74);
        assert_eq!(set_edit_hotkey(0x75), 0x74);
        assert_eq!(set_save_hotkey(0x74), 0x75);
        set_hotkeys(0x74, 0x74);
        assert_eq!(edit_hotkey(), 0x74);
        assert_eq!(save_hotkey(), 0x75);
    }
}
