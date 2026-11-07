# TODO

## 已知待办
- [x] **主窗口位置/尺寸记忆：拉伸/移动后下次启动恢复到上次位置**（完成 2026-11-01 @master工作区直改，待主人实测）
  - 实现：Config 新增 `window` 字段（WindowRect，物理像素 x/y/width/height，serde default 向后兼容）；lib.rs 新增 remember_window——Moved/Resized 事件即更新内存并按 1s 节流落盘，点关闭隐藏前清零节流强制落一次最终位置；setup 启动时读到 window 就 set_size/set_position 恢复（物理像素，覆盖 tauri.conf 的 center）。嵌入模式不受影响：恢复发生在前端 invoke 嵌入之前，SetParent 不改位置。clippy -D warnings 零警告，7 个单测通过。
- [x] **主界面动画只在动态真有变更时触发**（完成 2026-10-31 @master工作区直改，未提交待验收）
  - 根因：showView 每次重放 fade-up，而它被 syncFromRust 调用（窗口每次聚焦/配置变更都触发）→ 无变更也闪动画。修复：删掉 main.js showView 里的重放逻辑，动画只在视图首次显示和新事件卡插入时播。
- [x] **评论通知反复提醒修复 + 评论提醒文案改"有新消息" + 同一 issue/PR 未读期间不重复提醒**（完成 2026-10-31 @master工作区直改——主人有未提交 WIP，worktree 基线落后故未走 worktree，改动未提交待主人验收）
  - 根因：`/issues/{n}/comments` 端点 `direction=desc` 不生效（固定升序返回），`comments.first()` 拿到最旧评论当游标 → 游标每轮回退 → 新于游标的评论每轮重报。修复：游标一律取全量最大 id，过滤只认 id（github.rs poll_item）。
  - 附加：① do_poll 加 AtomicBool 互斥防并发轮询双报（lib.rs）；② toast 中 comment 事件显示"有新消息"（toast_summary）；③ 后端 notified 集合去重——同一 issue/PR 弹过 toast 且未读（点事件卡即已读，mark_read 命令）则不再弹（lib.rs + main.js）。
- [x] **Win+D 仍会隐藏贴桌面的窗口**（完成 2026-10-31 @worktree/wind-restore 合并 55278f3；2026-11-06 重写守护 a2c25de；2026-11-07 主人确认修复）。
  - 最终方案：宿主回到壁纸层 WorkerW（Progman / SHELLDLL_DefView 虽对 Win+D 严格免疫，但会吞掉子窗口鼠标消息）；守护不再依赖 IsWindowVisible，改 200ms guard_tick 唤回 + 每 1s 无条件 raise_z 把窗口提回桌面层 Z 序顶部；鼠标按下期间跳过，唤不回来指数退避到 2s。
  - 手动最小化 / 托盘隐藏 / 点关闭统一置 USER_HIDDEN，守护见到就放手；异常时日志打印 `[winembed] 守护唤回: visible=… iconic=… embedded=… user_hidden=…` 便于定位。
  - 历史：早期「每 2 秒查可见性/最小化再 SW_RESTORE」方案经主人实测不生效，已废弃。合并冲突两处均为假冲突，取并集解决，clippy -D warnings 零警告。
- [x] **贴桌面后主界面拖不动 / 拉不动 / 按钮点不了**（完成 2026-11-06 8f06ca6；2026-11-07 主人确认修复）。
  - 根因：为追求 Win+D 严格免疫把宿主改成 Progman / SHELLDLL_DefView，桌面窗口过程吞掉子窗口鼠标消息。修复：宿主改回壁纸层 WorkerW，并让高频 ShowWindow / SetWindowPos 在鼠标按下期间跳过，避免打断按下/抬起配对。
