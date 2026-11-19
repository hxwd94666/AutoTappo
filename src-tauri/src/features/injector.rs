//! 高级开箱：把 autotappo_hook.dll 注入 WuwaTappo，让 WebView2 带 CDP 端口启动。
//! 默认不启用；只有用户显式打开「高级开箱」时才会调用 ensure_cdp_pet()。

use std::ffi::c_void;
use std::mem::size_of;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use windows::core::{s, w, PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::Diagnostics::Debug::WriteProcessMemory;
use crate::logging;
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows::Win32::System::Memory::{
    VirtualAllocEx, VirtualFreeEx, MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_READWRITE,
};
use windows::Win32::System::Threading::{
    CreateProcessW, CreateRemoteThread, GetExitCodeProcess, GetExitCodeThread, OpenProcess,
    QueryFullProcessImageNameW, ResumeThread, TerminateProcess, WaitForSingleObject,
    CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT, INFINITE, PROCESS_INFORMATION,
    PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE, STARTUPINFOW,
};

pub const CDP_PORT: u16 = 9223;

pub fn cdp_alive() -> bool {
    ureq::get(&format!("http://127.0.0.1:{CDP_PORT}/json/version"))
        .timeout(Duration::from_millis(700))
        .call()
        .is_ok()
}

fn find_pet_pid() -> Option<u32> {
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0).ok()?;
        let mut entry = PROCESSENTRY32W {
            dwSize: size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        let mut ok = Process32FirstW(snap, &mut entry).is_ok();
        while ok {
            let name = String::from_utf16_lossy(&entry.szExeFile);
            let name = name.trim_end_matches('\0');
            if name.eq_ignore_ascii_case("WuwaTappo.exe") {
                let _ = CloseHandle(snap);
                return Some(entry.th32ProcessID);
            }
            ok = Process32NextW(snap, &mut entry).is_ok();
        }
        let _ = CloseHandle(snap);
        None
    }
}

fn process_path(pid: u32) -> Option<PathBuf> {
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 2048];
        let mut size = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut size);
        let _ = CloseHandle(h);
        if ok.is_err() {
            return None;
        }
        Some(PathBuf::from(String::from_utf16_lossy(&buf[..size as usize])))
    }
}

fn process_exit_code(pid: u32) -> Option<u32> {
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut code = 0u32;
        let ok = GetExitCodeProcess(h, &mut code).is_ok();
        let _ = CloseHandle(h);
        if ok { Some(code) } else { None }
    }
}

fn descendant_pids(root: u32) -> Vec<u32> {
    unsafe {
        let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { return Vec::new() };
        let mut entry = PROCESSENTRY32W {
            dwSize: size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        let mut pairs: Vec<(u32, u32)> = Vec::new();
        let mut ok = Process32FirstW(snap, &mut entry).is_ok();
        while ok {
            pairs.push((entry.th32ProcessID, entry.th32ParentProcessID));
            ok = Process32NextW(snap, &mut entry).is_ok();
        }
        let _ = CloseHandle(snap);

        let mut out = Vec::new();
        let mut frontier = vec![root];
        while let Some(parent) = frontier.pop() {
            for (pid, ppid) in &pairs {
                if *ppid == parent && *pid != root && !out.contains(pid) {
                    out.push(*pid);
                    frontier.push(*pid);
                }
            }
        }
        out
    }
}

fn terminate_tree(pid: u32) -> Result<(), String> {
    let children = descendant_pids(pid);
    logging::info(
        "inject",
        format!("结束旧桌宠进程树 root={pid} children={children:?}"),
    );
    for child in children.iter().rev() {
        if let Err(e) = terminate(*child) {
            logging::warn("inject", format!("结束子进程失败 pid={child}: {e}"));
        }
    }
    terminate(pid)
}

fn hook_log_path(pid: u32) -> PathBuf {
    crate::logging::dir().join(format!("hook-{pid}.log"))
}

fn log_webview_snapshot(wuwa_pid: u32) {
    let script = "$OutputEncoding=[Console]::OutputEncoding=[Text.Encoding]::UTF8;"
        .to_string()
        + &format!(
            "Write-Output '--- Wuwa Pid ---'; Write-Output '{wuwa_pid}';
             Write-Output '--- WebView2 ---';
             Get-CimInstance Win32_Process -Filter \"Name='msedgewebview2.exe'\" | Select-Object ProcessId,ParentProcessId,CommandLine | Format-List;
             Write-Output '--- Port 9223 ---';
             Get-NetTCPConnection -LocalPort 9223 -ErrorAction SilentlyContinue | Select-Object State,LocalAddress,LocalPort,OwningProcess | Format-List;
             Write-Output '--- End ---'"
        );
    logging::info("webview", format!("采集 WebView2 进程快照 wuwa_pid={wuwa_pid}"));
    match std::process::Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            &script,
        ])
        .creation_flags(0x08000000)
        .output()
    {
        Ok(output) => {
            let text = String::from_utf8_lossy(&output.stdout);
            for line in text.lines().take(500) {
                logging::info("webview", line);
            }
            if !output.status.success() {
                logging::warn(
                    "webview",
                    format!("WebView2 快照命令失败: {}", output.status),
                );
            }
        }
        Err(e) => logging::warn("webview", format!("无法采集 WebView2 快照: {e}")),
    }
}

