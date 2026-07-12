# Ome Music Windows 安装器与网易云运行时启动修复 Spec

## Why

用户安装 Ome Music 时 NSIS 报错 `Error opening file for writing: C:\Users\zerolyx\AppData\Local\Ome Music\resources\node\node.exe`，导致安装直接失败。根因有三：① `src-tauri/nsis-hooks.nsh` 没有 `NSIS_HOOK_PREINSTALL`，安装前不停止旧进程；② `lib.rs` 中 `ensure_local_netease_api_service` 调用 `command.spawn()` 后立刻丢弃 `Child` 句柄，托管的 `node.exe` 在 app 退出后孤儿化、再次启动时也无法复用；③ `NSIS_HOOK_POSTUNINSTALL` 默认清空 `$APPDATA/com.ome.music`、`$LOCALAPPDATA/com.ome.music` 和 Windows 凭据管理器密钥，覆盖升级或重装会无提示丢失登录态、曲库与设置。同时扫码登录后第一次搜索网易云时若服务未就绪会静默失败，用户体验上把"启动服务"误读为"编译环境"。本阶段目标是让 Ome Music 像普通 Windows 音乐软件一样"下载 → 双击安装 → 打开 → 直接使用"。

## What Changes

- 新增 `NSIS_HOOK_PREINSTALL`：安装/升级前精准停止 Ome Music 主进程与安装目录下 `$INSTDIR\resources\node\node.exe`，绝不误杀系统其他 node.exe；停止失败时弹出友好提示而非直接报错到 node.exe 写入。
- **BREAKING** 重写 `NSIS_HOOK_POSTUNINSTALL`：普通卸载默认保留用户数据（数据库、缓存、登录态、keyring）；不再默认删除 `$APPDATA/com.ome.music` 与 `$LOCALAPPDATA/com.ome.music`；不再默认清理凭据管理器密钥。
- 在 `AppState` 中保存托管 NetEase runtime 的 `Child` 句柄与运行状态；`ensure_netease_api_service` 复用已运行进程而非重复 spawn；新增 app 退出钩子优雅 kill 托管 node.exe，超时后强制 kill。
- 在 `TopSearch` / `MusicSourceProvider` 搜索路径中加入 NetEase service gate：未就绪时先 `ensureNeteaseApiService`，展示启动进度，ready 后再搜索，失败给出明确原因与重试。
- 新增 NetEase Source Service Status UI（分阶段进度：检查运行时 → 检查 API 文件 → 启动服务 → 等待健康检查 → Ready），文案严禁出现"编译 / npm install / Node.js / 开发服务器"。
- 版本号统一升级到 `0.3.5`，同步 README / README.zh-CN / BUILD / CHANGELOG / release workflow 文案。
- 确认 release 构建路径不执行 first-run `npm install` / `npx`（dev-only 代码已被 `#[cfg(debug_assertions)]` 守卫）；在文档中明确"安装包是完整可用环境"。

## Impact

- Affected specs: 无（首个 spec）
- Affected code:
  - `src-tauri/nsis-hooks.nsh`（核心：preinstall + 重写 postuninstall）
  - `src-tauri/src/lib.rs`（`ensure_local_netease_api_service`、`AppState`、app 退出钩子、`ManagedNeteaseApiRuntime` 生命周期）
  - `src-tauri/tauri.conf.json`（版本号 0.3.5；确认 `bundle.resources` 仍含 `resources/`）
  - `src/features/musicSources/provider.ts`（`ensureNeteaseApiService` 返回分阶段状态、`NetEaseServiceStatus` 扩展阶段字段）
  - `src/components/TopSearch.tsx`（搜索前的 NetEase service gate + 启动进度展示）
  - `src/components/ProviderSettingsPanel.tsx`（设置页 NetEase 服务状态卡片）
  - `package.json` / `src-tauri/Cargo.toml`（0.3.5）
  - `README.md` / `README.zh-CN.md` / `docs/BUILD.md` / `docs/CHANGELOG.md`（0.3.5 + 安装行为说明）
  - `.github/workflows/release.yml`（release body 文案，如有版本引用）

## ADDED Requirements

### Requirement: 安装前旧进程优雅停止

系统 SHALL 在 NSIS 安装/升级写入 `resources/node/node.exe` 之前，精准停止 Ome Music 主进程与安装目录 `$INSTDIR\resources\node\node.exe`，且 MUST NOT 终止系统上其他 node.exe。

#### Scenario: 旧版 Ome Music 正在运行时升级安装
- **WHEN** 用户在旧版 Ome Music 运行时双击新版安装包
- **THEN** 安装器在 preinstall 阶段停止 Ome Music 主进程与安装目录下托管 node.exe
- **AND** 等待 1-3 秒后确认文件未被占用
- **AND** 成功覆盖 `resources/node/node.exe`
- **AND** 不弹出 `Error opening file for writing` 错误

