# Ome Music 系统级审查与修复最终报告（v0.4.0 候选）

> 审查基线：branch `pr-15-qqmusic` @ 69c9794 + 上一轮整改工作区
> 完成基线：branch `pr-15-qqmusic` @ 151f2f0（本轮 6 个维护者提交在其后）
> 日期：2026-09-01（会话内验证）

---

## A. 当前整体健康度

**78 / 100（审查前基线约 65）**

依据：全部自动门禁绿（下节 G 证据），无未关闭 P0；P1 全部关闭；P2 大部分关闭；剩余 P2/P3 均为有文档依据的设计权衡或低风险残留。

---

## B. 各模块健康度（/10）

| 模块 | 评分 | 关键证据 |
| --- | --- | --- |
| Playback | 8.0 | 重试预算有上限（3 次，canplay 重置）；A-B-A 不丢状态；Like 不动进度；失败有真实 reason；audioCandidates/proxy 候选链完整 |
| Local | 7.0 | 授权目录持久化 + 启动恢复 asset scope；file_missing 已标注；embedded artwork→data URL→SVG 兜底闭环；路径无 canonicalize（大小写变体→重复行）残留 |
| NetEase | 7.5 | 状态机诚实（loginStatusKnown/unknown 区分）；单实例启动锁 + 响应结构健康检查；QR/poll 清理正确；base_url 无 scheme 校验 + cookie 进 query 残留（P2-12 见 E） |
| Bilibili | 7.5 | 搜索/元数据/播放/弹幕/氛围全链路真实；封面归一化 https；"Preparing" 谎报已修；A-B-A 恢复入队即复现 |
| QQ Music | 8.0 | allowlist 三层（API/登录/媒体）；登录状态机诚实（authenticated 仅 API 验证后）；凭据只进 keyring；QR 生命周期已修；VIP 诚实已修；Cookie 提取 Windows-gated + 名称/域过滤 |
| Cover | 8.0 | 统一 resolver（resolveTrackCover）+ ArtworkImage failed 重置；库行封面稳定 URL 策略（netease/bilibili/qqmusic 一致）；hydration 失败可重试；A-B-A/重启不丢 |
| Lyrics | 8.0 | 真三维舞台（Arc Room 默认）+ 翻译 + click seek + 元数据行过滤 + -1 语义；窗口化 ±12 + memo 恢复 |
| UI | 7.5 | 单 activeOverlay 状态机；Settings 模态可达性已补；弹幕安全区/data-danmaku-safe-zone；标题对话框叠层已修；PlayerDock idle 减速（P3 残留） |
| Queue | 7.5 | Clear/Remove 不动曲库；content-visibility 化渲染；source 标签+重试；Escape/背板；大队列仍有全量 DOM（有窗口化占位，未做虚拟列表） |
| Settings | 8.0 | 逐来源保存隔离；登录不自动启用来源；QR/导入/WebView 门控；配置加载完成前禁用保存；焦点/aria 已补 |
| Performance | 7.5 | Lyrics 窗口化；Queue skip 渲染；setProgress 稳定；GlobalDanmaku 模态挂起；30 分钟连续播放无明显退化（未经 profiler 量化，人工 QA 保留） |
| Security | 8.5 | 无 P0；凭据只进 keyring；日志只 exists/length/boolean/status/reason；SSRF 字面 IP 全拦 + 编码 IP 已拦；capability 最小化；asset scope 收窄；Known Risk 文档化 |
| Testing | 7.0 | 34 Rust 单元测试 + 22 Vitest 前端测试 + 40+ 静态回归断言；CI 现跑全部测试（Linux+Windows）；缺 Playwright 视觉回归与真实账号人工验收 |
| Database | 7.0 | WAL+busy_timeout；迁移幂等（列存在性守卫）；ALTER 不再吞错；SQL 全参数化；导入无事务 + 路径未规范化（P3 残留） |
| Release | 7.5 | 版本 9 处一致（含 netease-runtime 0.3.8 lockstep）；无秘密入库；tag 工整；CI 补测试；Known Risk 记录；CHANGELOG/README 更新；尚未人工验收 + 未 bump 0.4.0（冻结期） |

---

## C. 所有发现问题（P0 / P1 / P2 / P3）

完整清单见 `docs/AUDIT_FINDINGS.md`（含 severity/file:line/function/evidence/reproduction/impact/recommended fix/release_blocker）。摘要：

