//! Injected into WuwaTappo.exe before its WebView2 is created.
//! Hooks CreateProcessW and appends --remote-debugging-port to msedgewebview2.exe.

use retour::GenericDetour;
use std::ffi::c_void;
use windows::core::{s, PCWSTR, PWSTR};
use windows::Win32::Foundation::BOOL;
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};

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

static mut ORIGINAL: Option<CreateProcessWFn> = None;
static mut DETOUR: Option<GenericDetour<CreateProcessWFn>> = None;

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

    let mut owned: Option<Vec<u16>> = None;
    if !cmd.is_null() {
        let original_cmd = read_wide(cmd.0);
        let lower = original_cmd.to_ascii_lowercase();
        if lower.contains("msedgewebview2") && !lower.contains("--remote-debugging-port") {
            let mut v: Vec<u16> = original_cmd.encode_utf16().collect();
            v.extend(" --remote-debugging-port=9223".encode_utf16());
            v.push(0);
            owned = Some(v);
        }
    }

    let cmd_ptr = match owned.as_mut() {
        Some(v) => PWSTR(v.as_mut_ptr()),
        None => cmd,
    };

    if let Some(d) = &*std::ptr::addr_of!(DETOUR) {
        d.call(app, cmd_ptr, pa, ta, inherit, flags, env, cwd, si, pi)
    } else {
        original(app, cmd_ptr, pa, ta, inherit, flags, env, cwd, si, pi)
    }
}

unsafe fn install() {
    let kernel = match GetModuleHandleW(windows::core::w!("kernel32.dll")) {
        Ok(h) => h,
        Err(_) => return,
    };
    let addr = match GetProcAddress(kernel, s!("CreateProcessW")) {
        Some(a) => a,
        None => return,
    };
    let original: CreateProcessWFn = std::mem::transmute(addr);
    if let Ok(d) = GenericDetour::new(original, detour) {
        if d.enable().is_ok() {
            ORIGINAL = Some(original);
            DETOUR = Some(d);
        }
    }
}

#[no_mangle]
pub extern "system" fn DllMain(_inst: *mut c_void, reason: u32, _reserved: *mut c_void) -> i32 {
    if reason == 1 {
        unsafe { install() };
    }
    1
}
