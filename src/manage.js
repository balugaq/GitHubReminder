// 管理窗口：监听分组/搜索/排序/改名/删除 + 设置页（原主窗口设置页迁入）
// 跨窗口联动：ghr-nav 让主窗口切视图，ghr-appearance 通知主窗口同步外观

const { getCurrentWindow } = window.__TAURI__.window;
const { invoke } = window.__TAURI__.core;
const { listen, emit } = window.__TAURI__.event;

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

// ---- Tab 切换 ----
for (const btn of document.querySelectorAll(".tab-btn")) {
  btn.addEventListener("click", () => {
    for (const b of document.querySelectorAll(".tab-btn")) b.classList.remove("active");
    btn.classList.add("active");
    for (const p of document.querySelectorAll(".tab-panel")) p.classList.add("hidden");
    document.getElementById(btn.dataset.tab).classList.remove("hidden");
  });
}

// ---- 数据 ----
let config = { token: null, watches: [] };
let activeGroup = "all"; // all | single | repo:<owner/name>
let sortDesc = true;

async function refreshConfig() {
  config = await invoke("get_config");
}

function watchTitle(w) {
  return w.title || (w.number != null ? `${w.repo} #${w.number}` : w.repo);
}

function watchBadges(w) {
  if (w.number != null) {
    return [
      w.comments ? ["b-comment", "新评论"] : null,
      w.state_change ? ["b-state", "状态变更"] : null,
    ].filter(Boolean);
  }
  return [
    w.issues ? ["b-issue", "新 Issue"] : null,
    w.prs ? ["b-pr", "新 PR"] : null,
    w.releases ? ["b-release", "新 Release"] : null,
  ].filter(Boolean);
}

function groupKey(w) {
  return w.number != null ? "single" : `repo:${w.repo}`;
}

function groupLabel(key) {
  if (key === "all") return "全部";
  if (key === "single") return "🔖 单条监听";
  return key.slice(5);
}

function timeAgo(unixSecs) {
  if (!unixSecs) return "—";
  const diff = Math.max(0, Date.now() / 1000 - unixSecs);
  if (diff < 3600) return `${Math.floor(diff / 60)} 分钟前添加`;
  if (diff < 86400) return `${Math.floor(diff / 3600)} 小时前添加`;
  return `${Math.floor(diff / 86400)} 天前添加`;
}

// ---- 渲染 ----
function render() {
  const q = document.getElementById("manage-search").value.trim().toLowerCase();
  const items = config.watches
    .filter((w) => {
      const inGroup =
        activeGroup === "all" || groupKey(w) === activeGroup;
      const hay = `${w.repo} ${w.title || ""}`.toLowerCase();
      return inGroup && (!q || hay.includes(q));
    })
    .sort((a, b) => (sortDesc ? b.created - a.created : a.created - b.created));
  document.getElementById("btn-sort").textContent = sortDesc ? "↓ 最近添加" : "↑ 最早添加";

  // 分组导航
  const counts = new Map();
  for (const w of config.watches) {
    const k = groupKey(w);
    counts.set(k, (counts.get(k) || 0) + 1);
  }
  const nav = document.getElementById("group-nav");
  nav.innerHTML = `<div class="nav-label">项目分组</div>`;
  const mkGroup = (key, cnt) => {
    const el = document.createElement("div");
    el.className = "group-item" + (activeGroup === key ? " active" : "");
    const name = document.createElement("span");
    name.textContent = groupLabel(key);
    name.title = groupLabel(key);
    const badge = document.createElement("span");
    badge.className = "cnt";
    badge.textContent = cnt;
    el.append(name, badge);
    el.addEventListener("click", () => {
      activeGroup = key;
      render();
    });
    nav.appendChild(el);
  };
  mkGroup("all", config.watches.length);
  for (const [k, c] of counts) mkGroup(k, c);

  // 列表 / 空态
  const list = document.getElementById("manage-list");
  list.innerHTML = "";
  document.getElementById("manage-stat").textContent = `共 ${items.length} 项监听 · ${counts.size} 个项目`;

  if (items.length === 0) {
    const empty = document.createElement("div");
    empty.className = "empty-state";
    empty.innerHTML = `
      <div class="empty-icon">📭</div>
      <div class="empty-title">这里空空如也</div>
      <div class="empty-desc">添加仓库可以监听新 Issue / PR / Release；<br>贴一条 Issue 或 PR 链接，还能监听新评论和状态变更。</div>
      <button class="btn-primary">开始添加监听</button>`;
    // 空态引导 = 走主窗口初次引导页流程（主窗口自行 show + showView("view-add")）
    empty.querySelector("button").addEventListener("click", () => {
      emit("ghr-nav", "view-add").catch((err) => console.warn("导航失败:", err));
      getCurrentWindow().close();
    });
    list.appendChild(empty);
    document.getElementById("manage-stat").textContent = "暂无监听";
    return;
  }

  for (const w of items) {
    const row = document.createElement("div");
    row.className = "mg-row";

    const main = document.createElement("div");
    main.className = "mg-main";

    const name = document.createElement("div");
    name.className = "mg-name";
    name.textContent = watchTitle(w);
    if (!w.title) name.title = "默认显示名，点 ✏️ 可自定义";
    main.appendChild(name);

    const sub = document.createElement("div");
    sub.className = "mg-sub";
    sub.textContent = `${w.repo}${w.number != null ? ` #${w.number}` : ""} · ${timeAgo(w.created)}`;
    main.appendChild(sub);

    const badges = document.createElement("div");
    badges.className = "mg-badges";
    for (const [cls, label] of watchBadges(w)) {
      const b = document.createElement("span");
      b.className = `badge ${cls}`;
      b.textContent = label;
      badges.appendChild(b);
    }
    main.appendChild(badges);
    row.appendChild(main);

    // 操作：改名 / 删除
    const actions = document.createElement("div");
    actions.className = "mg-actions";

    const editBtn = document.createElement("button");
    editBtn.textContent = "✏️";
    editBtn.title = "编辑显示名";
    editBtn.addEventListener("click", () => startTitleEdit(row, w));
    actions.appendChild(editBtn);

    const delBtn = document.createElement("button");
    delBtn.textContent = "🗑";
    delBtn.title = "移除监听";
    delBtn.addEventListener("click", async () => {
      config = await invoke("remove_watch", { id: w.id });
      emit("ghr-config-changed").catch(() => {});
      render();
    });
    actions.appendChild(delBtn);

    row.appendChild(actions);
    list.appendChild(row);
  }
}

