# Ome Music 系统级审查问题清单（工作文档）

> 状态说明：本文档随审查进展持续更新。标记 [已核实] 的问题均有行号级代码证据。
> 基线：branch `pr-15-qqmusic` @ 69c9794 + 工作区整改。
> **2026-09-01 更新**：Phase 4 修复已完成并全部验证（见文末"已修复清单"）；标记 [已修复] 的问题均已关闭并有测试/构建证据。

---

## 已修复清单（Phase 4 完成项）

| ID | 问题 | 修复证据 |
| --- | --- | --- |
| P1-7 | mask_netease_cookie UTF-8 切片 panic | lib.rs `chars().take(4)` 字符边界掩码 + 4 个回归测试（`masks_` 系列） |
| P1-1 | QQ VIP 回退打印完整响应体 | qqmusic.rs 改为 debug-gated metadata-only + body_length |
| P2-1 | follow_qqmusic_login_redirect UTF-8 切片 | 新增 `truncate_html_fragment` helper（floor_char_boundary）替换 4 处 |
| P1-2/#4 | QQ QR 轮询泄漏/重复 | 重构为 useEffect+setInterval+cleanup；新版 createQQMusicQr 只生成 |
| P1 blocker #1 | QR 确认失败清理缺失+死循环 | lib.rs 捕获 import 错误为 terminal failed + 前端终态停止轮询 |
| P1 blocker #2 | 登录不启用来源+无门控 | 登录 UI 全部按钮 `disabled={!qqmusicEnabled}` + 提示条 |
| P2 #3 | effect 重跑清空已粘贴凭据 | `musicSourceConfigRef` 稳定回调 identity |
| P1-4 | 库行封面代理 TTL/LRU 驱逐 | bilibili/qqmusic track covers 改 no-op（保持稳定 CDN URL，与 netease 一致） |
| P1-5 | cover hydration 失败不重试 | 成功后才入 Set，失败留下轮次重试 |
| P1-6/A | Lyrics Room 无窗口化 | ±12 窗口 + 占位符保持滚动高度；will-change 收窄 |
| B | LyricsRoom memo 失效 | setProgress useCallback |
| J/P1-3 | Queue 全量渲染 | `.queue-row` content-visibility:auto + contain-intrinsic-size |
| D | Bilibili "Preparing Atmosphere" 谎报 | `!src` 直接显示 Cover Atmosphere |
| C/P2-8 | 弹幕暂停冻结 | 两层弹幕 isPlaying=false 时清空 |
| E/P2-7 | Settings 模态可达性 | role=dialog aria-modal + Escape + 背板点击 |
| F/P2-9 | 标题对话框叠层 | overlayOpen prop 联动收起 |
| H | 模态背后弹幕空转 | activeOverlay !== none 时不渲染全局弹幕 |
| P2-6 | scan_music_directory `..` 绕过 | normalize_music_scope_path 拒绝 ParentDir + 回归测试 |
| P3-10 | DB WAL/busy_timeout/ALTER 吞错/文件缺失 | WAL+busy_timeout；ALTER 守卫；load_tracks 标注 file_missing |
| P2-2/F5 | QQ 封面/头像缺白名单 | is_trusted_qqmusic_media_url 公开并在两条路径使用 |
| P2-10 | 媒体代理编码 IP 绕过 | u32::from_str_radix 识别十进制/八进制/十六进制 IP + 回归测试 |
| P2-3/F4 | logout 不完整 | 清空 QR 会话 + 尽力清 WebView2 cookie |
| P2-11/#24 | asset scope 过宽 | 静态 scope 收窄为 $HOME/Music/** |
| #23 | CSP img-src 缺 ome-media: | tauri.conf.json 补 ome-media: |
| P3-6/#1.1 | netease-runtime 版本漂移 | 0.3.8 + lockfile 重生成 |
| P3-5/3.1 | CI 缺测试 | cargo test（Linux+Windows）+ npm run test 步骤 |
| P3-8/4.1 | npm audit Known Risk | SECURITY.md Known Risk 章节 + CHANGELOG |
| P3-7/5.1/5.2 | README/CHANGELOG 过时 | README 双语言补 QQ Music；CHANGELOG [Unreleased] 全面更新 |
| P3-12 | 测试体系 | Vitest+RTL 基础设施 + 22 个单元测试（ArtworkImage/resolveTrackCover/lyrics/QueueDrawer） |

