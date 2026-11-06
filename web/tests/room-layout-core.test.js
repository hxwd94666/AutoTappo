const test = require('node:test');
const assert = require('node:assert/strict');
const core = require('../injected/room-layout-core.js');

const VP = { width: 2048, height: 1152 };

function memoryStorage(seed = {}) {
  const map = new Map(Object.entries(seed));
  return {
    getItem(key) { return map.has(key) ? map.get(key) : null; },
    setItem(key, value) { map.set(key, String(value)); },
    removeItem(key) { map.delete(key); },
    dump() { return Object.fromEntries(map); }
  };
}

test('top-left geometry round-trips through viewport normalization', () => {
  const rect = { x: 1000, y: 300, w: 100, h: 106 };
  const n = core.normalizeRect(rect, VP);
  const back = core.restoreRect(n, VP);
  assert.deepEqual(back, rect);
  const centered = core.restoreCenter(n, VP);
  assert.ok(Math.abs(centered.x + centered.w / 2 - (rect.x + rect.w / 2)) < 1e-9);
  assert.ok(Math.abs(centered.y + centered.h / 2 - (rect.y + rect.h / 2)) < 1e-9);
});

test('visible rect is restored separately from shell rect', () => {
  const entry = {
    x: 0.5, y: 0.5, w: 0.05, h: 0.1,
    visible: { x: 0.5, y: 0.55, w: 0.05, h: 0.06 }
  };
  assert.deepEqual(core.restoreRect(entry, VP), { x: 1024, y: 576, w: 102.4, h: 115.2 });
  assert.deepEqual(core.restoreVisible(entry, VP), { x: 1024, y: 633.6, w: 102.4, h: 69.12 });
});

test('overlap threshold is based on the candidate visible area', () => {
  const a = { x: 0, y: 0, w: 100, h: 100 };
  const b = { x: 0, y: 0, w: 100, h: 30 };
  assert.equal(core.overlapRatio(a, b), 0.3);
  assert.equal(core.blocked(a, [b], 0.30), false);
  assert.equal(core.blocked(a, [b], 0.29), true);
  assert.equal(core.blocked(a, [{ x: 200, y: 0, w: 100, h: 100 }]), false);
});

test('legacy center rows migrate to v3 top-left rows', () => {
  const room = core.migrateLegacy({ 42: { x: 0.5, y: 0.5, w: 0.1, h: 0.2 } }, 1234);
  const e = room.members['42'];
  assert.equal(e.x, 0.45);
  assert.equal(e.y, 0.4);
  assert.equal(e.visible.x, 0.45);
  assert.equal(e.lastSeen, 1234);
});

test('loadWithMigration only migrates when the v3 key is absent', () => {
  const v1Key = core.LEGACY_KEY_PREFIX + '1:R';
  const v3Key = core.KEY_PREFIX + '1:R';
  const storage = memoryStorage({ [v1Key]: JSON.stringify({ 7: { x: 0.5, y: 0.5, w: 0.1, h: 0.2 } }) });
  const first = core.loadWithMigration(storage, v3Key, v1Key, 99);
  assert.equal(first.migrated, true);
  assert.equal(first.room.members['7'].x, 0.45);
  core.saveRoom(storage, v3Key, first.room);
  const second = core.loadWithMigration(storage, v3Key, v1Key, 100);
  assert.equal(second.migrated, false);
  assert.equal(second.room.members['7'].x, 0.45);
});

test('saveRoom prunes invalid and oldest entries to the cap', () => {
  const storage = memoryStorage();
  const room = core.emptyRoom();
  for (let i = 0; i < core.MAX_MEMBERS + 5; i++) {
    room.members[String(i)] = { x: 0.1, y: 0.1, w: 0.1, h: 0.1, lastSeen: i + 1 };
  }
  room.members.bad = { x: 'nope', y: 0, w: 1, h: 1 };
  core.saveRoom(storage, core.KEY_PREFIX + '1:R', room);
  const raw = JSON.parse(storage.getItem(core.KEY_PREFIX + '1:R'));
  const ids = Object.keys(raw.members);
  assert.equal(ids.length, core.MAX_MEMBERS);
  assert.equal(raw.members.bad, undefined);
  assert.equal(raw.members['0'], undefined);
  assert.ok(raw.members[String(core.MAX_MEMBERS + 4)]);
});