### P0（发布阻塞）
- **无**。审查与修复过程中未发现需阻塞发布的 P0。

### P1（已全部修复）
| ID | 问题 | file:line | 修复提交 |
| --- | --- | --- | --- |
| P1-7 | `mask_netease_cookie` 字节切片 panic（release abort；用户输入可崩） | lib.rs:7502 | 87abe6c |
| P1-1 | QQ VIP 回退无条件打印完整 API 响应体 | qqmusic.rs:4097 | 87abe6c |
| P1-2/#4 | QQ QR 轮询 setTimeout 链泄漏/双轮询/卸载存活 | ProviderSettingsPanel:1227 | 976e8fd |
| P1-B1 | QR 确认→验证失败→会话不清理 + 前端死循环 + 误报"已过期" | lib.rs:1548 → 976e8fd 前端 | 87abe6c+976e8fd |
| P1-B2 | 登录不启用来源 + 登录 UI 无门控（"已连接但不可用"） | ProviderSettingsPanel | 976e8fd |
| P1-4 | 库行封面代理 TTL/LRU 驱逐 + 无重注册 | lib.rs:6140+ | 87abe6c |
| P1-5 | cover hydration 失败不重试、Set 只增不减、仅 netease | App.tsx:369 | f70221a |
| P1-6/A | LyricsRoom 全量渲染 + 每行 will-change 合成层 | NowPlayingHero:527 | f70221a |
| P1-3/J | Queue 全量渲染（千首卡顿） | QueueDrawer:137 | f70221a |
| D | Bilibili 视频氛围永久"Preparing Atmosphere" | NowPlayingHero:826 | f70221a |

### P2（已修复者带 ✓）
| ID | 问题 | 状态 |
| --- | --- | --- |
| P2-1/F2 | follow_qqmusic_login_redirect UTF-8 字节切片 ×4 | ✓ 87abe6c |
| P2-2/F5 | QQ 歌单封面/头像代理缺 QQ CDN 白名单 | ✓ 87abe6c |
| P2-3/F4 | logout 不完整（WebView2 cookies + QR 会话残留） | ✓ 87abe6c |
| P2-4/F3 | WebView2 Cookie 读取面过宽（空 URI 读全 profile） | **未修（E 节说明）** |
| P2-5 | QQ VIP `is_member` 由 cookie 存在性推断 | ✓ 87abe6c |
| P2-6 | scan_music_directory `..` 路径绕过 | ✓ 87abe6c |
| P2-7/E | Settings 模态无 Escape/背板/aria/焦点 | ✓ 976e8fd |
| P2-8/C | 暂停时弹幕冻结残影 | ✓ f70221a |
| P2-9/F | 标题对话框与其他 overlay 并存 | ✓ f70221a |
| P2-10 | 媒体代理编码 IP（十进制/十六进制）直通 + DNS 再绑定 | ✓ 部分（编码 IP 已拦；DNS pin 未做，见 E） |
| P2-11/#24 | assetProtocol scope 过宽（可读 DB + WebView2 cookies） | ✓ 87abe6c |
| #23 | CSP img-src 缺 ome-media:（非 Windows 封面全挂） | ✓ 87abe6c |
| #7 | 本地文件缺失不标 unavailableReason | ✓ 87abe6c |
| P2-12 | NetEase base_url 无 scheme/host 校验 + cookie 进 URL query | **未修（E 节说明）** |
| P3-10 | DB WAL/busy_timeout/ALTER 吞错 | ✓ 87abe6c |
| #3 | 主 effect 重跑清空已粘贴凭据 | ✓ 976e8fd |
| H | 模态背后全局弹幕空转 | ✓ f70221a |

### P3（未修/记录）
Cargo.toml panic=abort（设计选择）、keyring 非 Windows mock store、test_qqmusic_connection 原始错误、Referer 头拼接 playlist_id、parse_bilibili_duration 溢出、导入无事务、路径未 canonicalize、PlayerDock idle 减速、lyrics-scroll overflow-x、activeOverlay 死状态、搜索栏拖拽盲区、孤儿文档 LICENCE_DECISION 等 —— 均有 file:line 与理由记录在 AUDIT_FINDINGS.md。

---

## D. 本轮已修复的问题（提交清单，维护者 commits 在贡献者之后）