---

## P0（发布阻塞）

（当前无已确认 P0。）

---

## P1

### P1-1 [已核实] QQ 音乐 VIP 回退路径无条件打印完整 API 响应体
- **file**: `src-tauri/src/qqmusic.rs:4097-4102`（函数 `try_get_vip_from_login_info`）
- **evidence**:
  ```rust
  let resp_summary = serde_json::to_string(&value).unwrap_or_default();
  eprintln!(
      "[QQMusic] vip_status GetLoginInfo回退响应: {}",
      &resp_summary[..resp_summary.len().min(800)]
  );
  ```
  无 `#[cfg(debug_assertions)]` 门控，release 也输出。内容为 `GetLoginInfo` 完整响应（uin/昵称/头像等 PII；若服务端回显会话字段则升级为凭据泄露）。
- **来源**: 贡献者原始提交 7bf60b4（需作为维护者修复 commit 在其后追加）。
- **reproduction**: 登录后任意触发 `fetch_qqmusic_vip_status` → `GetLoginInfo` 回退路径。
- **impact**: 日志留存 PII；违反"日志只能显示 exists/length/boolean/status/reason"规范。
- **recommended fix**: 只打印 `req_code`、字段存在性布尔、body_length；删除原始响应打印。
- **release_blocker**: 否（但在发布前修复，成本极低）。

### P1-2 [已核实] QQ 音乐 QR 轮询定时器泄漏 + 重复轮询竞争
- **file**: `src/components/ProviderSettingsPanel.tsx:1227-1288`（`createQQMusicQr`）
- **evidence**: 使用递归 `setTimeout(poll, ...)`，无取消 ref；组件卸载（关闭设置面板）后轮询链继续运行并 setState（最长 2 分钟）；重复点击"生成二维码"启动多条并发轮询链，各自调用 `import_qqmusic_token` 竞争登录状态；网络错误路径无重试上限且继续轮询。对比 NetEase/Bilibili 轮询（`useEffect` + `setInterval` + `cancelled` + `clearInterval`，646-796 行）是正确模式。
- **reproduction**: 打开 QQ 音乐设置 → 生成二维码 → 关闭设置面板（轮询继续）；或连续点两次生成。
- **impact**: 定时器/状态更新泄漏；登录状态竞态抖动；卸载组件 setState。
- **recommended fix**: 重构为与 NetEase/Bilibili 相同的 `useEffect` + `setInterval` + cleanup 模式，并将 poll 链挂到 ref 以便重复生成时先取消旧链。
- **release_blocker**: 否。

### P1-3 [已核实] 队列（QueueDrawer）无虚拟化/截断，大库全量渲染
- **file**: `src/components/QueueDrawer.tsx:137-263`
- **evidence**: `tracks.map(...)` 全量渲染，每行含按钮 + ArtworkImage；TopSearch 有 `SOURCE_PAGE_SIZE` 截断，QueueDrawer 没有。500-1000+ 曲目时每行 DOM 节点数大。
- **reproduction**: 导入 1000 首本地曲目后打开 Queue。
- **impact**: 大队列明显卡顿；与"目前已有 Queue limited rendering"的声称不符。
- **recommended fix**: 渲染窗口化（如按滚动位置切片 ±60），或改为 `content-visibility: auto`。
- **release_blocker**: 否。

### P1-4 [已核实] Cover：代理 token 1h TTL + 256 LRU，长会话/大库下 Bilibili/QQMusic 封面永久 fallback；前端封面无重注册路径
- **file**: `src-tauri/src/lib.rs:71-73`（TTL/上限常量）、`6109-6163`（注册）、`882-891`（list_tracks 全量注册）；`src/components/ArtworkImage.tsx`（无重试）
- **evidence**: `MEDIA_PROXY_TTL = 1h`、`MEDIA_PROXY_MAX_ENTRIES = 256`；`list_tracks` 对每条 bilibili/qqmusic 行注册代理 → >256 行时即刻 LRU 驱逐；1 小时后全部过期。音频有 auto-retry（App.tsx handleError 重解析），**封面没有**任何重新注册路径。
- **reproduction**: 库 >256 首 bilibili/qqmusic 曲目 → 启动后部分封面即回退占位；单次会话 >1h → 全部回退。
- **impact**: 长会话/大库封面大面积永久占位（产品级体验缺陷）。
- **recommended fix**: 会话内封面 URL 稳定性策略：优先存稳定原始 URL（netease 已如此），bilibili/qqmusic 注册量大时提升上限或让 ArtworkImage onError 触发"重新解析封面"回调。
- **release_blocker**: 否（产品体验 gate）。