test('putMember stores shell and visible geometry and clamps viewport state', () => {
  const room = core.emptyRoom();
  const shell = { x: 100, y: 200, w: 100, h: 100 };
  const visible = { x: 110, y: 230, w: 80, h: 60 };
  assert.equal(core.putMember(room, '9', shell, visible, VP, 55), true);
  const e = room.members['9'];
  assert.equal(e.x, 100 / 2048);
  assert.equal(e.visible.y, 230 / 1152);
  assert.equal(e.lastSeen, 55);
  assert.equal(core.insideViewport(core.restoreVisible(e, VP), VP), true);
  const clamped = core.clampRect({ x: -500, y: 2000, w: 100, h: 100 }, VP);
  assert.ok(clamped.x >= core.MARGIN);
  assert.ok(clamped.y + clamped.h <= VP.height - core.MARGIN);
});

test('sanitizeEntry rejects non-finite or absurd geometry', () => {
  assert.equal(core.sanitizeEntry({ x: NaN, y: 0, w: 1, h: 1 }), null);
  assert.equal(core.sanitizeEntry({ x: 0, y: 0, w: 0, h: 1 }), null);
  assert.equal(core.sanitizeEntry({ x: 0, y: 0, w: 10, h: 10 }), null);
  const ok = core.sanitizeEntry({ x: 0, y: 0, w: 0.1, h: 0.1, visible: { x: 0, y: 0, w: 0.1, h: 0.05 } });
  assert.equal(ok.visible.h, 0.05);
});

test('findSlot places one horizontal friend below own visible body', () => {
  const own = { x: 1800, y: 200, w: 100, h: 100 };
  const rect = core.findSlot({ own, obstacles: [own], size: { w: 100, h: 100 }, viewport: VP, groupCount: 1, mode: 'horizontal' });
  assert.ok(rect);
  assert.equal(rect.y, own.y + own.h + 14);
  assert.ok(rect.x + rect.w <= VP.width - core.MARGIN);
  assert.equal(core.blocked(rect, [own]), false);
});

test('findSlot places one vertical friend above own visible body', () => {
  const own = { x: 1800, y: 900, w: 100, h: 100 };
  const rect = core.findSlot({ own, obstacles: [own], size: { w: 100, h: 100 }, viewport: VP, groupCount: 1, mode: 'vertical' });
  assert.ok(rect);
  assert.equal(rect.x, own.x);
  assert.equal(rect.y, own.y - 14 - 100);
  assert.equal(core.blocked(rect, [own]), false);
});

test('findSlot avoids an occupied first candidate and stays in viewport', () => {
  const own = { x: 1800, y: 200, w: 100, h: 100 };
  const blockedRect = { x: 1790, y: 314, w: 100, h: 100 };
  const rect = core.findSlot({ own, obstacles: [own, blockedRect], size: { w: 100, h: 100 }, viewport: VP, groupCount: 1, mode: 'horizontal' });
  assert.ok(rect);
  assert.equal(core.blocked(rect, [own, blockedRect]), false);
  assert.ok(rect.x >= core.MARGIN && rect.x + rect.w <= VP.width - core.MARGIN);
});

test('findSlot centers a multi-friend horizontal row around own', () => {
  const own = { x: 1000, y: 200, w: 100, h: 100 };
  const first = core.findSlot({ own, obstacles: [own], size: { w: 100, h: 100 }, viewport: VP, groupCount: 2, mode: 'horizontal' });
  assert.ok(first);
  const second = core.findSlot({ own, obstacles: [own, first], size: { w: 100, h: 100 }, viewport: VP, groupCount: 2, mode: 'horizontal' });
  assert.ok(second);
  assert.equal(core.blocked(second, [own, first]), false);
  assert.equal(second.y, first.y);
  assert.ok(second.x !== first.x);
});
test('assignSlots fills 19 base slots in order', () => {
  const ids = Array.from({ length: 19 }, (_, i) => 'c' + i);
  const out = core.assignSlots({}, ids, 19);
  assert.equal(out.slotCount, 19);
  ids.forEach((id, i) => assert.equal(out.assignments[id], i));
});

