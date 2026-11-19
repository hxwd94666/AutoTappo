//! Injected into WuwaTappo.exe before its WebView2 is created.
//! Hooks all common process-creation APIs and appends --remote-debugging-port.

use retour::GenericDetour;
use std::ffi::c_void;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use windows::core::{s, PCWSTR, PWSTR};
use windows::Win32::Foundation::{BOOL, HANDLE};
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows::Win32::System::Threading::PROCESS_INFORMATION;

#[link(name = "kernel32")]
extern "system" {
    fn SetEnvironmentVariableW(name: PCWSTR, value: PCWSTR) -> BOOL;
}

type CreateProcessWFn = unsafe extern "system" fn(
    PCWSTR,
    PWSTR,
    *const c_void,
    *const c_void,
    BOOL,
    u32,
    *const c_void,
    PCWSTR,
    *const c_void,
    *mut c_void,
) -> BOOL;

type CreateProcessAsUserWFn = unsafe extern "system" fn(
    HANDLE,
    PCWSTR,
    PWSTR,
    *const c_void,
    *const c_void,
    BOOL,
    u32,
    *const c_void,
    PCWSTR,
    *const c_void,
    *mut c_void,
) -> BOOL;

type CreateProcessWithTokenWFn = unsafe extern "system" fn(
    HANDLE,
    u32,
    PCWSTR,
    PWSTR,
    u32,
    *const c_void,
    PCWSTR,
    *const c_void,
    *mut c_void,
) -> BOOL;

type CreateProcessInternalWFn = unsafe extern "system" fn(
    HANDLE,
    PCWSTR,
    PWSTR,
    *const c_void,
    *const c_void,
    BOOL,
    u32,
    *const c_void,
    PCWSTR,
    *const c_void,
    *mut c_void,
    *mut HANDLE,
) -> BOOL;

type CreateProcessWithLogonWFn = unsafe extern "system" fn(
    PCWSTR,
    PCWSTR,
    PCWSTR,
    u32,
    PCWSTR,
    PWSTR,
    u32,
    *const c_void,
    PCWSTR,
    *const c_void,
    *mut c_void,
) -> BOOL;

static mut ORIGINAL: Option<CreateProcessWFn> = None;
static mut DETOUR: Option<GenericDetour<CreateProcessWFn>> = None;
static mut ORIGINAL_AS_USER: Option<CreateProcessAsUserWFn> = None;
static mut DETOUR_AS_USER: Option<GenericDetour<CreateProcessAsUserWFn>> = None;
static mut ORIGINAL_WITH_TOKEN: Option<CreateProcessWithTokenWFn> = None;
static mut DETOUR_WITH_TOKEN: Option<GenericDetour<CreateProcessWithTokenWFn>> = None;
static mut ORIGINAL_WITH_LOGON: Option<CreateProcessWithLogonWFn> = None;
static mut DETOUR_WITH_LOGON: Option<GenericDetour<CreateProcessWithLogonWFn>> = None;
static mut ORIGINAL_INTERNAL: Option<CreateProcessInternalWFn> = None;
static mut DETOUR_INTERNAL: Option<GenericDetour<CreateProcessInternalWFn>> = None;

fn log_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("AUTOTAPPO_HOOK_LOG_DIR") {
        return PathBuf::from(dir);
    }
    if let Some(base) = std::env::var_os("LOCALAPPDATA") {
        return PathBuf::from(base).join("AutoTappo").join("logs");
    }
    std::env::temp_dir().join("AutoTappo").join("logs")
}

fn log_path() -> PathBuf {
    log_dir().join(format!("hook-{}.log", std::process::id()))
}

fn hook_log(args: std::fmt::Arguments) {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let path = log_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "[{stamp}] {args}");
        let _ = f.flush();
    }
}

unsafe fn read_wide(p: *const u16) -> String {
    if p.is_null() {
        return String::new();
    }
    let mut len = 0usize;
    while *p.add(len) != 0 && len < 32768 {
        len += 1;
    }
    String::from_utf16_lossy(std::slice::from_raw_parts(p, len))
}

