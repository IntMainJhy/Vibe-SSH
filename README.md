# Vibe-SSH 产品落地页

[公开页面](https://intmainjhy.github.io/Vibe-SSH/)由 GitHub Pages 托管，发布源为 `gh-pages` 的根目录。源码保存在主分支的 `site/`。

本地运行 `python3 -m http.server 4173 --bind 127.0.0.1 --directory site`，打开 `http://127.0.0.1:4173/`。首屏由 WebGL 着色器绘制，支持暂停；滚出首屏或切换标签页时停止绘制。公告和卡片使用轻量 CSS 动效，系统开启减少动态效果时使用静态展示。截图由 Beta 1 的界面代码和开发样例数据生成。

发布新版时，更新公告、版本号、两种架构的下载链接、校验文件和版本说明。用发行版 API 或发布页面核对所有资源后，从主分支执行：

```sh
pages_commit=$(git subtree split --prefix site HEAD)
git push origin "$pages_commit:refs/heads/gh-pages"
```

页面由 GitHub Pages 提供静态托管。主下载入口指向 GitHub Releases，下载区同时链接 AtomGit 镜像发行版。

顶栏使用 [GitHub Octicons](https://github.com/primer/octicons) 的 GitHub 标识和 [AtomGit 站点图标](https://cdn-static.gitcode.com/static/images/logo-favicon.png) 链接各自的仓库。
