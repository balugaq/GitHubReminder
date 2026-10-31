// 主窗口：引导页 + 事件流 + 监听概览。添加/编辑/设置全部在 manage 窗口（emit 跨窗口联动）

const { getCurrentWindow } = window.__TAURI__.window;
const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const STORAGE_KEY = "ghr.settings";

function loadSettings() {
  try {
    return JSON.parse(localStorage.getItem(STORAGE_KEY)) || {};
  } catch {
    return {};
  }
}

function saveSettings(patch) {
  localStorage.setItem(STORAGE_KEY, JSON.stringify({ ...loadSettings(), ...patch }));
}

function hexToRgb(hex) {
  const n = parseInt(hex.slice(1), 16);
  return `${(n >> 16) & 255}, ${(n >> 8) & 255}, ${(n) & 255}`;
}

function applyAppearance() {
  const { color = "#4a90d9", opacity = 70 } = loadSettings();
  const root = document.documentElement;
  root.style.setProperty("--frame-color", hexToRgb(color));
  root.style.setProperty("--frame-alpha", String(opacity / 100));
}

// ---- 视图切换 ----
// fade-up 只在视图首次显示（display:none 翻开）和新事件卡插入时播一次，
// 不在这里重放：聚焦/配置同步都会走 showView，重放会让无变更也闪动画
function showView(id) {
  for (const v of document.querySelectorAll(".view")) {
    v.classList.add("hidden");
  }
  document.getElementById(id).classList.remove("hidden");
}

// ---- 业务配置（Rust 侧） ----
let config = { token: null, watches: [] };

async function refreshConfig() {
  config = await invoke("get_config");
}

// 配置可能被管理窗口改动，焦点回来时重拉一遍，避免主页展示过期状态
async function syncFromRust() {
  try {
    await refreshConfig();
  } catch {
    return;
  }
  goDefaultView();
}

function goDefaultView() {
  if (config.watches && config.watches.length > 0) {
    updateStatusHint();
    showView("view-list");
  } else {
    showView("view-guide");
  }
}

// ---- 左下角状态 hint ----
let lastPollSecs = null;

function updateStatusHint() {
  const watches = config.watches || [];
  const repos = new Set(watches.filter((w) => w.number == null).map((w) => w.repo.toLowerCase())).size;
  const singles = watches.filter((w) => w.number != null).length;
  const time = lastPollSecs != null
    ? new Date(lastPollSecs * 1000).toLocaleTimeString("zh-CN", { hour12: false })
    : "--:--:--";
  document.getElementById("hint-text").textContent =
    `正在监听 ${repos} 个仓库 + ${singles} 个单独条目 · 上次刷新在 ${time}`;
}

// ---- 事件流 ----
const KIND_LABEL = { issue: "Issue", pr: "PR", release: "Release", comment: "评论", state: "状态", closed_issue: "关Issue", closed_pr: "关PR" };
// 实体最近一次变化的中文摘要 + 状态中文（Todo6：以 issue/pr/release 为单位的聚合卡片）
const LAST_EVENT_LABEL = { open: "新建/开启", comment: "新评论", state: "状态变更", closed: "已关闭", release: "新版本" };
const STATE_LABEL = { open: "进行中", closed: "已关闭", published: "已发布" };
const PAGE_SIZE = 50; // 每个分类默认最多 50 条，展开再加 50

let activeStatus = "unread";
let feedLimit = PAGE_SIZE;

// 全量事件（含历史持久化），新事件追加，状态以后端为准
let allEvents = [];

function entityKey(ev) {
  return `${(ev.repo || "").toLowerCase()}:${ev.number ?? ""}`;
}

// 卡片交互：点开链接（未读自动转已读）+ 右键菜单
function bindCard(card, ev) {
  card.addEventListener("click", (e) => {
    e.preventDefault();
    // 未读点开即已读（同 GitHub 通知收件箱）；处理中/已完成不自动改
    if ((ev.status || "unread") === "unread") applyStatus(ev, "read");
    window.__TAURI__.opener.openUrl(ev.url);
  });
  card.addEventListener("contextmenu", (e) => {
    e.preventDefault();
    e.stopPropagation();
    openContextMenu(ev, e.clientX, e.clientY);
  });
}

