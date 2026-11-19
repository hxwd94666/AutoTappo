//! 运行日志：写入 LocalAppData，供界面“打开日志文件夹”排查高级开箱/注入问题。
//!
//! 日志按天分文件，线程安全，写入失败不影响主流程。

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime};

#[repr(C)]
struct SystemTimeLocal {
    year: u16,
    month: u16,
    day_of_week: u16,
    day: u16,
    hour: u16,
    minute: u16,
    second: u16,
    milliseconds: u16,
}

#[link(name = "kernel32")]
extern "system" {
    fn GetLocalTime(time: *mut SystemTimeLocal);
}

struct Logger {
    dir: PathBuf,
    date: String,
    file: File,
}

static LOGGER: OnceLock<Mutex<Logger>> = OnceLock::new();
const KEEP_DAYS: u64 = 14;
const MAX_BYTES: u64 = 10 * 1024 * 1024;

fn local_now() -> SystemTimeLocal {
    let mut value = SystemTimeLocal {
        year: 0,
        month: 0,
        day_of_week: 0,
        day: 0,
        hour: 0,
        minute: 0,
        second: 0,
        milliseconds: 0,
    };
    unsafe { GetLocalTime(&mut value) };
    value
}

fn date_of(t: &SystemTimeLocal) -> String {
    format!("{:04}-{:02}-{:02}", t.year, t.month, t.day)
}

fn timestamp_of(t: &SystemTimeLocal) -> String {
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03}",
        t.year, t.month, t.day, t.hour, t.minute, t.second, t.milliseconds
    )
}

fn log_dir_path() -> PathBuf {
    if let Some(base) = std::env::var_os("LOCALAPPDATA") {
        return PathBuf::from(base).join("AutoTappo").join("logs");
    }
    if let Some(base) = std::env::var_os("APPDATA") {
        return PathBuf::from(base).join("AutoTappo").join("logs");
    }
    std::env::temp_dir().join("AutoTappo").join("logs")
}

fn cleanup_old(dir: &PathBuf) {
    let cutoff = SystemTime::now()
        .checked_sub(Duration::from_secs(KEEP_DAYS * 24 * 60 * 60))
        .unwrap_or(SystemTime::UNIX_EPOCH);
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name.starts_with("autotappo-") || !name.ends_with(".log") {
            continue;
        }
        if let Ok(meta) = entry.metadata() {
            if let Ok(modified) = meta.modified() {
                if modified < cutoff {
                    let _ = fs::remove_file(path);
                }
            }
        }
    }
}

fn open_for_date(dir: &Path, date: &str) -> Option<File> {
    let path = dir.join(format!("autotappo-{date}.log"));
    let file = OpenOptions::new().create(true).append(true).open(&path).ok()?;
    Some(file)
}

/// 初始化日志目录；重复调用无副作用。
pub fn init() {
    let dir = log_dir_path();
    if fs::create_dir_all(&dir).is_err() {
        return;
    }
    cleanup_old(&dir);
    let now = local_now();
    let date = date_of(&now);
    let Some(file) = open_for_date(&dir, &date) else { return };
    let logger = Logger { dir, date, file };
    let _ = LOGGER.set(Mutex::new(logger));
    info(
        "app",
        format!(
            "AutoTappo {} 启动，arch={} os={}",
            env!("CARGO_PKG_VERSION"),
            std::env::consts::ARCH,
            std::env::consts::OS
        ),
    );
}

fn write_line(level: &str, tag: &str, message: &str) {
    let now = local_now();
    let date = date_of(&now);
    let line = format!("[{}] [{}] [{}] {}\n", timestamp_of(&now), level, tag, message);
    let Some(lock) = LOGGER.get() else {
        eprintln!("{}", line.trim_end());
        return;
    };
    let Ok(mut logger) = lock.lock() else { return };
    if logger.date != date {
        if let Some(file) = open_for_date(&logger.dir, &date) {
            logger.file = file;
            logger.date = date;
        }
    }
    if let Ok(meta) = logger.file.metadata() {
        if meta.len() > MAX_BYTES {
            // 达到上限后追加时间后缀，避免无限增长。
            let stamp = format!("{}-{}", logger.date, now.hour * 3600 + now.minute * 60 + now.second);
            if let Some(file) = open_for_date(&logger.dir, &format!("{stamp}-part")) {
                logger.file = file;
            }
        }
    }
    let _ = logger.file.write_all(line.as_bytes());
    let _ = logger.file.flush();
}

pub fn info(tag: &str, message: impl AsRef<str>) {
    write_line("INFO", tag, message.as_ref());
}

pub fn warn(tag: &str, message: impl AsRef<str>) {
    write_line("WARN", tag, message.as_ref());
}

pub fn error(tag: &str, message: impl AsRef<str>) {
    write_line("ERROR", tag, message.as_ref());
}

pub fn debug(tag: &str, message: impl AsRef<str>) {
    write_line("DEBUG", tag, message.as_ref());
}

pub fn dir() -> PathBuf {
    if let Some(lock) = LOGGER.get() {
        if let Ok(logger) = lock.lock() {
            return logger.dir.clone();
        }
    }
    log_dir_path()
}

/// 打开日志文件夹（资源管理器）。不经过 shell 解析，路径由程序自身生成。
pub fn open_dir() -> Result<(), String> {
    let path = dir();
    fs::create_dir_all(&path).map_err(|e| format!("创建日志目录失败: {e}"))?;
    info("logs", format!("打开日志目录: {}", path.display()));
    std::process::Command::new("explorer.exe")
        .arg(&path)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("打开日志目录失败: {e}"))
}
