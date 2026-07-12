# Tasks

- [x] Task 1: 编写 NSIS preinstall hook 停止旧进程
  - [x] SubTask 1.1: 在 `src-tauri/nsis-hooks.nsh` 新增 `NSIS_HOOK_PREINSTALL` macro，使用 PowerShell 按 ExecutablePath 等于 `$INSTDIR\resources\node\node.exe` 与 CommandLine 包含 `$INSTDIR\resources\netease-runtime` 精准匹配并停止进程
  - [x] SubTask 1.2: preinstall 先停止 Ome Music 主进程，等待 1-3 秒，再停止安装目录下 node.exe，再等待 1 秒
  - [x] SubTask 1.3: 停止后检测 `resources/node/node.exe` 是否仍被占用，若仍被占用则弹出友好提示并中止安装（不直接报错到写入失败）
  - [x] SubTask 1.4: 确保不使用 `taskkill /IM node.exe /F`，避免误杀系统其他 node.exe

- [x] Task 2: 重写 NSIS postuninstall 数据清理策略
  - [x] SubTask 2.1: 移除 `NSIS_HOOK_POSTUNINSTALL` 中删除 `$APPDATA/com.ome.music`、`$LOCALAPPDATA/com.ome.music` 的语句
  - [x] SubTask 2.2: 移除清理凭据管理器密钥（`cmdkey /delete:ome.music.*`）的语句
  - [x] SubTask 2.3: 移除删除注册表项 `HKCU "Software\com.ome.music"` 与 `HKCU "Software\Classes\com.ome.music"` 的语句
  - [x] SubTask 2.4: postuninstall 改为仅清理程序残留文件（如有），保留用户数据

- [x] Task 3: AppState 保存托管 NetEase runtime Child 句柄并复用
  - [x] SubTask 3.1: 在 `AppState` 增加 `Mutex<Option<Child>>`（或等价结构）保存托管 node 进程句柄
  - [x] SubTask 3.2: `ensure_local_netease_api_service` spawn 成功后将 Child 存入 AppState，而非丢弃
  - [x] SubTask 3.3: 再次调用 `ensure_netease_api_service` 时先检查 AppState 中的 Child 是否存活，存活则复用，不重复 spawn
  - [x] SubTask 3.4: 托管进程异常退出时清理 AppState 中的句柄，下次调用重新 spawn

- [x] Task 4: app 退出时清理托管 NetEase runtime 子进程
  - [x] SubTask 4.1: 在 tauri builder 上注册 `RunEvent::Exit` / `ExitRequested` 钩子
  - [x] SubTask 4.2: 退出钩子中从 AppState 取出托管 Child，先尝试优雅 kill（Windows 下 `taskkill /PID /T` 或 `Child::kill`），等待 2 秒
  - [x] SubTask 4.3: 优雅 kill 超时后强制 kill 子进程树
  - [x] SubTask 4.4: 确保只 kill 托管 Child，不影响系统其他 node.exe

- [x] Task 5: 扩展 NetEase 服务启动状态为分阶段进度
  - [x] SubTask 5.1: 在 `NetEaseServiceStatus`（前后端 DTO + TS interface）增加 `stage` 字段（如 `not_started / checking_runtime / checking_api / starting_service / waiting_health / ready / failed`）与可选进度百分比
  - [x] SubTask 5.2: `ensure_local_netease_api_service` 在各阶段返回对应 stage，而非单一 message
  - [x] SubTask 5.3: 文案统一为"正在启动网易云音乐源 / 第一次启动可能需要几秒"，严禁出现"编译 / npm install / Node.js / 开发服务器"

- [x] Task 6: 搜索网易云前 service gate
  - [x] SubTask 6.1: 在 `TopSearch` 或 `MusicSourceProvider` 搜索网易云路径前调用 `ensureNeteaseApiService`
  - [x] SubTask 6.2: 未 ready 时展示"正在启动音乐源"提示与分阶段进度，不静默失败
  - [x] SubTask 6.3: 服务 ready 后自动继续搜索
  - [x] SubTask 6.4: 启动失败时搜索区域显示错误原因与"重试"按钮

- [x] Task 7: 设置页 NetEase 服务状态卡片
  - [x] SubTask 7.1: 在 `ProviderSettingsPanel.tsx` 的 NetEase 卡片中展示当前 service stage 与进度
  - [x] SubTask 7.2: 启动失败时展示明确原因
  - [x] SubTask 7.3: 提供手动"重启服务"按钮（可选，若实现成本低）

