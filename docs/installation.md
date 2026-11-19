# 安装 Vibe-SSH macOS Beta

Vibe-SSH 是日常 SSH 客户端，可由外部 Agent 通过 Skill 调用。当前提供 macOS 安装包；Linux 是已验证的远端服务器系统，不代表已有 Linux 桌面安装包。

1. 按芯片选择 `macos-arm64`（Apple 芯片）或 `macos-x86_64`（Intel）。在 [Beta 发布页](https://github.com/IntMainJhy/Vibe-SSH/releases/tag/v0.2.0-beta.1)下载 DMG 或 ZIP，并对照同页 `SHA256SUMS.txt` 校验。
2. 打开 DMG，把 `Vibe-SSH.app` 拖到“应用程序”；ZIP 解压后也应先移入“应用程序”。从该位置首次启动，避免在临时解压目录长期运行。
3. Beta 使用临时签名，尚无 Developer ID 签名与 Apple 公证。其他 Mac 可能拦截打开；核对来源和校验值后，按系统“隐私与安全性”中的“仍要打开”流程处理。不要关闭系统整体安全检查。正式分发仍待开发者证书与公证。
4. 添加服务器，填写地址、账号与认证方式，获取指纹并通过可信渠道核对后保存。默认不向 Agent 开启主机接入；需要使用时在服务器设置中逐台开启。

在「服务器 → 导入配置」预览 OpenSSH 主机，再逐台核对并保存。「服务器设置」支持本地 SSH 密钥代理、已保存的 SSH 跳板和免认证 HTTP CONNECT 或 SOCKS5 代理。个人会话支持固定本地、动态 SOCKS5 和远端 TCP 转发。关闭会话会释放转发规则。

应用每次启动自动同步内嵌版本的 Skill 到 `~/.agents/skills/vibe-ssh`、`~/.claude/skills/vibe-ssh`，并创建 `~/.local/bin/vibe-ssh` 命令链接。无需在界面安装 Skill。无关同名条目会保留并在侧栏提示，不修改 shell 配置。若 Agent 的 PATH 不含该目录，可运行 `"$HOME/.local/bin/vibe-ssh" skill`。移动应用后重新启动即可修复属于 Vibe-SSH 的旧链接。

如果应用已运行、终端里的 `vibe-ssh status` 成功，但 Agent 中的同一命令连接失败，请检查宿主能否访问 `~/.vibe-ssh/agent.sock`。如宿主支持单路径授权，使用解析后的绝对路径，仅允许这个套接字；应用不会替用户修改沙箱。模型网关无响应时，Skill 仍可能已安装，需单独核对 Agent 的模型连接。

文件页可从 SFTP 根目录或账号主目录进入。单文件和文件夹都支持上传、下载与手动续传。同名文件不会静默覆盖。监控页显示 Linux 的 CPU、内存和挂载点用量。Agent 运行时可切换空间、查看输出或接管。

卸载时先退出应用，再把应用移入废纸篓。主机配置位于 `~/.vibe-ssh/hosts.json`，密码/口令位于系统钥匙串的 `io.github.intmainjhy.vibe-ssh` 服务；卸载应用不自动删除这些资料。按需移除配置目录和该服务对应条目，以及属于 Vibe-SSH 的 Skill/CLI 链接；不要删除其他工具的同名内容。

退出旧版本后替换应用并重新打开，主机设置、钥匙串条目与历史继续使用。历史数据库仍为 schema 1，已保存的命令和输出尾部可读。新任务另有受配额限制的原始输出归档。重启前未完成任务显示为结果未知，续传需核对路径并手动恢复。见[历史保留范围](adr/0007-task-history.md)。

有关 Beta 的验收范围和已知限制，见[发布说明](releases/0.2.0-beta.1.md)。使用过程中遇到问题，可提交[测试反馈](https://github.com/IntMainJhy/Vibe-SSH/issues/new/choose)。