### P1-5 [已核实] Cover hydration 失败不重试、Set 只增不减、仅覆盖 NetEase
- **file**: `src/App.tsx:369-394`
- **evidence**: `coverHydrationRef.current.add(target.id)` 在请求**前**加入；`.catch(() => {})` 空；Set 永不删除 → 同一曲目失败后永不再尝试；`needsNeteaseCoverHydration` 只限 netease（173-177）。
- **reproduction**: NetEase 搜索曲目导入时元数据请求失败 → 封面永远占位直至重启。
- **impact**: 封面修复能力弱；内存 Set 增长（量小）。
- **recommended fix**: 失败移出 Set 允许重试 + 失败计数上限；扩展到 bilibili/qqmusic 空封面行。

### P1-6 [已核实] Lyrics Room 无窗口化，长歌词全量渲染
- **file**: `src/components/NowPlayingHero.tsx:527-558, 567-588`
- **evidence**: `visualLines = useMemo(lyrics.map(...))` 全量映射；`cappedDistance` 只做视觉衰减，远行仍渲染 button。长歌词（数百行）每次换行重算全部行 + DOM 节点驻留。
- **reproduction**: 播放 300 行歌词的歌曲，观察 DOM 节点与换行成本。
- **impact**: 长歌词卡顿；与"Lyrics window rendering"声称不符（视觉上已合成空间舞台，但无窗口截断）。
- **recommended fix**: 按 `currentLyricIndex ± N`（如 ±16）切片后再 map；保留渐隐过渡需给窗口边界行补占位。

### P1-7 [已核实] `mask_netease_cookie` UTF-8 字节切片 panic（release `panic=abort` 下用户输入可致进程崩溃）
- **file**: `src-tauri/src/lib.rs:7502-7519`（函数 `mask_netease_cookie`）
- **evidence**:
  ```rust
  Some(value) if value.len() > 8 => {
      format!("MUSIC_U={}****{}", &value[..4], &value[value.len() - 4..])
  }
  None if cookie.len() > 8 => format!("{}****{}", &cookie[..4], &cookie[cookie.len() - 4..]),
  ```
  **纯字节切片**，`[..4]` / `[len-4..]` 若落在多字节 UTF-8 字符中间即 panic。调用点：`load_netease_source_config:4374`（keyring 存储 token）、`fetch_netease_login_status:7354/7376/7404`。
- **reproduction**: 用户在 Cookie 导入框粘贴含中文（或其他非 ASCII）字符、>8 字节的凭据（无 MUSIC_U 前缀或 MUSIC_U 值非 ASCII）→ 打开设置/登录状态刷新时进程 panic；`Cargo.toml:39 panic="abort"` 下直接整进程退出。
- **impact**: 用户输入触发应用崩溃（自 DoS）；与"字符串切片须按字符边界"规范直接冲突。
- **recommended fix**: 用 `char_indices()` 取安全前缀/后缀，或 `floor_char_boundary`，或改为仅对 ASCII 长度掩码（`chars().take(4)`）。
- **release_blocker**: 建议发布前修复（P1）。

### P1-8 [已核实（经子代理）] Bilibili 视频氛围 `src` 为空时永久谎报 "Preparing Atmosphere"
- **file**: `src/components/NowPlayingHero.tsx:756, 789-795`（`BilibiliVideoAtmosphere`）
- **evidence**: `{src && !videoFailed && (<video .../>)}` — src 为空时 video 不渲染 → onCanPlay/onError 永不触发 → 三态恒为 preparing；`videoFailed` 卸载后无重试。
- **impact**: 状态文案谎报（用户以为卡死）；与"必须优先 real Cover Atmosphere 而非永久 Preparing"要求直接冲突。
- **recommended fix**: `!src` 时直接渲染 "Cover Atmosphere"；videoFailed 加有限重试（复用播放重试预算模式）。
- **release_blocker**: 否（不影响播放，但属用户可感知的状态谎报）。

---

## P2

