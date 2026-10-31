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
- [~] **仓库级监听配置增强：关 Issue / 关 PR 事件 + 默认勾选调整**（执行中 @master工作区直改，2026-10-31，主人指定不走 worktree）
  - 需求：① 仓库级监听增加「关 Issue」「关 PR」两类事件；② 新增监听的默认勾选：关 Issue、关 PR、Release 均默认勾选。
- [ ] Explorer 重启后嵌入窗口的父级失效处理（检测父窗口销毁 → 自动重嵌）。
- [ ] 嵌入模式下键盘输入验证：PAT 输入框在 WorkerW 子窗口里能否正常打字（概念对话里的已知坑，待主人实测）。
- [ ] PAT 明文存储 → 迁移到 Windows 凭据管理器加密（当前存 config.json）。

## 里程碑
- [x] M1 骨架：透明窗口 / 引导页 / 设置页 / 0.5s 上浮动画
- [x] 窗口模式：默认贴桌面（WorkerW）+ 置顶开关
- [x] M2 PAT + 仓库级监听（新 Issue / PR / Release）+ 轮询事件流（待主人实测验证）
- [x] M3 单条颗粒度（Issue/PR 的 comment、状态变更）+ 已读状态（待主人实测验证）
- [x] M4 外观完善（系统 Toast 弹窗、开机自启；拉伸为原生 resizable 待实测边缘）（待主人实测验证）
- [x] M5 NSIS 打包 + 托盘图标 + 关闭隐藏到托盘（GitHub Reminder_0.1.0_x64-setup.exe，1.71 MiB，待实测）
- [x] GitHub CI（check+clippy）/ Release（tag 推送自动发布 NSIS）/ Dependabot（cargo/npm/actions 每周）
- [x] docs/HANDOFF.md 交接文档 + .gitattributes + .gitignore 精简
