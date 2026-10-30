# TODO

## 已知待办
- [ ] **Win+D 仍会隐藏贴桌面的窗口**（2026-10-30 主人实测）。候选方案：定时检测可见性自动唤回 / 防隐藏钩子 / 忽略。优先级低，主人说不重要。
- [ ] Explorer 重启后嵌入窗口的父级失效处理（检测父窗口销毁 → 自动重嵌）。
- [ ] 嵌入模式下键盘输入验证：PAT 输入框在 WorkerW 子窗口里能否正常打字（概念对话里的已知坑，待主人实测）。
- [ ] PAT 明文存储 → 迁移到 Windows 凭据管理器加密（当前存 config.json）。

## 里程碑
- [x] M1 骨架：透明窗口 / 引导页 / 设置页 / 0.5s 上浮动画
- [x] 窗口模式：默认贴桌面（WorkerW）+ 置顶开关
- [x] M2 PAT + 仓库级监听（新 Issue / PR / Release）+ 轮询事件流（待主人实测验证）
- [x] M3 单条颗粒度（Issue/PR 的 comment、状态变更）+ 已读状态（待主人实测验证）
- [x] M4 外观完善（系统 Toast 弹窗、开机自启；拉伸为原生 resizable 待实测边缘）（待主人实测验证）
- [ ] M5 打包 NSIS + 托盘图标
- [x] M5 NSIS 打包 + 托盘图标 + 关闭隐藏到托盘（GitHub Reminder_0.1.0_x64-setup.exe，1.71 MiB，待实测）
- [x] GitHub CI（check+clippy）/ Release（tag 推送自动发布 NSIS）/ Dependabot（cargo/npm/actions 每周）
- [x] docs/HANDOFF.md 交接文档 + .gitattributes + .gitignore 精简