### P2-1 [已核实] UTF-8 字节切片 panic（release `panic=abort` 下进程崩溃）
- **file**: `src-tauri/src/qqmusic.rs:2840-2843, 2863-2866, 2957-2959, 2971-2973`（`follow_qqmusic_login_redirect`）
- **evidence**: `let end = after.find(['"', '\'', '>', ';']).unwrap_or(after.len().min(500)); let extracted = &after[..end];` — `min(500)` 为字节索引，若 500 内无 ASCII 终止符且第 500 字节落在多字节 UTF-8 字符中间则 panic；HTML 为中文场景可触发。
- **impact**: 登录流程崩溃（应用自 DoS）。Cargo.toml `panic = "abort"` 放大。
- **recommended fix**: 用 `char_indices` 或 `str::floor_char_boundary` 找合法边界。

### P2-2 [已核实] QQ 封面/头像代理路径缺 QQ CDN 白名单（与搜索路径不一致）
- **file**: `src-tauri/src/lib.rs:1786-1789`（user playlists 封面）、`1884-1889`（profile 头像）
- **evidence**: 这两处用通用 `is_proxyable_remote_url` + `register_media_proxy`，未先过 `is_trusted_qqmusic_media_url`（对比 `qqmusic.rs:1975-1996` 搜索封面路径先白名单）。
- **impact**: 上游响应被篡改时任意公网主机被代理（字面内网 IP 仍被拒）。理论面。
- **recommended fix**: 统一先 `is_trusted_qqmusic_media_url` 再加白名单注册。

### P2-3 [已核实] QQ Music logout 不完整（WebView2 Cookies 持久文件 + 内存 QR 会话残留）
- **file**: `src-tauri/src/qqmusic.rs:1916-1921`，`src-tauri/src/lib.rs:1655-1668`
- **evidence**: logout 仅删 keyring；`$LOCALAPPDATA/com.ome.music/EBWebView/.../Cookies` 持久文件与 `qqmusic_qr_sessions` 内存映射未清理。
- **impact**: 登出后重开 webview 窗口仍是已登录态；磁盘残留会话。
- **recommended fix**: logout 调 CookieManager DeleteAllCookies（或删 profile 目录）+ 清空 qr_sessions。

### P2-4 [已核实] WebView2 Cookie 读取面过宽（空 URI 读全 profile）
- **file**: `src-tauri/src/lib.rs:2271-2306`（`get_webview2_all_cookies`）
- **evidence**: `HSTRING::from("")` 空 URI 读取 profile 全部 cookie，仅靠名称/域名 allowlist 过滤；域名白名单含裸 `qq.com` 父域 → 用户在 webview 窗口内访问其他 qq.com 子域时同名 cookie（skey/p_skey/uin）会被收割。
- **impact**: 跨 QQ 服务的凭据混淆进入 QQ 音乐 keyring。
- **recommended fix**: `GetCookies` 显式传 `https://y.qq.com/` URI（或 window navigation 限制信任域）。

### P2-5 [已核实] QQ Music VIP `is_member` 由 cookie 存在性推断（未 API 确认）
- **file**: `src-tauri/src/qqmusic.rs:4031-4039, 4057-4063`（`fetch_qqmusic_vip_status`）
- **evidence**: API 失败/未确认时仅凭 `has_key` 置 `is_member: true`，与登录状态机"authenticated 仅验证后设置"纪律不一致；前端可能显示会员标识。
- **impact**: 误导性 UI；非泄露。
- **recommended fix**: 置 `is_member: false` + `membership_known: false`（前端显示"未知"）。

### P2-6 [已核实] `scan_music_directory` 授权检查未规范化路径，`..` 可绕过
- **file**: `src-tauri/src/lib.rs:1032-1044`（`scan_music_directory`）
- **evidence**: `is_authorized = authorized.iter().any(|dir| requested.starts_with(dir))` — `Path::starts_with` 是词法组件比较，不解析 `..`；`C:\Music\..\Secret` starts_with `C:\Music` 为真，但实际解析到授权目录外。`discover_audio_files` 不 canonicalize。
- **impact**: 授权边界可被 `..` 组件绕过（文件枚举面）；自定义命令前端可调（XSS 场景放大）。
- **recommended fix**: 对 requested 与授权目录均做 canonicalize（`fs::canonicalize` 或词法级 `Component::ParentDir` 拒绝）后再比较。
- **release_blocker**: 否（本地用户自枚举，纵深防御）。

