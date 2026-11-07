# HANDOFF — GitHubReminder 交接文档

> 贴在桌面上的 GitHub 通知小窗。Tauri 2 + Rust + 原生 JS/CSS。

## 是什么

默认 480×320（可拉伸，下限 240×240）半透明通知框，通过 GitHub REST API 轮询监听：

- **仓库级**：新 Issue / 新 PR / 新 Release / 关闭事件（可勾选）
- **单条颗粒度**：指定 Issue/PR 的新评论、状态开关变更
- 事件按 `repo+number` 聚合成一张卡，状态分 未读 / 已读 / 处理中 / 已解决；最多钉住 3 张
- 主界面：分类 tab（带状态图标）、搜索框（标题 / 作者，`@用户名` 限定「涉及的人」，旧写法 `@//用户名/` 兼容）、默认 100 条触底续展、统计页
  - `@用户名` 必须是词首独立 token（前后空白/首尾），所以邮箱里的 `@` 不会误判；可放在关键词任意位置，多个人名是 AND
  - 「涉及的人」= 该卡的作者 + 指派人（后端 `Entity.people` 累积），**按设计不含评论者**；普通关键词只匹配标题 + 作者，两者语义刻意区分
- 新事件可选弹系统 Toast / 推 QQ 私信（多行卡片式，超长按行分段）
- 自动归类规则：dependabot 发起的 PR 在 close / merge 后归入「已解决」（按批次生效，不覆盖手动改过的状态）
- 窗口模式：默认**贴桌面**（壁纸层 WorkerW 嵌入，图标之上、普通窗口之下），可切换置顶
- Win+D 后由守护协程唤回；关闭按钮 = 隐藏到托盘；托盘左键切换显示，菜单可退出

## 构建 & 运行

```bash
npm install
npm test                  # node --test，搜索逻辑单测
npm run tauri dev         # 开发（前端热重载，Rust 改动自动重编）
npm run tauri build       # 产出 NSIS 安装包 → src-tauri/target/release/bundle/nsis/
```

前置依赖：Rust (MSVC)、Node 22、WebView2（Win10/11 自带）、NSIS（tauri 自动下载）。

### 调试开关（环境变量）

| 变量 | 作用 |
|---|---|
| `GHR_NO_EMBED=1` | 跳过桌面层嵌入，窗口保持普通顶层模式（调布局 / 排查交互用） |
| `GHR_WALLPAPER_LAYER=1` | 强制挂壁纸层 WorkerW（默认即此层，用于覆盖下面的实验开关） |
| `GHR_DESKTOP_HOST=1` | 实验：改挂 SHELLDLL_DefView 宿主（Progman），Win+D 严格免疫，但桌面窗口过程会吞掉子窗口鼠标输入，**勿日常使用** |

## 代码地图

```
src/
  index.html          主窗口三视图：引导页 / 事件流列表页 / 统计页
  main.js             主窗口交互：渲染、分类、搜索、置顶、外观、窗口模式
  search.js           搜索纯逻辑（classic script 挂 window.GhrSearch，Node vm 单测复用同一份）
  styles.css          主窗口样式：半透明色框、卡片、动画、跟随自定义色的滚动条
  manage.html/js/css  独立管理窗口：监听增删改、设置、Issue/PR 浏览勾选弹窗
src-tauri/src/
  main.rs             二进制入口
  lib.rs              AppState、轮询协程、全部 tauri 命令、托盘、Win+D 守护、窗口位置记忆
  store.rs            Config/Watch/Entity 数据模型 + config.json 读写 + 凭据管理器（PAT / QQ AppSecret）
  github.rs           GitHub API 客户端与轮询（仓库级 + 单条颗粒度 + 实体查重兜底）
  winembed.rs         桌面层嵌入 / 解除嵌入 / 守护唤回
  qqrelay.rs          内置 QQ 中转服务
tests/search.test.mjs   node --test 搜索逻辑单测
.github/workflows/     ci.yml（check+clippy）、release.yml（tag 推送 → NSIS 发布）
.github/dependabot.yml cargo/npm/actions 每周更新
todo.md                待办与里程碑进度
```

## 数据位置

- 配置：`%APPDATA%\com.githubreminder.app\config.json` —— 代理、通知开关、轮询间隔、监听项、五张游标表、通知实体（上限 200）、统计、窗口矩形
- **密钥不在 config.json**：GitHub PAT 与 QQ 机器人 AppSecret 都走 Windows 凭据管理器（服务名 `GitHubReminder`，账户分别 `pat` / `qq_app_secret`），`store::save` 落盘前剔除两项
- 旧版 config.json 里残留的 AppSecret 明文在启动时一次性迁入凭据管理器并剔除；写失败时本次仅内存保留、不删明文
- 设置页不回显密钥，只按是否已存显示占位提示；输入框留空 = 不改已存密钥，填新值才覆盖
- 通知状态 / 钉住：后端 `entities[].status` / `pinned`（旧的 localStorage `ghr.read` 已废弃）
- 外观（颜色 / 透明度 / 置顶）：localStorage `ghr.settings`

