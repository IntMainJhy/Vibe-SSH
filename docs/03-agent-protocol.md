# Vibe-SSH 外部 Agent 协议

## 状态与兼容性

公开测试版 0.2.0-beta.1 使用协议版本 2，此版本向后兼容 Preview 5 的调用。新增方法由 `status.capabilities` 报告，自动接入使用随包 CLI 和 Skill。真实可调用能力以 `vibe-ssh --help` 和 `vibe-ssh skill` 的输出为准。版本 1（M0 的 `probe`）已移除，原因与迁移说明见 [ADR 0002](adr/0002-task-spaces-and-agent-neutrality.md)。

协议主版本用于不兼容变化。请求版本与服务不一致时返回 `VERSION_MISMATCH`，不会带着错误结构继续执行。

## 入口

任何能执行本机命令行的 Agent（Codex、Claude Code、接入 DeepSeek 等模型的命令行工具）都可通过原生 `vibe-ssh` CLI 调用。SSH 连接由常驻服务持有，同一空间的 CLI 请求共享连接。

| 命令 | 用途 |
| --- | --- |
| `skill` | 输出与已安装版本一致的使用说明 |
| `status` | 服务版本、协议版本、实例标识 |
| `hosts` | 用户已开启 Agent 接入的主机 |
| `files list <space> [path] [--limit N]` | 一层远端目录的结构化元数据 |
| `files read <space> <path> [--max-bytes N]` | 读取一个有界远端 UTF-8 文本 |
| `resources <space>` | 单次 Linux 资源快照 |
| `space open / list / get / handoff / close` | 任务空间与控制权 |
| `run` | 在空间内执行命令或标准输入中的脚本 |
| `put` | 上传本地文件或目录 |
| `batch --host HOST --name GOAL [PLAN]` 或 `batch --lease LEASE [PLAN]` | 顺序提交一份本地命令及上传计划 |
| `history log RECORD [--stream stdout] [--offset 0] [--limit 8192]` | 分页读取授权任务的归档输出 |
| `diff <space> <project> <remote> [--select path] [--exclude glob]` | 只读元数据与所选内容差异预览 |
| `job get / wait / input / interrupt` | 查询、增量等待、输入、请求中断 |
| `install-skill` | 把引导式 Skill 链接到 Agent 目录（用户的显式动作） |
| `serve` | 无界面服务，仅供开发 |

添加主机、信任指纹、开启 Agent 接入、接管与归还不在 CLI 和套接字协议中，只能由用户在应用内完成。桌面的退出确认和复制命令使用应用内部命令 `exit_app`、`copy_text`，同样不对 Agent 套接字开放。

## 传输

服务监听 `$VIBE_SSH_HOME/agent.sock`，默认 `~/.vibe-ssh/agent.sock`。目录权限 0700，套接字 0600，并校验对端进程属于同一系统用户。不监听 TCP 端口。

每条连接处理一个请求：客户端发送一行以换行结尾的 JSON，服务返回一行 JSON 后关闭。请求行上限 2 MiB，10 秒内必须发完；同时处理的请求上限 64，超出返回 `RESOURCE_LIMIT`。`put` 的请求行之后紧跟归档数据，见“上传”。

```json
{"v": 2, "call": {"method": "run", "params": {"lease": "s1.ab12.9f3c…", "command": "npm run build", "cwd": "/srv/app", "pty": false, "wait_ms": 30000, "tail_bytes": 8192}}}
```

```json
{"ok": { "job": "j2.ab12", "status": "running", "…": "…" }}
{"error": {"code": "CONTROL_REVOKED", "message": "the user holds this space; stop writing to it"}}
```

方法：`status`、`hosts`、`space_open`、`space_list`、`space_get`、`space_close`、`space_handoff`、`run`、`put`、`job_get`、`job_wait`、`job_input`、`job_interrupt`、`diff`、`history_list`、`history_get`、`files_list`、`files_read`、`resources`。套接字上的所有调用都按 Agent 身份处理；请求里没有可以声明“我是用户”的字段。

## 结构化远端读取

Preview 5 之后新增如下方法。`status.capabilities` 包含对应方法名时可调用；旧应用应升级后再使用。保持协议 v2，无数据迁移。