function buildCard(ev) {
  const card = document.createElement("a");
  card.className = `event-card fade-up status-${ev.status || "unread"}`;
  if (ev.pinned) card.classList.add("pinned");
  card.href = ev.url;
  card.dataset.key = entityKey(ev);
  card.innerHTML =
    `<span class="event-kind kind-${ev.kind}">${KIND_LABEL[ev.kind] || ev.kind}</span>` +
    `<span class="event-repo"></span>` +
    `<span class="event-title"></span>` +
    `<span class="event-sub"></span>`;
  card.querySelector(".event-repo").textContent =
    ev.repo + (ev.number != null ? ` #${ev.number}` : "");
  card.querySelector(".event-title").textContent = ev.title;
  // 后端实体字段是驼峰序列化（lastEventKind / updatedAt / state / status）
  card.querySelector(".event-sub").textContent =
    (LAST_EVENT_LABEL[ev.lastEventKind] || ev.lastEventKind || "") +
    (ev.state ? ` · ${STATE_LABEL[ev.state] || ev.state}` : "");
  bindCard(card, ev);
  return card;
}

// 置顶区的紧凑卡片：单行，横向排布
function buildPinnedCard(ev) {
  const card = document.createElement("a");
  card.className = "pinned-card";
  card.href = ev.url;
  card.dataset.key = entityKey(ev);
  card.title = `${ev.repo}${ev.number != null ? ` #${ev.number}` : ""} ${ev.title}`;
  card.innerHTML =
    `<span class="pin-mark">📌</span>` +
    `<span class="event-kind kind-${ev.kind}">${KIND_LABEL[ev.kind] || ev.kind}</span>` +
    `<span class="pinned-repo"></span>` +
    `<span class="pinned-title"></span>`;
  card.querySelector(".pinned-repo").textContent =
    ev.repo + (ev.number != null ? ` #${ev.number}` : "");
  card.querySelector(".pinned-title").textContent = ev.title;
  bindCard(card, ev);
  return card;
}

function renderPinned() {
  const zone = document.getElementById("pinned-zone");
  zone.innerHTML = "";
  // 后端已限制最多 3 个，这里再兜一次底
  for (const ev of allEvents.filter((e) => e.pinned).slice(0, 3)) {
    zone.appendChild(buildPinnedCard(ev));
  }
}

// ---- 状态分类 tab ----
function renderTabs() {
  const counts = { unread: 0, read: 0, in_progress: 0, done: 0 };
  for (const ev of allEvents) {
    if (counts[ev.status] != null) counts[ev.status] += 1;
  }
  for (const btn of document.querySelectorAll(".status-tab")) {
    const key = btn.dataset.status;
    btn.classList.toggle("active", key === activeStatus);
    const badge = btn.querySelector(".tab-dot, .tab-count");
    if (!badge) continue;
    badge.textContent = String(counts[key] || 0);
    // 未读 0 条不亮红点；其余分类的计数始终显示
    if (badge.classList.contains("tab-dot")) {
      badge.classList.toggle("hidden", (counts[key] || 0) === 0);
    }
  }
}

function renderFeed() {
  renderPinned();
  renderTabs();
  const feed = document.getElementById("event-feed");
  const scrollTop = feed.scrollTop;
  feed.innerHTML = "";
  const showHint = (text) => {
    const p = document.createElement("p");
    p.id = "feed-empty";
    p.className = "hint";
    p.textContent = text;
    feed.appendChild(p);
  };
  if (allEvents.length === 0) {
    showHint(`暂无新事件。轮询每 ${config.pollInterval || 60} 秒一次，有新动态会自动浮现。`);
    return;
  }
  // 后端已按 updatedAt 降序，这里只按分类过滤
  const list = allEvents.filter((ev) => (ev.status || "unread") === activeStatus);
  if (list.length === 0) {
    showHint("该分类下还没有通知");
    return;
  }
  const shown = list.slice(0, feedLimit);
  for (const ev of shown) feed.appendChild(buildCard(ev));
  const rest = list.length - shown.length;
  if (rest > 0) {
    // 按钮排在列表末尾，滚到底即可见，点一次多展开 50 条
    const btn = document.createElement("button");
    btn.id = "btn-expand";
    btn.className = "btn-expand";
    btn.textContent = `展开更多（还有 ${rest} 条）`;
    btn.addEventListener("click", () => {
      feedLimit += PAGE_SIZE;
      renderFeed();
    });
    feed.appendChild(btn);
  }
  feed.scrollTop = scrollTop;
}

// ---- 状态变更 / 置顶 / 删除 / 复制 ----
let flashTimer = null;

