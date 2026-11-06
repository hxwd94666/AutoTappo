//! Tauri 应用装配：状态、worker、配置监听、托盘与窗口生命周期。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use tauri::Manager;

use crate::commands;
use crate::config::settings;
use crate::features::advanced::AdvancedController;
use crate::features::arrange;
use crate::features::chest::ChestController;
use crate::features::clicker::ClickerController;
use crate::features::silent::SilentController;
use crate::platform::{autostart, tray, window};
use crate::state::{AppState, Inner};

pub fn run(autostart_launch: bool) {
    let app = tauri::Builder::default()
        .setup(move |app| {
            // 静默控制器先创建，把同一个 running 原子量交给键盘热键监听，
            // 这样 F4 和面板/托盘切换的是同一个开关。
            let silent = Arc::new(SilentController::new());
            let clicker = Arc::new(ClickerController::new(Arc::clone(&silent.running)));
            silent.start_worker();
            clicker.start_worker();

            // 自动开箱引擎：独立原生线程
            let chest = Arc::new(ChestController::new());
            chest.start_worker();

            // 高级开箱：依赖自动开箱
            let advanced = Arc::new(AdvancedController::new(Arc::clone(&chest.enabled)));
            advanced.start_worker();
            arrange::start_worker();

            // 恢复上次配置（手动启动和开机自启都走同一套）。
            let initial_settings = settings::Settings::load();
            initial_settings.apply(&clicker, &silent, &chest, &advanced);

            let inner = Arc::new(Inner {
                clicker,
                chest,
                silent,
                advanced,
                tray: Mutex::new(None),
                tray_ok: AtomicBool::new(false),
                shutting_down: AtomicBool::new(false),
            });
            app.manage(AppState(Arc::clone(&inner)));

            // 配置监听：F4/托盘等直接改原子量的操作也能在 0.7 秒内落盘。
            let watch = Arc::clone(&inner);
            std::thread::spawn(move || {
                let mut last = settings::Settings::capture(
                    &watch.clicker, &watch.silent, &watch.chest, &watch.advanced,
                );
                loop {
                    std::thread::sleep(std::time::Duration::from_millis(700));
                    if watch.shutting_down.load(Ordering::Relaxed) {
                        break;
                    }
                    let cur = settings::Settings::capture(
                        &watch.clicker, &watch.silent, &watch.chest, &watch.advanced,
                    );
                    if cur != last {
                        let _ = cur.save();
                        last = cur;
                    }
                }
            });

            // 构造系统托盘图标
            match tray::build(app, Arc::clone(&inner)) {
                Ok(icon) => {
                    if let Ok(mut slot) = inner.tray.lock() {
                        *slot = Some(icon);
                    }
                    inner.tray_ok.store(true, Ordering::SeqCst);
                }
                Err(e) => {
                    eprintln!("[AutoTappo] 托盘图标创建失败: {e}");
                }
            }

            // 迁移旧的无参数自启任务；下一次登录即可带 --autostart 启动。
            autostart::ensure_launch_arg();

            // 开机自启时只驻留托盘，不创建面板窗口。
            if !autostart_launch {
                let _ = window::ensure(app.handle());
            }

            Ok(())
        })
        .on_window_event(|win, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let app = win.app_handle();
                let tray_ok = app.state::<AppState>().0.tray_ok.load(Ordering::SeqCst);
                if tray_ok {
                    // 点击右上角关闭：不退出程序，直接销毁窗口释放 WebView2 渲染内存
                    // 托盘图标依然常驻，点击托盘可随时秒级重新唤醒
                    api.prevent_close();
                    let _ = window::close(app);
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::system::get_status,
            commands::runtime::toggle_running,
            commands::runtime::set_speed,
            commands::runtime::set_hotkey,
            commands::runtime::set_mode,
            commands::runtime::add_clicks,
            commands::system::minimize_window,
            commands::system::close_window,
            commands::system::start_drag,
            commands::chest::get_chest_status,
            commands::chest::set_auto_chest,
            commands::chest::get_advanced_status,
            commands::chest::set_advanced_enabled,
            commands::layout::get_layout_edit,
            commands::layout::get_auto_arrange,
            commands::layout::set_auto_arrange,
            commands::layout::get_layout_mode,
            commands::layout::set_layout_mode,
            commands::layout::reset_layout,
            commands::layout::get_layout_edit_hotkey,
            commands::layout::set_layout_edit_hotkey,
            commands::layout::get_layout_save_hotkey,
            commands::layout::set_layout_save_hotkey,
            commands::chest::set_advanced_flags,
            commands::autostart::get_autostart,
            commands::autostart::set_autostart,
        ])
        .build(tauri::generate_context!())
        .expect("AutoTappo 启动失败");

    app.run(|handle, event| {
        // 关闭主窗口后窗口数为 0，Tauri 默认会请求退出。
        // 托盘常驻守护时必须拦住；只有用户在托盘菜单真正点击「退出」时才放行。
        if let tauri::RunEvent::ExitRequested { api, .. } = event {
            let state = handle.state::<AppState>();
            if !state.0.shutting_down.load(Ordering::SeqCst)
                && state.0.tray_ok.load(Ordering::SeqCst)
            {
                api.prevent_exit();
            }
        }
    });
}
