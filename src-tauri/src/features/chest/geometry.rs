use super::*;

extern "system" fn enum_cb(hwnd: Hwnd, lparam: isize) -> i32 {
    let ctx = unsafe { &mut *(lparam as *mut EnumCtx) };
    let cls = class_name(hwnd);
    if cls == "DesktopCatInput" && ctx.input == 0 {
        ctx.input = hwnd;
    } else if cls == "Tauri Window" && window_title(hwnd) == "Wuwa Tappo" && ctx.visual == 0 {
        ctx.visual = hwnd;
    }
    if ctx.input != 0 && ctx.visual != 0 {
        return 0; // 提前结束枚举
    }
    1
}

/// 桌宠输入层窗口当前是否可见。F9 隐藏时返回 Some(false)。
pub fn pet_input_visible() -> Option<bool> {
    let mut ctx = EnumCtx { input: 0, visual: 0 };
    unsafe { EnumWindows(enum_cb, &mut ctx as *mut EnumCtx as isize) };
    if ctx.input == 0 {
        return None;
    }
    Some(unsafe { IsWindowVisible(ctx.input) != 0 })
}

/// 找到桌宠的输入层（真实点击接收窗口）与视觉层窗口
pub(super) fn find_pet() -> Option<Found> {
    let mut ctx = EnumCtx { input: 0, visual: 0 };
    unsafe { EnumWindows(enum_cb, &mut ctx as *mut EnumCtx as isize) };
    if ctx.input == 0 {
        return None;
    }
    let visual = if ctx.visual != 0 {
        ctx.visual
    } else {
        unsafe { FindWindowW(wide("Tauri Window").as_ptr(), wide("Wuwa Tappo").as_ptr()) }
    };
    let rect = window_rect(ctx.input)?;
    // 输入层是全屏窗口，其左上角即本显示器的物理原点（多屏时可为负）
    Some(Found {
        input: ctx.input,
        visual,
        pid: window_pid(ctx.input),
        origin: (rect.l, rect.t),
        dpi: unsafe { GetDpiForWindow(ctx.input) },
    })
}

/// 读取窗口当前裁剪区域（窗口相对坐标）
pub(super) fn read_region(hwnd: Hwnd) -> Vec<Box2> {
    let hrgn = unsafe { CreateRectRgn(0, 0, 0, 0) };
    if hrgn == 0 {
        return Vec::new();
    }
    let mut out = Vec::new();
    unsafe {
        let kind = GetWindowRgn(hwnd, hrgn);
        if kind > RGN_NULL {
            let need = GetRegionData(hrgn, 0, std::ptr::null_mut());
            if need > 0 {
                let mut buf = vec![0u8; need as usize];
                let got = GetRegionData(hrgn, need, buf.as_mut_ptr() as *mut c_void);
                if got > 0 {
                    let hdr = &*(buf.as_ptr() as *const RGNDATAHEADER);
                    let header_size = std::mem::size_of::<RGNDATAHEADER>();
                    let count = hdr.n_count as usize;
                    if buf.len() >= header_size + count * 16 {
                        let base = buf.as_ptr() as usize + header_size;
                        for i in 0..count {
                            let p = (base + i * 16) as *const RECT;
                            let r = *p;
                            out.push(Box2 { l: r.left, t: r.top, r: r.right, b: r.bottom });
                        }
                    }
                }
            }
        } else if kind == RGN_ERROR {
            // 无区域（模块面板打开 / 被隐藏），返回空
        }
        DeleteObject(hrgn);
    }
    out
}

pub(super) fn region_hash(rects: &[Box2]) -> u64 {
    let mut h: u64 = 1469598103934665603;
    for r in rects {
        for v in [r.l, r.t, r.r, r.b] {
            h ^= v as u64;
            h = h.wrapping_mul(1099511628211);
        }
    }
    h
}

pub(super) fn region_is_empty(rects: &[Box2]) -> bool {
    rects.is_empty()
}