unsafe fn prepare(
    app: PCWSTR,
    cmd: PWSTR,
) -> (String, String, bool, bool, Option<Vec<u16>>) {
    let app_s = read_wide(app.0);
    let cmd_s = read_wide(cmd.0);
    let haystack = format!("{app_s} {cmd_s}").to_ascii_lowercase();
    let is_webview = haystack.contains("msedgewebview2");
    let has_port = haystack.contains("--remote-debugging-port");
    let mut owned = None;
    if is_webview && !has_port && !cmd.is_null() {
        let mut v: Vec<u16> = cmd_s.encode_utf16().collect();
        v.extend(" --remote-debugging-port=9223".encode_utf16());
        v.push(0);
        owned = Some(v);
    }
    (app_s, cmd_s, is_webview, has_port, owned)
}

fn child_pid(pi: *mut c_void) -> u32 {
    if pi.is_null() {
        return 0;
    }
    unsafe { (*(pi as *const PROCESS_INFORMATION)).dwProcessId }
}

unsafe extern "system" fn detour(
    app: PCWSTR,
    cmd: PWSTR,
    pa: *const c_void,
    ta: *const c_void,
    inherit: BOOL,
    flags: u32,
    env: *const c_void,
    cwd: PCWSTR,
    si: *const c_void,
    pi: *mut c_void,
) -> BOOL {
    let original = match std::ptr::read(std::ptr::addr_of!(ORIGINAL)) {
        Some(f) => f,
        None => return BOOL(0),
    };
    let (app_s, cmd_s, is_webview, has_port, mut owned) = prepare(app, cmd);
    let append = owned.is_some();
    let cmd_ptr = match owned.as_mut() {
        Some(v) => PWSTR(v.as_mut_ptr()),
        None => cmd,
    };
    hook_log(format_args!(
        "API=CreateProcessW app={:?} cmd={:?} is_webview={} has_port={} append={}",
        app_s, cmd_s, is_webview, has_port, append
    ));
    let result = if let Some(d) = &*std::ptr::addr_of!(DETOUR) {
        d.call(app, cmd_ptr, pa, ta, inherit, flags, env, cwd, si, pi)
    } else {
        original(app, cmd_ptr, pa, ta, inherit, flags, env, cwd, si, pi)
    };
    hook_log(format_args!("API=CreateProcessW result={} child_pid={}", result.0, child_pid(pi)));
    result
}

unsafe extern "system" fn detour_as_user(
    token: HANDLE,
    app: PCWSTR,
    cmd: PWSTR,
    pa: *const c_void,
    ta: *const c_void,
    inherit: BOOL,
    flags: u32,
    env: *const c_void,
    cwd: PCWSTR,
    si: *const c_void,
    pi: *mut c_void,
) -> BOOL {
    let original = match std::ptr::read(std::ptr::addr_of!(ORIGINAL_AS_USER)) {
        Some(f) => f,
        None => return BOOL(0),
    };
    let (app_s, cmd_s, is_webview, has_port, mut owned) = prepare(app, cmd);
    let append = owned.is_some();
    let cmd_ptr = match owned.as_mut() {
        Some(v) => PWSTR(v.as_mut_ptr()),
        None => cmd,
    };
    hook_log(format_args!(
        "API=CreateProcessAsUserW app={:?} cmd={:?} is_webview={} has_port={} append={}",
        app_s, cmd_s, is_webview, has_port, append
    ));
    let result = if let Some(d) = &*std::ptr::addr_of!(DETOUR_AS_USER) {
        d.call(token, app, cmd_ptr, pa, ta, inherit, flags, env, cwd, si, pi)
    } else {
        original(token, app, cmd_ptr, pa, ta, inherit, flags, env, cwd, si, pi)
    };
    hook_log(format_args!(
        "API=CreateProcessAsUserW result={} child_pid={}",
        result.0,
        child_pid(pi)
    ));
    result
}

unsafe extern "system" fn detour_with_token(
    token: HANDLE,
    logon_flags: u32,
    app: PCWSTR,
    cmd: PWSTR,
    creation_flags: u32,
    env: *const c_void,
    cwd: PCWSTR,
    si: *const c_void,
    pi: *mut c_void,
) -> BOOL {
    let original = match std::ptr::read(std::ptr::addr_of!(ORIGINAL_WITH_TOKEN)) {
        Some(f) => f,
        None => return BOOL(0),
    };
    let (app_s, cmd_s, is_webview, has_port, mut owned) = prepare(app, cmd);
    let append = owned.is_some();
    let cmd_ptr = match owned.as_mut() {
        Some(v) => PWSTR(v.as_mut_ptr()),
        None => cmd,
    };
    hook_log(format_args!(
        "API=CreateProcessWithTokenW app={:?} cmd={:?} is_webview={} has_port={} append={}",
        app_s, cmd_s, is_webview, has_port, append
    ));
    let result = if let Some(d) = &*std::ptr::addr_of!(DETOUR_WITH_TOKEN) {
        d.call(token, logon_flags, app, cmd_ptr, creation_flags, env, cwd, si, pi)
    } else {
        original(token, logon_flags, app, cmd_ptr, creation_flags, env, cwd, si, pi)
    };
    hook_log(format_args!(
        "API=CreateProcessWithTokenW result={} child_pid={}",
        result.0,
        child_pid(pi)
    ));
    result
}

