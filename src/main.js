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

// 配置可能被管理窗口改动，焦点回来时重拉一遍，避免主页展示过期状态。
// 同时静默补拉一次通知：启动那次 get_events 若撞上启动竞态失败，这里就是自愈通道；
// 已有通知的显示不该依赖首轮轮询是否跑完（ghr-events 只在有「新事件」时才广播）。
async function syncFromRust() {
  await refreshEvents({ silent: true });
  try {
    await refreshConfig();
  } catch {
    return;
  }
  goDefaultView();
}

// 分类记忆只在列表视图生效；切换即写而不等关闭事件——点关闭是隐藏到托盘，强杀进程也不该丢
const STATUS_KEYS = ["all", "unread", "read", "in_progress", "done"];

// 仅启动时定一次，之后不抢用户手上选的 tab；脏值不能让五个 tab 全不高亮
function pickDefaultStatus() {
  const remembered = loadSettings().lastStatus;
  if (STATUS_KEYS.includes(remembered)) {
    activeStatus = remembered;
    return;
  }
  if (allEvents.some((e) => e.status === "unread")) activeStatus = "unread";
  else if (allEvents.some((e) => e.status === "in_progress")) activeStatus = "in_progress";
  else activeStatus = "all";
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
const INITIAL_LIMIT = 100; // 默认只渲染 100 条，通知再多也不会一次性铺满滚动区
const LOAD_MORE_STEP = 100; // 滑到底自动再追加 100 条
const NEAR_BOTTOM_MARGIN = "32px 0px"; // 哨兵进入距底部 32px 就预加载下一批

let activeStatus = "all";
let feedLimit = INITIAL_LIMIT;
let feedObserver = null;

// 全量事件（含历史持久化），新事件追加，状态以后端为准
let allEvents = [];

// ---- 板视图 ----
const BOARD = window.GhrBoard;
let viewMode = "list"; // "list" | "board"，由管理窗口 emit 同步

// ---- 批量选择 ----
const BATCH = window.GhrBatch;
const selectedKeys = new Set();
let undoState = null; // { snapshot, selection }：最近一次批量编辑的现场，供撤销
let undoTimer = null;

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
  const key = entityKey(ev);
  const checked = selectedKeys.has(key);
  card.className = `event-card fade-up status-${ev.status || "unread"}${checked ? " selected" : ""}`;
  if (ev.pinned) card.classList.add("pinned");
  card.href = ev.url;
  card.dataset.key = key;
  card.innerHTML =
    `<span class="event-check${checked ? " checked" : ""}" role="checkbox" aria-checked="${checked}" title="选择这张卡"></span>` +
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
  // 作者可搜但卡片布局里不展示，hover 给个轻量提示，便于核对搜索命中原因
  if (ev.author) card.title = `作者: ${ev.author}`;
  bindCard(card, ev);
  card.querySelector(".event-check").addEventListener("click", (e) => {
    // 勾选框不能触发 <a> 的打开链接
    e.preventDefault();
    e.stopPropagation();
    toggleSelection(key);
  });
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
  counts.all = allEvents.length;
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

// 搜索纯逻辑在 search.js（classic script，挂 window.GhrSearch）。
// 查询存成字符串状态而非读 DOM：renderFeed / 触底续展 / tab 切换共用同一份，不会漂移。
const SEARCH = window.GhrSearch;
let searchQuery = "";

function searchActive() {
  return SEARCH.parseQuery(searchQuery).active;
}

// 后端已按 updatedAt 降序；分类 tab 与搜索条件叠加，两者都作用于全量 allEvents
// （不受 100 条分页限制），渲染和续展共用同一份列表。
function filteredEvents() {
  const base = activeStatus === "all"
    ? allEvents
    : allEvents.filter((ev) => (ev.status || "unread") === activeStatus);
  return SEARCH.searchEvents(base, searchQuery);
}

// 列表末尾的哨兵进入视口就自动追加下一批。每次 renderFeed 都会重建哨兵，
// 所以观察器也跟着重建；用交叉观察而不是 scroll 算阈值，重渲染时不会误触发，
// 也不会出现「内容不够高、没有滚动条就永远加载不了」的死角。
function observeFeedSentinel() {
  if (feedObserver) {
    feedObserver.disconnect();
    feedObserver = null;
  }
  const sentinel = document.getElementById("feed-more-hint");
  if (!sentinel) return;
  feedObserver = new IntersectionObserver(
    (entries) => {
      if (!entries.some((entry) => entry.isIntersecting)) return;
      const list = filteredEvents();
      if (feedLimit >= list.length) return;
      feedLimit = Math.min(feedLimit + LOAD_MORE_STEP, list.length);
      renderFeed();
    },
    { root: document.getElementById("event-feed"), rootMargin: NEAR_BOTTOM_MARGIN }
  );
  feedObserver.observe(sentinel);
}

// 两种视图共用一份 allEvents 与搜索条件，只在「怎么排」上分叉
function renderFeed() {
  // 列表会被整体重建，旧哨兵已失效，先断开观察器，避免回调打到空节点
  if (feedObserver) {
    feedObserver.disconnect();
    feedObserver = null;
  }
  renderPinned();
  renderTabs();
  if (viewMode === "board") renderBoard();
  else renderList();
  updateBatchBar();
}

function renderList() {
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
  const active = searchActive();
  if (allEvents.length === 0) {
    showHint(`暂无新事件。轮询每 ${config.pollInterval || 60} 秒一次，有新动态会自动浮现。`);
    return;
  }
  const list = filteredEvents();
  if (list.length === 0) {
    if (active) {
      showHint(`没有匹配「${searchQuery.trim()}」的通知`);
    } else {
      showHint(
        activeStatus === "all"
          ? "暂无新事件。轮询每 " + (config.pollInterval || 60) + " 秒一次，有新动态会自动浮现。"
          : "该分类下还没有通知"
      );
    }
    return;
  }
  const shown = list.slice(0, feedLimit);
  for (const ev of shown) feed.appendChild(buildCard(ev));
  const rest = list.length - shown.length;
  if (rest > 0) {
    // 纯提示哨兵：滚到这里会自动加载下一批，不用再手动点按钮
    const hint = document.createElement("p");
    hint.id = "feed-more-hint";
    hint.className = "feed-more-hint";
    hint.textContent = `${active ? `匹配 ${list.length} 条 · ` : ""}向下滚动继续加载（还有 ${rest} 条）`;
    feed.appendChild(hint);
  }
  feed.scrollTop = scrollTop;
  observeFeedSentinel();
}

// 白卡片是看板最标志性的层次；内容沿用列表卡片的字段，拖放只改状态
function buildBoardCard(ev) {
  const card = document.createElement("a");
  card.className = "board-card fade-up";
  card.href = ev.url;
  card.draggable = true;
  card.dataset.key = entityKey(ev);
  card.innerHTML =
    `<span class="board-meta"></span>` +
    `<span class="board-title"></span>` +
    `<span class="board-kind"></span>` +
    `<span class="board-sub"></span>`;
  card.querySelector(".board-meta").textContent =
    ev.repo + (ev.number != null ? ` #${ev.number}` : "");
  card.querySelector(".board-title").textContent = ev.title;
  card.querySelector(".board-kind").textContent = KIND_LABEL[ev.kind] || ev.kind;
  card.querySelector(".board-sub").textContent =
    (LAST_EVENT_LABEL[ev.lastEventKind] || ev.lastEventKind || "") +
    (ev.state ? ` · ${STATE_LABEL[ev.state] || ev.state}` : "");
  if (ev.author) card.title = `作者: ${ev.author}`;
  bindCard(card, ev);
  card.addEventListener("dragstart", (e) => {
    e.dataTransfer.setData("text/plain", card.dataset.key);
    e.dataTransfer.effectAllowed = "move";
    card.classList.add("dragging");
  });
  card.addEventListener("dragend", () => {
    card.classList.remove("dragging");
    clearDropTargets();
  });
  return card;
}

function clearDropTargets() {
  for (const col of document.querySelectorAll(".board-col.drop-target")) {
    col.classList.remove("drop-target");
  }
}

function onBoardDrop(targetStatus, e) {
  e.preventDefault();
  clearDropTargets();
  const key = e.dataTransfer.getData("text/plain");
  const ev = allEvents.find((item) => entityKey(item) === key);
  if (!ev) return;
  const status = BOARD.dropStatus(ev.status, targetStatus);
  if (!status) return;
  applyStatus(ev, status);
}

// 板视图一次渲染全量（后端上限 200 条），不用列表那套触底续展
function renderBoard() {
  const wrapEl = document.getElementById("board-wrap");
  const scrollLeft = wrapEl.scrollLeft;
  const wrap = document.getElementById("board-columns");
  wrap.innerHTML = "";
  const groups = BOARD.groupByStatus(SEARCH.searchEvents(allEvents, searchQuery));
  for (const [status, label] of BOARD.COLUMNS) {
    const col = document.createElement("div");
    col.className = "board-col";
    const head = document.createElement("div");
    head.className = "board-col-head";
    const name = document.createElement("span");
    name.textContent = label;
    const count = document.createElement("span");
    count.className = "board-col-count";
    count.textContent = `(${groups[status].length})`;
    head.append(name, count);
    const body = document.createElement("div");
    body.className = "board-col-body";
    if (groups[status].length === 0) {
      const empty = document.createElement("p");
      empty.className = "board-empty";
      empty.textContent = "暂无通知";
      body.appendChild(empty);
    } else {
      for (const ev of groups[status]) body.appendChild(buildBoardCard(ev));
    }
    col.append(head, body);
    col.addEventListener("dragover", (e) => {
      e.preventDefault();
      e.dataTransfer.dropEffect = "move";
      clearDropTargets();
      col.classList.add("drop-target");
    });
    col.addEventListener("drop", (e) => onBoardDrop(status, e));
    wrap.appendChild(col);
  }
  wrapEl.scrollLeft = scrollLeft;
}

// 视图模式只切显隐与渲染方式；分类由 pickDefaultStatus / 用户点击决定，这里不抢
function applyViewMode() {
  viewMode = loadSettings().viewMode === "board" ? "board" : "list";
  const board = viewMode === "board";
  document.getElementById("board-wrap").classList.toggle("hidden", !board);
  document.getElementById("event-feed").classList.toggle("hidden", board);
  document.getElementById("status-tabs").classList.toggle("hidden", board);
  document.getElementById("pinned-zone").classList.toggle("hidden", board);
  clearSelection();
  feedLimit = INITIAL_LIMIT;
  renderFeed();
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

// ---- 批量标记 + 撤销 ----
const UNDO_TIMEOUT_MS = 8000;

function updateBatchBar() {
  const bar = document.getElementById("batch-bar");
  const n = selectedKeys.size;
  bar.classList.toggle("hidden", n === 0 || viewMode === "board");
  document.getElementById("batch-count").textContent = `已选 ${n} 条`;
}

function paintSelection(key, on) {
  for (const el of document.querySelectorAll(".event-card")) {
    if (el.dataset.key !== key) continue;
    el.classList.toggle("selected", on);
    const check = el.querySelector(".event-check");
    if (!check) continue;
    check.classList.toggle("checked", on);
    check.setAttribute("aria-checked", String(on));
  }
}

function toggleSelection(key) {
  const on = !selectedKeys.has(key);
  if (on) selectedKeys.add(key);
  else selectedKeys.delete(key);
  paintSelection(key, on);
  updateBatchBar();
}

function clearSelection() {
  selectedKeys.clear();
  for (const el of document.querySelectorAll(".event-card.selected")) {
    el.classList.remove("selected");
    const check = el.querySelector(".event-check");
    if (!check) continue;
    check.classList.remove("checked");
    check.setAttribute("aria-checked", "false");
  }
  updateBatchBar();
}

// 轮询/删除后有些卡片已不在列表里，选中数不该虚高
function pruneSelection() {
  const valid = new Set(allEvents.map(entityKey));
  for (const key of [...selectedKeys]) {
    if (!valid.has(key)) selectedKeys.delete(key);
  }
}

function showUndoToast(count, text) {
  const el = document.getElementById("undo-toast");
  document.getElementById("undo-text").textContent = text || `已完成编辑 · ${count} 条`;
  el.classList.remove("show");
  void el.offsetWidth; // 连续两次编辑时强制重排，重新触发滑入动画
  el.classList.add("show");
  clearTimeout(undoTimer);
  undoTimer = setTimeout(() => {
    hideUndoToast();
    undoState = null;
  }, UNDO_TIMEOUT_MS);
}

function hideUndoToast() {
  clearTimeout(undoTimer);
  document.getElementById("undo-toast").classList.remove("show");
}

async function applyBatchStatus() {
  const status = document.getElementById("batch-status").value;
  const targets = allEvents.filter((ev) => selectedKeys.has(entityKey(ev)));
  if (targets.length === 0) {
    clearSelection();
    return;
  }
  const { payload, snapshot } = BATCH.buildBatch(targets, status);
  try {
    allEvents = await invoke("set_status_batch", payload);
  } catch (err) {
    flash(typeof err === "string" ? err : "批量标记失败");
    return;
  }
  // 双回滚现场：操作快照 + 当时的选中集合，撤销后能直接换个状态重新应用
  undoState = BATCH.captureUndo(snapshot, selectedKeys);
  clearSelection();
  renderFeed();
  showUndoToast(payload.repos.length);
}

async function undoBatch() {
  if (!undoState) return;
  const state = undoState;
  undoState = null; // 先摘掉现场防重复点击，失败时再放回去
  hideUndoToast();
  const payload = BATCH.buildUndo(state.snapshot);
  if (payload.repos.length === 0) return;
  try {
    allEvents = await invoke("set_status_batch", payload); // 操作回滚：逐条写回旧状态
  } catch (err) {
    // 没写成功就不能算撤销完，现场与 toast 一起还回去，让主人能重试
    undoState = state;
    showUndoToast(payload.repos.length, "撤销未成功 · 可重试");
    flash(typeof err === "string" ? err : "撤销失败，可重试");
    return;
  }
  // 选择状态回滚：精确回到编辑时那一批，不与 toast 期间新勾的卡片叠加
  selectedKeys.clear();
  const validKeys = new Set(allEvents.map(entityKey));
  for (const key of BATCH.restoreSelection(state.selection, validKeys)) {
    selectedKeys.add(key);
  }
  updateBatchBar();
  renderFeed();
  flash("已撤销，操作与选择均已恢复");
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

// 统一的通知拉取入口（后端才是实体聚合的唯一真相，前端不自己 upsert）。
// 两点很关键：
// 1) 失败时保留现有数据、不清空——一次瞬时失败不该把整个列表抹成空白；
// 2) 数据没变就不重绘，否则 focus 兜底每次都重建卡片、重放 fade-up 动画。
async function refreshEvents({ silent = false } = {}) {
  let next;
  try {
    next = await invoke("get_events");
  } catch (err) {
    console.error("读取通知失败:", err);
    if (!silent) flash(`读取通知失败：${err}`);
    return false;
  }
  if (JSON.stringify(next) === JSON.stringify(allEvents)) return true;
  allEvents = next;
  pruneSelection();
  renderFeed();
  return true;
}

async function loadEvents() {
  await refreshEvents();
}

async function addEvents() {
  await refreshEvents();
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
// 标记 + 最小化交给后端一次做完，守护不会把主人主动最小化当成 Win+D 藏窗弹回来
document.getElementById("btn-min").addEventListener("click", () => invoke("user_minimize"));
document.getElementById("btn-max").addEventListener("click", () => {
  getCurrentWindow().toggleMaximize().catch((err) => console.warn("切换最大化失败:", err));
});
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

// ---- 搜索框 ----
// 绑定放在顶层、不塞进 init：init 里有两次 await，等它跑完再绑会让启动瞬间的输入失效
const searchInput = document.getElementById("search-input");
const btnSearchClear = document.getElementById("btn-search-clear");

function clearSearch() {
  searchInput.value = "";
  searchQuery = "";
  btnSearchClear.classList.add("hidden");
  feedLimit = INITIAL_LIMIT;
  renderFeed();
  document.getElementById("event-feed").scrollTop = 0;
}

searchInput.addEventListener("input", () => {
  searchQuery = searchInput.value;
  btnSearchClear.classList.toggle("hidden", !searchQuery);
  feedLimit = INITIAL_LIMIT; // 换查询重新从 100 条起，别沿用上一批的分页位置
  renderFeed();
  document.getElementById("event-feed").scrollTop = 0;
});
btnSearchClear.addEventListener("click", () => {
  clearSearch();
  searchInput.focus();
});

// 批量操作与撤销同样放在顶层绑定，不塞进 init 的 await 之后
document.getElementById("btn-batch-apply").addEventListener("click", applyBatchStatus);
document.getElementById("btn-batch-clear").addEventListener("click", clearSelection);
document.getElementById("btn-undo").addEventListener("click", undoBatch);

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
  // 事件监听最先注册：首轮轮询 1s 后就可能 emit，等前面几个 await 跑完再注册会漏掉整批事件
  listen("ghr-events", () => addEvents());
  applyAppearance();
  applyWindowMode();

  try {
    await refreshConfig();
  } catch (e) {
    console.warn("读取配置失败:", e);
  }
  // 重启后从后端拉持久化通知，读态不丢
  await loadEvents();
  pickDefaultStatus();
  goDefaultView();
  applyViewMode();
  // 列表页初始是 hidden，显示出来后重新挂观察器，确保哨兵能被正确观察
  observeFeedSentinel();

  // 首次启动引导改由主窗口引导页承担，不再自动弹出管理窗口
  // 启动竞态兜底：配置在两窗口同时初始化期间可能未就绪
  setTimeout(syncFromRust, 1500);
  window.addEventListener("focus", syncFromRust);

  // 分类 tab 切换：重置分页并回到顶部
  for (const btn of document.querySelectorAll(".status-tab")) {
    btn.addEventListener("click", () => {
      if (activeStatus === btn.dataset.status) return;
      activeStatus = btn.dataset.status;
      if (viewMode === "list") saveSettings({ lastStatus: activeStatus });
      feedLimit = INITIAL_LIMIT;
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
    if (e.key !== "Escape") return;
    closeContextMenu();
    // 只有搜索框聚焦时 Esc 才清空，没聚焦时只收菜单、不动已有查询
    if (document.activeElement === searchInput && searchInput.value) clearSearch();
    else if (selectedKeys.size) clearSelection();
  });
  window.addEventListener("blur", closeContextMenu);
  document.getElementById("event-feed").addEventListener("scroll", closeContextMenu);

  // 轮询失败不再占界面空间：后台每轮会照常打日志，这里只留 console 便于排查
  listen("ghr-poll-error", (e) => {
    console.warn("轮询失败（下轮自动重试）:", e.payload);
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

  // 视图模式：值随事件带过来，主窗口先落盘再切，重启后 loadSettings 还能读到
  listen("ghr-view-mode", (e) => {
    const mode = e.payload === "board" ? "board" : "list";
    saveSettings({ viewMode: mode });
    applyViewMode();
  });
})();