test('assignSlots appends a 20th slot when full', () => {
  const ids = Array.from({ length: 20 }, (_, i) => 'c' + i);
  const out = core.assignSlots({}, ids, 19);
  assert.equal(out.slotCount, 20);
  assert.equal(out.assignments.c19, 19);
});

test('assignSlots moves highest slot into a middle gap', () => {
  const before = core.assignSlots({}, ['a', 'b', 'c', 'd', 'e'], 19);
  const out = core.assignSlots(before.assignments, ['a', 'b', 'd', 'e'], 19);
  assert.equal(out.assignments.a, 0);
  assert.equal(out.assignments.b, 1);
  assert.equal(out.assignments.e, 2);
  assert.equal(out.assignments.d, 3);
  assert.equal(out.assignments.c, undefined);
});

test('assignSlots fills a later empty slot with a new friend', () => {
  const before = core.assignSlots({}, ['a', 'b', 'c', 'd'], 19);
  const out = core.assignSlots(before.assignments, ['a', 'b', 'c', 'd', 'new'], 19);
  assert.equal(out.assignments.new, 4);
  assert.equal(out.slotCount, 19);
});

test('assignSlots keeps a late free slot after highest friend leaves', () => {
  const before = core.assignSlots({}, ['a', 'b', 'c'], 19);
  const out = core.assignSlots(before.assignments, ['a', 'b'], 19);
  assert.equal(out.assignments.a, 0);
  assert.equal(out.assignments.b, 1);
  assert.equal(out.slotCount, 19);
});
test('buildDirectionSlots starts horizontal right on own row and wraps by count', () => {
  const own = { x: 100, y: 100, w: 100, h: 100 };
  const slots = core.buildDirectionSlots({ mode: 'h-right', own, size: { w: 100, h: 100 }, viewport: VP, wrap: 6, count: 19 });
  assert.equal(slots[0].x, own.x + own.w + core.GAP);
  assert.equal(slots[0].y, own.y);
  assert.equal(slots[4].y, own.y);
  assert.equal(slots[5].y, own.y + own.h + core.GAP);
  assert.equal(slots[5].x, own.x);
});

test('buildDirectionSlots starts vertical up on own column and wraps by count', () => {
  const own = { x: 100, y: 500, w: 100, h: 100 };
  const slots = core.buildDirectionSlots({ mode: 'v-up', own, size: { w: 100, h: 100 }, viewport: VP, wrap: 5, count: 19 });
  assert.equal(slots[0].x, own.x);
  assert.equal(slots[0].y, own.y - core.GAP - own.h);
  assert.equal(slots[3].y, own.y - core.GAP - own.h - 3 * (own.h + core.GAP));
  assert.equal(slots[4].x, own.x + own.w + core.GAP);
  assert.equal(slots[4].y, own.y);
});

test('buildDirectionSlots auto wrap fills to the edge', () => {
  const own = { x: 100, y: 100, w: 100, h: 100 };
  const slots = core.buildDirectionSlots({ mode: 'h-right', own, size: { w: 100, h: 100 }, viewport: VP, wrap: 0, count: 19 });
  assert.ok(slots.length >= 17);
  assert.equal(slots[0].y, own.y);
  assert.ok(slots.every((s) => s.x + s.w <= VP.width - core.MARGIN));
});

test('buildDirectionSlots does not jump to the opposite side at the edge', () => {
  const own = { x: 1940, y: 100, w: 80, h: 100 };
  const slots = core.buildDirectionSlots({ mode: 'h-right', own, size: { w: 100, h: 100 }, viewport: VP, wrap: 0, count: 19 });
  assert.ok(slots.length > 0);
  assert.ok(slots[0].x + slots[0].w <= own.x);
});
