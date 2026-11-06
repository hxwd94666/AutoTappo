//! 高级开箱：把 autotappo_hook.dll 注入 WuwaTappo，让 WebView2 带 CDP 端口启动。
//! 默认不启用；只有用户显式打开「高级开箱」时才会调用 ensure_cdp_pet()。

use std::ffi::c_void;
use std::mem::size_of;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use windows::core::{s, w, PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::Diagnostics::Debug::WriteProcessMemory;
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows::Win32::System::Memory::{
    VirtualAllocEx, VirtualFreeEx, MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_READWRITE,
};
use windows::Win32::System::Threading::{
    CreateProcessW, CreateRemoteThread, OpenProcess, QueryFullProcessImageNameW, ResumeThread,
    TerminateProcess, WaitForSingleObject, CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT, INFINITE,
    PROCESS_INFORMATION, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE,
    STARTUPINFOW,
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
        let _ = CloseHandle(thread);
        let _ = VirtualFreeEx(process, remote, 0, MEM_RELEASE);
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
        CreateProcessW(
            PCWSTR::null(),
            PWSTR(cmd.as_mut_ptr()),
            None,
            None,
            false,
            flags,
            None,
            PCWSTR::null(),
            &si,
            &mut pi,
        )
        .map_err(|e| format!("CreateProcessW 失败: {e:?}"))?;

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
    if cdp_alive() {
        return Ok(());
    }
    let dll = dll_path()?;
    let exe = match find_pet_pid() {
        Some(pid) => {
            let running_path = process_path(pid);
            terminate(pid)?;
            let _ = wait_gone(pid, Duration::from_secs(5));
            running_path
                .filter(|p| p.is_file())
                .or_else(discover_pet_path)
                .ok_or_else(|| "读不到桌宠路径，且自动发现失败".to_string())?
        }
        None => discover_pet_path().ok_or_else(|| "找不到 WuwaTappo.exe（自动发现失败）".to_string())?,
    };
    start_process(&exe, Some(&dll))?;
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(25) {
        if cdp_alive() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(500));
    }
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