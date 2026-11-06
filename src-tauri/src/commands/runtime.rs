use std::sync::atomic::Ordering;
use tauri::State;

use crate::state::AppState;

#[tauri::command]
pub fn toggle_running(state: State<'_, AppState>) -> bool {
    let inner = &state.0;
    if inner.clicker.mode.load(Ordering::Relaxed) == 1 {
        let prev = inner.silent.running.fetch_xor(true, Ordering::SeqCst);
        !prev
    } else {
        let prev = inner.clicker.running.fetch_xor(true, Ordering::SeqCst);
        !prev
    }
}

#[tauri::command]
pub fn add_clicks(state: State<'_, AppState>, count: u64) -> Result<u64, String> {
    state.0.silent.queue_manual_add(count)
}

/// 切换刷数方式：0 = 键盘连点（会触发动画），1 = 静默注入（只加计数）
#[tauri::command]
pub fn set_mode(state: State<'_, AppState>, mode: u8) {
    let inner = &state.0;
    let mode = if mode == 1 { 1 } else { 0 };
    if mode == 1 {
        inner.clicker.running.store(false, Ordering::SeqCst);
    } else {
        inner.silent.running.store(false, Ordering::SeqCst);
    }
    inner.clicker.mode.store(mode, Ordering::SeqCst);
}

#[tauri::command]
pub fn set_speed(state: State<'_, AppState>, mode: u8) {
    state.0.clicker.speed_mode.store(mode, Ordering::SeqCst);
}

#[tauri::command]
pub fn set_hotkey(state: State<'_, AppState>, vk: i32) {
    state.0.clicker.hotkey_vk.store(vk, Ordering::SeqCst);
}