fn copy_hook_log(pid: u32) {
    let target = hook_log_path(pid);
    if !target.exists() {
        let fallback = std::env::temp_dir().join(format!("autotappo-hook-{pid}.log"));
        if fallback.exists() {
            let _ = std::fs::copy(&fallback, &target);
        }
    }
    if target.exists() {
        logging::info("hook", format!("Hook 日志: {}", target.display()));
    } else {
        logging::warn("hook", format!("Hook 日志不存在: {}", target.display()));
    }
}

fn webview_process_count() -> usize {
    unsafe {
        let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { return 0 };
        let mut entry = PROCESSENTRY32W {
            dwSize: size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        let mut count = 0usize;
        let mut ok = Process32FirstW(snap, &mut entry).is_ok();
        while ok {
            let name = String::from_utf16_lossy(&entry.szExeFile);
            if name.trim_end_matches('\0').eq_ignore_ascii_case("msedgewebview2.exe") {
                count += 1;
            }
            ok = Process32NextW(snap, &mut entry).is_ok();
        }
        let _ = CloseHandle(snap);
        count
    }
}

fn terminate(pid: u32) -> Result<(), String> {
    unsafe {
        let h = OpenProcess(PROCESS_TERMINATE, false, pid)
            .map_err(|e| format!("OpenProcess 失败: {e:?}"))?;
        let r = TerminateProcess(h, 1);
        let _ = CloseHandle(h);
        r.map_err(|e| format!("TerminateProcess 失败: {e:?}"))
    }
}

fn wait_gone(pid: u32, timeout: Duration) -> bool {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if find_pet_pid() != Some(pid) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    find_pet_pid() != Some(pid)
}

fn is_pet_exe(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.eq_ignore_ascii_case("WuwaTappo.exe"))
        .unwrap_or(false)
}

/// 把注册表里的 DisplayIcon / InstallLocation 规整成 WuwaTappo.exe 路径。
fn normalize_exe(raw: &str) -> Option<PathBuf> {
    let mut s = raw.trim().trim_matches('"').to_string();
    // DisplayIcon 常见格式："C:\...\WuwaTappo.exe",0
    if let Some(pos) = s.find(",0") {
        s.truncate(pos);
    }
    let s = s.trim().trim_matches('"');
    if s.is_empty() {
        return None;
    }
    let p = PathBuf::from(s);
    if p.is_file() && is_pet_exe(&p) {
        return Some(p);
    }
    let candidate = p.join("WuwaTappo.exe");
    if candidate.is_file() && is_pet_exe(&candidate) {
        return Some(candidate);
    }
    None
}

fn is_pet_display_name(display: &str) -> bool {
    let lower = display.to_lowercase();
    lower.contains("wuwa tappo")
        || lower.contains("wuwatappo")
        || lower.contains("wuthering waves tappo")
        || lower.contains("鸣潮桌宠")
}

