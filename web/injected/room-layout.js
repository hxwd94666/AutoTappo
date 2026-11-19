// 自动排列好友桌宠：包装官方 index.render，只改 vnode 的 left/top。
// 由 AutoTappo 的 arrange worker 通过 CDP 注入；几何/存储核心由 room-layout-core.js 提供。
(() => {
  'use strict';
  const VERSION = '16';
  if (window.__autotappoArrange) {
    if (window.__autotappoArrange.version === VERSION) {
      try { window.__autotappoArrange.setEnabled(true); } catch (e) {}
      return;
    }
    try { window.__autotappoArrange.setEnabled(false); } catch (e) {}
    try { window.__autotappoArrange.setEdit(false); } catch (e) {}
  }

  const CORE = window.__autotappoLayoutCore;
  if (!CORE) return;

  const GAP = CORE.GAP;
  const SLOT_PREFIX = 'autotappo-room-layout-v8:';
  const MANUAL_PREFIX = 'autotappo-room-manual-v9:';
  const SLOT_COUNT = 19;
  const DIRECTIONS = ['h-left', 'h-right', 'v-up', 'v-down'];
  const state = {
    enabled: false,
    edit: false,
    idx: null,
    orig: null,
    overrides: new Map(),
    key: '',
    profiles: { version: 8, modes: {} },
    assignments: new Map(),
    layoutMode: DIRECTIONS.includes(window.__autotappoLayoutMode)
      ? window.__autotappoLayoutMode
      : (window.__autotappoLayoutMode === 'overlap' ? 'overlap' : 'h-right'),
    wrap: Math.max(0, Math.min(20, Number(window.__autotappoLayoutWrap) || 0)),
    wrapDir: Number(window.__autotappoLayoutWrapDir) === 1 ? 1 : 0,
    manualMode: window.__autotappoLayoutManual !== false,
    manualKey: '',
    manualSlots: [],
    rosterIds: new Set(),
    alignMode: false,
    rebuildSlots: true,
    official: false,
    lastIds: '',
    timer: 0,
    overlapRaf: 0
  };

  function vueApp() {
    return document.querySelector('#app')?.__vue_app__ || null;
  }
  function findIndex() {
    const app = vueApp();
    const root = app?._container?._vnode?.component;
    return root?.subTree?.component?.subTree?.component || null;
  }
  function identity() {
    const app = vueApp();
    const pinia = app?.config?.globalProperties?.$pinia;
    const gs = pinia?._s?.get('GlobalState');
    const account = Number(gs?.userInfo?.id || gs?.userInfo?.uid || 0) || 0;
    const room = String(gs?.roomNo || gs?.userInfo?.room_no || '').trim();
    return { account, room };
  }
  function slotKeyOf(id) {
    return SLOT_PREFIX + id.account + ':' + id.room;
  }
  function emptyProfiles() {
    return { version: 8, modes: {} };
  }
  function loadProfiles() {
    if (!state.key) return emptyProfiles();
    try {
      const raw = JSON.parse(localStorage.getItem(state.key) || 'null');
      if (raw && (raw.version === 5 || raw.version === 8) && raw.modes && typeof raw.modes === 'object') {
        const modes = {};
        for (const mode of DIRECTIONS) {
          const p = raw.modes[mode];
          if (!p || !Array.isArray(p.slots)) continue;
          const slots = p.slots.filter((s) => s && Number.isFinite(Number(s.x)) && Number.isFinite(Number(s.y)))
            .map((s) => ({ x: Number(s.x), y: Number(s.y) }));
          modes[mode] = {
            anchor: p.anchor && Number.isFinite(Number(p.anchor.x)) && Number.isFinite(Number(p.anchor.y))
              ? { x: Number(p.anchor.x), y: Number(p.anchor.y) }
              : null,
            wrap: Math.max(0, Math.min(20, Number(p.wrap) || 0)),
            wrapDir: Number(p.wrapDir) === 1 ? 1 : 0,
            manual: !!p.manual,
            slots
          };
        }
        return { version: 8, modes };
      }
    } catch (e) {}
    return emptyProfiles();
  }

  function manualKeyOf(id) {
    return MANUAL_PREFIX + id.account + ':' + id.room;
  }
  function loadManual() {
    const id = identity();
    const key = manualKeyOf(id);
    if (state.manualKey !== key) {
      state.manualKey = key;
      state.manualSlots = [];
      try {
        const raw = JSON.parse(localStorage.getItem(key) || 'null');
        if (raw && raw.version === 9 && Array.isArray(raw.slots)) {
          state.manualSlots = raw.slots.filter((s) => s && Number.isFinite(Number(s.x)) && Number.isFinite(Number(s.y)))
            .map((s) => ({ x: Number(s.x), y: Number(s.y), w: Number(s.w) || 100, h: Number(s.h) || 106 }));
        }
      } catch (e) {}
    }
    return state.manualSlots;
  }
  function saveManual() {
    if (!state.manualKey) return;
    try { localStorage.setItem(state.manualKey, JSON.stringify({ version: 9, slots: state.manualSlots })); } catch (e) {}
  }
  function manualCenter(slot) {
    return { x: slot.x + (slot.w || 0) / 2, y: slot.y + (slot.h || 0) / 2 };
  }
  function reindexManual(own) {
    if (!own || !state.manualSlots.length) return;
    const owner = new Map();
    for (const [id, idx] of state.assignments) owner.set(idx, id);
    const oc = manualCenter({ x: own.visible.x, y: own.visible.y, w: own.visible.w, h: own.visible.h });
    const records = state.manualSlots.map((slot, i) => ({ slot, id: owner.get(i) || null, c: manualCenter(slot) }));
    records.sort((a, b) => {
      const da = (a.c.x - oc.x) ** 2 + (a.c.y - oc.y) ** 2;
      const db = (b.c.x - oc.x) ** 2 + (b.c.y - oc.y) ** 2;
      if (Math.abs(da - db) > 0.5) return da - db;
      if (Math.abs(a.slot.y - b.slot.y) > 0.5) return a.slot.y - b.slot.y;
      return a.slot.x - b.slot.x;
    });
    state.manualSlots = records.map((r) => r.slot);
    state.assignments = new Map();
    records.forEach((r, i) => { if (r.id) state.assignments.set(r.id, i); });
  }
  function syncManual(list) {
    const online = new Set(list.map((c) => c.id));
    for (const id of [...state.assignments.keys()]) if (!online.has(id)) state.assignments.delete(id);
    const used = new Set(state.assignments.values());
    for (const c of list) {
      if (state.assignments.has(c.id)) continue;
      let idx = -1;
      for (let i = 0; i < state.manualSlots.length; i++) if (!used.has(i)) { idx = i; break; }
      if (idx < 0) continue;
      state.assignments.set(c.id, idx);
      used.add(idx);
    }
  }
  function applyManual() {
    if (!state.enabled || state.edit) return;
    const idx = findIndex();
    if (!idx) return;
    const own = ownRects();
    const list = cats();
    if (!own || !list.length) return;
    const slots = loadManual();
    if (!slots.length) return;
    if (state.official) restoreOfficial();
    syncManual(list);
    if (!state.assignments.size) return;
    reindexManual(own);
    state.lastIds = list.map((c) => c.id).join(',');
    for (const c of list) settledIds.add(c.id);
    const placed = [];
    for (const c of list) {
      const si = state.assignments.get(c.id);
      const slot = si == null ? null : state.manualSlots[si];
      if (!slot) continue;
      placed.push({ c, shell: { x: slot.x, y: slot.y, w: c.shell.w, h: c.shell.h } });
    }
    if (!placed.length) return;
    if (applyOfficial(placed)) return;
    state.official = false;
    const next = new Map();
    for (const item of placed) {
      next.set(item.c.id, {
        left: item.c.left + (item.shell.x - item.c.shell.x),
        top: item.c.top + (item.shell.y - item.c.shell.y)
      });
    }
    state.overrides = next;
    if (state.idx !== idx) {
      if (state.orig && state.idx) { try { state.idx.render = state.orig; } catch (e) {} }
      state.idx = idx;
      state.orig = idx.render;
    }
    if (!state.orig) return;
    idx.render = function (...args) {
      const vnode = state.orig.apply(this, args);
      patchVNode(vnode);
      return vnode;
    };
    idx.update();
  }
  function viewport() {
    return {
      width: Math.max(1, window.innerWidth),
      height: Math.max(1, window.innerHeight)
    };
  }
  function ownRects() {
    const el = document.querySelector('.pet-dock-content');
    if (!el) return null;
    const r = el.getBoundingClientRect();
    if (!(r.width > 20 && r.height > 20)) return null;
    const shell = { x: r.x, y: r.y, w: r.width, h: r.height };
    return { el, shell, visible: visibleRectOf(el, shell) };
  }
  function visibleRectOf(el, shell) {
    let best = null;
    let bestArea = 0;
    for (const node of el.querySelectorAll('canvas, img, video')) {
      const r = node.getBoundingClientRect();
      const area = r.width * r.height;
      if (r.width > 10 && r.height > 10 && area > bestArea) {
        best = { x: r.x, y: r.y, w: r.width, h: r.height };
        bestArea = area;
      }
    }
    return best || shell;
  }
  function cats() {
    return [...document.querySelectorAll('.room-cat-shell[data-room-cat-id]')].map((el) => {
      const r = el.getBoundingClientRect();
      const shell = { x: r.x, y: r.y, w: r.width, h: r.height };
      return {
        id: String(el.dataset.roomCatId || ''),
        el,
        left: parseFloat(el.style.left) || 0,
        top: parseFloat(el.style.top) || 0,
        shell,
        visible: visibleRectOf(el, shell)
      };
    }).filter((c) => c.id && c.shell.w > 10 && c.shell.h > 10);
  }
  function patchVNode(v) {
    if (!v) return;
    const id = v.props && v.props['data-room-cat-id'] ? String(v.props['data-room-cat-id']) : '';
    if (id && state.overrides.has(id)) {
      const p = state.overrides.get(id);
      const style = {
        left: p.left + 'px',
        top: p.top + 'px'
      };
      if (p.zIndex) style.zIndex = String(p.zIndex);
      v.props.style = Object.assign({}, typeof v.props.style === 'object' && v.props.style ? v.props.style : {}, style);
    }
    if (v.component) patchVNode(v.component.subTree);
    if (Array.isArray(v.children)) v.children.forEach(patchVNode);
    if (v.suspense) patchVNode(v.suspense.activeBranch);
  }
  function refresh() {
    window.dispatchEvent(new Event('resize'));
    window.dispatchEvent(new Event('cat-main-window-bounds-changed'));
  }
  function restore() {
    if (state.idx && state.orig) {
      try { state.idx.render = state.orig; } catch (e) {}
      try { state.idx.update(); } catch (e) {}
    }
    state.idx = null;
    state.orig = null;
    state.overrides.clear();
  }

  function officialIndex() {
    try {
      const app = document.querySelector('#app')?.__vue_app__;
      const root = app?._container?._vnode?.component;
      const router = root?.subTree?.component;
      return router?.subTree?.component || null;
    } catch (e) { return null; }
  }
  function officialUpdate() {
    const idx = officialIndex();
    if (idx) { try { idx.update(); } catch (e) {} }
  }
  function detachCss() {
    if (state.idx && state.orig) {
      try { state.idx.render = state.orig; } catch (e) {}
    }
    state.idx = null;
    state.orig = null;
    state.overrides.clear();
  }
  function applyOfficial(placed) {
    const api = window.__autotappoRoomApi;
    if (!api || !api.members || !Array.isArray(api.members.value) || typeof api.lookup !== 'function') return false;
    detachCss();
    officialUpdate();
    const byId = new Map(api.members.value.map((m) => [String(m.id), m]));
    const current = new Map(cats().map((c) => [c.id, c]));
    let changed = false;
    for (const item of placed) {
      const m = byId.get(item.c.id);
      const c = current.get(item.c.id);
      if (!m || !c) continue;
      let ref;
      try { ref = api.lookup(m); } catch (e) { continue; }
      const v = ref && ref.value;
      if (!v) continue;
      v.left += item.shell.x - c.shell.x;
      v.top += item.shell.y - c.shell.y;
      changed = true;
    }
    if (!changed) return false;
    officialUpdate();
    state.official = true;
    setTimeout(refresh, 20);
    return true;
  }
  function restoreOfficial() {
    const api = window.__autotappoRoomApi;
    if (api && api.members && Array.isArray(api.members.value) && typeof api.lookup === 'function') {
      for (const m of api.members.value) {
        let ref;
        try { ref = api.lookup(m); } catch (e) { continue; }
        const v = ref && ref.value;
        if (!v) continue;
        v.left = m.left;
        v.top = m.top;
      }
      officialUpdate();
    }
    state.official = false;
    state.overrides.clear();
    refresh();
  }

  function sampleCat(list) {
    return list.find((c) => c.shell.w > 10 && c.shell.h > 10) || null;
  }
  function averageGap(list, mode) {
    if (!list || list.length < 2) return CORE.GAP;
    const horizontal = mode === 'h-left' || mode === 'h-right';
    const groups = new Map();
    for (const c of list) {
      const axis = horizontal ? c.shell.y : c.shell.x;
      const key = Math.round(axis / 60);
      if (!groups.has(key)) groups.set(key, []);
      groups.get(key).push(c);
    }
    const gaps = [];
    for (const group of groups.values()) {
      if (group.length < 2) continue;
      group.sort((a, b) => (horizontal ? a.shell.x - b.shell.x : a.shell.y - b.shell.y));
      for (let i = 1; i < group.length; i++) {
        const prev = group[i - 1];
        const cur = group[i];
        const center = horizontal
          ? Math.abs((cur.shell.x + cur.shell.w / 2) - (prev.shell.x + prev.shell.w / 2))
          : Math.abs((cur.shell.y + cur.shell.h / 2) - (prev.shell.y + prev.shell.h / 2));
        const size = horizontal
          ? (cur.shell.w + prev.shell.w) / 2
          : (cur.shell.h + prev.shell.h) / 2;
        gaps.push(center - size);
      }
    }
    if (!gaps.length) return CORE.GAP;
    const avg = gaps.reduce((a, b) => a + b, 0) / gaps.length;
    return Math.max(CORE.GAP, Math.min(200, avg));
  }
  function buildProfile(mode, own, list, wrap, count, gap) {
    const sample = sampleCat(list);
    if (!sample || !own) return null;
    const visibles = CORE.buildDirectionSlots({
      mode,
      own: own.visible,
      size: { w: sample.visible.w, h: sample.visible.h },
      viewport: viewport(),
      wrap: wrap || 0,
      wrapDir: state.wrapDir,
      gap: Number.isFinite(gap) ? gap : undefined,
      count: count || SLOT_COUNT
    });
    return {
      anchor: { x: own.shell.x, y: own.shell.y },
      wrap: wrap || 0,
      wrapDir: state.wrapDir,
      manual: false,
      slots: visibles.map((v) => ({
        x: v.x - (sample.visible.x - sample.shell.x),
        y: v.y - (sample.visible.y - sample.shell.y)
      }))
    };
  }
  function alignProfile(profile, own) {
    if (!profile.anchor) {
      profile.anchor = { x: own.shell.x, y: own.shell.y };
      return;
    }
    const dx = own.shell.x - profile.anchor.x;
    const dy = own.shell.y - profile.anchor.y;
    if (Math.abs(dx) < 0.5 && Math.abs(dy) < 0.5) return;
    for (const s of profile.slots) { s.x += dx; s.y += dy; }
    profile.anchor = { x: own.shell.x, y: own.shell.y };
  }
  function ensureProfile(mode, own, list) {
    if (!sampleCat(list) || !own) return null;
    let profile = state.profiles.modes[mode];
    if (!profile) {
      profile = buildProfile(mode, own, list, state.wrap, SLOT_COUNT);
      state.profiles.modes[mode] = profile;
    } else if (profile.wrap !== state.wrap || profile.wrapDir !== state.wrapDir) {
      profile = buildProfile(mode, own, list, state.wrap, SLOT_COUNT, averageGap(list, mode));
      state.profiles.modes[mode] = profile;
    }
    while (profile && profile.slots.length < SLOT_COUNT) {
      const full = buildProfile(mode, own, list, state.wrap, profile.slots.length + 1);
      const next = full && full.slots[profile.slots.length];
      if (!next) break;
      profile.slots.push(next);
    }
    return profile;
  }
  function appendProfileSlot(profile, mode, own, list) {
    const target = profile.slots.length + 1;
    const full = buildProfile(mode, own, list, state.wrap, target);
    const next = full && full.slots[target - 1];
    if (!next) return false;
    profile.slots.push(next);
    return true;
  }
  function reconcileAssignments(list, profile, mode, own) {
    const result = CORE.assignSlots(
      Object.fromEntries(state.assignments),
      list.map((c) => c.id),
      Math.max(profile.slots.length, SLOT_COUNT)
    );
    state.assignments = new Map(Object.entries(result.assignments));
    while (profile.slots.length < result.slotCount) {
      if (!appendProfileSlot(profile, mode, own, list)) break;
    }
    return profile;
  }



  function overlapOverrides(own, list) {
    const next = new Map();
    for (const c of list) {
      next.set(c.id, {
        left: c.left + (own.shell.x - c.shell.x),
        top: c.top + (own.shell.y - c.shell.y),
        zIndex: 5
      });
    }
    return next;
  }

  function startOverlap() {
    if (state.overlapRaf) return;
    let lastX = null;
    let lastY = null;
    const frame = () => {
      state.overlapRaf = 0;
      if (!state.enabled || state.edit || state.layoutMode !== 'overlap') {
        stopOverlap();
        return;
      }
      const own = ownRects();
      if (own) {
        const moved = lastX === null || lastY === null ||
          Math.abs(own.shell.x - lastX) > 0.5 || Math.abs(own.shell.y - lastY) > 0.5;
        if (moved) {
          const idx = findIndex();
          if (idx && state.idx === idx && state.orig) {
            const list = cats();
            if (list.length) {
              state.overrides = overlapOverrides(own, list);
              state.lastIds = list.map((c) => c.id).join(',');
              for (const c of list) settledIds.add(c.id);
              idx.update();
            }
          } else if (idx) {
            applyOverlap();
            return;
          }
          lastX = own.shell.x;
          lastY = own.shell.y;
        }
      }
      state.overlapRaf = window.requestAnimationFrame(frame);
    };
    state.overlapRaf = window.requestAnimationFrame(frame);
  }

  function stopOverlap() {
    if (state.overlapRaf) {
      window.cancelAnimationFrame(state.overlapRaf);
      state.overlapRaf = 0;
    }
    restore();
  }

  function applyOverlap() {
    if (!state.enabled || state.edit) return;
    const idx = findIndex();
    if (!idx) return;
    const id = identity();
    const nextKey = slotKeyOf(id);
    if (nextKey !== state.key) {
      state.key = nextKey;
      state.profiles = loadProfiles();
      state.assignments.clear();
      state.overrides.clear();
      state.lastIds = '';
    }
    const own = ownRects();
    if (!own) return;
    const list = cats();
    if (!list.length) return;
    state.official = false;
    detachCss();
    state.idx = idx;
    state.orig = idx.render;
    state.overrides = overlapOverrides(own, list);
    state.lastIds = list.map((c) => c.id).join(',');
    for (const c of list) settledIds.add(c.id);
    idx.render = function (...args) {
      const vnode = state.orig.apply(this, args);
      patchVNode(vnode);
      return vnode;
    };
    idx.update();
    startOverlap();
  }



  function applyCurrent() {
    if (state.manualMode) applyManual();
    else if (state.layoutMode === 'overlap') applyOverlap();
    else apply();
  }

  function apply() {
    if (state.manualMode) {
      applyManual();
      return;
    }
    if (state.layoutMode === 'overlap') {
      applyOverlap();
      return;
    }
    if (!state.enabled || state.edit) return;
    const idx = findIndex();
    if (!idx) return;
    const id = identity();
    const nextKey = slotKeyOf(id);
    if (nextKey !== state.key) {
      state.key = nextKey;
      state.profiles = loadProfiles();
      state.assignments.clear();
      state.overrides.clear();
      state.lastIds = '';
      state.rosterIds = new Set();
      state.alignMode = true;
      state.rebuildSlots = true;
    }
    const own = ownRects();
    if (!own) return;
    const list = cats();
    if (!list.length) return;
    const mode = DIRECTIONS.includes(state.layoutMode) ? state.layoutMode : 'h-right';
    let profile = ensureProfile(mode, own, list);
    if (!profile || !profile.slots.length) return;
    if (state.rebuildSlots) {
      profile = buildProfile(mode, own, list, state.wrap, SLOT_COUNT, averageGap(list, mode));
      state.profiles.modes[mode] = profile;
      state.alignMode = false;
    } else if (state.alignMode) {
      alignProfile(profile, own);
      state.alignMode = false;
    }
    state.rebuildSlots = false;
    const priorIds = state.rosterIds;
    const joined = list.filter((c) => !priorIds.has(c.id));
    const oldSlotCount = profile.slots.length;
    reconcileAssignments(list, profile, mode, own);
    if (priorIds.size && joined.length) {
      const oldList = list.filter((c) => priorIds.has(c.id));
      const gap = averageGap(oldList, mode);
      for (const c of joined) {
        const slotIndex = state.assignments.get(c.id);
        if (slotIndex == null || slotIndex < oldSlotCount) continue;
        const full = buildProfile(mode, own, list, state.wrap, slotIndex + 1, gap);
        const next = full && full.slots[slotIndex];
        if (next) profile.slots[slotIndex] = next;
      }
    }
    state.rosterIds = new Set(list.map((c) => c.id));
    state.lastIds = list.map((c) => c.id).join(',');
    for (const c of list) settledIds.add(c.id);
    const placed = [];
    for (const c of list) {
      const slotIndex = state.assignments.get(c.id);
      const slot = slotIndex == null ? null : profile.slots[slotIndex];
      if (!slot) continue;
      placed.push({
        c,
        shell: { x: slot.x, y: slot.y, w: c.shell.w, h: c.shell.h }
      });
    }
    if (!placed.length) return;
    if (applyOfficial(placed)) return;
    state.official = false;
    const next = new Map();
    for (const item of placed) {
      next.set(item.c.id, {
        left: item.c.left + (item.shell.x - item.c.shell.x),
        top: item.c.top + (item.shell.y - item.c.shell.y)
      });
    }
    state.overrides = next;
    if (state.idx !== idx) {
      if (state.orig && state.idx) { try { state.idx.render = state.orig; } catch (e) {} }
      state.idx = idx;
      state.orig = idx.render;
    }
    if (!state.orig) return;
    idx.render = function (...args) {
      const vnode = state.orig.apply(this, args);
      patchVNode(vnode);
      return vnode;
    };
    idx.update();
    setTimeout(refresh, 20);
  }

  function tick() {
    state.timer = window.setTimeout(tick, 700);
    if (!state.enabled || state.edit) return;
    const idx = findIndex();
    if (!idx) return;
    const id = identity();
    const k = slotKeyOf(id);
    const ids = cats().map((c) => c.id).join(',');
    if (state.manualMode) {
      if (k !== state.key || ids !== state.lastIds) applyManual();
      return;
    }
    if (state.layoutMode === 'overlap') {
      if (k !== state.key || ids !== state.lastIds || !state.overlapRaf) applyOverlap();
      return;
    }
    const hasApi = !!window.__autotappoRoomApi;
    const modePending = hasApi && !state.official;
    const cssPending = !state.official && (!state.overrides.size || state.idx !== idx);
    if (!hasApi) state.official = false;
    if (modePending || cssPending || k !== state.key || ids !== state.lastIds) {
      state.lastIds = ids;
      apply();
    }
  }
  function setEnabled(on) {
    if (!on) {
      if (!state.enabled) return;
      state.enabled = false;
      state.edit = false;
      stopOverlap();
      if (state.official) restoreOfficial(); else restore();
      refresh();
      return;
    }
    if (state.enabled) return;
    state.enabled = true;
    state.edit = false;
    state.lastIds = '';
    applyCurrent();
  }
  function setEdit(on) {
    if (!state.enabled) return;
    if (!state.manualMode) return;
    if (state.edit === !!on) return;
    state.edit = !!on;
    if (state.edit) {
      restore();
      refresh();
      return;
    }
    const own = ownRects();
    const list = cats();
    if (own && list.length) {
      const slots = state.manualSlots.slice();
      const used = new Set(state.assignments.values());
      for (const c of list) {
        let slotIndex = state.assignments.get(c.id);
        if (slotIndex == null) {
          for (let i = 0; i < slots.length; i++) if (!used.has(i)) { slotIndex = i; break; }
          if (slotIndex == null && slots.length < SLOT_COUNT) {
            slotIndex = slots.length;
            slots.push({ x: c.shell.x, y: c.shell.y, w: c.shell.w, h: c.shell.h });
          }
          if (slotIndex == null) continue;
          state.assignments.set(c.id, slotIndex);
          used.add(slotIndex);
        }
        if (slots[slotIndex]) slots[slotIndex] = { x: c.shell.x, y: c.shell.y, w: c.shell.w, h: c.shell.h };
      }
      state.manualSlots = slots;
      reindexManual(own);
      saveManual();
    }
    state.overrides.clear();
    state.lastIds = '';
    applyManual();
  }
  function setMode(mode) {
    const next = DIRECTIONS.includes(mode) ? mode : (mode === 'overlap' ? 'overlap' : 'h-right');
    if (state.layoutMode === next) return next;
    state.layoutMode = next;
    if (state.enabled && !state.edit) {
      if (next === 'overlap') {
        state.lastIds = '';
        applyOverlap();
      } else {
        stopOverlap();
        state.lastIds = '';
        state.alignMode = true;
        state.rebuildSlots = true;
        apply();
      }
    }
    return next;
  }
  function setWrap(value) {
    const wrap = Math.max(0, Math.min(20, Math.floor(Number(value) || 0)));
    if (state.wrap === wrap) return wrap;
    state.wrap = wrap;
    if (state.enabled && !state.edit && state.layoutMode !== 'overlap') {
      const mode = DIRECTIONS.includes(state.layoutMode) ? state.layoutMode : 'h-right';
      state.profiles.modes[mode] = null;
      state.lastIds = '';
      applyCurrent();
    }
    return wrap;
  }
  function setManual(value) {
    const manual = !!value;
    if (state.manualMode === manual) return manual;
    state.manualMode = manual;
    state.edit = false;
    state.lastIds = '';
    if (manual) {
      loadManual();
      applyManual();
    } else {
      state.rebuildSlots = true;
      applyCurrent();
    }
    return manual;
  }

  function setWrapDir(value) {
    const dir = Number(value) === 1 ? 1 : 0;
    if (state.wrapDir === dir) return dir;
    state.wrapDir = dir;
    if (state.enabled && !state.edit && state.layoutMode !== 'overlap') {
      const mode = DIRECTIONS.includes(state.layoutMode) ? state.layoutMode : 'h-right';
      state.profiles.modes[mode] = null;
      state.lastIds = '';
      applyCurrent();
    }
    return dir;
  }


  const pendingReveal = new Set();
  const settledIds = new Set();
  let pendingStyleReady = false;

  function ensurePendingStyle() {
    if (pendingStyleReady) return;
    pendingStyleReady = true;
    const style = document.createElement('style');
    style.id = 'autotappo-pending-style';
    style.textContent = '.room-cat-shell[data-autotappo-pending="1"] { visibility: hidden !important; }';
    (document.head || document.documentElement).appendChild(style);
  }

  function revealPending() {
    for (const el of pendingReveal) el.removeAttribute('data-autotappo-pending');
    pendingReveal.clear();
  }

  function hideCat(el) {
    if (!el || !el.isConnected) return false;
    if (el.getAttribute('data-autotappo-pending') === '1') return false;
    el.setAttribute('data-autotappo-pending', '1');
    pendingReveal.add(el);
    return true;
  }

  function markNewCats(node) {
    if (!(node instanceof Element)) return false;
    const found = [];
    if (node.matches('.room-cat-shell[data-room-cat-id]')) found.push(node);
    if (node.querySelectorAll) {
      found.push(...node.querySelectorAll('.room-cat-shell[data-room-cat-id]'));
    }
    let marked = false;
    for (const el of found) {
      const id = String(el.dataset.roomCatId || '');
      if (!id || settledIds.has(id)) continue;
      marked = hideCat(el) || marked;
    }
    return marked;
  }

  function isCatVisible(el) {
    if (!el || !el.isConnected) return false;
    const r = el.getBoundingClientRect();
    if (r.width <= 10 || r.height <= 10) return false;
    const style = getComputedStyle(el);
    return style.display !== 'none' && style.visibility !== 'hidden' && Number(style.opacity) > 0.01;
  }

  function pruneSettled() {
    const live = new Set(cats().map((c) => c.id));
    for (const id of settledIds) {
      if (!live.has(id)) settledIds.delete(id);
    }
  }

  let applyFrame = 0;
  function scheduleApply() {
    if (applyFrame) return;
    const raf = window.requestAnimationFrame
      ? window.requestAnimationFrame.bind(window)
      : (fn) => window.setTimeout(fn, 16);
    applyFrame = raf(() => {
      applyFrame = 0;
      if (!state.enabled || state.edit) {
        revealPending();
        return;
      }
      try {
        applyCurrent();
      } finally {
        window.setTimeout(revealPending, 80);
      }
    });
  }

  function hasCatNode(node) {
    if (!(node instanceof Element)) return false;
    return node.matches('.room-cat-shell[data-room-cat-id]') ||
      !!node.querySelector('.room-cat-shell[data-room-cat-id]');
  }

  function watchNewMembers() {
    if (typeof MutationObserver !== 'function' || !document.body) return;
    ensurePendingStyle();
    const observer = new MutationObserver((records) => {
      if (!state.enabled || state.edit) return;
      pruneSettled();
      let needApply = false;
      for (const record of records) {
        if (record.type === 'attributes') {
          const el = record.target;
          if (!(el instanceof Element) || !el.matches('.room-cat-shell[data-room-cat-id]')) continue;
          const id = String(el.dataset.roomCatId || '');
          if (!id) continue;
          if (el.getAttribute('data-autotappo-pending') === '1') {
            needApply = true;
            continue;
          }
          if (!isCatVisible(el)) {
            // 好友离线/被官方隐藏：下次重新出现时重新走一次隐藏-排列流程
            settledIds.delete(id);
            continue;
          }
          if (!settledIds.has(id)) {
            hideCat(el);
            needApply = true;
          }
          continue;
        }
        for (const node of record.addedNodes) {
          if (hasCatNode(node)) {
            markNewCats(node);
            needApply = true;
            break;
          }
        }
      }
      const ids = cats().map((c) => c.id).join(',');
      if (ids && ids !== state.lastIds) needApply = true;
      if (needApply) scheduleApply();
    });
    observer.observe(document.body, {
      childList: true,
      subtree: true,
      attributes: true,
      attributeFilter: ['style', 'class', 'data-room-cat-id']
    });
  }

  window.__autotappoArrange = {
    version: VERSION,
    setEnabled,
    setEdit,
    setMode,
    setManual,
    setWrap,
    setWrapDir,
    refresh,
    get wrap() { return state.wrap; },
    get wrapDir() { return state.wrapDir; },
    get enabled() { return state.enabled; },
    get edit() { return state.edit; }
  };
  watchNewMembers();
  tick();
})();