| 方法 | 参数 | 返回 |
| --- | --- | --- |
| `files_list` | `space`, `remote`, `limit` (1–1000) | `path` 为远端解析后的目录；`entries` 含 `name`, `path`, `kind`, `size`, `modified`, `permissions`；`truncated` 为是否触及项数或 64 KiB 元数据预算（JSON 转义开销另计） |
| `files_read` | `space`, `remote`, `max_bytes` (1–65536) | `path` 为请求路径，`bytes` 为原始 UTF-8 字节数，`text` 保留换行及 BOM；超限、非 UTF-8、NUL 和末级链接均不返回文本 |
| `resources` | `space` | `cpu_percent`（1 秒采样，可能为空）、`cores`、`uptime_s`、`load`（1/5/15 分钟）；原磁盘字段仍是根分区。另有 `mounts`（`path`, `total`, `used`, `available`）和 `mounts_truncated`，GNU `df` 支持时最多返回 64 项，不含 tmpfs/devtmpfs；容量单位为字节 |

使用已授权的 Agent 空间，无需写租约。人工接管后可以读取，关闭主机接入后拒绝新读，个人空间一直不可见。使用同一 SSH 连接；目录和文本走 SFTP，监控走固定只读 Linux 探针。没有设备路径、编辑令牌、递归目录、自动轮询或任意脚本参数，也不创建命令历史。完整边界见 [ADR 0009](adr/0009-agent-resource-reads.md)。

## 标识

| 标识 | 形式 | 说明 |
| --- | --- | --- |
| 空间 | `s<序号>.<实例>` | 实例标识在服务每次启动时重新生成 |
| 任务 | `j<序号>.<实例>` | |
| 租约 | `<空间>.<随机串>` | 写操作凭据；接管、交接、归还都会更换 |
| 游标 | `<任务>:<stdout 偏移>:<stderr 偏移>` | 字节偏移 |

带有旧实例标识的空间、任务、租约和游标返回 `INSTANCE_MISMATCH`，不会指向新实例里的对象。

租约防止的是过期请求和接管之后的写入，不是 Agent 之间的身份隔离：同一系统用户下的进程都能通过 `space get` 读到 Agent 空间的当前租约。用户自己的空间对套接字不可见。

## 任务空间与控制权

`space open <主机> --name <目标> --actor <Agent 名称>` 建立一条新的 SSH 连接并返回空间和租约。主机未开启 Agent 接入或不存在时统一返回 `HOST_NOT_AUTHORIZED`，不区分两种情况。`actor` 只用于界面显示。

`control` 字段：

```json
{"owner": "agent", "lease": "s1.ab12.9f3c…", "note": null}
```

用户持有空间时 `owner` 为 `human`，`lease` 为空，`note` 是 Agent 交接时填写的原因。

| 时刻 | Agent 的写操作结果 |
| --- | --- |
| 用户接管或 Agent 交接之后 | `CONTROL_REVOKED` |
| 用户保存关闭主机接入或删除主机之后 | `CONTROL_REVOKED` |
| 用户归还之后仍使用旧租约 | `STALE_LEASE` |
| 用 `space get` 取得新租约之后 | 正常 |

写操作指 `run`、`put`、`job input`、`job interrupt`、`space handoff`、`space close`。租约校验与向通道提交数据在同一把锁内完成，接管确认之后不会再有 Agent 字节被提交；已经交给网络的字节无法撤回。接管不终止远端进程，读取（`space get`、`job get`、`job wait`）始终可用。

Preview 5 之后，桌面保存关闭接入或删除主机时，对该主机的已有 Agent 空间执行撤销。实时任务输出仍可读，用户可中断，新的远端 `diff` 和 `space open` 返回 `HOST_NOT_AUTHORIZED`。重新开启接入不会自动归还原空间；必须由用户显式归还并签发新租约。在握手过程中变更该主机配置的 Agent 开启请求被拒绝，调用者应读取最新主机列表。上传已在进行时，撤销可能表现为 `upload_incomplete`；不要推断远端没有留下部分文件。详见 [ADR 0008](adr/0008-host-access-revocation.md)。

## 任务结果

```json
{
  "job": "j2.ab12", "space": "s1.ab12", "command": "npm run build",
  "status": "running", "reason": null, "exit_code": null, "exit_signal": null,
  "interrupt_requested": false,
  "stdout": {"text": "building…\n", "total_bytes": 12, "truncated": false, "invalid_utf8": false},
  "stderr": {"text": "", "total_bytes": 0, "truncated": false, "invalid_utf8": false},
  "stream_mode": "separate", "cursor": "j2.ab12:12:0", "more": false,
  "control": {"owner": "agent", "lease": "s1.ab12.9f3c…", "note": null},
  "elapsed_ms": 1042
}
```

| `status` | 条件 | `reason` |
| --- | --- | --- |
| `running` | 通道尚未关闭 | 空 |
| `succeeded` | 退出码为 0 | `exit_status` |
| `failed` | 退出码非 0、被信号终止、服务器拒绝启动、上传被本地中止 | `exit_status`、`exit_signal`、`start_rejected`、`upload_incomplete` |
| `unknown` | 通道关闭但没有退出状态，或连接中断 | `missing_exit_status`、`connection_lost` |