fn find_in_uninstall(root: winreg::HKEY) -> Option<PathBuf> {
    use winreg::enums::KEY_READ;
    use winreg::RegKey;
    for sub in [
        r"Software\Microsoft\Windows\CurrentVersion\Uninstall",
        r"Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall",
    ] {
        let Ok(key) = RegKey::predef(root).open_subkey_with_flags(sub, KEY_READ) else {
            continue;
        };
        for name in key.enum_keys().flatten() {
            let Ok(entry) = key.open_subkey_with_flags(&name, KEY_READ) else {
                continue;
            };
            let display: String = entry.get_value("DisplayName").unwrap_or_default();
            if !is_pet_display_name(&display) {
                continue;
            }
            if let Ok(loc) = entry.get_value::<String, _>("InstallLocation") {
                if let Some(p) = normalize_exe(&loc) {
                    return Some(p);
                }
            }
            if let Ok(icon) = entry.get_value::<String, _>("DisplayIcon") {
                if let Some(p) = normalize_exe(&icon) {
                    return Some(p);
                }
            }
        }
    }
    None
}

fn find_in_app_paths(root: winreg::HKEY) -> Option<PathBuf> {
    use winreg::enums::KEY_READ;
    use winreg::RegKey;
    let key = RegKey::predef(root)
        .open_subkey_with_flags(
            r"Software\Microsoft\Windows\CurrentVersion\App Paths\WuwaTappo.exe",
            KEY_READ,
        )
        .ok()?;
    let value: String = key.get_value("").ok()?;
    normalize_exe(&value)
}

/// 自动发现 WuwaTappo：
/// 1. 正在运行的进程路径（最准）；
/// 2. 注册表卸载项 / App Paths；
/// 3. 常见安装目录（含 LocalAppData、Program Files）。
fn discover_pet_path() -> Option<PathBuf> {
    if let Some(pid) = find_pet_pid() {
        if let Some(p) = process_path(pid) {
            if p.is_file() && is_pet_exe(&p) {
                return Some(p);
            }
        }
    }
    for root in [winreg::enums::HKEY_CURRENT_USER, winreg::enums::HKEY_LOCAL_MACHINE] {
        if let Some(p) = find_in_uninstall(root) {
            return Some(p);
        }
        if let Some(p) = find_in_app_paths(root) {
            return Some(p);
        }
    }
    let mut candidates: Vec<PathBuf> = vec![
        PathBuf::from(r"D:\WuwaTappo\WuwaTappo.exe"),
        PathBuf::from(r"C:\Program Files\WuwaTappo\WuwaTappo.exe"),
        PathBuf::from(r"C:\Program Files (x86)\WuwaTappo\WuwaTappo.exe"),
    ];
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        let base = PathBuf::from(local);
        candidates.push(base.join("Programs").join("WuwaTappo").join("WuwaTappo.exe"));
        candidates.push(base.join("WuwaTappo").join("WuwaTappo.exe"));
    }
    for var in ["ProgramFiles", "ProgramFiles(x86)"] {
        if let Some(pf) = std::env::var_os(var) {
            candidates.push(PathBuf::from(pf).join("WuwaTappo").join("WuwaTappo.exe"));
        }
    }
    candidates.into_iter().find(|p| p.is_file())
}

fn dll_path() -> Result<PathBuf, String> {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            for rel in [
                "autotappo_hook.dll",
                "resources\\autotappo_hook.dll",
                "native\\autotappo_hook\\target\\release\\autotappo_hook.dll",
            ] {
                let p = dir.join(rel);
                if p.exists() {
                    return Ok(p);
                }
            }
        }
    }
    let dev = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("native/autotappo_hook/target/release/autotappo_hook.dll");
    if dev.exists() {
        return Ok(dev);
    }
    Err("找不到 autotappo_hook.dll（应与 AutoTappo.exe 同目录）".into())
}

