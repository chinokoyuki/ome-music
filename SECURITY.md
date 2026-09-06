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

### Express / qs chain has no fix within express 4.x (since v0.4.0)

The same bundled runtime pins `express@4.22.2` (latest 4.x) and `body-parser`,
which resolve `qs@6.15.x` — below the `qs@6.16.0` fix for
[GHSA-x5fp-wj9c-mxmx](https://github.com/advisories/GHSA-x5fp-wj9c-mxmx) and
[GHSA-4mjr-xmp4-gh2g](https://github.com/advisories/GHSA-4mjr-xmp4-gh2g)
(moderate, denial-of-service via crafted query strings). No express 4.x release
raises the `qs` range, so the only real fixes are an express 5 migration or a
NetEase API runtime replacement.

**Status: Known Risk (accepted).** The service binds to `127.0.0.1` only and
parses query strings from the local app and from NetEase's own responses; it is
not reachable from the network. Tracked in `docs/CHANGELOG.md`.

### Resolved by compatible updates (2026-09-06)

`npm audit fix` (non-force, within declared semver ranges) plus a `vite`
6.4.3 upgrade cleared 9 of the 15 previously reported advisories in the root
tree and 9 of 15 in the bundled runtime tree:

- vite 6.4.3 — 3 high dev-server advisories (optimized-deps `.map` path
  traversal, launch-editor NTLMv2 disclosure, `server.fs.deny` Windows bypass)
  plus the esbuild dev-server advisory via the bundled esbuild 0.25.12.
- axios 1.18.1 (prototype-pollution / auth-injection family), form-data 4.0.6
  (CRLF injection), ip-address 10.7.0 (SSRF misclassification family),
  js-yaml 4.3.1, brace-expansion, browserslist, postcss-selector-parser.

The remaining six reports are exactly the two accepted risk chains above.
