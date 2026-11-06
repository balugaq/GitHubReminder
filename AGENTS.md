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
  # 创建后必须补两件事：
  cp .cargo/config.toml ../GitHubReminder-wt-<todo名>/.cargo/config.toml   # 共享 cargo target 目录
  cd ../GitHubReminder-wt-<todo名> && npm install                          # node_modules 各自独立（仅 16M，不搞共享）
  ```
- worktree 命名：`GitHubReminder-wt-<短名>`，分支命名：`worktree/<短名>`。
- 一个 worktree 只做一个 todo，不混装。
- `.cargo/config.toml` 把 target-dir 指向 `D:/github_clone/.cargo-target-GitHubReminder`（绝对路径，相对路径在各 worktree 下会解析到不同目录）。若该文件尚未提交进 master，则每个新 worktree 都要手动复制一份；主人提交后新分支自动携带，可省这一步。

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
- **红线**：不 push、不碰远程、不 rebase / reset / 改写已有历史。push 与历史改写的决定权归主人；任务级 commit 由 AI 及时完成（见 §5）。

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
- **cargo 构建目录全局共享**（`D:/github_clone/.cargo-target-GitHubReminder`，由 `.cargo/config.toml` 指定）：编译会互斥排队，同时只能有一个 worktree 在编；这是为换取零重复编译有意接受的代价。多 worktree 并发跑 cargo 时后到的会等待，不是卡死。
- 主人 `tauri dev` 运行中时 cargo 会撞编译锁（os error 5），编译前确认或轮询等待。
- 删除大体积构建产物（target 等）时，Windows 安全删除钩子对大目录整树删除会失败（trash 失败即拒绝），需先清零文件内容再删，或交主人手动 `rmdir /s /q`。
- 本机访问 GitHub API 依赖 Watt Toolkit MITM + native-tls + 强制 IPv4，勿动这三层。
- 数据文件在 `%APPDATA%\com.githubreminder.app\config.json`，含 token，不外传、不打印内容。

## 5. 提交规范：及时 commit + 提交前敏感信息检查

### 5.1 及时提交
- 一个任务做完并通过验收后**立即 commit**，不积压到下一个任务、不等主人来问。todo.md 状态同步与验收本身算任务的一部分，一起提交。
- 原则上「一个任务一个 commit」。确实拆不开（同一文件被多轮任务改动、无法用文件级边界区分）时才合并为一个 commit，但 message 要概括每项改动，并在汇报里说明合并原因。
- 提交前用 `git status --short` + `git diff --stat` 确认范围，不夹带无关文件、不夹带构建产物与本地配置。
- **红线不变**：不 push、不碰远程、不 rebase / reset / 改写已有历史。本节覆盖 §2.2 里「提交决定权归主人」的旧表述：任务级 commit 由 AI 主动完成，主人保留的是 push 与历史改写的决定权。
- 验收不通过、编译/静态核对有未解错误时**不许提交**，只汇报阻塞原因。

### 5.2 每次 commit 前的强制检查（硬闸门）
检查对象是**本次暂存的全部内容**（`git diff --cached`，含新增文件和删除行的上下文），不是只看改动摘要。

1. **凭据类**：GitHub PAT / `ghp_` / `github_pat_`、token、client_secret、cookie、Authorization / Bearer 头、QQ 机器人 webhook 与中转密钥、私钥、`%APPDATA%\...\config.json` 的实际内容。
2. **真实身份 / 业务数据**：真实 GitHub 用户名、真实仓库名与组织名、真实 issue / PR 标题与编号、真实 QQ 号 / 邮箱 / 手机号、真实统计数字、真实日志或截图片段。
3. **模板与示例数据（重点）**：单测 fixture、注释里的示例、文档模板、默认配置、推送文案样例，一律使用**常规通用示例**——`owner/repo`、`octocat/hello-world`、`example.com`、`alice` / `bob`、`#123`、`示例标题`。凡是看起来像真实账号、真实仓库、真实提交标题的，先替换成通用值再提交。历史提交里遗留的也要顺手改掉。
4. **本机环境**：`C:\Users\<真实用户名>`、本机绝对路径、内网地址、调试用环境变量的真实取值。

执行方式：
- 逐文件读一遍暂存 diff，再用 grep 扫上述关键词（中英文都扫）。
- 命中后判断是不是通用示例；不是就替换成通用值，**替换完重新检查一遍**才能提交。
- 汇报里必须写清检查结果：「已检查 N 个文件，无敏感信息」或「已把 X 替换为通用示例」。
- 拿不准是否敏感（例如来源不明的长串 token 样式字符串）→ **停下来问主人，不猜、不先提交**。
- 检查只读取内容，绝不把 `%APPDATA%` 下的配置内容打印到对话、日志或仓库里。
