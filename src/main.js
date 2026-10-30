// M2：监听管理 + 事件流（Rust 轮询 GitHub API，emit ghr-events）
// 外观（颜色/透明度/置顶）仍存 localStorage，业务配置（token/watches）在 Rust 侧 config.json

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
  return `${(n >> 16) & 255}, ${(n >> 8) & 255}, ${n & 255}`;
}

function applyAppearance() {
  const { color = "#4a90d9", opacity = 70 } = loadSettings();
  const root = document.documentElement;
  root.style.setProperty("--frame-color", hexToRgb(color));
  root.style.setProperty("--frame-alpha", String(opacity / 100));
}

// ---- 视图切换（fade-up 每次重放） ----
function showView(id) {
  for (const v of document.querySelectorAll(".view")) {
    v.classList.add("hidden");
  }
  const target = document.getElementById(id);
  target.classList.remove("hidden");
  for (const el of target.querySelectorAll(".fade-up")) {
    el.style.animation = "none";
  }
  void target.offsetWidth;
  for (const el of target.querySelectorAll(".fade-up")) {
    el.style.animation = "";
  }
}

// ---- 业务配置（Rust 侧） ----
let config = { token: null, watches: [] };

async function refreshConfig() {
  config = await invoke("get_config");
  updateTokenState();
}

function goDefaultView() {
  if (config.watches && config.watches.length > 0) {
    renderWatchList();
    showView("view-list");
  } else {
    showView("view-guide");
  }
}

function updateTokenState() {
  const el = document.getElementById("token-state");
  const input = document.getElementById("input-token");
  if (config.token) {
    el.textContent = "已设置";
    el.style.color = "#7CFC9B";
    input.value = "";
    input.placeholder = "已保存，输入新值可覆盖";
  } else {
    el.textContent = "未设置";
    el.style.color = "";
    input.value = "";
    input.placeholder = "ghp_... （仅存本机）";
  }
  document.getElementById("input-proxy").value = config.proxy || "";
  document.getElementById("input-notify").checked = config.notify !== false;
}

// ---- 监听项渲染 ----
function watchTags(w) {
  if (w.number != null) {
    const tags = [
      w.comments ? "评论" : null,
      w.state_change ? "状态" : null,
    ]
      .filter(Boolean)
      .join(" / ");
    return `#${w.number} · ${tags}`;
  }
  return [
    w.issues ? "Issue" : null,
    w.prs ? "PR" : null,
    w.releases ? "Release" : null,
  ]
    .filter(Boolean)
    .join(" / ");
}

function renderWatchList() {
  const list = document.getElementById("watch-list");
  list.innerHTML = "";
  for (const w of config.watches) {
    const row = document.createElement("div");
    row.className = "watch-row fade-up";
    row.innerHTML = `<span class="watch-repo"></span><span class="watch-tags"></span>`;
    row.querySelector(".watch-repo").textContent = w.repo;
    row.querySelector(".watch-tags").textContent = watchTags(w);
    const del = document.createElement("button");
    del.className = "watch-del";
    del.textContent = "×";
    del.title = "移除监听";
    del.addEventListener("click", async () => {
      config = await invoke("remove_watch", { id: w.id });
      goDefaultView();
    });
    row.appendChild(del);
    list.appendChild(row);
  }
}

// ---- 事件流 ----
const KIND_LABEL = { issue: "Issue", pr: "PR", release: "Release", comment: "评论", state: "状态" };
const READ_KEY = "ghr.read";

function loadReadSet() {
  try {
    return new Set(JSON.parse(localStorage.getItem(READ_KEY)) || []);
  } catch {
    return new Set();
  }
}

function saveReadSet(set) {
  // 只留最近 500 条，防无限膨胀
  localStorage.setItem(READ_KEY, JSON.stringify([...set].slice(-500)));
}

function eventKey(ev) {
  return `${ev.kind}:${ev.repo}:${ev.number ?? ""}:${ev.created_at}`;
}

function addEventCards(events) {
  const feed = document.getElementById("event-feed");
  const empty = document.getElementById("feed-empty");
  if (empty) empty.remove();
  const readSet = loadReadSet();
  for (const ev of events) {
    const key = eventKey(ev);
    const card = document.createElement("a");
    card.className = "event-card fade-up" + (readSet.has(key) ? " read" : "");
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
      readSet.add(key);
      saveReadSet(readSet);
      card.classList.add("read");
      window.__TAURI__.opener.openUrl(ev.url);
    });
    feed.prepend(card);
  }
  // 最多保留 30 条
  while (feed.children.length > 30) feed.lastChild.remove();
}

// ---- 窗口控制 ----
document.getElementById("btn-min").addEventListener("click", () => getCurrentWindow().minimize());
document.getElementById("btn-close").addEventListener("click", () => getCurrentWindow().close());

