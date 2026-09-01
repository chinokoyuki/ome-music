# Security Policy

## Supported Versions

Ome Music is currently pre-release. Security fixes are handled on the main development line.

## Reporting a Vulnerability

Please do not publish API keys, cookies, tokens, passwords, private account data, or exploit details in a public issue.

Report privately to the maintainer first. Include:

- A short description of the issue
- Affected version or commit
- Steps to reproduce without exposing secrets
- Suggested mitigation, if known

## Secret Handling Rules

- Do not commit `.env` files.
- Do not commit SQLite databases.
- Do not commit cookies, tokens, or API keys.
- Do not commit logs or diagnostics that include request headers.
- Do not commit screenshots showing account state or personal playlists.

## Known Risks

### Bundled NetEase API dependency chain has no safe upgrade path (since v0.3.x)

The managed NetEase Cloud Music runtime ships `NeteaseCloudMusicApi@4.32.0`, which
depends on `music-metadata@7.x` → `file-type@16.x`. `npm audit` reports
vulnerabilities in this chain (notably infinite-loop issues in the ASF parser:
[GHSA-5v7r-6r5c-r473](https://osv.dev/vulnerability/GHSA-5v7r-6r5c-r473) for
`file-type`, fixed only in ≥21.x, and
[CVE-2026-32256](https://dependabot.ecosyste.ms/advisories/CVE-2026-32256) for
`music-metadata`, which has no 7.x fix).

**Status: Known Risk (accepted).** There is no compatible upgrade: the current
npm `NeteaseCloudMusicApi` (4.32.0) is the latest release; the only `npm audit`
"fix" is `npm audit fix --force`, which force-downgrades to `NeteaseCloudMusicApi@3.47.5`
— a breaking change that would replace the maintained API server with an old
unmaintained one. `overrides` are not viable because `file-type` ≥17 breaks the
`music-metadata` API contract.

**Mitigations:** the vulnerable code paths only run inside the local
`127.0.0.1` NetEase API service parsing *server-provided* metadata; the app's
own media proxy rejects private/loopback destinations and never forwards user
cookies to arbitrary hosts. We monitor upstream for a safe update and will
upgrade or replace the runtime when one is available. Tracked in
`docs/CHANGELOG.md` alongside version history.
