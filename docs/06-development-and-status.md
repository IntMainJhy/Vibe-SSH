# Vibe-SSH 开发与实现状态

日期：2026-11-17。当前阶段：M1 Agent 闭环已实现并验证；M2 macOS 桌面应用已实现，内核与命令层已验证，真实窗口内的操作尚未经人工确认。

## 当前仓库

| 位置 | 当前内容 |
| --- | --- |
| [protocol](../crates/protocol/src/lib.rs) | 协议版本 2：请求、任务与空间视图、控制权、错误码、上限 |
| [service](../crates/core/src/service.rs) | 任务空间、任务、租约校验、接管/归还/交接、上传接收 |
| [server](../crates/core/src/server.rs) | 当前用户专属的 Unix 套接字，只暴露 Agent 操作 |
| [ssh](../crates/core/src/ssh.rs) | 固定指纹连接，私钥（可带口令）与密码认证，指纹探测 |
| [upload](../crates/core/src/upload.rs) | 在调用方进程内打包并按帧发送 |
| [output](../crates/core/src/output.rs) | 按字节偏移寻址的有界输出，UTF-8 安全 |
| [skill](../crates/core/src/skill.rs) | 引导式 Skill 与命令的安装，不覆盖无关条目 |
| [CLI](../crates/cli/src/lib.rs) | `vibe-ssh` 命令行 |
| [桌面后端](../apps/desktop/src-tauri/src/main.rs) | Tauri 外壳：内核在应用进程内运行，人工操作命令，钥匙串 |
| [桌面界面](../apps/desktop/src) | React 与 xterm.js：主机管理、任务空间、终端、接管与归还 |
| [Skill](../skills/vibe-ssh) | `SKILL.md` 引导文件与 `guide.md` 使用说明 |

## 本机验证证据