/// 按 x 区间重叠把矩形聚类（房间里可能有多只位置不同的桌宠）
pub(super) fn cluster_by_x(rects: &[Box2]) -> Vec<Vec<Box2>> {
    let n = rects.len();
    let mut parent: Vec<usize> = (0..n).collect();

    fn find(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }

    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by_key(|&i| rects[i].l);
    for a in 0..n {
        let i = order[a];
        for &j in order.iter().skip(a + 1) {
            if rects[j].l >= rects[i].r {
                break;
            }
            if rects[j].r > rects[i].l {
                let ri = find(&mut parent, i);
                let rj = find(&mut parent, j);
                if ri != rj {
                    parent[rj] = ri;
                }
            }
        }
    }

    let mut groups: HashMap<usize, Vec<Box2>> = HashMap::new();
    for (i, rect) in rects.iter().enumerate() {
        let root = find(&mut parent, i);
        groups.entry(root).or_default().push(*rect);
    }
    groups.into_values().collect()
}

/// 单簇内求锚点：逐行右边界剖面 → 平台 → 同一批行内的本体右缘 → 自校验
pub(super) fn anchors_in_cluster(rects: &[Box2]) -> Vec<Anchor> {
    let mut prof: HashMap<i32, i32> = HashMap::new();
    for r in rects {
        for y in r.t..r.b {
            let e = prof.entry(y).or_insert(-1);
            if r.r > *e {
                *e = r.r;
            }
        }
    }
    if prof.is_empty() {
        return Vec::new();
    }

    // 相同右边界 → (行数, 最小 y, 最大 y)
    let mut plateaus: HashMap<i32, (i32, i32, i32)> = HashMap::new();
    for (&y, &right) in prof.iter() {
        let e = plateaus.entry(right).or_insert((0, y, y));
        e.0 += 1;
        if y < e.1 {
            e.1 = y;
        }
        if y > e.2 {
            e.2 = y;
        }
    }

    let mut cands: Vec<(i32, i32, i32, i32)> = plateaus
        .iter()
        .filter(|(_, v)| v.0 >= 18)
        .map(|(&right, v)| (right, v.0, v.1, v.2))
        .collect();
    cands.sort_by_key(|a| std::cmp::Reverse(a.0));

    let mut out = Vec::new();
    for &(right, _rows, top, bottom) in cands.iter() {
        let band: i32 = bottom - top + 1;
        let scale = band as f32 / ANCHOR_H;
        // scale ≈ (DPI/96) × 桌宠大小比例。
        // 官方桌宠大小范围 50%~200%，叠加最高 200% DPI 时可达 4.0，
        // 因此区间取 0.35~4.6，两端各留测量误差余量。
        if !(0.35..=4.6).contains(&scale) {
            continue;
        }
        let lo = top - 2 * band;
        let hi = bottom + 2 * band;

        // 本体右缘：只取与锚点上下相邻的行（排除锚点自身所占的行），取众数
        let mut votes: HashMap<i32, i32> = HashMap::new();
        for y in lo..=hi {
            if y >= top && y <= bottom {
                continue;
            }
            if let Some(&v) = prof.get(&y) {
                if v < right {
                    *votes.entry(v).or_insert(0) += 1;
                }
            }
        }
        let dock_right = match votes.iter().max_by_key(|(v, c)| (**c, **v)) {
            Some((v, _)) => *v,
            None => continue,
        };

        let delta = (right - dock_right) as f32;
        if delta < 20.0 {
            continue;
        }
        let as_right = (delta / OFF_RIGHT - scale).abs();
        let as_left = (delta / OFF_LEFT - scale).abs();
        let (place, err) = if as_right <= as_left {
            (Place::Right, as_right)
        } else {
            (Place::Left, as_left)
        };
        if err > TOLERANCE * scale.max(1.0) {
            continue; // 比例对不上，说明不是宝箱锚点 → 安全放弃
        }

        let w = (ANCHOR_W * scale).round() as i32;
        let area = Box2 { l: right - w, t: top, r: right, b: bottom + 1 };
        out.push(Anchor { place, area, click: area.center(), scale });
    }
    out
}

