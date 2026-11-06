//! 持久化配置：让手动启动/开机自启都恢复上次的模式、运行状态、速度、热键、开箱开关。

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

use crate::features::advanced::AdvancedController;
use crate::features::chest::ChestController;
use crate::features::clicker::ClickerController;
use crate::features::silent::SilentController;

const DEFAULT_HOTKEY_VK: i32 = 0x74; // F5

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// 0 = 键盘，1 = 静默
    pub mode: u8,
    /// 上次退出/变更时是否正在刷数
    pub running: bool,
    /// 键盘模式速度档位
    pub speed: u8,
    pub hotkey_vk: i32,
    pub layout_edit_hotkey_vk: i32,    pub auto_chest: bool,
    pub advanced: bool,
    pub auto_arrange: bool,
    pub layout_mode: String,
    #[serde(default)]
    pub layout_wrap: u8,
    #[serde(default)]
    pub layout_wrap_dir: u8,
    pub lock_screen: bool,
    pub hidden: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            mode: 1,
            running: false,
            speed: 0,
            hotkey_vk: DEFAULT_HOTKEY_VK,
            layout_edit_hotkey_vk: 0x75,            auto_chest: false,
            advanced: false,
            auto_arrange: false,
            layout_mode: "h-right".into(),
            layout_wrap: 0,
            layout_wrap_dir: 0,
            lock_screen: false,
            hidden: false,
        }
    }
}

impl Settings {
    pub fn load() -> Self {
        let Some(path) = path() else {
            return Self::default();
        };
        let mut s = fs::read(&path)
            .ok()
            .and_then(|b| serde_json::from_slice::<Self>(&b).ok())
            .unwrap_or_default();
        // 旧默认 F4 → 新默认 F5；旧手动布局 F5 → 新 F6。
        if s.hotkey_vk == 0x73 {
            s.hotkey_vk = DEFAULT_HOTKEY_VK;
        }
        if s.layout_edit_hotkey_vk == 0x74 {
            s.layout_edit_hotkey_vk = 0x75;        }
        s
    }

    pub fn save(&self) -> Result<(), String> {
        let path = path().ok_or_else(|| "找不到 %APPDATA% 路径".to_string())?;
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).map_err(|e| format!("创建设置目录失败：{e}"))?;
        }
        let tmp = path.with_extension("json.tmp");
        let data = serde_json::to_vec_pretty(self).map_err(|e| format!("序列化设置失败：{e}"))?;
        fs::write(&tmp, data).map_err(|e| format!("写设置临时文件失败：{e}"))?;
        let _ = fs::remove_file(&path);
        fs::rename(&tmp, &path).map_err(|e| format!("替换设置文件失败：{e}"))
    }

    /// 从控制器当前原子状态捕获一份配置，供后台监听线程保存。
    pub fn capture(
        clicker: &ClickerController,
        silent: &SilentController,
        chest: &ChestController,
        advanced: &AdvancedController,
    ) -> Self {
        use std::sync::atomic::Ordering;
        let mode = clicker.mode.load(Ordering::Relaxed);
        let running = if mode == 1 {
            silent.running.load(Ordering::Relaxed)
        } else {
            clicker.running.load(Ordering::Relaxed)
        };
        let advanced_on = advanced.enabled.load(Ordering::Relaxed);
        Self {
            mode: if mode == 1 { 1 } else { 0 },
            running,
            speed: clicker.speed_mode.load(Ordering::Relaxed).min(2),
            hotkey_vk: clicker.hotkey_vk.load(Ordering::Relaxed),
            layout_edit_hotkey_vk: crate::features::arrange::edit_hotkey(),            auto_chest: chest.enabled.load(Ordering::Relaxed),
            advanced: advanced_on,
            auto_arrange: crate::features::arrange::enabled(),
            layout_mode: crate::features::arrange::layout_mode().to_string(),
            layout_wrap: crate::features::arrange::layout_wrap(),
            layout_wrap_dir: crate::features::arrange::layout_wrap_dir(),
            lock_screen: advanced_on,
            hidden: advanced_on,
        }
    }

    pub fn apply(
        &self,
        clicker: &ClickerController,
        silent: &SilentController,
        chest: &ChestController,
        advanced: &AdvancedController,
    ) {
        use std::sync::atomic::Ordering;
        let mode = if self.mode == 1 { 1 } else { 0 };
        clicker.mode.store(mode, Ordering::SeqCst);
        clicker.speed_mode.store(self.speed.min(2), Ordering::SeqCst);
        clicker.hotkey_vk.store(
            if self.hotkey_vk > 0 { self.hotkey_vk } else { DEFAULT_HOTKEY_VK },
            Ordering::SeqCst,
        );
        crate::features::arrange::set_hotkeys(self.layout_edit_hotkey_vk);
        crate::features::arrange::set_layout_mode(&self.layout_mode);
        crate::features::arrange::set_layout_wrap(self.layout_wrap);
        crate::features::arrange::set_layout_wrap_dir(self.layout_wrap_dir);
        if mode == 1 {
            silent.running.store(self.running, Ordering::SeqCst);
            clicker.running.store(false, Ordering::SeqCst);
        } else {
            clicker.running.store(self.running, Ordering::SeqCst);
            silent.running.store(false, Ordering::SeqCst);
        }

        chest.set_enabled(self.auto_chest);
        // 高级开箱依赖自动开箱；开启时统一覆盖锁屏/隐藏两个场景。
        let advanced_on = self.advanced && self.auto_chest;
        advanced.enabled.store(advanced_on, Ordering::SeqCst);
        advanced.lock_screen.store(advanced_on, Ordering::SeqCst);
        advanced.hidden.store(advanced_on, Ordering::SeqCst);
        crate::features::arrange::set_enabled(self.auto_arrange && advanced_on);
    }
}

