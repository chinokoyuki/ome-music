# Maintenance Guide

Ome Music should stay small, readable, and easy to release. This document is the checklist for repository maintenance.

## Release Checklist

1. Make sure `main` is green in CI (including `cargo test` on Linux + Windows and `npm run test`).
2. Update versions together:
   - `package.json`
   - `package-lock.json`
   - `src-tauri/Cargo.toml`
   - `src-tauri/Cargo.lock` (the `ome-music-player` package entry)
   - `src-tauri/resources/netease-runtime/package.json` (lockstep manifest; also regenerate its lockfile — see step 4)
   - `src-tauri/tauri.conf.json`
   - README installer references
   - `docs/BUILD.md`
   - `docs/CHANGELOG.md`
3. Run local checks:
   - `npm run build`
   - `npm run test`
   - `cargo check` from `src-tauri`
   - `cargo test --workspace` from `src-tauri`
   - `npm run lint`
   - `npm run format:check`
   - `npm run docs:check`
4. If the managed NetEase runtime version changed, regenerate the lockfile and commit it:
   - `npm install --package-lock-only --omit=dev --prefix "src-tauri/resources/netease-runtime"`
5. If the bundled Node.js runtime version changed, update both `$nodeVersion` and `$nodeSha256` in `scripts/prepare-netease-runtime.ps1` (the SHA256 is published at `https://nodejs.org/dist/v<NODE_VERSION>/SHASUMS256.txt`).
6. Open a pull request for the release maintenance patch.
7. Merge only after CI passes.
8. Run the `Release Windows Build` workflow manually first and validate the uploaded installer artifact. Manual runs must not publish a GitHub Release.
9. Create a version tag, for example `v0.4.0`, only after manual installer validation passes.
10. Confirm the tagged `Release Windows Build` workflow generates the NSIS installer and the `release-sha256-checksums` artifact.
11. Verify the published installer's SHA256 against the checksum artifact before announcing the release.
12. Download the installer from GitHub Releases and launch it on Windows before announcing the release.

## Repository Hygiene

- Do not commit release installers, portable executables, local databases, logs, cache folders, screenshots with private data, cookies, or API keys.
- Keep README files short and current.
- Keep CHANGELOG entries concise. Avoid dumping raw internal notes into release history.
- Keep workflows focused. CI should validate the project; release should publish the Windows installer.
- Optional infrastructure such as Docker images should live in a separate workflow if it returns later.

## CI Expectations

The default CI workflow should answer:

- Does Rust compile? (Linux **and** Windows — Windows covers the WebView2-only QQ Music code)
- Does Clippy pass?
- Is Rust formatting correct?
- Do Rust unit tests pass? (`cargo test --workspace`)
- Does TypeScript compile?
- Does ESLint pass?
- Does the static regression suite pass? (`npm run test`)
- Does the frontend production build pass?

The release workflow should answer:

- Can the Windows NSIS installer be produced as a manual validation artifact?
- Can a tag push publish the validated installer without creating accidental `main` releases?

## Version Policy

- Patch versions are for fixes, maintenance, and packaging stability.
- Minor versions are for user-visible feature groups.
- Do not create a release tag until the maintenance PR is merged and `main` is green.
