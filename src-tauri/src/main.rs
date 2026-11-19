// Release 发布版禁用 Windows 控制台黑框
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod commands;
mod config;
mod features;
mod logging;
mod platform;
mod state;

fn main() {
    logging::init();
    logging::info("app", format!("args={:?}", std::env::args().collect::<Vec<_>>()));

    // 多屏 / 高 DPI 下必须拿到物理像素坐标，区域与截图像素才能对齐
    features::chest::ensure_dpi_aware();

    let autostart_launch = std::env::args().any(|a| a == "--autostart");
    if !platform::single_instance::acquire(!autostart_launch) {
        logging::warn("app", "单实例互斥已存在，退出本次启动");
        return;
    }

    // 注册「显示器开关状态」电源通知：锁定 / 息屏时必须停手，
    // 否则我们注入的键击会把已经黑掉的显示器重新点亮（详见 session.rs 顶部说明）
    platform::session::init();

    app::run(autostart_launch);
}