fn inject_dll(process: HANDLE, dll: &Path) -> Result<(), String> {
    unsafe {
        let dll_wide: Vec<u16> = dll.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
        let bytes = dll_wide.len() * 2;
        let remote = VirtualAllocEx(process, None, bytes, MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE);
        if remote.is_null() {
            return Err("VirtualAllocEx 失败".into());
        }
        let mut written = 0usize;
        WriteProcessMemory(
            process,
            remote,
            dll_wide.as_ptr() as *const c_void,
            bytes,
            Some(&mut written),
        )
        .map_err(|e| format!("WriteProcessMemory 失败: {e:?}"))?;

        let kernel = GetModuleHandleW(w!("kernel32.dll")).map_err(|e| format!("{e:?}"))?;
        let load = GetProcAddress(kernel, s!("LoadLibraryW"))
            .ok_or_else(|| "GetProcAddress(LoadLibraryW) 失败".to_string())?;
        let start: unsafe extern "system" fn(*mut c_void) -> u32 = std::mem::transmute(load);
        let thread = CreateRemoteThread(process, None, 0, Some(start), Some(remote), 0, None)
            .map_err(|e| format!("CreateRemoteThread 失败: {e:?}"))?;
        WaitForSingleObject(thread, INFINITE);
        let mut load_result = 0u32;
        let _ = GetExitCodeThread(thread, &mut load_result);
        let _ = CloseHandle(thread);
        let _ = VirtualFreeEx(process, remote, 0, MEM_RELEASE);
        if load_result == 0 {
            logging::error("inject", "LoadLibraryW 返回 0，DLL 加载失败");
            return Err("LoadLibraryW 返回 0，DLL 加载失败（依赖缺失或被拦截）".into());
        }
        logging::info("inject", format!("LoadLibraryW 成功 handle=0x{load_result:X}"));
        Ok(())
    }
}

fn start_process(exe: &Path, inject: Option<&Path>) -> Result<u32, String> {
    unsafe {
        let mut cmd: Vec<u16> = format!("\"{}\"", exe.display())
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let si = STARTUPINFOW {
            cb: size_of::<STARTUPINFOW>() as u32,
            ..Default::default()
        };
        let mut pi = PROCESS_INFORMATION::default();
        let flags = if inject.is_some() {
            CREATE_SUSPENDED | CREATE_UNICODE_ENVIRONMENT
        } else {
            CREATE_UNICODE_ENVIRONMENT
        };
        let cwd_wide: Option<Vec<u16>> = exe.parent().map(|dir| {
            dir.as_os_str()
                .encode_wide()
                .chain(std::iter::once(0))
                .collect()
        });
        let cwd_ptr = cwd_wide
            .as_ref()
            .map_or(PCWSTR::null(), |v| PCWSTR(v.as_ptr()));
        logging::info(
            "inject",
            format!(
                "启动工作目录: {}",
                exe.parent().map(|p| p.display().to_string()).unwrap_or_default()
            ),
        );
        let env_key = "WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS";
        let old_env = std::env::var_os(env_key);
        let hook_log_key = "AUTOTAPPO_HOOK_LOG_DIR";
        let old_hook_log = std::env::var_os(hook_log_key);
        if inject.is_some() {
            let log_dir = crate::logging::dir();
            let _ = std::fs::create_dir_all(&log_dir);
            std::env::set_var(hook_log_key, &log_dir);
            logging::info("inject", format!("设置 {hook_log_key}={}", log_dir.display()));
            let mut value = old_env
                .as_ref()
                .map(|v| v.to_string_lossy().to_string())
                .unwrap_or_default();
            if !value.contains("--remote-debugging-port") {
                if !value.is_empty() && !value.ends_with(' ') {
                    value.push(' ');
                }
                value.push_str("--remote-debugging-port=9223");
            }
            std::env::set_var(env_key, &value);
            logging::info("inject", format!("设置 {env_key}={value}"));
        }
        let created = CreateProcessW(
            PCWSTR::null(),
            PWSTR(cmd.as_mut_ptr()),
            None,
            None,
            false,
            flags,
            None,
            cwd_ptr,
            &si,
            &mut pi,
        );
        if inject.is_some() {
            match old_env {
                Some(value) => std::env::set_var(env_key, value),
                None => std::env::remove_var(env_key),
            }
            match old_hook_log {
                Some(value) => std::env::set_var(hook_log_key, value),
                None => std::env::remove_var(hook_log_key),
            }
        }
        created.map_err(|e| format!("CreateProcessW 失败: {e:?}"))?;

        if let Some(dll) = inject {
            if let Err(e) = inject_dll(pi.hProcess, dll) {
                let _ = TerminateProcess(pi.hProcess, 1);
                let _ = CloseHandle(pi.hThread);
                let _ = CloseHandle(pi.hProcess);
                return Err(e);
            }
            let _ = ResumeThread(pi.hThread);
        }
        let _ = CloseHandle(pi.hThread);
        let _ = CloseHandle(pi.hProcess);
        Ok(pi.dwProcessId)
    }
}

