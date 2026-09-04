# Strayd

Strayd 是一个通过 npm 分发的跨平台 Rust TUI/CLI，用来发现和管理本地开发服务、临时公网隧道与 SSH 服务。

## 安装与使用

正式包发布后可直接安装，用户不需要 Rust：

```bash
npm install --global strayd
strayd
```

无子命令时进入 TUI，也可用于脚本：

```bash
strayd list
strayd list --kind tunnel --json
strayd list --platform linux --port 5000
strayd stop tunnel --port 5000 --dry-run
strayd stop tunnel --port 5000 --yes
strayd stop group --port 5000 --yes
strayd stop dev --project storefront --all --yes
```

`list` 与 `stop` 支持 `--id`、`--port/-p`、`--project`、`--platform windows|linux|macos` 和 `--runtime`。停止命令匹配多项时必须显式传 `--all`；实际关闭前需要交互确认或传 `--yes`，`--dry-run` 只打印计划。

TUI 使用 `Tab` 或 `1-4` 切页，`j/k` 选择，`d` 关闭开发服务，`t` 关闭隧道，`x` 关闭整个关联组，`r` 刷新，`q` 退出。

## 平台行为

- Linux 与 WSL：都按 Linux 宿主直接读取端口和进程，不调用 `wsl.exe`。systemd 托管的服务会尝试停止对应 unit。
- Windows：使用 Windows 原生端口/进程接口扫描，并通过 `taskkill /T /F` 结束进程树。
- macOS：使用 macOS 原生端口/进程接口扫描，通过 `TERM` 后再 `KILL` 的方式结束进程树。

端口枚举基于 `netstat2` 的系统 API，进程枚举与身份校验基于 `sysinfo`。Strayd 会识别常见前端运行时以及 cloudflared、ngrok、SSH 反向转发、frpc、localtunnel 和 bore；隧道命令中的本地目标会与对应监听端口组成同一资源组。`sshd` 会展示，但始终受保护，不能被停止。

部分系统进程的端口或命令行可能受操作系统权限限制；此时工具只展示当前用户有权读取的信息，不会触发提权提示。

## 本地开发

```bash
bun install
bun test
bun run check
bun run pack:cli
```

`bun run pack:cli` 只装箱当前宿主的二进制。完整发布包由 `build-npm.yml` 在 Linux、Windows、macOS 的 x64/arm64 宿主上分别原生构建后统一装箱。
