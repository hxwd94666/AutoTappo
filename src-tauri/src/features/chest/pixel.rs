use super::*;
use super::geometry::is_occluded;

fn capture(area_screen: &Box2) -> Option<(Vec<u8>, i32, i32)> {
    let w = area_screen.w();
    let h = area_screen.h();
    if w <= 0 || h <= 0 {
        return None;
    }
    unsafe {
        let screen = GetDC(0);
        if screen == 0 {
            return None;
        }
        let mem = CreateCompatibleDC(screen);
        if mem == 0 {
            ReleaseDC(0, screen);
            return None;
        }
        let bmp = CreateCompatibleBitmap(screen, w, h);
        if bmp == 0 {
            DeleteDC(mem);
            ReleaseDC(0, screen);
            return None;
        }
        let old = SelectObject(mem, bmp);
        let ok = BitBlt(mem, 0, 0, w, h, screen, area_screen.l, area_screen.t, SRCCOPY);

        let mut bi = BITMAPINFO {
            header: BITMAPINFOHEADER {
                bi_size: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                bi_width: w,
                bi_height: -h, // 自上而下
                bi_planes: 1,
                bi_bit_count: 32,
                bi_compression: 0,
                bi_size_image: 0,
                bi_x_pels: 0,
                bi_y_pels: 0,
                bi_clr_used: 0,
                bi_clr_important: 0,
            },
            colors: [0; 3],
        };
        let mut buf = vec![0u8; (w * h * 4) as usize];
        let lines = if ok != 0 {
            GetDIBits(
                mem,
                bmp,
                0,
                h as u32,
                buf.as_mut_ptr() as *mut c_void,
                &mut bi,
                DIB_RGB_COLORS,
            )
        } else {
            0
        };

        SelectObject(mem, old);
        DeleteObject(bmp);
        DeleteDC(mem);
        ReleaseDC(0, screen);

        if lines == 0 {
            None
        } else {
            Some((buf, w, h))
        }
    }
}

/// 采样外扩边距：锚点框右侧/上/下都是纯壁纸，用它估计背后底色。
fn ring_margin(scale: f32) -> i32 {
    let m = (7.0 * scale).round() as i32;
    m.clamp(5, 20)
}

/// 是否开启像素判定诊断日志（设置环境变量 `AUTOTAPPO_CHEST_DEBUG=1` 后生效）。
/// 用于实机校准 stage 2 的比例阈值，不设该变量时几乎零开销。
fn debug_on() -> bool {
    use std::sync::atomic::{AtomicU8, Ordering};
    static FLAG: AtomicU8 = AtomicU8::new(2); // 2=未初始化
    let v = FLAG.load(Ordering::Relaxed);
    if v != 2 {
        return v == 1;
    }
    let on = std::env::var_os("AUTOTAPPO_CHEST_DEBUG").is_some();
    FLAG.store(if on { 1 } else { 0 }, Ordering::Relaxed);
    on
}

/// 诊断日志文件路径：`%TEMP%\autotappo-chest-debug.log`
/// （本程序是 GUI 子系统，没有控制台，stderr 看不到，只能落盘）
fn debug_log_path() -> std::path::PathBuf {
    std::env::temp_dir().join("autotappo-chest-debug.log")
}

/// 写一行诊断日志；未开启诊断或写入失败时静默忽略（绝不影响主流程）
pub(super) fn debug_log(args: std::fmt::Arguments) {
    if !debug_on() {
        return;
    }
    use std::io::Write;
    let path = debug_log_path();
    // 超过 1 MB 就清空重来，避免长时间常驻把磁盘写满
    if let Ok(md) = std::fs::metadata(&path) {
        if md.len() > 1_048_576 {
            let _ = std::fs::remove_file(&path);
        }
    }
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        let _ = writeln!(f, "{}", args);
    }
}

