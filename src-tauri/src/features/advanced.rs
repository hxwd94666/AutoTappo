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
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::features::cdp::Cdp;
use crate::features::injector;
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
}

impl AdvancedController {
    pub fn new(chest_enabled: Arc<AtomicBool>) -> Self {
        Self {
            enabled: Arc::new(AtomicBool::new(false)),
            lock_screen: Arc::new(AtomicBool::new(false)),
            hidden: Arc::new(AtomicBool::new(false)),
            shutdown: Arc::new(AtomicBool::new(false)),
            status: Arc::new(Mutex::new(AdvancedSnapshot::default())),
            chest_enabled,
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

        thread::spawn(move || {
            let mut cdp: Option<Cdp> = None;
            let mut next_try = Instant::now();
            let mut left_next: u64 = 0;
            let mut right_next: u64 = 0;
            // 每次开启只允许尝试一次注入/重启，失败后不再自动杀桌宠。
            let mut restart_attempted = false;
            let mut capture_failures = 0u32;

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
                    if let Err(e) = injector::ensure_cdp_pet() {
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
                    match Cdp::connect() {
                        Ok(c) => cdp = Some(c),
                        Err(e) => {
                            set_status(
                                &status,
                                "error",
                                &format!("CDP 连接失败：{e}"),
                                false,
                                false,
                            );
                            next_try = Instant::now() + Duration::from_secs(10);
                            continue;
                        }
                    }
                }
                let Some(client) = cdp.as_mut() else { continue };

                // 3. 读状态
                let state = match client.read_state() {
                    Ok(v) => v,
                    Err(e) => {
                        cdp = None;
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
                }

                set_status(&status, "ready", "CDP 已就绪", true, true);

                // 5. 只在用户勾选的场景下接管
                let locked = session::desktop_locked();
                let pet_hidden = matches!(chest::pet_input_visible(), Some(false));
                let should_claim = (lock_screen.load(Ordering::Relaxed) && locked)
                    || (hidden.load(Ordering::Relaxed) && pet_hidden);
                if !should_claim {
                    set_status(&status, "waiting", "等待锁屏/隐藏条件", true, true);
                    next_try = Instant::now() + Duration::from_secs(3);
                    continue;
                }

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
                        let msg = format!(
                            "已开启{}宝箱，消耗 {} 点击",
                            if place == "left" { "左侧" } else { "右侧" },
                            data["click_consume"].as_u64().unwrap_or(0)
                        );
                        set_status(&status, "opened", &msg, true, true);
                        next_try = Instant::now() + Duration::from_secs(15);
                    }
                    Ok(v) => {
                        let err = v["err"].as_str().unwrap_or("官方返回失败").to_string();
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
    if let Ok(mut s) = status.lock() {
        s.state = state.to_string();
        s.detail = detail.to_string();
        s.cdp = cdp;
        s.chest_fn = chest_fn;
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
