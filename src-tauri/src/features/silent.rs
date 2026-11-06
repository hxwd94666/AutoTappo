//! 静默刷数：只改官方本地计数缓存，不发送任何键盘事件。
//!
//! 已实测的官方链路（2026-11-05）：
//!   1. 真实按键抬起 -> Rust 后端把内存 localClickCount 写入
//!      %APPDATA%\com.kuro.wuwatappo.cn\key_press_count_cn-cn-prod.dat
//!      （循环 XOR，密钥 kuro-mc-keypress-store-v1，明文 {"t":N,"environment":"cn-cn-prod"}）
//!   2. 每约 30 秒 -> 前端 un() 读取该文件，与内存 localClickCount 取 max，
//!      算出 T = 服务端上次总数 + 本地增量，通过 WebSocket 发 "cr" {num:T}
//!   3. 上报成功 -> 写 cumulative_click_cn-cn-prod.dat，并把 key_press 清零
//!
//! 所以只要在官方清零后写入目标增量，下一次同步就会静默加上去；
//! 整个过程中没有任何 key-press 事件，桌宠不会播放手部打字动画。
//!
//! 预算（每 30 秒最多决策一次）：
//!   最近 30 分钟已注入 < 2000 -> 按缺口补刷（单次 40~90）
//!   已注入 >= 2000            -> 随机跳过，偶尔小补，上限 2150
//!   KP==0 && injected>0       -> 官方同步消费完毕，进入下一次决策
//!   KP<injected               -> 官方 keyup 覆盖，合并真实增量
//!   KP>injected               -> un() 已读取，等待同步，不再叠加
//!
//! 真实按键完全走官方链路，所以动画、联机同步、计数都不受影响。

use rand::Rng;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// 官方 key_press 缓存的 XOR 密钥（从 exe 字符串提取）
const XOR_KEY: &[u8] = b"kuro-mc-keypress-store-v1";
const ENV_NAME: &str = "cn-cn-prod";
const STORE_DIR: &str = "com.kuro.wuwatappo.cn";
const STORE_FILE: &str = "key_press_count_cn-cn-prod.dat";

const POLL_MS: u64 = 200;
/// 官方 30 秒同步完成后的写入延迟
const INJECT_DELAY_MIN_MS: u64 = 800;
const INJECT_DELAY_MAX_MS: u64 = 2600;
const MERGE_DELAY_MIN_MS: u64 = 400;
const MERGE_DELAY_MAX_MS: u64 = 1200;

/// 30 分钟预算窗口；目标 2000（两个宝箱），上限 2150。
const WINDOW_MS: u64 = 30 * 60 * 1000;
const TARGET_MIN: u64 = 2000;
const TARGET_MAX: u64 = 2150;
const MIN_INJECT: u64 = 40;
const MAX_INJECT: u64 = 90;
pub const MAX_MANUAL_ADD: u64 = 1_000_000;

/// 静默注入预算：按“最近 30 分钟已写入量”决定本轮是否注入、注入多少。
/// 时间全部用毫秒，便于单元测试模拟 60 个同步点。
struct Budget {
    window_start_ms: u64,
    events: VecDeque<(u64, u64)>,
}

impl Budget {
    fn new(start_ms: u64) -> Self {
        Self {
            window_start_ms: start_ms,
            events: VecDeque::new(),
        }
    }

    fn reset(&mut self, start_ms: u64) {
        self.window_start_ms = start_ms;
        self.events.clear();
    }

    fn prune(&mut self, now_ms: u64) {
        while let Some(&(at, _)) = self.events.front() {
            if at + WINDOW_MS <= now_ms {
                self.events.pop_front();
            } else {
                break;
            }
        }
    }

    fn recent(&mut self, now_ms: u64) -> u64 {
        self.prune(now_ms);
        self.events.iter().map(|(_, n)| *n).sum()
    }

    /// 返回本轮注入量；None 表示本轮跳过。
    fn decide(&mut self, now_ms: u64, rng: &mut impl Rng) -> Option<u64> {
        let recent = self.recent(now_ms);
        if recent >= TARGET_MAX {
            return None;
        }
        let remaining = TARGET_MAX - recent;
        let elapsed = now_ms.saturating_sub(self.window_start_ms).min(WINDOW_MS);
        let expected = TARGET_MIN.saturating_mul(elapsed) / WINDOW_MS;
        let amount = if recent < expected {
            let deficit = expected - recent;
            (deficit + rng.gen_range(10..=30)).clamp(MIN_INJECT, MAX_INJECT)
        } else if rng.gen_bool(0.70) {
            return None;
        } else {
            rng.gen_range(35..=60)
        };
        let amount = amount.min(remaining);
        if amount == 0 { None } else { Some(amount) }
    }