fn path() -> Option<PathBuf> {
    let appdata = std::env::var_os("APPDATA")?;
    Some(PathBuf::from(appdata).join("AutoTappo").join("settings.json"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn settings_roundtrip() {
        let s = Settings {
            mode: 0,
            running: true,
            speed: 2,
            hotkey_vk: 0x76,
            layout_edit_hotkey_vk: 0x75,            auto_chest: true,
            advanced: true,
            auto_arrange: false,
            layout_mode: "h-right".into(),
            layout_wrap: 0,
            layout_wrap_dir: 0,
            lock_screen: true,
            hidden: true,
        };
        let encoded = serde_json::to_vec(&s).unwrap();
        let back: Settings = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(s, back);
    }

    #[test]
    fn apply_and_capture_restores_state() {
        let silent = SilentController::new();
        let clicker = ClickerController::new(Arc::clone(&silent.running));
        let chest = ChestController::new();
        let advanced = AdvancedController::new(Arc::clone(&chest.enabled));
        let s = Settings {
            mode: 0,
            running: true,
            speed: 2,
            hotkey_vk: 0x76,
            layout_edit_hotkey_vk: 0x75,            auto_chest: true,
            advanced: true,
            auto_arrange: false,
            layout_mode: "h-right".into(),
            layout_wrap: 0,
            layout_wrap_dir: 0,
            lock_screen: true,
            hidden: true,
        };
        s.apply(&clicker, &silent, &chest, &advanced);
        assert_eq!(clicker.mode.load(std::sync::atomic::Ordering::Relaxed), 0);
        assert!(clicker.running.load(std::sync::atomic::Ordering::Relaxed));
        assert!(!silent.running.load(std::sync::atomic::Ordering::Relaxed));
        assert!(chest.enabled.load(std::sync::atomic::Ordering::Relaxed));
        assert!(advanced.enabled.load(std::sync::atomic::Ordering::Relaxed));

        let back = Settings::capture(&clicker, &silent, &chest, &advanced);
        assert_eq!(back.mode, 0);
        assert!(back.running);
        assert_eq!(back.speed, 2);
        assert_eq!(back.hotkey_vk, 0x76);
        assert!(back.auto_chest);
        assert!(back.advanced);
        assert_eq!(back.layout_edit_hotkey_vk, 0x75);    }

    #[test]
    fn missing_fields_use_defaults() {
        let back: Settings = serde_json::from_slice(br#"{"mode":0}"#).unwrap();
        assert_eq!(back.mode, 0);
        assert_eq!(back.hotkey_vk, DEFAULT_HOTKEY_VK);
        assert!(!back.running);
        assert_eq!(back.layout_mode, "h-right");
        assert_eq!(back.layout_wrap, 0);
    }
}