没有退出码时 `exit_code` 为空，不会被转换成 0。退出状态之后到达的输出会继续收集，直到通道关闭。PTY 任务的 `stream_mode` 为 `merged`，错误输出并入 `stdout`。

每次 `run` 是独立的 Shell 上下文。`--cwd` 必须是绝对路径，按 POSIX 单引号规则转义后拼入命令。

## 等待与增量输出

`run` 和 `put` 最多等待 `--wait` 秒（默认 30，上限 120），到时任务仍在运行就返回 `running`，这不是失败。之后用返回的 `cursor` 调用 `job wait`：有新输出、任务结束、控制权变化或超时都会返回。

CLI 的 `run --summary` 和 `put --summary` 返回单行 JSON，保留任务、记录、退出码、状态、原因、两路输出、游标与当前控制方。精简模式不回显脚本或租约。默认输出、退出码、套接字协议 v2 与实际操作保持兼容。需要完整字段的调用者可继续用原输出或 `job get`。

`batch` 读取至多 256 KiB 的 JSON，对象包含 1 至 12 个 `steps`。每个步骤有 `name`，`type: "run"` 提供 `command` 和可选远端 `cwd`，`type: "put"` 提供 `local`、绝对 `remote_dir` 和可选 `exclude`。本地路径相对 CLI 工作目录。计划完全校验后，按给定主机新开一处空间，或在原租约下使用现有空间。每步输出尾部按每路 2 KiB 限制。

`outcome: "completed"` 表示顺序内全部收到成功回执。否则返回 `stopped`，包括已尝试步骤、结果或结构化错误及 `unsubmitted_steps`。回执包含 `deployment` 历史 ID。阶段未完成退出码为 1，单步工具错误退出码为 2。等待到期或结果不明确时保留游标并停止提交。结果不明时先核实实际状态，再安排剩余操作。[ADR 0013](adr/0013-cli-batch.md)

v2 新增 `deployment_start {lease, steps:[{name,kind}]}`、`deployment_step {lease,deployment,index,action,wait_ms}` 和 `deployment_stop {lease,deployment,reason}`。步骤使用从 0 开始的索引。上传步骤在同一连接续传归档帧，和原 `put` 一样由 CLI 自行读取本地文件。新方法需要随应用升级，原 `run` 和 `put` 保持兼容。后续每步继续核对同一租约。

- 输出按字节计量，返回的文本不会切断 UTF-8 字符；末尾不完整的字符留到下一次。
- 每路输出保留最近 1 MiB。游标落在已被轮换的位置时，从最早可用处返回并置 `truncated`。指向未来位置的游标同样置 `truncated`。
- `more` 为真表示游标之后还有已收到的输出。
- `job get` 返回尾部 `--tail` 字节；之前还有输出时置 `truncated`。

## 上传

`put <本地路径> <远端目录>` 由 CLI 进程自己运行本机的 `tar` 打包，服务通过 exec 通道交给远端 `tar -xof -` 解包。本地文件由调用方进程以调用方自己的权限读取，服务不会替调用方打开任何本地路径。目录上传的是其内容。同路径文件被覆盖，远端多出的文件不删除，不套用本机的文件属主。`--exclude` 传给本机 tar。

归档数据在 `put` 请求行之后按帧发送：4 字节大端长度加数据，单帧最多 32 KiB，长度为 0 的帧表示归档完整结束。只有本机 tar 成功退出后才发送结束帧。连接在结束帧之前断开，或 60 秒内没有新的帧，都按上传被中止处理。CLI 在全部数据发完后才返回。

上传的每一块数据都经过与输入相同的租约校验。上传中途被接管、发送方退出或本机 tar 失败时，任务以 `failed`、`upload_incomplete` 结束，`exit_code` 为空，远端目录里是不完整的副本。

## 错误语义

