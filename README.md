# Vibe-SSH

Vibe-SSH 是供用户与外部 AI 共同使用的桌面 SSH 客户端。用户手动添加自己的服务器，Codex 读取项目提供的 Skill 后通过本地接口执行操作；用户在客户端观看同一执行过程，并可接管输入。

项目不内置大模型或自主决策 Agent。首版技术路线为 Tauri 2、React/TypeScript、Rust/Tokio、russh 和 xterm.js。

## 当前阶段

2026-11-16：详细设计基线已保存，Rust 工作区、开发版 CLI 和 M0 首个可运行切片已建立。本机 4 项领域测试与 11 项 SSH 集成测试通过。完整桌面客户端、持久凭据库和部署工作流属于后续里程碑。

先阅读 [方案总览](docs/README.md)，再阅读 [交付与验收计划](docs/05-delivery-and-acceptance.md)。

## 本地开发

```sh
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo fmt --all -- --check
cargo run -p vibe-ssh-cli -- capabilities
```

当前测试在回环地址启动临时 SSH/SFTP 协议服务，生成临时密钥，不使用用户 SSH 配置或真实服务器。开发版 `probe` 可根据手工准备的连接配置执行指定请求，但还不是正式的已保存主机或后台服务入口。

已实现与待实现范围、测试证据及实验入口见 [开发与实现状态](docs/06-development-and-status.md)。

## 产品必须保持的边界

- 服务器由用户手动配置，AI 使用已经授权的主机标识。
- 外部 Codex 负责推理，本地内核负责确定性执行和状态返回。
- CLI 与界面共享同一个内核，执行与观察不能各自维护一套会话。
- 接管在内核撤销输入权；中断进程是另一个明确动作。
- 不用提示符或日志关键词猜测命令是否成功；没有退出状态时返回未知。
- 性能结论必须来自同等条件的测量。

## 文档与贡献

[架构决策](docs/adr/0001-runtime-and-reuse.md)记录技术路线及备选方案，[协议设计](docs/03-agent-protocol.md)记录外部 AI 契约，[安全与可靠性](docs/04-security-and-reliability.md)记录凭据、权限与故障边界。

第三方组件的许可证保留在各自上游。Vibe-SSH 自身的对外发布许可证尚未确定，当前仓库不代表已获得企业生产环境验收。
