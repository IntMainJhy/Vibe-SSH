# Vibe-SSH 开发与实现状态

日期：2026-11-16。当前阶段：M1 Agent 闭环已实现并在本机验证；M2 macOS 桌面应用尚未开始。

## 当前仓库

| 位置 | 当前内容 |
| --- | --- |
| [protocol](../crates/protocol/src/lib.rs) | 协议版本 2：请求、任务与空间视图、控制权、错误码、上限 |
| [service](../crates/core/src/service.rs) | 任务空间、任务、租约校验、接管/归还/交接、上传 |
| [server](../crates/core/src/server.rs) | 当前用户专属的 Unix 套接字，只暴露 Agent 操作 |
| [ssh](../crates/core/src/ssh.rs) | 固定指纹连接，私钥（可带口令）与密码认证 |
| [output](../crates/core/src/output.rs) | 按字节偏移寻址的有界输出，UTF-8 安全 |
| [skill](../crates/core/src/skill.rs) | 引导式 Skill 的安装，不覆盖无关条目 |
| [CLI](../crates/cli/src/main.rs) | `vibe-ssh` 命令行 |
| [Skill](../skills/vibe-ssh) | `SKILL.md` 引导文件与 `guide.md` 使用说明 |
| [回环夹具测试](../crates/core/tests) | 固定响应的 SSH 服务上的协议、失败路径与控制权 |
| [OpenSSH 端到端测试](../crates/cli/tests/openssh.rs) | 真实 CLI、真实套接字、系统 sshd |

## 本机验证证据

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

当前结果（macOS，Rust 1.98.0）：格式与 Clippy 通过；26 项测试通过，连续运行 5 次无失败。其中内核单元测试 6 项，回环夹具集成测试 17 项，OpenSSH 端到端测试 3 项。

| 验收项 | 当前证据 | 仍需补充 |
| --- | --- | --- |
| AC-001 身份 | 错误指纹在认证前被拒绝，认证次数为 0 | 界面上的指纹核对 |
| AC-002 执行 | 分离输出、非零退出码、退出状态后的尾部数据；真实 OpenSSH 上同样成立 | 真实 Linux 服务器 |
| AC-003 未知结果 | 缺失退出码与连接中断返回未知，不返回成功 | 真实网络故障 |
| AC-005 PTY | 真实 OpenSSH 上的提示符、输入应答、合并输出 | TUI 程序 |
| AC-008 输出 | 字节上限、增量游标、轮换缺口标记、UTF-8 不切断 | 磁盘日志 |
| AC-010 空间隔离 | 两个空间两条连接；一条断开后另一条继续执行 | 实际并发负载下的延迟测量 |
| AC-011 主机授权 | 未开启接入的主机不列出、不能打开，且不发起连接 | — |
| AC-012 接管 | 接管后 16 个迟到输入、新任务、调整尺寸、中断、关闭、交接、上传全部被拒绝；经真实套接字和 OpenSSH 复验，被拒绝的命令未在服务器上产生文件 | 界面触发的接管 |
| AC-013 归还与交接 | 旧租约返回 `STALE_LEASE`，新租约可用；交接附带原因 | — |
| AC-014 用户空间 | 对 Agent 不可列出、不可读取、不可写入、不可关闭，不能交给 Agent | — |
| AC-015 增量等待 | 长任务先返回运行中，凭游标读完，标记文件证明只执行一次 | — |
| AC-016 真实互操作 | 系统 sshd 上完成执行、含特殊字符的工作目录、脚本、PTY、中断、上传、加密私钥 | 密码认证只在夹具上验证；真实 Linux 服务器 |
| AC-017 版本与实例 | 协议版本不符、服务未运行、旧实例标识、同目录第二个服务都有明确错误；套接字目录 0700、套接字 0600 | — |
| AC-018 Skill 安装 | 已存在的无关同名目录保持原样并在结果中列出 | — |

回环夹具通过真实 SSH 协议通信，但远端命令是固定响应，不执行收到的命令；它证明通道、状态与控制权处理。OpenSSH 端到端测试使用系统自带的 `/usr/sbin/sshd`，以当前用户身份监听回环随机端口，使用临时主机密钥和临时身份，不读取用户的 SSH 配置，结束时终止进程并删除临时目录。远端 Shell 是 macOS 的，不是 Linux。

## 使用方式

服务未随应用提供之前，用无界面方式运行：

```sh
cargo build -p vibe-ssh-cli
target/debug/vibe-ssh serve
```

主机由用户手写到 `~/.vibe-ssh/hosts.json`（数组）。指纹必须通过独立渠道核对：

```json
[
  {
    "id": "blog",
    "name": "博客服务器",
    "host": "203.0.113.10",
    "port": 22,
    "username": "deploy",
    "identity_file": "/absolute/path/to/key",
    "host_key_sha256": "SHA256:independently-verified-fingerprint",
    "agent_access": true
  }
]
```

无界面服务只支持未加密的私钥；加密私钥的口令和密码认证需要应用提供凭据来源。命令说明见 `vibe-ssh --help` 和 `vibe-ssh skill`，协议见[外部 Agent 协议](03-agent-protocol.md)。

Skill 尚未安装到本机的 Agent 目录；`vibe-ssh install-skill` 是用户的显式动作。

## 已知限制

- 没有桌面应用：用户目前无法在界面上添加主机、观看或接管。接管与归还已在内核中实现并测试，但只能由进程内调用触发。
- 任务与输出只在内存中，服务退出即丢失。
- 同一系统用户下的进程都能读到 Agent 空间的当前租约；租约防过期请求，不防其他本机进程。
- 通过标准输入提交的多行脚本在远端作为一个任务运行，接管不能阻止已在运行的脚本的后续行。
- 上传依赖远端有 `tar`，没有逐文件的冲突检测，也没有下载。
- 没有性能基准，没有提速结论。
- CI 工作流已保存但尚未在远端运行；声明的最低 Rust 版本 1.89 未经独立工具链验证。
- 锁文件含上游引入的预发布加密依赖，正式发布前需要确认。

## 下一开发切片

1. M2：Tauri 外壳、主机管理与钥匙串、用户终端、Agent 空间的观看与接管。
2. 在真实 Linux 测试主机上复验 AC-016，并让 Codex 与 Claude Code 通过已安装的 Skill 实际完成一次部署。
3. 任务记录持久化，以及脱离连接运行的长任务。
