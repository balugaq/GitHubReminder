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
// all | single | owner:<owner> | repo:<owner/name>
let activeGroup = "all";
let sortDesc = true;
// owner 展开状态跨会话记忆
let expandedOwners = new Set(loadSettings().expandedOwners || []);

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
      w.stateChange ? ["b-state", "状态变更"] : null,
    ].filter(Boolean);
  }
  return [
    w.issues ? ["b-issue", "新 Issue"] : null,
    w.prs ? ["b-pr", "新 PR"] : null,
    w.releases ? ["b-release", "新 Release"] : null,
    w.closedIssues ? ["b-state", "关 Issue"] : null,
    w.closedPrs ? ["b-state", "关 PR"] : null,
  ].filter(Boolean);
}

function groupKey(w) {
  return w.number != null ? "single" : `repo:${w.repo}`;
}

function watchInGroup(w, group) {
  if (group === "all") return true;
  if (group === "single") return w.number != null;
  if (group.startsWith("owner:")) return w.number == null && w.repo.split("/")[0] === group.slice(6);
  if (group.startsWith("repo:")) return w.number == null && w.repo === group.slice(5);
  return false;
}

function saveExpandedOwners() {
  saveSettings({ expandedOwners: [...expandedOwners] });
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
      const inGroup = watchInGroup(w, activeGroup);
      const hay = `${w.repo} ${w.title || ""}`.toLowerCase();
      return inGroup && (!q || hay.includes(q));
    })
    .sort((a, b) => (sortDesc ? b.created - a.created : a.created - b.created));
  document.getElementById("btn-sort").textContent = sortDesc ? "↓ 最近添加" : "↑ 最早添加";

  // 分组导航：owner 一级可展开，展开后列出该 owner 的仓库级监听
  const owners = new Map(); // owner -> Map(repo -> count)
  let singleCount = 0;
  for (const w of config.watches) {
    if (w.number != null) {
      singleCount++;
      continue;
    }
    const [owner, repo] = w.repo.split("/");
    if (!owners.has(owner)) owners.set(owner, new Map());
    const repos = owners.get(owner);
    repos.set(repo, (repos.get(repo) || 0) + 1);
  }
  const nav = document.getElementById("group-nav");
  nav.innerHTML = `<div class="nav-label">项目分组</div>`;
  const mkNavRow = ({ key, label, cnt, className = "group-item", expandable = null }) => {
    const el = document.createElement("div");
    el.className = `${className}${activeGroup === key ? " active" : ""}`;
    if (expandable !== null) {
      const arrow = document.createElement("span");
      arrow.className = "owner-arrow";
      arrow.textContent = expandable ? "▾" : "▸";
      el.appendChild(arrow);
    }
    const name = document.createElement("span");
    name.textContent = label;
    name.title = label;
    const badge = document.createElement("span");
    badge.className = "cnt";
    badge.textContent = cnt;
    el.append(name, badge);
    el.addEventListener("click", () => {
      activeGroup = key;
      render();
    });
    return el;
  };
  nav.appendChild(mkNavRow({ key: "all", label: "全部", cnt: config.watches.length }));
  nav.appendChild(mkNavRow({ key: "single", label: "🔖 单条监听", cnt: singleCount }));
  for (const [owner, repos] of [...owners.entries()].sort(([a], [b]) => a.localeCompare(b))) {
    const ownerCount = [...repos.values()].reduce((a, b) => a + b, 0);
    const expanded = expandedOwners.has(owner);
    const row = mkNavRow({
      key: `owner:${owner}`,
      label: owner,
      cnt: ownerCount,
      expandable: expanded,
    });
    row.addEventListener("click", () => {
      // 点 owner = 展开/收起并按 owner 过滤
      if (expanded) expandedOwners.delete(owner);
      else expandedOwners.add(owner);
      saveExpandedOwners();
      activeGroup = `owner:${owner}`;
      render();
    });
    nav.appendChild(row);
    if (expanded) {
      for (const [repo, cnt] of [...repos.entries()].sort(([a], [b]) => a.localeCompare(b))) {
        const full = `${owner}/${repo}`;
        const sub = mkNavRow({ key: `repo:${full}`, label: repo, cnt, className: "group-item group-sub" });
        sub.addEventListener("click", () => {
          activeGroup = `repo:${full}`;
          render();
        });
        nav.appendChild(sub);
      }
    }
  }

  // 列表 / 空态
  const list = document.getElementById("manage-list");
  list.innerHTML = "";
  document.getElementById("manage-stat").textContent =
    `共 ${items.length} 项监听 · ${owners.size + (singleCount > 0 ? 1 : 0)} 个分组`;

  if (items.length === 0) {
    const empty = document.createElement("div");
    empty.className = "empty-state";
    empty.innerHTML = `
      <div class="empty-icon">📭</div>
      <div class="empty-title">这里空空如也</div>
      <div class="empty-desc">添加仓库可以监听新 Issue / PR / Release；<br>贴一条 Issue 或 PR 链接，还能监听新评论和状态变更。</div>
      <button class="btn-primary">开始添加监听</button>`;
    // 空态引导 = 直接在本窗口展开添加表单
    empty.querySelector("button").addEventListener("click", () => toggleAddForm(true));
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
    editBtn.title = "编辑监听设置";
    editBtn.addEventListener("click", () => openEditForm(w));
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

// ---- 编辑表单：显示名 + 勾选项（repo/number 不可改，后端 None = 不动） ----
let editingId = null;

function openEditForm(w) {
  editingId = w.id;
  document.getElementById("edit-form").classList.remove("hidden");
  document.getElementById("edit-error").classList.add("hidden");
  document.getElementById("edit-title").value = w.title || "";
  document.getElementById("edit-target").textContent =
    `监听对象：${w.repo}${w.number != null ? ` #${w.number}` : ""}`;
  const isRepo = w.number == null;
  document.getElementById("edit-repo-options").classList.toggle("hidden", !isRepo);
  document.getElementById("edit-item-options").classList.toggle("hidden", isRepo);
  const setChk = (id, v) => (document.getElementById(id).checked = !!v);
  setChk("edit-chk-issues", w.issues);
  setChk("edit-chk-prs", w.prs);
  setChk("edit-chk-releases", w.releases);
  setChk("edit-chk-closed-issues", w.closedIssues);
  setChk("edit-chk-closed-prs", w.closedPrs);
  setChk("edit-chk-comments", w.comments);
  setChk("edit-chk-state", w.stateChange);
  document.getElementById("edit-form").scrollIntoView({ block: "nearest" });
}

function toggleEditForm(show) {
  document.getElementById("edit-form").classList.toggle("hidden", !show);
  editingId = null;
}

// 不适用的那一组传 null（后端不动它），适用的那组传当前勾选
document.getElementById("btn-edit-save").addEventListener("click", async () => {
  if (!editingId) return;
  const errEl = document.getElementById("edit-error");
  const isRepo = !document.getElementById("edit-repo-options").classList.contains("hidden");
  const g = (id) => document.getElementById(id).checked;
  try {
    config = await invoke("update_watch", {
      id: editingId,
      title: document.getElementById("edit-title").value,
      ...(isRepo
        ? {
            issues: g("edit-chk-issues"),
            prs: g("edit-chk-prs"),
            releases: g("edit-chk-releases"),
            closedIssues: g("edit-chk-closed-issues"),
            closedPrs: g("edit-chk-closed-prs"),
          }
        : {
            comments: g("edit-chk-comments"),
            stateChange: g("edit-chk-state"),
          }),
    });
    emit("ghr-config-changed").catch(() => {});
    toggleEditForm(false);
    render();
  } catch (err) {
    errEl.textContent = String(err);
    errEl.classList.remove("hidden");
  }
});

document.getElementById("btn-edit-cancel").addEventListener("click", () => toggleEditForm(false));

// ---- 监听 tab 控件 ----
document.getElementById("manage-search").addEventListener("input", render);
document.getElementById("btn-sort").addEventListener("click", () => {
  sortDesc = !sortDesc;
  render();
});

// 回补最近 14 天内创建、当前仍 open 的 Issue/PR，补进事件流并发通知
document.getElementById("btn-backfill").addEventListener("click", async (e) => {
  const btn = e.target;
  if (btn.disabled) return;
  btn.disabled = true;
  const old = btn.textContent;
  btn.textContent = "⟳ 回补中…";
  try {
    const n = await invoke("backfill_14d");
    btn.textContent = n > 0 ? `✓ 已回补 ${n} 条` : "✓ 无需回补";
    setTimeout(() => { btn.textContent = old; }, 3000);
  } catch (err) {
    alert(`回补失败：${err}`);
    btn.textContent = old;
  } finally {
    btn.disabled = false;
  }
});

// ---- 内嵌添加表单（原主窗口添加页迁入） ----
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

function toggleAddForm(show) {
  document.getElementById("add-form").classList.toggle("hidden", !show);
  if (show) {
    document.getElementById("input-target").value = "";
    document.getElementById("add-error").classList.add("hidden");
    refreshAddOptions();
    document.getElementById("input-target").focus();
  }
}

document.getElementById("input-target").addEventListener("input", refreshAddOptions);
document.getElementById("btn-manage-add").addEventListener("click", () => {
  toggleAddForm(document.getElementById("add-form").classList.contains("hidden"));
});
document.getElementById("btn-add-cancel").addEventListener("click", () => toggleAddForm(false));

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
      closedIssues: document.getElementById("chk-closed-issues").checked,
      closedPrs: document.getElementById("chk-closed-prs").checked,
      comments: document.getElementById("chk-comments").checked,
      stateChange: document.getElementById("chk-state").checked,
    });
    emit("ghr-config-changed").catch(() => {});
    toggleAddForm(false);
    render();
  } catch (err) {
    errEl.textContent = String(err);
    errEl.classList.remove("hidden");
  }
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
  document.getElementById("input-poll-interval").value = config.pollInterval || 60;
  document.getElementById("input-github-username").value = config.githubUsername || "";
  document.getElementById("input-qq-notify").checked = config.qqNotify === true;
  document.getElementById("input-qq-port").value = config.qqRelayPort || "";
  document.getElementById("input-qq-appid").value = config.qqAppId || "";
  document.getElementById("input-qq-secret").value = config.qqAppSecret || "";
  document.getElementById("input-qq-openid").value = config.qqOpenid || "";
  document.getElementById("input-qq-autoconfig").checked = config.qqAutoconfig === true;
  updateQqNotifyVisibility();
  document.getElementById("input-topmost").checked = loadSettings().topmost || false;
}

