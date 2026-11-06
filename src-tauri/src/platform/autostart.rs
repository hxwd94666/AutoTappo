//! 开机自启：用任务计划程序（ONLOGON + HIGHEST），避免注册表 Run 每次开机弹 UAC。

use std::os::windows::process::CommandExt;
use std::process::Command;

const TASK_NAME: &str = "AutoTappo";
const LAUNCH_ARG: &str = "--autostart";
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

fn schtasks(args: &[&str]) -> Result<std::process::Output, String> {
    Command::new("schtasks")
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| format!("调用 schtasks 失败：{e}"))
}

/// schtasks 默认会禁止电池供电时启动、并在切换到电池时停止任务。
/// 这个程序必须持续挂机，所以显式关闭这两个电源策略。
fn relax_power_settings() -> Result<(), String> {
    let script = format!(
        "$t = Get-ScheduledTask -TaskName '{}' -ErrorAction Stop;          $t.Settings.DisallowStartIfOnBatteries = $false;          $t.Settings.StopIfGoingOnBatteries = $false;          Set-ScheduledTask -TaskName '{}' -Settings $t.Settings | Out-Null",
        TASK_NAME, TASK_NAME
    );
    let out = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| format!("调用 PowerShell 失败：{e}"))?;
    if !out.status.success() {
        return Err(format!(
            "关闭自启任务电池限制失败：{}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(())
}

pub fn is_enabled() -> bool {
    schtasks(&["/Query", "/TN", TASK_NAME])
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// 旧版本创建的启动任务没有 --autostart；这里做一次迁移，下一次登录即静默启动。
pub fn ensure_launch_arg() {
    if !is_enabled() {
        return;
    }
    // 旧任务也要修正电源策略，不能只在新建时处理。
    let _ = relax_power_settings();
    let Ok(out) = schtasks(&["/Query", "/TN", TASK_NAME, "/XML"]) else {
        return;
    };
    let xml = String::from_utf8_lossy(&out.stdout);
    let current_exe = std::env::current_exe()
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    // 旧任务既没有参数，也可能指向旧路径；两者任一不匹配就重建为当前 exe。
    if !xml.contains(LAUNCH_ARG) || !xml.contains(&current_exe) {
        let _ = set_enabled(true);
    }
}

pub fn set_enabled(enabled: bool) -> Result<bool, String> {
    if enabled {
        let exe = std::env::current_exe().map_err(|e| format!("取当前路径失败：{e}"))?;
        let tr = format!("\"{}\" {}", exe.display(), LAUNCH_ARG);
        let out = schtasks(&[
            "/Create", "/TN", TASK_NAME, "/TR", &tr, "/SC", "ONLOGON", "/RL", "HIGHEST", "/F",
        ])?;
        if !out.status.success() {
            return Err("创建开机自启任务失败（需要管理员权限）".into());
        }
        relax_power_settings()?;
    } else {
        let out = schtasks(&["/Delete", "/TN", TASK_NAME, "/F"])?;
        if !out.status.success() && is_enabled() {
            return Err("删除开机自启任务失败".into());
        }
    }
    Ok(is_enabled())
}
