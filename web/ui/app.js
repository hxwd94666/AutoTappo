// AutoTappo 前端逻辑（浅色简洁版）
(function () {
  "use strict";

  const isTauri = typeof window.__TAURI__ !== "undefined" && typeof window.__TAURI__.core !== "undefined";
  const invoke = isTauri
    ? window.__TAURI__.core.invoke
    : async (cmd, args) => {
        console.log("[preview]", cmd, args);
        return null;
      };

  const $ = (id) => document.getElementById(id);
  const statusDot = $("status-dot");
  const statusText = $("status-text");
  const heroMode = $("hero-mode");
  const counter = $("counter");
  const counterLabel = $("counter-label");
  const btnToggle = $("btn-toggle");
  const modeTabs = document.querySelectorAll(".seg");
  const silentPanel = $("silent-panel");
  const speedPanel = $("speed-panel");
  const silentTotal = $("silent-total");
  const silentStatus = $("silent-status");
  const keyboardHint = $("keyboard-hint");
  const speedButtons = document.querySelectorAll(".speed-btn");
  const hotkeyRow = $("hotkey-row");
  const hotkeyValue = $("hotkey-value");
  const addCountInput = $("add-count");
  const addButton = $("add-btn");
  const addSub = $("add-sub");
  const chestSwitch = $("chest-switch");
  const advSwitch = $("adv-switch");
  const advSub = $("adv-sub");
  const arrangeSwitch = $("arrange-switch");
  const arrangeSub = $("arrange-sub");
  const layoutGroup = $("layout-group");
  const layoutToggle = $("layout-toggle");
  const layoutGroupBody = $("layout-group-body");
  const layoutHLeftBtn = $("layout-h-left");
  const layoutHRightBtn = $("layout-h-right");
  const layoutVUpBtn = $("layout-v-up");
  const layoutVDownBtn = $("layout-v-down");
  const layoutOverlapBtn = $("layout-overlap");
  const layoutModeSub = $("layout-mode-sub");
  const layoutWrapRow = $("layout-wrap-row");
  const layoutWrapInput = $("layout-wrap");
  const layoutWrapSub = $("layout-wrap-sub");
  const layoutWrapDirRow = $("layout-wrap-dir-row");
  const layoutWrapDirSelect = $("layout-wrap-dir");
  const layoutWrapDirSub = $("layout-wrap-dir-sub");
  const layoutEditHotkeyBtn = $("layout-edit-hotkey");
  const layoutEditSub = $("layout-edit-sub");
  const autostartSwitch = $("autostart-switch");
  const btnMin = $("btn-min");
  const btnClose = $("btn-close");
  const confirmModal = $("confirm-modal");
  const confirmOk = $("confirm-ok");
  const confirmCancel = $("confirm-cancel");

  let currentMode = 1;
  let currentRunning = false;
  let currentSpeed = 0;
  let currentHotkeyName = localStorage.getItem("autotappo_hotkey_name") || "F5";
  let currentHotkeyVK = parseInt(localStorage.getItem("autotappo_hotkey_vk"), 10) || 0x74;
  let autoChest = localStorage.getItem("autotappo.autoChest") === "1";
  let advancedEnabled = false;
  let autoArrange = false;
  let layoutMode = "h-right";
  let layoutWrap = 0;
  let layoutWrapDir = 0;
  let layoutExpanded = false;
  let layoutEditing = false;
  let layoutEditHotkeyVK = parseInt(localStorage.getItem("autotappo_layout_edit_hotkey_vk"), 10) || 0x75;
  let layoutEditHotkeyName = localStorage.getItem("autotappo_layout_edit_hotkey_name") || "F6";
  let layoutRecording = null;
  let autostartEnabled = false;
  let recording = false;
  let previewInterval = null;
  let previewCount = 0;

  if (currentHotkeyVK === 0x73) {
    currentHotkeyVK = 0x74;
    currentHotkeyName = "F5";
    localStorage.setItem("autotappo_hotkey_vk", "116");
    localStorage.setItem("autotappo_hotkey_name", "F5");
  }
  if (layoutEditHotkeyVK === 0x74) {
    layoutEditHotkeyVK = 0x75;
    layoutEditHotkeyName = "F6";
    localStorage.setItem("autotappo_layout_edit_hotkey_vk", "117");
    localStorage.setItem("autotappo_layout_edit_hotkey_name", "F6");
  }
  hotkeyValue.textContent = currentHotkeyName;

  // ---------- 基础渲染 ----------
  function fmt(n) {
    return Number(n || 0).toLocaleString("en-US");
  }

  function renderMode() {
    modeTabs.forEach((tab) => {
      tab.classList.toggle("active", parseInt(tab.dataset.mode, 10) === currentMode);
    });
    const silent = currentMode === 1;
    silentPanel.hidden = !silent;
    speedPanel.hidden = silent;
    silentStatus.hidden = !silent;
    keyboardHint.hidden = silent;
    counterLabel.textContent = silent ? "本次静默注入" : "本次模拟点击";
    heroMode.textContent = silent ? "静默模式" : "键盘模式";
    renderToggle();
  }

  function renderToggle() {
    const active = currentRunning;
    const modeWord = currentMode === 1 ? "静默" : "挂机";
    btnToggle.textContent = active
      ? "暂停" + modeWord + " (" + currentHotkeyName + ")"
      : "开始" + modeWord + " (" + currentHotkeyName + ")";
    btnToggle.classList.toggle("running", active);
  }

  function renderChestSwitch() {
    chestSwitch.setAttribute("aria-checked", autoChest ? "true" : "false");
    renderAdvSwitch();
  }

  function setAdvSub(text, isError) {
    advSub.textContent = text;
    advSub.title = text;
    advSub.classList.toggle("error", !!isError);
  }

  function renderAdvSwitch() {
    if (!autoChest) advancedEnabled = false;
    advSwitch.disabled = !autoChest;
    advSwitch.setAttribute("aria-disabled", autoChest ? "false" : "true");
    advSwitch.setAttribute("aria-checked", advancedEnabled ? "true" : "false");
    if (!autoChest) setAdvSub("需先开启自动开箱");
    else if (!advancedEnabled) setAdvSub("锁屏 / 隐藏桌宠时也能开箱");
    renderLayoutHotkeys();
  }


  async function submitAddClicks() {
    if (addButton.disabled) return;
    if (!isTauri) {
      addSub.textContent = "一键加数仅在桌面版可用";
      return;
    }
    const count = Number(addCountInput.value);
    if (!Number.isInteger(count) || count < 1 || count > 1000000) {
      addSub.textContent = "请输入 1～1,000,000 的整数";
      addCountInput.focus();
      return;
    }
    addButton.disabled = true;
    try {
      const accepted = Number(await invoke("add_clicks", { count }));
      addSub.textContent = "已提交 " + accepted.toLocaleString("zh-CN") + " 次，等待官方同步";
    } catch (e) {
      addSub.textContent = "提交失败：" + e;
    } finally {
      addButton.disabled = false;
    }
  }

  addButton.addEventListener("click", submitAddClicks);
  addCountInput.addEventListener("keydown", (event) => {
    if (event.key === "Enter") {
      event.preventDefault();
      submitAddClicks();
    }
  });
  addCountInput.addEventListener("input", () => {
    if (!addButton.disabled && addSub.textContent !== "写入本地计数，等待下次同步") {
      addSub.textContent = "写入本地计数，等待下次同步";
    }
  });

  function renderLayoutGroup() {
    layoutToggle.setAttribute("aria-expanded", layoutExpanded ? "true" : "false");
    layoutGroup.classList.toggle("expanded", layoutExpanded);
    layoutGroupBody.hidden = !layoutExpanded;
  }

  function layoutModeLabel(mode) {
    if (mode === 'h-left') return '横左';
    if (mode === 'h-right') return '横右';
    if (mode === 'v-up') return '竖上';
    if (mode === 'v-down') return '竖下';
    if (mode === 'overlap') return '重叠';
    return '横右';
  }

  function layoutModeHint(mode) {
    if (mode === 'h-left') return '好友从自己左侧同排开始';
    if (mode === 'v-up') return '好友从自己上方同列开始';
    if (mode === 'v-down') return '好友从自己下方同列开始';
    return '好友从自己右侧同排开始';
  }

  function renderLayoutHotkeys() {
    if (!advancedEnabled) {
      autoArrange = false;
      layoutEditing = false;
      layoutRecording = null;
    }
    arrangeSwitch.disabled = !advancedEnabled;
    arrangeSwitch.setAttribute('aria-disabled', advancedEnabled ? 'false' : 'true');
    arrangeSwitch.setAttribute('aria-checked', autoArrange ? 'true' : 'false');
    const overlapMode = layoutMode === 'overlap';
    if (overlapMode) layoutEditing = false;
    const layoutDisabled = !advancedEnabled || !autoArrange || layoutEditing;
    layoutHLeftBtn.disabled = layoutDisabled;
    layoutHRightBtn.disabled = layoutDisabled;
    layoutVUpBtn.disabled = layoutDisabled;
    layoutVDownBtn.disabled = layoutDisabled;
    layoutOverlapBtn.disabled = layoutDisabled;
    layoutHLeftBtn.classList.toggle('active', layoutMode === 'h-left');
    layoutHRightBtn.classList.toggle('active', layoutMode === 'h-right');
    layoutVUpBtn.classList.toggle('active', layoutMode === 'v-up');
    layoutVDownBtn.classList.toggle('active', layoutMode === 'v-down');
    layoutOverlapBtn.classList.toggle('active', overlapMode);
    layoutModeSub.textContent = !advancedEnabled || !autoArrange
      ? '开启位置排列后可用'
      : (overlapMode ? '所有好友叠在自己身后，拖动时跟随' : layoutModeHint(layoutMode));
    layoutWrapRow.hidden = overlapMode;
    layoutWrapInput.disabled = !advancedEnabled || !autoArrange || overlapMode;
    if (document.activeElement !== layoutWrapInput) {
      layoutWrapInput.value = layoutWrap > 0 ? String(layoutWrap) : '';
    }
    layoutWrapSub.textContent = overlapMode
      ? '重叠模式不使用换行'
      : (layoutMode === 'h-left' || layoutMode === 'h-right'
        ? '每行数量，包含自己；0 或空为到顶'
        : '每列数量，包含自己；0 或空为到顶');
    const horizontalMode = layoutMode === 'h-left' || layoutMode === 'h-right';
    layoutWrapDirRow.hidden = overlapMode;
    layoutWrapDirSelect.disabled = !advancedEnabled || !autoArrange || overlapMode;
    layoutWrapDirSelect.options[0].textContent = horizontalMode ? '向下' : '向右';
    layoutWrapDirSelect.options[1].textContent = horizontalMode ? '向上' : '向左';
    layoutWrapDirSelect.value = String(layoutWrapDir);
    layoutWrapDirSub.textContent = overlapMode
      ? '重叠模式不使用换行方向'
      : (horizontalMode
        ? (layoutWrapDir === 1 ? '换行到上一行' : '换行到下一行')
        : (layoutWrapDir === 1 ? '换行到左侧一列' : '换行到右侧一列'));
    layoutEditHotkeyBtn.disabled = !advancedEnabled || !autoArrange || overlapMode;
    layoutEditHotkeyBtn.classList.toggle('recording', layoutRecording === 'edit');
    layoutEditHotkeyBtn.textContent = layoutRecording === 'edit' ? '…' : layoutEditHotkeyName;
    arrangeSub.textContent = !advancedEnabled
      ? '需先开启高级开箱'
      : (autoArrange ? '已开启 · ' + layoutModeLabel(layoutMode) : '已关闭');
    layoutEditSub.textContent = !advancedEnabled || !autoArrange
      ? '开启位置排列后可用'
      : (overlapMode ? '重叠模式不使用编辑' : (layoutEditing ? '手动布局 · 编辑中，按第二次完成编辑' : '手动布局 · 按一次进入编辑，按第二次完成编辑'));
  }


  async function syncLayoutHotkeys() {
    if (!isTauri) {
      layoutEditing = false;
      renderLayoutHotkeys();
      return;
    }
    try {
      if (advancedEnabled) {
        autoArrange = !!(await invoke("get_auto_arrange"));
        const modeValue = String(await invoke("get_layout_mode"));
        layoutMode = ["h-left", "h-right", "v-up", "v-down", "overlap"].includes(modeValue) ? modeValue : "h-right";
        layoutWrap = Math.max(0, Number(await invoke("get_layout_wrap")) || 0);
        layoutWrapDir = Number(await invoke("get_layout_wrap_dir")) === 1 ? 1 : 0;
        layoutEditing = !!(await invoke("get_layout_edit"));
        layoutEditHotkeyVK = Number(await invoke("get_layout_edit_hotkey")) || 0x75;
        layoutEditHotkeyName = vkLabel(layoutEditHotkeyVK);
      } else {
        layoutEditing = false;
      }
      renderLayoutHotkeys();
    } catch (e) {}
  }

  function renderAutostartSwitch() {
    autostartSwitch.setAttribute("aria-checked", autostartEnabled ? "true" : "false");
  }

  function vkLabel(vk) {
    if (VK_NAMES[vk]) return VK_NAMES[vk];
    if (vk >= 0x30 && vk <= 0x5a) return String.fromCharCode(vk);
    return "VK" + vk;
  }

  async function syncChestState() {
    if (!isTauri) return;
    try {
      const s = await invoke("get_chest_status");
      autoChest = !!s.enabled;
      renderChestSwitch();
    } catch (e) {}
  }

  // ---------- 状态轮询 ----------
  function updateUI(status) {
    if (!status) return;
    currentRunning = !!status.running;
    currentSpeed = Number(status.speed_mode) || 0;
    if (typeof status.mode === "number" && status.mode !== currentMode) {
      currentMode = status.mode === 0 ? 0 : 1;
      renderMode();
    }
    if (typeof status.hotkey_vk === "number" && status.hotkey_vk > 0) {
      currentHotkeyVK = status.hotkey_vk;
      currentHotkeyName = vkLabel(status.hotkey_vk);
      if (!recording) hotkeyValue.textContent = currentHotkeyName;
    }
    counter.textContent = fmt(status.total_sent);
    statusText.textContent = currentRunning ? "运行中" : "已暂停";
    statusDot.classList.toggle("running", currentRunning);
    renderToggle();

    speedButtons.forEach((b) => {
      b.classList.toggle("active", parseInt(b.dataset.speed, 10) === currentSpeed);
    });

    if (currentMode === 1) {
      const s = status.silent || {};
      silentTotal.textContent = fmt(s.last_amount || 0) + " / " + fmt(s.total || 0);
      silentStatus.textContent = s.detail || "—";
      silentStatus.style.color =
        s.state === "error" ? "var(--danger)" : s.state === "injected" ? "var(--success)" : "var(--muted)";
    }
  }

  async function pollStatus() {
    if (!isTauri) return;
    try {
      updateUI(await invoke("get_status"));
    } catch (e) {
      /* 静默失败 */
    }
  }

  function applyAdvanced(s) {
    if (!s) return;
    advancedEnabled = autoChest && !!s.enabled;
    renderAdvSwitch();
    if (!autoChest || !s.enabled) return;
    const state = String(s.state || "");
    const detail = String(s.detail || "");
    let text = detail || "高级开箱运行中";
    if (state === "ready") text = "运行中 · " + (detail || "CDP 已就绪");
    else if (state === "waiting") text = "运行中 · " + (detail || "等待锁屏 / 隐藏条件");
    else if (state === "opened") text = "运行中 · " + (detail || "已开启宝箱");
    else if (state === "starting") text = detail || "正在检查桌宠…";
    else if (state === "error") text = detail || "高级开箱运行失败";
    setAdvSub(text, state === "error");
  }

  async function pollAdvanced() {
    if (!isTauri) return;
    try {
      applyAdvanced(await invoke("get_advanced_status"));
      await syncLayoutHotkeys();
    } catch (e) {}
  }

  async function refreshAutostart() {
    if (!isTauri) {
      autostartEnabled = false;
      renderAutostartSwitch();
      return;
    }
    try {
      autostartEnabled = !!(await invoke("get_autostart"));
      renderAutostartSwitch();
    } catch (e) {}
  }

  // ---------- 主按钮 / 模式 ----------
  btnToggle.addEventListener("click", async () => {
    if (!isTauri) {
      currentRunning = !currentRunning;
      updatePreview();
      return;
    }
    try {
      currentRunning = !!(await invoke("toggle_running"));
      await pollStatus();
    } catch (e) {
      console.error(e);
    }
  });

  modeTabs.forEach((tab) => {
    tab.addEventListener("click", async () => {
      const mode = parseInt(tab.dataset.mode, 10) === 0 ? 0 : 1;
      if (mode === currentMode) return;
      currentMode = mode;
      localStorage.setItem("autotappo_mode", String(mode));
      renderMode();
      if (isTauri) {
        try {
          await invoke("set_mode", { mode });
          await pollStatus();
        } catch (e) {}
      } else {
        updatePreview();
      }
    });
  });

  speedButtons.forEach((b) => {
    b.addEventListener("click", async () => {
      currentSpeed = parseInt(b.dataset.speed, 10);
      if (isTauri) {
        try {
          await invoke("set_speed", { mode: currentSpeed });
        } catch (e) {}
      }
      speedButtons.forEach((x) => x.classList.toggle("active", x === b));
    });
  });

  // ---------- 热键录制 ----------
  const VK_NAMES = {
    0x70: "F1", 0x71: "F2", 0x72: "F3", 0x73: "F4", 0x74: "F5",
    0x75: "F6", 0x76: "F7", 0x77: "F8", 0x78: "F9", 0x79: "F10",
    0x7a: "F11", 0x7b: "F12",
    0x24: "Home", 0x23: "End", 0x21: "PageUp", 0x22: "PageDown",
    0x2d: "Insert", 0x2e: "Delete", 0x09: "Tab", 0x14: "CapsLock",
    0x90: "NumLock", 0x91: "ScrollLock", 0x13: "Pause", 0x2c: "PrintScreen",
  };

  function keyToVK(e) {
    if (e.keyCode) return e.keyCode;
    const k = e.key;
    if (/^F([1-9]|1[0-2])$/.test(k)) return 0x6f + parseInt(k.slice(1), 10);
    if (k.length === 1) return k.toUpperCase().charCodeAt(0);
    return 0;
  }

  function keyName(e, vk) {
    if (VK_NAMES[vk]) return VK_NAMES[vk];
    if (e.key && e.key.length === 1) return e.key.toUpperCase();
    return "VK" + vk;
  }

  function startRecord() {
    if (recording) return;
    recording = true;
    hotkeyValue.classList.add("recording");
    hotkeyValue.textContent = "请按键…";
    window.addEventListener("keydown", onRecordKey, true);
  }

  function stopRecord() {
    recording = false;
    hotkeyValue.classList.remove("recording");
    hotkeyValue.textContent = currentHotkeyName;
    window.removeEventListener("keydown", onRecordKey, true);
  }

  function onRecordKey(e) {
    e.preventDefault();
    e.stopPropagation();
    if (e.key === "Escape") {
      stopRecord();
      return;
    }
    const vk = keyToVK(e);
    if (!vk) return;
    if (vk === 0x76 || vk === 0x77 || vk === 0x78) {
      hotkeyValue.textContent = "F7/F8/F9 不可用";
      return;
    }
    if (vk === layoutEditHotkeyVK) {
      hotkeyValue.textContent = "布局快捷键占用";
      return;
    }
    currentHotkeyVK = vk;
    currentHotkeyName = keyName(e, vk);
    localStorage.setItem("autotappo_hotkey_vk", String(vk));
    localStorage.setItem("autotappo_hotkey_name", currentHotkeyName);
    stopRecord();
    renderToggle();
    if (isTauri) invoke("set_hotkey", { vk }).catch(() => {});
  }

  hotkeyRow.addEventListener("click", startRecord);

  // ---------- 布局快捷键 ----------
  function stopLayoutRecord() {
    layoutRecording = null;
    window.removeEventListener("keydown", onLayoutRecordKey, true);
    renderLayoutHotkeys();
  }

  function onLayoutRecordKey(e) {
    e.preventDefault();
    e.stopPropagation();
    if (e.key === "Escape") {
      stopLayoutRecord();
      return;
    }
    const target = layoutRecording;
    if (!target) return;
    const vk = keyToVK(e);
    if (!vk) return;
    const targetBtn = layoutEditHotkeyBtn;
    if (vk === 0x76 || vk === 0x77 || vk === 0x78) {
      targetBtn.textContent = "禁用";
      targetBtn.title = "F7/F8/F9 为官方保留键";
      setTimeout(renderLayoutHotkeys, 900);
      return;
    }
    if (vk === currentHotkeyVK) {
      targetBtn.textContent = "占用";
      targetBtn.title = "该键已被其它功能占用";
      setTimeout(renderLayoutHotkeys, 900);
      return;
    }
    const name = keyName(e, vk);
    layoutEditHotkeyVK = vk;
    layoutEditHotkeyName = name;
    localStorage.setItem("autotappo_layout_edit_hotkey_vk", String(vk));
    localStorage.setItem("autotappo_layout_edit_hotkey_name", name);
    if (isTauri) invoke("set_layout_edit_hotkey", { vk }).catch(() => {});
    stopLayoutRecord();
  }

  function startLayoutRecord(target) {
    if (!advancedEnabled || layoutRecording) return;
    if (recording) stopRecord();
    layoutRecording = target;
    renderLayoutHotkeys();
    window.addEventListener("keydown", onLayoutRecordKey, true);
  }

  layoutEditHotkeyBtn.addEventListener("click", () => startLayoutRecord("edit"));

  // ---------- 自动开箱 ----------
  chestSwitch.addEventListener("click", async () => {
    autoChest = !autoChest;
    localStorage.setItem("autotappo.autoChest", autoChest ? "1" : "0");
    if (!autoChest) advancedEnabled = false;
    renderChestSwitch();
    if (isTauri) {
      try {
        await invoke("set_auto_chest", { enabled: autoChest });
        if (!autoChest) await invoke("set_advanced_enabled", { enabled: false });
        await pollAdvanced();
      } catch (e) {}
    }
  });

  // ---------- 高级开箱（默认关闭） ----------
  function askAdvancedConfirm() {
    return new Promise((resolve) => {
      confirmModal.hidden = false;
      const done = (value) => {
        confirmModal.hidden = true;
        confirmOk.removeEventListener("click", onOk);
        confirmCancel.removeEventListener("click", onCancel);
        confirmModal.removeEventListener("click", onBackdrop);
        resolve(value);
      };
      const onOk = () => done(true);
      const onCancel = () => done(false);
      const onBackdrop = (e) => {
        if (e.target === confirmModal) done(false);
      };
      confirmOk.addEventListener("click", onOk);
      confirmCancel.addEventListener("click", onCancel);
      confirmModal.addEventListener("click", onBackdrop);
    });
  }

  advSwitch.addEventListener("click", async () => {
    if (!autoChest) return;
    if (!advancedEnabled) {
      const ok = await askAdvancedConfirm();
      if (!ok) return;
    }
    advancedEnabled = !advancedEnabled;
    renderAdvSwitch();
    if (isTauri) {
      try {
        if (advancedEnabled) {
          await invoke("set_advanced_flags", { lockScreen: true, hidden: true });
          await invoke("set_advanced_enabled", { enabled: true });
        } else {
          await invoke("set_advanced_enabled", { enabled: false });
          await invoke("set_advanced_flags", { lockScreen: false, hidden: false });
        }
        await pollAdvanced();
      } catch (e) {
        alert("切换高级开箱失败：" + e);
      }
    }
  });

  // ---------- 位置排列 ----------
  function toggleLayoutGroup() {
    layoutExpanded = !layoutExpanded;
    renderLayoutGroup();
  }

  layoutToggle.addEventListener("click", (event) => {
    if (event.target.closest("#arrange-switch")) return;
    toggleLayoutGroup();
  });
  layoutToggle.addEventListener("keydown", (event) => {
    if (event.target.closest("#arrange-switch")) return;
    if (event.key !== "Enter" && event.key !== " ") return;
    event.preventDefault();
    toggleLayoutGroup();
  });

  arrangeSwitch.addEventListener("click", async (event) => {
    event.stopPropagation();
    if (!isTauri || !advancedEnabled) return;
    const next = !autoArrange;
    try {
      autoArrange = !!(await invoke("set_auto_arrange", { enabled: next }));
    } catch (e) {
      autoArrange = false;
    }
    renderLayoutHotkeys();
  });

  async function applyLayoutMode(mode) {
    if (!isTauri || !advancedEnabled || !autoArrange || layoutEditing) return;
    try {
      const actual = String(await invoke("set_layout_mode", { mode }));
      layoutMode = ["h-left", "h-right", "v-up", "v-down", "overlap"].includes(actual) ? actual : "h-right";
    } catch (e) {}
    renderLayoutHotkeys();
  }

  layoutHLeftBtn.addEventListener("click", () => applyLayoutMode("h-left"));
  layoutHRightBtn.addEventListener("click", () => applyLayoutMode("h-right"));
  layoutVUpBtn.addEventListener("click", () => applyLayoutMode("v-up"));
  layoutVDownBtn.addEventListener("click", () => applyLayoutMode("v-down"));
  layoutOverlapBtn.addEventListener("click", () => applyLayoutMode("overlap"));
  layoutWrapInput.addEventListener("change", async () => {
    if (!isTauri || !advancedEnabled || !autoArrange || layoutEditing) return;
    let wrap = Math.floor(Number(layoutWrapInput.value) || 0);
    wrap = Math.max(0, Math.min(20, wrap));
    if (wrap === 1) wrap = 2;
    try {
      layoutWrap = Math.max(0, Number(await invoke("set_layout_wrap", { wrap })) || 0);
    } catch (e) {}
    renderLayoutHotkeys();
  });
  layoutWrapDirSelect.addEventListener("change", async () => {
    if (!isTauri || !advancedEnabled || !autoArrange || layoutEditing) return;
    const dir = Number(layoutWrapDirSelect.value) === 1 ? 1 : 0;
    try {
      layoutWrapDir = Number(await invoke("set_layout_wrap_dir", { dir })) === 1 ? 1 : 0;
    } catch (e) {}
    renderLayoutHotkeys();
  });


  // ---------- 开机自启 ----------
  autostartSwitch.addEventListener("click", async () => {
    if (!isTauri) {
      autostartEnabled = !autostartEnabled;
      renderAutostartSwitch();
      return;
    }
    const next = !autostartEnabled;
    autostartSwitch.disabled = true;
    try {
      const actual = !!(await invoke("set_autostart", { enabled: next }));
      autostartEnabled = actual;
      renderAutostartSwitch();
      if (actual !== next) alert("任务计划状态未按预期更新，请检查系统权限。");
    } catch (e) {
      alert("设置开机自启失败：" + e);
    } finally {
      autostartSwitch.disabled = false;
    }
  });

  // 自绘标题栏的最小化 / 关闭
  btnMin.addEventListener("click", async () => {
    if (isTauri) await invoke("minimize_window").catch(() => {});
  });
  btnClose.addEventListener("click", async () => {
    if (isTauri) await invoke("close_window").catch(() => {});
  });

  // 无边框窗口的拖动：按钮区域不触发
  const titlebar = document.querySelector(".titlebar");
  if (titlebar) {
    titlebar.addEventListener("mousedown", (e) => {
      if (e.button !== 0 || !isTauri) return;
      if (e.target.closest(".titlebar-actions")) return;
      invoke("start_drag").catch(() => {});
    });
  }

  // ---------- 预览模式 ----------
  function updatePreview() {
    updateUI({
      running: currentRunning,
      speed_mode: currentSpeed,
      total_sent: previewCount,
      mode: currentMode,
      silent: { state: currentRunning ? "injected" : "idle", detail: currentRunning ? "预览：等待同步" : "静默模式在锁屏/隐藏桌宠时仍生效，不影响桌宠动画", last_amount: 0, total: previewCount },
    });
  }

  // ---------- 启动 ----------
  const storedMode = localStorage.getItem("autotappo_mode");
  currentMode = storedMode === null ? 1 : (parseInt(storedMode, 10) === 1 ? 1 : 0);
  renderMode();
  renderChestSwitch();
  renderAdvSwitch();
  renderLayoutGroup();
  renderLayoutHotkeys();
  renderAutostartSwitch();

  if (isTauri) {
    // 启动时从后端读取配置；不要用 localStorage 反向覆盖后端配置。
    syncChestState();
    pollStatus();
    pollAdvanced();
    syncLayoutHotkeys();
    refreshAutostart();
    setInterval(pollStatus, 500);
    setInterval(pollAdvanced, 1000);
  } else {
    updatePreview();
    previewInterval = setInterval(() => {
      if (currentRunning) {
        previewCount += 1;
        updatePreview();
      }
    }, 1000);
  }
})();
