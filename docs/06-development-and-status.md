# Vibe-SSH 开发与实现状态

日期：2026-11-16。当前阶段：M0 首个可运行内核切片。

## 当前仓库

| 位置 | 当前内容 |
| --- | --- |
| [protocol](../crates/protocol/src/lib.rs) | 执行请求与结果、控制权类型、协议版本 |
| [core](../crates/core/src/lib.rs) | SSH 连接、执行、PTY 通道输入、输出上限和 SFTP 接入 |
| [CLI](../crates/cli/src/main.rs) | capabilities、skill、开发 probe |
| [SSH 集成测试](../crates/core/tests/ssh_integration.rs) | 回环服务上的协议、失败路径和输入权检查 |
| [测试夹具](../crates/core/tests/support/mod.rs) | 临时身份、固定响应 SSH 服务、内存 SFTP 文件 |
| [Skill](../skills/vibe-ssh/SKILL.md) | 只描述 M0 实际可用接口 |
| [CI](../.github/workflows/rust.yml) | macOS、Windows、Linux 的格式、Clippy 和测试工作流 |

CI 工作流已经保存，但尚未在远端 CI 运行。当前结果来自本机 macOS，Rust 1.98.0。Cargo 声明的最低 Rust 版本为 1.89，仍需独立工具链验证。

## 本机验证证据

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

当前结果：4 项领域测试、11 项 SSH 集成测试通过，格式与 Clippy 通过。Skill 使用 skill-creator 的 quick_validate.py 校验通过。

| 验收项 | 当前证据 | 仍需补充 |
| --- | --- | --- |
| AC-001 身份 | 正确指纹连接，错误指纹在认证前失败 | 正式主机信任界面 |
| AC-002 执行 | 分离输出、非零退出码和退出状态后的尾部数据 | 真实 OpenSSH 互操作 |
| AC-003 未知结果 | 缺失退出码与观察超时不返回成功 | 真实网络故障与进程恢复检查 |
| AC-004 复用 | 多通道只发生一次认证和连接 | 后台跨 CLI 调用复用 |
| AC-005 PTY | PTY 请求、输入回显、尺寸消息和请求拒绝路径 | 实际操作系统 PTY、Shell 和 TUI |
| AC-006 接管 | 接管确认后 16 个迟到旧请求被拒绝 | UI/IPC 层竞态与授权身份 |
| AC-007 归还 | 新版本控制权有效，旧令牌与跨通道令牌无效 | 外部 Codex 完整交互 |
| AC-008 输出 | 字节上限、UTF-8 分包、截断边界与无效字节标记 | 增量游标、事件订阅与持久日志 |
| AC-009 SFTP | 同一连接上的虚拟文件字节往返一致 | 真实磁盘、权限、冲突与同步 |

测试夹具通过真实 SSH/SFTP 协议通信，但远端命令使用固定响应，不会执行收到的 Shell 命令。它证明通道与状态处理，不代表已通过真实 Linux 服务器部署验收。POSIX 路径转义另在本机临时目录中通过实际 Shell 验证。

## 可用命令

```sh
cargo run -p vibe-ssh-cli -- --help
cargo run -p vibe-ssh-cli -- capabilities
cargo run -p vibe-ssh-cli -- skill
```

`capabilities` 明确报告 `m0_development` 阶段，以及不存在的桌面、凭据库、后台 IPC 和部署工作流。仓库 Skill 尚未安装到用户全局目录。

## 开发 probe

只在用户明确选择主机、身份和操作后使用。连接资料由人工创建；不自动发现主机、私钥或信任指纹。

连接文件结构如下，示例值需要替换，指纹必须通过独立渠道核对：

```json
{
  "host": "203.0.113.10",
  "port": 22,
  "username": "your-test-account",
  "identity_file": "/absolute/path/to/test-identity",
  "host_key_sha256": "SHA256:independently-verified-fingerprint"
}
```

请求文件为数组：

```json
[
  {
    "command": "printf 'connection verified\\n'",
    "cwd": "/tmp",
    "pty": false,
    "timeout_ms": 30000,
    "output_limit": 8192
  }
]
```

```sh
cargo run -p vibe-ssh-cli -- probe --config /absolute/path/connection.json --requests /absolute/path/requests.json
```

M0 只读取未加密 OpenSSH 私钥。每次 probe 建立一条连接，并在该连接上顺序执行 1 至 32 个请求，遇到失败或未知结果即停。每个请求独立 Shell 上下文；`cwd` 必须是绝对 POSIX 路径。

输出上限为每流 4 至 1,048,576 字节，默认每流 8,192 字节，只保留尾部并记录总字节数。PTY 输出标记为合并。观察超时关闭本地通道并返回未知，不保证远端进程已经停止。

当前 CLI 的结果在请求序列结束后返回 JSON，尚未提供 UI 实时观察或外部增量等待。工具错误通过 stderr 的 JSON 错误返回。若后续请求发生工具错误，前面任务的完整持久记录尚不可用，因此此入口限于开发验证。

退出码：0 表示全部已执行请求成功；1 表示收集到失败或未知结果；2 表示工具、配置或通信错误。远端退出码在结果 JSON 的 `exit_code` 中。

## 内核边界

输入权对象当前只在可信进程内使用，不等价于跨进程身份认证。每个通道使用独立租约，控制版本递增。单次输入最多 32 KiB；提交超时说明字节可能部分送达，不能直接重试。

当前的 SFTP 入口是底层库能力，还没有正式文件策略、目录同步、覆盖保护或 UI。当前没有数据库、系统凭据存储、后台服务或桌面进程。没有完成性能基准，也没有提速倍数结论。

## 下一开发切片

1. 通过专用测试环境验证 OpenSSH 互操作、真实 PTY/Shell、认证与断线行为。
2. 实现有界增量事件与游标，再设计后台进程和本地 IPC 的可运行切片。
3. 进入 M1：Tauri 壳、手动主机配置、可信指纹保存与真实终端。

当前锁文件含上游引入的预发布加密依赖。M0 锁定用于验证；正式发布前需要确认成熟依赖组合、平台构建、互操作矩阵与相关通告，不能把本轮构建视为生产版本批准。
