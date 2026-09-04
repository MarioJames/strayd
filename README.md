# Strayd

[![npm version](https://img.shields.io/npm/v/strayd?color=3dd6d0&label=npm)](https://www.npmjs.com/package/strayd)
[![CI](https://github.com/MarioJames/strayd/actions/workflows/build-npm.yml/badge.svg)](https://github.com/MarioJames/strayd/actions/workflows/build-npm.yml)
[![license](https://img.shields.io/badge/license-MIT-ab72ff)](LICENSE)

Strayd 是一个通过 npm 分发的跨平台 Rust TUI/CLI，用来发现、关联和停止散落在本机上的开发服务与临时公网隧道。

<p align="center">
  <img src="docs/screenshots/tui-overview.svg" alt="Strayd TUI：开发服务与 Cloudflare Tunnel 关联视图" width="100%" />
</p>

## 它解决什么

开发服务器和临时隧道经常由不同终端、IDE 或后台任务启动。端口还在监听，但原始终端早已找不到；看到一个进程时，又无法判断它是否正在承载别的服务。

Strayd 会：

- 识别 Next.js、Vite、Nuxt、Astro、SvelteKit、Remix、Angular、Storybook、Webpack、Rspack、Parcel、Node.js、Bun 和 Deno 服务。
- 识别 Cloudflare Quick Tunnel、ngrok、SSH 反向转发、frp、localtunnel 和 bore。
- 解析隧道的本地目标，把隧道与对应端口的源服务放进同一组。
- 支持只停开发服务、只停隧道，或按依赖顺序停止整个关联组。
- 展示并保护 `sshd`，避免误关远程入口。
- 根据当前系统自动使用 Windows、Linux/WSL 或 macOS 的原生探测与终止方式。
- 内置简体中文与英文界面，按系统语言自动选择，并在中性 locale 下使用时区兜底。

## 安装

要求 Node.js 18 或更高版本；不需要安装 Rust。

```bash
npm install --global strayd
strayd
```

也可以不安装直接体验：

```bash
npx strayd
```

支持以下原生组合：

- Linux x64 / arm64，包括 WSL
- Windows x64 / arm64
- macOS Intel / Apple Silicon

## TUI 操作

启动后左侧是资源组，右侧展示端口、进程、工作目录以及隧道到源服务的关联链路。所有停止操作都需要再次确认。

| 操作 | 键盘 | 鼠标 |
| --- | --- | --- |
| 切换分类 | `Tab`、`Shift+Tab`、`1-5` | 点击顶部完整分类区域 |
| 选择资源组 | `↑/↓`、`j/k`、`Home/End` | 点击资源行或滚轮 |
| 创建隐藏规则 | `h` | 点击 `HIDE` / `隐藏` |
| 勾选规则字段 | `Space` | 点击字段行 |
| 移除隐藏规则 | 在配置页按 `u` / `Delete` | 点击 `REMOVE` / `移除` |
| 停止开发服务 | `d` | 点击 `SERVICE` |
| 停止隧道 | `t` | 点击 `TUNNEL` |
| 停止整个组 | `x` | 点击 `GROUP` |
| 刷新 / 退出 | `r` / `q` | 点击 `REFRESH` / `QUIT` |
| 确认 / 取消 | `Enter` / `Esc` | 点击确认框按钮 |

终端启用鼠标捕获后，如需选择或复制文字，通常可以按住 `Shift` 再拖动鼠标。

## 持久化配置

Strayd 可以直接在 TUI 中维护隐藏规则：选中一个资源后按 `h`，勾选作为匹配条件的字段并保存；默认选择“端口 + 运行时”。进入顶部 `CONFIG` / `隐藏配置` 页可以查看并移除已有规则。修改会立即写入配置文件并刷新界面。

<p align="center">
  <img src="docs/screenshots/tui-config-editor.svg" alt="Strayd TUI 隐藏规则编辑器" width="100%" />
</p>

也可以直接维护 TOML。先生成带注释的模板：

```bash
strayd config init
strayd config path
strayd config show
```

配置位置遵循系统约定：

- Linux/WSL：`$XDG_CONFIG_HOME/strayd/config.toml`，默认是 `~/.config/strayd/config.toml`
- macOS：`~/Library/Application Support/strayd/config.toml`
- Windows：`%APPDATA%\strayd\config.toml`

例如，只隐藏监听 22 端口的 `sshd`，但不会隐藏同组中代理该端口的隧道：

```toml
version = 1
language = "auto"

[[display.hide]]
ports = [22]
runtimes = ["sshd"]
```

<p align="center">
  <img src="docs/screenshots/configuration.svg" alt="Strayd 持久化配置示例" width="100%" />
</p>

每条 `[[display.hide]]` 都是一条独立规则。规则之间是 OR；同一规则中填写的字段是 AND；同一数组内任意值匹配即可。空规则不会隐藏任何内容。

| 字段 | 匹配方式 |
| --- | --- |
| `ports` | 监听端口或隧道本地目标端口 |
| `runtimes` | 运行时名称，如 `sshd`、`vite`、`cloudflared` |
| `kinds` | `dev`、`tunnel`、`system`、`other` |
| `platforms` | `windows`、`linux`、`macos` |
| `process_names` | 进程名，忽略大小写的包含匹配 |
| `projects` | 项目名或工作目录，忽略大小写的包含匹配 |
| `commands` | 完整命令行，忽略大小写的包含匹配 |
| `ids` | Strayd 资源 ID，忽略大小写的精确匹配 |

规则同时作用于 TUI、`list` 和 `stop`。临时绕过配置可使用 `--no-config`；此时 TUI 配置页为只读。指定其他文件可使用 `--config <PATH>`：

```bash
strayd --no-config
strayd --config ./team-strayd.toml list
```

## 语言

`language = "auto"` 会优先读取系统 locale；`C`、`POSIX` 等中性 locale 再通过系统时区判断，仍无法判断时使用英文。配置和命令行都可以显式固定语言：

```toml
language = "zh-cn" # 或 "en"
```

```bash
strayd --language zh-cn
strayd --language en
strayd --language auto
```

优先级为 `--language` > 配置文件 > 系统语言/时区。JSON 字段、命令名、筛选参数和运行时标识保持不变，便于脚本稳定使用。

## CLI

无子命令时默认进入 TUI。脚本和自动化场景可以使用：

```bash
strayd list
strayd --language en list
strayd list --kind tunnel --json
strayd list --platform linux --port 5000
strayd stop tunnel --port 5000 --dry-run
strayd stop tunnel --port 5000 --yes
strayd stop group --port 5000 --yes
strayd stop dev --project storefront --all --yes
```

`list` 与 `stop` 支持 `--id`、`--port/-p`、`--project`、`--platform windows|linux|macos` 和 `--runtime`。停止命令匹配多项时必须显式传入 `--all`；实际关闭前需要交互确认或传入 `--yes`，`--dry-run` 只打印计划。

## 平台行为与安全边界

- Linux/WSL：读取 Linux 端口和进程；systemd 托管服务会尝试停止对应 unit。
- Windows：读取 Windows 原生端口和进程，并通过 `taskkill /T /F` 结束进程树。
- macOS：读取 macOS 原生端口和进程，先发送 `TERM`，必要时再发送 `KILL`。
- 每次停止前都会复核 PID 与进程启动标识，避免 PID 被复用后误杀新进程。
- 包含受保护资源的组拒绝整组停止；`sshd` 始终不可由 Strayd 终止。
- 部分系统进程的端口或命令行受操作系统权限限制。Strayd 只展示当前用户可读取的信息，不主动提权。

## 本地开发

```bash
bun install
bun run test
bun run check
bun run pack:cli
```

`bun run pack:cli` 只装箱当前宿主的二进制。完整 npm 包由 [build-npm.yml](.github/workflows/build-npm.yml) 在六种原生 runner 上分别构建、测试并统一装箱。

## License

[MIT](LICENSE)