- [x] Task 8: 确认 release 构建不执行 first-run npm/npx
  - [x] SubTask 8.1: 审查 `lib.rs` 中 `#[cfg(not(debug_assertions))]` 守卫，确认 `find_netease_api_entry`、`is_node_available`、`is_npx_available`、npx fallback 路径在 release 构建中均为 dead code
  - [x] SubTask 8.2: 确认 release 构建仅走 `resolve_managed_netease_api_runtime` 分支，依赖已打包的 `resources/netease-runtime/node_modules`
  - [x] SubTask 8.3: 在 `docs/BUILD.md` 与 `docs/CONFIGURATION.md` 明确"安装包是完整可用环境，不执行 first-run npm install"

- [x] Task 9: 版本号统一升级到 0.3.5
  - [x] SubTask 9.1: `package.json` version → 0.3.5
  - [x] SubTask 9.2: `src-tauri/Cargo.toml` version → 0.3.5
  - [x] SubTask 9.3: `src-tauri/tauri.conf.json` version → 0.3.5
  - [x] SubTask 9.4: `README.md` / `README.zh-CN.md` 安装包名 → 0.3.5
  - [x] SubTask 9.5: `docs/BUILD.md` 安装包名与 tag 示例 → 0.3.5
  - [x] SubTask 9.6: `docs/CHANGELOG.md` 新增 `[0.3.5]` 条目
  - [x] SubTask 9.7: `.github/workflows/release.yml` release body 版本引用（如有）→ 0.3.5

- [x] Task 10: 文档同步更新
  - [x] SubTask 10.1: `docs/CHANGELOG.md` 添加 0.3.5 Fixed 条目（安装失败修复、preinstall 清理、退出清理子进程、服务启动状态、卸载不删数据、release 内置 runtime 说明）
  - [x] SubTask 10.2: `docs/CONFIGURATION.md` / `docs/TROUBLESHOOTING.md` 明确"卸载默认保留数据，覆盖升级不丢登录态/曲库"
  - [x] SubTask 10.3: README / BUILD 明确"安装包是完整可用环境，普通用户不需要编译/npm/Node.js"

- [x] Task 11: 运行检查
  - [x] SubTask 11.1: `npm ci`
  - [x] SubTask 11.2: `npm run lint`
  - [x] SubTask 11.3: `npm run format:check`
  - [x] SubTask 11.4: `npm run build`
  - [x] SubTask 11.5: `npm run docs:check`
  - [x] SubTask 11.6: `cargo check --workspace`（CI 在 ubuntu-latest 运行；本地沙箱可能因缺 Tauri Linux 系统库而无法运行，需明确说明）
  - [x] SubTask 11.7: 若环境允许，`npm run release:windows` 前置检查

- [x] Task 12: 安全与产物复查后提交
  - [x] SubTask 12.1: `git status` 确认无 `node_modules` / `dist` / `target` / `*.exe` / `*.msi` / `*.sqlite3` 暂存
  - [x] SubTask 12.2: 确认 `src-tauri/resources/netease-runtime/` 仅含 `package.json` + `package-lock.json`，无 `node_modules`
  - [x] SubTask 12.3: 创建分支 `fix/windows-installer-runtime-lock`
  - [x] SubTask 12.4: 提交，信息 `fix: prevent Windows installer failure from locked managed runtime`
  - [x] SubTask 12.5: 推送并创建 PR 到 main，PR 标题同提交信息，PR 正文包含安装失败原因、修复内容、已完成测试、未完成测试及原因
  - [x] SubTask 12.6: 不直接 merge，等用户验收

# Task Dependencies

- Task 3 依赖 Task 1（理解根因后再改 AppState）
- Task 4 依赖 Task 3（退出钩子需要 AppState 中的 Child 句柄）
- Task 5 依赖 Task 3（状态字段需要 AppState 配合）
- Task 6 依赖 Task 5（搜索 gate 复用分阶段状态）
- Task 7 依赖 Task 5（设置卡片复用分阶段状态）
- Task 8 可与 Task 3-7 并行（独立审查）
- Task 9 可与 Task 1-8 并行（纯版本号修改）
- Task 10 依赖 Task 9（CHANGELOG 需要版本号）
- Task 11 依赖 Task 1-10 全部完成
- Task 12 依赖 Task 11 通过
