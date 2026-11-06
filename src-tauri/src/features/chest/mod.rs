//! 自动开箱引擎（Rust 侧落地实现）
//!
//! 设计要点（全部来自实机逆向与验证，见 AGENTS.md）：
//! 1. 几何来源唯一权威：`GetWindowRgn` + `GetRegionData`（窗口相对坐标，需加窗口原点）；
//! 2. 真实点击接收窗口不是 `Tauri Window`，而是官方单独的 `DesktopCatInput` 顶层窗口；
//! 3. 锚点定位只用 CSS 比例常数 (62/44/46/36)，因此与分辨率、DPI、桌宠位置完全解耦；
//! 4. 就绪判定只能靠像素（区域在就绪/冷却两种状态下完全相同）；
//! 5. 官方进程是以管理员运行的，`PostMessage` 定向投递需要同等完整性，否则 ERROR_ACCESS_DENIED；
//! 6. 资源：稳态每 30 分钟只唤醒约 40 次，每次 < 1 ms，循环内零分配。

use serde::Serialize;
use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};


// ==================== Win32 FFI ====================

type Hwnd = isize;
type Hdc = isize;
type Hbitmap = isize;
type Hgdiobj = isize;
type Hrgn = isize;
type Handle = isize;

const GWL_EXSTYLE: i32 = -20;
const WS_EX_TRANSPARENT: i32 = 0x0000_0020;

const WM_MOUSEMOVE: u32 = 0x0200;
const WM_LBUTTONDOWN: u32 = 0x0201;
const WM_LBUTTONUP: u32 = 0x0202;
const MK_LBUTTON: usize = 0x0001;

const ERROR_ACCESS_DENIED: i32 = 5;
const SRCCOPY: u32 = 0x00CC_0020;
const DIB_RGB_COLORS: u32 = 0;
const TOKEN_QUERY: u32 = 0x0008;
const TOKEN_ELEVATION_CLASS: u32 = 20;

const RGN_ERROR: i32 = 0;
const RGN_NULL: i32 = 1;

// CSS 比例常数（官方 reward-anchor / reward-btn 尺寸，永不随缩放变化）
const ANCHOR_W: f32 = 62.0;
const ANCHOR_H: f32 = 44.0;
const OFF_RIGHT: f32 = 46.0;
const OFF_LEFT: f32 = 36.0;
const TOLERANCE: f32 = 0.15;

/// 开箱成功后的固定睡眠：略早于 30 分钟醒来，再进入 2 秒细扫
const COOLDOWN_SLEEP_MS: u64 = 29 * 60 * 1000 + 45_000;
/// 冷却状态未知剩余时间时的探测间隔
const COOLDOWN_PROBE_MS: u64 = 30_000;
/// 就绪扫描间隔
const READY_SCAN_MS: u64 = 2_000;
/// 遮挡退避上限
const OCCLUDED_BACKOFF_MAX_MS: u64 = 30_000;
/// 提权不足后的冷却
const NEED_ADMIN_BACKOFF_MS: u64 = 60_000;

#[repr(C)]
#[derive(Clone, Copy)]
#[allow(clippy::upper_case_acronyms)]
struct RECT {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

#[repr(C)]
#[allow(clippy::upper_case_acronyms)]
struct RGNDATAHEADER {
    dw_size: u32,
    i_type: u32,
    n_count: u32,
    n_rgn_size: u32,
    rc_bound: RECT,
}

#[repr(C)]
#[allow(clippy::upper_case_acronyms)]
struct BITMAPINFOHEADER {
    bi_size: u32,
    bi_width: i32,
    bi_height: i32,
    bi_planes: u16,
    bi_bit_count: u16,
    bi_compression: u32,
    bi_size_image: u32,
    bi_x_pels: i32,
    bi_y_pels: i32,
    bi_clr_used: u32,
    bi_clr_important: u32,
}

#[repr(C)]
#[allow(clippy::upper_case_acronyms)]
struct BITMAPINFO {
    header: BITMAPINFOHEADER,
    colors: [u32; 3],
}

type WndEnumProc = extern "system" fn(Hwnd, isize) -> i32;

#[link(name = "user32")]
extern "system" {
    fn FindWindowW(class: *const u16, title: *const u16) -> Hwnd;
    fn EnumWindows(proc: WndEnumProc, lparam: isize) -> i32;
    fn GetClassNameW(hwnd: Hwnd, buf: *mut u16, len: i32) -> i32;
    fn GetWindowTextW(hwnd: Hwnd, buf: *mut u16, len: i32) -> i32;
    fn GetWindowRect(hwnd: Hwnd, rect: *mut RECT) -> i32;
    fn GetWindowRgn(hwnd: Hwnd, hrgn: Hrgn) -> i32;
    fn GetWindowLongW(hwnd: Hwnd, index: i32) -> i32;
    fn GetWindowThreadProcessId(hwnd: Hwnd, pid: *mut u32) -> u32;
    fn GetDpiForWindow(hwnd: Hwnd) -> u32;
    fn IsWindowVisible(hwnd: Hwnd) -> i32;
    fn IsIconic(hwnd: Hwnd) -> i32;
    fn PostMessageW(hwnd: Hwnd, msg: u32, wparam: usize, lparam: isize) -> i32;
    fn GetDC(hwnd: Hwnd) -> Hdc;
    fn ReleaseDC(hwnd: Hwnd, hdc: Hdc) -> i32;
    fn GetAsyncKeyState(vkey: i32) -> i16;
    fn SetProcessDpiAwarenessContext(value: isize) -> i32;
}

#[link(name = "gdi32")]
extern "system" {
    fn CreateRectRgn(x1: i32, y1: i32, x2: i32, y2: i32) -> Hrgn;
    fn GetRegionData(hrgn: Hrgn, size: u32, buf: *mut c_void) -> u32;
    fn DeleteObject(obj: Hgdiobj) -> i32;
    fn CreateCompatibleDC(hdc: Hdc) -> Hdc;
    fn CreateCompatibleBitmap(hdc: Hdc, w: i32, h: i32) -> Hbitmap;
    fn SelectObject(hdc: Hdc, obj: Hgdiobj) -> Hgdiobj;
    fn BitBlt(dst: Hdc, x: i32, y: i32, w: i32, h: i32, src: Hdc, sx: i32, sy: i32, rop: u32) -> i32;
    fn GetDIBits(hdc: Hdc, bmp: Hbitmap, start: u32, lines: u32, bits: *mut c_void, bi: *mut BITMAPINFO, usage: u32) -> i32;
    fn DeleteDC(hdc: Hdc) -> i32;
}

#[link(name = "advapi32")]
extern "system" {
    fn OpenProcessToken(process: Handle, access: u32, token: *mut Handle) -> i32;
    fn GetTokenInformation(token: Handle, class: u32, buf: *mut c_void, len: u32, ret_len: *mut u32) -> i32;
}

#[link(name = "kernel32")]
extern "system" {
    fn GetCurrentProcess() -> Handle;
    fn CloseHandle(handle: Handle) -> i32;
}

// ==================== 基础类型 ====================

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    Left,
    Right,
}