// QQ 互联开关关着收起整块；凭证只有自动配置开启时才需要（外部服务模式下由外部持有）
function updateQqNotifyVisibility() {
  const extra = document.getElementById("qq-notify-extra");
  const creds = document.getElementById("qq-credentials");
  const notifyOn = document.getElementById("input-qq-notify").checked === true;
  const autoOn = document.getElementById("input-qq-autoconfig").checked === true;
  if (extra) extra.classList.toggle("hidden", !notifyOn);
  if (creds) creds.classList.toggle("hidden", !(notifyOn && autoOn));
}

document.getElementById("input-token").addEventListener("change", async (e) => {
  const val = e.target.value.trim();
  try {
    config = await invoke("set_token", { token: val || null });
  } catch (err) {
    // 凭据管理器写入失败时 token 不会被改动，提示后让用户重试
    alert("保存令牌失败：" + err);
  }
  updateTokenState();
});

document.getElementById("input-proxy").addEventListener("change", async (e) => {
  config = await invoke("set_proxy", { proxy: e.target.value.trim() || null });
});

document.getElementById("input-notify").addEventListener("change", async (e) => {
  config = await invoke("set_notify", { notify: e.target.checked });
});

document.getElementById("input-poll-interval").addEventListener("change", async (e) => {
  const v = parseInt(e.target.value, 10);
  if (!Number.isInteger(v) || v <= 0) {
    e.target.value = config.pollInterval || 60;
    return;
  }
  config = await invoke("set_poll_interval", { secs: v });
  e.target.value = config.pollInterval || 60;
});