#### Scenario: 托管 node.exe 仍被占用且无法停止
- **WHEN** preinstall 停止后文件仍被占用
- **THEN** 安装器弹出友好提示 "Ome Music is still running in the background. Please close it and retry installation."
- **AND** 不直接报错到 node.exe 写入失败

#### Scenario: 系统上存在其他 node.exe
- **WHEN** 用户机器上运行着其他项目的 node.exe
- **THEN** 安装器仅按 ExecutablePath 等于 `$INSTDIR\resources\node\node.exe` 或 CommandLine 包含 `$INSTDIR\resources\netease-runtime` 精准匹配
- **AND** 不影响其他 node.exe 进程

### Requirement: 托管 NetEase runtime 进程生命周期管理

系统 SHALL 在 `AppState` 中保存托管 NetEase runtime 的 `Child` 句柄与运行状态，并在 app 退出时优雅停止、超时强制 kill。

#### Scenario: 复用已运行托管进程
- **WHEN** `ensure_netease_api_service` 被多次调用且托管进程仍在运行
- **THEN** 复用现有 `Child` 句柄而非重复 spawn
- **AND** 不会在系统中产生多个 `resources/node/node.exe` 进程

#### Scenario: app 退出清理子进程
- **WHEN** 用户关闭 Ome Music
- **THEN** app 退出钩子优雅停止托管 node.exe
- **WHEN** 优雅停止超时
- **THEN** 强制 kill 托管 node.exe
- **AND** 安装目录下不再残留孤儿 node.exe

### Requirement: NetEase 服务启动状态可见

系统 SHALL 在 UI 中展示 NetEase Source Service 的分阶段启动状态与进度。

#### Scenario: 首次启动服务展示进度
- **WHEN** 用户扫码登录后 NetEase 服务未就绪
- **THEN** UI 展示分阶段进度（10% 检查运行时 → 30% 检查 API 文件 → 50% 启动服务 → 70% 等待健康检查 → 100% Ready）
- **AND** 文案为 "正在启动网易云音乐源 / 第一次启动可能需要几秒 / 本地音乐可以立即播放"
- **AND** 不出现 "编译 / npm install / Node.js / 开发服务器" 文案

#### Scenario: 启动失败展示原因
- **WHEN** NetEase 服务启动失败
- **THEN** UI 展示明确失败原因
- **AND** 提供"重试"按钮

### Requirement: 搜索网易云前服务就绪门控

系统 SHALL 在搜索网易云歌曲前确认 NetEase API service ready，未就绪时自动启动并展示进度。

#### Scenario: 首次搜索自动启动服务
- **WHEN** 用户扫码登录后立即搜索网易云歌曲且服务未就绪
- **THEN** 搜索流程调用 `ensureNeteaseApiService`
- **AND** 展示 "正在启动音乐源" 提示
- **AND** 服务 ready 后自动继续搜索
- **AND** 不出现点击搜索无反应

#### Scenario: 服务启动失败搜索给出重试
- **WHEN** `ensureNeteaseApiService` 启动失败
- **THEN** 搜索区域显示错误原因与"重试"按钮
- **AND** 不让用户误以为扫码登录失败

## MODIFIED Requirements

### Requirement: 卸载数据清理策略

普通卸载 SHALL 默认保留用户数据（数据库、缓存、登录态、keyring、注册表项）。原 `NSIS_HOOK_POSTUNINSTALL` 中删除 `$APPDATA/com.ome.music`、`$LOCALAPPDATA/com.ome.music`、凭据管理器密钥、注册表项的行为被移除。

#### Scenario: 卸载默认保留数据
- **WHEN** 用户通过控制面板/设置卸载 Ome Music
- **THEN** 卸载仅移除程序文件
- **AND** 保留 `$APPDATA/com.ome.music` 与 `$LOCALAPPDATA/com.ome.music`
- **AND** 保留 Windows 凭据管理器中的网易云/Bilibili/LLM 密钥
- **AND** 保留注册表项

#### Scenario: 覆盖升级不丢数据
- **WHEN** 用户在新版安装包上覆盖升级
- **THEN** 登录态、曲库数据库、缓存、设置全部保留
- **AND** 不出现无提示数据清空

### Requirement: 版本号统一为 0.3.5

`package.json`、`src-tauri/Cargo.toml`、`src-tauri/tauri.conf.json`、README、README.zh-CN、docs/BUILD、docs/CHANGELOG、release workflow 文案 SHALL 统一为 `0.3.5`，并由 `docs:check` 持续强制。

## REMOVED Requirements

### Requirement: 卸载默认清空所有用户数据

**Reason**: 危险行为，覆盖升级与重装会无提示丢失登录态、曲库、设置、密钥；违背普通 Windows 软件预期。

**Migration**: 卸载默认保留数据；如需完全清理，用户可在 Settings 中清理缓存或手动删除 `$APPDATA/com.ome.music`、`$LOCALAPPDATA/com.ome.music`。文档明确说明此策略。
