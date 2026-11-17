# 依赖许可文本

Apache-2.0.txt 为 Apache License 2.0 原文，仅供相应依赖的发行声明使用，不代表 Vibe-SSH 自身采用此许可。

来源：russh 上游提交 b33092aad1e51c66258a12e8b83fc473a68d9d34 的 LICENSE.txt（https://github.com/warp-tech/russh/blob/b33092aad1e51c66258a12e8b83fc473a68d9d34/LICENSE.txt）。部分 Apache-2.0 依赖归档不含许可，且 Cargo 记录的上游提交不可访问时，用此标准文本补齐，并在发行清单保留依赖名称、版本与来源。

MPL-2.0.txt 来源：Mozilla 官方 https://www.mozilla.org/media/MPL/2.0/index.txt 。发行清单为每个依赖提供版本和源代码入口（crates.io），其中包含 MPL 覆盖的 selectors 源码；这是依赖的许可，不改变 Vibe-SSH 自身许可。

其余 upstream-* 文本来自 notices.json 对应的 Cargo VCS 提交；文件名按内容 SHA-256 去重，打包时验证摘要并复制到对应依赖目录。