// GitHub 用户名：用于判断 assignee 里是否有本人（空 = 不判断）
document.getElementById("input-github-username").addEventListener("change", async (e) => {
  config = await invoke("set_github_username", {
    username: e.target.value.trim() || null,
  });
  e.target.value = config.githubUsername || "";
});

document.getElementById("input-qq-notify").addEventListener("change", async (e) => {
  config = await invoke("set_qq_notify", { notify: e.target.checked });
  updateQqNotifyVisibility();
});

// 端口/凭证任一改动都整包保存（后端按需重启内置中转）
async function saveQqSettings() {
  const portEl = document.getElementById("input-qq-port");
  const port = parseInt(portEl.value, 10);
  config = await invoke("set_qq_settings", {
    port: Number.isInteger(port) && port > 0 && port <= 65535 ? port : null,
    appId: document.getElementById("input-qq-appid").value.trim() || null,
    appSecret: document.getElementById("input-qq-secret").value.trim() || null,
    openid: document.getElementById("input-qq-openid").value.trim() || null,
  });
}

["input-qq-port", "input-qq-appid", "input-qq-secret", "input-qq-openid"].forEach((id) => {
  document.getElementById(id).addEventListener("change", saveQqSettings);
});

document.getElementById("input-qq-autoconfig").addEventListener("change", async (e) => {
  config = await invoke("set_qq_autoconfig", { enable: e.target.checked });
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
