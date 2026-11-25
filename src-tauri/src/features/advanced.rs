//! 高级开箱：DLL 注入 + CDP。
//!
//! 默认全部关闭。只有用户显式打开「高级开箱」后才会：
//!   1. 结束并按带调试端口的方式重启一次 WuwaTappo；
//!   2. 注入 autotappo_hook.dll；
//!   3. 通过 CDP 调用官方 openChest。
//!
//! 「锁屏领取」「隐藏领取」是两个独立子开关，默认关闭。
//! 不开启时 AutoTappo 不会注入、不会重启桌宠，正常用户完全无感。

use serde::Serialize;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::features::cdp::Cdp;
use crate::features::injector;
use crate::logging;
use crate::{features::chest, platform::session};

#[derive(Clone, Serialize)]
pub struct AdvancedSnapshot {
    pub enabled: bool,
    pub lock_screen: bool,
    pub hidden: bool,
    pub state: String,
    pub detail: String,
    pub cdp: bool,
    pub chest_fn: bool,
}

impl Default for AdvancedSnapshot {
    fn default() -> Self {
        Self {
            enabled: false,
            lock_screen: false,
            hidden: false,
            state: "idle".to_string(),
            detail: "高级开箱已关闭".to_string(),
            cdp: false,
            chest_fn: false,
        }
    }
}

pub struct AdvancedController {
    pub enabled: Arc<AtomicBool>,
    pub lock_screen: Arc<AtomicBool>,
    pub hidden: Arc<AtomicBool>,
    /// 自动恢复好友桌宠布局（依赖高级开箱的 CDP）。
    pub shutdown: Arc<AtomicBool>,
    status: Arc<Mutex<AdvancedSnapshot>>,
    chest_enabled: Arc<AtomicBool>,
    left_cooldown: Arc<AtomicU64>,
    right_cooldown: Arc<AtomicU64>,
}

impl AdvancedController {
    pub fn new(
        chest_enabled: Arc<AtomicBool>,
        left_cooldown: Arc<AtomicU64>,
        right_cooldown: Arc<AtomicU64>,
    ) -> Self {
        Self {
            enabled: Arc::new(AtomicBool::new(false)),
            lock_screen: Arc::new(AtomicBool::new(false)),
            hidden: Arc::new(AtomicBool::new(false)),
            shutdown: Arc::new(AtomicBool::new(false)),
            status: Arc::new(Mutex::new(AdvancedSnapshot::default())),
            chest_enabled,
            left_cooldown,
            right_cooldown,
        }
    }

    pub fn snapshot(&self) -> AdvancedSnapshot {
        let mut s = self.status.lock().map(|v| v.clone()).unwrap_or_default();
        s.enabled = self.enabled.load(Ordering::Relaxed);
        s.lock_screen = self.lock_screen.load(Ordering::Relaxed);
        s.hidden = self.hidden.load(Ordering::Relaxed);
        s
    }