/// 用「按钮框之外」的一圈像素估计壁纸底色（逐通道中位数，抗局部噪声）
fn background_estimate(buf: &[u8], w: i32, h: i32, inner: &Box2) -> Option<(i32, i32, i32)> {
    let mut rs: Vec<u8> = Vec::new();
    let mut gs: Vec<u8> = Vec::new();
    let mut bs: Vec<u8> = Vec::new();
    for y in 0..h {
        for x in 0..w {
            if x >= inner.l && x < inner.r && y >= inner.t && y < inner.b {
                continue; // 框内是按钮本身，不参与背景估计
            }
            let i = ((y * w + x) * 4) as usize;
            bs.push(buf[i]);
            gs.push(buf[i + 1]);
            rs.push(buf[i + 2]);
        }
    }
    if rs.len() < 20 {
        return None; // 框外像素不足（未外扩），放弃背景差分
    }
    rs.sort_unstable();
    gs.sort_unstable();
    bs.sort_unstable();
    let mid = rs.len() / 2;
    Some((rs[mid] as i32, gs[mid] as i32, bs[mid] as i32))
}

/// 就绪判定（两级）：
///
/// 阶段 1：绝对阈值 —— 100% 不透明度下的原始判据，与旧版行为完全一致；
/// 阶段 2：背景差分 —— 低不透明度时屏幕像素是 `a*C + (1-a)*B`（a=不透明度，B=壁纸），
///         用框外一圈壁纸估出 B，则 `d = screen - B = a*(C - B)`，
///         **d 的方向与 a、B 都无关**，据此判定按钮内容色相即可。
fn classify(buf: &[u8], w: i32, h: i32, inner: &Box2) -> Readiness {
    if buf.len() < (w * h * 4) as usize || inner.w() <= 0 || inner.h() <= 0 {
        return Readiness::Unknown;
    }
    let px = |x: i32, y: i32| -> (i32, i32, i32) {
        let i = ((y * w + x) * 4) as usize;
        (buf[i + 2] as i32, buf[i + 1] as i32, buf[i] as i32)
    };

    // 按钮框内再内缩 12%，排除圆角与阴影
    let inset_w = (inner.w() as f32 * 0.12) as i32;
    let inset_h = (inner.h() as f32 * 0.12) as i32;
    let x0 = inner.l + inset_w;
    let x1 = inner.r - inset_w;
    let y0 = inner.t + inset_h;
    let y1 = inner.b - inset_h;
    if x1 <= x0 || y1 <= y0 {
        return Readiness::Unknown;
    }

    // ---------- 阶段 1：绝对阈值（向后兼容 100% 不透明度） ----------
    let mut total: u32 = 0;
    let mut warm: u32 = 0;
    let mut white: u32 = 0;
    for y in y0..y1 {
        for x in x0..x1 {
            let (r, g, b) = px(x, y);
            total += 1;
            if r > 180 && (r - b) > 45 && (r - g) > 10 {
                warm += 1;
            }
            if r > 235 && g > 230 && b > 215 {
                white += 1;
            }
        }
    }
    if total == 0 {
        return Readiness::Unknown;
    }
    let warm_ratio = warm as f32 / total as f32;
    let white_ratio = white as f32 / total as f32;
    if debug_on() {
        debug_log(format_args!(
            "[chest] stage1 warm={:.3} white={:.3} box={}x{}",
            warm_ratio,
            white_ratio,
            inner.w(),
            inner.h()
        ));
    }
    if warm_ratio >= 0.30 && white_ratio < 0.15 {
        return Readiness::Ready;
    }
    if white_ratio >= 0.15 {
        return Readiness::Cooldown;
    }

    // ---------- 阶段 2：背景差分（兼容低不透明度） ----------
    let Some((bg_r, bg_g, bg_b)) = background_estimate(buf, w, h, inner) else {
        return Readiness::Unknown;
    };

    let mut total2: u32 = 0;
    let mut warm2: u32 = 0;
    let mut white2: u32 = 0;
    for y in y0..y1 {
        for x in x0..x1 {
            let (r, g, b) = px(x, y);
            let dr = r - bg_r;
            let dg = g - bg_g;
            let db = b - bg_b;
            let mag = dr.abs() + dg.abs() + db.abs();
            total2 += 1;
            if mag < 18 {
                continue; // 与壁纸几乎无差别：不透明度太低，或白底白钮
            }
            let m = mag as f32;
            // 就绪：暖橙，红显著高于蓝
            if (dr - db) as f32 >= 0.42 * m && dr > 0 {
                warm2 += 1;
            }
            // 冷却：奶白，三通道接近且整体比壁纸亮
            let mx = dr.max(dg).max(db);
            let mn = dr.min(dg).min(db);
            if (mx - mn) as f32 <= 0.22 * m && dr > 0 && dg > 0 {
                white2 += 1;
            }
        }
    }
    if total2 == 0 {
        return Readiness::Unknown;
    }
    let wr2 = warm2 as f32 / total2 as f32;
    let wh2 = white2 as f32 / total2 as f32;
    if debug_on() {
        debug_log(format_args!(
            "[chest] stage2 bg=({},{},{}) warm={:.3} white={:.3}",
            bg_r, bg_g, bg_b, wr2, wh2
        ));
    }
    if wr2 >= 0.28 && wh2 < 0.15 {
        Readiness::Ready
    } else if wh2 >= 0.18 {
        Readiness::Cooldown
    } else {
        Readiness::Unknown
    }
}

