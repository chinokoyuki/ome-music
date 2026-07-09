# Build Guide

## Requirements

- Windows 10/11
- Node.js 20 or later
- Rust stable toolchain
- Microsoft Edge WebView2 Runtime

## Install Dependencies

```bash
npm install
```

## Frontend Build

```bash
npm run build
```

## Desktop Development

```bash
npm run desktop
```

This starts the Vite development server and launches the Tauri desktop app.

## Windows Release

```bash
npm run release:windows
```

Expected outputs:

- `src-tauri/target/release/ome-music-player.exe`
- `src-tauri/target/release/bundle/nsis/Ome.Music_0.3.8_x64-setup.exe`

The release executable and installer should not require `npm`, `cargo`, Vite, or a development server on the user's machine.

## Installer Contents and User Data

The Windows installer bundles a complete, ready-to-run environment. Normal users do not need to compile, run npm install, or install Node.js. The managed NetEase runtime (node.exe + NeteaseCloudMusicApi) is packaged at build time and shipped inside the installer.

Uninstalling Ome Music preserves user data (library, login sessions, settings, caches) by default. To fully clean up, users can manually delete `%APPDATA%\com.ome.music` and `%LOCALAPPDATA%\com.ome.music`.

## GitHub Release Build

The `Release Windows Build` workflow publishes a GitHub Release only when a tag
matching `v*` is pushed. Manual `workflow_dispatch` runs are for validation
only: they build the Windows installer and upload it as a workflow artifact, but
they do not create or update a GitHub Release.

Example:

```bash
git tag v0.3.8
git push origin v0.3.8
```

The tagged workflow should create a GitHub Release and upload the Windows NSIS installer.
Use the manual workflow first when validating a re-cut or failed release candidate.

### Managed NetEase runtime supply chain

The Windows installer bundles a managed Node.js + `NeteaseCloudMusicApi` runtime. The release workflow hardens this supply chain:

- The Node.js archive is pinned to an exact version and verified against a committed SHA256 in `scripts/prepare-netease-runtime.ps1` before extraction.
- The `NeteaseCloudMusicApi` version is pinned exactly (no caret) in `src-tauri/resources/netease-runtime/package.json`.
- The managed runtime is installed with `npm ci --omit=dev` from the committed `package-lock.json`, so every release resolves the same transitive dependency tree.
- SHA256 checksums for every installer artifact are written to the workflow log and uploaded as the `release-sha256-checksums` artifact.

When bumping the Node.js runtime:

1. Update `$nodeVersion` and `$nodeSha256` together in `scripts/prepare-netease-runtime.ps1`. The hash is published at `https://nodejs.org/dist/v<NODE_VERSION>/SHASUMS256.txt`.
2. When bumping `NeteaseCloudMusicApi`, update the pinned version in `src-tauri/resources/netease-runtime/package.json` and regenerate the lockfile locally with:

   ```bash
   npm install --package-lock-only --omit=dev --prefix "src-tauri/resources/netease-runtime"
   ```

3. Commit both `package.json` and `package-lock.json`. Never commit the resulting `node_modules/`.

## Do Not Commit

- `node_modules/`
- `dist/`
- `src-tauri/target/`
- `release/`
- installers or portable executables
- logs, caches, databases, screenshots with private data
- API keys, cookies, sessions, or tokens
