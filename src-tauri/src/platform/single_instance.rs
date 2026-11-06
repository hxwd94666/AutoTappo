//! Windows 命名互斥：保证同一会话只运行一个 AutoTappo。

const ERROR_ALREADY_EXISTS: u32 = 183;
const MB_OK: u32 = 0x0000_0000;
const MB_ICONINFORMATION: u32 = 0x0000_0040;
const MB_SETFOREGROUND: u32 = 0x0001_0000;

#[link(name = "kernel32")]
extern "system" {
    fn CreateMutexW(
        attrs: *const std::ffi::c_void,
        initial_owner: i32,
        name: *const u16,
    ) -> isize;
    fn GetLastError() -> u32;
    fn CloseHandle(handle: isize) -> i32;
    fn MessageBoxW(
        hwnd: isize,
        text: *const u16,
        caption: *const u16,
        flags: u32,
    ) -> i32;
}

fn wide(value: &str) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    std::ffi::OsStr::new(value)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

/// 只允许一个 AutoTappo 实例。
/// 第二个实例直接退出，避免两个 arrange worker 用不同模式轮流覆盖同一房间。
pub fn acquire(show_message: bool) -> bool {
    let name = wide("Local\\AutoTappo-single-instance");
    let handle = unsafe { CreateMutexW(std::ptr::null(), 0, name.as_ptr()) };
    if handle == 0 {
        // 创建失败时失败开放，避免因权限等异常导致程序完全打不开
        return true;
    }
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        unsafe { CloseHandle(handle) };
        if show_message {
            let text = wide("AutoTappo 已在运行，请从系统托盘打开。");
            let caption = wide("AutoTappo");
            unsafe {
                MessageBoxW(
                    0,
                    text.as_ptr(),
                    caption.as_ptr(),
                    MB_OK | MB_ICONINFORMATION | MB_SETFOREGROUND,
                );
            }
        }
        return false;
    }
    // 成功持有命名互斥量；句柄保持到进程退出，不在这里关闭
    true
}