| 提交 | 主题 | 覆盖 |
| --- | --- | --- |
| `87abe6c` | fix(security) | mask UTF-8、QQ 日志脱敏、登录跳转切片、编码 IP 拦截、封面白名单、logout 清理、asset scope、CSP、SECURITY Known Risk |
| `976e8fd` | fix(qqmusic) | QR 轮询 effect 化、终态处理、登录门控、effect 依赖稳定、Settings 模态可达性 |
| `f70221a` | fix(playback) | 封面 hydration 重试、setProgress 稳定、Lyrics 窗口化、Queue content-visibility、弹幕暂停清理、Bilibili 标签、标题对话框 |
| `4ede725` | test+ci+docs | Vitest×22、CI 测试步骤（Linux+Win）、回归护栏扩容、CHANGELOG/README/MAINTENANCE、netease-runtime lockstep |
| `151f2f0` | docs(qa) | 人工 QA Checklist v0.4.0 |

---

## E. 没有修的问题及原因

1. **P2-4 WebView2 Cookie 空 URI 读取**：仅靠域+名 allowlist 过滤。不改为按 URI 读取，因为 QQ 登录链会在 ptlogin2.qq.com/graph.qq.com 设置同域 cookie，按 y.qq.com URI 过滤会漏掉登录必需字段；窗口专用 + 完整性门控已把误收风险压到极低（审查共识 P2 可接受）。
2. **P2-12 NetEase base_url 无校验 + cookie 进 query**：NeteaseCloudMusicApi 协议把 cookie 放 query（上游约束）；外部 http 地址（局域网设备播放）是既有用户流程，强制 https/localhost 会破坏真实用法。需产品决策后处理，不能单方面改。
3. **媒体代理 DNS 再绑定 pin**：注册时 resolve+pin 与 CDN 多 IP/轮换冲突，误伤面大；已拦截编码 IP 形态（最常见绕过），残留为文档化理论面（需用户配置恶意源）。
4. **导入事务化 / 路径 canonicalize / parse_bilibili_duration 溢出**：P3 级，WAL+busy_timeout 已显著降低影响；留待后续清理轮。
5. **PlayerDock idle 减速、activeOverlay 死状态收敛、搜索栏拖拽盲区**：视觉/交互打磨项，无功能影响，且未经 Playwright 截图确认的视觉改动有回归风险——按"小步验证"原则留待 UI 专项。
6. **Playwright 视觉回归**：工具用量限制无法在本会话执行（与上一轮报告一致）；已列入人工 QA Checklist 与 CI 后续项，不冒充完成。

---

## F. 新增回归测试

- **Rust（+4，共 34）**：`masks_ascii_netease_cookie_without_leaking_preview`、`masks_short_and_missing_music_u_cookies`、`masks_non_ascii_cookie_without_panicking`、`masks_non_ascii_plain_cookie_without_panicking`、`music_scope_normalization_rejects_parent_traversal`、媒体代理编码 IP 拒绝（并入 media_proxy_rejects 系列）
- **Vitest（+22，4 个文件）**：resolveTrackCover×5、ArtworkImage×4（failed 重置）、lyricsResolver×8（元数据过滤/-1 语义/偏移）、QueueDrawer×5（来源标签/播放/Escape/Clear）
- **静态回归护栏（+8 组断言）**：QR 终态、登录门控、字符边界掩码、稳定封面、编码 IP、file_missing、WAL、asset scope 收窄

---

## G. 所有测试结果（最终提交状态 HEAD=151f2f0 实测）

| 检查 | 结果 |
| --- | --- |
| `npm run test`（regression + vitest） | ✅ 40+ 回归断言 + 22 vitest 全过 |
| `npm run lint` | ✅ 0 问题 |
| `npm run format:check` | ✅ 全部格式化 |
| `npm run docs:check` | ✅ |
| `npm run build`（tsc + vite） | ✅ |
| `npx tsc --noEmit` | ✅ |
| `cargo fmt --all -- --check` | ✅ |
| `cargo check --workspace` | ✅ |
| `cargo clippy --workspace -- -D warnings` | ✅ |
| `cargo test --workspace` | ✅ **34 passed / 0 failed / 1 ignored**（联网诊断按设计忽略） |
| `git diff --check` | ✅（唯一提示为 html 文档 EOF 空行，非门禁项） |

