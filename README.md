# Ome Music

English | [中文](./README.zh-CN.md)

Ome Music is a lightweight Windows music player built for people who just want to open the app and listen.

You do not need to know Node.js, Rust, Vite, URLs, packaging, or command lines to use Ome Music. Download the Windows setup file, install it, open Ome Music, and connect your music sources from the app.

## For Normal Users

1. Open the [GitHub Releases page](https://github.com/zerolyx/ome-music/releases).
2. Download `Ome.Music_0.3.8_x64-setup.exe`.
3. Double-click the setup file and follow the installer.
4. Open Ome Music from the Start Menu or desktop shortcut.
5. Search a song, import local music, or scan the QR code in Settings to connect NetEase Cloud Music.

The current build is unsigned, so Windows SmartScreen may show a warning. Choose **More info** and **Run anyway** only if you downloaded it from this repository.

Older releases are deprecated and no longer recommended. Always download the latest version from the Releases page.

## What You Can Do

- Play local music from your computer.
- Search and play available NetEase Cloud Music tracks with your own account session.
- Use Bilibili as a music and video-atmosphere source.
- Search and play QQ Music tracks with QR, Cookie, or official WebView2 sign-in (QQ and WeChat).
- View covers, lyrics, video atmosphere, and gentle danmaku.
- Keep local listening data on your device.

Ome Music does not bypass memberships, copyright, region restrictions, or platform access rules.

## Screenshots

![Ome Music main screenshot](docs/assets/screenshot-main.png)

![Ome Music settings screenshot](docs/assets/screenshot-settings.png)

## First Launch

- Local music works without signing in.
- NetEase Cloud Music works best after QR login in **Settings > Music Sources**.
- If a member-only or restricted song still cannot play, Ome Music will show it as unavailable instead of crashing.
- Bilibili can be used for public content first; sign-in unlocks account-only access where available.

## Privacy

- API keys, cookies, sessions, local databases, caches, and logs should never be committed to GitHub.
- Local music files are referenced by path and are not uploaded by Ome Music.
- NetEase Cloud Music and Bilibili use your own session only.

See [docs/PRIVACY.md](docs/PRIVACY.md) and [SECURITY.md](SECURITY.md).

## For Developers

Use this section only if you want to build Ome Music from source.

Requirements:

- Windows 10/11
- Node.js 20 or later
- Rust stable toolchain
- Microsoft Edge WebView2 Runtime

Install dependencies:

```bash
npm install
```

Run the desktop app in development:

```bash
npm run desktop
```

Build the frontend:

```bash
npm run build
```

Build the Windows release:

```bash
npm run release:windows
```

Developer notes live in:

- [Build Guide](docs/BUILD.md)
- [Configuration](docs/CONFIGURATION.md)
- [Troubleshooting](docs/TROUBLESHOOTING.md)
- [Maintenance Guide](docs/MAINTENANCE.md)
- [Changelog](docs/CHANGELOG.md)
- [Contributing](CONTRIBUTING.md)
- [Third-party Notices](THIRD_PARTY_NOTICES.md)

## License

Ome Music is released under the MIT License. See [LICENSE](LICENSE).