- [x] **通知持久化 + 已读/悬挂 + 统计功能**（完成 2026-10-31 @master工作区直改，未提交待验收）
  - 持久化：事件落 config.json（`events`，上限 200 条），重启后 get_events 拉回，已读状态不丢。
  - 已读：点事件卡 → mark_read（粒度=同一 issue/PR 一次读掉），后端置 read=true。
  - 悬挂：feed 顶部悬挂区最多 3 条未读（高亮边框），其余进历史区（新在前）。
  - 统计：config.json `stats` 按 repo 和 repo#n 两维度累计各类型次数；主界面新增「📊 查看统计」入口 + 统计视图。
  - 改动：store.rs（StoredEvent/events/stats）、lib.rs（record_events/get_events/get_stats/mark_read 扩展）、index.html、main.js、styles.css。clippy -D warnings 零警告，视觉待主人目视确认。
  - 问题：收到通知后完全关闭软件再重开，通知全部丢失。
  - 需求：① 通知支持已读/未读状态；② 悬挂机制——主界面最多悬挂 3 条通知；③ 统计功能——按 owner/repo 或 owner/repo/#issue/pr 维度统计每个通知类型出现次数；④ 新增「查看统计」入口（按钮）。
- [x] **首次使用默认监听 microsoft/vscode**（完成 2026-10-31 @worktree/first-watch，已合并 master 08ee58d）
  - 实现：lib.rs setup 中检测 config.json 不存在 → 预置 microsoft/vscode 仓库级监听（issues+prs）并立即落盘。clippy -D warnings 零警告。
- [x] **仓库级监听配置增强：关 Issue / 关 PR 事件 + 默认勾选调整**（完成 2026-10-31 @master工作区直改，未提交待验收）
  - 后端：Watch 加 `closed_issues`/`closed_prs`（serde default 向后兼容），Config 加 `cursor_closed` 游标；github.rs 新增 `poll_closed`——拉 `state=closed&sort=updated`，按 **closed_at** 游标过滤（评论等 updated_at 变化不会误报），kind 为 `closed_issue`/`closed_pr`；add_watch 参数与校验扩展，remove_watch 清游标。
  - 前端：添加表单新增「关 Issue」「关 PR」勾选框，新 Issue/PR/Release/关Issue/关PR 全默认勾选（Release 由不勾改勾）；管理列表徽标、事件流 KIND_LABEL、kind 配色（灰系）同步。
  - 注意：首次运行预置的 microsoft/vscode 保持 issues+prs 不变（未开关闭/release 监听）。clippy -D warnings 零警告；旧配置里已有监听的 closed_issues/closed_prs 为 false，需手动编辑才开启（暂无编辑勾选入口，如需要可加）。
  - 待主人目视确认：添加表单勾选框布局、事件卡「关Issue/关PR」标签颜色。
  - 需求：① 仓库级监听增加「关 Issue」「关 PR」两类事件；② 新增监听的默认勾选：关 Issue、关 PR、Release 均默认勾选。
- [x] **QQ 互联通知接入（js → Rust 内置改写）**（完成 2026-10-31 @master工作区直改，未提交待验收）
  - qq_relay.js 已改写为内置模块 src-tauri/src/qqrelay.rs：最小 HTTP/1.1 服务（POST /send JSON、GET /send?text=、CORS）+ token 缓存（过期前 1 分钟刷新）+ @ 转义 + 1900 字分段；不再依赖 node / 外部脚本。
  - 设置项全部无默认值：中转端口、AppID、AppSecret、OpenID——四项 + 自动配置开，内置服务才启动；改设置即时起停（JoinHandle abort 重启）。
  - 自动配置开关保留（主人要求：外部可能已有服务在跑）：开 = 本应用起内置服务；关 = 只发推送不起服务。
  - 事件推送不走 HTTP 回环，直调 qqrelay::send_to_qq；失败静默。旧「通知链接」设置项随 node 方案删除。
  - 注意：AppSecret 明文存 config.json（与原 js 硬编码同级风险，如需可后续进凭据管理器）。clippy -D warnings 零警告。
  - 待主人实测：设置四项 + 开自动配置 → 端口监听、curl POST /send 推到 QQ；新事件推 QQ。
