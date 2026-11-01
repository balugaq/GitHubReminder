# TODO

## 已知待办
- [x] **主窗口位置/尺寸记忆：拉伸/移动后下次启动恢复到上次位置**（完成 2026-11-01 @master工作区直改，待主人实测）
  - 实现：Config 新增 `window` 字段（WindowRect，物理像素 x/y/width/height，serde default 向后兼容）；lib.rs 新增 remember_window——Moved/Resized 事件即更新内存并按 1s 节流落盘，点关闭隐藏前清零节流强制落一次最终位置；setup 启动时读到 window 就 set_size/set_position 恢复（物理像素，覆盖 tauri.conf 的 center）。嵌入模式不受影响：恢复发生在前端 invoke 嵌入之前，SetParent 不改位置。clippy -D warnings 零警告，7 个单测通过。
- [x] **主界面动画只在动态真有变更时触发**（完成 2026-10-31 @master工作区直改，未提交待验收）
  - 根因：showView 每次重放 fade-up，而它被 syncFromRust 调用（窗口每次聚焦/配置变更都触发）→ 无变更也闪动画。修复：删掉 main.js showView 里的重放逻辑，动画只在视图首次显示和新事件卡插入时播。
- [x] **评论通知反复提醒修复 + 评论提醒文案改"有新消息" + 同一 issue/PR 未读期间不重复提醒**（完成 2026-10-31 @master工作区直改——主人有未提交 WIP，worktree 基线落后故未走 worktree，改动未提交待主人验收）
  - 根因：`/issues/{n}/comments` 端点 `direction=desc` 不生效（固定升序返回），`comments.first()` 拿到最旧评论当游标 → 游标每轮回退 → 新于游标的评论每轮重报。修复：游标一律取全量最大 id，过滤只认 id（github.rs poll_item）。
  - 附加：① do_poll 加 AtomicBool 互斥防并发轮询双报（lib.rs）；② toast 中 comment 事件显示"有新消息"（toast_summary）；③ 后端 notified 集合去重——同一 issue/PR 弹过 toast 且未读（点事件卡即已读，mark_read 命令）则不再弹（lib.rs + main.js）。
- [x] **Win+D 仍会隐藏贴桌面的窗口**（完成 2026-10-31 @worktree/wind-restore，已合并 master 55278f3，待主人实测 Win+D 效果）。
  - 方案：定时检测自动唤回——嵌入模式下守护协程每 2 秒查可见性/最小化，被 Win+D 藏了就 SW_RESTORE。托盘开关、点关闭先置 USER_HIDDEN 标记，守护协程不抢窗口。
  - 合并冲突两处均为假冲突（各加各的），取并集解决，clippy -D warnings 零警告。候选方案：定时检测可见性自动唤回 / 防隐藏钩子 / 忽略。优先级低，主人说不重要。
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
  - 待主人实测：填好用户名后，QQ 推送应形如「[@me] [ytdd9527] · 标题」；系统 Toast 文案未改（仍为精简式），如需同步可再提。


- [ ] **QQ 推送文案改为多行卡片式**（参照主人 2026-11-01 提供截图的下面那种样式）：
  ```
  [GitHubReminder] 通知 · GitHub 动态
  PR #459 · ytd9527/NetworksExpansion
  fix: 修改元件转运台逻辑
  https://github.com/ytd9527/NetworksExpansion/pull/459
  ```
  即：第一行应用名+通知类别，第二行类型+编号+仓库，第三行标题，第四行链接（issue/pr/release 各自拼对应 URL）。当前是「#459 fix: 标题 ...以及另外 N 条新动态」的聚合一行式，要改。
- [ ] **拉取可靠性：不依赖时间戳判断新事件**——GitHub 拉取有时会失败，改为自添加仓库后比对「最新几条是否已在记录里」来判定增量（类似游标兜底：每轮取最新 N 条，逐条查 events 已存则跳过，避免基于 updated_at/时间窗漏报）。与现有 cursor 机制的关系需梳理，避免双重判定打架。
- [ ] **添加仓库时弹出 Issue/PR 选择弹窗**：输入仓库后弹「GitHub Issue/PR 界面」弹窗，列出该仓库的 issues/prs 供勾选，勾中的自动附带为该 repo 下的单条监听（复用现有单条颗粒度 Watch），免去手动输编号。依赖 GitHub 列表 API（分页、开/闭状态筛选）。
- [x] **Explorer 重启后嵌入窗口的父级失效处理**（无需求，2026-11-01 主人确认不做）。
- [ ] **主界面批量选择通知并标记状态**：通知卡片支持多选，批量标记状态（未读/已读/处理中/已完成）；标记完成后主界面底部向上弹出一个 toast，显示「已完成编辑」+ 撤销按钮，点撤销恢复到多选前的状态快照。
- [x] **嵌入模式下键盘输入验证**（完成 2026-10-31，主人实测 PAT 输入框在 WorkerW 子窗口中可正常打字）。
- [x] **PAT 明文存储 → 迁移到 Windows 凭据管理器加密**（完成 2026-10-31 @master工作区直改，未提交待验收）  - 实现：Config.token 改 `#[serde(skip)]`（内存持有，config.json 不再落明文）；store.rs 新增 save_token/load_token 走 keyring 3（windows-native 凭据管理器，服务名 GitHubReminder/账户 pat）。
  - 迁移：启动时检测 config.json 里的旧明文 token → 写入凭据管理器并打印日志；写失败则本次会话仅内存保留（不回写磁盘）。凭据管理器读不到时回退会话内存值。
  - 修正（2026-10-31 二次）：初版 `#[serde(skip)]` 会把 token 从 get_config/set_token 的 API 响应里一并删掉，前端永远显示"未设置"（表现为无法添加 token）。改为 token 正常序列化、仅 store::save 落盘前剔除；启动时 cfg.token = load_token()（凭据管理器为唯一来源），不做旧明文迁移（主人明确不要）。
  - set_token 改为 Result，凭据写入失败时报错给前端（manage.js alert 提示，原值不动）。
  - clippy -D warnings 零警告。待主人实测：添加 token 后状态变"已设置"，重启后轮询/通知正常。

## 里程碑
- [x] M1 骨架：透明窗口 / 引导页 / 设置页 / 0.5s 上浮动画
- [x] 窗口模式：默认贴桌面（WorkerW）+ 置顶开关
- [x] M2 PAT + 仓库级监听（新 Issue / PR / Release）+ 轮询事件流（待主人实测验证）
- [x] M3 单条颗粒度（Issue/PR 的 comment、状态变更）+ 已读状态（待主人实测验证）
- [x] M4 外观完善（系统 Toast 弹窗、开机自启；拉伸为原生 resizable 待实测边缘）（待主人实测验证）
- [x] M5 NSIS 打包 + 托盘图标 + 关闭隐藏到托盘（GitHub Reminder_0.1.0_x64-setup.exe，1.71 MiB，待实测）
- [x] GitHub CI（check+clippy）/ Release（tag 推送自动发布 NSIS）/ Dependabot（cargo/npm/actions 每周）
- [x] docs/HANDOFF.md 交接文档 + .gitattributes + .gitignore 精简