### P2-7 [已核实] Settings 模态无 Escape/背板点击/焦点管理（a11y 与交互一致性）
- **file**: `src/components/ProviderSettingsPanel.tsx:1469-1489`；`src/App.tsx:2152`
- **evidence**: 外层 `fixed inset-0 z-50` 无 onClick/role/aria-modal；无 Escape 处理（对比 QueueDrawer 有 Escape+背板+role=dialog）；关闭即卸载无退场动画。
- **impact**: 键盘可达性缺失；模态一致性差。
- **recommended fix**: Escape 关闭 + 背板点击关闭 + `role="dialog" aria-modal="true"` + 焦点管理。

### P2-8 [已核实（经子代理）] 暂停时弹幕冻结残影不清理
- **file**: `src/components/DanmakuAtmosphereLayer.tsx:166-177`、`GlobalDanmakuAtmosphereLayer.tsx:64-72/189-197`
- **evidence**: `animationPlayState: isPlaying ? "running" : "paused"` + `fill-mode: both` → 暂停时 animationend 永不触发，行以中间帧定格；清理只在 seek/切歌/设置变更。
- **impact**: 违反"Idle 必须安静"——暂停是最应安静的状态却残留文字。
- **recommended fix**: `isPlaying` 变 false 时一次性 fade-out 后清空。

### P2-9 [已核实] 标题展开对话框可与其他 overlay 并存（违反单一 primary overlay）
- **file**: `src/components/NowPlayingHero.tsx:370-399`（`isTitleExpanded` 本地状态不在 activeOverlay 状态机内）
- **evidence**: 背板 z-30 < search z-40 / quickSettings z-50；标题对话框打开时可再开搜索/快捷设置 → 多层半透明叠层。
- **recommended fix**: 标题展开并入 activeOverlay（如 "title" 态），或打开其他 overlay 时先收标题。

### P2-10 [已核实] 媒体代理 DNS rebinding / 十进制 IP 直通（SSRF 纵深缺口）
- **file**: `src-tauri/src/lib.rs:5937-5974`（`is_safe_remote_media_url`），`Err(_) => true`（5972）
- **evidence**: 非字面 IP 主机名一律放行且注册时无解析校验（运行时由 reqwest 二次解析）；`http://2130706433/`（十进制 127.0.0.1）与 `127.0.0.1.nip.io` 类域名可绕过 IP 检查。
- **impact**: 用户配置恶意源的前提下的内网请求面。
- **recommended fix**: 注册时解析 host 并 pin（reqwest `.resolve()`），或注册时拒绝解析为私网 IP 的主机名。
- **release_blocker**: 否（攻击链依赖用户配置恶意源）。

### P2-11 [已核实] assetProtocol scope 过宽（整个 APPDATA/LOCALAPPDATA 可经 asset: 读取）
- **file**: `src-tauri/tauri.conf.json:29-32`
- **evidence**: scope 含 `$APPDATA/com.ome.music/**` 和 `$LOCALAPPDATA/com.ome.music/**`（后者含 WebView2 共享 profile：EBWebView/Default/Network/Cookies 数据库）。前端可经 `convertFileSrc` 读取应用 DB 与 WebView2 cookie 库（本地前端被攻陷时）。
- **recommended fix**: 收窄为 cache/tmp/cover-cache 等子目录；用户音乐目录已由运行时 `allow_directory` 授予。
- **release_blocker**: 否（严格 CSP + 本地内容缓解）。

### P2-12 [已核实] NetEase base_url 无 scheme/主机校验 + cookie 进 URL query
- **file**: `src-tauri/src/lib.rs:4392-4423`（保存）、`6744-6824`（`request_netease_json_response`，`query_items.push(("cookie", token))` 6785-6787）
- **evidence**: `let base_url = payload.base_url.trim().trim_end_matches('/');` 无任何校验；http:// 明文也放行；请求时全量 cookie 既进 COOKIE 头又作为 URL query 参数（出现在服务器/代理日志）。
- **recommended fix**: 默认仅 localhost；外部地址要求 https；cookie 仅走 header。
- **release_blocker**: 否（用户显式配置前提）。

### P2-13 [已核实（经子代理）] Settings 模态无 Escape/背板点击/焦点管理（同 P2-7，双源确认）

