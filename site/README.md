# Vibe-SSH 产品落地页

[官网](https://vibe-ssh.intmainjhy.cn/)通过 Nginx 提供静态页面，[GitHub Pages](https://intmainjhy.github.io/Vibe-SSH/)保留镜像。源码保存在主分支的 `site/`，Pages 的发布源为 `gh-pages` 的根目录。

本地运行 `python3 -m http.server 4173 --bind 127.0.0.1 --directory site`，打开 `http://127.0.0.1:4173/`。首屏由 WebGL 着色器绘制，支持暂停；滚出首屏或切换标签页时停止绘制。公告和卡片使用轻量 CSS 动效，系统开启减少动态效果时使用静态展示。截图由 Beta 1 的界面代码和开发样例数据生成。

当前页面区分 PolyForm Strict 新源码政策与仍按 MIT 提供的 Beta 1 下载。企业版 Pro 为闭源商业版本。许可说明见根目录 LICENSING.md；不允许用更新页面文案的方式追溯更改旧包许可。

发布新版时，更新公告、版本号、两种架构的下载链接、校验文件和版本说明。用发行版 API 或发布页面核对所有资源后，从主分支执行：

```sh
pages_commit=$(git subtree split --prefix site HEAD)
git push origin "$pages_commit:refs/heads/gh-pages"
```

部署官网时上传 `index.html`、样式、脚本和 `assets/` 到新版本目录。校验文件后切换 `/srv/vibe-ssh.intmainjhy.cn/current`。Nginx 使用该路径提供页面，Certbot 使用 Webroot 定期续期。主下载入口指向 GitHub Releases，下载区同时链接 AtomGit 镜像发行版。

顶栏使用 [GitHub Octicons](https://github.com/primer/octicons) 的 GitHub 标识和 [AtomGit 站点图标](https://cdn-static.gitcode.com/static/images/logo-favicon.png) 链接各自的仓库。
