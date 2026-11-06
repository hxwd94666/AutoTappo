#[tauri::command]
pub fn get_autostart() -> bool {
    crate::platform::autostart::is_enabled()
}

#[tauri::command]
pub fn set_autostart(enabled: bool) -> Result<bool, String> {
    crate::platform::autostart::set_enabled(enabled)
}