// 轻提示：置顶已满、复制结果等，2.2s 自动消失
function flash(text) {
  const el = document.getElementById("float-msg");
  el.textContent = text;
  el.classList.remove("hidden");
  clearTimeout(flashTimer);
  flashTimer = setTimeout(() => el.classList.add("hidden"), 2200);
}

async function applyStatus(ev, status) {
  try {
    allEvents = await invoke("set_status", {
      repo: ev.repo,
      number: ev.number ?? null,
      status,
    });
    renderFeed();
  } catch (err) {
    console.warn("更新状态失败:", err);
  }
}

// 置顶/取消置顶；超过 3 个时后端拒绝，这里把原因提示出来
async function setPin(ev, pinned) {
  try {
    allEvents = await invoke("set_pin", {
      repo: ev.repo,
      number: ev.number ?? null,
      pinned,
    });
    renderFeed();
  } catch (err) {
    flash(typeof err === "string" ? err : "置顶失败");
  }
}

async function deleteEntity(ev) {
  try {
    allEvents = await invoke("delete_event", {
      repo: ev.repo,
      number: ev.number ?? null,
    });
    renderFeed();
  } catch (err) {
    console.warn("删除通知失败:", err);
  }
}

async function copyText(text) {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    // 裁剪板 API 不可用时退回 execCommand
    const ta = document.createElement("textarea");
    ta.value = text;
    ta.style.position = "fixed";
    ta.style.opacity = "0";
    document.body.appendChild(ta);
    ta.select();
    let ok = false;
    try {
      ok = document.execCommand("copy");
    } catch {
      ok = false;
    }
    ta.remove();
    return ok;
  }
}

// ---- 右键菜单 ----
function closeContextMenu() {
  const menu = document.getElementById("ctx-menu");
  menu.classList.add("hidden");
  menu.innerHTML = "";
}

function addCtxItem(menu, label, onClick, opts = {}) {
  const b = document.createElement("button");
  b.className = "ctx-item" + (opts.checked ? " checked" : "") + (opts.danger ? " danger" : "");
  b.textContent = label;
  b.addEventListener("click", () => {
    closeContextMenu();
    onClick();
  });
  menu.appendChild(b);
}

function openContextMenu(ev, x, y) {
  const menu = document.getElementById("ctx-menu");
  menu.innerHTML = "";
  const current = ev.status || "unread";
  const statusItems = [
    ["unread", "未读"],
    ["read", "已读"],
    ["in_progress", "处理中"],
    ["done", "已完成"],
  ];
  for (const [status, label] of statusItems) {
    addCtxItem(menu, label, () => applyStatus(ev, status), { checked: current === status });
  }
  // 置顶（钉住）——第 4 张会被后端拒绝
  addCtxItem(menu, ev.pinned ? "取消置顶" : "置顶", () => setPin(ev, !ev.pinned), {
    checked: !!ev.pinned,
  });
  const sep = document.createElement("div");
  sep.className = "ctx-sep";
  menu.appendChild(sep);
  addCtxItem(menu, "复制链接", async () => {
    const ok = await copyText(ev.url);
    flash(ok ? "已复制链接" : "复制失败");
  });
  addCtxItem(menu, "删除", () => deleteEntity(ev), { danger: true });

  menu.classList.remove("hidden");
  // 先量尺寸再定位，避免贴边溢出
  const rect = menu.getBoundingClientRect();
  const left = Math.min(x, window.innerWidth - rect.width - 6);
  const top = Math.min(y, window.innerHeight - rect.height - 6);
  menu.style.left = `${Math.max(4, left)}px`;
  menu.style.top = `${Math.max(4, top)}px`;
}

async function loadEvents() {
  try {
    allEvents = await invoke("get_events");
  } catch (err) {
    console.warn("读取历史通知失败:", err);
    allEvents = [];
  }
  renderFeed();
}

async function addEvents() {
  // 收到变化事件后直接从后端重拉已聚合的实体列表，避免前端自己维护 upsert
  try {
    allEvents = await invoke("get_events");
  } catch (err) {
    console.warn("拉取通知失败:", err);
    return;
  }
  renderFeed();
}

