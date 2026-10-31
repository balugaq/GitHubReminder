# AGENTS.md — AI 协作守则（GitHubReminder 项目）

本文件是 AI 助手在本仓库工作的硬性规则。违反任何一条都视为无效工作。

## 1. Todo 工作流（核心规则）

### 1.1 依赖链前置声明
- 每次开始一个 todo 前，必须先明确标注它依赖哪些其他 todo（依赖链：`A → B → C` 表示 C 依赖 B，B 依赖 A）。
- 依赖链要**发给主人确认**（对话内或 QQ 推送），之后才能动手。
- 有新 todo 加入时，若它改变了已有依赖链，需重新输出整条链。

### 1.2 依赖链跳跃检查（硬闸门）
- 开始 todo 前，逐个检查其前置 todo 的状态（todo.md 中的标记）。
- **任何前置未完成 → 不予执行**，并提醒主人：「todo X 依赖的 todo Y 尚未完成，已跳过，请先处理 Y 或确认跳过依赖」。
- 主人明确说「跳过依赖，继续做」才允许例外，且此例外要记录到当天日志。

### 1.3 认领标记（防重复劳动）
- 开始执行一个 todo 后，**第一时间**在 `todo.md` 中将其标注为「正在执行」并注明 worktree 分支名，例如：
  ```
  - [~] M6 托盘图标优化（执行中 @worktree/m6-tray, 2026-10-31）
  ```
- 标记落盘之前，不得开始写任何代码。
- 完成后改为 `[x]`；中途放弃改回 `[ ]` 并注明原因。
- 目的：多个并行任务（可能来自不同会话）通过 todo.md 互斥认领，避免重复完成相同内容。

## 2. Git Worktree 并行规范

### 2.1 Worktree 创建
- 每个 todo 在独立 worktree 中完成：
  ```bash
  git worktree add ../GitHubReminder-wt-<todo名> -b worktree/<todo名>
  ```
- worktree 命名：`GitHubReminder-wt-<短名>`，分支命名：`worktree/<短名>`。
- 一个 worktree 只做一个 todo，不混装。

### 2.2 合并规则（重要边界）
- worktree 内**允许正常 commit**（分支上的提交是工作过程记录）。
- 验收通过后，将分支合并到本地 `master`：
  ```bash
  git checkout master && git merge worktree/<短名>
  ```
- 合并后删除分支与 worktree：
  ```bash
  git branch -d worktree/<短名> && git worktree remove ../GitHubReminder-wt-<短名>
  ```
- **红线**：不 push、不碰远程、不 rebase / reset / 改写已有历史。提交与推送的最终决定权归主人。

### 2.3 冲突处理
- 合并前先 `git merge --no-commit --no-ff` 干跑检查冲突（或直接观察 merge 输出）。
- 有冲突：停下报告主人哪些文件冲突、双方改动是什么，**不擅自仲裁**。

## 3. 验收标准

worktree 中的工作要合并进 master，必须满足：
1. `cargo check` + `cargo clippy`（-D warnings）零警告；
2. 前端改动经静态核对落地（grep 确认），视觉类标注「待主人目视确认」；
3. todo.md 状态已同步更新；
4. 若涉及新依赖：说明加了什么、为什么。
5. **AI 禁止自行启动浏览器/渲染引擎做视觉验证**（主人目视验收，见用户级守则）。

## 4. 已知项目约束（速查）

- Rust 工具链不在 Git Bash PATH：`export PATH="$(cygpath -u "$USERPROFILE")/.cargo/bin:$PATH"`。
- 主人 `tauri dev` 运行中时 cargo 会撞编译锁（os error 5），编译前确认或轮询等待。
- 本机访问 GitHub API 依赖 Watt Toolkit MITM + native-tls + 强制 IPv4，勿动这三层。
- 数据文件在 `%APPDATA%\com.githubreminder.app\config.json`，含 token，不外传、不打印内容。