- [x] **五项改进：QQ配置显隐/监听频率/owner分组导航/编辑监听/关事件显示**（完成 2026-10-31 @master工作区直改，未提交待验收）
  - ① QQ 机器人端口/凭证改为开启「内置自动配置」才显示（#qq-credentials 二级收起）。
  - ② 监听频率开放设置：Config.poll_interval（默认 60，运行时夹 [10,3600]，轮循每轮重读改完即生效）；设置页输入框；主窗口空态文案动态化。
  - ③ 左侧导航改为 owner 一级分组（▾ 可展开）→ 展开列出该 owner 下仓库级监听（缩进子项）；单条监听独立分组；展开状态存 localStorage；activeGroup 新增 owner:<owner> 语义。
  - ④ 编辑监听：✏️ 打开编辑表单（显示名留空=恢复默认 + 全部勾选项），repo/number 不可改；后端 update_watch 扩展全字段（None=不动），保存校验仓库级五选一/单条级二选一。
  - ⑤ 关 Issue/PR 显示排查：渲染链路（KIND_LABEL/buildCard/CSS 配色）本来就没缺，根因是老监听改不了勾选 + add_watch 同 repo 去重导致关事件从未被轮询——④即解药，无额外代码。
  - clippy -D warnings 零警告。视觉待主人目视确认。
  - 修正（同日二轮）：① Watch 序列化是 camelCase，JS 误读 snake_case 的 closed_issues/closed_prs/state_change 恒 undefined → 编辑表单勾选不回显、列表徽标不出现（老 3 项 issues/prs/releases 碰巧两写法同名才正常）；已统一改读 closedIssues/closedPrs/stateChange。② 「全部」「单条监听」导航行漏绑 click 无响应；已把点击处理收敛进 mkNavRow。
- [x] **任务栏图标仍是旧图标**（完成 2026-11-01，主人确认已修复）。
- [x] **N1 通知重启后"消失"根因定位与修复**（完成 2026-11-01 @master工作区直改）
  - 现象：关闭 app 重开，主界面 4 个分类全空，看不到任何通知；托盘退出同样如此。
  - **根因（已用双向探针实测确认，非猜测）**：数据从未丢失——落盘 config.json 有 2 条合法实体、后端 `[init] 通知实体: 2 条`、后端 `get_events` 探针打印「正常返回 2 条」、前端探针打印「拿到 2 条」。断点在 **前端 `invoke` 调用这一跳**：故障态下 `invoke("get_events")` 抛错被 `loadEvents` 的 `catch` 吞掉（`allEvents = []`），界面因此显示为空，误判为"没有数据"。
  - **放大器**：默认 tab 恒为 `unread`，而这批实体状态是 `read`/`in_progress`（主人右键设过），即使 invoke 成功也落在非默认分类——这是"看起来全空"的第二层原因。
  - 已修：① `loadEvents`/`addEvents` 的 catch 改为 `console.error` + 界面 flash 提示，不再静默置空；② 新增「全部」tab 并作默认视图，四个状态分类保留；③ `store::save` 改原子写（写 config.json.tmp 再 rename 覆盖），杜绝强杀进程留下半截 JSON 导致下次启动整体清空。
  - 验收：新增单元测试 `save_overwrites_existing_file_on_windows`（覆盖已有文件 + 无 tmp 残留 + JSON 仍合法）通过；cargo clippy --all-targets -D warnings 零警告；node --check 通过。
  - 结论：**关 cmd 窗口/托盘退出不会导致保存失败**——数据在每次轮询后就落盘，不依赖退出流程。真正的风险是"写到一半被杀"，已由原子写消除。
