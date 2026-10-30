# HANDOFF — GitHubReminder 交接文档

> 贴在桌面上的 GitHub 通知小窗。Tauri 2 + Rust + 原生 JS/CSS。

## 是什么

480×320（可拉伸）半透明通知框，通过 GitHub REST API 轮询监听：

- **仓库级**：新 Issue / 新 PR / 新 Release（可勾选）
- **单条颗粒度**：指定 Issue/PR 的新评论、状态开关变更
- 新事件以 0.5s 自下而上动画浮现于窗口，可选弹系统 Toast
- 窗口模式：默认**贴桌面**（WorkerW 嵌入，图标之上、普通窗口之下），可切换置顶
- 关闭按钮 = 隐藏到托盘；托盘左键切换显示，菜单可退出

## 构建 & 运行

```bash
npm install
npm run tauri dev    # 开发（前端热重载，Rust 改动自动重编）
npm run tauri build  # 产出 NSIS 安装包 → src-tauri/target/release/bundle/nsis/
```

前置依赖：Rust (MSVC)、Node 22、WebView2（Win10/11 自带）、NSIS（tauri 自动下载）。

### 调试开关（环境变量）

| 变量 | 作用 |
|---|---|
| `GHR_NO_EMBED=1` | 跳过 WorkerW 嵌入，窗口保持普通模式（调布局用） |

## 代码地图

```
src/                  前端（无框架，原生 JS/CSS）
  index.html          三视图：引导页 / 列表页(事件流+监听管理) / 设置页 / 添加监听页
  main.js             视图切换、fadeUp 动画重放、事件渲染、已读标记、设置
  styles.css          半透明色框、卡片、动画
src-tauri/src/
  lib.rs              入口：AppState、60s 轮询协程、全部 tauri 命令、托盘、关闭隐藏
  store.rs            Config/Watch 数据模型 + config.json 读写
  github.rs           GitHub API 客户端与轮询逻辑（仓库级 + 单条颗粒度）
  winembed.rs         WorkerW 嵌入 / 解除嵌入（Progman 0x052C + SetParent）
.github/workflows/    ci.yml（check+clippy）、release.yml（tag 推送 → NSIS 发布）
.github/dependabot.yml  cargo/npm/actions 每周更新
todo.md               待办与里程碑进度
```

## 数据位置

- 配置：`%APPDATA%\com.githubreminder.app\config.json`（PAT、代理、notify、watches、四个游标表）
- 已读事件：WebView localStorage `ghr.read`（最多 500 键）
- 外观（颜色/透明度/置顶）：localStorage `ghr.settings`

## 游标与首波静默约定

每个监听项按类型记游标，**首次轮询只记游标不报存量事件**：

| 监听类型 | 游标 |
|---|---|
| 仓库 Issue/PR | 最后一条 `created_at`（cursor_issues） |
| Release | 最后 release id（cursor_release） |
| 单条评论 | 最后评论 id（cursor_comment） |
| 单条状态 | 最后 state（cursor_state） |

## 关键决策与坑（重要，别踩第二遍）

1. **国内网络链路（血泪史，三层叠加）**：
   - 本机直连 `api.github.com` 不通 → reqwest 需走代理：已开 `system-proxy` 特性 + 设置页可手填代理；
   - DNS 返回 IPv6 但 v6 路由不通 → `resolve_to_addrs` 强制 IPv4（Node undici 有 Happy Eyeballs 而 reqwest 没有，这是 MyWorkspace 能通而本项目最初不能通的原因）；
   - 本机用 Watt Toolkit(Steam++) 加速 GitHub → MITM 自签证书（issuer=`CN=SteamTools Certificate, O=BeyondDimension`）→ TLS 后端必须用 **native-tls**（走 Windows 系统证书库）而非 rustls-tls（内置 Mozilla 根，不认它）。MyWorkspace 靠 `NODE_EXTRA_CA_CERTS` 解决同类问题。
2. **WorkerW 嵌入**：保持 `WS_POPUP` + `WS_EX_TOOLWINDOW`；嵌入后窗口在所有普通窗口之下，最大化任何应用都会盖住它——这是该模式的物理规律，所以默认贴桌面但保留置顶开关。
3. **cargo build lock**：dev 会话与手动 cargo 并行会撞锁（os error 5），编译前确认 dev 空闲或等待重试。
4. **crates.io 直连极慢**：`~/.cargo/config.toml` 已配 rsproxy 镜像。

## 发布流程

推 tag 即发：`git tag v0.1.0 && git push origin v0.1.0` → release.yml 用 tauri-action 构建 NSIS 并挂到 GitHub Release。CI 在 push/PR 时跑 cargo check + clippy。

## 已知未解（详见 todo.md）

- Win+D 仍会隐藏贴桌面模式的窗口
- Explorer 重启后嵌入失效需手动切模式（未做自动重嵌）
- PAT 明文存 config.json（待迁移 Windows 凭据管理器）
- 嵌入模式下键盘输入待充分验证（PAT 输入框路径）
