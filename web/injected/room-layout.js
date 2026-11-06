// 自动排列好友桌宠：包装官方 index.render，只改 vnode 的 left/top。
// 由 AutoTappo 的 arrange worker 通过 CDP 注入；几何/存储核心由 room-layout-core.js 提供。
(() => {
  'use strict';
  if (window.__autotappoArrange) {
    try { window.__autotappoArrange.setEnabled(true); } catch (e) {}
    return;
  }

  const CORE = window.__autotappoLayoutCore;
  if (!CORE) return;

  const GAP = CORE.GAP;
  const state = {
    enabled: false,
    edit: false,
    idx: null,
    orig: null,
    overrides: new Map(),
    key: '',
    room: CORE.emptyRoom(),
    layoutMode: window.__autotappoLayoutMode === 'vertical' ? 'vertical' : 'horizontal',
    official: false,
    lastIds: '',
    editBase: null,
    timer: 0
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
  function keyOf(id) {
    return CORE.KEY_PREFIX + id.account + ':' + id.room;
  }
  function legacyKeyOf(id) {
    return CORE.LEGACY_KEY_PREFIX + id.account + ':' + id.room;
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
  function loadRoom() {
    if (!state.key) return CORE.emptyRoom();
    const id = identity();
    const result = CORE.loadWithMigration(localStorage, state.key, legacyKeyOf(id), Date.now());
    state.room = result.room;
    if (result.migrated) CORE.saveRoom(localStorage, state.key, state.room);
    return state.room;
  }
  function storeRoom() {
    if (state.key) CORE.saveRoom(localStorage, state.key, state.room);
  }
  function putMember(room, c, shell, visible) {
    CORE.putMember(room, c.id, shell, visible, viewport(), Date.now());
  }
  function saveCats() {
    const room = loadRoom();
    for (const c of cats()) putMember(room, c, c.shell, c.visible);
    storeRoom();
  }
  function targetVisible(shell, c) {
    return {
      x: shell.x + (c.visible.x - c.shell.x),
      y: shell.y + (c.visible.y - c.shell.y),
      w: c.visible.w,
      h: c.visible.h
    };
  }
  function savedTargetShell(entry, c) {
    const center = CORE.restoreCenter(entry, viewport());
    return {
      x: center.x + center.w / 2 - c.shell.w / 2,
      y: center.y + center.h / 2 - c.shell.h / 2,
      w: c.shell.w,
      h: c.shell.h
    };
  }
  function clampIfOffscreen(shell, c) {
    const visible = targetVisible(shell, c);
    if (CORE.insideViewport(visible, viewport())) return shell;
    return CORE.clampRect(shell, viewport());
  }
  function offlineObstacles(room, currentIds) {
    const out = [];
    const vp = viewport();
    for (const [id, entry] of Object.entries(room.members)) {
      if (currentIds.has(String(id))) continue;
      const visible = CORE.restoreVisible(entry, vp);
      if (CORE.insideViewport(visible, vp)) out.push(visible);
    }
    return out;
  }
  function findSlot(c, own, obstacles, groupCount) {
    const offsetX = c.visible.x - c.shell.x;
    const offsetY = c.visible.y - c.shell.y;
    const visible = CORE.findSlot({
      own: own ? own.visible : null,
      obstacles,
      size: { w: c.visible.w, h: c.visible.h },
      viewport: viewport(),
      groupCount,
      mode: state.layoutMode
    });
    if (!visible) return CORE.clampRect({ x: c.shell.x, y: c.shell.y, w: c.shell.w, h: c.shell.h }, viewport());
    return {
      x: visible.x - offsetX,
      y: visible.y - offsetY,
      w: c.shell.w,
      h: c.shell.h
    };
  }
  function patchVNode(v) {
    if (!v) return;
    const id = v.props && v.props['data-room-cat-id'] ? String(v.props['data-room-cat-id']) : '';
    if (id && state.overrides.has(id)) {
      const p = state.overrides.get(id);
      v.props.style = Object.assign({}, typeof v.props.style === 'object' && v.props.style ? v.props.style : {}, {
        left: p.left + 'px',
        top: p.top + 'px'
      });
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
  function apply() {
    if (!state.enabled || state.edit) return;
    const idx = findIndex();
    if (!idx) return;
    const id = identity();
    const nextKey = keyOf(id);
    if (nextKey !== state.key) {
      state.key = nextKey;
      state.overrides.clear();
      state.lastIds = '';
    }
    const own = ownRects();
    if (!own) return;
    const list = cats();
    if (!list.length) return;
    state.lastIds = list.map((c) => c.id).join(',');
    for (const c of list) settledIds.add(c.id);
    const room = loadRoom();
    const currentIds = new Set(list.map((c) => c.id));
    const obstacles = [own.visible, ...offlineObstacles(room, currentIds)];
    const placed = [];
    const pending = [];
    for (const c of list) {
      const entry = room.members[c.id];
      if (!entry) {
        pending.push(c);
        continue;
      }
      const shell = clampIfOffscreen(savedTargetShell(entry, c), c);
      const visible = targetVisible(shell, c);
      if (CORE.blocked(visible, obstacles)) {
        pending.push(c);
      } else {
        placed.push({ c, shell, visible });
        obstacles.push(visible);
      }
    }
    for (const c of pending) {
      const shell = findSlot(c, own, obstacles, pending.length);
      const visible = targetVisible(shell, c);
      placed.push({ c, shell, visible });
      obstacles.push(visible);
    }
    for (const item of placed) putMember(room, item.c, item.shell, item.visible);
    if (!placed.length) return;
    storeRoom();
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
    const k = keyOf(id);
    const ids = cats().map((c) => c.id).join(',');
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
      if (state.official) restoreOfficial(); else restore();
      refresh();
      return;
    }
    if (state.enabled) return;
    state.enabled = true;
    state.edit = false;
    state.lastIds = '';
    apply();
  }
  function baseOf(c) {
    return {
      shell: { x: c.shell.x, y: c.shell.y, w: c.shell.w, h: c.shell.h },
      visible: { x: c.visible.x, y: c.visible.y, w: c.visible.w, h: c.visible.h }
    };
  }
  function changed(before, c) {
    if (!before) return true;
    const fields = [
      [before.shell, c.shell],
      [before.visible, c.visible]
    ];
    for (const [a, b] of fields) {
      if (Math.abs(a.x - b.x) > 0.75 || Math.abs(a.y - b.y) > 0.75 ||
          Math.abs(a.w - b.w) > 0.75 || Math.abs(a.h - b.h) > 0.75) return true;
    }
    return false;
  }
  function setEdit(on) {
    if (!state.enabled) return;
    if (state.edit === !!on) return;
    state.edit = !!on;
    if (state.edit) {
      saveCats();
      state.editBase = null;
      restore();
      refresh();
      setTimeout(() => {
        if (!state.edit) return;
        state.editBase = {};
        for (const c of cats()) state.editBase[c.id] = baseOf(c);
      }, 80);
    } else {
      const room = loadRoom();
      const base = state.editBase || {};
      for (const c of cats()) {
        const before = base[c.id];
        if (!changed(before, c)) continue;
        putMember(room, c, c.shell, c.visible);
      }
      storeRoom();
      state.editBase = null;
      state.overrides.clear();
      state.lastIds = '';
      apply();
    }
  }
  function setMode(mode) {
    const next = mode === 'vertical' ? 'vertical' : 'horizontal';
    if (state.layoutMode === next) return next;
    state.layoutMode = next;
    if (state.enabled && !state.edit) {
      const room = loadRoom();
      for (const c of cats()) delete room.members[c.id];
      storeRoom();
      state.lastIds = '';
      apply();
    }
    return next;
  }
  function reset() {
    if (state.edit) return false;
    const id = identity();
    state.key = keyOf(id);
    state.room = CORE.emptyRoom();
    CORE.saveRoom(localStorage, state.key, state.room);
    state.lastIds = '';
    state.overrides.clear();
    apply();
    return true;
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
        apply();
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
    setEnabled,
    setEdit,
    setMode,
    reset,
    refresh,
    get enabled() { return state.enabled; },
    get edit() { return state.edit; }
  };
  watchNewMembers();
  tick();
})();