/// 确保桌宠以「带 CDP 端口」的方式运行。会强杀一次旧桌宠。
pub fn ensure_cdp_pet() -> Result<(), String> {
    logging::info("inject", "ensure_cdp_pet 开始");
    if cdp_alive() {
        logging::info("inject", "CDP 已在线，无需注入");
        return Ok(());
    }
    let dll = dll_path()?;
    logging::info("inject", format!("DLL: {}", dll.display()));
    let exe = match find_pet_pid() {
        Some(pid) => {
            let running_path = process_path(pid);
            logging::info("inject", format!("结束旧桌宠 pid={pid}"));
            terminate_tree(pid)?;
            let _ = wait_gone(pid, Duration::from_secs(5));
            running_path
                .filter(|p| p.is_file())
                .or_else(discover_pet_path)
                .ok_or_else(|| "读不到桌宠路径，且自动发现失败".to_string())?
        }
        None => discover_pet_path().ok_or_else(|| "找不到 WuwaTappo.exe（自动发现失败）".to_string())?,
    };
    logging::info("inject", format!("启动并注入: {}", exe.display()));
    let pid = start_process(&exe, Some(&dll))?;
    logging::info("inject", format!("启动完成 pid={pid}，等待 CDP"));
    std::thread::sleep(Duration::from_millis(300));
    copy_hook_log(pid);
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(25) {
        if cdp_alive() {
            logging::info("inject", "CDP 端口已就绪");
            copy_hook_log(pid);
            return Ok(());
        }
        if let Some(code) = process_exit_code(pid) {
            if code != 259 {
                logging::error(
                    "inject",
                    format!(
                        "WuwaTappo 提前退出 pid={pid} exit_code={code} webview2_count={}",
                        webview_process_count()
                    ),
                );
                log_webview_snapshot(pid);
                return Err(format!("WuwaTappo 已退出 (exit code {code})"));
            }
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    logging::error(
        "inject",
        format!(
            "CDP 端口未在 25 秒内就绪 pid={pid} exit_code={:?} webview2_count={}",
            process_exit_code(pid),
            webview_process_count()
        ),
    );
    copy_hook_log(pid);
    log_webview_snapshot(pid);
    Err("CDP 端口未在 25 秒内就绪".into())
}


#[cfg(test)]
mod tests {
    use super::*;

    fn temp_exe_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "autotappo-test-{tag}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("WuwaTappo.exe"), b"MZ").unwrap();
        dir
    }

    #[test]
    fn normalize_directory() {
        let dir = temp_exe_dir("dir");
        let got = normalize_exe(dir.to_str().unwrap()).unwrap();
        assert!(got.ends_with("WuwaTappo.exe"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn normalize_display_icon() {
        let dir = temp_exe_dir("icon");
        let exe = dir.join("WuwaTappo.exe");
        let raw = format!("\"{}\",0", exe.display());
        let got = normalize_exe(&raw).unwrap();
        assert_eq!(got, exe);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn discover_prefers_running_pet() {
        if find_pet_pid().is_none() {
            return; // CI / 没开桌宠时跳过
        }
        let path = discover_pet_path().expect("running pet should be discoverable");
        assert!(path.is_file(), "discovered path must exist: {}", path.display());
        assert_eq!(
            path.file_name().unwrap().to_string_lossy().to_ascii_lowercase(),
            "wuwatappo.exe"
        );
    }

    #[test]
    fn display_name_filter_excludes_autotappo() {
        assert!(!is_pet_display_name("AutoTappo"));
        assert!(!is_pet_display_name("鸣潮工坊"));
        assert!(is_pet_display_name("Wuwa Tappo"));
        assert!(is_pet_display_name("鸣潮桌宠"));
    }

    #[test]
    fn normalize_rejects_non_pet_exe() {
        let dir = temp_exe_dir("other-exe");
        let other = dir.join("鸣潮工坊.exe");
        std::fs::write(&other, b"MZ").unwrap();
        assert!(!is_pet_exe(&other));
        assert!(normalize_exe(other.to_str().unwrap()).is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn normalize_rejects_missing() {
        assert!(normalize_exe("Z:\\definitely-missing\\WuwaTappo.exe").is_none());
    }
}