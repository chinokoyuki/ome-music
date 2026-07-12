import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";

const read = (path) => readFile(new URL(`../${path}`, import.meta.url), "utf8");
const expectMatch = (content, pattern, message) => assert.ok(pattern.test(content), message);
const expectNoMatch = (content, pattern, message) => assert.ok(!pattern.test(content), message);

const [rustLib, qqmusic, provider, settings, quickSettings, topSearch, queueDrawer] =
  await Promise.all([
    read("src-tauri/src/lib.rs"),
    read("src-tauri/src/qqmusic.rs"),
    read("src/features/musicSources/provider.ts"),
    read("src/components/ProviderSettingsPanel.tsx"),
    read("src/components/LyricsSourceMenu.tsx"),
    read("src/components/TopSearch.tsx"),
    read("src/components/QueueDrawer.tsx"),
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
  "QQ Music credentials must be persisted only by the validated import command",
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

console.log("Regression checks passed: QQ Music auth and network boundaries are guarded.");