| 错误 | 含义 | Agent 应当 |
| --- | --- | --- |
| `INVALID_REQUEST` | 参数或结构无效 | 修正参数 |
| `VERSION_MISMATCH` | 协议版本不一致 | 使用随应用安装的 CLI，重新读取 `vibe-ssh skill` |
| `NOT_FOUND` | 空间或任务不存在、已关闭或已被淘汰 | 重新打开空间并查证远端状态 |
| `INSTANCE_MISMATCH` | 标识属于已结束的服务实例 | 同上 |
| `HOST_NOT_AUTHORIZED` | 主机未向 Agent 开放或不存在 | 交由用户处理 |
| `HOST_KEY_MISMATCH` | 主机公钥与用户核对过的指纹不一致 | 停止，交由用户核对 |
| `AUTHENTICATION_FAILED` | 认证失败 | 不猜测凭据，交由用户处理 |
| `CONNECTION_FAILED` | 连接建立失败或空间的连接已断开 | 查证远端状态后再决定 |
| `CONTROL_REVOKED` | 用户持有该空间 | 停止写入，不另开空间绕过 |
| `STALE_LEASE` | 控制权已归还，租约已更换 | 读取空间，查证远端状态，不直接重放被拒绝的命令 |
| `DELIVERY_UNKNOWN` | 提交超时，字节可能部分送达 | 先看任务输出，不直接重发 |
| `CHANNEL_REJECTED` | 服务器拒绝了 PTY 等请求 | 调整请求 |
| `RESOURCE_LIMIT` | 空间、任务或并发请求达到上限 | 等待或关闭已完成的空间 |
| `SERVICE_UNAVAILABLE` | 服务未运行或无法连接本地套接字；详细原因见 `message` | 未启动时请用户打开应用；权限拒绝时请用户检查 Agent 宿主的套接字授权，不更改沙箱策略 |
| `INTERNAL` | 本机文件或内部错误 | 报告给用户 |

等待超时不是错误。错误信息不包含密码、私钥内容或口令。

## CLI 退出码

0：调用成功，且返回的任务成功或仍在运行。1：返回的任务失败或结果未知。2：工具、服务或协议错误，错误以 JSON 写到标准错误。远端退出码只在 JSON 的 `exit_code` 中。

## 上限

每个服务 32 个空间；每个空间同时 8 个运行中的任务，保留最近 64 个任务；单条命令 1 MiB；单次输入 32 KiB；单次读取 1 MiB。

## Skill

安装到 Agent 目录的 `SKILL.md` 指向当前版本的 `vibe-ssh skill`，可按任务读取静态站点或 Compose 指南。使用说明涵盖授权主机、任务空间、输出处理、故障查证、接管与交接、HTTP 核验。

安装器只创建指向 `~/.vibe-ssh/skill/vibe-ssh` 的符号链接。目标位置已有不属于本工具的同名条目时不覆盖，并在结果中列出。


0.2.0-preview.2 保持协议 v2；新增的桌面 SFTP、设备文件选择、资源监控和自动接入配置不加入套接字。CLI `put` 仍使用已有 tar 流和覆盖语义，与桌面的“不覆盖单文件上传”是不同的明确入口。Skill 引导文件随应用启动同步，使用说明始终由当前 CLI 输出，无需协议迁移。

0.2.0-preview.3 的文本预览、保存和草稿释放仅为桌面人工命令，不增加 Agent 方法；Agent `status` 不包含个人打开文档的信息。`FILE_CHANGED` 仅为桌面乐观版本检查错误，控制代次变化仍为 `CONTROL_REVOKED` / `STALE_LEASE`，提交结果待核实时为 `DELIVERY_UNKNOWN`。协议仍为 v2，无 CLI 迁移。

目录差异：v2 新增 `diff {space, remote, manifest}` 只读方法。manifest 为 `entries[{path,kind,size,digest,excluded}]`、`excludes` 和 `truncated`；仅允许规范相对路径。CLI 在调用方读取所选内容，服务不接收设备路径。返回 `{remote, entries[{path,local_size,remote_size,status}], truncated}`。状态包括 same、changed、local_only、remote_only、type_conflict、directory、excluded、unsupported、unverified；截断时不据缺失判独有。预算与边界见 ADR 0006。旧方法兼容，新 CLI 需配套更新应用以支持新增方法。

应用差异清单和 SFTP 上传续传由用户在桌面发起，目录差异的套接字接口继续提供只读预览。

持久记录：v2 增加 `history_list {offset,limit}`（offset 0..512，limit 1..50），返回 `{records,more}`；`history_get {record,tail_bytes}` 返回元数据与 stdout/stderr Stream，最多每路 64 KiB。`h<32位hex>` 是独立历史 ID，不接受为 job / lease / cursor。Agent 只可读取 Agent 来源且目前主机仍允许接入的记录；不存在和不可见记录统一 NOT_FOUND。运行结果新增 `record`、`record_saved`、`record_error`，记录失败不等于远端失败。重启的未结束意图为 unknown/client_restarted；数据保存范围、锁、schema 与磁盘故障语义见 ADR 0007。未改变既有 v2 方法，新命令需更新应用。

`history_log {record,stream,offset,limit}` 接受 stdout 或 stderr，单页最多 65,536 个原始字节，用 `next_offset` 读取下一页。响应同时给出实际归档字节、观察到的总量及完整性。无效 UTF-8 以替代字符表示，原始字节通过桌面保存对话框导出。所有页沿用历史可见性检查。[任务日志保留](adr/0007-task-history.md#任务日志文件)
