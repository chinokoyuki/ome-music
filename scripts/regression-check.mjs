import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";

const read = (path) => readFile(new URL(`../${path}`, import.meta.url), "utf8");
const expectMatch = (content, pattern, message) => assert.ok(pattern.test(content), message);
const expectNoMatch = (content, pattern, message) => assert.ok(!pattern.test(content), message);

const [
  rustLib,
  qqmusic,
  provider,
  settings,
  quickSettings,
  topSearch,
  queueDrawer,
  nowPlaying,
  lyricsResolver,
  globalStyles,
  app,
] = await Promise.all([
  read("src-tauri/src/lib.rs"),
  read("src-tauri/src/qqmusic.rs"),
  read("src/features/musicSources/provider.ts"),
  read("src/components/ProviderSettingsPanel.tsx"),
  read("src/components/LyricsSourceMenu.tsx"),
  read("src/components/TopSearch.tsx"),
  read("src/components/QueueDrawer.tsx"),
  read("src/components/NowPlayingHero.tsx"),
  read("src/features/lyrics/lyricsResolver.ts"),
  read("src/styles/globals.css"),
  read("src/App.tsx"),
]);

expectNoMatch(
  rustLib,
  /generate_handler!\[[\s\S]*qqmusic_debug_dump/,
  "production Tauri commands must not expose QQ Music credential diagnostics",
);
expectNoMatch(
  provider,
  /debugDump\s*\(/,
  "the frontend provider must not expose credential diagnostics",
);
expectMatch(provider, /credentialPresent:\s*boolean/, "credential presence must be explicit");
expectMatch(provider, /status:\s*QQMusicAuthState/, "auth state must be explicit");
expectMatch(rustLib, /QQMusicAuthState::Authenticated/, "verified sessions must be explicit");
expectMatch(qqmusic, /QQMusicAuthState::Unknown/, "unverified sessions must stay unknown");
expectNoMatch(
  qqmusic,
  /return Ok\(\(nickname\.to_string\(\),\s*avatar_url\.to_string\(\)\)\)/,
  "session verification must return uin and nickname in the declared order",
);
expectNoMatch(
  rustLib,
  /logged_in:\s*qqmusic_credential_is_complete/,
  "credential shape must never be treated as authenticated state",
);
expectMatch(settings, /describeQQMusicAuthState/, "settings must distinguish auth states");
expectMatch(
  quickSettings,
  /qqmusicLoginStatus\?\.credentialPresent/,
  "quick settings must distinguish saved credentials",
);
expectMatch(
  qqmusic,
  /Only official QQ Music HTTPS endpoints are allowed/,
  "QQ Music API endpoints must be allowlisted",
);
expectMatch(rustLib, /is_safe_remote_media_url/, "media proxy URLs must be validated");
assert.equal(
  [...rustLib.matchAll(/\bsave_qqmusic_token\(/g)].length,
  1,
  "QQ Music credentials must be persisted through one Rust boundary",
);
expectMatch(
  topSearch,
  /const started = await onPlayQQMusic\(song\);[\s\S]*if \(started\) \{[\s\S]*setOpen\(false\)/,
  "QQ Music search must stay open when playback fails",
);
expectMatch(
  topSearch,
  /<ArtworkImage[\s\S]*source="qqmusic"/,
  "QQ Music search covers must use ArtworkImage",
);
expectMatch(queueDrawer, /case "qqmusic":[\s\S]*return "QQ音乐"/, "queue must label QQ Music");
expectMatch(queueDrawer, /<ArtworkImage/, "queue covers must use ArtworkImage");

const globalSettingsSave = settings.match(
  /const save = async \(\) => \{[\s\S]*?\n  const testSource = async/,
)?.[0];
assert.ok(globalSettingsSave, "the curator save boundary must remain identifiable");
expectNoMatch(
  globalSettingsSave,
  /save(?:SourceDraft|BilibiliDraft|QQMusicDraft)/,
  "saving curator settings must not write any music-source settings",
);
expectMatch(
  settings,
  /getNeteaseSourceConfig\(\),\s*getBilibiliSourceConfig\(\),\s*getQQMusicSourceConfig\(\)/,
  "settings must wait for every source config before becoming interactive",
);
expectNoMatch(
  settings,
  /setQQMusicEnabled\(true\)/,
  "authentication must not silently enable QQ Music",
);

expectMatch(
  topSearch,
  /getMusicSourceAvailability/,
  "search must read one authoritative source-availability snapshot",
);
expectNoMatch(topSearch, /SourceDisabledHint/, "disabled sources must not render search groups");
expectNoMatch(topSearch, /onOpenSettings/, "search must never open settings on the user's behalf");
expectMatch(
  topSearch,
  /query\.trim\(\)\.length >= 2 && neteaseEnabled/,
  "NetEase results must be gated by its enabled state",
);
expectMatch(
  topSearch,
  /query\.trim\(\)\.length >= 2 && bilibiliEnabled/,
  "Bilibili results must be gated by its enabled state",
);
expectMatch(
  topSearch,
  /query\.trim\(\)\.length >= 2 && qqmusicEnabled/,
  "QQ Music results must be gated by its enabled state",
);

expectNoMatch(rustLib, /document\.title/, "credentials must never travel through window titles");
expectNoMatch(rustLib, /\[0u8;\s*8192\]/, "credential import must not use a fixed 8 KB buffer");
expectNoMatch(
  provider,
  /extractWebviewCookie/,
  "QQ Music credentials must never be returned to React",
);
expectNoMatch(
  provider,
  /interface QQMusicQrLogin \{[\s\S]*cookies:/,
  "QQ Music QR bootstrap credentials must remain in Rust",
);
expectMatch(
  settings,
  /微信扫码 \/ WeChat QR/,
  "QQ Music settings must expose the official WeChat sign-in path",
);
expectNoMatch(
  settings,
  /openSecureWebLogin\("qqmusic"\)/,
  "QQ Music must not claim that a system-browser session can be imported automatically",
);
expectMatch(
  rustLib,
  /qqmusic_qr_sessions:\s*Mutex<HashMap<String, String>>/,
  "QQ Music QR session credentials must stay in Rust memory",
);
expectMatch(
  rustLib,
  /import_qqmusic_webview_session/,
  "WebView session validation and persistence must remain atomic in Rust",
);
expectMatch(
  rustLib,
  /trusted official WebView session saved[\s\S]*status: QQMusicAuthState::CredentialPresent/,
  "a complete trusted WebView session must remain explicitly credential-present when legacy verification is unavailable",
);
expectMatch(
  rustLib,
  /let uri = HSTRING::from\(""\)/,
  "QQ Music WebView import must read the isolated profile before applying the cookie allowlist",
);
expectMatch(
  qqmusic,
  /"wxuin"[\s\S]*"wxopenid"[\s\S]*"wxunionid"[\s\S]*"wxrefresh_token"/,
  "QQ Music credential validation must recognize official WeChat session identity fields",
);
expectMatch(
  qqmusic,
  /request_qqmusic_json_body[\s\S]*\.post\(trusted_url\)[\s\S]*music\.vkey\.GetVkey[\s\S]*UrlGetVkey/,
  "QQ Music playback must prefer the modern JSON POST UrlGetVkey flow",
);
expectMatch(
  qqmusic,
  /music\.audioCdnDispatch\.cdnDispatch[\s\S]*GetCdnDispatch/,
  "QQ Music playback must use the current CDN dispatch response",
);
expectNoMatch(
  qqmusic,
  /session verification succeeded via account asset probe/,
  "public profile assets must never be treated as proof of authenticated playback",
);
expectMatch(
  qqmusic,
  /StatusCode::FORBIDDEN[\s\S]*status: "failed"/,
  "a forbidden direct QR endpoint must stop polling and guide the user to official sign-in",
);
expectMatch(
  qqmusic,
  /fn build_qqmusic_comm[\s\S]*"g_tk_new_20200303": g_tk_new[\s\S]*"authst": authst/,
  "authenticated QQ Music requests must carry the Web session signature in comm",
);

expectMatch(
  rustLib,
  /managed_netease_start_lock:\s*Arc<tokio::sync::Mutex<\(\)>>/,
  "NetEase startup must have a single-flight lock",
);
expectMatch(
  rustLib,
  /let _startup_guard = start_lock\.lock\(\)\.await/,
  "NetEase process startup must be serialized",
);
expectMatch(
  rustLib,
  /None => NeteaseSourceConfigDto \{\s*enabled: false/,
  "NetEase must be opt-in for a new profile",
);
expectMatch(
  app,
  /getMusicSourceAvailability\(\)[\s\S]*if \(availability\.netease\)/,
  "startup must initialize only enabled providers",
);

expectNoMatch(
  nowPlaying,
  /md:grid-cols-\[minmax\(340px,420px\)_minmax\(560px,1fr\)\]/,
  "the minimum window must not inherit the wide fixed-column layout",
);
expectMatch(
  globalStyles,
  /@media \(min-width: 900px\)[\s\S]*grid-template-columns: minmax\(250px, 32vw\) minmax\(0, 1fr\)/,
  "the minimum window must use the compact player grid",
);
expectNoMatch(
  nowPlaying,
  /now-playing-stage[^\n]*grid-cols-1/,
  "the player stage must not override its responsive columns with a permanent one-column utility",
);
expectMatch(
  globalStyles,
  /--player-side-padding:[\s\S]*\.now-playing-stage[\s\S]*padding-inline: var\(--player-side-padding\)[\s\S]*\.player-dock-controls[\s\S]*left: var\(--player-side-padding\)/,
  "the artwork column and player controls must share one horizontal baseline",
);
expectNoMatch(
  await read("src/components/PlayerControls.tsx"),
  /player-dock-controls[^\n]*left-\[/,
  "PlayerControls must not override the shared player baseline with a local left offset",
);
expectMatch(
  lyricsResolver,
  /isLyricMetadataLine\(text, startTime, track\)/,
  "timed title and production-credit headers must not enter the lyric stage",
);
expectMatch(
  lyricsResolver,
  /let result = -1/,
  "lyrics must remain inactive until playback reaches the first real line",
);

// ── v0.4.0 hardening guards ────────────────────────────────────────────

// 1. QR confirmation failures must be terminal: Rust captures the import
// error as a `failed` status (instead of `?` short-circuiting before session
// cleanup), and the frontend stops polling on failed/timeout/expired.
expectMatch(
  rustLib,
  /return Ok\(QQMusicQrCheckPublicDto \{\s*status: "failed"\.to_string\(\),\s*message: Some\(error\)/,
  "a failed QQ Music QR verification must surface as terminal status, not skip cleanup",
);
expectMatch(
  settings,
  /qqmusicQrStatus === "expired"\s*\|\|\s*qqmusicQrStatus === "failed"\s*\|\|\s*qqmusicQrStatus === "confirmed"/,
  "QQ Music QR polling must not continue after a terminal state",
);
expectMatch(
  settings,
  /timer = window\.setTimeout\(\(\) => void poll\(\), delayMs\);[\s\S]*window\.clearTimeout\(timer\)/,
  "QQ Music QR polling must live in an effect with cleanup",
);
expectMatch(
  settings,
  /qqmusicQrGenerationRef\.current \+= 1/,
  "a new QR must invalidate in-flight polls from the previous QR (stale-response guard)",
);
expectMatch(
  settings,
  /else if \(result\.status === "expired"\)[\s\S]*setQQMusicQrStatus\("expired"\)/,
  "QR expiry must be decided by the server, never faked by a local timer",
);

// 2. Signing in must never silently enable a source, and the login UI must
// be gated behind the source's enabled state.
expectMatch(
  settings,
  /请先开启 QQ 音乐来源再登录[\s\S]*Signing in never enables a source\s*automatically/,
  "the disabled QQ Music source must explain that sign-in does not enable it",
);
// QR + import + WebView sign-in buttons must all be gated by the source's
// enabled state (sign-in never enables a source; the buttons stay disabled
// until the user enables QQ Music explicitly).
expectMatch(
  settings,
  /disabled=\{isCreatingQQMusicQr \|\| !qqmusicEnabled\}/,
  "the QR button must be disabled until the source is enabled",
);
expectMatch(
  settings,
  /disabled=\{!qqmusicToken\.trim\(\) \|\| !qqmusicEnabled\}/,
  "the cookie import button must be disabled until the source is enabled",
);

// 3. UTF-8-safe cookie masking: no byte-index slicing on cookie values.
expectNoMatch(
  rustLib,
  /fn mask_netease_cookie[\s\S]*&value\[\.\.4\]/,
  "cookie masking must not byte-slice (non-ASCII input panics in release)",
);
expectMatch(
  rustLib,
  /value\.chars\(\)\.take\(4\)/,
  "cookie masking must slice on char boundaries",
);

// 4. Library-row covers stay stable (no proxy-token churn / clearing).
expectNoMatch(
  rustLib,
  /fn proxy_bilibili_track_covers\([\s\S]{0,400}?register_media_proxy/,
  "Bilibili library-row covers must not be replaced with short-lived proxy tokens",
);
expectNoMatch(
  qqmusic,
  /fn proxy_qqmusic_track_covers\([\s\S]{0,400}?register_media_proxy/,
  "QQ Music library-row covers must not be replaced with short-lived proxy tokens",
);

// 5. Media proxy must reject encoded IP literals (decimal/hex/octal forms).
expectMatch(
  rustLib,
  /u32::from_str_radix\(body, radix\)/,
  "media proxy must recognize numeric IP encodings beyond plain dotted-quad",
);

// 6. Local missing files are surfaced as unavailable instead of silent.
expectMatch(
  rustLib,
  /"file_missing"\.to_string\(\)/,
  "missing local files must be surfaced as file_missing",
);

// 7. SQLite hardened: WAL + busy timeout present at startup.
expectMatch(
  rustLib,
  /PRAGMA journal_mode = WAL; PRAGMA busy_timeout = 5000;/,
  "SQLite must run in WAL mode with a busy timeout",
);

// 8. Asset protocol scope must not expose the app data / WebView2 profile.
expectNoMatch(
  await read("src-tauri/tauri.conf.json"),
  /APPDATA\/com\.ome\.music/,
  "static asset scope must not expose the app data or WebView2 profile",
);

console.log(
  "Regression checks passed: settings ownership, source isolation, auth boundaries, service startup, compact layout and v0.4.0 hardening are guarded.",
);