pub(super) fn locate(rects: &[Box2]) -> Vec<Anchor> {
    let mut found: Vec<Anchor> = Vec::new();
    for cl in cluster_by_x(rects) {
        found.extend(anchors_in_cluster(&cl));
    }
    // 去重（右锚点可能把左锚点当成候选）
    found.sort_by_key(|a| (a.area.t, a.area.l));
    found.dedup_by_key(|a| (a.place as u8, a.area.t));
    found
}

// ==================== 遮挡检测 ====================

struct ZEntry {
    hwnd: Hwnd,
    rect: Box2,
    transparent: bool,
}

fn z_order() -> Vec<ZEntry> {
    let mut out: Vec<ZEntry> = Vec::new();

    extern "system" fn cb(hwnd: Hwnd, lparam: isize) -> i32 {
        let out = unsafe { &mut *(lparam as *mut Vec<ZEntry>) };
        if unsafe { IsWindowVisible(hwnd) } == 0 || unsafe { IsIconic(hwnd) } != 0 {
            return 1;
        }
        if let Some(rect) = window_rect(hwnd) {
            let ex = unsafe { GetWindowLongW(hwnd, GWL_EXSTYLE) };
            out.push(ZEntry { hwnd, rect, transparent: (ex & WS_EX_TRANSPARENT) != 0 });
        }
        1
    }

    unsafe { EnumWindows(cb, &mut out as *mut Vec<ZEntry> as isize) };
    out
}

/// 只有排在桌宠视觉层**之前**（更靠上）的窗口才可能遮挡它
pub(super) fn is_occluded(area_screen: &Box2, pet: &Found) -> Option<String> {
    let order = z_order();
    let pet_index = order.iter().position(|e| e.hwnd == pet.visual)?;
    for e in order.iter().take(pet_index) {
        if window_pid(e.hwnd) == pet.pid {
            continue; // 自己人（输入层 / Toast / 面板）不算遮挡
        }
        if e.transparent {
            continue; // 点击穿透的全屏覆盖层，不构成遮挡
        }
        if e.rect.intersects(area_screen) {
            return Some(format!("{} / {}", class_name(e.hwnd), window_title(e.hwnd)));
        }
    }
    None
}


#[cfg(test)]
mod tests {
    use super::*;

    fn b(l: i32, t: i32, r: i32, b: i32) -> Box2 {
        Box2 { l, t, r, b }
    }

    #[test]
    fn box_intersects_and_center() {
        let a = b(0, 0, 10, 10);
        assert!(a.intersects(&b(5, 5, 15, 15)));
        assert!(!a.intersects(&b(10, 0, 20, 10)));
        assert_eq!(a.center(), (5, 5));
    }

    #[test]
    fn cluster_splits_disjoint_groups() {
        let rects = vec![
            b(0, 0, 100, 100),
            b(50, 0, 150, 100),
            b(300, 0, 400, 100),
        ];
        let clusters = cluster_by_x(&rects);
        assert_eq!(clusters.len(), 2);
        let mut sizes: Vec<usize> = clusters.iter().map(|c| c.len()).collect();
        sizes.sort_unstable();
        assert_eq!(sizes, vec![1, 2]);
    }

    #[test]
    fn region_hash_changes_with_rects() {
        let a = vec![b(0, 0, 10, 10)];
        let mut c = a.clone();
        assert_eq!(region_hash(&a), region_hash(&c));
        c.push(b(20, 20, 30, 30));
        assert_ne!(region_hash(&a), region_hash(&c));
        assert!(region_is_empty(&[]));
        assert!(!region_is_empty(&a));
    }

    #[test]
    fn locate_empty_is_empty() {
        assert!(locate(&[]).is_empty());
    }
}