## 游标与首波静默约定

每个监听项按类型记游标，**首次轮询只记游标不报存量事件**：

| 监听类型 | 游标 |
|---|---|
| 仓库 Issue/PR | 最后一条 `created_at`（cursor_issues） |
| Release | 最后 release id（cursor_release） |
| 单条评论 | 最后评论 id（cursor_comment） |
| 单条状态 | 最后 state（cursor_state） |
| 仓库关闭事件 | 最后 `closed_at`（cursor_closed） |

拉取可靠性另有一层**实体查重兜底**：报新事件前查实体表（`repo+number` 已存在、或 state 已是 closed 就跳过）。时间戳游标仍是主判定；实体被 200 条上限裁剪后自动退回纯游标，避免双重判定打架。

## 关键决策与坑（重要，别踩第二遍）

1. **国内网络链路（血泪史，三层叠加）**：
   - 本机直连 `api.github.com` 不通 → reqwest 需走代理：已开 `system-proxy` 特性 + 设置页可手填代理；
   - DNS 返回 IPv6 但 v6 路由不通 → `resolve_to_addrs` 强制 IPv4（Node undici 有 Happy Eyeballs 而 reqwest 没有，这是 MyWorkspace 能通而本项目最初不能通的原因）；
   - 本机用 Watt Toolkit(Steam++) 加速 GitHub → MITM 自签证书（issuer=`CN=SteamTools Certificate, O=BeyondDimension`）→ TLS 后端必须用 **native-tls**（走 Windows 系统证书库）而非 rustls-tls（内置 Mozilla 根，不认它）。MyWorkspace 靠 `NODE_EXTRA_CA_CERTS` 解决同类问题。
2. **桌面层嵌入**：保持 `WS_POPUP` + `WS_EX_TOOLWINDOW`；嵌入后窗口在所有普通窗口之下，最大化任何应用都会盖住它——这是该模式的物理规律，所以默认贴桌面但保留置顶开关。默认宿主是**壁纸层 WorkerW**；`SHELLDLL_DefView` 的宿主（Progman）虽然 Win+D 严格免疫，但桌面窗口过程会吞掉子窗口的鼠标消息，表现为「窗口看得见却拖不动、点不了」。
3. **Win+D 守护**：不能只靠 `IsWindowVisible` 判断——Win+D 未必改 `WS_VISIBLE`，也可能只是把桌面压到窗口前面。守护 200ms 体检唤回 + 每 1s 无条件把窗口提回桌面层 Z 序顶部。反过来，高频 `ShowWindow` / `SetWindowPos` 会打断鼠标按下/抬起配对，所以鼠标按下期间必须跳过，唤不回来就指数退避到 2s。
4. **用户主动隐藏标记**：托盘隐藏、点关闭、点最小化都置 `USER_HIDDEN`，守护见到就放手；窗口重新可见且未最小化时自动复位，否则守护会和主人抢窗口。托盘切换要用 `is_visible() && !is_minimized()` 判断，最小化的窗口 `is_visible` 仍是 true。
5. **cargo build lock**：dev 会话与手动 cargo 并行会撞锁（os error 5），编译前确认 dev 空闲或等待重试。
6. **crates.io 直连极慢**：`~/.cargo/config.toml` 已配 rsproxy 镜像。

## 发布流程

推 tag 即发：`git tag v0.1.0 && git push origin v0.1.0` → release.yml 用 tauri-action 构建 NSIS 并挂到 GitHub Release。CI 在 push/PR 时跑 cargo check + clippy。

## 已知未解（详见 todo.md）

- Win+D 守护与贴桌面后的鼠标交互已于 2026-11-07 由主人实测确认修复（宿主 WorkerW + 200ms guard_tick + 1s raise_z）；异常时日志仍会打印 `[winembed] 守护唤回: visible=… iconic=… embedded=… user_hidden=…`，据此可一次定位卡在哪一环
- 置顶模式（未嵌入）的免疫与「手动最小化」边界靠 `USER_HIDDEN` 标记区分，同样待实测
- 旧存档里的历史卡片缺 author / people，需等该卡片有新事件才补全（「涉及的人」不含评论者是设计约定，不是缺陷）
- 主界面批量选择通知 + 标记状态 + 撤销 toast（todo 未实现）