- [x] **N2 QQ 推送文案改为主人指定格式 + 新增用户名设置**（完成 2026-11-01 @master工作区直改，依赖 N1）
  - 目标格式（每条事件一块，不折叠、全部发出，可分多次发）：
    ```
    [GitHubReminder]
    owner/repo #461
    [@me] [username] · 标题
    https://github.com/owner/repo/issues/461
    ```
  - `[Issuer]`/`[PRER]` 按主人确认**换成实际用户名**（提 issue / pr 的人）；行首 `[@me]` 改用短标记 **@me**，且**仅当 assignee 里有本人时**显示。
  - 「是否本人」判定：设置里新增 **GitHub 用户名** 输入项，由用户自行填写；拉取时需带出 issue/PR 的 `user.login` 与 `assignees[].login`（github.rs 的 Issue 结构体当前**没有**这两个字段，需补）。
  - 折叠策略调整：去掉「最多 8 条 + 折叠」，改为**全部条目都发**，超长按 qqrelay 的 1900 字上限分段（split_text 已按行切，链接不会被截断）。
  - 已实现：github.rs 的 Event 补 `author`/`assignees` 字段（Issue/IssueDetail/Release 均补 GhUser，poll_item 提前抓一次 issue 详情供 state/comment 共用，comment 的 author/assignees 取所属 issue）；store.rs 的 Config 补 `github_username`；lib.rs 新增 `set_github_username` 命令并重写 `qq_summary`；manage.html/manage.js 加用户名输入项。
  - 验收：新增 5 个 QQ 文案单元测试（三行格式逐字断言、@me 条件、用户名大小写、12 条不折叠）全通过；cargo clippy -D warnings 零警告；node --check 通过。
  - 待主人实测：填好用户名后，QQ 推送应形如「[@me] [octocat] · 标题」；系统 Toast 文案未改（仍为精简式），如需同步可再提。


- [x] **QQ 推送文案改为多行卡片式**（完成 2026-11-01 @master直改，待主人实测 QQ 推送效果）：
  ```
  [GitHubReminder] 通知 · GitHub 动态
  PR #459 · octocat/hello-world
  fix: 示例提交标题
  https://github.com/octocat/hello-world/pull/459
  ```
  即：第一行应用名+通知类别，第二行类型+编号+仓库，第三行标题，第四行链接（issue/pr/release 各自拼对应 URL）。当前是「#459 fix: 标题 ...以及另外 N 条新动态」的聚合一行式，要改。
  实现：qq_summary 每事件三行（`类型 #n · 仓库` / 标题 / 链接），推送头一行 `[GitHubReminder] 通知 · GitHub 动态`；评论统一「有新消息」。2026-11-06 按主人要求把 `[@me]`/`[用户名]` 标记**加回标题行**（`[@me] [author] · 标题`）：[@me] 仅在 assignee 含设置里的 GitHub 用户名时显示，其他情况只留 `[author]` 或纯标题。
- [x] **拉取可靠性：不依赖时间戳判断新事件**（完成 2026-11-01 @master直改，待主人实测）
  实现：github.rs 新增 entity_seen 兜底——poll_issues 报前查实体表已有该 repo+number 则跳过；poll_closed 报前查实体 state 已是 closed 则跳过。时间戳游标仍是主判定，兜底只在游标异常时防重报；实体被 200 条上限裁剪后回到纯游标。新增 2 个单测。
