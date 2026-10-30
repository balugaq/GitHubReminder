# GitHubReminder — 桌面 GitHub 通知小窗 · 项目规划

> 一句话：贴在桌面上的半透明通知框，通过 GitHub API 监听指定仓库 / Issue / PR / Release，新事件从底部 0.5s 上浮浮现。

## 一、项目目标

1. **外观**：480×320 默认尺寸、可拉伸的半透明色块，顶部 title「GitHub 通知」，颜色可调。
2. **嵌入感**：贴在桌面（WorkerW 方案），但**默认置顶模式 + 可切换嵌入模式**（嵌入会被最大化窗口完全遮住，见概念对话里的取舍结论；成熟工具都这么做）。
3. **监听能力**（GitHub REST API，轮询实现，无 WebSocket 依赖）：
   - 仓库级：新 Issue / 新 PR / 新 Release，可勾选关注哪些类型；
   - 单条颗粒度：对指定 Issue / PR 监听新 comment、状态开关变更；
   - 触发系统 Toast 弹窗（可选），点击跳转 GitHub 页面。
4. **引导流程**：无任何监听项时，框内显示引导按钮 → 添加 GitHub PAT（令牌）→ 添加监听对象。
5. **动效**：所有内容展示统一用 0.5s 自下而上浮现（CSS transition 即可）。

## 二、技术栈决策（哪些用、哪些不用）

| 组件 | 用不用 | 理由 |
|---|---|---|
| Tauri 2 | ✅ | WebView2 渲染 UI，包体小 |
| Rust | ✅ | WorkerW 嵌入、窗口层级、PAT 加密存储、轮询调度、系统 Toast |
| Node.js | ✅（仅构建期） | 只当 Vite 打包工具用，运行时完全不依赖 |
| 前端框架（React/Vue） | ❌ | 一个 480×320 的小窗，原生 JS + CSS 足够，少一层构建复杂度 |
| WebSocket / GraphQL | ❌ | REST 轮询（仓库级 60s、单条 120s）已满足，省掉 PAT 高权限和复杂度 |
| 数据库 | ❌ | 配置 + 已读事件状态存 JSON 文件（Tauri 的 appDataDir） |

## 三、架构

```
┌─ WebView (前端: 原生 JS/CSS) ─────────────┐
│  设置面板 / 监听列表 / 事件卡片 / 引导页    │
│  ← invoke() →        ← event.listen() ──  │
├─ Rust 核心 (src-tauri) ───────────────────┤
│  · 窗口管理: 置顶/嵌入(WorkerW)/拉伸/穿透  │
│  · 轮询调度: tokio 定时任务, 按 watch 规则 │
│  · GitHub client: reqwest + PAT           │
│  · 事件对比: ETag/If-Modified-Since 增量   │
│  · 存储: JSON (config.json + seen.json)   │
│  · PAT 加密: windows 凭据管理器            │
│  · 系统通知: tauri-plugin-notification    │
└───────────────────────────────────────────┘
```

命令（invoke）清单：
- `add_watch` / `remove_watch` / `list_watches`
- `set_token` / `has_token`
- `set_window_mode(topmost | workerw | normal)`
- `set_color` / `set_opacity`

事件（Rust → 前端 emit）：
- `new-events`（一批新 Issue/PR/Release/comment/状态变更）
- `poll-error`（网络失败、PAT 失效 → 前端标红提示）

## 四、监听规则数据模型

```jsonc
// config.json
{
  "watches": [
    { "id": "w1", "type": "repo", "repo": "owner/name",
      "issues": true, "prs": true, "releases": false },
    { "id": "w2", "type": "issue", "repo": "owner/name", "number": 42,
      "comments": true, "state_change": true }
  ],
  "appearance": { "color": "#4A90D9", "opacity": 0.7, "mode": "topmost" },
  "poll_seconds": { "repo": 60, "item": 120 }
}
```

API 端点映射：
- 新 Issue → `GET /repos/{o}/{r}/issues?state=all&since=<上次时间>`
- 新 PR → 同上（`pull_request` 字段区分）
- 新 Release → `GET /repos/{o}/{r}/releases/latest` 比对 id
- 新 comment → `GET /repos/{o}/{r}/issues/{n}/comments?since=`
- 状态变更 → `GET /repos/{o}/{r}/issues/{n}` 比对 `state`

增量技巧：条件请求带 `If-None-Match`（ETag），304 不消耗配额。PAT 即使只读也建议配，未认证 60 次/小时很快用完。

## 五、WorkerW 嵌入要点（Rust 侧）

1. `FindWindowW("Progman", null)` → `SendMessageW(progman, 0x052C, 0, 0)` 召唤 WorkerW；
2. `EnumWindows` 找到 `SHELLDLL_DefView` 的父级 WorkerW，取它**后面那个** WorkerW；
3. `SetParent(hwnd, workerw)` —— 保持 `WS_POPUP` 样式，配合 `WS_EX_TOOLWINDOW` 隐藏 Alt+Tab / 任务栏；
4. **键盘焦点坑**：嵌成 child 后 WebView 可能收不到键盘输入，Dev 期专门测「引导页输入 PAT」这条路径，不行就回退置顶模式；
5. 提供失败回退：嵌入失败自动回置顶，UI 上提示。

## 六、Dev 期快速调试

1. **双窗口调试**：主窗口跑正常 UI；`tauri dev` 热重载前端，Rust 改动自动重编译。
2. **不嵌桌面临时开关**：环境变量 `GHR_NO_EMBED=1` 跳过 WorkerW 逻辑，调试布局时窗口就是普通窗口，免得被自己埋进桌面够不着。
3. **兜底快捷键**：全局注册 `Ctrl+Alt+G` 强制显示/重置窗口位置——窗口跑到屏幕外或嵌没影了能救回来。
4. **假数据模式**：`GHR_MOCK=1` 时 Rust 不打真 API，定时发假事件，专调浮现动画和卡片渲染，不烧配额。
5. **日志**：`tauri-plugin-log`，Rust 和前端 console 统一落文件，出问题有轨迹可查。

## 七、里程碑

| 阶段 | 内容 | 验收 |
|---|---|---|
| M1 | Tauri 骨架 + 480×320 半透明框 + 置顶模式 + 引导页 | 无监听时显示引导按钮 |
| M2 | PAT 设置 + 仓库级监听（新 Issue）+ 0.5s 上浮动画 | 真仓库开个 issue 能弹出来 |
| M3 | PR / Release / 单条颗粒度监听 + 事件卡片列表 + 已读状态 | 各类型事件区分展示 |
| M4 | WorkerW 嵌入 + 模式切换 + 拉伸 + 颜色/透明度调节 + 系统弹窗 | 三种窗口模式切换正常 |
| M5 | 打包（NSIS 安装包）+ 开机自启 + 托盘图标 | 单 exe/安装包可用 |

## 八、风险清单

1. **嵌入模式键盘输入**（最大不确定性）→ M4 第一天就测，不行降级为「置顶 + 低透明度」。
2. **GitHub API 配额** → 强制建议配 PAT；轮询间隔可调；ETag 条件请求。
3. **WebView2 依赖** → Win10/11 自带，NSIS 打包时可配置下载器兜底。
4. **窗口被 Win+D 隐藏**（置顶模式）→ 后续可加防隐藏钩子，先不做。