    pub fn start_worker(&self) {
        let enabled = Arc::clone(&self.enabled);
        let lock_screen = Arc::clone(&self.lock_screen);
        let hidden = Arc::clone(&self.hidden);
        let shutdown = Arc::clone(&self.shutdown);
        let status = Arc::clone(&self.status);
        let chest_enabled = Arc::clone(&self.chest_enabled);
        let left_cooldown = Arc::clone(&self.left_cooldown);
        let right_cooldown = Arc::clone(&self.right_cooldown);

        thread::spawn(move || {
            let mut cdp: Option<Cdp> = None;
            let mut next_try = Instant::now();
            let mut next_target_check = Instant::now();
            let mut left_next: u64 = 0;
            let mut right_next: u64 = 0;
            // 每次开启只允许尝试一次注入/重启，失败后不再自动杀桌宠。
            let mut restart_attempted = false;
            let mut capture_failures = 0u32;
            let mut last_wait_signature = String::new();

            while !shutdown.load(Ordering::Relaxed) {
                if !enabled.load(Ordering::Relaxed) {
                    cdp = None;
                    left_next = 0;
                    right_next = 0;
                    restart_attempted = false;
                    capture_failures = 0;
                    set_status(&status, "idle", "高级开箱已关闭", false, false);
                    thread::sleep(Duration::from_secs(2));
                    continue;
                }

                // 高级开箱依赖自动开箱；未开启时连注入都不做，避免无意义重启桌宠。
                if !chest_enabled.load(Ordering::Relaxed) {
                    cdp = None;
                    left_next = 0;
                    right_next = 0;
                    restart_attempted = false;
                    capture_failures = 0;
                    set_status(&status, "waiting", "自动开箱未开启", false, false);
                    thread::sleep(Duration::from_secs(2));
                    continue;
                }

                if Instant::now() < next_try {
                    thread::sleep(Duration::from_millis(500));
                    continue;
                }

                // 1. 注入：每次开启只允许尝试一次，失败后不再自动杀桌宠
                if !injector::cdp_alive() {
                    if restart_attempted {
                        set_status(
                            &status,
                            "error",
                            "高级开箱失败：CDP 不可用，已停止自动重启桌宠。请关闭再开启高级开箱重试。",
                            false,
                            false,
                        );
                        thread::sleep(Duration::from_secs(2));
                        continue;
                    }
                    restart_attempted = true;
                    set_status(&status, "starting", "正在重启桌宠并注入 DLL…", false, false);
                    logging::info("advanced", "开始重启桌宠并注入 DLL");
                    if let Err(e) = injector::ensure_cdp_pet() {
                        logging::error("advanced", format!("注入/重启失败: {e}"));
                        set_status(&status, "error", &format!("高级开箱失败：{e}"), false, false);
                        thread::sleep(Duration::from_secs(2));
                        continue;
                    }
                } else {
                    // CDP 已可用：之后即使端口丢失，也不再自动重启桌宠
                    restart_attempted = true;
                }

                // 2. 连接
                if cdp.is_none() {
                    match Cdp::connect_main() {
                        Ok(c) => {
                            cdp = Some(c);
                            next_target_check = Instant::now() + Duration::from_secs(5);
                        }
                        Err(e) => {
                            logging::warn("advanced", format!("连接主页面失败: {e}"));
                            set_status(
                                &status,
                                "waiting",
                                &format!("等待游戏主页面 #/：{e}"),
                                false,
                                false,
                            );
                            next_try = Instant::now() + Duration::from_secs(3);
                            continue;
                        }
                    }
                }
                let Some(client) = cdp.as_mut() else { continue };

                // 页面可能从 #/ 切到 panel 等子页面；旧连接会捕获错误页面的
                // openChest，导致宝箱 DOM 始终不可见。定期校验并回到 #/。
                if Instant::now() >= next_target_check {
                    next_target_check = Instant::now() + Duration::from_secs(5);
                    match client.current_page_url() {
                        Ok(url) if url.ends_with("#/") => {}
                        Ok(url) => {
                            logging::warn("advanced", format!("CDP 目标页面不是 #/，准备重连: {url}"));
                            cdp = None;
                            set_status(
                                &status,
                                "starting",
                                "CDP 页面已切换，正在重连游戏主页面…",
                                false,
                                false,
                            );
                            next_try = Instant::now() + Duration::from_secs(2);
                            continue;
                        }
                        Err(e) => {
                            cdp = None;
                            set_status(
                                &status,
                                "starting",
                                &format!("CDP 页面检查失败，正在重连：{e}"),
                                false,
                                false,
                            );
                            next_try = Instant::now() + Duration::from_secs(3);
                            continue;
                        }
                    }
                }

                // 3. 读状态
                let state = match client.read_state() {
                    Ok(v) => v,
                    Err(e) => {
                        cdp = None;
                        logging::error("advanced", format!("read_state 失败: {e}"));
                        set_status(
                            &status,
                            "error",
                            &format!("读取状态失败：{e}"),
                            false,
                            false,
                        );
                        next_try = Instant::now() + Duration::from_secs(10);
                        continue;
                    }
                };
                let chest_fn = state["chestrFn"].as_bool().unwrap_or(false);

                // 4. 首次使用或页面刷新后，重新从闭包捕获 openChest。
                // 启动阶段页面可能还没准备好，先静默重试几次，避免 UI 闪红。
                if !chest_fn {
                    if let Err(e) = client.capture_open_chest() {
                        capture_failures = capture_failures.saturating_add(1);
                        logging::warn(
                            "advanced",
                            format!("捕获 openChest 失败({capture_failures}/3): {e}"),
                        );
                        if capture_failures < 3 {
                            set_status(
                                &status,
                                "starting",
                                &format!("正在捕获 openChest…（{}/3）", capture_failures),
                                true,
                                false,
                            );
                            next_try = Instant::now() + Duration::from_secs(2);
                        } else {
                            set_status(
                                &status,
                                "error",
                                &format!("捕获 openChest 失败：{e}"),
                                true,
                                false,
                            );
                            next_try = Instant::now() + Duration::from_secs(10);
                        }
                        continue;
                    }
                    capture_failures = 0;
                    logging::info("advanced", "捕获 openChest 成功");
                }

                set_status(&status, "ready", "CDP 已就绪", true, true);

                // 无论高级开箱当前是否接管，都把官方 Pinia 冷却共享给普通自动开箱。
                // 普通自动开箱点击后图标可能不刷新，但 Pinia 的时间戳通常已更新。
                let shared_now = state["now"].as_u64().unwrap_or(0);
                if let Some(t) = state["left"].as_u64() {
                    if t > shared_now {
                        let prev = left_cooldown.fetch_max(t, Ordering::Relaxed);
                        if t > prev {
                            logging::info("advanced", format!("共享左侧冷却 until={t}"));
                        }
                    }
                }
                if let Some(t) = state["right"].as_u64() {
                    if t > shared_now {
                        let prev = right_cooldown.fetch_max(t, Ordering::Relaxed);
                        if t > prev {
                            logging::info("advanced", format!("共享右侧冷却 until={t}"));
                        }
                    }
                }

                // 5. 只在用户勾选的场景下接管
                let locked = session::desktop_locked();
                let pet_hidden = matches!(chest::pet_input_visible(), Some(false));
                let should_claim = (lock_screen.load(Ordering::Relaxed) && locked)
                    || (hidden.load(Ordering::Relaxed) && pet_hidden);
                if !should_claim {
                    let signature = format!(
                        "locked={} pet_hidden={} lock_enabled={} hidden_enabled={}",
                        locked,
                        pet_hidden,
                        lock_screen.load(Ordering::Relaxed),
                        hidden.load(Ordering::Relaxed)
                    );
                    if last_wait_signature != signature {
                        logging::info(
                            "advanced",
                            format!("等待锁屏/隐藏条件 {signature}"),
                        );
                        last_wait_signature = signature;
                    }
                    set_status(&status, "waiting", "等待锁屏/隐藏条件", true, true);
                    next_try = Instant::now() + Duration::from_secs(3);
                    continue;
                }
                last_wait_signature.clear();

                // 7. 判断左右宝箱
                let now_ms = state["now"].as_u64().unwrap_or(0);
                let left_in = state["leftIn"].as_i64().unwrap_or(0);
                let right_in = state["rightIn"].as_i64().unwrap_or(0);
                // 服务端时间戳也作为冷却下限，避免重启后 DOM 残留按钮。
                if let Some(t) = state["left"].as_u64() {
                    if t > left_next {
                        left_next = t;
                    }
                }
                if let Some(t) = state["right"].as_u64() {
                    if t > right_next {
                        right_next = t;
                    }
                }
                // 必须同时满足：DOM 有 reward-btn + 本地冷却已过。
                // 直接调 openChest 时 Pinia 不一定马上刷新，单看 DOM 会重复开冷却中的宝箱。
                let left_ready =
                    is_ready(state["leftBtn"].as_bool(), left_in <= 0, now_ms, left_next);
                let right_ready = is_ready(
                    state["rightBtn"].as_bool(),
                    right_in <= 0,
                    now_ms,
                    right_next,
                );
                let place = if left_ready {
                    "left"
                } else if right_ready {
                    "right"
                } else {
                    set_status(&status, "waiting", "暂无就绪宝箱", true, true);
                    next_try = Instant::now() + Duration::from_secs(10);
                    continue;
                };

                let cumulative = state["cumulative"].as_u64().unwrap_or(0);
                // totalClickCount 是当前可消费的剩余点击数；没有该字段时退回累计值。
                let available = state["totalClickCount"].as_u64().unwrap_or(cumulative);
                if available < 1000 {
                    set_status(&status, "waiting", "剩余点击数不足以开箱", true, true);
                    next_try = Instant::now() + Duration::from_secs(60);
                    continue;
                }

                // 8. 调官方 openChest；click_num 在页面内取调用瞬间的 cumulativeClickCount。
                logging::info(
                    "advanced",
                    format!("调用 openChest place={place} cumulative={cumulative} available={available}"),
                );
                match client.open_chest(place) {
                    Ok(v) if v["ok"].as_bool() == Some(true) => {
                        let data = &v["result"]["data"];
                        let fallback_next = now_ms.saturating_add(30 * 60 * 1000);
                        let left_next_resp =
                            normalize_next(data["left_next_time"].as_u64(), now_ms);
                        let right_next_resp =
                            normalize_next(data["right_next_time"].as_u64(), now_ms);
                        if let Some(t) = left_next_resp {
                            left_next = t;
                        } else if place == "left" {
                            left_next = fallback_next;
                        }
                        if let Some(t) = right_next_resp {
                            right_next = t;
                        } else if place == "right" {
                            right_next = fallback_next;
                        }
                        if left_next > now_ms {
                            left_cooldown.fetch_max(left_next, Ordering::Relaxed);
                        }
                        if right_next > now_ms {
                            right_cooldown.fetch_max(right_next, Ordering::Relaxed);
                        }
                        let msg = format!(
                            "已开启{}宝箱，消耗 {} 点击",
                            if place == "left" { "左侧" } else { "右侧" },
                            data["click_consume"].as_u64().unwrap_or(0)
                        );
                        logging::info(
                            "advanced",
                            format!("openChest 成功 place={place} left_next={left_next} right_next={right_next}"),
                        );
                        set_status(&status, "opened", &msg, true, true);
                        next_try = Instant::now() + Duration::from_secs(15);
                    }
                    Ok(v) => {
                        let err = v["err"].as_str().unwrap_or("官方返回失败").to_string();
                        logging::warn("advanced", format!("openChest 返回失败 place={place} err={err}"));
                        set_status(
                            &status,
                            "waiting",
                            &format!("开箱未成功：{err}"),
                            true,
                            true,
                        );
                        let cooling =
                            err.to_ascii_lowercase().contains("cooldown") || err.contains("冷却");
                        next_try =
                            Instant::now() + Duration::from_secs(if cooling { 300 } else { 15 });
                    }
                    Err(e) => {
                        cdp = None;
                        logging::error("advanced", format!("openChest 调用异常 place={place} err={e}"));
                        set_status(
                            &status,
                            "error",
                            &format!("openChest 调用失败：{e}"),
                            false,
                            false,
                        );
                        next_try = Instant::now() + Duration::from_secs(10);
                    }
                }
            }

            set_status(&status, "idle", "高级开箱已停止", false, false);
        });
    }
}