function startTitleEdit(row, w) {
  const main = row.querySelector(".mg-main");
  const nameEl = main.querySelector(".mg-name");
  const input = document.createElement("input");
  input.className = "title-input";
  input.value = w.title || watchTitle(w);
  input.placeholder = "留空恢复默认";
  nameEl.replaceWith(input);
  input.focus();
  input.select();

  const commit = async () => {
    config = await invoke("update_watch", { id: w.id, title: input.value });
    render();
  };
  input.addEventListener("keydown", (e) => {
    if (e.key === "Enter") input.blur();
    if (e.key === "Escape") {
      input.value = w.title || "";
      input.blur();
    }
  });
  input.addEventListener("blur", commit);
}

// ---- 监听 tab 控件 ----
document.getElementById("manage-search").addEventListener("input", render);
document.getElementById("btn-sort").addEventListener("click", () => {
  sortDesc = !sortDesc;
  render();
});

// 添加监听：复用主窗口添加页
document.getElementById("btn-manage-add").addEventListener("click", () => {
  emit("ghr-nav", "view-add").catch((err) => console.warn("导航失败:", err));
  getCurrentWindow().close();
});

// ---- 设置 tab（原主窗口设置逻辑迁入） ----
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
  document.getElementById("input-topmost").checked = loadSettings().topmost || false;
}

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
  emit("ghr-appearance").catch(() => {});
});

document.getElementById("input-opacity").addEventListener("input", (e) => {
  const v = Number(e.target.value);
  document.getElementById("opacity-val").textContent = `${v}%`;
  saveSettings({ opacity: v });
  applyAppearance();
  emit("ghr-appearance").catch(() => {});
});

document.getElementById("input-topmost").addEventListener("change", (e) => {
  saveSettings({ topmost: e.target.checked });
  invoke("set_window_mode", { topmost: e.target.checked }).catch((err) =>
    console.warn("窗口模式设置失败:", err)
  );
});

// ---- 窗口控制 ----
document.getElementById("btn-min").addEventListener("click", () => getCurrentWindow().minimize());
document.getElementById("btn-close").addEventListener("click", () => getCurrentWindow().close());

// ---- 初始化 ----
(async function init() {
  applyAppearance();
  try {
    await refreshConfig();
  } catch (e) {
    console.warn("读取配置失败:", e);
  }
  updateTokenState();
  try {
    document.getElementById("input-autostart").checked = await invoke("get_autostart");
  } catch {
    /* 自启状态读取失败不影响其余功能 */
  }
  const { color = "#4a90d9", opacity = 70 } = loadSettings();
  document.getElementById("input-color").value = color;
  document.getElementById("input-opacity").value = opacity;
  document.getElementById("opacity-val").textContent = `${opacity}%`;
  render();

  // 主窗口删了监听 / 配置变化时，管理窗口同步刷新
  listen("ghr-config-changed", async () => {
    await refreshConfig();
    render();
    updateTokenState();
  });
})();