unsafe extern "system" fn detour_with_logon(
    user: PCWSTR,
    domain: PCWSTR,
    password: PCWSTR,
    logon_flags: u32,
    app: PCWSTR,
    cmd: PWSTR,
    creation_flags: u32,
    env: *const c_void,
    cwd: PCWSTR,
    si: *const c_void,
    pi: *mut c_void,
) -> BOOL {
    let original = match std::ptr::read(std::ptr::addr_of!(ORIGINAL_WITH_LOGON)) {
        Some(f) => f,
        None => return BOOL(0),
    };
    let (app_s, cmd_s, is_webview, has_port, mut owned) = prepare(app, cmd);
    let append = owned.is_some();
    let cmd_ptr = match owned.as_mut() {
        Some(v) => PWSTR(v.as_mut_ptr()),
        None => cmd,
    };
    hook_log(format_args!(
        "API=CreateProcessWithLogonW app={:?} cmd={:?} is_webview={} has_port={} append={}",
        app_s, cmd_s, is_webview, has_port, append
    ));
    let result = if let Some(d) = &*std::ptr::addr_of!(DETOUR_WITH_LOGON) {
        d.call(user, domain, password, logon_flags, app, cmd_ptr, creation_flags, env, cwd, si, pi)
    } else {
        original(user, domain, password, logon_flags, app, cmd_ptr, creation_flags, env, cwd, si, pi)
    };
    hook_log(format_args!(
        "API=CreateProcessWithLogonW result={} child_pid={}",
        result.0,
        child_pid(pi)
    ));
    result
}


