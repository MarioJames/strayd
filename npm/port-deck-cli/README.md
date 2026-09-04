# port-deck-cli

一个用于管理 Windows 与 WSL 开发服务、临时公网隧道和 SSH 服务的 Rust TUI。

> `port-deck-cli` 是临时包名，正式命名前不会发布到 npm registry。

## 安装

```bash
npm install --global ./port-deck-cli-0.1.0.tgz
port-deck
```

当前包支持 Windows x64 与 WSL/Linux x64。直接运行会进入 TUI，也可以用于脚本：

```bash
port-deck list
port-deck list --kind tunnel --json
port-deck stop tunnel --port 5000 --dry-run
port-deck stop tunnel --port 5000 --yes
port-deck stop group --port 5000 --yes
port-deck stop dev --project storefront --all --yes
```

停止命令支持 `--id`、`--port/-p`、`--project`、`--origin`、`--distro` 和 `--runtime` 筛选。匹配多项时必须显式使用 `--all`；省略 `--yes` 时要求在交互终端输入确认。

TUI 快捷键：`Tab` 或 `1-4` 切换页面，`j/k` 或方向键选择，`d` 关闭开发服务，`t` 关闭隧道，`x` 关闭整个关联组，`r` 刷新，`q` 退出。所有停止操作都有二次确认，包含 `sshd` 的组不允许整组关闭。