### P2-14 [已核实（经子代理）] 暂停时弹幕冻结残影（同 P2-8，双源确认）

---

## P3

### P3-1 [已核实] `open_qqmusic_webview_login` 未加 Windows gate（lib.rs:2346，导入有 gate 打开没有）
### P3-2 [已核实] `test_qqmusic_connection` 错误信息嵌入 reqwest 原始错误（qqmusic.rs:2212；当前匿名 URL 安全）
### P3-3 [已核实] `Referer` 头拼接用户可控 playlist_id（qqmusic.rs:3696-3700；CRLF 被 reqwest 拒，字符集未限制）
### P3-4 keyring 非 Windows 平台退回 mock store（Cargo.toml:30；与 Windows-only 目标一致）
### P3-5 [已核实] CI 缺测试步骤（.github/workflows/ci.yml：无 cargo test / npm run test / windows job）
### P3-6 [已核实] 受管 NetEase runtime manifest 版本漂移（netease-runtime/package.json=0.3.7，应用 0.3.8）
### P3-7 [已核实] CHANGELOG [Unreleased] 过期（"No unreleased changes yet" 但分支有 8 个未发布提交）；README 双语言未提 QQ Music
### P3-8 [已核实] npm audit：7 漏洞（根锁文件）/ 5 漏洞（netease-runtime 锁文件），无安全升级路径，必须记录 Known Risk
### P3-9 未跟踪内部文档（PRODUCT_REMEDIATION_PLAN / REMEDIATION_EXECUTION_REPORT）与孤儿文档（LICENSE_DECISION/TODO-app-refactor/size-budget）
### P3-10 DB：无 WAL/busy_timeout/user_version；ALTER 吞错（lib.rs:4175）；upsert/歌单导入无事务；本地文件缺失不标 unavailable_reason
### P3-11 嵌入式 artwork data URL 全量入库（DB 膨胀；几百首 ≈ 上百 MB）
### P3-12 LyricsRoom memo 失效（setProgress 非 useCallback → 4Hz 全列表重渲染）；每行 will-change 常驻合成层（globals.css:251-254）
### P3-13 GlobalDanmaku 在模态背后继续动画（rAF 空转）；PlayerDock 无 idle 减速；lyrics-scroll overflow-x 计算值 auto；activeOverlay 死状态 "source"/"lyricsTools"；搜索栏与标题栏拖拽区 12px 重叠

---

## P3

### P3-1 [已核实] `open_qqmusic_webview_login` 未加 Windows gate（lib.rs:2346，导入有 gate 打开没有）
### P3-2 [已核实] `test_qqmusic_connection` 错误信息嵌入 reqwest 原始错误（qqmusic.rs:2212；当前匿名 URL 安全）
### P3-3 [已核实] `Referer` 头拼接用户可控 playlist_id（qqmusic.rs:3696-3700；CRLF 被 reqwest 拒，字符集未限制）
### P3-4 keyring 非 Windows 平台退回 mock store（Cargo.toml:30；与 Windows-only 目标一致）

---

## 已核实为干净的关键项（附证据）

- 凭据管线：keyring（DPAPI）存储、仅发 QQ 官方域、日志全部 exists/length/boolean/status/reason。
- `qqmusic_debug_dump` 为 `#[cfg(test)]` 且未注册命令；无条件 `eprintln!` 仅布尔/长度。
- 登录状态机：`Authenticated` 仅在 `verify_qqmusic_session` API 验证通过后设置；`SignedOut/Failed/Expired/CredentialPresent` 映射真实。
- 媒体代理本地 IP/私网/环回/IPv6 全面拦截 + 每跳重定向复检；不带 Cookie 转发。
- 保存隔离：`save()` 只写 curator；三个来源各自 draft 保存；启用开关有回滚。
- 启动只初始化已启用来源（App.tsx 480-606 diff）；NetEase 单实例 + start_lock 双检（lib.rs 6408-6592）。
- 快照封面 `stableArtworkUrl` 剥离 ome-media（lastSessionSnapshot.ts:94-103）；入库前不代理（lib.rs:8016-8044）。
- ArtworkImage failed 随 src 重置（ArtworkImage.tsx:22-24）；Queue/Search/Hero 主要共用 resolver。
- capabilities 最小权限（仅 window 控制 + event）。