unsafe extern "system" fn detour_internal(
    token: HANDLE,
    app: PCWSTR,
    cmd: PWSTR,
    pa: *const c_void,
    ta: *const c_void,
    inherit: BOOL,
    flags: u32,
    env: *const c_void,
    cwd: PCWSTR,
    si: *const c_void,
    pi: *mut c_void,
    new_token: *mut HANDLE,
) -> BOOL {
    let original = match std::ptr::read(std::ptr::addr_of!(ORIGINAL_INTERNAL)) {
        Some(f) => f,
        None => return BOOL(0),
    };
    let (app_s, cmd_s, is_webview, has_port, mut owned) = prepare(app, cmd);
    let append = owned.is_some();
    let cmd_ptr = match owned.as_mut() {
        Some(v) => PWSTR(v.as_mut_ptr()),
        None => cmd,
    };
    hook_log(format_args!(
        "API=CreateProcessInternalW app={:?} cmd={:?} is_webview={} has_port={} append={}",
        app_s, cmd_s, is_webview, has_port, append
    ));
    let result = if let Some(d) = &*std::ptr::addr_of!(DETOUR_INTERNAL) {
        d.call(
            token, app, cmd_ptr, pa, ta, inherit, flags, env, cwd, si, pi, new_token,
        )
    } else {
        original(
            token, app, cmd_ptr, pa, ta, inherit, flags, env, cwd, si, pi, new_token,
        )
    };
    hook_log(format_args!(
        "API=CreateProcessInternalW result={} child_pid={}",
        result.0,
        child_pid(pi)
    ));
    result
}
unsafe fn install() {
    let kernel = match GetModuleHandleW(windows::core::w!("kernel32.dll")) {
        Ok(h) => h,
        Err(e) => {
            hook_log(format_args!("GetModuleHandleW(kernel32) failed: {e:?}"));
            return;
        }
    };
    if let Some(addr) = GetProcAddress(kernel, s!("CreateProcessW")) {
        let original: CreateProcessWFn = std::mem::transmute(addr);
        match GenericDetour::new(original, detour) {
            Ok(d) => match d.enable() {
                Ok(()) => {
                    ORIGINAL = Some(original);
                    DETOUR = Some(d);
                    hook_log(format_args!("install CreateProcessW success"));
                }
                Err(e) => hook_log(format_args!("enable CreateProcessW failed: {e:?}")),
            },
            Err(e) => hook_log(format_args!("new CreateProcessW failed: {e:?}")),
        }
    } else {
        hook_log(format_args!("GetProcAddress(CreateProcessW) failed"));
    }

    let advapi = match GetModuleHandleW(windows::core::w!("advapi32.dll")) {
        Ok(h) => h,
        Err(e) => {
            hook_log(format_args!("GetModuleHandleW(advapi32) failed: {e:?}"));
            return;
        }
    };
    if let Some(addr) = GetProcAddress(advapi, s!("CreateProcessAsUserW")) {
        let original: CreateProcessAsUserWFn = std::mem::transmute(addr);
        match GenericDetour::new(original, detour_as_user) {
            Ok(d) => match d.enable() {
                Ok(()) => {
                    ORIGINAL_AS_USER = Some(original);
                    DETOUR_AS_USER = Some(d);
                    hook_log(format_args!("install CreateProcessAsUserW success"));
                }
                Err(e) => hook_log(format_args!("enable CreateProcessAsUserW failed: {e:?}")),
            },
            Err(e) => hook_log(format_args!("new CreateProcessAsUserW failed: {e:?}")),
        }
    }
    if let Some(addr) = GetProcAddress(advapi, s!("CreateProcessWithTokenW")) {
        let original: CreateProcessWithTokenWFn = std::mem::transmute(addr);
        match GenericDetour::new(original, detour_with_token) {
            Ok(d) => match d.enable() {
                Ok(()) => {
                    ORIGINAL_WITH_TOKEN = Some(original);
                    DETOUR_WITH_TOKEN = Some(d);
                    hook_log(format_args!("install CreateProcessWithTokenW success"));
                }
                Err(e) => hook_log(format_args!("enable CreateProcessWithTokenW failed: {e:?}")),
            },
            Err(e) => hook_log(format_args!("new CreateProcessWithTokenW failed: {e:?}")),
        }
    }
    if let Some(addr) = GetProcAddress(advapi, s!("CreateProcessWithLogonW")) {
        let original: CreateProcessWithLogonWFn = std::mem::transmute(addr);
        match GenericDetour::new(original, detour_with_logon) {
            Ok(d) => match d.enable() {
                Ok(()) => {
                    ORIGINAL_WITH_LOGON = Some(original);
                    DETOUR_WITH_LOGON = Some(d);
                    hook_log(format_args!("install CreateProcessWithLogonW success"));
                }
                Err(e) => hook_log(format_args!("enable CreateProcessWithLogonW failed: {e:?}")),
            },
            Err(e) => hook_log(format_args!("new CreateProcessWithLogonW failed: {e:?}")),
        }
    }

    let kernelbase = match GetModuleHandleW(windows::core::w!("kernelbase.dll")) {
        Ok(h) => h,
        Err(e) => {
            hook_log(format_args!("GetModuleHandleW(kernelbase) failed: {e:?}"));
            return;
        }
    };
    if let Some(addr) = GetProcAddress(kernelbase, s!("CreateProcessInternalW")) {
        let original: CreateProcessInternalWFn = std::mem::transmute(addr);
        match GenericDetour::new(original, detour_internal) {
            Ok(d) => match d.enable() {
                Ok(()) => {
                    ORIGINAL_INTERNAL = Some(original);
                    DETOUR_INTERNAL = Some(d);
                    hook_log(format_args!("install CreateProcessInternalW success"));
                }
                Err(e) => hook_log(format_args!("enable CreateProcessInternalW failed: {e:?}")),
            },
            Err(e) => hook_log(format_args!("new CreateProcessInternalW failed: {e:?}")),
        }
    } else {
        hook_log(format_args!("GetProcAddress(CreateProcessInternalW) failed"));
    }
}

#[no_mangle]
pub extern "system" fn DllMain(_inst: *mut c_void, reason: u32, _reserved: *mut c_void) -> i32 {
    if reason == 1 {
        hook_log(format_args!(
            "DllMain attach pid={} path={}",
            std::process::id(),
            log_path().display()
        ));
        let env_ok = unsafe {
            SetEnvironmentVariableW(
                windows::core::w!("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS"),
                windows::core::w!("--remote-debugging-port=9223"),
            )
        };
        hook_log(format_args!("SetEnvironmentVariable result={}", env_ok.0));
        unsafe { install() };
        hook_log(format_args!("install finished"));
    }
    1
}
