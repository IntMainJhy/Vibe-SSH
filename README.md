# Vibe-SSH

Vibe-SSH 是供用户与外部 Agent 共同使用的 SSH 客户端。用户手动添加自己的服务器；Codex、Claude Code 等 Agent 读取项目提供的 Skill 后，通过本地 `vibe-ssh` 命令行执行操作。Agent 的每个目标是一个任务空间，有独立的 SSH 连接，用户可以同时使用自己的终端，并随时接管 Agent 的空间。

产品定位是日常 SSH 客户端，外部 Agent 通过 Skill 接入。项目不内置大模型或自主决策 Agent。技术路线为 Rust/Tokio、russh、Tauri 2、React/TypeScript 和 xterm.js，首版只面向 macOS。

## 当前阶段

2026-11-17：Agent 闭环已实现，并在真实 Linux 服务器（本机 Debian 13 虚拟机）上验证：执行、终端、上传、中断、接管、sudo 密码交接。macOS 桌面应用已实现：添加服务器并核对指纹、钥匙串、自己的终端、观看并接管 Agent 的空间。静态站点（Nginx）和 Docker Compose 两个部署流程已写入 Skill，并在 Linux 上照原文跑通，包括失败与恢复。

还没有的证据：桌面窗口里的点击操作和钥匙串没有经过人工验收；没有真实 Agent 凭 Skill 独立完成过部署；没有在公网服务器上验证过。

桌面工作台现支持服务器与会话搜索、手动重连、外观切换、终端字号记忆和完整命令复制。长文本输入按 Unicode 边界分块，发送失败会显示错误并停止后续排队输入。

先阅读 [方案总览](docs/README.md)，实际能力与限制见 [开发与实现状态](docs/06-development-and-status.md)。

## 本地开发

```sh
(cd apps/desktop && npm ci && npm test && npm run build)
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo fmt --all -- --check
(cd apps/desktop && npx tauri build)
```

桌面 crate 在编译时嵌入已构建的界面，所以要先构建前端。

需要一台真实的 Linux 服务器来试用或测试时，`scripts/linux-vm.sh up` 会在本机启动一台 Debian 虚拟机（需要 Apple 芯片的 Mac 和 `brew install qemu`），`scripts/linux-vm.sh hosts` 输出可以直接放进 `~/.vibe-ssh/hosts.json` 的主机配置。

测试在回环地址启动临时 SSH 服务并生成临时密钥，不使用用户的 SSH 配置或真实服务器。端到端测试需要 macOS 自带的 `/usr/sbin/sshd`。

## 产品必须保持的边界

- 服务器由用户手动配置，Agent 只能使用用户已开启接入的主机。
- 外部 Agent 负责推理，本地内核负责确定性执行和状态返回。
- CLI 与界面共享同一个内核，执行与观察不能各自维护一套会话。
- 接管在内核撤销写权限；中断进程是另一个明确动作。
- 用户自己的终端对 Agent 不可见；不同任务空间不共用连接。
- 不用提示符或日志关键词猜测命令是否成功；没有退出状态时返回未知。
- 性能结论必须来自同等条件的测量。

## 文档与贡献

架构决策见 [ADR 0001](docs/adr/0001-runtime-and-reuse.md) 与 [ADR 0002](docs/adr/0002-task-spaces-and-agent-neutrality.md)，[协议说明](docs/03-agent-protocol.md)记录外部 Agent 契约，[安全与可靠性](docs/04-security-and-reliability.md)记录凭据、权限与故障边界。

第三方组件的许可证保留在各自上游。Vibe-SSH 自身的对外发布许可证尚未确定，当前仓库不代表已获得生产环境验收。
