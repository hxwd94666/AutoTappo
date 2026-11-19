use super::geometry::*;
use super::pixel::*;
use super::*;
use crate::features::uia::{self, Uia};

fn post_click(hwnd: Hwnd, client: (i32, i32)) -> Result<(), i32> {
    let lparam = (((client.1 as i64) << 16) | ((client.0 as i64) & 0xFFFF)) as isize;
    unsafe {
        let ok_move = PostMessageW(hwnd, WM_MOUSEMOVE, 0, lparam);
        thread::sleep(Duration::from_millis(30));
        let ok_down = PostMessageW(hwnd, WM_LBUTTONDOWN, MK_LBUTTON, lparam);
        thread::sleep(Duration::from_millis(40));
        let ok_up = PostMessageW(hwnd, WM_LBUTTONUP, 0, lparam);
        if ok_move == 0 || ok_down == 0 || ok_up == 0 {
            Err(std::io::Error::last_os_error().raw_os_error().unwrap_or(0))
        } else {
            Ok(())
        }
    }
}

// ==================== 提权 ====================

fn is_elevated() -> bool {
    unsafe {
        let mut token: Handle = 0;
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return false;
        }
        let mut elevation: u32 = 0;
        let mut len: u32 = 0;
        let ok = GetTokenInformation(
            token,
            TOKEN_ELEVATION_CLASS,
            &mut elevation as *mut u32 as *mut c_void,
            4,
            &mut len,
        );
        CloseHandle(token);
        ok != 0 && elevation != 0
    }
}

// ==================== 控制器与工作线程 ====================

pub struct ChestController {
    pub enabled: Arc<AtomicBool>,
    pub shutdown: Arc<AtomicBool>,
    pub snapshot: Arc<Mutex<ChestSnapshot>>,
    pub opens_ok: Arc<AtomicU64>,
    pub opens_failed: Arc<AtomicU64>,
    /// 高级开箱直接调用 openChest 后写入的服务端冷却截止时间（Unix 毫秒）。
    /// 官方 UI 可能残留 reward-btn，普通自动开箱必须用这个门禁避免误点。
    pub left_cooldown_until_ms: Arc<AtomicU64>,
    pub right_cooldown_until_ms: Arc<AtomicU64>,
    /// 开启自动开箱时置位，让 worker 立即重扫，而不是等当前冷却睡眠结束。
    wake: Arc<AtomicBool>,
}

const READY_POLL_MS: u64 = 5_000;
const READY_WINDOW: Duration = Duration::from_secs(60);
const UNKNOWN_PROBE: Duration = Duration::from_secs(300);

/// UIA 冷却调度：正常睡到剩余时间前 8 秒；临近就绪时每 5 秒复查，
/// 最多复查 60 秒；状态未知时 5 分钟兜底。
fn uia_wait(min_remain: Option<u32>, verify_until: &mut Option<Instant>, now: Instant) -> Duration {
    match min_remain {
        Some(remain) if remain > 8 => {
            *verify_until = None;
            Duration::from_secs(((remain - 8) as u64).clamp(2, 20 * 60))
        }
        Some(_) => {
            let deadline = verify_until.get_or_insert(now + READY_WINDOW);
            if now < *deadline {
                Duration::from_millis(READY_POLL_MS)
            } else {
                *verify_until = None;
                UNKNOWN_PROBE
            }
        }
        None => {
            *verify_until = None;
            UNKNOWN_PROBE
        }
    }
}

/// 用本体容器矩形预测左右宝箱中心位置。
///
/// 官方 CSS：左锚点 `right:-36px; bottom:24px`，右锚点 `right:-46px; bottom:72px`，
/// 锚点尺寸 62×44。取两者预测中心的最近者，即可判定左右。
fn predict_place(rect: uia::Rect, dock: uia::Rect) -> Place {
    let s = (dock.b - dock.t).max(1) as f32 / 211.0;
    let (cx, cy) = rect.center();
    let (cx, cy) = (cx as f32, cy as f32);
    let right_edge = dock.r as f32;
    let bottom = dock.b as f32;

    let lx = right_edge + 5.0 * s; // 36 − 62/2
    let ly = bottom - 46.0 * s; // 24 + 44/2
    let rx = right_edge + 15.0 * s; // 46 − 62/2
    let ry = bottom - 94.0 * s; // 72 + 44/2

    let dl = (cx - lx) * (cx - lx) + (cy - ly) * (cy - ly);
    let dr = (cx - rx) * (cx - rx) + (cy - ry) * (cy - ry);
    if dl <= dr {
        Place::Left
    } else {
        Place::Right
    }
}

