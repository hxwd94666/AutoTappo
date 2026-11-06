// AutoTappo room layout geometry/storage core.
// Pure functions shared by the injected page script and Node tests.
(function (root, factory) {
  if (typeof module === 'object' && module.exports) module.exports = factory();
  else root.__autotappoLayoutCore = factory();
})(typeof globalThis !== 'undefined' ? globalThis : this, function () {
  'use strict';

  const VERSION = 3;
  const KEY_PREFIX = 'autotappo-room-layout-v3:';
  const LEGACY_KEY_PREFIX = 'autotappo-room-layout-v1:';
  const MAX_MEMBERS = 500;
  const OVERLAP = 0.30;
  const GAP = 8;
  const MARGIN = 4;

  function clamp(value, min, max) {
    return Math.max(min, Math.min(max, value));
  }

  function finite(value) {
    return typeof value === 'number' && Number.isFinite(value);
  }

  function positive(value) {
    return finite(value) && value > 0;
  }

  function normalizeRect(rect, viewport) {
    const width = Math.max(1, Number(viewport && viewport.width) || 1);
    const height = Math.max(1, Number(viewport && viewport.height) || 1);
    if (!rect || !finite(rect.x) || !finite(rect.y) || !positive(rect.w) || !positive(rect.h)) {
      return null;
    }
    return {
      x: rect.x / width,
      y: rect.y / height,
      w: rect.w / width,
      h: rect.h / height
    };
  }

  function centerOf(entry) {
    return {
      x: (entry.x + entry.w / 2),
      y: (entry.y + entry.h / 2)
    };
  }

  function restoreCenter(entry, viewport) {
    const width = Math.max(1, Number(viewport && viewport.width) || 1);
    const height = Math.max(1, Number(viewport && viewport.height) || 1);
    const c = centerOf(entry);
    const w = entry.w * width;
    const h = entry.h * height;
    return {
      x: c.x * width - w / 2,
      y: c.y * height - h / 2,
      w,
      h
    };
  }

  function restoreRect(entry, viewport) {
    const width = Math.max(1, Number(viewport && viewport.width) || 1);
    const height = Math.max(1, Number(viewport && viewport.height) || 1);
    return {
      x: entry.x * width,
      y: entry.y * height,
      w: entry.w * width,
      h: entry.h * height
    };
  }

  function normalizeVisible(shell, visible, viewport) {
    const n = normalizeRect(visible, viewport);
    if (!n) return null;
    return n;
  }

  function sanitizeRect(value) {
    if (!value || !finite(value.x) || !finite(value.y) || !positive(value.w) || !positive(value.h)) {
      return null;
    }
    return { x: value.x, y: value.y, w: value.w, h: value.h };
  }

  function sanitizeEntry(value) {
    if (!value || !finite(value.x) || !finite(value.y) || !positive(value.w) || !positive(value.h)) {
      return null;
    }
    if (Math.abs(value.x) > 4 || Math.abs(value.y) > 4 || value.w > 4 || value.h > 4) {
      return null;
    }
    const out = {
      x: value.x,
      y: value.y,
      w: value.w,
      h: value.h,
      lastSeen: finite(value.lastSeen) && value.lastSeen > 0 ? value.lastSeen : 0
    };
    const visible = sanitizeRect(value.visible);
    if (visible) out.visible = visible;
    return out;
  }

  function emptyRoom() {
    return { version: VERSION, updatedAt: 0, members: {} };
  }

  function loadRoom(storage, key) {
    try {
      const raw = JSON.parse(storage.getItem(key) || 'null');
      if (!raw || raw.version !== VERSION || !raw.members || typeof raw.members !== 'object') {
        return emptyRoom();
      }
      const members = {};
      for (const [id, entry] of Object.entries(raw.members)) {
        if (!id) continue;
        const clean = sanitizeEntry(entry);
        if (clean) members[String(id)] = clean;
      }
      return {
        version: VERSION,
        updatedAt: finite(raw.updatedAt) ? raw.updatedAt : 0,
        members
      };
    } catch (e) {
      return emptyRoom();
    }
  }

  function migrateLegacy(raw, now) {
    const room = emptyRoom();
    if (!raw || typeof raw !== 'object') return room;
    const stamp = finite(now) ? now : Date.now();
    for (const [id, entry] of Object.entries(raw)) {
      if (!entry || !finite(entry.x) || !finite(entry.y) || !positive(entry.w) || !positive(entry.h)) continue;
      // v1 stored the normalized center; v3 stores the top-left shell rectangle.
      const shell = {
        x: entry.x - entry.w / 2,
        y: entry.y - entry.h / 2,
        w: entry.w,
        h: entry.h
      };
      const clean = sanitizeEntry({ ...shell, visible: shell, lastSeen: stamp });
      if (clean) room.members[String(id)] = clean;
    }
    room.updatedAt = stamp;
    return room;
  }

  function loadWithMigration(storage, key, legacyKey, now) {
    let hasV3 = false;
    try { hasV3 = storage.getItem(key) !== null; } catch (e) {}
    if (hasV3) return { room: loadRoom(storage, key), migrated: false };
    try {
      const raw = JSON.parse(storage.getItem(legacyKey) || 'null');
      if (raw && typeof raw === 'object') {
        return { room: migrateLegacy(raw, now), migrated: true };
      }
    } catch (e) {}
    return { room: emptyRoom(), migrated: false };
  }

  function saveRoom(storage, key, room) {
    const members = {};
    const rows = Object.entries(room && room.members ? room.members : {})
      .map(([id, entry]) => [id, sanitizeEntry(entry)])
      .filter(([, entry]) => !!entry)
      .sort((a, b) => (b[1].lastSeen || 0) - (a[1].lastSeen || 0))
      .slice(0, MAX_MEMBERS);
    for (const [id, entry] of rows) members[id] = entry;
    const out = { version: VERSION, updatedAt: Date.now(), members };
    try {
      storage.setItem(key, JSON.stringify(out));
      return true;
    } catch (e) {
      return false;
    }
  }

  function putMember(room, id, shell, visible, viewport, now) {
    if (!room || !id) return false;
    const n = normalizeRect(shell, viewport);
    if (!n) return false;
    const vn = normalizeVisible(shell, visible, viewport);
    const entry = {
      x: n.x,
      y: n.y,
      w: n.w,
      h: n.h,
      visible: vn || n,
      lastSeen: finite(now) ? now : Date.now()
    };
    room.members[String(id)] = entry;
    return true;
  }

  function restoreVisible(entry, viewport) {
    return restoreRect(entry.visible || entry, viewport);
  }

  function intersects(a, b) {
    return a.x < b.x + b.w && a.x + a.w > b.x && a.y < b.y + b.h && a.y + a.h > b.y;
  }

  function insideViewport(rect, viewport, margin) {
    const m = finite(margin) ? margin : MARGIN;
    const view = { x: m, y: m, w: Math.max(0, viewport.width - m * 2), h: Math.max(0, viewport.height - m * 2) };
    return intersects(rect, view);
  }

  function clampRect(rect, viewport, margin) {
    const m = finite(margin) ? margin : MARGIN;
    const width = Math.max(1, viewport.width);
    const height = Math.max(1, viewport.height);
    const w = Math.min(rect.w, width - m * 2);
    const h = Math.min(rect.h, height - m * 2);
    let x = rect.x;
    let y = rect.y;
    if (x + w > width - m) x = width - m - w;
    if (x < m) x = m;
    if (y + h > height - m) y = height - m - h;
    if (y < m) y = m;
    return { x, y, w, h };
  }

  function overlapRatio(candidate, obstacle) {
    const left = Math.max(candidate.x, obstacle.x);
    const top = Math.max(candidate.y, obstacle.y);
    const right = Math.min(candidate.x + candidate.w, obstacle.x + obstacle.w);
    const bottom = Math.min(candidate.y + candidate.h, obstacle.y + obstacle.h);
    const iw = right - left;
    const ih = bottom - top;
    if (iw <= 0 || ih <= 0) return 0;
    const area = Math.max(1, candidate.w * candidate.h);
    return (iw * ih) / area;
  }

  function blocked(candidate, obstacles, threshold) {
    const limit = finite(threshold) ? threshold : OVERLAP;
    for (const obstacle of obstacles || []) {
      if (!obstacle) continue;
      if (overlapRatio(candidate, obstacle) > limit) return true;
    }
    return false;
  }


  function findSlot(options) {
    const opts = options || {};
    const vp = opts.viewport || { width: 1, height: 1 };
    const own = opts.own || null;
    const size = opts.size || { w: 0, h: 0 };
    const w = Number(size.w);
    const h = Number(size.h);
    if (!(w > 0 && h > 0)) return null;
    const obstacles = opts.obstacles || [];
    const mode = opts.mode === 'vertical' ? 'vertical' : 'horizontal';
    const group = Math.max(1, Math.floor(Number(opts.groupCount) || 1));
    const gap = finite(opts.gap) ? opts.gap : GAP;
    const margin = finite(opts.margin) ? opts.margin : MARGIN;
    const capacityX = Math.max(1, Math.floor((vp.width - margin * 2 + gap) / (w + gap)));
    const capacityY = Math.max(1, Math.floor((vp.height - margin * 2 + gap) / (h + gap)));

    function free(rect) {
      return !blocked(rect, obstacles);
    }
    function fallback(x, y) {
      return clampRect({ x, y, w, h }, vp, margin);
    }

    if (mode === 'vertical') {
      const perCol = Math.min(capacityY, group);
      const total = perCol * h + (perCol - 1) * gap;
      const baseX = own ? own.x + own.w - w : vp.width - w - margin;
      const startX = clamp(baseX, margin, Math.max(margin, vp.width - w - margin));
      const top = own ? own.y - 14 : vp.height - margin;
      for (let col = 0; col < 16; col++) {
        const x = startX - col * (w + gap);
        if (x < margin) break;
        for (let row = 0; row < perCol; row++) {
          const y = top - h - row * (h + gap);
          if (y < margin) break;
          const rect = { x, y, w, h };
          if (free(rect)) return rect;
        }
      }
      const belowY = own ? own.y + own.h + 14 : vp.height - h - 24;
      for (let row = 0; row < 8; row++) {
        const y = belowY + row * (h + gap);
        if (y + h > vp.height - margin) break;
        for (let col = 0; col < capacityX; col++) {
          const rect = { x: margin + col * (w + gap), y, w, h };
          if (free(rect)) return rect;
        }
      }
      return fallback(startX, belowY);
    }

    const perRow = Math.min(capacityX, group);
    const total = perRow * w + (perRow - 1) * gap;
    const ownCenter = own ? own.x + own.w / 2 : vp.width / 2;
    const startX = clamp(ownCenter - total / 2, margin, Math.max(margin, vp.width - total - margin));
    const baseY = own ? own.y + own.h + 14 : vp.height - h - 24;
    for (let row = 0; row < 16; row++) {
      const y = baseY + row * (h + gap);
      if (y + h > vp.height - margin) break;
      for (let col = 0; col < perRow; col++) {
        const rect = { x: startX + col * (w + gap), y, w, h };
        if (free(rect)) return rect;
      }
    }
    const aboveY = own ? own.y - h - 14 : margin;
    for (let row = 0; row < 8; row++) {
      const y = aboveY - row * (h + gap);
      if (y < margin) break;
      for (let col = 0; col < perRow; col++) {
        const rect = { x: startX + col * (w + gap), y, w, h };
        if (free(rect)) return rect;
      }
    }
    return fallback(ownCenter - w / 2, baseY);
  }

  function buildDirectionSlots(options) {
    const opts = options || {};
    const own = opts.own || null;
    const size = opts.size || {};
    const vp = opts.viewport || { width: 1, height: 1 };
    const count = Math.max(1, Math.floor(Number(opts.count) || 19));
    if (!own || !(size.w > 0 && size.h > 0)) return [];
    const w = Number(size.w);
    const h = Number(size.h);
    const gap = finite(opts.gap) ? opts.gap : GAP;
    const margin = finite(opts.margin) ? opts.margin : MARGIN;
    const mode = opts.mode === 'h-left' || opts.mode === 'v-up' || opts.mode === 'v-down' ? opts.mode : 'h-right';
    const horizontal = mode === 'h-left' || mode === 'h-right';
    let wrapItems = Math.max(0, Math.floor(Number(opts.wrap) || 0));
    if (wrapItems === 1) wrapItems = 2;
    const wrapDir = Number(opts.wrapDir) === 1 ? 1 : 0;
    const explicit = wrapItems > 1 ? wrapItems : 0;
    const stepX = w + gap;
    const stepY = h + gap;
    const lineStepX = horizontal ? 0 : (wrapDir === 1 ? -stepX : stepX);
    const lineStepY = horizontal ? (wrapDir === 1 ? -stepY : stepY) : 0;
    const firstX = mode === 'h-right' ? own.x + own.w + gap : (mode === 'h-left' ? own.x - gap - w : own.x);
    const firstY = mode === 'v-up' ? own.y - gap - h : (mode === 'v-down' ? own.y + own.h + gap : own.y);
    function capacityAt(start, dir) {
      if (horizontal) {
        if (dir > 0) return start + w <= vp.width - margin
          ? Math.floor((vp.width - margin - start - w) / stepX) + 1 : 0;
        return start >= margin ? Math.floor((start - margin) / stepX) + 1 : 0;
      }
      if (dir > 0) return start + h <= vp.height - margin
        ? Math.floor((vp.height - margin - start - h) / stepY) + 1 : 0;
      return start >= margin ? Math.floor((start - margin) / stepY) + 1 : 0;
    }
    const primaryX = horizontal ? (mode === 'h-left' ? -1 : 1) : 0;
    const primaryY = horizontal ? 0 : (mode === 'v-up' ? -1 : 1);
    const capFirst = horizontal
      ? capacityAt(firstX, primaryX)
      : capacityAt(firstY, primaryY);
    const capLater = horizontal
      ? capacityAt(own.x, primaryX)
      : capacityAt(own.y, primaryY);
    if (capFirst < 1) {
      const opposite = { 'h-right': 'h-left', 'h-left': 'h-right', 'v-up': 'v-down', 'v-down': 'v-up' }[mode];
      if (opposite && opposite !== mode) {
        return buildDirectionSlots(Object.assign({}, opts, { mode: opposite }));
      }
    }
    const slots = [];
    let line = 0;
    while (slots.length < count && line < 64) {
      const isFirst = line === 0;
      const startX = isFirst ? firstX : own.x;
      const startY = isFirst ? firstY : own.y;
      const lineCap = isFirst ? capFirst : capLater;
      const requested = explicit
        ? (isFirst ? Math.max(1, explicit - 1) : explicit)
        : lineCap;
      const maxLine = Math.max(0, Math.min(requested, lineCap));
      for (let pos = 0; pos < maxLine && slots.length < count; pos++) {
        const x = startX + (horizontal ? primaryX * pos * stepX : line * lineStepX);
        const y = startY + (horizontal ? line * lineStepY : primaryY * pos * stepY);
        if (x + w < margin || x > vp.width - margin || y + h < margin || y > vp.height - margin) continue;
        slots.push({ x, y, w, h });
      }
      line += 1;
      if (maxLine === 0 && !explicit && line > 2) break;
    }
    return slots;
  }



  function assignSlots(assignments, onlineIds, slotCount) {
    const map = new Map();
    const src = assignments && typeof assignments === 'object' ? assignments : {};
    for (const [id, raw] of Object.entries(src)) {
      const idx = Number(raw);
      if (id && Number.isInteger(idx) && idx >= 0) map.set(String(id), idx);
    }
    const ids = [...new Set((onlineIds || []).map(String).filter(Boolean))];
    const online = new Set(ids);
    for (const id of [...map.keys()]) if (!online.has(id)) map.delete(id);
    let count = Math.max(1, Math.floor(Number(slotCount) || 19));
    for (const idx of map.values()) if (idx >= count) count = idx + 1;
    for (;;) {
      const used = new Set(map.values());
      let low = -1;
      for (let i = 0; i < count; i++) if (!used.has(i)) { low = i; break; }
      if (low < 0) break;
      let high = -1;
      let highId = '';
      for (const [id, idx] of map) {
        if (idx > low && idx > high) { high = idx; highId = id; }
      }
      if (high < 0) break;
      map.set(highId, low);
    }
    for (const id of ids) {
      if (map.has(id)) continue;
      const used = new Set(map.values());
      let idx = -1;
      for (let i = 0; i < count; i++) if (!used.has(i)) { idx = i; break; }
      if (idx < 0) { idx = count; count += 1; }
      map.set(id, idx);
    }
    return { assignments: Object.fromEntries(map), slotCount: count };
  }

  return {
    VERSION,
    KEY_PREFIX,
    LEGACY_KEY_PREFIX,
    MAX_MEMBERS,
    OVERLAP,
    GAP,
    MARGIN,
    clamp,
    finite,
    normalizeRect,
    restoreCenter,
    restoreRect,
    restoreVisible,
    normalizeVisible,
    sanitizeEntry,
    emptyRoom,
    loadRoom,
    loadWithMigration,
    migrateLegacy,
    saveRoom,
    putMember,
    intersects,
    insideViewport,
    clampRect,
    findSlot,
    buildDirectionSlots,
    assignSlots,
    overlapRatio,
    blocked
  };
});
