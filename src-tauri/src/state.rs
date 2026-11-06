use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use tauri::tray::TrayIcon;

use crate::features::advanced::AdvancedController;
use crate::features::chest::ChestController;
use crate::features::clicker::ClickerController;
use crate::features::silent::SilentController;

/// 全局共享状态。
pub struct Inner {
    pub clicker: Arc<ClickerController>,
    pub chest: Arc<ChestController>,
    pub silent: Arc<SilentController>,
    pub advanced: Arc<AdvancedController>,
    pub tray: Mutex<Option<TrayIcon>>,
    pub tray_ok: AtomicBool,
    pub shutting_down: AtomicBool,
}

pub struct AppState(pub Arc<Inner>);