/// 同一侧只保留一条（合并就绪标记、取更小的剩余时间）
fn push_chest(list: &mut Vec<UiaChest>, c: UiaChest) {
    if let Some(slot) = list.iter_mut().find(|x| x.place == c.place) {
        // 冷却期间官方可能同时保留 reward-btn 和 reward-countdown；
        // 同一侧只要出现 countdown，就不能视为就绪。
        slot.ready = slot.ready && c.ready;
        slot.remain = match (slot.remain, c.remain) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (None, b) => b,
            (a, None) => a,
        };
        return;
    }
    list.push(c);
}

/// 把 UIA 报告翻译成「左右宝箱 + 客户区点击坐标」
fn resolve_chests(rep: &uia::Report, pet: &Found) -> Vec<UiaChest> {
    let mut out: Vec<UiaChest> = Vec::new();
    let mut items = rep.items.clone();

    if let Some(dock) = rep.dock {
        for it in items.iter() {
            let (cx, cy) = it.center();
            push_chest(
                &mut out,
                UiaChest {
                    place: predict_place(it.rect, dock),
                    ready: it.ready,
                    remain: it.remain_secs,
                    click: (cx - pet.origin.0, cy - pet.origin.1),
                },
            );
        }
    } else {
        // 没有本体矩形可参考时，退化为「更高的那个是右宝箱」
        items.sort_by_key(|it| it.rect.t);
        for (i, it) in items.iter().enumerate() {
            let (cx, cy) = it.center();
            push_chest(
                &mut out,
                UiaChest {
                    place: if i == 0 { Place::Right } else { Place::Left },
                    ready: it.ready,
                    remain: it.remain_secs,
                    click: (cx - pet.origin.0, cy - pet.origin.1),
                },
            );
        }
    }
    out
}

