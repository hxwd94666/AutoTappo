#[tauri::command]
pub fn open_log_dir() -> Result<(), String> {
    crate::logging::open_dir()
}

#[tauri::command]
pub fn get_log_dir() -> String {
    crate::logging::dir().to_string_lossy().to_string()
}
