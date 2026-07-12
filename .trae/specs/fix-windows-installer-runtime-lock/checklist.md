# Checklist

## NSIS 安装器

- [x] `src-tauri/nsis-hooks.nsh` 存在 `NSIS_HOOK_PREINSTALL` macro，停止旧进程发生在写入 `resources/node/node.exe` 之前
- [x] preinstall 仅按 ExecutablePath 等于 `$INSTDIR\resources\node\node.exe` 或 CommandLine 包含 `$INSTDIR\resources\netease-runtime` 精准匹配进程，不使用 `taskkill /IM node.exe /F`
- [x] preinstall 先停止 Ome Music 主进程，等待 1-3 秒，再停止安装目录 node.exe
- [x] 停止后检测文件占用，仍被占用时弹出友好提示 "Ome Music is still running in the background..." 而非直接报错到写入失败
- [x] `NSIS_HOOK_POSTUNINSTALL` 不再删除 `$APPDATA/com.ome.music` 与 `$LOCALAPPDATA/com.ome.music`
- [x] `NSIS_HOOK_POSTUNINSTALL` 不再清理凭据管理器密钥（`cmdkey /delete:ome.music.*`）
- [x] `NSIS_HOOK_POSTUNINSTALL` 不再删除注册表项 `HKCU "Software\com.ome.music"` 与 `HKCU "Software\Classes\com.ome.music"`

## 托管 NetEase runtime 生命周期

- [x] `AppState` 包含保存托管 `Child` 句柄的字段（如 `Mutex<Option<Child>>`）
- [x] `ensure_local_netease_api_service` spawn 成功后将 `Child` 存入 AppState，不再丢弃
- [x] 重复调用 `ensure_netease_api_service` 时复用已存活 Child，不重复 spawn
- [x] 注册了 `RunEvent::Exit` / `ExitRequested` 钩子，退出时取 AppState 中 Child 优雅 kill
- [x] 优雅 kill 超时（约 2 秒）后强制 kill 子进程树
- [x] 退出钩子仅 kill 托管 Child，不影响系统其他 node.exe

## NetEase 服务启动状态与搜索 gate

- [x] `NetEaseServiceStatus`（前后端）包含 `stage` 字段（`not_started / checking_runtime / checking_api / starting_service / waiting_health / ready / failed`）
- [x] `ensure_local_netease_api_service` 在各阶段返回对应 stage
- [x] 文案为"正在启动网易云音乐源 / 第一次启动可能需要几秒"，不出现"编译 / npm install / Node.js / 开发服务器"
- [x] 搜索网易云前调用 `ensureNeteaseApiService`，未 ready 时展示"正在启动音乐源"提示
- [x] 服务 ready 后自动继续搜索，不出现点击搜索无反应
- [x] 启动失败时搜索区域显示错误原因与"重试"按钮
- [x] 设置页 NetEase 卡片展示当前 service stage 与进度，失败展示原因

## Release 构建路径

- [x] 审查确认 `find_netease_api_entry`、`is_node_available`、`is_npx_available`、npx fallback 在 release 构建（`#[cfg(not(debug_assertions))]`）下为 dead code
- [x] release 构建仅走 `resolve_managed_netease_api_runtime` 分支
- [x] release 构建不执行 first-run `npm install` / `npx`

## 版本号与文档

- [x] `package.json`、`src-tauri/Cargo.toml`、`src-tauri/tauri.conf.json` 版本均为 `0.3.5`
- [x] `README.md`、`README.zh-CN.md`、`docs/BUILD.md` 安装包名与 tag 示例为 `0.3.5`
- [x] `docs/CHANGELOG.md` 新增 `[0.3.5]` 条目，包含安装失败修复、preinstall 清理、退出清理子进程、服务启动状态、卸载不删数据、release 内置 runtime 说明
- [x] `docs/CONFIGURATION.md` / `docs/TROUBLESHOOTING.md` 明确"卸载默认保留数据，覆盖升级不丢登录态/曲库"
- [x] README / BUILD 明确"安装包是完整可用环境，普通用户不需要编译/npm/Node.js"
- [ ] `npm run docs:check` 通过（版本一致性强制）

## 检查与提交

- [x] `npm ci` 通过
- [x] `npm run lint` 通过（0 errors）
- [x] `npm run format:check` 通过
- [x] `npm run build` 通过
- [ ] `npm run docs:check` 通过
- [ ] `cargo check --workspace` 通过，或明确说明沙箱无法运行的原因（缺 Tauri Linux 系统库）
- [x] `git status` 确认无 `node_modules` / `dist` / `target` / `*.exe` / `*.msi` / `*.sqlite3` 暂存
- [x] `src-tauri/resources/netease-runtime/` 仅含 `package.json` + `package-lock.json`，无 `node_modules`
- [x] 分支 `fix/windows-installer-runtime-lock` 已创建并推送
- [x] 提交信息为 `fix: prevent Windows installer failure from locked managed runtime`
- [ ] PR 已创建，标题同提交信息，PR 正文包含安装失败原因、修复内容、已完成测试、未完成测试及原因
- [x] 未直接 merge main，等用户验收

## 测试场景（文档化，沙箱无法执行真实 Windows 安装）

- [ ] 测试 1 干净安装：卸载旧版 → 安装新版 → 启动 → 搜索本地音乐 → 打开设置 → 网易云源能启动（沙箱无法执行，需用户在 Windows 验收）
- [ ] 测试 2 覆盖升级：旧版运行 + node.exe 运行 → 不手动杀 → 运行新版安装包 → 安装器处理旧进程 → 不出现 `Error opening file for writing`（沙箱无法执行，需用户验收）
- [ ] 测试 3 扫码登录后搜索：安装新版 → 扫码 → 搜索 → 未就绪显示启动进度 → ready 后可搜索 → 失败有重试（沙箱无法执行完整流程，但前端逻辑可在 build 后静态验证）
- [ ] 测试 4 关闭后进程清理：启动 → 启动网易云服务 → 关闭 Ome Music → 检查安装目录 node.exe 不残留（沙箱无法执行，需用户验收）
- [ ] 测试 5 重装保留数据：安装并登录 → 导入本地音乐 → 卸载/覆盖 → 重装 → 用户数据不丢（沙箱无法执行，需用户验收）