impl ChestController {
    pub fn new() -> Self {
        Self {
            enabled: Arc::new(AtomicBool::new(false)),
            shutdown: Arc::new(AtomicBool::new(false)),
            snapshot: Arc::new(Mutex::new(ChestSnapshot::default())),
            opens_ok: Arc::new(AtomicU64::new(0)),
            opens_failed: Arc::new(AtomicU64::new(0)),
            left_cooldown_until_ms: Arc::new(AtomicU64::new(0)),
            right_cooldown_until_ms: Arc::new(AtomicU64::new(0)),
            wake: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn cooldown_handles(&self) -> (Arc<AtomicU64>, Arc<AtomicU64>) {
        (
            Arc::clone(&self.left_cooldown_until_ms),
            Arc::clone(&self.right_cooldown_until_ms),
        )
    }

    pub fn set_enabled(&self, on: bool) {
        self.enabled.store(on, Ordering::SeqCst);
        if on {
            self.wake.store(true, Ordering::SeqCst);
        }
    }

    pub fn status(&self) -> ChestSnapshot {
        self.snapshot.lock().map(|s| s.clone()).unwrap_or_default()
    }

    pub fn start_worker(&self) {
        let enabled = Arc::clone(&self.enabled);
        let left_cooldown = Arc::clone(&self.left_cooldown_until_ms);
        let right_cooldown = Arc::clone(&self.right_cooldown_until_ms);
        let shutdown = Arc::clone(&self.shutdown);
        let snapshot = Arc::clone(&self.snapshot);
        let opens_ok = Arc::clone(&self.opens_ok);
        let opens_failed = Arc::clone(&self.opens_failed);
        let wake = Arc::clone(&self.wake);

        thread::spawn(move || {
            let mut next_check = Instant::now();
            let mut verify_until: Option<Instant> = None;
            let mut occluded_backoff = Duration::from_secs(2);
            let mut left_state = String::from("-");
            let mut right_state = String::from("-");
            let mut last_detail = String::from("待机");

            // UIA 会话：惰性创建；失败后 5 分钟再试，期间不影响旧路径
            let mut uia_session: Option<Uia> = None;
            let mut uia_retry_at = Instant::now();

            // 左右宝箱的到点时间（Unix 秒；0 = 未知）。
            // 用 Cell 是为了让下面只读 snapshot 的 publish 闭包也能读到最新值。
            let left_ready_at = std::cell::Cell::new(0u64);
            let right_ready_at = std::cell::Cell::new(0u64);

            let publish = |state: &str, detail: &str, left: &str, right: &str| {
                if let Ok(mut s) = snapshot.lock() {
                    s.enabled = enabled.load(Ordering::Relaxed);
                    s.elevated = is_elevated();
                    s.state = state.to_string();
                    s.detail = detail.to_string();
                    s.left = left.to_string();
                    s.right = right.to_string();
                    s.left_ready_at = left_ready_at.get();
                    s.right_ready_at = right_ready_at.get();
                    s.opens_ok = opens_ok.load(Ordering::Relaxed) as u32;
                    s.opens_failed = opens_failed.load(Ordering::Relaxed) as u32;
                    s.last_event_unix = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs())
                        .unwrap_or(0);
                }
            };

            while !shutdown.load(Ordering::Relaxed) {
                // 未启用：低频休眠，几乎零开销
                if !enabled.load(Ordering::Relaxed) {
                    if last_detail != "已关闭" {
                        publish("idle", "已关闭", &left_state, &right_state);
                        last_detail = "已关闭".to_string();
                    }
                    thread::sleep(Duration::from_millis(500));
                    continue;
                }

                // 开启自动开箱时立即重扫，不能继续睡旧的冷却计划。
                if wake.swap(false, Ordering::SeqCst) {
                    next_check = Instant::now();
                    verify_until = None;
                }

                let now = Instant::now();
                if now < next_check {
                    let wait = (next_check - now).min(Duration::from_millis(200));
                    thread::sleep(wait);
                    continue;
                }

                // 礼貌避让：用户在打字 / 划词 / 拖拽时不动作
                if user_is_active() {
                    next_check = Instant::now() + Duration::from_millis(500);
                    continue;
                }

                // 锁屏 / 息屏：既点不到，也没必要（唤醒显示器反而打扰用户）
                if crate::platform::session::should_pause() {
                    next_check = Instant::now() + Duration::from_secs(2);
                    continue;
                }

                // ① 找窗口
                let pet = match find_pet() {
                    Some(p) => p,
                    None => {
                        left_state = "-".into();
                        right_state = "-".into();
                        last_detail = "未找到桌宠进程".into();
                        publish("no-pet", "未找到桌宠进程", &left_state, &right_state);
                        next_check = Instant::now() + Duration::from_secs(5);
                        continue;
                    }
                };

                // ①.5 UIA 优先路径：直接读官方 WebView 的 DOM。
                //
                // 这条路与「窗口命中区域是否被其他桌宠合并」完全无关（多猫重叠的根因），
                // 也不受不透明度、桌宠大小、DPI 影响，并且能拿到精确剩余时间。
                // 只有 UIA 不可用时才降级到 ② 之后的区域 + 像素老路径。
                if Instant::now() >= uia_retry_at {
                    if uia_session.is_none() {
                        uia_session = Uia::new();
                        if uia_session.is_none() {
                            // COM 初始化失败：5 分钟后再试，期间走老路径
                            uia_retry_at = Instant::now() + Duration::from_secs(300);
                            debug_log(format_args!("[chest] UIA 初始化失败，降级到区域+像素"));
                        }
                    }

                    let outcome = match uia_session.as_ref() {
                        Some(u) => match u.probe(pet.visual) {
                            Some(rep) if rep.items.is_empty() => UiaProbe::Empty,
                            Some(rep) => UiaProbe::Hit(resolve_chests(&rep, &pet)),
                            None => {
                                // 调用失败：丢弃会话，5 分钟后再重建
                                uia_retry_at = Instant::now() + Duration::from_secs(300);
                                uia_session = None;
                                debug_log(format_args!("[chest] UIA 探测失败，降级到区域+像素"));
                                UiaProbe::Unavailable
                            }
                        },
                        None => UiaProbe::Unavailable,
                    };

                    match outcome {
                        // 界面在，但一个宝箱元素都没有：累计点击不足 1000，或面板打开
                        UiaProbe::Empty => {
                            left_ready_at.set(0);
                            right_ready_at.set(0);
                            left_state = "-".into();
                            right_state = "-".into();
                            last_detail = "宝箱未解锁（累计点击数不足 1000）".into();
                            publish("no-chest", &last_detail, &left_state, &right_state);
                            next_check = Instant::now() + Duration::from_secs(15);
                            continue;
                        }
                        UiaProbe::Hit(mut chests) => {
                            // 高级开箱绕过官方 UI 直接调用 openChest 后，官方可能仍残留
                            // reward-btn。共享冷却优先于 DOM，避免点击冷却中的宝箱。
                            let now_ms = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .map(|d| d.as_millis() as u64)
                                .unwrap_or(0);
                            for c in chests.iter_mut() {
                                let until = match c.place {
                                    Place::Left => left_cooldown.load(Ordering::Relaxed),
                                    Place::Right => right_cooldown.load(Ordering::Relaxed),
                                };
                                if until > now_ms {
                                    c.ready = false;
                                    let remain = ((until - now_ms).div_ceil(1000))
                                        .min(u32::MAX as u64) as u32;
                                    c.remain = Some(c.remain.map_or(remain, |r| r.min(remain)));
                                }
                            }
                            let mut min_remain: Option<u32> = None;
                            let mut any_ready = false;
                            let now_unix = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .map(|d| d.as_secs())
                                .unwrap_or(0);
                            let mut l_at: u64 = 0;
                            let mut r_at: u64 = 0;

                            for c in chests.iter() {
                                let text = if c.ready {
                                    "就绪".to_string()
                                } else if let Some(r) = c.remain {
                                    format!("冷却 {}", uia::fmt_mmss(r))
                                } else {
                                    "冷却中".to_string()
                                };
                                let at = match (c.ready, c.remain) {
                                    (false, Some(r)) => now_unix + r as u64,
                                    _ => 0,
                                };
                                match c.place {
                                    Place::Left => {
                                        left_state = text.clone();
                                        l_at = at;
                                    }
                                    Place::Right => {
                                        right_state = text.clone();
                                        r_at = at;
                                    }
                                }
                                if c.ready {
                                    any_ready = true;
                                } else if let Some(r) = c.remain {
                                    min_remain = Some(min_remain.map_or(r, |m: u32| m.min(r)));
                                }
                            }
                            // 交给前端做本地秒级走字（后端会因为长睡眠而「冻结」）
                            left_ready_at.set(l_at);
                            right_ready_at.set(r_at);

                            debug_log(format_args!(
                                "[chest] UIA 左={} 右={} 就绪={}",
                                left_state, right_state, any_ready
                            ));

                            if !any_ready {
                                // 正常：睡到最早冷却前 8 秒；临近：5 秒复查，最多 60 秒。
                                let wait = uia_wait(min_remain, &mut verify_until, Instant::now());
                                last_detail = "冷却中，到点自动开启".into();
                                publish("cooldown", &last_detail, &left_state, &right_state);
                                next_check = Instant::now() + wait;
                                continue;
                            }

                            // 有就绪的宝箱：逐个点击（每轮只点一个，点完立刻复查）
                            verify_until = None;
                            let mut acted = false;
                            if let Some(c) = chests.iter().find(|c| c.ready) {
                                match post_click(pet.input, c.click) {
                                    Ok(()) => {
                                        thread::sleep(Duration::from_millis(400));
                                        let still_ready = uia_session
                                            .as_ref()
                                            .and_then(|u| u.probe(pet.visual))
                                            .map(|rep| {
                                                resolve_chests(&rep, &pet)
                                                    .iter()
                                                    .any(|n| n.place == c.place && n.ready)
                                            })
                                            .unwrap_or(false);

                                        if still_ready {
                                            opens_failed.fetch_add(1, Ordering::Relaxed);
                                            last_detail = format!(
                                                "{} 宝箱点击未生效，稍后重试",
                                                c.place.as_str()
                                            );
                                            publish(
                                                "retry",
                                                &last_detail,
                                                &left_state,
                                                &right_state,
                                            );
                                            // 官方 UI 可能短暂未刷新；同一侧至少退避 30 秒，
                                            // 避免连续点击已冷却宝箱产生「宝箱冷却中」toast。
                                            next_check = Instant::now() + Duration::from_secs(30);
                                        } else {
                                            opens_ok.fetch_add(1, Ordering::Relaxed);
                                            last_detail =
                                                format!("已开启 {} 宝箱", c.place.as_str());
                                            publish(
                                                "opened",
                                                &last_detail,
                                                &left_state,
                                                &right_state,
                                            );
                                            // UIA 路径不需要「睡满 29 分钟」：
                                            // 开完立刻重探，开掉的那个会变成 30 分钟倒计时，
                                            // 下一轮自然按剩余时间睡；另一个若也就绪能马上点掉。
                                            next_check =
                                                Instant::now() + Duration::from_millis(3_000);
                                        }
                                        acted = true;
                                    }
                                    Err(code) => {
                                        opens_failed.fetch_add(1, Ordering::Relaxed);
                                        if code == ERROR_ACCESS_DENIED {
                                            last_detail =
                                                "需要管理员权限（官方桌宠以管理员运行）".into();
                                            publish(
                                                "need-admin",
                                                &last_detail,
                                                &left_state,
                                                &right_state,
                                            );
                                            next_check = Instant::now()
                                                + Duration::from_millis(NEED_ADMIN_BACKOFF_MS);
                                        } else {
                                            last_detail = format!("点击投递失败 (code {code})");
                                            publish(
                                                "error",
                                                &last_detail,
                                                &left_state,
                                                &right_state,
                                            );
                                            next_check = Instant::now() + Duration::from_secs(10);
                                        }
                                        acted = true;
                                    }
                                }
                            }
                            if acted {
                                continue;
                            }
                        }
                        UiaProbe::Unavailable => {
                            // 降级到区域 + 像素路径；老路径没有精确时间，清掉倒计时
                            left_ready_at.set(0);
                            right_ready_at.set(0);
                            verify_until = None;
                        }
                    }
                }

                // ② 读区域（每轮都读，绝不缓存）
                let rects = read_region(pet.input);
                if region_is_empty(&rects) {
                    left_state = "-".into();
                    right_state = "-".into();
                    last_detail = "桌宠面板打开或已隐藏".into();
                    publish("idle", "桌宠面板打开或已隐藏", &left_state, &right_state);
                    next_check = Instant::now() + Duration::from_secs(5);
                    continue;
                }

                // ③ 稳定性校验：60ms 后再读一次，必须完全一致（用户拖拽时区域会抖动）
                let h1 = region_hash(&rects);
                thread::sleep(Duration::from_millis(60));
                let rects2 = read_region(pet.input);
                if region_hash(&rects2) != h1 {
                    last_detail = "桌宠正在移动".into();
                    publish("moving", "桌宠正在移动", &left_state, &right_state);
                    next_check = Instant::now() + Duration::from_millis(800);
                    continue;
                }

                // ④ 定位锚点
                let anchors = locate(&rects2);
                if anchors.is_empty() {
                    left_state = "-".into();
                    right_state = "-".into();
                    last_detail = "暂无宝箱（累计点击数不足）".into();
                    publish(
                        "no-chest",
                        "暂无宝箱（累计点击数不足）",
                        &left_state,
                        &right_state,
                    );
                    next_check = Instant::now() + Duration::from_secs(10);
                    continue;
                }

                // ⑤ 逐个锚点判定
                let mut acted = false;
                let mut occluded_any = false;
                let mut cooldown_any = false;
                let mut unknown_any = false;

                let now_ms = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0);
                for anchor in anchors.iter() {
                    let (mut state, mut text) = readiness_of(anchor, pet.origin, &pet);
                    let until = match anchor.place {
                        Place::Left => left_cooldown.load(Ordering::Relaxed),
                        Place::Right => right_cooldown.load(Ordering::Relaxed),
                    };
                    if state == Readiness::Ready && until > now_ms {
                        state = Readiness::Cooldown;
                        text = format!(
                            "高级冷却 {}",
                            uia::fmt_mmss(((until - now_ms).div_ceil(1000)) as u32)
                        );
                    }
                    match anchor.place {
                        Place::Left => left_state = text.clone(),
                        Place::Right => right_state = text.clone(),
                    }

                    match state {
                        Readiness::Cooldown => {
                            cooldown_any = true;
                        }
                        Readiness::Unknown => {
                            if text.starts_with("被遮挡") {
                                occluded_any = true;
                            } else {
                                unknown_any = true;
                            }
                        }
                        Readiness::Ready => {
                            // ⑥ 投递点击（客户区坐标 = 屏幕坐标 - 窗口原点，锚点本身就是窗口相对坐标）
                            let client = anchor.click;
                            match post_click(pet.input, client) {
                                Ok(()) => {
                                    // ⑦ 点后验证：350ms 后复查，必须不再处于就绪
                                    thread::sleep(Duration::from_millis(350));
                                    let after = read_region(pet.input);
                                    let changed = region_hash(&after) != h1;
                                    let re_anchors = locate(&after);
                                    let still_ready = re_anchors
                                        .iter()
                                        .find(|a| a.place == anchor.place)
                                        .map(|a| {
                                            readiness_of(a, pet.origin, &pet).0 == Readiness::Ready
                                        })
                                        .unwrap_or(false);

                                    if changed && !still_ready {
                                        opens_ok.fetch_add(1, Ordering::Relaxed);
                                        last_detail =
                                            format!("已开启 {} 宝箱", anchor.place.as_str());
                                        publish("opened", &last_detail, &left_state, &right_state);
                                        next_check = Instant::now()
                                            + Duration::from_millis(COOLDOWN_SLEEP_MS);
                                        acted = true;
                                    } else {
                                        // 点了但没生效：可能跨屏后区域与绘制不一致，降频重试
                                        opens_failed.fetch_add(1, Ordering::Relaxed);
                                        last_detail = format!(
                                            "{} 宝箱点击未生效，稍后重试",
                                            anchor.place.as_str()
                                        );
                                        publish("retry", &last_detail, &left_state, &right_state);
                                        next_check = Instant::now() + Duration::from_secs(5);
                                        acted = true;
                                    }
                                }
                                Err(code) => {
                                    opens_failed.fetch_add(1, Ordering::Relaxed);
                                    if code == ERROR_ACCESS_DENIED {
                                        last_detail =
                                            "需要管理员权限（官方桌宠以管理员运行）".into();
                                        publish(
                                            "need-admin",
                                            &last_detail,
                                            &left_state,
                                            &right_state,
                                        );
                                        next_check = Instant::now()
                                            + Duration::from_millis(NEED_ADMIN_BACKOFF_MS);
                                    } else {
                                        last_detail = format!("点击投递失败 (code {code})");
                                        publish("error", &last_detail, &left_state, &right_state);
                                        next_check = Instant::now() + Duration::from_secs(10);
                                    }
                                    acted = true;
                                }
                            }
                        }
                    }
                    if acted {
                        break;
                    }
                }

                if acted {
                    continue;
                }

                // ⑧ 排定下一轮
                if occluded_any {
                    next_check = Instant::now() + occluded_backoff;
                    occluded_backoff =
                        (occluded_backoff * 2).min(Duration::from_millis(OCCLUDED_BACKOFF_MAX_MS));
                    publish("occluded", &last_detail, &left_state, &right_state);
                } else {
                    occluded_backoff = Duration::from_secs(2);
                    if cooldown_any {
                        last_detail = "冷却中，稍后探测".into();
                        publish("cooldown", &last_detail, &left_state, &right_state);
                        next_check = Instant::now() + Duration::from_millis(COOLDOWN_PROBE_MS);
                    } else if unknown_any {
                        last_detail = "画面异常，本轮跳过".into();
                        publish("unknown", &last_detail, &left_state, &right_state);
                        next_check = Instant::now() + Duration::from_secs(5);
                    } else {
                        // 理论上不会走到这里
                        next_check = Instant::now() + Duration::from_millis(READY_SCAN_MS);
                    }
                }
            }

            publish("stopped", "已退出", &left_state, &right_state);
        });
    }
}

