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
const KIND_LABEL = { issue: "Issue", pr: "PR", release: "Release", comment: "评论", state: "状态" };
const MAX_HANGING = 3; // 悬挂位：最多 3 条未读置顶

// 全量事件（含历史持久化），新事件追加，read 状态以后端为准
let allEvents = [];

function eventKey(ev) {
  return `${ev.kind}:${ev.repo}:${ev.number ?? ""}:${ev.created_at}`;
}

function buildCard(ev) {
  const card = document.createElement("a");
  card.className = "event-card fade-up" + (ev.read ? " read" : " unread");
  card.href = ev.url;
  card.innerHTML =
    `<span class="event-kind kind-${ev.kind}">${KIND_LABEL[ev.kind] || ev.kind}</span>` +
    `<span class="event-repo"></span>` +
    `<span class="event-title"></span>`;
  card.querySelector(".event-repo").textContent =
    ev.repo + (ev.number != null ? ` #${ev.number}` : "");
  card.querySelector(".event-title").textContent = ev.title;
  card.addEventListener("click", (e) => {
    e.preventDefault();
    // 已读粒度与 toast 去重一致：同一 issue/PR 一次读掉
    for (const other of allEvents) {
      if (other.repo === ev.repo && other.number === ev.number) other.read = true;
    }
    invoke("mark_read", { repo: ev.repo, number: ev.number ?? null }).catch(() => {});
    renderFeed();
    window.__TAURI__.opener.openUrl(ev.url);
  });
  return card;
}

function renderFeed() {
  const feed = document.getElementById("event-feed");
  feed.innerHTML = "";
  const empty = document.createElement("p");
  empty.id = "feed-empty";
  empty.className = "hint";
  empty.textContent = "暂无新事件。轮询每 60 秒一次，有新动态会自动浮现。";
  if (allEvents.length === 0) {
    feed.appendChild(empty);
    return;
  }
  // 悬挂区：最近 3 条未读置顶高亮；其余进历史区（新在前）
  const newestFirst = [...allEvents].reverse();
  const hanging = newestFirst.filter((ev) => !ev.read).slice(0, MAX_HANGING);
  const hangingKeys = new Set(hanging.map(eventKey));
  const history = newestFirst.filter((ev) => !hangingKeys.has(eventKey(ev))).slice(0, 50);
  const zone = document.createElement("div");
  zone.id = "hang-zone";
  for (const ev of hanging) zone.appendChild(buildCard(ev));
  feed.appendChild(zone);
  for (const ev of history) feed.appendChild(buildCard(ev));
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

function addEvents(events) {
  if (!Array.isArray(events)) return;
  allEvents.push(...events.map((ev) => ({ ...ev, read: false })));
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
document.getElementById("btn-add-watch").addEventListener("click", () => {
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

  // 无任何监听时自动打开管理窗口引导用户添加
  if (!config.watches || config.watches.length === 0) {
    invoke("open_manage").catch(() => {});
  }
  // 启动竞态兜底：配置在两窗口同时初始化期间可能未就绪
  setTimeout(syncFromRust, 1500);
  window.addEventListener("focus", syncFromRust);

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
