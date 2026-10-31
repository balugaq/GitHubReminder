# TODO

## 已知待办
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
- [ ] Explorer 重启后嵌入窗口的父级失效处理（检测父窗口销毁 → 自动重嵌）。
- [x] **嵌入模式下键盘输入验证**（完成 2026-10-31，主人实测 PAT 输入框在 WorkerW 子窗口中可正常打字）。
- [x] **PAT 明文存储 → 迁移到 Windows 凭据管理器加密**（完成 2026-10-31 @master工作区直改，未提交待验收）
  - 实现：Config.token 改 `#[serde(skip)]`（内存持有，config.json 不再落明文）；store.rs 新增 save_token/load_token 走 keyring 3（windows-native 凭据管理器，服务名 GitHubReminder/账户 pat）。
  - 迁移：启动时检测 config.json 里的旧明文 token → 写入凭据管理器并打印日志；写失败则本次会话仅内存保留（不回写磁盘）。凭据管理器读不到时回退会话内存值。
  - set_token 改为 Result，凭据写入失败时报错给前端（manage.js alert 提示，原值不动）。
  - clippy -D warnings 零警告。待主人实测：重启后通知/轮询正常（token 从凭据管理器恢复）。

## 里程碑
- [x] M1 骨架：透明窗口 / 引导页 / 设置页 / 0.5s 上浮动画
- [x] 窗口模式：默认贴桌面（WorkerW）+ 置顶开关
- [x] M2 PAT + 仓库级监听（新 Issue / PR / Release）+ 轮询事件流（待主人实测验证）
- [x] M3 单条颗粒度（Issue/PR 的 comment、状态变更）+ 已读状态（待主人实测验证）
- [x] M4 外观完善（系统 Toast 弹窗、开机自启；拉伸为原生 resizable 待实测边缘）（待主人实测验证）
- [x] M5 NSIS 打包 + 托盘图标 + 关闭隐藏到托盘（GitHub Reminder_0.1.0_x64-setup.exe，1.71 MiB，待实测）
- [x] GitHub CI（check+clippy）/ Release（tag 推送自动发布 NSIS）/ Dependabot（cargo/npm/actions 每周）
- [x] docs/HANDOFF.md 交接文档 + .gitattributes + .gitignore 精简