pub(super) fn readiness_of(anchor: &Anchor, origin: (i32, i32), pet: &Found) -> (Readiness, String) {
    let screen = Box2 {
        l: anchor.area.l + origin.0,
        t: anchor.area.t + origin.1,
        r: anchor.area.r + origin.0,
        b: anchor.area.b + origin.1,
    };
    if let Some(who) = is_occluded(&screen, pet) {
        return (Readiness::Unknown, format!("被遮挡: {who}"));
    }
    // 右/上/下外扩一圈纯壁纸，供低不透明度时估计背后底色；
    // 左侧不扩：锚点框左侧紧邻桌宠本体，扩进来会把本体像素混入背景估计。
    let m = ring_margin(anchor.scale);
    let outer = Box2 {
        l: screen.l,
        t: screen.t - m,
        r: screen.r + m,
        b: screen.b + m,
    };
    match capture(&outer) {
        Some((buf, w, h)) => {
            let inner = Box2 {
                l: screen.l - outer.l,
                t: screen.t - outer.t,
                r: screen.r - outer.l,
                b: screen.b - outer.t,
            };
            let state = classify(&buf, w, h, &inner);
            let text = match state {
                Readiness::Ready => "就绪",
                Readiness::Cooldown => "冷却中",
                Readiness::Unknown => "无法判定",
            };
            debug_log(format_args!("[chest] 判定 = {}", text));
            (state, text.to_string())
        }
        None => (Readiness::Unknown, "截取失败".into()),
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: i32, h: i32, rgb: (u8, u8, u8)) -> Vec<u8> {
        let mut buf = vec![0u8; (w * h * 4) as usize];
        for i in (0..buf.len()).step_by(4) {
            buf[i] = rgb.2;
            buf[i + 1] = rgb.1;
            buf[i + 2] = rgb.0;
            buf[i + 3] = 255;
        }
        buf
    }

    fn inner() -> Box2 {
        Box2 { l: 20, t: 20, r: 80, b: 80 }
    }

    #[test]
    fn warm_pixels_are_ready() {
        let buf = solid(100, 100, (255, 138, 61));
        assert_eq!(classify(&buf, 100, 100, &inner()), Readiness::Ready);
    }

    #[test]
    fn white_pixels_are_cooldown() {
        let buf = solid(100, 100, (245, 240, 230));
        assert_eq!(classify(&buf, 100, 100, &inner()), Readiness::Cooldown);
    }

    #[test]
    fn uniform_gray_is_unknown() {
        let buf = solid(100, 100, (128, 128, 128));
        assert_eq!(classify(&buf, 100, 100, &inner()), Readiness::Unknown);
    }

    #[test]
    fn short_buffer_is_unknown() {
        let buf = vec![0u8; 10];
        assert_eq!(classify(&buf, 100, 100, &inner()), Readiness::Unknown);
    }
}
