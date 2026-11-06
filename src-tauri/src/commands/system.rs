use serde::Serialize;
use tauri::{AppHandle, Manager, State};

use crate::state::AppState;

#[derive(Serialize)]
pub struct AppStatusResponse {
    pub running: bool,
    pub speed_mode: u8,
    pub total_sent: u64,
    pub hotkey_vk: i32,
    /// 0 = 键盘连点，1 = 静默注入
    pub mode: u8,
    pub silent: crate::features::silent::SilentSnapshot,
}

#[tauri::command]
pub fn get_status(state: State<'_, AppState>) -> AppStatusResponse {
    use std::sync::atomic::Ordering;
    let inner = &state.0;
    let mode = inner.clicker.mode.load(Ordering::Relaxed);
    let (running, total_sent) = if mode == 1 {
        (
            inner.silent.running.load(Ordering::Relaxed),
            inner.silent.total.load(Ordering::Relaxed),
        )
    } else {
        (
            inner.clicker.running.load(Ordering::Relaxed),
            inner.clicker.total_sent.load(Ordering::Relaxed),
        )
    };
    AppStatusResponse {
        running,
        speed_mode: inner.clicker.speed_mode.load(Ordering::Relaxed),
        total_sent,
        hotkey_vk: inner.clicker.hotkey_vk.load(Ordering::Relaxed),
        mode,
        silent: inner.silent.snapshot(),
    }
}

#[tauri::command]
pub fn minimize_window(app: AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.minimize();
    }
}

#[tauri::command]
pub fn close_window(app: AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.close();
    }
}

#[tauri::command]
pub fn start_drag(app: AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.start_dragging();
    }
}
