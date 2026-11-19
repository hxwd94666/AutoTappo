use std::sync::atomic::Ordering;
use tauri::State;

use crate::logging;
use crate::state::AppState;

#[tauri::command]
pub fn get_chest_status(state: State<'_, AppState>) -> crate::features::chest::ChestSnapshot {
    state.0.chest.status()
}

#[tauri::command]
pub fn set_auto_chest(state: State<'_, AppState>, enabled: bool) {
    logging::info("settings", format!("auto_chest={enabled}"));
    state.0.chest.set_enabled(enabled);
}

#[tauri::command]
pub fn get_advanced_status(state: State<'_, AppState>) -> crate::features::advanced::AdvancedSnapshot {
    state.0.advanced.snapshot()
}

#[tauri::command]
pub fn set_advanced_enabled(state: State<'_, AppState>, enabled: bool) {
    logging::info("settings", format!("advanced_enabled={enabled}"));
    state.0.advanced.enabled.store(enabled, Ordering::SeqCst);
    crate::features::arrange::set_enabled(enabled);
}

#[tauri::command]
pub fn set_advanced_flags(state: State<'_, AppState>, lock_screen: bool, hidden: bool) {
    logging::info("settings", format!("advanced_flags lock_screen={lock_screen} hidden={hidden}"));
    let a = &state.0.advanced;
    a.lock_screen.store(lock_screen, Ordering::SeqCst);
    a.hidden.store(hidden, Ordering::SeqCst);
}
