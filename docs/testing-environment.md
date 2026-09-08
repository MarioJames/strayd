# Strayd 测试环境

测试环境用轻量 mock 程序构造应用目录、命令行、父子进程和 TCP 监听，再由生产扫描器与停止逻辑调用真实系统 API 验证。无需安装第三方应用；所有应用场景均在临时目录内启动和清理。

## 快速运行

前置：Rust 1.98.0、Bun 1.3.12；运行时基础套件另需 Node 和 Python。测试依赖由 `Cargo.lock`、`bun.lock` 锁定。

```bash
bun install --frozen-lockfile
bun run test
bun run check

# 一次构建后运行多个套件；默认每个套件最多 180 秒
bun scripts/test-env.ts run --profile native --suite native,tui,runtime-smoke,faults,replay

# 运行应用 mock：普通二进制的子命令与守护模式、测试播放器、Electron 辅助进程树
bun scripts/test-env.ts run --suite app-mock

# 查看入口、只运行某个场景
bun scripts/test-env.ts list
bun scripts/test-env.ts run --profile native --suite native --scenario native-home-proxy
bun scripts/test-env.ts run --profile replay --scenario native-home-proxy

# 验证运行器自身的超时、中断、错误分类和清理
bun run test:runner

# 每次运行打印报告路径；可重复执行独立清理
bun scripts/test-env.ts cleanup --run <run-id>
```

`--no-build` 使用已有 debug 产物；缺产物会报 blocked。`--timeout-ms` 设置单个执行步骤的截止时间。`--run` 可指定唯一 ID，已有运行目录不能覆盖。多个 suite 用逗号分隔；即使一个套件失败，其余独立套件仍会执行。

报告位于 `.test-env/runs/<run-id>/report.json`，并列目录保留各套件报告、所属进程身份、有限诊断日志、PTY 截图文本和依赖版本。`pass / fail / blocked / not-applicable` 分别代表通过、断言或执行失败、缺少必需能力、不适用于该平台；blocked 和清理失败均返回非零退出码。

## 已实现套件

| suite | 实际执行与独立断言 |
| --- | --- |
| `smoke` | 普通二进制的子命令与守护模式、真实可执行文件名、TCP/IPv6、多端口、排除 UDP 与扫描器自身、CLI |
| `native` | smoke 加同端口多地址、扫描期间退出、生命周期、macOS 直接执行 `.app` |
| `lifecycle` | 父子树、子进程先退出、TERM 无响应、目标已退出、错误启动标识、sshd 保护及独立对照进程 |
| `cli` | JSON/端口筛选、dry-run、非交互确认、实际停止、config init/show/path、隐藏及绕过、损坏配置和无法写入路径 |
| `tui` | portable-pty 启动真实 TUI；中英文，132×40 / 100×18 / 80×24，持续计时、缩放、滚动、完整 OSC 52 复制、选择回顶、弹窗取消、保存隐藏规则、确认停止、终端恢复 |
| `runtime-smoke` | 真实 Node、Bun、Python 脚本及模块；Linux 另验 Node 修改后的 OS 进程标题 |
| `runtimes` | 上述基础运行时加 Deno、Java JAR、.NET DLL；缺少运行时/编译器即 blocked |
| `faults` | 故意启动失败、不发送 ready、断言失败，检查异常和进程回收 |
| `stability` | 五轮八个同名进程并存，每个三个端口；检查发现与回收并记录耗时 |
| `replay` | 七个明确标记 synthetic 的路径/命名/缺失字段记录，调用实际规则函数，禁止系统动作 |
| `package` | 临时安装真实 tgz 后，经 Node 包入口运行 CLI；另检查原生文件缺失错误、产物内容和哈希 |
| `tunnel` | 实际 OpenSSH 客户端连接离线 Paramiko SSH 对端，反向转发已知响应，检查与源进程关联及停止后保留源进程 |
| `systemd` | 任务专属 transient service；发现 unit、拒绝错误关系、停止和独立回收 |
| `permissions` | 专用容器内，普通用户调用 CLI 和原生终止接口；root 所属进程保持存活，接口返回 SignalDenied |
| `wsl` | PowerShell 启动 Windows TCP 监听，Linux 夹具在另一个 loopback 地址绑定同端口；只发现 Linux 资源 |
| `desktop` | macOS LaunchServices 实际启动最小 `.app`，验证 bundle 名称并清理 |
| `app-mock` | 普通二进制两种启动方式、中文 `.app` 路径、Electron 风格主进程及 utility helper；真实 CLI/采集器检查名称与端口，停止父子树并保留同名对照实例；无需安装应用 |

固定日期、时区、运行时长边界、分类、关联、跨平台请求等规则契约继续由既有 Cargo 测试覆盖。集成套件没有替换它们，也不把宿主所有资源的数量或排序当成预期。