- [x] **主界面启动不显示 / 窗口过小：位置记忆存到最小化离屏矩形**（完成 2026-11-06 @master直改，未提交待主人实测）：config.json 实测存的是 `x/y=-32000` + `199×34`，启动时 set_size/set_position 把它恢复到屏幕外。修复：① remember_window 跳过最小化/隐藏事件与离屏/小于 240 的矩形；② 启动读到坏存档则忽略、置空并落盘自愈；③ tauri.conf 主窗口 minWidth/minHeight 改 240×240 且运行时 set_min_size 兜底（配置只约束拖拽）；④ 托盘切换遇到离屏/过小窗口强制拉回居中。新增 3 个几何单测。
- [x] **添加仓库时弹出 Issue/PR 选择弹窗**（完成 2026-11-01 @master直改，待主人实测；后端 list_repo_items 命令 + 管理窗口勾选弹窗，开/闭状态筛选，勾中按单条监听（新评论+状态变更）批量添加，已在监听的置灰默认勾选，重复添加报错汇总提示；视觉待主人目视确认）。
- [x] **Explorer 重启后嵌入窗口的父级失效处理**（无需求，2026-11-01 主人确认不做）。
- [x] **主界面批量选择通知并标记状态**（完成 2026-11-07 @master直改）：通知卡片左上角勾选框多选；选中后操作条选状态 + 应用（后端 `set_status_batch` 一次落盘改多张卡）；完成后底部向上滑入 toast「已完成编辑 · N 条」+ 撤销按钮（8s 有效）。2026-11-07 主人确认撤销语义 = **选择状态回滚 + 操作回滚**：`captureUndo` 统一封存逐条旧状态与编辑时的选中集合，撤销时逐条写回状态、先清空当前选择再精确重建那批选中（不与期间新勾的叠加、剪掉已失效卡片）；写失败则现场与 toast 一起还回，可重试。纯逻辑抽 `src/batch.js`（buildBatch / buildUndo / captureUndo / restoreSelection）配 8 个 Node 单测，Rust 侧 `apply_statuses` 配 4 个单测；视觉与交互待主人目视确认。
- [x] **嵌入模式下键盘输入验证**（完成 2026-10-31，主人实测 PAT 输入框在 WorkerW 子窗口中可正常打字）。
- [x] **PAT 明文存储 → 迁移到 Windows 凭据管理器加密**（完成 2026-10-31 @master工作区直改，未提交待验收）  - 实现：Config.token 改 `#[serde(skip)]`（内存持有，config.json 不再落明文）；store.rs 新增 save_token/load_token 走 keyring 3（windows-native 凭据管理器，服务名 GitHubReminder/账户 pat）。
  - 迁移：启动时检测 config.json 里的旧明文 token → 写入凭据管理器并打印日志；写失败则本次会话仅内存保留（不回写磁盘）。凭据管理器读不到时回退会话内存值。
  - 修正（2026-10-31 二次）：初版 `#[serde(skip)]` 会把 token 从 get_config/set_token 的 API 响应里一并删掉，前端永远显示"未设置"（表现为无法添加 token）。改为 token 正常序列化、仅 store::save 落盘前剔除；启动时 cfg.token = load_token()（凭据管理器为唯一来源），不做旧明文迁移（主人明确不要）。
  - set_token 改为 Result，凭据写入失败时报错给前端（manage.js alert 提示，原值不动）。
  - clippy -D warnings 零警告。待主人实测：添加 token 后状态变"已设置"，重启后轮询/通知正常。

## 本轮任务（2026-11-07）
依赖链（已发给主人确认）：`T1 最大化按钮` 独立；`T2 视图模式 + 板视图 + 拖拽 → T3 列表分类记忆`。
三条共享 index.html / main.js / styles.css，故主人确认走**单 worktree 串行**（worktree/viewmode），逐条 commit。
- [x] **T1 主界面右上角最大化按钮**（完成 2026-11-07 @worktree/viewmode）
  - index.html 标题栏加 `#btn-max`（□），main.js 绑 `getCurrentWindow().toggleMaximize()`，capabilities/default.json 补 `core:window:allow-toggle-maximize`；按主人选择不改 Rust 逻辑。
  - 验收：cargo check + clippy --all-targets -D warnings 零代码警告（stderr 的 os error 5 是共享 incremental 锁的环境警告），node --check 通过，grep 确认三处落地。
  - 已知回归（主人明确「先不动」）：最大化触发 Resized → remember_window 把全屏矩形写进 config.window，下次启动会恢复成没有最大化标志的伪全屏。
  - 待主人目视确认：按钮位置/图标；嵌入（WorkerW 子窗口）模式下 toggleMaximize 的实际表现。