```sh
(cd apps/desktop && npm ci && npm run build)
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

当前结果（macOS，Rust 1.98.0）：格式与 Clippy 通过；28 项测试通过，连续运行 6 次无失败。内核单元测试 6 项，回环夹具集成测试 18 项，CLI 端到端测试 3 项（其中 2 项使用真实 OpenSSH），桌面命令测试 1 项（使用真实 OpenSSH）。

### M1 Agent 闭环

| 验收项 | 当前证据 | 仍需补充 |
| --- | --- | --- |
| AC-001 身份 | 错误指纹在认证前被拒绝，认证次数为 0 | — |
| AC-002 执行 | 分离输出、非零退出码、退出状态后的尾部数据；真实 OpenSSH 上同样成立 | 真实 Linux 服务器 |
| AC-003 未知结果 | 缺失退出码与连接中断返回未知，不返回成功 | 真实网络故障 |
| AC-005 PTY | 真实 OpenSSH 上的提示符、输入应答、合并输出 | TUI 程序 |
| AC-008 输出 | 字节上限、增量游标、轮换缺口标记、UTF-8 不切断 | 磁盘日志 |
| AC-010 空间隔离 | 两个空间两条连接；一条断开后另一条继续执行 | 并发负载下的延迟测量 |
| AC-011 主机授权 | 未开启接入的主机不列出、不能打开，且不发起连接 | — |
| AC-012 接管 | 接管后迟到输入、新任务、调整尺寸、中断、关闭、交接、上传全部被拒绝，服务器收到的命令列表里没有被拒绝的任何一条；经真实套接字和 OpenSSH 复验 | — |
| AC-013 归还与交接 | 旧租约返回 `STALE_LEASE`，新租约可用；交接附带原因 | — |
| AC-014 用户空间 | 对 Agent 不可列出、不可读取、不可写入、不可关闭，不能交给 Agent | — |
| AC-015 增量等待 | 长任务先返回运行中，凭游标读完，标记文件证明只执行一次 | — |
| AC-016 真实互操作 | 系统 sshd 上完成执行、含特殊字符的工作目录、脚本、PTY、中断、上传、加密私钥 | 密码认证只在夹具上验证；真实 Linux 服务器 |
| AC-017 版本与实例 | 协议版本不符、服务未运行、旧实例标识、同目录第二个服务都有明确错误；目录 0700、套接字 0600 | — |
| AC-018 Skill 安装 | 已存在的无关同名目录保持原样并在结果中列出 | — |

上传另有三项证据：排除规则生效且非 ASCII 路径完整；上传中途被接管，任务以 `upload_incomplete` 失败；发送方进程被杀，任务同样以 `upload_incomplete` 失败而不是成功。

### M2 桌面应用

| 验收项 | 当前证据 | 仍需补充 |
| --- | --- | --- |
| AC-019 界面配置 | 命令层：探测指纹、未取得指纹时拒绝保存、保存后生成主机 ID（真实 sshd）。表单在浏览器预览中用示例数据检查过 | 在真实窗口里完整走一遍 |
| AC-020 凭据 | 代码已写：钥匙串读写，只在私钥无法直接读取或需要密码时才访问 | 没有任何测试或运行触及钥匙串 |
| AC-021 并行使用 | 内核层同 AC-010 | 在界面里边看 Agent 边打字的实际体验 |
| AC-022 界面接管 | 命令层（真实 sshd）：接管前用户输入被拒绝，接管后 Agent 被拒绝而用户输入生效，归还后用户被拒绝。接管按钮在浏览器预览中点过 | 在真实窗口里点击 |
| AC-023 退出提示 | 代码已写 | 未验证 |
| AC-031 同一命令 | 发布版应用的可执行文件链接为 `vibe-ssh` 后完成 `status`、`hosts`、`space open`、`put`、`run`、`space close`；3 MB 上传内容逐字节一致，被排除的 `.env` 未上传 | 界面里的安装按钮未点过 |

发布版应用在临时数据目录下启动过：套接字建立，界面加载并调用了内核（通过 `VIBE_SSH_TRACE=1` 的调用记录确认），Agent 打开空间并运行任务后界面开始读取该任务的输出。本次开发环境不允许截屏，所以真实窗口的画面没有被看到过；界面外观的检查来自浏览器预览加示例数据。

回环夹具通过真实 SSH 协议通信，但远端命令是固定响应。OpenSSH 测试使用系统自带的 `/usr/sbin/sshd`，以当前用户身份监听回环随机端口，使用临时密钥，不读取用户的 SSH 配置，结束时终止进程并删除临时目录。远端 Shell 是 macOS 的，不是 Linux。

## 构建与运行

```sh
cd apps/desktop
npm ci
npx tauri build
open ../../target/release/bundle/macos/Vibe-SSH.app
```

应用是临时签名，没有公证，只适合在构建它的这台 Mac 上运行。数据目录是 `~/.vibe-ssh`（可用 `VIBE_SSH_HOME` 改变）：`hosts.json` 保存主机，`agent.sock` 是 Agent 入口。密码和私钥口令在系统钥匙串里，服务名 `io.github.intmainjhy.vibe-ssh`。

界面左下角的“让 Agent 可以使用”把 Skill 链接到 `~/.agents/skills` 和 `~/.claude/skills`，并把应用自身链接为 `~/.local/bin/vibe-ssh`。之后 Agent 的用法见 `vibe-ssh skill`，协议见[外部 Agent 协议](03-agent-protocol.md)。

开发界面时 `npm run dev` 可在浏览器里用示例数据预览，示例数据不进入正式构建。不带界面运行内核用 `cargo run -p vibe-ssh-cli -- serve`，它只支持未加密的私钥。

## 已知限制

- 应用窗口关闭即退出，所有空间和连接随之结束；没有托盘常驻。
- 任务与输出只在内存中，应用退出即丢失。
- 同一系统用户下的进程都能读到 Agent 空间的当前租约；租约防过期请求，不防其他本机进程。
- 通过标准输入提交的多行脚本在远端作为一个任务运行，接管不能阻止已在运行的脚本的后续行。
- 上传依赖远端有 `tar`，没有逐文件的冲突检测；没有下载和文件浏览。
- 界面里用户的终端只有登录 Shell；Agent 的非终端任务输出按原样显示，不解析进度。
- 没有部署参考流程（静态站点、Docker Compose）；Skill 只说明通用的执行、上传与控制权规则。
- 没有性能基准，没有提速结论。
- CI 工作流已保存但尚未在远端运行；声明的最低 Rust 版本 1.89 未经独立工具链验证。
- 锁文件含上游引入的预发布加密依赖，正式发布前需要确认。

## 下一开发切片

1. 用户在真实窗口里完成 AC-019 至 AC-023 的人工验收，并让 Codex 与 Claude Code 通过已安装的 Skill 在真实 Linux 测试主机上完成一次部署。
2. 部署参考流程：静态站点加 Nginx、Docker Compose，各自在真实主机上验证后写入 Skill。
3. 任务记录持久化、托盘常驻、文件浏览与下载。