场景按类型命名，应用统一使用自定义名称：`fixture-agent`、`测试播放器`、`Strayd Test Desk` / `Test Helper`；运行时项目使用 `fixture-project`。保留 Node、Python、Electron 等技术类型名称用于说明覆盖范围。

`app-mock` 已纳入 `smoke` 和 `native`，随普通 CI 执行。`.app` 目录可在各宿主上构造并执行本机格式的夹具；Linux/Windows 的通过只证明这些路径的识别契约，macOS LaunchServices 另由可选 `desktop` 套件验证。

`tests/scenarios/catalog.json` 记录场景 ID、能力、启动模板和独立预期，契约会进入报告。实际 PID、端口和时间来自 OS 子进程句柄、实际 bind 后的 ready 消息、启动前后时间区间；没有先占端口再释放给夹具。名称预期来自明确规则和录制文件中的字面量，不由生产命名函数生成。

现有七份录制均为 synthetic。未来导入 `source: captured` 的记录必须提供 metadata 中的 `arch`、`collectorVersion`、`applicationVersion`、`capturedAt`、`missingFields`、`redaction`；缺少元数据会拒绝回放。captured 记录的执行方式仍是只读回放，不代表本机运行过对应应用。

## Linux 隔离镜像

```bash
# 首次可由统一入口构建，也可显式建立可复用镜像
bun scripts/test-env.ts run --profile linux-container --suite native

docker build -f tests/environments/linux/Dockerfile -t strayd-test:local .
docker build -f tests/environments/linux/Dockerfile.extended -t strayd-test:extended .

bun scripts/test-env.ts run --profile linux-container \
  --image strayd-test:extended --suite native,tui,runtimes,stability,tunnel
bun scripts/test-env.ts run --profile permissions-container \
  --image strayd-test:local --suite permissions
STRAYD_TEST_IMAGE=strayd-test:local bun test tests/container-runner.test.ts
```

基础镜像固定 Rust、Node、Bun 镜像 digest，提供 Python、JDK、OpenSSH 和 Paramiko；扩展镜像复制固定 digest 的 Deno 2.5.6 和 .NET SDK 8.0.414。所有 Rust 产物都在镜像内构建，避免宿主 glibc 差异。

普通测试以 UID 10001 执行，独立 PID/网络命名空间、`--network none`、`--cap-drop ALL`。只挂载本次证据目录，不挂宿主 `/proc` 或 Docker socket，不发布端口。权限容器使用 root 编排，只补充切换用户和清理进程所需的 SETUID/SETGID/KILL；被测权限调用降至 tester 用户。没有新增宿主账号。

容器内 scratch 位于私有 `/tmp`，证据写入 `/evidence`。因此即便容器被强制结束，临时文件也随容器销毁。容器 PID 账本带命名空间标记，绝不交给宿主 PID 清理器。镜像保留作为构建缓存；每次测试创建的容器自动删除。镜像 ID 与 `dependency-versions.txt` 进入报告：Debian 传递依赖仍取构建时软件源，不能仅凭 Dockerfile 宣称未来每次构建的所有系统包完全相同。

容器共享宿主内核。本地在 WSL2 内跑容器，不等价于已经验证其他 Linux 内核，更不能代替 macOS/Windows 原生采集。

## 浏览器终端

`tests/browser/server.ts` 的页面通过 WebSocket 连接实际 Bun PTY，xterm 只渲染终端输出；没有重画一份模拟 TUI。启动时创建两份自有夹具，扫描后只将它们留在隔离配置的可见列表中。

本地 Agent 验收通过 `browser-harness` 准备服务，将其返回的实际 `APP_URL` 交给项目 journey：

```bash
# BH_DIR 为当前环境 browser-harness 技能的 scripts 目录
cargo build --locked -p port-deck-cli -p strayd-test-support
BH_DEV_COMMAND='bun tests/browser/server.ts' bun "$BH_DIR/bh.ts" prepare "$PWD"
# 仅在 prepare 成功后使用输出的 APP_URL
APP_URL='<prepare 返回的 URL>' AGENT_BROWSER_SESSION=strayd-test-env \
  bun tests/browser/journey.ts
# 按技能流程采证，然后 cleanup 同一 target 并 close 同一 browser session
```

journey 检查中英文三个尺寸、真实按键和点击、计时、完整复制、取消与退出，截图及结果写入 `.test-env/browser-evidence`。`scripts/browser-check.ts` 是 CI 的独立服务生命周期入口，也接受外部 `APP_URL`。它使用相同 journey，关闭测试浏览器并停止自身创建的服务；本地 Agent 的正式验收仍以 browser-harness 为入口。

浏览器运行需本机已有 Chrome 或先在测试环境执行 `bunx --no-install agent-browser install`。Bun Terminal 桥接运行在 Linux/macOS；Windows 终端由原生 portable-pty / ConPTY 套件验证，测试端会响应 ConPTY 初始化的光标位置查询。服务最长运行五分钟，正常退出会关闭 PTY、夹具控制管道及 HTTP 服务。