// ---- 统计视图 ----
function renderStats(stats) {
  const list = document.getElementById("stats-list");
  list.innerHTML = "";
  const entries = Object.entries(stats);
  if (entries.length === 0) {
    list.innerHTML = '<p class="hint">还没有统计数据</p>';
    return;
  }
  // 仓库维度在前，单条维度在后
  entries.sort(([a], [b]) => (a.includes("#") ? 1 : 0) - (b.includes("#") ? 1 : 0));
  for (const [key, kinds] of entries) {
    const row = document.createElement("div");
    row.className = "stats-row";
    const name = document.createElement("span");
    name.className = "stats-key";
    name.textContent = key;
    const detail = document.createElement("span");
    detail.className = "stats-detail";
    detail.textContent = Object.entries(kinds)
      .map(([kind, count]) => `${KIND_LABEL[kind] || kind} ×${count}`)
      .join(" · ");
    row.appendChild(name);
    row.appendChild(detail);
    list.appendChild(row);
  }
}

async function openStats() {
  showView("view-stats");
  try {
    renderStats(await invoke("get_stats"));
  } catch (err) {
    console.warn("读取统计失败:", err);
  }
}

// ---- 窗口控制 ----
document.getElementById("btn-min").addEventListener("click", () => getCurrentWindow().minimize());
document.getElementById("btn-close").addEventListener("click", () => getCurrentWindow().close());

// 设置/管理/添加入口：开（或唤起）独立管理窗口
document.getElementById("btn-settings").addEventListener("click", () => {
  invoke("open_manage").catch((err) => console.warn("打开管理窗口失败:", err));
});
document.getElementById("btn-guide-start").addEventListener("click", () => {
  invoke("open_manage").catch((err) => console.warn("打开管理窗口失败:", err));
});
document.getElementById("btn-stats").addEventListener("click", () => openStats());
document.getElementById("btn-stats-back").addEventListener("click", () => goDefaultView());

// 🔄 立即刷新
document.getElementById("btn-refresh").addEventListener("click", async () => {
  const btn = document.getElementById("btn-refresh");
  if (btn.disabled) return;
  btn.disabled = true;
  btn.classList.add("spinning");
  try {
    lastPollSecs = await invoke("poll_now");
    updateStatusHint();
  } catch (err) {
    console.warn("立即刷新失败:", err);
  } finally {
    btn.disabled = false;
    btn.classList.remove("spinning");
  }
});

function applyWindowMode() {
  // 默认贴桌面（不置顶）；嵌入失败时降级为普通窗口
  const { topmost = false } = loadSettings();
  invoke("set_window_mode", { topmost }).catch((err) => {
    console.warn("窗口模式设置失败，降级为普通窗口:", err);
  });
}

// ---- 初始化 ----
(async function init() {
  applyAppearance();
  applyWindowMode();

  try {
    await refreshConfig();
  } catch (e) {
    console.warn("读取配置失败:", e);
  }
  // 重启后从后端拉持久化通知，读态不丢
  await loadEvents();
  goDefaultView();

  // 首次启动引导改由主窗口引导页承担，不再自动弹出管理窗口
  // 启动竞态兜底：配置在两窗口同时初始化期间可能未就绪
  setTimeout(syncFromRust, 1500);
  window.addEventListener("focus", syncFromRust);

  // 分类 tab 切换：重置分页并回到顶部
  for (const btn of document.querySelectorAll(".status-tab")) {
    btn.addEventListener("click", () => {
      if (activeStatus === btn.dataset.status) return;
      activeStatus = btn.dataset.status;
      feedLimit = PAGE_SIZE;
      renderFeed();
      document.getElementById("event-feed").scrollTop = 0;
    });
  }

  // 右键菜单：点空白 / 滚轮 / 失焦 / Esc 都收起
  document.addEventListener("click", closeContextMenu);
  document.addEventListener("contextmenu", (e) => {
    if (!e.target.closest(".event-card")) closeContextMenu();
  });
  document.addEventListener("keydown", (e) => {
    if (e.key === "Escape") closeContextMenu();
  });
  window.addEventListener("blur", closeContextMenu);
  document.getElementById("event-feed").addEventListener("scroll", closeContextMenu);

  listen("ghr-events", (e) => addEvents(e.payload));

  listen("ghr-poll-error", (e) => {
    const el = document.getElementById("poll-error");
    el.textContent = `⚠ 轮询失败：${e.payload}（检查网络/代理后等待下轮自动重试）`;
    el.classList.remove("hidden");
  });

  // 管理窗口改了配置（增删监听等），主窗口同步刷新
  listen("ghr-config-changed", () => syncFromRust());

  // 后台轮循每轮完成都会广播时间戳（含手动刷新）
  listen("ghr-poll-done", (e) => {
    lastPollSecs = e.payload;
    updateStatusHint();
  });

  // 管理窗口改了外观，实时同步主窗口
  listen("ghr-appearance", () => applyAppearance());
})();
