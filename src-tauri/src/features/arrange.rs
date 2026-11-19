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
static EDIT_HOTKEY: AtomicI32 = AtomicI32::new(0x75);
static LAYOUT_MANUAL: AtomicBool = AtomicBool::new(true);
static LAYOUT_MODE: AtomicU8 = AtomicU8::new(0);
static LAYOUT_WRAP: AtomicU8 = AtomicU8::new(0);
static LAYOUT_WRAP_DIR: AtomicU8 = AtomicU8::new(0);

pub fn edit() -> bool {
    EDIT.load(Ordering::SeqCst)
}

pub fn layout_manual() -> bool {
    LAYOUT_MANUAL.load(Ordering::SeqCst)
}

pub fn set_layout_manual(value: bool) {
    LAYOUT_MANUAL.store(value, Ordering::SeqCst);
    if !value {
        EDIT.store(false, Ordering::SeqCst);
    }
}

pub fn layout_mode() -> &'static str {
    match LAYOUT_MODE.load(Ordering::SeqCst) {
        2 => "h-left",
        3 => "v-up",
        4 => "v-down",
        5 => "overlap",
        _ => "h-right",
    }
}

pub fn set_layout_mode(mode: &str) {
    let value = match mode.trim().to_ascii_lowercase().as_str() {
        "h-left" => 2,
        "v-up" => 3,
        "v-down" => 4,
        "overlap" => 5,
        "vertical" => 3,
        _ => 1,
    };
    LAYOUT_MODE.store(value, Ordering::SeqCst);
    if value == 5 {
        EDIT.store(false, Ordering::SeqCst);
    }
}

pub fn layout_wrap() -> u8 {
    LAYOUT_WRAP.load(Ordering::SeqCst)
}

pub fn set_layout_wrap(wrap: u8) {
    let wrap = wrap.min(20);
    LAYOUT_WRAP.store(if wrap == 1 { 2 } else { wrap }, Ordering::SeqCst);
}

pub fn layout_wrap_dir() -> u8 {
    LAYOUT_WRAP_DIR.load(Ordering::SeqCst)
}

pub fn set_layout_wrap_dir(dir: u8) {
    LAYOUT_WRAP_DIR.store(if dir == 0 { 0 } else { 1 }, Ordering::SeqCst);
}

fn valid_key(vk: i32, fallback: i32) -> i32 {
    if vk > 0 && !matches!(vk, 0x76..=0x78) { vk } else { fallback }
}

pub fn edit_hotkey() -> i32 {
    EDIT_HOTKEY.load(Ordering::SeqCst)
}

pub fn set_edit_hotkey(vk: i32) -> i32 {
    let vk = valid_key(vk, 0x75);
    EDIT_HOTKEY.store(vk, Ordering::SeqCst);
    vk
}

pub fn set_hotkeys(edit: i32) {
    EDIT_HOTKEY.store(valid_key(edit, 0x75), Ordering::SeqCst);
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
                        match layout_mode() {
                            "h-left" => "window.__autotappoLayoutMode = 'h-left'",
                            "v-up" => "window.__autotappoLayoutMode = 'v-up'",
                            "v-down" => "window.__autotappoLayoutMode = 'v-down'",
                            "overlap" => "window.__autotappoLayoutMode = 'overlap'",
                            _ => "window.__autotappoLayoutMode = 'h-right'",
                        },
                    );
                    eval(&mut cdp, &format!("window.__autotappoLayoutManual = {}", if layout_manual() { "true" } else { "false" }));
                    let wrap = layout_wrap();
                    let wrap_dir = layout_wrap_dir();
                    eval(&mut cdp, &format!("window.__autotappoLayoutWrap = {wrap}"));
                    eval(&mut cdp, &format!("window.__autotappoLayoutWrapDir = {wrap_dir}"));
                    eval(&mut cdp, SCRIPT);
                    eval(&mut cdp, &format!("window.__autotappoArrange && window.__autotappoArrange.setWrap && window.__autotappoArrange.setWrap({wrap})"));
                    eval(&mut cdp, &format!("window.__autotappoArrange && window.__autotappoArrange.setWrapDir && window.__autotappoArrange.setWrapDir({wrap_dir})"));
                    eval(&mut cdp, &format!("window.__autotappoArrange && window.__autotappoArrange.setManual && window.__autotappoArrange.setManual({})", if layout_manual() { "true" } else { "false" }));
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
                        match layout_mode() {
                            "h-left" => "window.__autotappoArrange && window.__autotappoArrange.setMode('h-left')",
                            "v-up" => "window.__autotappoArrange && window.__autotappoArrange.setMode('v-up')",
                            "v-down" => "window.__autotappoArrange && window.__autotappoArrange.setMode('v-down')",
                            "overlap" => "window.__autotappoArrange && window.__autotappoArrange.setMode('overlap')",
                            _ => "window.__autotappoArrange && window.__autotappoArrange.setMode('h-right')",
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
    loop {
        let edit_pressed = unsafe { (GetAsyncKeyState(edit_hotkey()) as u16) & 0x8000 != 0 };
        if ENABLED.load(Ordering::SeqCst) && edit_pressed && !edit_down {
            if !layout_manual() || layout_mode() == "overlap" {
                EDIT.store(false, Ordering::SeqCst);
            } else {
                let next = !EDIT.load(Ordering::SeqCst);
                EDIT.store(next, Ordering::SeqCst);
            }
        }
        edit_down = edit_pressed;
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
    fn hotkeys_reject_reserved_keys_and_keep_one_layout_key() {
        set_hotkeys(0x74);
        assert_eq!(edit_hotkey(), 0x74);
        assert_eq!(set_edit_hotkey(0x77), 0x75);
        assert_eq!(set_edit_hotkey(0x74), 0x74);
        set_hotkeys(0x75);
        assert_eq!(edit_hotkey(), 0x75);
    }
}