## npm 安装包

```bash
bun run pack:cli
bun scripts/test-package.ts dist/npm
# 或指定一个明确的 tgz
bun scripts/test-env.ts run --suite package,tui --artifact dist/npm/strayd-0.2.1.tgz
```

包安装在含空格的临时目录，经 `node .../bin/strayd.cjs` 进入真实原生程序。测试完成后删掉安装目录。`package,tui` 同时检查 TTY；只运行 Cargo 产物不能替代安装包验收。默认本地 pack 只有当前平台二进制，不能据此声称六个平台都验证过。测试 crate 标记 `publish = false`，npm 文件清单继续限定为 bin/native/说明文件。

## 可选系统专项

专项入口会附加对应的必需套件，不能用普通 smoke 绕过专项能力检查：

```bash
bun scripts/test-env.ts run --profile systemd-vm --suite lifecycle
bun scripts/test-env.ts run --profile wsl --suite native
bun scripts/test-env.ts run --profile desktop --suite native
```

systemd 默认使用当前用户的 manager。一次性 VM 的系统 manager 可设置 `STRAYD_TEST_SYSTEMD_SCOPE=system`。unit 名带测试前缀和唯一身份，设置 `RuntimeMaxSec=120`，stop 前验证扫描所得关系，清理账本记录该 unit。不要让通用 PID 用例停止 runner 自己的 systemd unit。

WSL 需启用 PowerShell interoperability；Windows 监听由本次启动的 PowerShell 进程持有，控制管道关闭或 15 秒上限后释放。127.0.0.1 与 127.0.0.2 用于避免镜像网络中同地址的端口冲突，本测试不修改 WSL 网络设置。

## CI 门禁

| workflow | 配置的执行范围 |
| --- | --- |
| `test.yml` | PR/main：Ubuntu x64、Windows x64、macOS arm64；Cargo/TS 检查、native/CLI/PTY/运行时基础/回放/故障；Linux 另跑真实浏览器终端 |
| `extended-test.yml` | 每日/手动：六架构、完整运行时、稳定性、离线 SSH、权限容器，以及独立专项 runner |
| `build-npm.yml` | 六架构原生测试后统一装箱，再在六种原生 runner 上安装同一个 tgz 并验证 CLI/TTY |

三个可选专项 runner 标签为 `strayd-systemd`、`strayd-wsl`、`strayd-desktop`；配置好对应系统能力后，设置仓库变量 `STRAYD_SPECIAL_RUNNERS=enabled`。未启用时 availability 记录 not-requested，不阻塞 mock 验收；显式运行某个专项而缺少能力仍返回 blocked。没有第三方应用安装任务。Linux 托管 runner 上，`scripts/ci-native.ts` 将夹具测试放入独立 systemd scope，保持原用户身份，避免夹具继承 runner 的服务归属；scope 运行结束后自动回收。

报告由 always artifact 步骤上传，清理失败使运行失败。本次只修改本地 workflow，没有 push 或 dispatch，因此尚无远端 CI 执行结论。GitHub runner 的实际 OS 镜像版本仍随每次报告记录；六架构构建成功不能证明最低 macOS/Windows/glibc 版本兼容。

## 本次实测边界

2026-09-08，Linux x64 / WSL2 6.6.87.2：原生、CLI、PTY、回放、稳定性、运行器故障清理、systemd 用户临时服务、WSL Windows/Linux 同端口、npm 安装包与浏览器交互已实测。Linux 容器的基础和完整运行时、跨用户权限也已实测；离线 SSH 场景使用本地真实协议对端，无公网依赖。

macOS/Windows 原生采集与六架构包安装已配置 CI，尚未在本次会话实际运行；macOS LaunchServices 仍需相应机器。四个应用 mock 场景（普通二进制 proxy/daemon、中文应用包、Electron 风格父子树）已在本机通过，并完成自有进程和临时目录清理。应用 mock 验证进程发现、命名和停止等 Strayd 契约，不验证第三方应用内部功能。

## 清理与证据边界

- 夹具 ready 提供真实 PID/端口；独立账本记录 PID、启动时间和可执行文件。发现 PID 已复用时不终止新进程。
- 正常结束先关闭控制管道、回收子进程，再独立核对所有登记对象；终止不依赖被测 `strayd stop`。
- Bun 外层在失败、超时、SIGINT/SIGTERM 后继续执行独立清理；容器销毁及夹具最长寿命为强制终止提供兜底。`bun test tests/runner.test.ts` 实际检查超时与 SIGINT 后的 OS 进程状态。
- 普通测试只保留自有进程的扫描证据，不提交整机进程转储、浏览器 profile 或原始凭证。所有运行输出位于 Git 忽略目录。
- PID 复用通过错误启动标识契约验证，不宣称已经复现内核级复用竞态。公网隧道、未知第三方启动器和未列出的 OS 版本均不在本次实测声明内。
