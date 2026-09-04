# strayd

跨平台本地开发服务与临时公网隧道管理工具，使用 Rust 实现，通过 npm 分发 TUI 与脚本化命令。

## 安装

```bash
npm install --global strayd
strayd
```

支持 Windows、Linux（包括 WSL）与 macOS 的 x64/arm64。安装包会按 Node.js 的 `process.platform` 和 `process.arch` 自动运行对应原生程序，不要求用户安装 Rust。

```bash
strayd
strayd list --kind tunnel
strayd list --platform macos --json
strayd stop tunnel --port 5000 --dry-run
strayd stop group --port 5000 --yes
strayd stop dev --project storefront --all --yes
```

停止命令支持 `--id`、`--port/-p`、`--project`、`--platform` 和 `--runtime` 筛选。匹配多项时必须显式使用 `--all`；省略 `--yes` 时要求在交互终端输入确认。

TUI 快捷键：`Tab` 或 `1-4` 切换页面，`j/k` 或方向键选择，`d` 关闭开发服务，`t` 关闭隧道，`x` 关闭整个关联组，`r` 刷新，`q` 退出。所有停止操作都有二次确认，包含 `sshd` 的组不允许整组关闭。
