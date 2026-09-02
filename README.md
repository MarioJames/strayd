# Port Deck

Port Deck 是一个用 Tauri 2 与 Rust 编写的 Windows 托盘工具，用来发现并结束 Windows、WSL 中“占着端口却找不到在哪”的本地开发服务。

## 能做什么

- 扫描 Windows 与所有运行中的 WSL 发行版，按进程合并多个监听端口。
- 展示端口、PID、命令行、工作目录、项目名和发行版。
- 识别 Next.js、Vite、Nuxt、Astro、SvelteKit、Remix、Angular、Storybook、Webpack、Rspack、Parcel、Node.js、Bun 与 Deno。
- 支持按端口、项目、进程或路径搜索，并按 Windows / WSL、开发服务筛选。
- 两次确认后结束整个进程树；执行前再次比对进程启动标识，避免 PID 已复用时误杀新进程。
- 关闭窗口后留在系统托盘；左键托盘图标恢复窗口。

`docker-desktop` 与 `docker-desktop-data` 是内部发行版，不会进入 WSL 扫描；Docker 映射到 Windows 的监听端口仍会由 Windows 扫描发现。

## 开发

```bash
bun install
cargo test -p port-deck-core
bun run dev
```

`bun run dev` 可在浏览器里查看带模拟数据的界面。`bun tauri dev` 在 WSL 中运行的是 Linux 调试版；正式 Windows 行为以交叉编译产物为准。

## 从 WSL 打包 Windows 安装器

开发机已配置 Rust stable、MSVC target、cargo-xwin、LLVM/LLD 与 NSIS，直接运行：

```bash
bun run build:windows
```

产物位于：

```text
target/x86_64-pc-windows-msvc/release/bundle/nsis/
```

WSL 可以生成 NSIS `setup.exe`；MSI 仍要求在 Windows 上使用 WiX 构建。首次交叉编译会下载 Windows SDK 到 `~/.cache/cargo-xwin`，后续项目可以复用。

## 扫描与终止边界

WSL 扫描通过 `wsl.exe --list --running --quiet` 获取已运行发行版，再以 root 读取 `ss` 和 `/proc`；不会为了扫描而启动已停止的发行版。发行版缺少 `ss` 时，界面会明确提示安装 `iproute2`。

终止 Windows 服务使用 `taskkill /T /F`，终止 WSL 服务会先向目标及其后代发送 `TERM`，短暂等待后仅对仍存活的同一进程树发送 `KILL`。Windows 受保护进程仍可能要求以管理员身份运行 Port Deck。
