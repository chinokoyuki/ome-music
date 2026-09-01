# Ome Music

[English](./README.md) | 中文

Ome Music 是一个给普通用户使用的 Windows 音乐播放器。目标很简单：下载安装，双击打开，然后开始听歌。

你不需要懂 Node.js、Rust、Vite、URL、打包、命令行，也不需要自己启动开发服务器。正常使用只需要下载 Windows 安装包。

## 普通用户怎么安装

1. 打开 [GitHub Releases 页面](https://github.com/zerolyx/ome-music/releases)。
2. 下载 `Ome.Music_0.3.8_x64-setup.exe`。
3. 双击安装包，按提示安装。
4. 从桌面快捷方式或开始菜单打开 Ome Music。
5. 直接搜索歌曲、导入本地音乐，或者在设置里扫码连接网易云音乐。

当前版本还没有代码签名，Windows SmartScreen 可能会提醒风险。如果你确认安装包来自本仓库，可以点击“更多信息”，再选择“仍要运行”。

旧版本已下架或不再推荐使用。请始终从 GitHub Releases 下载最新版本安装包。

## 它能做什么

- 播放电脑里的本地音乐。
- 用你自己的网易云账号会话搜索和播放可用歌曲。
- 使用 Bilibili 作为音乐和视频氛围来源。
- 搜索和播放 QQ 音乐歌曲，支持扫码、Cookie 导入与官方窗口登录（QQ 与微信）。
- 显示封面、歌词、视频氛围和轻量弹幕。
- 把曲库和播放记录保存在本机。

Ome Music 不会绕过会员、版权、地区限制或平台访问规则。

## 截图

![Ome Music 主界面](docs/assets/screenshot-main.png)

![Ome Music 设置界面](docs/assets/screenshot-settings.png)

## 第一次打开

- 本地音乐不需要登录。
- 网易云音乐建议在“设置 > 音乐来源”里扫码登录。
- 如果会员、版权、地区或平台限制导致歌曲仍然不可播放，Ome Music 会提示不可用，不会崩溃。
- Bilibili 可以先搜索公开内容；登录后可访问账号权限内的内容。

## 隐私

- 不要把 API Key、Cookie、登录会话、本地数据库、缓存、日志提交到 GitHub。
- 本地音乐只保存路径引用，不会上传你的音乐文件。
- 网易云音乐和 Bilibili 只使用你自己的登录状态。

详见 [docs/PRIVACY.md](docs/PRIVACY.md) 和 [SECURITY.md](SECURITY.md)。

## 开发者说明

下面内容只适合想从源码构建项目的人。普通用户不需要执行这些命令。

环境要求：

- Windows 10/11
- Node.js 20 或更高版本
- Rust stable toolchain
- Microsoft Edge WebView2 Runtime

安装依赖：

```bash
npm install
```

开发模式运行：

```bash
npm run desktop
```

构建前端：

```bash
npm run build
```

构建 Windows 发布版：

```bash
npm run release:windows
```

更多开发文档：

- [构建说明](docs/BUILD.md)
- [配置说明](docs/CONFIGURATION.md)
- [排障指南](docs/TROUBLESHOOTING.md)
- [维护指南](docs/MAINTENANCE.md)
- [更新日志](docs/CHANGELOG.md)
- [贡献指南](CONTRIBUTING.md)
- [第三方声明](THIRD_PARTY_NOTICES.md)

## 开源许可

Ome Music 使用 MIT License 开源。详见 [LICENSE](LICENSE)。