/// 礼貌避让：用户在打字、切换输入法或拖拽时不注入
fn user_is_active() -> bool {
    let down = |vk: i32| unsafe { (GetAsyncKeyState(vk) as u16 & 0x8000) != 0 };
    if down(0x01) || down(0x02) || down(0x04) {
        return true;
    }
    if down(0x10) || down(0x11) || down(0x12) || down(0x5B) || down(0x5C) {
        return true;
    }
    if down(0x20) || down(0x08) || down(0x0D) {
        return true;
    }
    false
}

/// 供 main 早期调用，确保坐标是物理像素（多屏 / 高 DPI 必需）
pub fn ensure_dpi_aware() {
    unsafe {
        SetProcessDpiAwarenessContext(-4isize); // PER_MONITOR_AWARE_V2
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(l: i32, t: i32, right: i32, b: i32) -> uia::Rect {
        uia::Rect { l, t, r: right, b }
    }

    #[test]
    fn uia_wait_uses_five_second_ready_poll() {
        let now = Instant::now();
        let mut verify = None;
        assert_eq!(
            uia_wait(Some(8), &mut verify, now),
            Duration::from_millis(5_000)
        );
        assert!(verify.is_some());

        let mut verify = Some(now);
        assert_eq!(
            uia_wait(Some(100), &mut verify, now),
            Duration::from_secs(92)
        );
        assert!(verify.is_none());
    }

    #[test]
    fn uia_wait_falls_back_after_ready_window() {
        let now = Instant::now();
        let mut verify = None;
        assert_eq!(
            uia_wait(Some(8), &mut verify, now),
            Duration::from_millis(5_000)
        );
        let later = now + Duration::from_secs(61);
        assert_eq!(
            uia_wait(Some(8), &mut verify, later),
            Duration::from_secs(300)
        );
        assert!(verify.is_none());
        assert_eq!(uia_wait(None, &mut verify, later), Duration::from_secs(300));
    }

    #[test]
    fn enabling_auto_chest_wakes_worker() {
        let controller = ChestController::new();
        controller.wake.store(false, Ordering::SeqCst);
        controller.set_enabled(true);
        assert!(controller.wake.load(Ordering::SeqCst));
    }

    #[test]
    fn predict_left_and_right() {
        let dock = r(0, 0, 200, 211);
        // 左锚点中心 (205,165)，右锚点中心 (215,117)
        assert_eq!(predict_place(r(190, 140, 220, 190), dock), Place::Left);
        assert_eq!(predict_place(r(200, 92, 230, 142), dock), Place::Right);
    }

    #[test]
    fn push_chest_countdown_wins_same_place() {
        let mut list = Vec::new();
        push_chest(
            &mut list,
            UiaChest {
                place: Place::Left,
                ready: false,
                remain: Some(10),
                click: (1, 1),
            },
        );
        push_chest(
            &mut list,
            UiaChest {
                place: Place::Left,
                ready: true,
                remain: Some(5),
                click: (2, 2),
            },
        );
        assert_eq!(list.len(), 1);
        assert!(!list[0].ready);
        assert_eq!(list[0].remain, Some(5));
    }

    #[test]
    fn push_chest_ready_then_countdown_is_cooling() {
        let mut list = Vec::new();
        push_chest(
            &mut list,
            UiaChest {
                place: Place::Right,
                ready: true,
                remain: None,
                click: (2, 2),
            },
        );
        push_chest(
            &mut list,
            UiaChest {
                place: Place::Right,
                ready: false,
                remain: Some(20),
                click: (2, 2),
            },
        );
        assert_eq!(list.len(), 1);
        assert!(!list[0].ready);
    }

    #[test]
    fn snapshot_default_is_idle() {
        let s = ChestSnapshot::default();
        assert_eq!(s.state, "idle");
        assert_eq!(s.opens_ok, 0);
        assert_eq!(s.opens_failed, 0);
    }
}