CI（.github/workflows/ci.yml）：push/PR 现跑 rust（check/clippy/fmt/**test**）+ **rust-windows（check+test）** + frontend（ci/npm ci/tsc/lint/format/build/**test**）+ docs。

---

## H. 人工验收项目

`docs/QA_CHECKLIST_v0.4.0.md` —— 10 组 40 项：
1. QQ 扫码×3 + 微信扫码×3 + 取消/过期/断网不造假 + 失败终态不误报过期 + 未启用门控 + 登出完整
2. 网易云登录×3 + 30 次搜索/切歌单实例 + 会员真实 reason + 停用不启动进程 + 非 ASCII Cookie 不崩
3. A-B-A 播放 + 队列连点 + Like 不重置进度 + 文件缺失标记 + Clear 不动曲库 + 千首队列 + 长歌词 + 播放失败 reason
4. 封面四来源统一 + 重启恢复 + A-B-A 不丢 + 长会话不过期
5. Bilibili 视频/封面氛围 + 弹幕不裁切 + 暂停清空 + 模态挂起
6. 窗口矩阵（1040×640…全屏）+ 最大化还原 10 次 + Escape + 模态焦点 + 标题对话框
7. 重启持久化（曲目/位置/音量/速度/来源/登录/授权目录）
8. 设置隔离（逐来源保存不变、关闭来源零请求、全关仅本地）
9. 安装/卸载/校验和
10. 自动化对照表

未通过前：**不合并、不打 tag、不发布**。

---

## I. PR #15 是否建议合并

**建议：可以合并（在人工 QA 完成后）。**

- 贡献者 4 个提交（7bf60b4→e524cbd，chinoshizuyuki）原样保留，作者与历史完整；
- 维护者修复（1af57d8→69c9794 + 本轮 87abe6c/976e8fd/f70221a/4ede725/151f2f0）全部在其后，构成干净的 merge commit 素材；
- 合并方式：**Create a merge commit**（禁止 squash/rebase，遵守贡献者历史要求）；
- 合并前必须：QA Checklist 通过（H）、CI 全绿（已满足）、main 分支目标状态确认。

---

## J. 是否可以进入 v0.4.0

**条件结论：可以进入 v0.4.0 RC（代码与自动化已达标），前提是人工 QA 通过。**

进入 RC 前由维护者执行：
1. 合并 PR #15（merge commit）；
2. 人工 QA Checklist 40 项通过；
3. 九处版本 bump 0.4.0（含 Cargo.lock 与 netease-runtime manifest）+ README/BUILD/CHANGELOG 收口；
4. 推送 tag v0.4.0 触发 Release workflow，校验 NSIS 校验和。

---

## K. v0.4.0 是否可以发布

**结论：当前不能发布（选项 2：可以合并，但不能发布 的加强版——合并前需完成人工验收）。**

理由（诚实声明，非"基本可以"）：
- ✅ 代码质量、安全、测试、CI 证据完备（本报告 G 节）；
- ❌ 真实账号扫码（QQ/微信）、会员播放、窗口矩阵、NSIS 干净安装 **未经人工验收** —— 没有任何自动化可以替代这四项；
- ❌ 版本 bump 与 tag 尚未执行（冻结期设计）；
- ❌ Playwright 视觉回归未执行（工具限制，已记录）。

---

## 最终结论（四档之一）

> **结论：当前状态 = 可以合并，但不能发布。**
> 具体路径：**人工 QA（H 节 40 项）通过 → 维护者 merge PR #15（merge commit）→ 九处 0.4.0 bump → v0.4.0 RC → 干净环境安装验收 → 正式发布 v0.4.0。**
> 在任何一项人工验收通过之前，任何"可以发布"的说法都无证据支撑。

---

## 附：关键交付物
- `docs/AUDIT_FINDINGS.md`（P0-P3 全量清单 + 已修复对照表）
- `docs/QA_CHECKLIST_v0.4.0.md`（人工验收清单）
- `docs/CHANGELOG.md` / `docs/MAINTENANCE.md` / `README.md` / `README.zh-CN.md` / `SECURITY.md`（发布一致性更新）
- 测试基础设施：`vite.config.ts`（vitest）、`src/test/setup.ts`、4 个测试文件
- 提交：`87abe6c` `976e8fd` `f70221a` `4ede725` `151f2f0`（均在 `pr-15-qqmusic`，贡献者提交之后）