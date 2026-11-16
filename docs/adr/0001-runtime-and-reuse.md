# ADR 0001 原生内核与开源复用路线

日期：2026-11-16。状态：用户已接受产品路线，具体依赖组合由 M0 验证。

## 背景

Vibe-SSH 需要让外部 Codex 通过 Skill 操作服务器，同时提供日常桌面 SSH 功能。性能重点包括低开销调用、连接复用、有界输出和减少模型往返，不仅是界面渲染速度。

用户明确要求手动添加自己的服务器、不内置 Agent、能观察与接管操作。项目从空目录开始，没有必须保留的现有技术栈。

## 决定

使用 Rust/Tokio 实现原生内核与 CLI，使用 russh 处理 SSH，russh-sftp 处理文件子系统，Tauri 2 配合 React/TypeScript 和 xterm.js 实现桌面界面。通过版本化本地协议连接 CLI、界面与同一内核。

优先作为依赖使用上游库。只有出现可复现的上游缺陷且必须修复时才维护补丁或小范围 fork，避免默认承担整套客户端分支的长期维护。

## 比较过的路线

| 路线 | 优点 | 代价 | 结论 |
| --- | --- | --- | --- |
| Rust + Tauri + russh | 原生 CLI 与内核，UI/执行分层 | Rust 开发、异步状态和跨平台打包成本 | 首选，M0 验证 |
| TypeScript + Electron + ssh2 | 语言统一，桌面生态完整 | 运行时体积与进程成本需要测量 | 可行备选，不因语言直接否定 |
| Electerm 整体二开 | 传统 SSH 功能与文件管理现成 | 继承大量产品功能、状态与依赖 | 传统功能优先时重新评估 |
| Tabby 插件或二开 | 插件边界与成熟终端工作区 | 接受 Angular/Electron 和现有生命周期 | 适合外部控制实验 |
| Go 内核 | 并发和原生 CLI 可行 | 仍需选择桌面与终端整合方式 | 保留为技术备选 |
| WinkTerm 整体二开 | 外部 Agent API 与会话可观察性可参考 | 内置模型/Agent 栈与目标重合度有限 | 借鉴接口，不默认 fork |

Rust 不自动带来部署总时间提升。协议调用次数、网络、远端构建、日志量和模型决策延迟都影响最终结果。M0 或基准发现不适配时，通过新增决策调整底座，不能直接隐藏问题。

## 上游核对

| 项目 | 已核对事实 | 来源 |
| --- | --- | --- |
| russh | Rust 异步 SSH 库，Apache-2.0，有 exec、PTY 和 SFTP 示例 | [源码与文档](https://github.com/Eugeny/russh) |
| russh-sftp | 与 russh 配合的 SFTP 子系统，Apache-2.0 | [源码](https://github.com/AspectUnk/russh-sftp) |
| xterm.js | 前端终端组件，MIT，本身不是 SSH 客户端 | [说明](https://github.com/xtermjs/xterm.js) |
| ssh2 | Node.js SSH 客户端/服务端库，MIT | [源码](https://github.com/mscdex/ssh2) |
| Electerm | Electron/React 客户端，MIT，当前依赖含 SSH 与终端组件 | [依赖](https://github.com/electerm/electerm/blob/master/package.json) |
| Tabby | TypeScript/Angular/Electron；当前 SSH 源码调用 russh 绑定 | [SSH 源码](https://github.com/Eugeny/tabby/blob/master/tabby-ssh/src/session/ssh.ts) |
| SSHYard | Go/MyGo 客户端，AGPL-3.0 | [仓库](https://github.com/Licoy/sshyard) |
| WinkTerm | Python 后端，提供外部 HTTP Agent API 和 Skill | [仓库](https://github.com/Cznorth/winkterm) |
| Tauri | 支持 Web 前端与原生后端的桌面框架 | [官方架构说明](https://v2.tauri.app/concept/process-model/) |

依赖解析以 Cargo.lock 为实际版本事实，不把上游 main 分支中的示例版本当成发布锁定版本。示例代码中无条件信任主机公钥的写法不能直接用于本项目。

## 后果

需要维护 Rust 和 TypeScript 两条开发链路；协议结构需控制兼容性。CLI 可以独立提供机器入口，界面重载不必重建远端连接。任务控制、凭据和日志边界能够在内核统一测试。

首版不会同时实现多种 SSH 后端。保留模块适配边界即可，避免提前写一个没有实际用途的通用插件框架。
