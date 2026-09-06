# Ome Music 系统级审查与修复最终报告（v0.4.0 候选 · 2026-09-06 轮）

> 审查基线：branch `pr-15-qqmusic` @ f50f235（上一轮报告提交）
> 完成基线：branch `pr-15-qqmusic` @ 02caf16（本轮 4 个提交在其后）
> 本轮提交：`bb6c2ca`（回归守卫对齐）、`7bd9b86`（QQ 扫码登录连续性）、`4137422`（依赖安全兼容更新）、`02caf16`（clippy -D warnings 修复）
> 日期：2026-09-06（本机实测全部门禁）

---

## A. 当前整体健康度

**86 / 100**（审查基线 78；本轮净提升来自依赖安全、clippy 门禁与回归护栏收敛）

依据：全部自动门禁绿（G 节实测证据）；上一轮 P0 无新增且已关闭；本轮新发现 2 个 P0 级门禁问题（回归护栏漂移、clippy 失败）已修复；npm audit 15→6 且剩余 2 条均为有文档依据的 Accepted Risk；未执行人工 QA（不构成 PASS 依据，见 H）。

---

## B. 各模块健康度（/10）

| 模块 | 评分 | 关键证据（本轮实测） |
| --- | --- | --- |
| Playback | 9.0 | requestId 防陈旧响应（playableRequestRef/selectionRef×3）；重试预算上限 2 次自动 + canplay 重置；latest-ref 模式（handleAudioEndedRef/currentTrackRef）；loop one/all/curator 边界正确；Like 不动进度；失败有真实 reason 映射 |
| Local | 8.5 | file_missing 标记 + 恢复后可用性检查；asset scope 收窄（$HOME/Music/** + 运行期授权）；WAL+busy_timeout；路径/元数据/嵌入封面闭环；重启恢复（lastSessionSnapshot + stableArtworkUrl 去临时 URL） |
| NetEase | 8.5 | serviceReady≠signedIn（loginStatusKnown 诚实区分）；单实例启动锁 + 响应结构健康检查；QR 流程 803→cookie→验证；keyring 优先 + 文档化 fallback；会员 reason 真实（vip_required/trial_only） |
| Bilibili | 8.5 | A-B-A 恢复：audio/cover/video/danmaku 全部以 currentTrackId 为键重解析；视频不可用回退封面氛围（非永久 Preparing）；弹幕 effect 清理 + 重试上限 1 次；media proxy 候选链 |
| QQ Music | 8.5 | 凭据零打印（exists/length/boolean only）；debug_dump 为 #[cfg(test)] 且不在命令面；allowlist 三层（API/登录/媒体/WebView）；登录状态机诚实（authenticated 仅 API 验证）；**本轮新增** QR 登录连续性（cookie 会话回流 + 代际守卫 + 服务端权威过期） |
| Cover | 9.0 | 统一 resolver（resolveTrackCover）；临时 ome-media URL 持久化前清理（stableArtworkUrl）；ArtworkImage src 变化重置 failed；库行稳定 CDN URL（禁止代理 token 换库行封面，回归护栏 #4）；四来源一致 |
| Lyrics | 8.5 | 真三维舞台（perspective/translate3d/rotate/scale/opacity/blur，Arc Room 默认）+ 点击 seek + 翻译 + 元数据行过滤 + 未达首行不激活（-1 语义）；窗口化 ±12 渲染 |
| UI | 8.0 | 单 activeOverlay 状态机（同一时间一个 primary overlay）；Escape/背板关闭；z-index 分层（danmaku z-22 < notice z-40 < settings z-50 < env z-60）；弹幕安全区 data-danmaku-safe-zone 动态避让 |
| Queue | 8.5 | Clear/Remove 不触碰曲库（tracks 与 queue 语义分离，Clear 只清 agentQueue+重置播放）；窗口化渲染 + content-visibility；来源标签/重试/连点防抖（requestId）；Escape/背板 |
| Settings | 8.5 | 逐来源保存隔离（保存边界回归护栏）；登录不自动启用来源 + 按钮门控；QR/导入/WebView 三入口统一门控；配置加载完成前禁用保存；Quick/Full 职责分离 |
| Performance | 8.0 | Lyrics 窗口化、Queue 跳过渲染、setProgress 稳定引用、GlobalDanmaku 动态模态挂起（only when playing + overlay=none）；30 分钟 soak 未经 profiler 量化（人工 QA 保留项） |
| Security | 9.5 | 无凭据日志（含重定向链）；UTF-8 字符边界掩码；SSRF：注册+每跳重定向校验、字面/编码 IP 全拦；Cookie 只进 keyring/内存 map；WebView2 隔离 profile + 域/名 allowlist；CSP 合理；**本轮** npm audit 15→6，剩余 2 条 Accepted Risk 文档化 |
| Testing | 8.0 | 35 Rust 单元测试（+1，较上轮）+ 22 Vitest + 44 条静态回归断言（本轮 2 改 2 增）；CI 全流程本地复验绿；缺 Playwright 视觉回归与真实账号人工验收（记录在案） |
| Database | 8.5 | WAL + busy_timeout=5000；迁移幂等（pragma_table_info 守卫 ALTER）；sourceId 无碰撞（source 前缀隔离）；SQL 参数化；重启恢复 |
| Release | 8.0 | 版本 9 处一致（0.3.8，未 bump）；CI 工作流存在且本机全绿；npm ci dry-run 根+运行时均通过；未执行安装包构建（保留给 RC 阶段）；tag 未打（干净 main 后打） |

---

## C. 所有发现问题（P0 / P1 / P2 / P3）

### P0（本轮新发现，均已修复或文档化）

| ID | 问题 | file | 证据 | 修复 |
| --- | --- | --- | --- | --- |
| P0-1 | 静态回归护栏与新版 QQ 扫码轮询契约漂移：`npm test` 失败（setInterval 模式 + 前端伪造 timeout 终态已不存在） | scripts/regression-check.mjs | 实测 `npm test` 在 `test:regression` 抛 AssertionError；工作树 ProviderSettingsPanel 已改为递归 setTimeout + 代际守卫 | `bb6c2ca`：守卫改为编码新契约（终态 expired/failed/confirmed；effect 内 timer 清理；代际 +1 使陈旧 poll 失效；过期仅由服务端 65 决定） |
| P0-2 | `cargo clippy -- -D warnings` 失败：新 QR 状态 match 分支 6 处 `unneeded return`（尾表达式返回分号导致类型为 () 的隐患） | src-tauri/src/qqmusic.rs:2820-2902 | 实测 clippy 6 errors；移除 return 后 E0308（尾分号），二次修复 | `02caf16`：删除冗余 return 与尾分号，match 成为纯尾表达式；clippy/fmt/test 全绿 |
| P0-3 | vite 5.4.21 直接依赖携带 3 个 high 级 dev-server 公告（.map 路径穿越、launch-editor NTLMv2 泄露、server.fs.deny Windows 绕过）+ esbuild dev-server 公告 | package.json | npm audit：vite direct=high，fix=vite@8.2.2；实测 5.4.21 无 5.x 修复版 | `4137422`：升级 vite ^6.4.3（plugin-react@4.3.4 与 vitest@4.1.11 peer 均支持）；tsc/build/vitest 全绿 |
| P0-4 | 运行时与根依赖链漏洞：axios 1.17.0（原型污染族）、form-data、ip-address（SSRF 分类族）、js-yaml 等 9 项在 semver 内有兼容修复 | npm audit 15 项 | 实测 `npm audit fix`（非 force）可清 9/15 | `4137422`：根 + netease-runtime 双 lockfile 兼容更新；剩余 6 项 = 2 条 Accepted Risk 链，SECURITY.md/CHANGELOG 已记录 |
| P0-5 | NeteaseCloudMusicApi→music-metadata→file-type 链无兼容修复（Known Risk，延续） | resources/netease-runtime | audit fix 仅指向破坏性降级 3.47.5 | 保持文档化 Accepted Risk（SECURITY.md 已更新复核） |
| P0-6 | express→qs 链无 express 4.x 修复（本轮新记录） | resources/netease-runtime | express 4.22.2 为 4.x 最新，qs 固定 <6.16；body-parser 2.3.0 仍 qs ^6.15.2 | 记录 Accepted Risk（仅 127.0.0.1 本地 DoS，网络不可达） |

### P1（延续上轮，已全部关闭；本轮复查无新增）

Cover A-B-A/重启、库行封面稳定、QR 轮询生命周期、登录门控、cookie 掩码 panic、Lyrics/Queue 窗口化、Bilibili 氛围回退——上轮已修（87abe6c/976e8fd/f70221a 等），本轮对 `resolveTrackCover`/`ArtworkImage`/`playableRequestRef`/`handleAudioEndedRef`/`clearQueue`/弹幕 effect 逐一代码复查通过。

### P2（设计权衡，未改，有依据）

1. **App.tsx 仍是 2407 行单体** + `source === "netease"|"bilibili"|"qqmusic"` 分支存在于播放解析（resolvePlayable/playQueueTrack/导入流）。provider 方法已承载元数据/播放/登录，但 App 壳仍做来源编排。按用户指示"不为架构漂亮而大拆"，记为后续架构项（usePlaybackEngine/useQueueController 演化路径）。
2. **`tracks` 同时承载曲库与队列**：Clear/Remove 已证不删曲库，语义分离靠注释与测试锁定，结构上未拆分。
3. **LyricsRoom 预设仅 Arc Room 可切换**（DEFAULT_LYRIC_ROOM_PRESET 硬编码）：符合"默认 Arc Room"，多预设 UI 未做。
4. **WebView2 cookie 按域+名过滤而非按 URI**：QQ 登录链跨域设 cookie 的协议约束，完整性门控已压风险（上轮审查共识）。

### P3（低优先，未改）

1. qqmusic.rs 部分 `eprintln!` 未用 debug_assertions 门控（内容仅计数/状态码，无凭据）。
2. PR 分支历史含 3 个 "Merge branch 'zerolyx:main' into main" 提交（历史噪声，禁止改写）。
3. NetEase cookie 明文镜像文件（base64，PersonalConfig/netease_session.local）：设计如此（keyring 读取失败兜底），文档已注明。

---

## D. 本轮已经修复的问题

1. 回归护栏漂移（P0-1）→ `bb6c2ca`
2. clippy 6 errors（P0-2）→ `02caf16`
3. vite 5.4.21 高危公告（P0-3）→ `4137422`（vite 6.4.3）
4. 根 + 运行时 9 项依赖漏洞（P0-4）→ `4137422`（audit fix 兼容更新）
5. 既有工作区 QR 登录连续性未提交 → `7bd9b86`（cookie 会话回流 + 代际守卫 + 退避轮询，cargo/tsc/回归全绿后提交）
6. SECURITY.md / CHANGELOG 更新到新风险状态（`4137422`）

---

## E. 没有修的问题及原因

1. **file-type/music-metadata 链**：无兼容修复（降级 3.47.5 为破坏性且换用无人维护版本）；本地 127.0.0.1 解析服务端元数据，可利用性低 → Accepted Risk。
2. **express→qs 链**：express 4.x 无修复版；本地 DoS 型 → Accepted Risk。
3. **App.tsx 单体/来源分支**：架构重构违背"本轮不大拆"约束 → 后续轮。
4. **Lyrics 多预设/Playwright 视觉回归/30 分钟 soak/大队列 1000+ 实测**：需交互式或真实环境，本会话无 GUI 执行条件 → 人工 QA 清单（H）。
5. **版本 bump 0.4.0 / tag**：发布动作，须在干净 main 上由维护者在 QA 后执行（本轮结论 I/J 路径）。
6. **Cargo.toml 行尾伪差异**：内容一致，已恢复，无需处理。

---

## F. 新增回归测试

- **静态回归护栏（+2 改 2 增，共 44 条断言）**：
  - 改：QR 终态（expired/failed/confirmed，不再有前端伪造 timeout）；轮询生命周期（递归 setTimeout + effect 清理）。
  - 增：`qqmusicQrGenerationRef.current += 1`（新 QR 使陈旧 poll 失效）；服务端权威过期（`result.status === "expired"` → setQQMusicQrStatus，禁本地计时器伪造）。
- **Rust 测试基线**：35 passed（含 mask/cookie 合并/URL 校验等既有覆盖；本轮未新增 Rust 测试——QR 连续性函数为网络态逻辑，静态守卫 + 人工 QA 覆盖）。
- **Vitest**：22 passed（resolveTrackCover/ArtworkImage/lyricsResolver/QueueDrawer，无新增文件——UI 计时器逻辑改由静态守卫锁定契约）。

---

## G. 所有测试结果（HEAD=02caf16 本机实测）

| 检查 | 结果 |
| --- | --- |
| `cargo fmt --all -- --check` | ✅ |
| `cargo check --workspace` | ✅ |
| `cargo clippy --workspace -- -D warnings` | ✅（本轮修复后首次全绿） |
| `cargo test --workspace` | ✅ 35 passed / 0 failed / 1 ignored |
| `npx tsc --noEmit` | ✅ |
| `npm run build`（tsc + vite 6.4.3） | ✅ 1664 modules |
| `npm run lint` | ✅ 0 问题 |
| `npm run format:check` | ✅ |
| `npm run docs:check` | ✅ |
| `npm test`（regression + vitest） | ✅ 44 条断言 + 22 vitest |
| `npm ci --dry-run`（根） | ✅ |
| `npm ci --omit=dev --dry-run --prefix netease-runtime` | ✅ |
| `npm audit`（根 / 运行时） | ⚠️ 15→6；剩余 2 条 Accepted Risk 链（见 E） |

工作树：干净（唯一历史遗留为 Cargo.toml 行尾，已恢复）。

---

## H. 人工验收项目

沿用 `docs/QA_CHECKLIST_v0.4.0.md` 10 组 40 项，本轮特别强调（含本轮改动回归面）：

1. QQ 扫码 ×3 + 微信扫码 ×3：确认后 VIP/普通歌均真实播放；取消/过期/断网不造假过期；**旧 QR 轮询不覆盖新 QR（代际守卫）**；登录态在重启后保持。
2. 网易云登录 ×3 + 30 次搜索/切歌单；会员歌显示真实 reason（vip_required 等）。
3. A-B-A（Bilibili→任意→Bilibili）：audio/cover/video/danmaku 全恢复；弹幕暂停清空、不裁切中文下半部、不挡 PlayerDock/Queue/Settings。
4. 封面四来源统一 + 重启恢复 + A-B-A 不丢 + 长会话（>2h）不过期。
5. 队列：Clear 不动曲库；连点播放稳定；1000+ 曲目流畅。
6. 窗口矩阵（1040×640…全屏）+ 最大化还原 10 次 + Escape + 模态焦点。
7. 重启持久化（曲目/位置/音量/速度/来源/登录/授权目录）。
8. 设置隔离（逐来源保存不变、关闭来源零请求、全关仅本地）。
9. NSIS 干净安装/卸载/校验和；首次启动无需手动装 Node。
10. 30 分钟连续播放无 degradation（CPU/内存/GPU 观察）。

未通过前：**不打 tag、不发布**。

---

## I. PR #15 是否建议合并

**建议：可以合并。**

- 贡献者 4 个提交（7bf60b4→e524cbd，@chinoshizuyuki）原样保留，author/历史完整，未被 squash/rebase/重写；
- 维护者提交（1af57d8→f50f235 + 本轮 4 个）全部位于贡献者提交之后，构成 merge commit 素材；
- 合并方式：**Create a merge commit**（禁止 squash/rebase/复制重提）；
- 全部自动门禁本机绿；合并后 main 即满足 CI 门禁（Linux+Windows 均由 CI 覆盖）。
- 合并时机：维护者决定。建议与人工 QA 并行/在其后。

---

## J. 是否可以进入 v0.4.0

**条件结论：可以进入 v0.4.0 RC（代码与自动化证据完备），前提是人工 QA 通过。**

进入 RC 的路径（由维护者执行）：
1. 合并 PR #15（merge commit）；
2. 人工 QA 40 项通过（H 节）；
3. 九处版本 bump 0.4.0（package.json/package-lock/Cargo.toml/Cargo.lock/tauri.conf/netease-runtime manifest/README/BUILD/CHANGELOG）+ `docs:check` 复核；
4. 干净 main 上推送 tag v0.4.0 → Release workflow → NSIS 校验和复核。

---

## K. v0.4.0 是否可以发布

**结论：当前不能发布。**

- ✅ 代码质量、安全（含本轮 audit 收敛）、测试、CI 证据完备（G 节）；
- ❌ 真实账号扫码（QQ/微信）、会员播放、窗口矩阵、NSIS 干净安装、30 分钟 soak **未经人工验收**——自动化无法替代；
- ❌ 版本 bump 与 tag 未执行（须在干净 main 上）；
- ❌ Playwright 视觉回归未执行（工具限制，已记录）。

---

## 最终结论（四档之一）

> **结论：当前状态 = 可以合并，但不能发布（选项 2）。**
> 具体路径：**人工 QA（H 节 40 项）通过 → 维护者以 merge commit 合并 PR #15 → 九处 0.4.0 bump → v0.4.0 RC → 干净环境安装验收 → 正式发布 v0.4.0。**
> 在人工验收完成前，任何"可以发布"的说法都无证据支撑；本轮所有自动化证据（G 节）均如实列出，未把"测试绿"等同于"可发布"。

---

## 附：本轮关键交付物
- 提交：`bb6c2ca` `7bd9b86` `4137422` `02caf16`（均在 `pr-15-qqmusic`，贡献者提交之后）
- 文档：`SECURITY.md`（新 Accepted Risk + 已解决清单）、`docs/CHANGELOG.md`（依赖加固条目）
- 回归护栏：`scripts/regression-check.mjs`（44 条断言）
- 沿用：`docs/AUDIT_FINDINGS.md`、`docs/QA_CHECKLIST_v0.4.0.md`