    fn record(&mut self, now_ms: u64, amount: u64) {
        self.events.push_back((now_ms, amount));
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct KeyPressStore {
    t: u64,
    environment: String,
}

#[derive(Clone, Serialize)]
pub struct SilentSnapshot {
    /// idle | waiting | injected | no-file | error
    pub state: String,
    pub detail: String,
    pub last_amount: u64,
    pub total: u64,
}

impl Default for SilentSnapshot {
    fn default() -> Self {
        Self {
            state: "idle".to_string(),
            detail: "静默模式在锁屏/隐藏桌宠时仍生效，不影响桌宠动画".to_string(),
            last_amount: 0,
            total: 0,
        }
    }
}

pub struct SilentController {
    pub running: Arc<AtomicBool>,
    pub shutdown: Arc<AtomicBool>,
    pub total: Arc<AtomicU64>,
    manual: Arc<AtomicU64>,
    status: Arc<Mutex<SilentSnapshot>>,
}

impl SilentController {
    pub fn new() -> Self {
        Self {
            running: Arc::new(AtomicBool::new(false)),
            shutdown: Arc::new(AtomicBool::new(false)),
            total: Arc::new(AtomicU64::new(0)),
            manual: Arc::new(AtomicU64::new(0)),
            status: Arc::new(Mutex::new(SilentSnapshot::default())),
        }
    }

    /// 提交一次性加数任务，由静默 worker 串行写文件。
    pub fn queue_manual_add(&self, amount: u64) -> Result<u64, String> {
        if amount == 0 {
            return Err("加数数量必须大于 0".into());
        }
        if amount > MAX_MANUAL_ADD {
            return Err(format!("一次最多加 {MAX_MANUAL_ADD} 次"));
        }
        self.manual.fetch_add(amount, Ordering::SeqCst);
        Ok(amount)
    }

    pub fn snapshot(&self) -> SilentSnapshot {
        self.status
            .lock()
            .map(|s| s.clone())
            .unwrap_or_default()
    }

    pub fn start_worker(&self) {
        let running = Arc::clone(&self.running);
        let shutdown = Arc::clone(&self.shutdown);
        let total = Arc::clone(&self.total);
        let manual = Arc::clone(&self.manual);
        let status = Arc::clone(&self.status);

        thread::spawn(move || {
            let path = match store_path() {
                Some(p) => p,
                None => {
                    set_status(&status, "error", "找不到 %APPDATA% 路径", 0, &total);
                    return;
                }
            };

            // round_n: 本轮计划注入量；injected: 本轮已写入文件的值
            let mut round_n: u64 = 0;
            let mut injected: u64 = 0;
            let mut round_counted = false;
            let mut pending_at: Option<Instant> = None;
            let run_start = Instant::now();
            let mut budget = Budget::new(0);
            let mut next_tick = Instant::now() + Duration::from_secs(30);

            while !shutdown.load(Ordering::Relaxed) {
                let now = Instant::now();
                let now_ms = run_start.elapsed().as_millis() as u64;

                // 一键加数：即使静默任务暂停也允许执行，一次只写一笔并等待官方同步。
                let manual_add = manual.swap(0, Ordering::SeqCst);
                if manual_add > 0 {
                    if !path.exists() {
                        manual.fetch_add(manual_add, Ordering::SeqCst);
                        set_status(
                            &status,
                            "no-file",
                            "未检测到桌宠计数缓存，请先启动 WuwaTappo",
                            0,
                            &total,
                        );
                        thread::sleep(Duration::from_millis(1000));
                        continue;
                    }
                    match read_value(&path) {
                        Ok(current) => {
                            if let Err(e) = ensure_backup(&path) {
                                manual.fetch_add(manual_add, Ordering::SeqCst);
                                set_status(&status, "error", &e, 0, &total);
                                thread::sleep(Duration::from_millis(1000));
                                continue;
                            }
                            match write_value(&path, current.saturating_add(manual_add)) {
                                Ok(()) => {
                                    total.fetch_add(manual_add, Ordering::SeqCst);
                                    set_status(
                                        &status,
                                        "injected",
                                        &format!("一键加数 {manual_add} 次，等待官方同步"),
                                        manual_add,
                                        &total,
                                    );
                                }
                                Err(e) => {
                                    manual.fetch_add(manual_add, Ordering::SeqCst);
                                    set_status(&status, "error", &e, 0, &total);
                                    thread::sleep(Duration::from_millis(1000));
                                    continue;
                                }
                            }
                        }
                        Err(e) => {
                            manual.fetch_add(manual_add, Ordering::SeqCst);
                            set_status(&status, "error", &e, 0, &total);
                            thread::sleep(Duration::from_millis(1000));
                            continue;
                        }
                    }
                    thread::sleep(Duration::from_millis(POLL_MS));
                    continue;
                }

                if !running.load(Ordering::Relaxed) {
                    round_n = 0;
                    injected = 0;
                    round_counted = false;
                    pending_at = None;
                    budget.reset(now_ms);
                    next_tick = now + Duration::from_secs(30);
                    set_status(&status, "idle", "静默模式在锁屏/隐藏桌宠时仍生效，不影响桌宠动画", 0, &total);
                    thread::sleep(Duration::from_millis(250));
                    continue;
                }

                if !path.exists() {
                    set_status(
                        &status,
                        "no-file",
                        "未检测到桌宠计数缓存，请先启动 WuwaTappo",
                        0,
                        &total,
                    );
                    thread::sleep(Duration::from_millis(1000));
                    continue;
                }

                let kp = match read_value(&path) {
                    Ok(v) => v,
                    Err(e) => {
                        set_status(&status, "error", &e, 0, &total);
                        thread::sleep(Duration::from_millis(1000));
                        continue;
                    }
                };

                // 官方同步成功后会把 KP 清零：上一轮已被消费，重置状态。
                if kp == 0 && injected > 0 {
                    round_n = 0;
                    injected = 0;
                    round_counted = false;
                    pending_at = None;
                    next_tick = now + random_inject_delay();
                }

                // 写入窗口到点
                if let Some(at) = pending_at {
                    if now < at {
                        set_status(&status, "waiting", "等待随机写入窗口…", 0, &total);
                        thread::sleep(Duration::from_millis(POLL_MS));
                        continue;
                    }

                    // 延迟期间用户可能真实按键，重新读一次并以最新值为基准合并
                    let current = read_value(&path).unwrap_or(kp);

                    // 如果 un() 已经读过我们的注入，KP 会 >= injected，
                    // 此时绝不能再次叠加，否则会重复计数。
                    if injected > 0 && current >= injected {
                        pending_at = None;
                        next_tick = now + Duration::from_secs(30);
                        set_status(&status, "waiting", "已并入官方增量，等待同步…", 0, &total);
                        thread::sleep(Duration::from_millis(POLL_MS));
                        continue;
                    }

                    if round_n == 0 {
                        pending_at = None;
                        next_tick = now + Duration::from_secs(30);
                        continue;
                    }
                    let target = round_n + current;
                    if let Err(e) = ensure_backup(&path) {
                        set_status(&status, "error", &e, 0, &total);
                        pending_at = None;
                        next_tick = now + Duration::from_secs(30);
                        thread::sleep(Duration::from_millis(1000));
                        continue;
                    }
                    match write_value(&path, target) {
                        Ok(()) => {
                            injected = target;
                            if !round_counted {
                                total.fetch_add(round_n, Ordering::Relaxed);
                                budget.record(now_ms, round_n);
                                round_counted = true;
                            }
                            let detail =
                                format!("本轮写入 {} 次（含真实按键 {}），等待同步", round_n, current);
                            set_status(&status, "injected", &detail, round_n, &total);
                            pending_at = None;
                            next_tick = now + Duration::from_secs(30);
                        }
                        Err(e) => {
                            set_status(&status, "error", &e, 0, &total);
                            pending_at = None;
                            next_tick = now + Duration::from_secs(30);
                            thread::sleep(Duration::from_millis(1000));
                        }
                    }
                    thread::sleep(Duration::from_millis(POLL_MS));
                    continue;
                }

                // 每 ~30 秒最多做一次预算决策；跳过时不要立刻重新调度。
                if kp == 0 && now >= next_tick {
                    let mut rng = rand::thread_rng();
                    match budget.decide(now_ms, &mut rng) {
                        Some(amount) => {
                            round_n = amount;
                            round_counted = false;
                            pending_at = Some(now + random_inject_delay());
                            let detail = format!("本轮预算 {} 次，准备写入…", amount);
                            set_status(&status, "waiting", &detail, 0, &total);
                        }
                        None => {
                            next_tick = now + Duration::from_secs(30);
                            set_status(&status, "waiting", "本轮跳过，等待下一随机窗口…", 0, &total);
                        }
                    }
                    thread::sleep(Duration::from_millis(POLL_MS));
                    continue;
                }

                if injected == 0 {
                    if kp > 0 {
                        set_status(&status, "waiting", "检测到真实按键，等待随机注入窗口…", 0, &total);
                    } else {
                        set_status(&status, "waiting", "等待下一次随机注入窗口…", 0, &total);
                    }
                } else if kp < injected {
                    // 官方 keyup 用纯真实增量覆盖了文件：保留本轮注入并合并最新真实增量
                    set_status(&status, "waiting", "真实按键覆盖，准备重新合并…", 0, &total);
                    pending_at = Some(now + random_merge_delay());
                } else if kp == injected {
                    set_status(&status, "injected", "本轮已写入，等待下一次同步", round_n, &total);
                } else {
                    // kp > injected：un() 已读取并合并，等待同步
                    set_status(&status, "waiting", "已并入官方增量，等待同步…", 0, &total);
                }

                thread::sleep(Duration::from_millis(POLL_MS));
            }

            set_status(&status, "idle", "静默刷数已停止", 0, &total);
        });
    }
}

fn set_status(
    status: &Arc<Mutex<SilentSnapshot>>,
    state: &str,
    detail: &str,
    last_amount: u64,
    total: &Arc<AtomicU64>,
) {
    if let Ok(mut s) = status.lock() {
        s.state = state.to_string();
        s.detail = detail.to_string();
        s.last_amount = last_amount;
        s.total = total.load(Ordering::Relaxed);
    }
}

fn store_path() -> Option<PathBuf> {
    let appdata = std::env::var_os("APPDATA")?;
    Some(
        PathBuf::from(appdata)
            .join(STORE_DIR)
            .join(STORE_FILE),
    )
}

fn backup_path(path: &Path) -> PathBuf {
    path.with_extension("dat.autotappo.bak")
}

fn xor(data: &[u8], key: &[u8]) -> Vec<u8> {
    data.iter()
        .enumerate()
        .map(|(i, b)| b ^ key[i % key.len()])
        .collect()
}

fn read_value(path: &Path) -> Result<u64, String> {
    let bytes = fs::read(path).map_err(|e| format!("读取 key_press 缓存失败：{e}"))?;
    let plain = xor(&bytes, XOR_KEY);
    let store: KeyPressStore =
        serde_json::from_slice(&plain).map_err(|e| format!("解析 key_press 缓存失败：{e}"))?;
    if store.environment != ENV_NAME {
        return Err(format!(
            "环境不匹配（文件为 {}，期望 {}）",
            store.environment, ENV_NAME
        ));
    }
    Ok(store.t)
}

fn write_value(path: &Path, value: u64) -> Result<(), String> {
    let store = KeyPressStore {
        t: value,
        environment: ENV_NAME.to_string(),
    };
    let plain = serde_json::to_vec(&store).map_err(|e| format!("序列化失败：{e}"))?;
    let encoded = xor(&plain, XOR_KEY);

    // 先写临时文件再原子替换，避免官方读到半截 JSON。
    let tmp = path.with_extension("dat.autotappo.tmp");
    fs::write(&tmp, &encoded).map_err(|e| format!("写临时文件失败：{e}"))?;

    let mut last_err = String::new();
    for attempt in 0..5 {
        match fs::rename(&tmp, path) {
            Ok(()) => return Ok(()),
            Err(e) => {
                last_err = e.to_string();
                thread::sleep(Duration::from_millis(60 * (attempt + 1)));
            }
        }
    }
    let _ = fs::remove_file(&tmp);
    Err(format!("替换 key_press 缓存失败：{last_err}"))
}

/// 首次写入前备份官方原始文件；备份失败就拒绝写入，避免把用户数据写坏。
fn ensure_backup(path: &Path) -> Result<(), String> {
    let bak = backup_path(path);
    if bak.exists() || !path.exists() {
        return Ok(());
    }
    fs::copy(path, &bak).map_err(|e| format!("备份 key_press 缓存失败：{e}"))?;
    Ok(())
}

fn random_inject_delay() -> Duration {
    let ms = rand::thread_rng().gen_range(INJECT_DELAY_MIN_MS..=INJECT_DELAY_MAX_MS);
    Duration::from_millis(ms)
}

fn random_merge_delay() -> Duration {
    let ms = rand::thread_rng().gen_range(MERGE_DELAY_MIN_MS..=MERGE_DELAY_MAX_MS);
    Duration::from_millis(ms)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xor_roundtrip() {
        let plain = br#"{"t":7,"environment":"cn-cn-prod"}"#;
        let encoded = xor(plain, XOR_KEY);
        assert_ne!(encoded.as_slice(), plain);
        assert_eq!(xor(&encoded, XOR_KEY).as_slice(), plain);
    }

    #[test]
    fn json_shape_matches_official() {
        let store = KeyPressStore {
            t: 100,
            environment: ENV_NAME.to_string(),
        };
        let plain = serde_json::to_vec(&store).unwrap();
        assert_eq!(
            plain.as_slice(),
            br#"{"t":100,"environment":"cn-cn-prod"}"#.as_slice()
        );
        let back: KeyPressStore = serde_json::from_slice(&plain).unwrap();
        assert_eq!(back.t, 100);
        assert_eq!(back.environment, ENV_NAME);
    }

    #[test]
    fn manual_add_validates_range() {
        let c = SilentController::new();
        assert!(c.queue_manual_add(0).is_err());
        assert!(c.queue_manual_add(MAX_MANUAL_ADD + 1).is_err());
        assert_eq!(c.queue_manual_add(1).unwrap(), 1);
        assert_eq!(c.queue_manual_add(MAX_MANUAL_ADD).unwrap(), MAX_MANUAL_ADD);
        assert_eq!(c.manual.load(Ordering::SeqCst), MAX_MANUAL_ADD + 1);
    }

    #[test]
    fn budget_hits_30min_target() {
        use rand::rngs::StdRng;
        use rand::SeedableRng;
        let mut budget = Budget::new(0);
        let mut rng = StdRng::seed_from_u64(2026);
        let mut total = 0;
        let mut injections = 0;
        for slot in 1..=60 {
            let now = slot * 30_000;
            if let Some(amount) = budget.decide(now, &mut rng) {
                total += amount;
                injections += 1;
                budget.record(now, amount);
            }
            assert!(budget.recent(now) <= TARGET_MAX, "30 分钟窗口超上限");
        }
        assert!((2000..=2150).contains(&total), "30 分钟总量={total}");
        assert!((20..60).contains(&injections), "实际注入次数={injections}");
    }

    #[test]
    fn budget_second_window_is_stable() {
        use rand::rngs::StdRng;
        use rand::SeedableRng;
        let mut budget = Budget::new(0);
        let mut rng = StdRng::seed_from_u64(99);
        let mut second = 0;
        for slot in 1..=120 {
            let now = slot * 30_000;
            if let Some(amount) = budget.decide(now, &mut rng) {
                if slot > 60 {
                    second += amount;
                }
                budget.record(now, amount);
            }
            assert!(budget.recent(now) <= TARGET_MAX, "窗口超上限 slot={slot}");
        }
        assert!((2000..=2150).contains(&second), "第二个 30 分钟={second}");
    }

    #[test]
    fn file_roundtrip_and_atomic_replace() {
        let dir = std::env::temp_dir().join(format!(
            "autotappo_silent_test_{}_{}",
            std::process::id(),
            Instant::now().elapsed().as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(STORE_FILE);

        let plain = serde_json::to_vec(&KeyPressStore {
            t: 42,
            environment: ENV_NAME.to_string(),
        })
        .unwrap();
        fs::write(&path, xor(&plain, XOR_KEY)).unwrap();
        assert_eq!(read_value(&path).unwrap(), 42);

        write_value(&path, 1234).unwrap();
        assert_eq!(read_value(&path).unwrap(), 1234);

        // 模拟同步后官方清零
        write_value(&path, 0).unwrap();
        assert_eq!(read_value(&path).unwrap(), 0);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn backup_is_created_once() {
        let dir = std::env::temp_dir().join(format!(
            "autotappo_silent_bak_{}_{}",
            std::process::id(),
            Instant::now().elapsed().as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(STORE_FILE);
        fs::write(&path, b"seed").unwrap();

        ensure_backup(&path).unwrap();
        let bak = backup_path(&path);
        assert_eq!(fs::read(&bak).unwrap(), b"seed");

        // 第二次不覆盖已有备份
        fs::write(&path, b"changed").unwrap();
        ensure_backup(&path).unwrap();
        assert_eq!(fs::read(&bak).unwrap(), b"seed");

        let _ = fs::remove_dir_all(&dir);
    }
}