- [x] **T2 设置增加视图模式（列表 / 板视图）+ 板视图拖拽切分类**（完成 2026-11-07 @worktree/viewmode）
  - 设置页加 `#input-view-mode`（列表 / 板），值存 `ghr.settings` 并用 `emit("ghr-view-mode", mode)` **带 payload** 同步主窗口，不赌跨窗口 localStorage 实时同步；主窗口收到后先落盘再切。
  - 板视图在 `#view-list` 内与列表互斥显示，共用搜索栏与 status-hint；tabs / 置顶区 / 批量条在板视图隐藏。4 列 = 4 个状态分类，列头带计数，白卡片沿用 PaperMC 看板的「repo #num / 标题 / 标签 / 元数据行」层次。
  - 横向溢出按需求保留：`#board-columns` 固定 232px 列宽不收缩，`#board-wrap` `overflow-x:auto` 出横向滚动条；两层滚动容器都换成自定义滚动条样式。
  - 拖拽用 HTML5 DnD，drop 复用已有 `set_status` 命令（无需改 Rust）；拖回原列 / 非法列不发请求，靠 `board.js::dropStatus` 判定。
  - 纯逻辑抽 `src/board.js`（groupByStatus / normalizeStatus / dropStatus）配 `tests/board.test.mjs` 5 个单测，fixture 全用通用示例。板视图一次渲染全量（后端上限 200 条），不套列表的触底续展。
  - 验收：`npm test` 26/26 通过、三个 JS `node --check` 通过、cargo check + clippy --all-targets -D warnings 零代码警告、grep 确认落地。
  - 待主人目视确认：看板风格还原度、窗口过小时横向滚动条能否向右滑、拖放高亮与手感、白卡片在半透明底色上的对比。
- [x] **T3 列表视图记忆程序关闭时所在分类，重启直接切回**（完成 2026-11-07 @worktree/viewmode）
  - `pickDefaultStatus()` 优先读 `ghr.settings.lastStatus`，用 `STATUS_KEYS`（5 个合法分类）校验；脏值 / 首次使用才回退原逻辑（有未读→未读，其次处理中，否则全部）。仅启动时定一次，之后不抢用户手上选的 tab。
  - 切 tab 时（仅列表视图）**即时**写 `lastStatus`，不挂关闭钩子：主窗口点关闭其实是隐藏到托盘、窗口不销毁，且强杀进程也该保住最后一次选择。
  - 板视图下 tabs 隐藏天然不写；从板视图切回列表仍停在记忆的分类。
  - 逻辑内联在 main.js（十行级，抽单测收益低），验收走全量回归 + grep 静态核对。
  - 验收：`npm test` 26/26 通过、`node --check` 通过、cargo check + clippy --all-targets -D warnings 零代码警告、grep 确认 6 处落地。
  - 待主人实测：列表视图切到某分类 → 托盘退出/重启 → 是否直接落在该分类。
- [x] **板视图拖拽不生效修复**（完成 2026-11-07 a21e5b2 @master直改）：根因是 Tauri v2 在 Windows 默认接管 WebView2 拖放通道（文件拖入监听），页面内 HTML5 DnD 事件不触发；tauri.conf.json 主窗口加 `"dragDropEnabled": false`（全仓无文件拖放依赖，无副作用）。待主人目视确认拖拽切分类实际手感。
- [x] **记住窗口最大化状态（治「伪全屏」回归）**（完成 2026-11-07 b3aaf0a @master直改）：WindowRect 加 `maximized` 字段（serde default 兼容旧存档）；remember_window 最大化时跳过全屏矩形只翻标记（next_window_rect 纯函数 + 3 单测）；启动先恢复普通矩形再 maximize。cargo test --lib 39/39、clippy 过。待主人目视确认链路：最大化→托盘退出→重启应直接最大化，取消最大化应回到上次普通位置；嵌入模式下最大化表现一并看。

## 里程碑
- [x] M1 骨架：透明窗口 / 引导页 / 设置页 / 0.5s 上浮动画
- [x] 窗口模式：默认贴桌面（WorkerW）+ 置顶开关
- [x] M2 PAT + 仓库级监听（新 Issue / PR / Release）+ 轮询事件流（待主人实测验证）
- [x] M3 单条颗粒度（Issue/PR 的 comment、状态变更）+ 已读状态（待主人实测验证）
- [x] M4 外观完善（系统 Toast 弹窗、开机自启；拉伸为原生 resizable 待实测边缘）（待主人实测验证）
- [x] M5 NSIS 打包 + 托盘图标 + 关闭隐藏到托盘（GitHub Reminder_0.1.0_x64-setup.exe，1.71 MiB，待实测）
- [x] GitHub CI（check+clippy）/ Release（tag 推送自动发布 NSIS）/ Dependabot（cargo/npm/actions 每周）
- [x] docs/HANDOFF.md 交接文档 + .gitattributes + .gitignore 精简