// ---- 视图切换入口 ----
document.getElementById("btn-settings").addEventListener("click", async () => {
  await refreshConfig();
  document.getElementById("input-autostart").checked = await invoke("get_autostart");
  showView("view-settings");
});
document.getElementById("btn-guide-start").addEventListener("click", () => showView("view-add"));
document.getElementById("btn-back").addEventListener("click", goDefaultView);

// ---- 添加监听：自动识别 owner/name 或 Issue/PR 链接 ----
const GH_URL_RE = /github\.com\/([\w.-]+)\/([\w.-]+)\/(?:issues|pull)\/(\d+)/i;

function parseTarget(raw) {
  const s = raw.trim().replace(/^https?:\/\//, "");
  const m = s.match(GH_URL_RE);
  if (m) return { repo: `${m[1]}/${m[2]}`, number: Number(m[3]) };
  if (/^[\w.-]+\/[\w.-]+$/.test(s.replace(/\/+$/, ""))) {
    return { repo: s.replace(/\/+$/, ""), number: null };
  }
  return null;
}

function refreshAddOptions() {
  const { number } = parseTarget(document.getElementById("input-target").value) || {};
  document.getElementById("repo-options").classList.toggle("hidden", number != null);
  document.getElementById("item-options").classList.toggle("hidden", number == null);
}

document.getElementById("input-target").addEventListener("input", refreshAddOptions);

document.getElementById("btn-add-watch").addEventListener("click", () => {
  document.getElementById("input-target").value = "";
  document.getElementById("add-error").classList.add("hidden");
  refreshAddOptions();
  showView("view-add");
});
document.getElementById("btn-add-cancel").addEventListener("click", goDefaultView);

document.getElementById("btn-add-confirm").addEventListener("click", async () => {
  const errEl = document.getElementById("add-error");
  const parsed = parseTarget(document.getElementById("input-target").value);
  if (!parsed) {
    errEl.textContent = "无法识别：请输入 owner/name 或完整的 Issue/PR 链接";
    errEl.classList.remove("hidden");
    return;
  }
  try {
    config = await invoke("add_watch", {
      repo: parsed.repo,
      number: parsed.number,
      issues: document.getElementById("chk-issues").checked,
      prs: document.getElementById("chk-prs").checked,
      releases: document.getElementById("chk-releases").checked,
      comments: document.getElementById("chk-comments").checked,
      stateChange: document.getElementById("chk-state").checked,
    });
    goDefaultView();
  } catch (err) {
    errEl.textContent = String(err);
    errEl.classList.remove("hidden");
  }
});

// ---- 设置项 ----
document.getElementById("input-token").addEventListener("change", async (e) => {
  const val = e.target.value.trim();
  config = await invoke("set_token", { token: val || null });
  updateTokenState();
});

document.getElementById("input-proxy").addEventListener("change", async (e) => {
  config = await invoke("set_proxy", { proxy: e.target.value.trim() || null });
});

document.getElementById("input-notify").addEventListener("change", async (e) => {
  config = await invoke("set_notify", { notify: e.target.checked });
});

document.getElementById("input-autostart").addEventListener("change", async (e) => {
  try {
    e.target.checked = await invoke("set_autostart", { enable: e.target.checked });
  } catch (err) {
    console.warn("设置开机自启失败:", err);
    e.target.checked = await invoke("get_autostart");
  }
});

document.getElementById("input-color").addEventListener("input", (e) => {
  saveSettings({ color: e.target.value });
  applyAppearance();
});

document.getElementById("input-opacity").addEventListener("input", (e) => {
  const v = Number(e.target.value);
  document.getElementById("opacity-val").textContent = `${v}%`;
  saveSettings({ opacity: v });
  applyAppearance();
});

function applyWindowMode() {
  // 默认贴桌面（不置顶）；嵌入失败时降级为普通窗口
  const { topmost = false } = loadSettings();
  invoke("set_window_mode", { topmost }).catch((err) => {
    console.warn("窗口模式设置失败，降级为普通窗口:", err);
  });
}

document.getElementById("input-topmost").addEventListener("change", (e) => {
  saveSettings({ topmost: e.target.checked });
  applyWindowMode();
});

// ---- 初始化 ----
(async function init() {
  const { color = "#4a90d9", opacity = 70, topmost = false } = loadSettings();
  document.getElementById("input-color").value = color;
  document.getElementById("input-opacity").value = opacity;
  document.getElementById("opacity-val").textContent = `${opacity}%`;
  document.getElementById("input-topmost").checked = topmost;
  applyAppearance();
  applyWindowMode();

  try {
    await refreshConfig();
  } catch (e) {
    console.warn("读取配置失败:", e);
  }
  goDefaultView();

  listen("ghr-events", (e) => {
    if (Array.isArray(e.payload)) addEventCards(e.payload);
  });

  listen("ghr-poll-error", (e) => {
    const el = document.getElementById("poll-error");
    el.textContent = `⚠ 轮询失败：${e.payload}（检查网络/代理后等待下轮自动重试）`;
    el.classList.remove("hidden");
  });
})();
