# 2026-11-18 安装与独立 Agent 验收

环境：Apple Silicon、macOS 27.2 (26B123)。远端为本轮新建 Debian 13 QEMU 实例，使用校验过的基础镜像、独立磁盘和密钥，仅经回环转发访问。没有接触常用 Vibe-SSH 数据、SSH 配置或远端服务器。

## AC-030 安装包与空白资料目录

使用 [install-acceptance.py](../../scripts/install-acceptance.py)，输入 preview.4 arm64 DMG、preview.5 arm64 DMG/ZIP、本地候选应用和临时 SSH 主机条目。所有发布存档匹配随版 SHA-256；挂载为只读，复制到临时 Applications，严格验证签名。Preview 5 两种存档的应用可执行文件逐字节一致。

| 操作 | 实际结果 |
| --- | --- |
| 空资料目录启动 preview.4 | 授权主机列表为空；原生应用自动创建两个 Skill 链接与 CLI；CLI 状态对应刚启动的进程 |
| preview.4 → preview.5 | 主机 JSON 内容保留，SSH 重新连接工作；新版本无旧活动空间，旧版不存在的历史从空列表开始 |
| preview.5 → 本地候选 | 版本匹配的指南可读；主机配置和新建的命令输出记录跨安装读回 |
| 冲突、迁移路径与移除 | 无关的 Claude Skill 保持原样；移动应用后一次启动修复 CLI 链接；移除应用后主机和历史保留，手动只移除自己的链接 |

边界：在现有 macOS 的空白应用资料目录中直接运行签名包，没有使用干净 macOS 系统、Finder/Gatekeeper 的首次放行流程、独立 Intel 机器或系统钥匙串迁移。上述流程通过；AC-030 的干净系统门槛仍开放。

## AC-033 外部 Agent

在临时工作目录布置相同的静态 `dist`、由候选原生应用生成的 Skill/CLI 链接。Nginx 1.26.3 由夹具预装。Agent 收到[验收任务](agent-deployment-task.md)，没有收到 SSH 密钥、命令脚本或仓库文档。远端部署与故障修改全部由 Agent 经应用 CLI 完成。

- **Codex CLI 0.167.0：静态站点通过。** 初次在不允许套接字连接的配置下发现 Skill、读取版本指南，随后停在 `hosts`。同一套接字在宿主可用，沙箱探针返回 `PermissionError`。仅对临时应用的 Unix 套接字授权后运行，没有修改全局配置。首次部署、故障和交接已完成，但第一次将宿主转发端口用于来宾监听，因此未通过宿主 URL 检查。澄清端口含义后，在新站点独立重跑：来宾与宿主均 HTTP 200，响应和产物逐字节一致；针对本站点的故意错误使 `nginx -t` 返回 1，旧页面仍可访问；恢复配置后检查通过并重载。交接后 `owner=human`，验收方使用旧租约发出无害请求，CLI 返回 `CONTROL_REVOKED`。其他站点保留。
- **Claude Code 2.1.276：发现 Skill，部署受阻。** 非交互启动事件列出 `vibe-ssh`，模型网关请求收到 `ConnectionRefused`。配置指向本地网关，独立 TCP 检查也被拒绝。没有模型工具调用、部署或交接，不更改模型和网关设置。

Codex 复验一次计 112 秒（含模型调用和启动），19 个 Shell 调用，其中 15 个包含 Vibe-SSH CLI，应用记录 10 个远端任务（9 成功、1 次故意失败）。仅作为工作流基线；没有与其他工具做配对性能测试。宿主复核响应 SHA-256：`8ba4010619d4bade5168973147db98317a32365040a277f98ee6c1c54cf4bebf`。

由验收结果补充两项产品行为：套接字权限拒绝仍返回协议 v2 的 `SERVICE_UNAVAILABLE`，但消息明确说明权限原因；静态站点指南区分远端监听和客户端转发端口。不建议 Agent 自行绕过其宿主设置。AC-033 仍需 Claude Code 的独立部署证据。

## 追加验收

Codex CLI 0.169.1 使用安装的 Compose Skill 在另一轮 Debian 13 夹具独立部署。Docker 26.1.5、Compose 2.26.1 由夹具预装。输入是可离线构建的两份静态容器产物，故障版本的健康检查指向不存在的路径。宿主 42959 转发到来宾 8080。运行完成用时 115.8 秒，11 次 Shell 调用，10 次包含 Vibe-SSH CLI。

先发布健康版本，校验容器和宿主 HTTP。第二版实际启动后报告容器 unhealthy，预期的任务失败进入历史。Codex 恢复原发布，另外用宿主 HTTP 与 Debian 的容器状态、`current` 链接核对结果。恢复后的正文和健康版一致，`current` 指向原发布。Agent 交接后空间为 human，验收方用原租约提交无害操作被拒绝为 `CONTROL_REVOKED`。

Claude Code 2.1.276 以禁用工具的最小探针复验模型连通性，API 返回 403。尚未启动独立部署。
