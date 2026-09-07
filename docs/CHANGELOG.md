# Changelog

This project follows small, traceable releases. Keep entries short and focused on what changed for users and maintainers.

## [Unreleased]

> Status: release candidate work — v0.4.0 will be tagged only after manual
> acceptance (QQ/WeChat scan sign-in, NetEase session stability, window
> matrix, installer) and a maintainer merge of PR #15. The entries below
> describe what is already implemented and verified by automated checks.

### Added

- **QQ Music source (PR #15, by @chinoshizuyuki)**: search, playback with
  per-quality fallback (Hi-Res → Lossless → EXHigh → Higher → Standard) and
  multi-CDN candidates, lyrics + translated lyrics, playlists, liked songs,
  user profile, VIP status, QR login, Cookie import, and an official
  QQ Music WebView2 sign-in window (QQ + WeChat) whose session is validated
  and persisted entirely inside Rust (Windows Credential Manager).
- Source settings isolation: each music source (NetEase / Bilibili / QQ
  Music) is saved and enabled independently; enabling a source never resets
  the others, and signing in never silently enables a source.
- Startup only initializes *enabled* sources; disabled sources start no
  services and open no login checks.
- Compact player layout at 900px with a shared side-padding baseline;
  previously the 1040px minimum window inherited a wide fixed-column layout.

### Changed

- **QQ Music sign-in rebuilt around the official WebView (P0 fix)**: the
  app-owned official sign-in window is now the primary login — after the user
  confirms on their phone, a backend watcher collects the session via the
  native WebView2 CookieManager, bootstraps a QQ Music session from QQ account
  cookies when needed, verifies it against the QQ Music API, stores it in the
  OS keyring and closes the window automatically (no manual "Extract/Check"
  steps). A verification failure now saves nothing and reports the real
  reason; the direct ptlogin QR is demoted to an experimental Advanced entry
  (QQ servers 403 it on many real networks) with an "Use Official Sign-in"
  escape hatch, and Cookie import moved into the same Advanced block.
- NetEase service startup is single-flight (async lock + re-check), health
  checks verify the API response shape instead of any HTTP 200, and the
  bundled runtime becomes optional per user configuration.
- QQ Music credential handling hardened: QR bootstrap cookies live only in
  Rust memory, WebView2 cookie extraction is Windows-gated with a domain +
  name allowlist, logs show only presence/length/booleans, and `debug_dump`
  is test-only and removed from the command surface.
- Local library rows that point at missing files are surfaced as
  `file_missing` instead of silently failing at play time.
- SQLite runs in WAL mode with a busy timeout; legacy column migrations
  surface errors instead of swallowing them.

### Fixed

- UTF-8-safe cookie masking and login-redirect HTML extraction (no byte
  slicing panics on non-ASCII input; release builds abort on panic).
- QQ Music QR polling lifecycle: polls stop on terminal states, die with the
  settings panel, and a failed verification shows a real reason instead of a
  misleading "QR expired".
- QQ Music VIP status reports "unknown" (membership not confirmed) instead
  of claiming membership from cookie presence alone.
- Cover stability: library-row covers for Bilibili / QQ Music keep stable
  CDN URLs instead of short-lived proxy tokens; failed cover hydration
  retries instead of permanently falling back.
- Playback / UI: lyric room renders only a window around the active line;
  queue rows skip off-screen rendering; the Bilibili atmosphere label stops
  falsely reporting "Preparing" when only cover art is available; danmaku
  clears on pause; the title dialog retires when an overlay opens; the
  settings modal closes on Escape / backdrop and is keyboard-accessible.

### Security

- Known Risk recorded in `SECURITY.md`: the bundled
  `NeteaseCloudMusicApi → music-metadata → file-type` chain has no
  non-breaking fix; `npm audit fix --force` would force-downgrade to an
  unmaintained 3.x line and is therefore prohibited.
- Dependency hardening (2026-09-06): compatible-range `npm audit fix` plus a
  `vite` 6.4.3 upgrade cleared 9 of the 15 previously reported advisories in
  both the root tree and the bundled NetEase runtime tree (vite dev-server
  family, esbuild dev-server, axios 1.18.1, form-data 4.0.6, ip-address
  10.7.0, js-yaml, brace-expansion, browserslist, postcss-selector-parser).
  Two accepted risk chains remain, both recorded in `SECURITY.md`:
  the `music-metadata → file-type` chain (unchanged) and the new
  `express → qs` chain (no fix within express 4.x; local 127.0.0.1 only).

### CI

- CI now runs `cargo test` (Linux + Windows) and `npm run test`
  (static regression guards) on every push/PR; Windows job compiles the
  WebView2-only QQ Music code that Linux CI cannot reach.

## [0.3.8] - 2026-07-09

Maintenance release. No new user-facing features. Focuses on cleaning up the QQ Music settings UI and tightening CI/lint hygiene.

### Changed

- QQ Music settings panel trimmed: removed unused VIP status badge, playlist shelf, test-connection / check-VIP / debug buttons, and the associated dead event handlers and state. The remaining buttons were re-laid into a uniform 2-column grid matching the NetEase and Bilibili cards.
- `webview2-com` and `windows` crate dependencies moved under `[target.'cfg(windows)'.dependencies]` so Linux CI no longer attempts to compile Windows-only WebView2 cookie-extraction code.
- Bumped version to 0.3.8.

### Fixed

- Resolved all Rust `unused variable` / `unused assignment` warnings and clippy errors (`needless-range-loop`, `manual-pattern-char-comparison`, `manual-strip`) so `cargo clippy --workspace -- -D warnings` passes clean.
- Resolved ESLint `@typescript-eslint/no-unused-vars` errors in `LyricsSourceMenu.tsx` and `ProviderSettingsPanel.tsx`.
- docs:check now passes: README, README.zh-CN, BUILD.md, and CHANGELOG.md all reference the canonical 0.3.8 version.

## [0.3.7] - 2026-06-30

Stability release. No new features. Focuses on fixing the NetEase login/playback state mismatch and overlay/lyrics/danmaku/UI issues exposed during v0.3.6 acceptance, plus unblocking local music playback from user-picked folders outside `$HOME/Music`.

### Added

- NetEase VIP status now exposes `membershipKnown` so the Settings panel can distinguish "membership unknown (API unreachable)" from "Non-member (API confirmed)". Without this flag, a transient `/vip/info` failure surfaced as `Non-member` and misled users into thinking their membership had lapsed.
- TopSearch NetEase click only dismisses the search popover on playback success. On failure the results stay visible with a banner pointing to the playback notice, so the user can pick another track instead of having the popover silently close.
- Runtime asset scope grant: user-picked music folders (secondary drives, USB sticks, Desktop, etc.) are now authorized at runtime via `asset_protocol_scope().allow_directory()` both at import time and at startup restore, so playback no longer fails for folders outside the static `$HOME/Music` scope. The static `tauri.conf.json` scope stays locked down — `$HOME/**` and whole-disk access are never granted.
- Managed NetEase runtime manifest bumped in lockstep with the app version (0.3.6 → 0.3.7) for traceability.

### Fixed

- Queue / Settings / More overlays are now mutually exclusive via a unified `activeOverlay` state (`none` | `queue` | `settings` | `more` | `source` | `lyricsTools`). Opening one closes the others; Esc and click-outside close the current overlay. TopSearch opening Settings also closes Queue.
- Lyrics Room spatial stage: per-line `translate3d` + `rotate` + `scale` form an arc layout instead of flat scatter; line-height and padding-block increased so the active line is no longer clipped; transforms moved off the outer button to an inner span to avoid blur/transform cropping. The container keeps scroll while each line allows `overflow: visible`. Visual presets: Calm / Arc Room / Stage / Dream.
- Danmaku corridor height increased and `line-height: 1.35` so large comments render fully instead of being cropped; new `arc` and `mixed` motion styles with arc path, gentle y-shift, fade in/out and light scale, so the default motion reads as emotional floating rather than a marquee.
- Main UI subtraction: OmeRadio and DjCurator panels dimmed to opacity 45/60 with hover restore so they no longer compete with the cover/lyrics focus; playback speed chip hidden unless `speed ≠ 1`; Queue / Settings / More / Lyrics tools all behind the unified overlay manager. No features removed; only layering adjusted.
- NetEase token read now retries 3× with 80ms backoff, and cookie merge during login-status polling protects `MUSIC_U` so a stale refresh no longer overwrites a valid session cookie.
- Real-time login propagation: `onNetEaseLoginChanged` / `onBilibiliLoginChanged` callbacks now refresh the App top-level `sourceLoginStatus`, Settings panel, and VIP state immediately on QR scan success — users no longer need to close and reopen Settings to see "Signed in".
- "NetEase source ready" label corrected to "本地网易云服务已启动" so users no longer conflate service-ready with account-signed-in.

### Fixed (final v0.3.7 re-release)

These fixes ship in the re-cut v0.3.7 tag after acceptance found the first cut still had a real-world P0 and several P1 polish gaps.

- NetEase "signed in but playback says Sign in needed" root cause fixed. The OS keyring can transiently fail to read (Windows Credential Manager restart, Linux Secret Service contention) and `read_netease_token` then returned `None`, which the playback path misclassified as `not_logged_in` — while the App shell's cached `sourceLoginStatus` still showed "Signed in" from an earlier snapshot. Three changes close the gap: (1) `save_netease_token` now mirrors the cookie to the legacy plaintext fallback file instead of deleting it after a successful keyring write, so a keyring read failure degrades gracefully; (2) `fetch_netease_playable_url_with_level` re-reads the token at the playback entry point (`config.token.or_else(read_netease_token)`) so a transient resolve-time miss no longer becomes a "not signed in" verdict; (3) the Settings panel Login line now distinguishes "Signed in" / "Session expired" / "Not signed in (cookie stored)" / "Signed out" so source-ready never masquerades as signed-in.
- Playback failure now refreshes the App shell's cached `sourceLoginStatus` when the reason is `not_logged_in` / `cookie_missing`, so the Settings page and the playback notice can no longer disagree at the same time.
- Overlay exclusivity hardened with a `useEffect` safety net: if Queue and Settings ever end up open in the same render, the most recently opened one wins and the other is forced off — guaranteeing they can never visually stack.
- Subtitle / danmaku bottom clipping fixed: `.ambient-danmaku-line` changed from `overflow: hidden` to `overflow: visible` (long lines kept in check by `max-width` + `white-space: nowrap`), and lyric lines bumped to `leading-[1.18]` + `min-h-[8.5rem]` + `py-3` so descenders and the text glow are never cropped at larger sizes or during arc vertical motion.
- Curved motion refined: default danmaku `motionStyle` is now `arc` (was `drift`); the `arc` / `ambient-arc` keyframes were retuned from a sharp "dip then rise" to a soft sinusoidal "rise → soft peak → settle" with rotation following the curve tangent, so motion reads as emotion in the air rather than a rail. Lyric-room `yShift` (7→9) and `rotate` (0.7→0.95) nudged so the spatial arc reads more strongly while staying calm.
- Main UI subtraction extended: TopSearch and the top-right Quick Settings trigger now rest faint (opacity ~55% / ~45%) and brighten only on hover/focus, with lighter shadow/border on the trigger. Position, size and behavior are unchanged, so discoverability stays intact while the cover/lyrics focal point stops competing with always-on chrome.

### Notes

- This is an unsigned development build; Windows SmartScreen may warn.
- Queue / tracks full separation is deferred to a follow-up refactor PR: the default play queue still rides on the `tracks` array, but `clearQueue` already preserves the library and search results, and `agentQueue` for radio/curator is already independent. Full separation would touch `playAdjacentTrack` / `rebuildShuffleOrder` / `currentIndex` and is out of scope for a stability release.
- The unified `NetEaseAuthPlaybackSnapshot` diagnostic structure and the 11-reason playback-failure enumeration (`not_logged_in` / `cookie_missing` / `cookie_expired` / `vip_required` / `trial_only` / `no_copyright` / `region_restricted` / `url_null` / `api_failed` / `media_proxy_failed` / `audio_play_failed`) are deferred to a follow-up PR; this release ships the minimal UX fixes only.
- Bumped version to 0.3.7.

## [0.3.6] - 2026-06-30

Older release. Superseded by 0.3.7. Older releases are deprecated and no longer recommended.

### Added

- Player Dock functional keys: unified `PlaybackMode` cycle (curator → loop → repeat-one → shuffle), like button on the cover with local persistence + Taste Signal, and a light glass "More" menu.
- Like / Less like this taste signals feed the local listening-memory pipeline.
- Playback speed controls: explicit 6-option selection grid (0.5 / 0.75 / 1 / 1.25 / 1.5 / 2x) in the More menu, persisted to `ome.playback.speed` and surviving restarts and track switches.
- Queue Drawer: right-side slide-over glass panel with click-to-play, current-track highlight + equalizer animation, row-level like/remove (hover) and list-level like-all / clear / recommend-similar toggle.
- Lyrics Room alpha: spatial stage with three depth tiers (current / nearby / far), radial drift, micro-rotation, breathing animation, click-a-line-to-seek, and a soft no-lyrics empty state.
- Hidden Lyric Tools: bottom-right hotspot cluster that appears only on hover; Translation reveals translated lyrics (data-permitting); Romanization / Word-by-word stay disabled until a data pipeline exists.
- Playlist Shelf: card grid replacing the old Listening Memory table, per-playlist progress state machine (reading / imported locally N · relative time / failed), persisted local import records, and heuristic identification of the NetEase "我喜欢的音乐" entry.

### Fixed

- Restored stable NetEase search, cover display, login-state detection, and playback pipeline (from PR #12).
- Restored Bilibili replay hydration for cover, video atmosphere, audio, and danmaku (from PR #12).
- Prevented playback progress and lyric position from resetting on metadata-only liked updates: `reloadLyrics` / `importLyrics` now read `currentTrackRef.current` (latest-ref stable pattern), and the track-switch reset effect is keyed to `currentTrackId` (a real track switch) instead of `currentTrack` object identity.
- Queue safety: Clear / Remove no longer touch the overloaded `tracks` array (local library + search results + imports), so clearing or removing from the queue never wipes the visible library or search results.
- Hidden half-finished More-menu stubs (Add to playlist, View source) so the immersive player no longer parades disabled entries.

### Notes

- This is an unsigned development build; Windows SmartScreen may warn.
- The Windows installer bundles the managed NetEase runtime; users do not need Node.js, npm, Rust, or command line tools.
- Playlist Shelf records a one-way local import from NetEase, NOT a two-way cloud sync.
- Bumped version to 0.3.6.

## [0.3.5] - 2026-06-29

### Fixed

- Fixed Windows installer failure (`Error opening file for writing ... node.exe`) when an old version of Ome Music or its bundled managed runtime was still running.
- Added NSIS pre-install hook to gracefully stop old Ome Music and bundled node.exe processes before writing files, without killing unrelated system Node.js processes.
- Stored the managed NetEase runtime child process handle in AppState so it is reused instead of spawning duplicates, and is killed on app exit to prevent orphaned node.exe.
- Added NetEase service startup status with staged progress (checking runtime → checking API → starting service → waiting for health → ready) so users see clear feedback instead of silent search failures.
- Added a search gate that ensures the NetEase service is ready before searching, with automatic retry and clear error messages.
- Removed dangerous post-uninstall data deletion; uninstall now preserves user data (library, login, settings, caches, keyring) by default. Upgrade/reinstall no longer loses data.

### Security

- Narrowed the Tauri asset protocol scope to remove default access to the Downloads directory.
- Added path authorization to `scan_music_directory` so arbitrary front-end paths cannot be scanned.

### Maintenance

- Clarified in BUILD and CONFIGURATION docs that the Windows installer is a complete ready-to-run environment and does not require npm, Node.js, or compilation for normal users.
- Hardened NetEase and Bilibili QR login polling with client-side max-life timeouts and consecutive-error thresholds to stop infinite retry loops.
- Added SHA256 verification for the bundled Node.js runtime in the release workflow and switched to `npm ci --omit=dev` for reproducible managed-runtime installs.
- Added a lightweight `docs:check` script and CI job for documentation consistency.
- Bumped version to 0.3.5.

## [0.3.4] - 2026-06-29

### Fixed

- Merged NetEase session cookies from both the response body and `Set-Cookie` header so QR login no longer loses session fragments.
- Disabled Node.js runtime probing in release builds; the installer uses the bundled managed runtime instead of searching for `npx` on the user's machine.
- Added retry logic to NetEase login status checks to tolerate transient upstream failures.
- Improved NetEase playback error classification with bilingual keyword detection.
- Restored the initial page splash-screen fix (inline body background colour) that was lost during the main squash.
- Hid misleading Bilibili password and SMS login buttons; Bilibili account sign-in is routed through Secure Web Login.
- Added a refactor TODO for splitting `App.tsx` into focused hooks.

## [0.3.3] - 2026-06-28

### Fixed

- Bundled a managed NetEase Cloud Music runtime into the Windows installer so normal users do not need to install Node.js or configure an API URL.
- Made the NetEase source start through the bundled runtime first, then fall back to the development package only when running from source.
- Enabled the NetEase source by default for new installs.
- Reduced the search popover flicker by opening results only after the user types a query.
- Rewrote NetEase setup and troubleshooting docs from a normal-user perspective.

## [0.3.2] - 2026-06-28

### Maintenance

- Cleaned up the GitHub CI workflow into two focused jobs: Rust checks and frontend checks.
- Cleaned up the Windows release workflow so installer publishing is no longer blocked by optional Docker image jobs.
- Rewrote the English and Chinese README files to remove mojibake, stale installer names, and outdated release notes.
- Added a maintenance guide for release hygiene, CI expectations, and repository cleanup rules.
- Updated version references to `0.3.2`.

### Fixed

- Normalized NetEase session cookies before saving and reading them, reducing cases where QR login appeared successful but playback requests behaved as signed out.
- Avoided classifying a valid member playback URL as preview-only when the signed-in account has an active membership.
- Prevented page-level vertical scrolling in the immersive player shell.
- Disabled window resizing to avoid stretched player layouts.
- Removed the startup onboarding pop-in from the normal launch path.
- Removed visible shuffle/repeat controls from the main player footer to keep the playback UI minimal.

## [0.3.1] - 2026-06-28

### Fixed

- Improved playback queue behavior around shuffle, previous track, repeat, and expired source URLs.
- Improved NetEase and Bilibili QR login states and timeout handling.
- Improved search result pagination and disabled-source messaging.
- Added lint, formatting, and CI checks.

## [0.3.0] - 2026-06-27

### Added

- Added first-run onboarding.
- Added Bilibili QR login improvements.
- Added NetEase QR login fixes around cookie retrieval.

## [0.2.0] - 2026-06-27

### Added

- Added Node.js environment detection for the local NetEaseCloudMusicApi service.
- Added clearer configuration and troubleshooting documentation for NetEase source setup.

## [0.1.0] - 2026-06-27

### Added

- Initial public Windows release.
- Local music library, NetEase Cloud Music source, Bilibili source, lyrics, danmaku, settings, and release packaging.
