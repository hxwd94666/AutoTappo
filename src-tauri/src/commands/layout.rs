#[tauri::command]
pub fn get_layout_manual() -> bool {
    crate::features::arrange::layout_manual()
}

#[tauri::command]
pub fn set_layout_manual(manual: bool) -> bool {
    crate::features::arrange::set_layout_manual(manual);
    crate::features::arrange::layout_manual()
}

#[tauri::command]
pub fn get_layout_mode() -> String {
    crate::features::arrange::layout_mode().to_string()
}

#[tauri::command]
pub fn set_layout_mode(mode: String) -> String {
    crate::features::arrange::set_layout_mode(&mode);
    crate::features::arrange::layout_mode().to_string()
}

#[tauri::command]
pub fn get_layout_wrap() -> u8 {
    crate::features::arrange::layout_wrap()
}

#[tauri::command]
pub fn set_layout_wrap(wrap: u8) -> u8 {
    crate::features::arrange::set_layout_wrap(wrap);
    crate::features::arrange::layout_wrap()
}

#[tauri::command]
pub fn get_layout_wrap_dir() -> u8 {
    crate::features::arrange::layout_wrap_dir()
}

#[tauri::command]
pub fn set_layout_wrap_dir(dir: u8) -> u8 {
    crate::features::arrange::set_layout_wrap_dir(dir);
    crate::features::arrange::layout_wrap_dir()
}

#[tauri::command]
pub fn get_layout_edit_hotkey() -> i32 {
    crate::features::arrange::edit_hotkey()
}

#[tauri::command]
pub fn set_layout_edit_hotkey(vk: i32) -> i32 {
    crate::features::arrange::set_edit_hotkey(vk)
}


#[tauri::command]
pub fn get_layout_edit() -> bool {
    crate::features::arrange::edit()
}