impl Place {
    fn as_str(self) -> &'static str {
        match self {
            Place::Left => "left",
            Place::Right => "right",
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Box2 {
    l: i32,
    t: i32,
    r: i32,
    b: i32,
}

impl Box2 {
    fn w(&self) -> i32 {
        self.r - self.l
    }
    fn h(&self) -> i32 {
        self.b - self.t
    }
    fn intersects(&self, o: &Box2) -> bool {
        self.l < o.r && self.r > o.l && self.t < o.b && self.b > o.t
    }
    fn center(&self) -> (i32, i32) {
        ((self.l + self.r) / 2, (self.t + self.b) / 2)
    }
}

#[derive(Clone, Copy, Debug)]
struct Anchor {
    place: Place,
    area: Box2, // 窗口相对坐标
    click: (i32, i32),
    scale: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Readiness {
    Ready,
    Cooldown,
    Unknown,
}

/// UIA 探测到的一个宝箱（坐标已换算成客户区坐标，可直接投递点击）
#[derive(Clone, Copy, Debug)]
struct UiaChest {
    place: Place,
    ready: bool,
    remain: Option<u32>,
    click: (i32, i32),
}

/// UIA 单轮探测结果
enum UiaProbe {
    /// UIA 不可用或调用失败 —— 本轮降级走区域 + 像素路径
    Unavailable,
    /// 读到了界面，但没有宝箱元素（累计点击不足 1000，或面板打开隐藏）
    Empty,
    Hit(Vec<UiaChest>),
}

/// 对外暴露给前端的快照
#[derive(Clone, Serialize)]
pub struct ChestSnapshot {
    pub enabled: bool,
    pub elevated: bool,
    pub state: String,
    pub detail: String,
    pub left: String,
    pub right: String,
    /// 左/右宝箱的「到点时间」（Unix 秒，0 = 未知）。
    /// 前端拿它本地每秒走字，界面就能真的跟着倒计时，
    /// 而不是被后端的长睡眠冻住。
    pub left_ready_at: u64,
    pub right_ready_at: u64,
    pub opens_ok: u32,
    pub opens_failed: u32,
    pub last_event_unix: u64,
}

impl Default for ChestSnapshot {
    fn default() -> Self {
        Self {
            enabled: false,
            elevated: false,
            state: "idle".into(),
            detail: "未启动".into(),
            left: "-".into(),
            right: "-".into(),
            left_ready_at: 0,
            right_ready_at: 0,
            opens_ok: 0,
            opens_failed: 0,
            last_event_unix: 0,
        }
    }
}

// ==================== 窗口与几何 ====================

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn class_name(hwnd: Hwnd) -> String {
    let mut buf = [0u16; 256];
    let n = unsafe { GetClassNameW(hwnd, buf.as_mut_ptr(), 256) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

fn window_title(hwnd: Hwnd) -> String {
    let mut buf = [0u16; 256];
    let n = unsafe { GetWindowTextW(hwnd, buf.as_mut_ptr(), 256) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

fn window_pid(hwnd: Hwnd) -> u32 {
    let mut pid: u32 = 0;
    unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
    pid
}

fn window_rect(hwnd: Hwnd) -> Option<Box2> {
    let mut r = RECT { left: 0, top: 0, right: 0, bottom: 0 };
    if unsafe { GetWindowRect(hwnd, &mut r) } == 0 {
        return None;
    }
    Some(Box2 { l: r.left, t: r.top, r: r.right, b: r.bottom })
}

#[allow(dead_code)]
struct Found {
    input: Hwnd,
    visual: Hwnd,
    pid: u32,
    origin: (i32, i32),
    dpi: u32,
}

struct EnumCtx {
    input: Hwnd,
    visual: Hwnd,
}

mod geometry;
mod pixel;
mod state;

pub use geometry::pet_input_visible;
pub use state::{ensure_dpi_aware, ChestController};