fn is_ready(dom: Option<bool>, fallback: bool, now_ms: u64, next_ms: u64) -> bool {
    now_ms >= next_ms && dom.unwrap_or(fallback)
}

fn normalize_next(value: Option<u64>, now_ms: u64) -> Option<u64> {
    value
        .map(|n| {
            if n > 0 && n < 1_000_000_000_000 {
                n.saturating_mul(1000)
            } else {
                n
            }
        })
        .filter(|n| *n > now_ms)
}

fn set_status(
    status: &Arc<Mutex<AdvancedSnapshot>>,
    state: &str,
    detail: &str,
    cdp: bool,
    chest_fn: bool,
) {
    let mut changed = false;
    if let Ok(mut s) = status.lock() {
        changed = s.state != state || s.detail != detail;
        s.state = state.to_string();
        s.detail = detail.to_string();
        s.cdp = cdp;
        s.chest_fn = chest_fn;
    }
    // ready/waiting 会在每个轮询周期切换，不能写日志；只记录错误、开箱成功、
    // 正在启动/捕获等有意义的状态变化。
    if changed && state != "ready" && state != "waiting" {
        let message = format!("state={state} detail={detail} cdp={cdp} chest_fn={chest_fn}");
        if state == "error" {
            logging::error("advanced", message);
        } else {
            logging::info("advanced", message);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_dom_waits_for_local_cooldown() {
        assert!(!is_ready(Some(true), true, 100, 200));
        assert!(is_ready(Some(true), true, 200, 200));
    }

    #[test]
    fn missing_dom_uses_timestamp_fallback() {
        assert!(is_ready(None, true, 100, 0));
        assert!(!is_ready(None, false, 100, 0));
    }

    #[test]
    fn next_time_normalizes_seconds() {
        assert_eq!(
            normalize_next(Some(1_800_000_000), 1_700_000_000_000),
            Some(1_800_000_000_000)
        );
        assert_eq!(
            normalize_next(Some(1_800_000_000_000), 1_700_000_000_000),
            Some(1_800_000_000_000)
        );
        assert_eq!(normalize_next(Some(0), 1_700_000_000_000), None);
    }
}
