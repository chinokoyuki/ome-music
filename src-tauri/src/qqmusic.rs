// ── QQ Music 平台适配 — Rust 后端模块 ────────────────────────────
// QQ Music platform adapter — Rust backend module.
//
// QQ 音乐 API 均为标准 HTTP + JSON，参照 Bilibili 适配器的 reqwest 直连模式。
// 独立模块封装 sign 算法、HTTP 请求、响应解析和媒体代理。
//
// 声音品质文件名对照（Quality filename reference）:
//   standard  → M800 (128k mp3,  免费)
//   higher    → M500 (320k mp3,  需绿钻)
//   exhigh    → M500 (320k mp3,  复用 higher)
//   lossless  → F000 (flac,      需绿钻)
//   hires     → RS01 (flac,      需超级会员)

use std::collections::HashMap;

use base64::{engine::general_purpose, Engine as _};
use serde::{Deserialize, Serialize};

use crate::{
    is_proxyable_remote_url, register_media_proxy, AppState, PlayableUrlDto, SourceSongDto,
};

// ── 常量 ──────────────────────────────────────────────────────────────

pub const QQMUSIC_KEYRING_SERVICE: &str = "ome.music.source.qqmusic";
pub const QQMUSIC_KEYRING_ACCOUNT: &str = "local";
pub const QQMUSIC_DEFAULT_BASE_URL: &str = "https://c.y.qq.com";
pub const QQMUSIC_U_BASE_URL: &str = "https://u.y.qq.com";
pub const QQMUSIC_REFERER: &str = "https://y.qq.com/portal/player.html";
pub const QQMUSIC_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";
const QQMUSIC_MAX_MEDIA_CANDIDATES: usize = 12;

fn is_trusted_qqmusic_api_host(host: &str) -> bool {
    matches!(host, "c.y.qq.com" | "u.y.qq.com" | "y.qq.com")
}

fn is_trusted_qqmusic_login_host(host: &str) -> bool {
    host == "y.qq.com"
        || host.ends_with(".y.qq.com")
        || host == "graph.qq.com"
        || host == "ptlogin2.qq.com"
        || host.ends_with(".ptlogin2.qq.com")
}

pub fn is_trusted_qqmusic_webview_url(value: &str) -> bool {
    reqwest::Url::parse(value)
        .ok()
        .filter(|url| {
            url.scheme() == "https"
                && url.username().is_empty()
                && url.password().is_none()
                && url.port_or_known_default() == Some(443)
        })
        .and_then(|url| {
            url.host_str()
                .map(|host| host == "y.qq.com" || host.ends_with(".y.qq.com"))
        })
        .unwrap_or(false)
}

fn is_trusted_qqmusic_media_host(host: &str) -> bool {
    host == "y.gtimg.cn"
        || host.ends_with(".gtimg.cn")
        || host == "q.qlogo.cn"
        || host == "qqmusic.qq.com"
        || host.ends_with(".qqmusic.qq.com")
        || host == "aqqmusic.tc.qq.com"
}

fn parse_trusted_qqmusic_api_url(value: &str) -> Result<reqwest::Url, String> {
    let url = reqwest::Url::parse(value)
        .map_err(|_| "QQ 音乐地址无效 / Invalid QQ Music URL.".to_string())?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port_or_known_default() != Some(443)
        || !url
            .host_str()
            .map(is_trusted_qqmusic_api_host)
            .unwrap_or(false)
    {
        return Err(
            "仅允许 QQ 音乐官方 HTTPS 地址 / Only official QQ Music HTTPS endpoints are allowed."
                .to_string(),
        );
    }
    Ok(url)
}

fn parse_trusted_qqmusic_login_url(value: &str) -> Result<reqwest::Url, String> {
    let url = reqwest::Url::parse(value)
        .map_err(|_| "QQ 登录跳转地址无效 / Invalid QQ login redirect.".to_string())?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port_or_known_default() != Some(443)
        || !url
            .host_str()
            .map(is_trusted_qqmusic_login_host)
            .unwrap_or(false)
    {
        return Err(
            "已拒绝不受信任的 QQ 登录跳转 / Untrusted QQ login redirect was blocked.".to_string(),
        );
    }
    Ok(url)
}

pub fn is_trusted_qqmusic_media_url(value: &str) -> bool {
    reqwest::Url::parse(value)
        .ok()
        .filter(|url| {
            matches!(url.scheme(), "http" | "https")
                && url.username().is_empty()
                && url.password().is_none()
                && matches!(url.port_or_known_default(), Some(80 | 443))
        })
        .and_then(|url| url.host_str().map(is_trusted_qqmusic_media_host))
        .unwrap_or(false)
}

/// HTML 片段提取辅助：在第一个 ASCII 终止符处截断；若超长（500 字节内无
/// 终止符），按字符边界截断，绝不落在多字节 UTF-8 字符中间（否则
/// `&s[..end]` 会在 release panic=abort 下崩掉整个进程）。
fn truncate_html_fragment(value: &str) -> &str {
    const MAX_HINT: usize = 500;
    if let Some(pos) = value.find(['"', '\'', '>', ';', ')']) {
        return &value[..pos];
    }
    let byte_end = value.len().min(MAX_HINT);
    let boundary = value.floor_char_boundary(byte_end);
    &value[..boundary]
}

pub fn validate_qqmusic_base_url(value: &str) -> Result<String, String> {
    let mut url = parse_trusted_qqmusic_api_url(value.trim().trim_end_matches('/'))?;
    url.set_query(None);
    url.set_fragment(None);
    Ok(url.as_str().trim_end_matches('/').to_string())
}

fn qqmusic_api_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            let trusted = attempt.url().scheme() == "https"
                && attempt
                    .url()
                    .host_str()
                    .map(is_trusted_qqmusic_api_host)
                    .unwrap_or(false);
            if !trusted {
                attempt.stop()
            } else if attempt.previous().len() >= 3 {
                attempt.error("too many QQ Music redirects")
            } else {
                attempt.follow()
            }
        }))
        .build()
        .map_err(|error| format!("QQ 音乐网络客户端初始化失败 / QQ Music client failed: {error}"))
}

/// QQ 登录浏览器特征头 / QQ Login browser-like headers
/// ptlogin2 服务器会检查这些头，缺失时返回 403 Forbidden（反自动化机制）。
/// ptlogin2 server checks these headers; missing them triggers 403 (anti-bot).
fn qq_login_browser_headers() -> reqwest::header::HeaderMap {
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert("User-Agent", QQMUSIC_UA.parse().unwrap());
    headers.insert("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,image/apng,*/*;q=0.8,application/signed-exchange;v=b3;q=0.7".parse().unwrap());
    headers.insert(
        "Accept-Language",
        "zh-CN,zh;q=0.9,en;q=0.8".parse().unwrap(),
    );
    headers.insert("Sec-Fetch-Dest", "document".parse().unwrap());
    headers.insert("Sec-Fetch-Mode", "navigate".parse().unwrap());
    headers.insert("Sec-Fetch-Site", "same-origin".parse().unwrap());
    headers.insert("Sec-Fetch-User", "?1".parse().unwrap());
    headers.insert("Upgrade-Insecure-Requests", "1".parse().unwrap());
    headers
}

/// 音质 → filename 前缀映射
/// 注意：QQ 音乐文件名前缀不代表码率高低，而是约定好的编码格式标识。
///   M500 = 128kbps MP3 (标准音质，免费歌曲无需 VIP)
///   M800 = 320kbps MP3 (高品质，多数歌曲需要绿钻)
///   F000 = FLAC 无损 (需要绿钻)
///   RS01 = Hi-Res (需要超级会员)
pub const QQMUSIC_QUALITY_MAP: &[(&str, &str, &str)] = &[
    ("standard", "C400", "m4a"),  // 128k m4a  免费 / Free
    ("higher", "M500", "mp3"),    // 128k mp3  免费 / Free
    ("exhigh", "M800", "mp3"),    // 320k mp3  绿钻 / Green VIP
    ("lossless", "F000", "flac"), // 无损 / Lossless
    ("hires", "RS01", "flac"),    // Hi-Res 超级会员 / Super VIP
];

// ── DTOs ──────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct QQMusicSourceConfigDto {
    pub enabled: bool,
    pub base_url: String,
    pub has_token: bool,
    pub masked_token: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveQQMusicSourceConfigPayload {
    pub enabled: bool,
    pub base_url: Option<String>,
    pub token: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ResolvedQQMusicSourceConfig {
    pub enabled: bool,
    pub base_url: String,
    pub token: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct QQMusicLoginStatusDto {
    pub logged_in: bool,
    pub credential_present: bool,
    pub status: QQMusicAuthState,
    pub uin: String,
    pub nickname: String,
    pub avatar_url: String,
    pub vip_type: String, // "none" | "green" | "super"
    pub message: String,
}

#[derive(Debug, Serialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum QQMusicAuthState {
    SignedOut,
    CredentialPresent,
    Verifying,
    Authenticated,
    Expired,
    Unknown,
    Failed,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct QQMusicQrLoginDto {
    pub url: String,
    pub key: String,
    pub cookies: String,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct QQMusicQrCheckDto {
    pub status: String, // "waiting" | "scanned" | "confirmed" | "expired" | "failed"
    pub cookie: Option<String>,
    pub message: Option<String>,
    /// 每次 poll 后最新累积的登录 cookie（含本轮的 Set-Cookie 回流）。
    /// 即使状态仍为 waiting/scanned 也返回，供 Rust 侧 session map 更新，
    /// 保证连续 poll 共享同一组不断更新的 cookie（qr session continuity）。
    pub cookies: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QQMusicSourceSongPayload {
    pub song_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QQMusicPlayablePayload {
    pub song_id: String,
    pub quality: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QQMusicSearchPayload {
    pub query: String,
    pub page: Option<u32>,
    pub page_size: Option<u32>,
}

// ── Sign 算法 ────────────────────────────────────────────────────────

/// QQ音乐请求签名算法。
/// 逆向自 y.qq.com Web 端 JS 的 getSign 函数。
/// 纯计算，无网络依赖。
pub fn qqmusic_get_sign(data: &str) -> String {
    const MAP: [u8; 16] = [
        0x61, 0x32, 0x33, 0x61, 0x31, 0x62, 0x34, 0x33, 0x63, 0x35, 0x61, 0x36, 0x64, 0x37, 0x65,
        0x38,
    ];
    let md5 = format!("{:x}", md5::compute(data.as_bytes()));
    let mut sign = String::with_capacity(36);
    sign.push_str("zzc");
    for &m in MAP.iter().take(16) {
        let idx = (m as usize) % md5.len();
        sign.push(md5.as_bytes()[idx] as char);
    }
    sign
}

/// QQ 登录 ptqrtoken 生成 (hash33 算法)
pub fn qqmusic_hash33(s: &str) -> u32 {
    let mut h: u32 = 0;
    for c in s.chars() {
        h = h.wrapping_add(h.wrapping_shl(5)).wrapping_add(c as u32);
    }
    h & 0x7fffffff
}

/// QQ 音乐 g_tk (ACSRF token) 生成
/// 逆向自 y.qq.com 前端 JS 的 getACSRFToken 函数
/// 算法: hash = 5381; for each char: hash += (hash << 5) + charCode; return hash & 0x7fffffff
fn qqmusic_gtk(key: &str) -> u32 {
    let mut hash: u32 = 5381;
    for c in key.chars() {
        hash = hash
            .wrapping_add(hash.wrapping_shl(5))
            .wrapping_add(c as u32);
    }
    hash & 0x7fffffff
}

/// 从 cookie 字符串中提取指定名称的 cookie 值（原始值，不做清理）
pub fn extract_cookie_raw(cookie: &str, name: &str) -> Option<String> {
    let prefix = format!("{}=", name);
    for part in cookie.split(';') {
        let part = part.trim();
        if part.to_lowercase().starts_with(&prefix.to_lowercase()) {
            if let Some((_, val)) = part.split_once('=') {
                let val = val.trim();
                if !val.is_empty() {
                    return Some(val.to_string());
                }
            }
        }
    }
    None
}

/// 单个 cookie 条目（仅 name/value；attributes 在解析时剥离）。
/// 最终发送请求时只构造 `name=value; name=value`，绝不把
/// Expires/Path/Domain/HttpOnly/Secure/SameSite 作为 cookie 值发回服务器。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CookieEntry {
    pub name: String,
    pub value: String,
}

impl CookieEntry {
    /// 把单条 Set-Cookie 头拆成 name/value。
    /// 关键：value 可能包含 `=`（base64 填充，例如 qm_keyst/qqmusic_key），
    /// 必须用 `split_once('=')` 而不是 `split('=').nth(1)`——后者会把
    /// `qm_keyst=abc==` 截断成 `abc`，导致保存的凭据损坏、后续 verify 失败。
    pub fn from_set_cookie(raw: &str) -> Option<Self> {
        let name_value = raw.split(';').next()?.trim();
        if name_value.is_empty() {
            return None;
        }
        let (name, value) = name_value.split_once('=')?;
        let name = name.trim();
        if name.is_empty() {
            return None;
        }
        // 保留 value 原样（含内部 `=`），仅去首尾空白。
        Some(CookieEntry {
            name: name.to_string(),
            value: value.trim().to_string(),
        })
    }
}

/// 把 Cookie header 格式（`a=1; b=2`）解析为条目列表。
pub(crate) fn parse_cookie_header(cookie: &str) -> Vec<CookieEntry> {
    cookie
        .split(';')
        .filter_map(|part| {
            let part = part.trim();
            if part.is_empty() {
                return None;
            }
            let (name, value) = part.split_once('=')?;
            let name = name.trim();
            if name.is_empty() {
                return None;
            }
            Some(CookieEntry {
                name: name.to_string(),
                value: value.trim().to_string(),
            })
        })
        .collect()
}

/// 合并 cookie 条目列表为发送用的 Cookie header 字符串。
/// 规则：
///   - 同名 cookie 后面的覆盖前面的；
///   - 空 value 的条目（服务器删除 cookie 的 Set-Cookie: name=; Expires=...）
///     跳过：不清除旧值（登录过程中服务器可能先发空值再发新值，或仅删除
///     其他域的同名 cookie；按主键名合并时保留最近的有效值更安全）；
///   - 顺序保持首次出现的顺序（新有效的同名条目原地替换值）。
pub(crate) fn merge_cookie_entries(entries: Vec<CookieEntry>) -> String {
    let mut order: Vec<String> = Vec::new();
    let mut map: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for entry in entries {
        if entry.value.is_empty() {
            continue;
        }
        if !map.contains_key(&entry.name) {
            order.push(entry.name.clone());
        }
        map.insert(entry.name.clone(), entry.value);
    }
    order
        .into_iter()
        .filter_map(|name| map.get(&name).map(|value| format!("{name}={value}")))
        .collect::<Vec<_>>()
        .join("; ")
}

/// 把一个 Set-Cookie 头列表合并进现有 cookie header 字符串（QQ 登录链）。
/// returns: 更新后的 cookie header。
pub(crate) fn merge_set_cookie_header(existing: &str, set_cookies: &[String]) -> String {
    let mut entries = parse_cookie_header(existing);
    for raw in set_cookies {
        if let Some(entry) = CookieEntry::from_set_cookie(raw) {
            // 同名替换（保持原位置），空值跳过（不清除有效旧值）。
            if entry.value.is_empty() {
                continue;
            }
            if let Some(slot) = entries.iter_mut().find(|e| e.name == entry.name) {
                slot.value = entry.value;
            } else {
                entries.push(entry);
            }
        }
    }
    merge_cookie_entries(entries)
}

/// 提取 QQ 音乐签名 key / Extract QQ Music signing key
/// 现代 QQ 音乐 web 使用 qm_keyst 字段（非 qqmusic_key）。
/// Modern QQ Music web uses qm_keyst (not qqmusic_key).
/// 优先 qm_keyst，回退到 qqmusic_key 以兼容旧 cookie。
pub fn extract_qqmusic_signing_key(cookie: &str) -> Option<String> {
    extract_cookie_raw(cookie, "qm_keyst").or_else(|| extract_cookie_raw(cookie, "qqmusic_key"))
}

pub fn qqmusic_credential_is_complete(cookie: &str) -> bool {
    let has_signing_key = extract_qqmusic_signing_key(cookie).is_some()
        || extract_cookie_raw(cookie, "p_skey").is_some()
        || extract_cookie_raw(cookie, "superkey").is_some()
        || extract_cookie_raw(cookie, "psrf_qqaccess_token").is_some();
    let has_qq_identity = ["uin", "pt2gguin", "superuin", "p_uin"]
        .iter()
        .any(|name| extract_cookie_raw(cookie, name).is_some());
    let has_wechat_uin = ["wxuin", "euin"]
        .iter()
        .any(|name| extract_cookie_raw(cookie, name).is_some());
    let has_wechat_openid = ["wxopenid", "psrf_qqopenid"]
        .iter()
        .any(|name| extract_cookie_raw(cookie, name).is_some());
    let has_wechat_unionid = ["wxunionid", "psrf_qqunionid"]
        .iter()
        .any(|name| extract_cookie_raw(cookie, name).is_some());
    let has_wechat_refresh = ["wxrefresh_token", "psrf_qqrefresh_token"]
        .iter()
        .any(|name| extract_cookie_raw(cookie, name).is_some());
    let has_wechat_identity =
        has_wechat_uin && has_wechat_openid && has_wechat_unionid && has_wechat_refresh;
    has_signing_key && (has_qq_identity || has_wechat_identity)
}

pub fn classify_qqmusic_auth_failure(
    error: &str,
    credential_present: bool,
    credential_complete: bool,
) -> QQMusicAuthState {
    if !credential_present {
        return QQMusicAuthState::SignedOut;
    }
    if !credential_complete {
        return QQMusicAuthState::Failed;
    }
    let normalized = error.to_ascii_lowercase();
    if normalized.contains("expired")
        || normalized.contains("session_invalid")
        || normalized.contains("登录过期")
        || normalized.contains("会话过期")
    {
        QQMusicAuthState::Expired
    } else {
        QQMusicAuthState::Unknown
    }
}

/// 从 config 中提取 g_tk 和 g_tk_new_20200303
/// g_tk_new_20200303 = gtk(qm_keyst || qqmusic_key || p_skey || skey || p_lskey || lskey)
/// g_tk = gtk(skey || qm_keyst || qqmusic_key || p_skey)
fn resolve_qqmusic_gtk(config: &ResolvedQQMusicSourceConfig) -> (u32, u32) {
    let cookie = config.token.as_deref().unwrap_or("");
    // 优先 qm_keyst（现代 QQ 音乐 web），回退 qqmusic_key
    let signing_key = extract_qqmusic_signing_key(cookie);
    let p_skey = extract_cookie_raw(cookie, "p_skey");
    let skey = extract_cookie_raw(cookie, "skey");
    let p_lskey = extract_cookie_raw(cookie, "p_lskey");
    let lskey = extract_cookie_raw(cookie, "lskey");

    // g_tk_new_20200303: signing_key || p_skey || skey || p_lskey || lskey
    let key_new = signing_key
        .as_deref()
        .or(p_skey.as_deref())
        .or(skey.as_deref())
        .or(p_lskey.as_deref())
        .or(lskey.as_deref())
        .unwrap_or("");
    let g_tk_new = qqmusic_gtk(key_new);

    // g_tk: skey || signing_key || p_skey
    let key_old = skey
        .as_deref()
        .or(signing_key.as_deref())
        .or(p_skey.as_deref())
        .unwrap_or("");
    let g_tk = qqmusic_gtk(key_old);

    (g_tk, g_tk_new)
}

// ── HTTP 请求辅助 ────────────────────────────────────────────────────

/// 清空 URL/purl 查询字符串中的 uin 参数值（uin=123 → uin=）
/// 用于服务器未认证 cookie 时，让 purl 的 uin 与匿名 vkey 归属一致。
/// Clear the uin param value in a URL/purl query string (uin=123 → uin=).
/// Used when server didn't authenticate cookie, to match the anonymous vkey.
fn clear_uin_param(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        // 找 "uin=" 的位置
        if i + 4 <= bytes.len() && &bytes[i..i + 4] == b"uin=" {
            // 前一个字符必须是 ? 或 &（确实是查询参数）
            let prev_ok = i > 0 && (bytes[i - 1] == b'?' || bytes[i - 1] == b'&');
            if prev_ok {
                result.push_str("uin=");
                i += 4;
                // 跳过数字，直到 & 或 # 或字符串结束
                while i < bytes.len() && bytes[i] != b'&' && bytes[i] != b'#' {
                    i += 1;
                }
                continue;
            }
        }
        result.push(bytes[i] as char);
        i += 1;
    }
    result
}

/// 构造 QQ 音乐 comm 公共参数对象
/// 包含 uin/format/ct/cv，匹配 QQ 音乐 web 客户端实际发送的格式。
/// comm.uin 缺失时 API 会将请求视为匿名（uin:""），导致 500005 和无效 purl。
/// Build the comm object for musicu.fcg requests.
/// Without comm.uin, the API treats the request as anonymous.
fn build_qqmusic_comm(config: &ResolvedQQMusicSourceConfig) -> serde_json::Value {
    let uin_str = resolve_qqmusic_uin(config);
    let cookie = config.token.as_deref().unwrap_or("");
    let authst = extract_qqmusic_signing_key(cookie)
        .or_else(|| extract_cookie_raw(cookie, "p_skey"))
        .or_else(|| extract_cookie_raw(cookie, "skey"))
        .unwrap_or_default();
    let (g_tk, g_tk_new) = resolve_qqmusic_gtk(config);
    serde_json::json!({
        "uin": uin_str,
        "format": "json",
        "ct": 24,
        "cv": 4_747_474,
        "inCharset": "utf-8",
        "outCharset": "utf-8",
        "notice": 0,
        "platform": "yqq.json",
        "needNewCode": 1,
        "g_tk": g_tk,
        "g_tk_new_20200303": g_tk_new,
        "authst": authst
    })
}

/// 构造 QQ 音乐请求的默认 headers（Referer + UA 伪造）
fn qqmusic_default_headers() -> HashMap<&'static str, String> {
    let mut headers = HashMap::new();
    headers.insert("Referer", QQMUSIC_REFERER.to_string());
    headers.insert("User-Agent", QQMUSIC_UA.to_string());
    headers.insert("Accept", "application/json, text/plain, */*".to_string());
    headers
}

/// 发送 QQ 音乐 GET 请求并返回文本
pub async fn request_qqmusic_text(
    config: &ResolvedQQMusicSourceConfig,
    url: &str,
    query: &[(&str, &str)],
    extra_headers: Option<&HashMap<&str, String>>,
) -> Result<String, String> {
    let client = qqmusic_api_client()?;
    let trusted_url = parse_trusted_qqmusic_api_url(url)?;
    let mut req = client.get(trusted_url).query(query);

    for (key, value) in qqmusic_default_headers() {
        req = req.header(key, value);
    }
    if let Some(extra) = extra_headers {
        for (key, value) in extra {
            req = req.header(*key, value.as_str());
        }
    }
    if let Some(ref cookie) = config.token {
        req = req.header("Cookie", cookie.as_str());
    }

    let response = req
        .timeout(std::time::Duration::from_secs(12))
        .send()
        .await
        .map_err(|error| {
            if error.is_timeout() {
                "timeout: QQ音乐请求超时 / QQ Music request timed out.".to_string()
            } else {
                "api_failed: QQ音乐请求失败 / QQ Music request failed.".to_string()
            }
        })?;

    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|_| "QQ音乐响应读取失败 / Failed to read QQ Music response.".to_string())?;

    if status == 412 || status == 503 {
        return Err(
            "rate_limited: 请求过于频繁，请稍后再试 / Rate limited, please try later.".to_string(),
        );
    }
    if text.trim_start().starts_with("<!DOCTYPE") || text.trim_start().starts_with("<html") {
        return Err("QQ音乐需要网页验证 / QQ Music requires web verification.".to_string());
    }

    Ok(text)
}

/// 发送 QQ 音乐 musicu.fcg 请求
/// QQ音乐 web 客户端使用 GET + data 查询参数（整个 JSON body URL 编码后放入 URL）。
/// POST 方式会导致 500005 错误，必须用 GET。
pub async fn request_qqmusic_json_post(
    config: &ResolvedQQMusicSourceConfig,
    url: &str,
    body: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    let client = qqmusic_api_client()?;
    let trusted_url = parse_trusted_qqmusic_api_url(url)?;

    let body_str = serde_json::to_string(body).unwrap_or_default();

    // 简化 URL：仅 format=json + data 参数
    // QQ音乐 musicu.fcg 非加密接口仅需 format 和 data，
    // 额外的 g_tk/loginUin/hostUin 等参数会导致部分 API 返回 500005。
    // Simplified URL: only format=json + data param.
    // The non-encrypted musicu.fcg endpoint only needs format and data;
    // extra params like g_tk/loginUin/hostUin cause some APIs to return 500005.
    let encoded_body = urlencoding::encode(&body_str);
    let full_url = format!("{}?format=json&data={}", trusted_url, encoded_body);
    let mut req = client.get(&full_url);

    req = req.header("Referer", "https://y.qq.com/");
    req = req.header("Origin", "https://y.qq.com");
    req = req.header("User-Agent", QQMUSIC_UA);
    req = req.header("Accept", "application/json, text/plain, */*");
    if let Some(ref cookie) = config.token {
        // 确保 cookie 同时包含 qm_keyst 和 qqmusic_key（值相同）
        // QQ 音乐 API 可能检查 qqmusic_key 字段名
        // Ensure cookie has both qm_keyst and qqmusic_key (same value)
        let mut cookie_full = cookie.clone();
        if let Some(qm_keyst_val) = extract_cookie_raw(cookie, "qm_keyst") {
            if extract_cookie_raw(cookie, "qqmusic_key").is_none() {
                cookie_full = format!("{}; qqmusic_key={}", cookie_full, qm_keyst_val);
                eprintln!("[QQMusic] cookie 补充 qqmusic_key 字段（值同 qm_keyst）");
            }
        }
        req = req.header("Cookie", cookie_full.as_str());
    }

    let response = req
        .timeout(std::time::Duration::from_secs(12))
        .send()
        .await
        .map_err(|error| {
            if error.is_timeout() {
                "timeout: QQ音乐请求超时 / QQ Music request timed out.".to_string()
            } else {
                "api_failed: QQ音乐请求失败 / QQ Music request failed.".to_string()
            }
        })?;

    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|_| "QQ音乐响应读取失败 / Failed to read QQ Music response.".to_string())?;

    #[cfg(debug_assertions)]
    eprintln!(
        "[QQMusic] API response: status={status}, body_length={}",
        text.len()
    );

    if status == 412 || status == 503 {
        return Err("rate_limited: 请求过于频繁，请稍后再试 / Rate limited.".to_string());
    }
    if text.trim_start().starts_with("<!DOCTYPE") || text.trim_start().starts_with("<html") {
        return Err("QQ音乐需要网页验证 / QQ Music requires web verification.".to_string());
    }

    serde_json::from_str::<serde_json::Value>(&text).map_err(|_| {
        format!(
            "QQ音乐 JSON 解析失败 / Failed to parse QQ Music JSON (status={status}, body_length={})",
            text.len()
        )
    })
}

/// 发送现代 QQ 音乐 JSON POST 请求。
/// `music.vkey.GetVkey.UrlGetVkey` 使用 JSON 请求体；继续复用 GET + data
/// 会让服务端回落到匿名或返回旧版无效 vkey。
/// Send a modern QQ Music JSON POST request. UrlGetVkey expects a JSON body;
/// using the legacy GET wrapper can silently downgrade the session to anonymous.
async fn request_qqmusic_json_body(
    config: &ResolvedQQMusicSourceConfig,
    url: &str,
    body: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    let client = qqmusic_api_client()?;
    let trusted_url = parse_trusted_qqmusic_api_url(url)?;
    let mut request = client
        .post(trusted_url)
        .header("Referer", "https://y.qq.com/")
        .header("Origin", "https://y.qq.com")
        .header("User-Agent", QQMUSIC_UA)
        .header("Accept", "application/json, text/plain, */*")
        .json(body);

    if let Some(cookie) = config.token.as_deref() {
        request = request.header("Cookie", cookie);
    }

    let response = request
        .timeout(std::time::Duration::from_secs(12))
        .send()
        .await
        .map_err(|error| {
            if error.is_timeout() {
                "timeout: QQ音乐播放地址请求超时 / QQ Music playback request timed out.".to_string()
            } else {
                "api_failed: QQ音乐播放地址请求失败 / QQ Music playback request failed.".to_string()
            }
        })?;
    let status = response.status();
    let text = response.text().await.map_err(|_| {
        "QQ音乐播放响应读取失败 / Failed to read QQ Music playback response.".to_string()
    })?;

    #[cfg(debug_assertions)]
    eprintln!(
        "[QQMusic] modern vkey response: status={status}, body_length={}",
        text.len()
    );

    if status == 412 || status == 503 {
        return Err("rate_limited: 请求过于频繁，请稍后再试 / Rate limited.".to_string());
    }
    if !status.is_success() {
        return Err(format!(
            "api_failed: QQ音乐播放地址返回状态 {} / QQ Music playback returned status {}.",
            status.as_u16(),
            status.as_u16()
        ));
    }
    if text.trim_start().starts_with("<!DOCTYPE") || text.trim_start().starts_with("<html") {
        return Err("QQ音乐需要网页验证 / QQ Music requires web verification.".to_string());
    }

    serde_json::from_str::<serde_json::Value>(&text).map_err(|_| {
        format!(
            "QQ音乐播放响应解析失败 / Failed to parse QQ Music playback JSON (body_length={})",
            text.len()
        )
    })
}

/// 发送 QQ 音乐 API 请求（匿名，不发送 Cookie）
pub async fn request_qqmusic_json_post_anon(
    url: &str,
    body: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    let client = qqmusic_api_client()?;
    let trusted_url = parse_trusted_qqmusic_api_url(url)?;
    let body_str = serde_json::to_string(body).unwrap_or_default();
    let encoded_body = urlencoding::encode(&body_str);
    let full_url = format!("{}?format=json&data={}", trusted_url, encoded_body);
    let mut req = client.get(&full_url);
    req = req.header("Referer", "https://y.qq.com/");
    req = req.header("Origin", "https://y.qq.com");
    req = req.header("User-Agent", QQMUSIC_UA);
    req = req.header("Accept", "application/json, text/plain, */*");

    let response = req
        .timeout(std::time::Duration::from_secs(12))
        .send()
        .await
        .map_err(|error| {
            if error.is_timeout() {
                "timeout: QQ音乐请求超时 / QQ Music request timed out.".to_string()
            } else {
                "api_failed: QQ音乐请求失败 / QQ Music request failed.".to_string()
            }
        })?;

    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|_| "QQ音乐响应读取失败 / Failed to read QQ Music response.".to_string())?;

    if status == 412 || status == 503 {
        return Err("rate_limited: 请求过于频繁，请稍后再试 / Rate limited.".to_string());
    }

    if text.trim_start().starts_with("<!DOCTYPE") || text.trim_start().starts_with("<html") {
        return Err("QQ音乐需要网页验证 / QQ Music requires web verification.".to_string());
    }

    serde_json::from_str::<serde_json::Value>(&text).map_err(|_| {
        format!(
            "QQ音乐 JSON 解析失败 / Failed to parse QQ Music JSON (status={status}, body_length={})",
            text.len()
        )
    })
}

// ── 封面 URL 构造 ────────────────────────────────────────────────────

/// 根据 albummid 构造 QQ 音乐封面 URL
pub fn qqmusic_cover_url(albummid: &str) -> String {
    if albummid.is_empty() {
        return String::new();
    }
    format!("https://y.gtimg.cn/music/photo_new/T002R300x300M000{albummid}.jpg")
}

/// 根据 songmid 构造分享链接
pub fn qqmusic_share_url(songmid: &str) -> String {
    format!("https://y.qq.com/n/ryqq/songDetail/{songmid}")
}

// ── JSON 辅助 ────────────────────────────────────────────────────────

fn json_text(value: Option<&serde_json::Value>) -> Option<String> {
    value
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty())
}

fn json_u64(value: Option<&serde_json::Value>) -> Option<u64> {
    value.and_then(|v| v.as_u64())
}

fn qqmusic_song_id_parts(value: &str) -> (&str, Option<&str>) {
    match value.split_once('|') {
        Some((songmid, media_mid)) if !songmid.is_empty() && !media_mid.is_empty() => {
            (songmid, Some(media_mid))
        }
        _ => (value, None),
    }
}

fn qqmusic_media_mid(value: &serde_json::Value) -> Option<String> {
    json_text(value.get("strMediaMid"))
        .or_else(|| json_text(value.get("media_mid")))
        .or_else(|| json_text(value.get("mediaMid")))
        .or_else(|| {
            value.get("file").and_then(|file| {
                json_text(file.get("media_mid")).or_else(|| json_text(file.get("mediaMid")))
            })
        })
        .filter(|value| !value.is_empty())
}

fn qqmusic_source_song_id(songmid: &str, media_mid: Option<&str>) -> String {
    match media_mid.filter(|value| !value.is_empty() && *value != songmid) {
        Some(media_mid) => format!("{songmid}|{media_mid}"),
        None => songmid.to_string(),
    }
}

// ── 搜索 ──────────────────────────────────────────────────────────────

/// 搜索 QQ 音乐歌曲
pub async fn search_qqmusic(
    config: &ResolvedQQMusicSourceConfig,
    query: &str,
    page: u32,
    page_size: u32,
) -> Result<Vec<SourceSongDto>, String> {
    // 搜索是公开 API，不需要 cookie。
    // 策略：先不带 cookie 搜索（最稳定），失败则带 cookie 重试，
    // 再失败则尝试 search_for_qq_cp 端点。
    let search_params = [
        ("w", query),
        ("p", &page.to_string()),
        ("n", &page_size.to_string()),
        ("t", "0"), // type: 0=song
        ("format", "json"),
    ];

    // 1) 不带 cookie 搜索 client_search_cp
    let config_no_cookie = ResolvedQQMusicSourceConfig {
        enabled: config.enabled,
        base_url: config.base_url.clone(),
        token: None,
    };
    if let Ok(songs) = try_qqmusic_search(
        &config_no_cookie,
        "https://c.y.qq.com/soso/fcgi-bin/client_search_cp",
        &search_params,
    )
    .await
    {
        return Ok(songs);
    }

    // 2) 带 cookie 搜索 client_search_cp
    if config.token.is_some() {
        if let Ok(songs) = try_qqmusic_search(
            config,
            "https://c.y.qq.com/soso/fcgi-bin/client_search_cp",
            &search_params,
        )
        .await
        {
            return Ok(songs);
        }
    }

    // 3) 不带 cookie 搜索 search_for_qq_cp（备用端点）
    if let Ok(songs) = try_qqmusic_search(
        &config_no_cookie,
        "https://c.y.qq.com/soso/fcgi-bin/search_for_qq_cp",
        &search_params,
    )
    .await
    {
        return Ok(songs);
    }

    // 4) 带 cookie 搜索 search_for_qq_cp
    if config.token.is_some() {
        if let Ok(songs) = try_qqmusic_search(
            config,
            "https://c.y.qq.com/soso/fcgi-bin/search_for_qq_cp",
            &search_params,
        )
        .await
        {
            return Ok(songs);
        }
    }

    Err(
        "QQ音乐搜索失败: 所有端点均不可用 / QQ Music search failed: all endpoints unavailable."
            .to_string(),
    )
}

/// 内部辅助：尝试搜索并解析结果，失败返回 None
async fn try_qqmusic_search(
    config: &ResolvedQQMusicSourceConfig,
    url: &str,
    params: &[(&str, &str)],
) -> Result<Vec<SourceSongDto>, String> {
    let text = request_qqmusic_text(config, url, params, None).await?;

    let value: serde_json::Value =
        serde_json::from_str(&text).map_err(|_| "QQ音乐搜索响应解析失败".to_string())?;

    let code = value.get("code").and_then(|v| v.as_i64()).unwrap_or(-1);
    if code != 0 {
        let msg = json_text(value.get("message")).unwrap_or_else(|| "unknown error".to_string());
        return Err(format!("QQ音乐搜索失败: {msg}"));
    }

    let songs = value
        .get("data")
        .and_then(|d| d.get("song"))
        .and_then(|s| s.get("list"))
        .and_then(|l| l.as_array())
        .cloned()
        .unwrap_or_default();

    Ok(songs
        .iter()
        .map(source_song_from_qqmusic_search_json)
        .collect())
}

/// 从搜索结果的 JSON 映射为 SourceSongDto
fn source_song_from_qqmusic_search_json(value: &serde_json::Value) -> SourceSongDto {
    let songmid = json_text(value.get("songmid")).unwrap_or_default();
    let media_mid = qqmusic_media_mid(value);
    let songname = json_text(value.get("songname")).unwrap_or_else(|| "Unknown Song".to_string());
    let title = songname
        .replace("<em>", "")
        .replace("</em>", "")
        .replace("&quot;", "\"")
        .replace("&amp;", "&")
        .trim()
        .to_string();

    let artist = value
        .get("singer")
        .and_then(|s| s.as_array())
        .and_then(|arr| arr.first())
        .and_then(|s| s.get("name"))
        .and_then(|n| n.as_str())
        .unwrap_or("Unknown Artist")
        .to_string();

    let albumname =
        json_text(value.get("albumname")).unwrap_or_else(|| "Unknown Album".to_string());
    let albummid = json_text(value.get("albummid")).unwrap_or_default();
    let cover_url = qqmusic_cover_url(&albummid);

    let interval = json_u64(value.get("interval")).unwrap_or(0); // seconds
    let source_url = if !songmid.is_empty() {
        Some(qqmusic_share_url(&songmid))
    } else {
        None
    };

    // 判断是否付费
    let unavailable = false; // 搜索时先标记为可用，播放时再判断

    SourceSongDto {
        id: qqmusic_source_song_id(&songmid, media_mid.as_deref()),
        source: Some("qqmusic".to_string()),
        title,
        artist,
        album: albumname,
        duration_seconds: interval,
        cover_url,
        playable_url: None,
        unavailable,
        unavailable_reason: None,
        bvid: None,
        aid: None,
        cid: None,
        uploader: None,
        danmaku_count: None,
        play_count: None,
        page_index: None,
        source_url,
    }
}

// ── 歌曲元数据 ────────────────────────────────────────────────────────

/// 获取 QQ 音乐单曲详情
pub async fn fetch_qqmusic_song_metadata(
    config: &ResolvedQQMusicSourceConfig,
    song_id: &str,
) -> Result<SourceSongDto, String> {
    let (songmid, requested_media_mid) = qqmusic_song_id_parts(song_id);
    // 使用 musicu.fcg 获取歌曲详情
    let _guid = rand_guid();
    let (_g_tk, _g_tk_new) = resolve_qqmusic_gtk(config);
    let _qqmusic_key =
        extract_qqmusic_signing_key(config.token.as_deref().unwrap_or("")).unwrap_or_default();
    let body = serde_json::json!({
        "comm": build_qqmusic_comm(config),
        "req_1": {
            "module": "music.pf_song_detail_svr",
            "method": "get_song_detail",
            "param": {
                "song_mid": songmid,
            }
        }
    });

    let value =
        request_qqmusic_json_post(config, "https://u.y.qq.com/cgi-bin/musicu.fcg", &body).await?;

    let info = value
        .get("req_1")
        .and_then(|r| r.get("data"))
        .and_then(|d| d.get("track_info"))
        .ok_or_else(|| "QQ音乐未返回歌曲详情 / QQ Music did not return track info.".to_string())?;

    let songname = json_text(info.get("name")).unwrap_or_else(|| "Unknown Song".to_string());
    let artist = info
        .get("singer")
        .and_then(|s| s.as_array())
        .and_then(|arr| arr.first())
        .and_then(|s| s.get("name"))
        .and_then(|n| n.as_str())
        .unwrap_or("Unknown Artist")
        .to_string();
    let album_obj = info.get("album");
    let albumname = json_text(album_obj.and_then(|a| a.get("name")))
        .unwrap_or_else(|| "Unknown Album".to_string());
    let albummid = json_text(album_obj.and_then(|a| a.get("mid"))).unwrap_or_default();
    let media_mid =
        qqmusic_media_mid(info).or_else(|| requested_media_mid.map(ToString::to_string));
    let cover_url = qqmusic_cover_url(&albummid);
    let interval = json_u64(info.get("interval")).unwrap_or(0);

    Ok(SourceSongDto {
        id: qqmusic_source_song_id(songmid, media_mid.as_deref()),
        source: Some("qqmusic".to_string()),
        title: songname,
        artist,
        album: albumname,
        duration_seconds: interval,
        cover_url,
        playable_url: None,
        unavailable: false,
        unavailable_reason: None,
        bvid: None,
        aid: None,
        cid: None,
        uploader: None,
        danmaku_count: None,
        play_count: None,
        page_index: None,
        source_url: Some(qqmusic_share_url(songmid)),
    })
}

// ── 播放链接 (vKey) ──────────────────────────────────────────────────

fn build_qqmusic_vkey_comm(config: &ResolvedQQMusicSourceConfig) -> serde_json::Value {
    let mut comm = serde_json::json!({
        "format": "json",
        "ct": 24,
        "cv": 0,
        "uin": resolve_qqmusic_uin(config),
    });
    if let Some(authst) = extract_qqmusic_signing_key(config.token.as_deref().unwrap_or("")) {
        if let Some(object) = comm.as_object_mut() {
            object.insert("authst".to_string(), serde_json::Value::String(authst));
        }
    }
    comm
}

fn qqmusic_modern_filenames(quality: &str, songmid: &str, media_mid: Option<&str>) -> Vec<String> {
    let variants: &[(&str, &str)] = match quality {
        "hires" => &[("RS01", "flac"), ("F000", "flac")],
        "lossless" => &[("F000", "flac")],
        "exhigh" => &[("M800", "mp3"), ("C600", "m4a")],
        "higher" => &[("M500", "mp3"), ("C400", "m4a")],
        _ => &[("C400", "m4a"), ("M500", "mp3")],
    };
    let file_mid = media_mid.filter(|value| !value.is_empty());
    variants
        .iter()
        .map(|(prefix, extension)| match file_mid {
            Some(media_mid) => format!("{prefix}{media_mid}.{extension}"),
            None => format!("{prefix}{songmid}{songmid}.{extension}"),
        })
        .collect()
}

fn normalized_qqmusic_cdn_base(value: &str) -> Option<String> {
    let trimmed = value.trim().trim_end_matches('/');
    let https = if let Some(rest) = trimmed.strip_prefix("http://") {
        format!("https://{rest}")
    } else {
        trimmed.to_string()
    };
    let with_slash = format!("{https}/");
    is_trusted_qqmusic_media_url(&with_slash).then_some(https)
}

fn qqmusic_vkey_candidate_urls(
    response: &serde_json::Value,
    data: &serde_json::Value,
    purl: &str,
) -> Vec<String> {
    let trimmed_purl = purl.trim();
    if trimmed_purl.starts_with("http://") || trimmed_purl.starts_with("https://") {
        let promoted = trimmed_purl.replacen("http://", "https://", 1);
        return is_trusted_qqmusic_media_url(&promoted)
            .then_some(promoted)
            .into_iter()
            .collect();
    }
    if trimmed_purl.is_empty()
        || trimmed_purl.contains(';')
        || trimmed_purl.contains("..")
        || trimmed_purl.contains('\\')
    {
        return Vec::new();
    }

    let mut bases = Vec::new();
    for source in [
        response.get("req").and_then(|req| req.get("data")),
        response.get("req").and_then(|req| req.get("result")),
        Some(data),
    ]
    .into_iter()
    .flatten()
    {
        if let Some(sips) = source.get("sip").and_then(|value| value.as_array()) {
            for sip in sips.iter().filter_map(|value| value.as_str()) {
                if let Some(base) = normalized_qqmusic_cdn_base(sip) {
                    if !bases.contains(&base) {
                        bases.push(base);
                    }
                }
            }
        }
    }
    if bases.is_empty() {
        bases.push("https://isure.stream.qqmusic.qq.com".to_string());
    }

    bases
        .into_iter()
        .map(|base| {
            format!(
                "{}/{}",
                base.trim_end_matches('/'),
                trimmed_purl.trim_start_matches('/')
            )
        })
        .filter(|url| is_trusted_qqmusic_media_url(url))
        .take(QQMUSIC_MAX_MEDIA_CANDIDATES)
        .collect()
}

async fn fetch_modern_qqmusic_vkey_candidates(
    config: &ResolvedQQMusicSourceConfig,
    songmid: &str,
    filename: &str,
    guid: &str,
) -> Result<Vec<String>, String> {
    let uin = resolve_qqmusic_uin(config);
    let body = serde_json::json!({
        "loginUin": uin,
        "comm": build_qqmusic_vkey_comm(config),
        "req": {
            "module": "music.audioCdnDispatch.cdnDispatch",
            "method": "GetCdnDispatch",
            "param": {
                "guid": guid,
                "uid": "0",
                "use_new_domain": 1,
                "use_ipv6": 1
            }
        },
        "req_0": {
            "module": "music.vkey.GetVkey",
            "method": "UrlGetVkey",
            "param": {
                "uin": resolve_qqmusic_uin(config),
                "filename": [filename],
                "guid": guid,
                "songmid": [songmid],
                "songtype": [0],
                "ctx": 0
            }
        }
    });
    let response =
        request_qqmusic_json_body(config, "https://u.y.qq.com/cgi-bin/musicu.fcg", &body).await?;
    let request = response.get("req_0");
    let request_code = request
        .and_then(|value| value.get("code"))
        .and_then(|value| value.as_i64())
        .unwrap_or(0);
    let data = request
        .and_then(|value| value.get("data").or_else(|| value.get("result")))
        .or_else(|| response.get("data"))
        .ok_or_else(|| "no_data".to_string())?;
    let retcode = data
        .get("retcode")
        .and_then(|value| value.as_i64())
        .unwrap_or(0);
    if request_code == 104009 || retcode == 104009 {
        return Err("region_restricted: QQ 音乐拒绝了当前网络会话 / QQ Music rejected the current network session."
            .to_string());
    }
    if request_code != 0 {
        return Err(format!("api_code_{request_code}"));
    }

    let info = data
        .get("midurlinfo")
        .and_then(|value| value.as_array())
        .and_then(|items| items.first())
        .or_else(|| {
            data.get("data")
                .and_then(|nested| nested.get("midurlinfo"))
                .and_then(|value| value.as_array())
                .and_then(|items| items.first())
        })
        .ok_or_else(|| "no_data".to_string())?;
    let purl = info
        .get("purl")
        .and_then(|value| value.as_str())
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| classify_qqmusic_no_purl(Some(info)).to_string())?;
    let candidates = qqmusic_vkey_candidate_urls(&response, data, purl);
    if candidates.is_empty() {
        return Err("no_copyright".to_string());
    }
    Ok(candidates)
}

/// 获取播放链接 (vKey 签名)
/// 支持音质降级：如果请求的音质失败（API错误或空purl），
/// 自动尝试更低音质直到 standard。
pub async fn fetch_qqmusic_playable_url(
    config: &ResolvedQQMusicSourceConfig,
    song_id: &str,
    quality: Option<&str>,
) -> Result<PlayableUrlDto, String> {
    let requested_song_id = song_id;
    let (songmid, media_mid) = qqmusic_song_id_parts(song_id);
    let requested_quality = quality.unwrap_or("standard");

    // 音质降级链：从高到低
    // standard(C400 m4a) 与 higher(M500 mp3) 同为 128k，互为格式降级：
    // 某些歌曲仅有 m4a 或仅有 mp3 编码，需要跨格式回退提升播放成功率。
    // standard / higher are both 128k but different formats (m4a vs mp3);
    // cross-format fallback increases playback success rate for songs that
    // only have one encoding available on the CDN.
    let quality_chain: &[&str] = match requested_quality {
        "hires" => &["hires", "lossless", "exhigh", "higher", "standard"],
        "lossless" => &["lossless", "exhigh", "higher", "standard"],
        "exhigh" => &["exhigh", "higher", "standard"],
        "higher" => &["higher", "standard"],
        _ => &["standard", "higher"],
    };

    let uin = resolve_qqmusic_uin_num(config);
    let (_g_tk, _g_tk_new) = resolve_qqmusic_gtk(config);
    let _qqmusic_key =
        extract_qqmusic_signing_key(config.token.as_deref().unwrap_or("")).unwrap_or_default();
    let guid = rand_guid();
    let loginflag = if config.token.is_some() { 1 } else { 0 };

    let mut _last_error: Option<String> = None;
    let mut last_reason: String = "unknown".to_string();
    // 收集所有音质格式的播放 URL 作为候选
    // API 的 fnameHitCache_200 不可靠（CDN 可能实际返回 404），
    // 因此收集所有可用音质的 URL，让 MediaProxy 依次尝试。
    // Collect playback URLs from all quality formats as candidates.
    // API's fnameHitCache_200 is unreliable (CDN may still return 404),
    // so we collect URLs from all available qualities and let MediaProxy try each.
    let mut collected_urls: Vec<String> = Vec::new();

    for &q in quality_chain {
        for filename in qqmusic_modern_filenames(q, songmid, media_mid) {
            #[cfg(debug_assertions)]
            eprintln!("[QQMusic] modern playback attempt: quality={q}");
            match fetch_modern_qqmusic_vkey_candidates(config, songmid, &filename, &guid).await {
                Ok(urls) if !urls.is_empty() => {
                    #[cfg(debug_assertions)]
                    eprintln!(
                        "[QQMusic] modern playback resolved {} trusted candidate(s)",
                        urls.len()
                    );
                    let primary = urls[0].clone();
                    return Ok(PlayableUrlDto {
                        song_id: requested_song_id.to_string(),
                        url: Some(primary),
                        video_url: None,
                        unavailable: false,
                        reason: None,
                        debug: None,
                        audio_candidates: urls.into_iter().skip(1).collect(),
                        video_candidates: Vec::new(),
                    });
                }
                Ok(_) => {}
                Err(error) if error.starts_with("region_restricted") => {
                    return Ok(PlayableUrlDto {
                        song_id: requested_song_id.to_string(),
                        url: None,
                        video_url: None,
                        unavailable: true,
                        reason: Some("region_restricted".to_string()),
                        debug: None,
                        audio_candidates: Vec::new(),
                        video_candidates: Vec::new(),
                    });
                }
                Err(error) => {
                    #[cfg(debug_assertions)]
                    eprintln!("[QQMusic] modern playback candidate unavailable: {error}");
                    _last_error = Some(error.clone());
                    last_reason = classify_qqmusic_playurl_reason(&error).to_string();
                }
            }
        }

        let (prefix, ext) = QQMUSIC_QUALITY_MAP
            .iter()
            .find(|(ql, _, _)| *ql == q)
            .map(|(_, p, e)| (*p, *e))
            .unwrap_or(("M800", "mp3"));

        let filename = format!("{}{}.{}", prefix, songmid, ext);

        let body = serde_json::json!({
            "comm": build_qqmusic_comm(config),
            "req_1": {
                "module": "vkey.GetVkeyServer",
                "method": "CgiGetVkey",
                "param": {
                    "guid": guid,
                    "songmid": [songmid],
                    "songtype": [0],
                    "filename": [filename],
                    "uin": uin.to_string(),
                    "loginflag": loginflag,
                    "platform": "20"
                }
            }
        });

        #[cfg(debug_assertions)]
        eprintln!("[QQMusic] playback attempt: quality={q}");

        let result =
            request_qqmusic_json_post(config, "https://u.y.qq.com/cgi-bin/musicu.fcg", &body).await;

        match result {
            Ok(value) => {
                let req_code = value
                    .get("req_1")
                    .and_then(|r| r.get("code"))
                    .and_then(|c| c.as_i64())
                    .unwrap_or(-1);
                eprintln!("[QQMusic] playback quality={q}: req_1.code={req_code}");
                if req_code != 0 {
                    _last_error = Some(format!("req_1.code={req_code}"));
                    last_reason = if req_code == 500005 {
                        "session_invalid".to_string()
                    } else {
                        format!("api_code_{req_code}")
                    };
                    continue;
                }

                let data = value.get("req_1").and_then(|r| r.get("data"));

                if data.is_none() {
                    _last_error = Some("no data".to_string());
                    last_reason = "no_data".to_string();
                    continue;
                }

                let data = data.unwrap();

                // 检查 data.uin：如果为空，说明服务器未认证我们的 cookie，
                // 返回的 vkey 是匿名 vkey，但 purl 里仍带我们的 uin 参数会导致 CDN 404。
                // 后续会清空 purl 中的 uin 参数以匹配匿名 vkey。
                // Check data.uin: if empty, server didn't authenticate our cookie.
                // The returned vkey is anonymous but purl still carries our uin → CDN 404.
                // We'll clear the uin param in purl to match the anonymous vkey.
                let resp_uin = data.get("uin").and_then(|u| u.as_str()).unwrap_or("");
                let server_authenticated = !resp_uin.is_empty();
                if !server_authenticated {
                    eprintln!("[QQMusic] playback quality={q}: data.uin 为空(服务器未认证cookie)，将清空purl中的uin参数以匹配匿名vkey");
                }

                let midurlinfo = data
                    .get("midurlinfo")
                    .and_then(|m| m.as_array())
                    .and_then(|arr| arr.first());

                let purl = midurlinfo
                    .and_then(|m| m.get("purl"))
                    .and_then(|p| p.as_str())
                    .filter(|s| !s.is_empty());

                // 检查 CDN 缓存状态：fnameHitCache_404 表示文件在 CDN 上不存在
                let msg = data.get("msg").and_then(|m| m.as_str()).unwrap_or("");
                let cdn_cache_404 = msg.contains("fnameHitCache_404");

                if let Some(purl) = purl {
                    if cdn_cache_404 {
                        eprintln!("[QQMusic] playback quality={q}: purl存在但CDN标记404(fnameHitCache_404)，跳过此音质");
                        _last_error = Some("cdn_cache_404".to_string());
                        last_reason = "no_copyright".to_string();
                        continue;
                    }

                    // 收集 API 返回的所有 sip（CDN 地址）
                    // Collect all sip entries (CDN addresses) returned by API
                    let sip_array: Vec<String> = data
                        .get("sip")
                        .and_then(|s| s.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|s| s.as_str())
                                .map(|raw| {
                                    // 核弹级清理：只保留 ASCII URL 合法字符
                                    raw.chars()
                                        .filter(|c| {
                                            c.is_ascii()
                                                && (c.is_ascii_alphanumeric()
                                                    || *c == '.'
                                                    || *c == ':'
                                                    || *c == '/'
                                                    || *c == '-'
                                                    || *c == '_')
                                        })
                                        .collect::<String>()
                                        .trim_end_matches('/')
                                        .to_string()
                                })
                                .filter(|s| s.starts_with("http"))
                                .collect()
                        })
                        .unwrap_or_default();
                    // purl 清理：只保留 ASCII URL 字符
                    // purl cleaning: keep only ASCII URL chars
                    let mut purl_clean: String = purl
                        .chars()
                        .filter(|c| {
                            c.is_ascii()
                                && (c.is_ascii_alphanumeric()
                                    || *c == '.'
                                    || *c == ':'
                                    || *c == '/'
                                    || *c == '?'
                                    || *c == '='
                                    || *c == '&'
                                    || *c == '-'
                                    || *c == '_'
                                    || *c == '%')
                        })
                        .collect();

                    // 若服务器未认证 cookie（data.uin 为空），下发的 vkey 是匿名 vkey，
                    // 但 purl 里仍带我们的 uin 参数，CDN 校验 vkey↔uin 不匹配会 404。
                    // 此时把 purl 中的 uin 参数清空，与匿名 vkey 归属一致。
                    // If server didn't authenticate cookie (data.uin empty), the vkey is anonymous,
                    // but purl still carries our uin param. CDN checks vkey↔uin mismatch → 404.
                    // Clear the uin param in purl to match the anonymous vkey.
                    if !server_authenticated {
                        purl_clean = clear_uin_param(&purl_clean);
                        eprintln!("[QQMusic] playback quality={q}: data.uin为空，已清空purl中的uin参数以匹配匿名vkey");
                    }
                    // 构建播放 URL：对每个 sip 都生成一个 URL
                    // Build URL for each sip
                    if purl_clean.starts_with("http") {
                        // purl 已是完整 URL（旧格式）
                        if is_trusted_qqmusic_media_url(&purl_clean)
                            && !collected_urls.contains(&purl_clean)
                        {
                            collected_urls.push(purl_clean);
                        }
                    } else if !sip_array.is_empty() {
                        // 有 sip：对每个 sip 拼接 sip + "/" + purl
                        let purl_with_slash = if purl_clean.starts_with('/') {
                            purl_clean.clone()
                        } else {
                            format!("/{}", purl_clean)
                        };
                        for sip in &sip_array {
                            let url = format!("{}{}", sip, purl_with_slash);
                            if is_trusted_qqmusic_media_url(&url) && !collected_urls.contains(&url)
                            {
                                collected_urls.push(url);
                            }
                        }
                    } else {
                        // 无 sip：用默认 host
                        let url = format!(
                            "http://ws.stream.qqmusic.qq.com/{}",
                            purl_clean.trim_start_matches('/')
                        );
                        if is_trusted_qqmusic_media_url(&url) && !collected_urls.contains(&url) {
                            collected_urls.push(url);
                        }
                    }

                    // 额外添加硬编码 CDN 备选（以防 API 返回的 sip 不全）
                    // Additional hardcoded CDN fallbacks
                    let known_qq_hosts = [
                        "ws.stream.qqmusic.qq.com",
                        "dl.stream.qqmusic.qq.com",
                        "isure.stream.qqmusic.qq.com",
                        "streamoc.music.tc.qq.com",
                    ];
                    let last_url = collected_urls.last().cloned().unwrap_or_default();
                    for &host in &known_qq_hosts {
                        let alt = if last_url.contains("aqqmusic.tc.qq.com") {
                            last_url.replace("aqqmusic.tc.qq.com", host)
                        } else if last_url.contains("sjy")
                            && last_url.contains(".stream.qqmusic.qq.com")
                        {
                            // 替换 sjy6.stream.qqmusic.qq.com 等
                            let old_host: String = last_url
                                .split("//")
                                .nth(1)
                                .and_then(|s| s.split('/').next())
                                .unwrap_or("")
                                .to_string();
                            if !old_host.is_empty() {
                                last_url.replace(&old_host, host)
                            } else {
                                continue;
                            }
                        } else {
                            continue;
                        };
                        if is_trusted_qqmusic_media_url(&alt) && !collected_urls.contains(&alt) {
                            collected_urls.push(alt);
                        }
                    }
                    collected_urls.truncate(QQMUSIC_MAX_MEDIA_CANDIDATES);
                    eprintln!(
                        "[QQMusic] playback: 已收集 {} 个候选 URL",
                        collected_urls.len()
                    );
                    // 继续尝试其他音质格式，不立即返回
                    // Continue trying other quality formats
                } else {
                    // purl 为空 → 当前音质不可用，尝试更低音质
                    let reason = classify_qqmusic_no_purl(midurlinfo);
                    eprintln!("[QQMusic] playback quality={q}: purl为空, reason={reason}");
                    _last_error = Some(format!("empty purl ({reason})"));
                    last_reason = reason.to_string();
                    continue;
                }
            }
            Err(e) => {
                eprintln!("[QQMusic] playback quality={q}: 请求失败: {e}");
                _last_error = Some(e);
                last_reason = "request_failed".to_string();
                continue;
            }
        }
    }

    // 音质链遍历结束
    // Quality chain iteration complete
    if !collected_urls.is_empty() {
        eprintln!(
            "[QQMusic] playback: 共收集 {} 个候选 URL，返回给 MediaProxy 依次尝试",
            collected_urls.len()
        );
        let primary = collected_urls[0].clone();
        let rest: Vec<String> = collected_urls[1..].to_vec();
        return Ok(PlayableUrlDto {
            song_id: requested_song_id.to_string(),
            url: Some(primary),
            video_url: None,
            unavailable: false,
            reason: None,
            debug: None,
            audio_candidates: rest,
            video_candidates: Vec::new(),
        });
    }

    // 所有音质都失败了
    eprintln!("[QQMusic] playback: 所有音质均失败, last_reason={last_reason}");

    // 匿名访问回退：带认证的请求全部失败时，尝试不带认证信息请求 standard 音质。
    // vkey.GetVkeyServer 是公开 API，standard (C400 m4a 128k) 对免费歌曲应可用。
    // 触发条件：
    //   1. session_invalid / api_code_*     —— 会话或 API 级错误
    //   2. vip_required / no_copyright      —— 带无效 cookie 时 API 常返回空 purl 并归类为这两者；
    //                                           匿名重试可绕过无效 cookie 对 standard 的影响。
    //   3. request_failed / no_data / unknown —— 兜底，最大化播放成功率
    // 唯一不重试的情况是带 cookie 时根本没发出请求（quality_chain 为空，理论上不会发生）。
    let should_try_anon = last_reason == "session_invalid"
        || last_reason == "server_not_authenticated"
        || last_reason.starts_with("api_code_")
        || last_reason == "vip_required"
        || last_reason == "no_copyright"
        || last_reason == "request_failed"
        || last_reason == "no_data"
        || last_reason == "unknown";
    if should_try_anon {
        // 匿名访问同样尝试 standard + higher 两种格式，最大化播放成功率
        // Anonymous access also tries both standard and higher formats
        for &q in &["standard", "higher"] {
            let (prefix, ext) = QQMUSIC_QUALITY_MAP
                .iter()
                .find(|(ql, _, _)| *ql == q)
                .map(|(_, p, e)| (*p, *e))
                .unwrap_or(("C400", "m4a"));
            let filename = format!("{}{}.{}", prefix, songmid, ext);
            let guid = rand_guid();

            let body_anon = serde_json::json!({
                "comm": {
                    "uin": "0",
                    "format": "json",
                    "ct": 24,
                    "cv": 0
                },
                "req_1": {
                    "module": "vkey.GetVkeyServer",
                    "method": "CgiGetVkey",
                    "param": {
                        "guid": guid,
                        "songmid": [songmid],
                        "songtype": [0],
                        "filename": [filename],
                        "uin": "0",
                        "loginflag": 0,
                        "platform": "20"
                    }
                }
            });

            #[cfg(debug_assertions)]
            eprintln!("[QQMusic] anonymous playback attempt: quality={q}");
            let result_anon =
                request_qqmusic_json_post_anon("https://u.y.qq.com/cgi-bin/musicu.fcg", &body_anon)
                    .await;

            if let Ok(value) = result_anon {
                let req_code = value
                    .get("req_1")
                    .and_then(|r| r.get("code"))
                    .and_then(|c| c.as_i64())
                    .unwrap_or(-1);
                eprintln!("[QQMusic] playback 匿名 {q}: req_1.code={req_code}");

                if req_code == 0 {
                    if let Some(data) = value.get("req_1").and_then(|r| r.get("data")) {
                        let msg = data.get("msg").and_then(|m| m.as_str()).unwrap_or("");
                        if msg.contains("fnameHitCache_404") {
                            eprintln!("[QQMusic] playback 匿名 {q}: CDN标记404，跳过");
                            continue;
                        }
                        let midurlinfo = data
                            .get("midurlinfo")
                            .and_then(|m| m.as_array())
                            .and_then(|arr| arr.first());
                        let purl = midurlinfo
                            .and_then(|m| m.get("purl"))
                            .and_then(|p| p.as_str())
                            .filter(|s| !s.is_empty());

                        if let Some(purl) = purl {
                            // 与认证路径相同的核弹级 sip/purl 清理逻辑
                            // same nuclear cleaning approach as authenticated path
                            let sip_raw = data
                                .get("sip")
                                .and_then(|s| s.as_array())
                                .and_then(|arr| arr.first())
                                .and_then(|s| s.as_str())
                                .unwrap_or("");
                            let sip_filtered: String = sip_raw
                                .chars()
                                .filter(|c| {
                                    c.is_ascii()
                                        && (c.is_ascii_alphanumeric()
                                            || *c == '.'
                                            || *c == ':'
                                            || *c == '/'
                                            || *c == '-'
                                            || *c == '_')
                                })
                                .collect();
                            let sip_clean = sip_filtered.trim_end_matches('/').to_string();

                            let purl_clean: String = purl
                                .chars()
                                .filter(|c| {
                                    c.is_ascii()
                                        && (c.is_ascii_alphanumeric()
                                            || *c == '.'
                                            || *c == ':'
                                            || *c == '/'
                                            || *c == '?'
                                            || *c == '='
                                            || *c == '&'
                                            || *c == '-'
                                            || *c == '_'
                                            || *c == '%')
                                })
                                .collect();

                            let url = if purl_clean.starts_with("http") {
                                purl_clean.clone()
                            } else if !sip_clean.is_empty() {
                                let purl_with_slash = if purl_clean.starts_with('/') {
                                    purl_clean.clone()
                                } else {
                                    format!("/{}", purl_clean)
                                };
                                format!("{}{}", sip_clean, purl_with_slash)
                            } else {
                                format!(
                                    "http://ws.stream.qqmusic.qq.com/{}",
                                    purl_clean.trim_start_matches('/')
                                )
                            };
                            eprintln!(
                                "[QQMusic] playback: ✅ 匿名访问 {q} 成功获取播放链接，加入候选"
                            );
                            if is_trusted_qqmusic_media_url(&url)
                                && !collected_urls.contains(&url)
                                && collected_urls.len() < QQMUSIC_MAX_MEDIA_CANDIDATES
                            {
                                collected_urls.push(url);
                            }
                        }
                    }
                }
            }
        }
        eprintln!("[QQMusic] playback: 匿名访问也失败");
    }

    // 匿名访问后再次检查是否收集到 URL
    if !collected_urls.is_empty() {
        eprintln!(
            "[QQMusic] playback: 匿名访问后共收集 {} 个候选 URL",
            collected_urls.len()
        );
        let primary = collected_urls[0].clone();
        let rest: Vec<String> = collected_urls[1..].to_vec();
        return Ok(PlayableUrlDto {
            song_id: requested_song_id.to_string(),
            url: Some(primary),
            video_url: None,
            unavailable: false,
            reason: None,
            debug: None,
            audio_candidates: rest,
            video_candidates: Vec::new(),
        });
    }

    Ok(PlayableUrlDto {
        song_id: requested_song_id.to_string(),
        url: None,
        video_url: None,
        unavailable: true,
        reason: Some(last_reason),
        debug: None,
        audio_candidates: Vec::new(),
        video_candidates: Vec::new(),
    })
}

/// 分类 purl 为空的原因
fn classify_qqmusic_no_purl(midurlinfo: Option<&serde_json::Value>) -> &'static str {
    // QQ 音乐返回空 purl 通常意味着: 版权限制 / VIP限定 / 下架
    if let Some(info) = midurlinfo {
        // 检查是否有错误信息
        if let Some(err) = info.get("errtype") {
            if let Some(code) = err.as_i64() {
                match code {
                    0 => return "vip_required",
                    _ => return "no_copyright",
                }
            }
        }
    }
    "no_copyright"
}

/// 错误分类 (用于播放失败时)
pub fn classify_qqmusic_playurl_reason(error_msg: &str) -> &'static str {
    let msg = error_msg.to_lowercase();
    if msg.contains("rate_limited") || msg.contains("412") || msg.contains("503") {
        "rate_limited"
    } else if msg.contains("vip") || msg.contains("绿钻") {
        "vip_required"
    } else if msg.contains("copyright") || msg.contains("版权") {
        "no_copyright"
    } else if msg.contains("region") || msg.contains("地区") {
        "region_restricted"
    } else if msg.contains("removed") || msg.contains("下架") {
        "song_removed"
    } else if msg.contains("expired") || msg.contains("过期") {
        "session_expired"
    } else if msg.contains("sign") || msg.contains("签名") {
        "sign_invalid"
    } else if msg.contains("timeout") || msg.contains("超时") {
        "timeout"
    } else {
        "api_failed"
    }
}

// ── 歌词 ──────────────────────────────────────────────────────────────

/// 获取 QQ 音乐歌词
pub async fn fetch_qqmusic_lyrics(
    config: &ResolvedQQMusicSourceConfig,
    song_id: &str,
) -> Result<(String, String), String> {
    let (songmid, _) = qqmusic_song_id_parts(song_id);
    let text = request_qqmusic_text(
        config,
        "https://c.y.qq.com/lyric/fcgi-bin/fcg_query_lyric_new.fcg",
        &[("songmid", songmid), ("nobase64", "1"), ("format", "json")],
        None,
    )
    .await?;

    let value: serde_json::Value =
        serde_json::from_str(&text).map_err(|_| "QQ音乐歌词响应解析失败".to_string())?;

    let code = value.get("code").and_then(|v| v.as_i64()).unwrap_or(-1);

    if code == -1310 {
        return Err(
            "QQ音乐歌词 Referer 校验失败 / QQ Music lyrics referer check failed.".to_string(),
        );
    }
    if code != 0 {
        return Ok((String::new(), String::new())); // 无歌词不是错误
    }

    let lyrics = value
        .get("lyric")
        .and_then(|l| l.as_str())
        .unwrap_or("")
        .to_string();

    // 翻译歌词
    let trans = value
        .get("trans")
        .and_then(|t| t.as_str())
        .unwrap_or("")
        .to_string();

    // base64 解码歌词（如果 nobase64=1 不生效，歌词会被 base64 编码）
    let lyrics =
        if lyrics.trim().is_empty() || lyrics.starts_with("[ti:") || lyrics.starts_with("[ar:") {
            lyrics
        } else {
            // 尝试 base64 解码
            general_purpose::STANDARD
                .decode(lyrics.trim())
                .ok()
                .and_then(|bytes| String::from_utf8(bytes).ok())
                .unwrap_or(lyrics)
        };

    let trans = if trans.trim().is_empty() || trans.starts_with("[ti:") || trans.starts_with("[ar:")
    {
        trans
    } else {
        general_purpose::STANDARD
            .decode(trans.trim())
            .ok()
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .unwrap_or(trans)
    };

    Ok((lyrics, trans))
}

// ── Cookie / Token 管理 ──────────────────────────────────────────────

/// QQ 账号会话 → QQ 音乐会话引导（bootstrap）。
///
/// 某些登录链（QQ 账号侧扫码 redirect、手动 Cookie 导入）只携带 QQ 账号身份
/// Cookie（uin/skey/p_skey 等），缺少音乐侧签名 Cookie（qm_keyst / qqmusic_key）。
/// 该引导以既有 Cookie 访问一次固定可信的 QQ 音乐官方首页（y.qq.com），
/// 尽力让官方端点为该身份发放音乐侧 Cookie，并把 Set-Cookie 合并回集合。
///
/// 约束：
/// - 目标 URL 是编译期常量，不接受任何用户输入（非任意 URL 请求器）；
/// - 只合并 Cookie 名称（空值删除语义沿用 merge_set_cookie_header）；
/// - 引导结果是否"已登录"仍只能由 verify_qqmusic_session 裁决；
/// - 日志只允许状态码 / 计数 / 长度，绝无 Cookie 值。
pub async fn bootstrap_qqmusic_session(cookie: &str) -> Result<String, String> {
    const BOOTSTRAP_URL: &str = "https://y.qq.com/";
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client
        .get(BOOTSTRAP_URL)
        .header("Cookie", cookie)
        .header("User-Agent", QQMUSIC_UA)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = resp.status();
    let set_cookies: Vec<String> = resp
        .headers()
        .get_all("set-cookie")
        .iter()
        .filter_map(|v| v.to_str().ok().map(ToString::to_string))
        .collect();
    let merged = merge_set_cookie_header(cookie, &set_cookies);
    eprintln!(
        "[QQMusic] bootstrap: status={}, set_cookie_count={}, merged_cookie_count={}",
        status,
        set_cookies.len(),
        merged.split(';').filter(|p| !p.trim().is_empty()).count()
    );
    Ok(merged)
}

pub fn read_qqmusic_token() -> Option<String> {
    keyring::Entry::new(QQMUSIC_KEYRING_SERVICE, QQMUSIC_KEYRING_ACCOUNT)
        .ok()
        .and_then(|entry| entry.get_password().ok())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

pub fn save_qqmusic_token(token: &str) -> Result<(), String> {
    let trimmed = token.trim();
    if trimmed.is_empty() {
        return Err("QQ音乐会话凭据为空 / QQ Music session credential is empty.".to_string());
    }
    let entry = keyring::Entry::new(QQMUSIC_KEYRING_SERVICE, QQMUSIC_KEYRING_ACCOUNT)
        .map_err(|e| format!("keyring::Entry::new failed: {e}"))?;
    entry.set_password(trimmed).map_err(|e| {
        // keyring v3 默认使用 mock store（不启用平台 feature 时），
        // mock store 在进程退出后丢失数据。
        // 启用 windows-native feature 后使用 Windows Credential Manager 持久化。
        eprintln!("[QQMusic] ❌ keyring set_password failed: {}", e);
        format!("keyring set_password failed: {e}")
    })?;
    // 立即读回验证
    match entry.get_password() {
        Ok(read_back) if read_back == trimmed => {
            eprintln!(
                "[QQMusic] ✅ keyring save+readback OK, length={}",
                trimmed.len()
            );
        }
        Ok(other) => {
            eprintln!(
                "[QQMusic] ⚠️ keyring readback mismatch: saved_len={}, readback_len={}",
                trimmed.len(),
                other.len()
            );
        }
        Err(e) => {
            eprintln!("[QQMusic] ⚠️ keyring readback failed: {}", e);
        }
    }
    Ok(())
}

pub fn delete_qqmusic_token() -> Result<(), String> {
    if let Ok(entry) = keyring::Entry::new(QQMUSIC_KEYRING_SERVICE, QQMUSIC_KEYRING_ACCOUNT) {
        let _ = entry.delete_credential();
    }
    Ok(())
}

/// 从 token (cookie 字符串) 中提取 uin
pub fn resolve_qqmusic_uin(config: &ResolvedQQMusicSourceConfig) -> String {
    config
        .token
        .as_deref()
        .and_then(extract_cookie_value)
        .unwrap_or_else(|| "0".to_string())
}

/// 从 token 中提取 uin 并解析为 u64 数字类型。
/// QQ音乐 musicu.fcg API 的 comm.uin 和 vec_uin 需要数字类型，
/// 现代 QQ 音乐 web 客户端 comm.uin 为字符串类型。
pub fn resolve_qqmusic_uin_num(config: &ResolvedQQMusicSourceConfig) -> u64 {
    let s = resolve_qqmusic_uin(config);
    s.parse::<u64>().unwrap_or(0)
}

fn extract_cookie_value(cookie: &str) -> Option<String> {
    // QQ 登录后 cookie 中可能没有独立的 uin= 字段，
    // 但 pt2gguin= 和 superuin= 中包含 uin 值（格式如 o1747846382）。
    // 按优先级依次尝试 QQ 与微信登录可能写入的数字身份字段。
    // 注意：Set-Cookie 可能先发空值再发实际值（如 pt2gguin=; pt2gguin=o1747846382），
    // 所以对每个名字遍历所有匹配，跳过空值继续找。
    for name in &["uin", "pt2gguin", "superuin", "p_uin", "wxuin"] {
        let prefix = format!("{}=", name);
        for part in cookie.split(';') {
            let part = part.trim();
            if !part.to_lowercase().starts_with(&prefix) {
                continue;
            }
            if let Some((_, val)) = part.split_once('=') {
                let cleaned = val
                    .trim()
                    .trim_start_matches('o')
                    .trim_start_matches('0')
                    .to_string();
                if !cleaned.is_empty() {
                    return Some(cleaned);
                }
            }
        }
    }
    eprintln!(
        "[QQMusic] extract_cookie_value: no uin found in cookie (len={})",
        cookie.len()
    );
    None
}

// ── 媒体代理 ──────────────────────────────────────────────────────────

/// 为搜索结果中的 QQ 音乐封面注册代理
pub fn proxy_qqmusic_search_covers(
    state: &AppState,
    songs: &mut [SourceSongDto],
) -> Result<(), String> {
    for song in songs
        .iter_mut()
        .filter(|s| s.source.as_deref() == Some("qqmusic"))
    {
        if is_proxyable_remote_url(&song.cover_url) && is_trusted_qqmusic_media_url(&song.cover_url)
        {
            song.cover_url = register_media_proxy(state, &song.cover_url, "image")?;
        } else if !song.cover_url.is_empty()
            && !song.cover_url.starts_with("ome-media:")
            && !song.cover_url.contains("ome-media.localhost")
        {
            song.cover_url.clear();
        }
    }
    Ok(())
}

/// DB 中的 QQ 音乐 track 封面：保持稳定 URL（https://y.gtimg.cn 等）直出。
/// 与 NetEase/Bilibili 库行策略一致——不替换为短命代理 token（TTL/LRU
/// 驱逐会让大队列与长会话回到占位封面），也绝不因为"无法代理"而清空封面
/// 字段。搜索结果的瞬时封面仍走 proxy_qqmusic_search_covers。
pub fn proxy_qqmusic_track_covers(
    state: &AppState,
    tracks: &mut [crate::TrackDto],
) -> Result<(), String> {
    let _ = state;
    let _ = tracks;
    Ok(())
}

/// 为播放链接注册媒体代理 (复用 ome-media://)
pub fn proxy_qqmusic_playback(
    state: &AppState,
    playback: &mut PlayableUrlDto,
) -> Result<(), String> {
    let mut candidates: Vec<String> = Vec::new();
    if let Some(ref url) = playback.url {
        if is_trusted_qqmusic_media_url(url) {
            candidates.push(url.clone());
        }
    }
    candidates.extend(
        playback
            .audio_candidates
            .iter()
            .filter(|url| is_trusted_qqmusic_media_url(url))
            .cloned(),
    );
    candidates.sort();
    candidates.dedup();
    candidates.truncate(QQMUSIC_MAX_MEDIA_CANDIDATES);

    if candidates.is_empty() {
        if !playback.unavailable {
            playback.unavailable = true;
            playback.reason = Some("no_copyright".to_string());
        }
        return Ok(());
    }

    let proxied = crate::register_media_proxy_candidates(state, candidates, "audio")?;
    playback.url = Some(proxied);
    playback.audio_candidates.clear();
    Ok(())
}

// ── 辅助 ──────────────────────────────────────────────────────────────

/// 生成随机 GUID (纯数字字符串)
fn rand_guid() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    format!("{}", ts % 10000000000u128)
}

// ── 配置读取 ──────────────────────────────────────────────────────────

use rusqlite::{params, Connection, OptionalExtension};

pub fn load_qqmusic_source_config(db: &Connection) -> Result<QQMusicSourceConfigDto, String> {
    let stored = db
        .query_row(
            "SELECT enabled, base_url FROM music_source_settings WHERE id = 'qqmusic'",
            [],
            |row| Ok((row.get::<_, i64>(0)? == 1, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(|e| e.to_string())?;

    let has_token = read_qqmusic_token().is_some();
    let masked_token = if has_token {
        "••••••••••••".to_string()
    } else {
        String::new()
    };

    Ok(match stored {
        Some((enabled, base_url)) => QQMusicSourceConfigDto {
            enabled,
            base_url: validate_qqmusic_base_url(&base_url)
                .unwrap_or_else(|_| QQMUSIC_DEFAULT_BASE_URL.to_string()),
            has_token,
            masked_token,
        },
        None => QQMusicSourceConfigDto {
            enabled: false,
            base_url: QQMUSIC_DEFAULT_BASE_URL.to_string(),
            has_token,
            masked_token,
        },
    })
}

pub fn save_qqmusic_source_config_to_db(
    db: &Connection,
    payload: SaveQQMusicSourceConfigPayload,
) -> Result<(), String> {
    let requested_base_url = payload
        .base_url
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or(QQMUSIC_DEFAULT_BASE_URL);
    let base_url = validate_qqmusic_base_url(requested_base_url)?;

    if let Some(token) = payload
        .token
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        save_qqmusic_token(token)?;
    }

    db.execute(
        "INSERT INTO music_source_settings (id, enabled, base_url, token_ref, created_at, updated_at)
         VALUES ('qqmusic', ?1, ?2, 'local', CURRENT_TIMESTAMP, CURRENT_TIMESTAMP)
         ON CONFLICT(id) DO UPDATE SET
           enabled = excluded.enabled,
           base_url = excluded.base_url,
           token_ref = excluded.token_ref,
           updated_at = CURRENT_TIMESTAMP",
        params![crate::bool_to_int(payload.enabled), base_url],
    )
    .map_err(|e| e.to_string())?;

    Ok(())
}

pub fn resolve_qqmusic_source_config(
    db: &Connection,
) -> Result<ResolvedQQMusicSourceConfig, String> {
    let config = load_qqmusic_source_config(db)?;
    if !config.enabled {
        return Err("QQ音乐来源未启用 / QQ Music source is not enabled.".to_string());
    }
    Ok(ResolvedQQMusicSourceConfig {
        enabled: config.enabled,
        base_url: config.base_url,
        token: read_qqmusic_token(),
    })
}

// ── 测试连接 ──────────────────────────────────────────────────────────

/// 仅供测试的脱敏诊断。正式构建不注册对应 Tauri command。
/// Redacted diagnostics for tests only; no production Tauri command is registered.
#[cfg(test)]
pub fn debug_dump_qqmusic(db: &Connection) -> serde_json::Value {
    let config = load_qqmusic_source_config(db);
    let token = read_qqmusic_token();
    redacted_qqmusic_credential_metadata(
        config.as_ref().map(|value| value.enabled).unwrap_or(false),
        token.as_deref(),
    )
}

#[cfg(test)]
fn redacted_qqmusic_credential_metadata(
    config_enabled: bool,
    token: Option<&str>,
) -> serde_json::Value {
    let cookie_str = token.unwrap_or("");

    serde_json::json!({
        "config_enabled": config_enabled,
        "token_exists": token.is_some(),
        "token_length": cookie_str.len(),
        "contains_qqmusic_key": cookie_str.contains("qqmusic_key="),
        "contains_qm_keyst": cookie_str.contains("qm_keyst="),
        "contains_uin": cookie_str.contains("uin="),
        "contains_pt2gguin": cookie_str.contains("pt2gguin="),
        "contains_p_skey": cookie_str.contains("p_skey="),
        "contains_superkey": cookie_str.contains("superkey="),
    })
}

pub async fn test_qqmusic_connection() -> Result<String, String> {
    let client = reqwest::Client::new();
    let resp = client
        .get("https://c.y.qq.com/soso/fcgi-bin/client_search_cp")
        .query(&[
            ("w", "test"),
            ("p", "1"),
            ("n", "1"),
            ("t", "0"),
            ("format", "json"),
        ])
        .header("Referer", QQMUSIC_REFERER)
        .header("User-Agent", QQMUSIC_UA)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await;

    match resp {
        Ok(r) if r.status().is_success() => {
            Ok("Connected. QQ音乐来源已就绪。 / QQ Music source is ready.".to_string())
        }
        Ok(r) => Err(format!(
            "QQ音乐返回异常状态码 {} / QQ Music returned status {}",
            r.status().as_u16(),
            r.status().as_u16()
        )),
        Err(e) => Err(format!("无法连接QQ音乐 / Cannot reach QQ Music: {e}")),
    }
}

// ── 会话验证 ──────────────────────────────────────────────────────────

/// 验证 QQ音乐 Cookie 是否有效，返回 (uin, nickname)
pub async fn verify_qqmusic_session(
    config: &ResolvedQQMusicSourceConfig,
) -> Result<(String, String), String> {
    let uin_str = resolve_qqmusic_uin(config);
    let uin_num = resolve_qqmusic_uin_num(config);
    let cookie_str = config.token.as_deref().unwrap_or("");
    #[cfg(debug_assertions)]
    eprintln!(
        "[QQMusic] session verification metadata: credential_length={}, has_uin={}, has_signing_key={}",
        cookie_str.len(),
        uin_num > 0,
        extract_qqmusic_signing_key(cookie_str).is_some()
    );
    eprintln!(
        "[QQMusic] verify_session: 含 qqmusic_key={}",
        cookie_str.contains("qqmusic_key=")
    );
    eprintln!(
        "[QQMusic] verify_session: 含 superkey={}",
        cookie_str.contains("superkey=")
    );

    // 计算 g_tk CSRF token（从 p_skey 或 qqmusic_key 生成）
    let (_g_tk, _g_tk_new) = resolve_qqmusic_gtk(config);

    // 提取 qqmusic_key 用于 comm 对象
    let qqmusic_key = extract_qqmusic_signing_key(cookie_str).unwrap_or_default();
    eprintln!(
        "[QQMusic] verify_session: qqmusic_key length={}",
        qqmusic_key.len()
    );

    // 方式1: 用 userInfo.BaseUserInfoServer.get_user_baseinfo 验证
    // comm 对象匹配 QQ 音乐 web 客户端实际发送的字段
    let body = serde_json::json!({
        "comm": build_qqmusic_comm(config),
        "req_1": {
            "module": "userInfo.BaseUserInfoServer",
            "method": "get_user_baseinfo",
            "param": {
                "vec_uin": [uin_num]
            }
        }
    });

    let result1 =
        request_qqmusic_json_post(config, "https://u.y.qq.com/cgi-bin/musicu.fcg", &body).await;

    match &result1 {
        Ok(value) => {
            // 检查 req_1.code 是否为 0
            let req_code = value
                .get("req_1")
                .and_then(|r| r.get("code"))
                .and_then(|c| c.as_i64())
                .unwrap_or(-1);

            if req_code == 0 {
                // 验证成功，提取昵称
                let data = value.get("req_1").and_then(|r| r.get("data"));

                let nickname = data
                    .and_then(|d| d.get("map"))
                    .and_then(|m| m.get(&uin_str))
                    .and_then(|info| info.get("nick"))
                    .and_then(|n| n.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "QQ音乐用户".to_string());

                #[cfg(debug_assertions)]
                eprintln!("[QQMusic] session verification succeeded via user info");
                return Ok((uin_str, nickname));
            }
            eprintln!("[QQMusic] verify_session 方式1 req_1.code={req_code}，尝试方式2...");
        }
        Err(e) => {
            eprintln!("[QQMusic] verify_session 方式1失败: {e}，尝试方式2...");
        }
    }

    // 方式2: 用 playlist.PlayListPlazaServer.GetUserPlayList 验证（更宽松）
    let body2 = serde_json::json!({
        "comm": build_qqmusic_comm(config),
        "req_1": {
            "module": "playlist.PlayListPlazaServer",
            "method": "GetUserPlayList",
            "param": {
                "uin": uin_num
            }
        }
    });

    let result2 =
        request_qqmusic_json_post(config, "https://u.y.qq.com/cgi-bin/musicu.fcg", &body2).await;

    match &result2 {
        Ok(value) => {
            let req_code = value
                .get("req_1")
                .and_then(|r| r.get("code"))
                .and_then(|c| c.as_i64())
                .unwrap_or(-1);

            if req_code == 0 {
                eprintln!("[QQMusic] verify_session 方式2成功");
                return Ok((uin_str, "QQ音乐用户".to_string()));
            }
            eprintln!(
                "[QQMusic] verify_session 方式2 req_1.code={}，尝试方式3(最简comm)...",
                req_code
            );
        }
        Err(e) => {
            eprintln!("[QQMusic] verify_session 方式2错误: {}", e);
        }
    }

    // 方式3: 带完整comm字段，用 music.UserInfoServer.GetLoginInfo 验证
    let body3 = serde_json::json!({
        "comm": build_qqmusic_comm(config),
        "req_1": {
            "module": "music.UserInfoServer",
            "method": "GetLoginInfo",
            "param": {}
        }
    });

    let result3 =
        request_qqmusic_json_post(config, "https://u.y.qq.com/cgi-bin/musicu.fcg", &body3).await;

    match &result3 {
        Ok(value) => {
            let req_code = value
                .get("req_1")
                .and_then(|r| r.get("code"))
                .and_then(|c| c.as_i64())
                .unwrap_or(-1);
            if req_code == 0 {
                eprintln!("[QQMusic] verify_session 方式3成功!");
                let nickname = value
                    .get("req_1")
                    .and_then(|r| r.get("data"))
                    .and_then(|d| d.get("user_baseinfo"))
                    .and_then(|u| u.get("nick"))
                    .and_then(|n| n.as_str())
                    .filter(|value| !value.is_empty())
                    .unwrap_or("QQ音乐用户");
                return Ok((uin_str, nickname.to_string()));
            }
            eprintln!("[QQMusic] verify_session 方式3 req_1.code={}", req_code);
        }
        Err(e) => {
            eprintln!("[QQMusic] verify_session 方式3错误: {e}");
        }
    }

    Err("QQ音乐验证失败，所有验证方式均失败 / All verification methods failed.".to_string())
}

// ── QR 登录 ──────────────────────────────────────────────────────────

/// QQ ptlogin 二维码状态的显式解析结果。
/// 状态码来自 ptqrlogin 的 ptuiCB 回调第一参数：
///   0  = 登录成功（需 follow redirect 拿最终 cookie）
///   65 = 二维码过期（server 明确告知）
///   66 = 等待扫码
///   67 = 已扫码待手机确认
///   68 = 用户取消
///   -1 / 其它 = 未知（可能是 rate limit、网络劫持页、反自动化等）
/// 前端不得根据本地计时伪造 expired——过期与否只以 server 返回为准。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum QQQrState {
    Waiting,
    Scanned,
    Confirmed,
    Expired,
    Canceled,
    Unknown,
}

impl QQQrState {
    pub fn as_str(&self) -> &'static str {
        match self {
            QQQrState::Waiting => "waiting",
            QQQrState::Scanned => "scanned",
            QQQrState::Confirmed => "confirmed",
            QQQrState::Expired => "expired",
            QQQrState::Canceled => "canceled",
            QQQrState::Unknown => "unknown",
        }
    }
}

/// 从 ptqrlogin 的 200 响应体解析 ptuiCB 回调状态。
/// ptuiCB('66','0','...') / ptuiCB("67","0",...) 均可解析；
/// 无法识别时返回 Unknown（绝不猜测 expired）。
fn parse_ptui_state(text: &str) -> QQQrState {
    let body = text.trim();
    let code = body
        .split(['\'', '"', '(', ')', ',', ' '])
        .find_map(|token| {
            let token = token.trim();
            if token.is_empty() {
                return None;
            }
            token.parse::<i32>().ok()
        });
    match code {
        Some(0) => QQQrState::Confirmed,
        Some(65) => QQQrState::Expired,
        Some(66) => QQQrState::Waiting,
        Some(67) => QQQrState::Scanned,
        Some(68) => QQQrState::Canceled,
        Some(_) => QQQrState::Unknown,
        // 无数字 token（HTML 验证页 / 反自动化 / 纯文本错误）→ Unknown，
        // 调用方应继续等待或退避，不能误报 expired。
        None => QQQrState::Unknown,
    }
}

/// 生成 QQ 音乐二维码登录
pub async fn create_qqmusic_qr() -> Result<QQMusicQrLoginDto, String> {
    let client = reqwest::Client::new();

    // Step 1: 访问 xlogin 获取 pt_login_sig 等初始 cookie
    // 必须带 pt_3rd_aid=100497308（QQ音乐 QQ Connect 应用ID），
    // 否则 ptqrlogin 会被 QQ 服务器拒绝（403 Forbidden）。
    // pt_3rd_aid 使会话类型为 QQ Connect OAuth，与 y.qq.com 网站登录一致。
    // s_url 指向 y.qq.com，确保登录后重定向到 y.qq.com 域（设置 qqmusic_key）。
    let xlogin_url = "https://xui.ptlogin2.qq.com/cgi-bin/xlogin?appid=716027609&daid=383&pt_3rd_aid=100497308&style=33&login_text=%E6%8E%88%E6%9D%83%E5%B9%B6%E7%99%BB%E5%BD%95&hide_title_bar=1&hide_border=1&target=self&s_url=https%3A%2F%2Fy.qq.com%2F";
    let xlogin_resp = client
        .get(xlogin_url)
        .headers(qq_login_browser_headers())
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| format!("xlogin请求失败: {e}"))?;

    // 收集 xlogin 的 cookie（包含 pt_login_sig 等关键 cookie）
    let xlogin_cookies: String = xlogin_resp
        .headers()
        .get_all("set-cookie")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .filter_map(|v| v.split(';').next())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .collect::<Vec<_>>()
        .join("; ");

    // Step 2: 访问 ptqrshow 获取二维码图片和 qrsig，带上 xlogin 的 cookie
    // pt_3rd_aid 必须与 xlogin 一致，否则 QR 码与会话不匹配。
    let t: f64 = {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .subsec_nanos() as f64;
        (nanos % 1_000_000.0) / 1_000_000.0
    };
    let ptqrshow_url = format!(
        "https://ssl.ptlogin2.qq.com/ptqrshow?appid=716027609&e=2&l=M&s=3&d=72&v=4&t={t}&daid=383&pt_3rd_aid=100497308"
    );
    let mut ptqrshow_headers = qq_login_browser_headers();
    if let Ok(v) = "https://xui.ptlogin2.qq.com/cgi-bin/xlogin?appid=716027609&daid=383&pt_3rd_aid=100497308&style=33&s_url=https%3A%2F%2Fy.qq.com%2F".parse() {
        ptqrshow_headers.insert("Referer", v);
    }
    let resp = client
        .get(&ptqrshow_url)
        .headers(ptqrshow_headers)
        .header("Cookie", &xlogin_cookies)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| format!("QQ登录请求失败: {e}"))?;

    // 收集 ptqrshow 的 cookie（包含 qrsig）
    let ptqrshow_cookies: String = resp
        .headers()
        .get_all("set-cookie")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .filter_map(|v| v.split(';').next())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .collect::<Vec<_>>()
        .join("; ");

    // 合并 xlogin + ptqrshow 的所有 cookie（同名覆盖，空值跳过）
    let merged = merge_cookie_entries(
        parse_cookie_header(&xlogin_cookies)
            .into_iter()
            .chain(parse_cookie_header(&ptqrshow_cookies))
            .collect(),
    );
    let all_cookies = merged;

    let qrsig = all_cookies
        .split("; ")
        .find_map(|part| {
            if part.starts_with("qrsig=") {
                Some(part.strip_prefix("qrsig=").unwrap_or("").to_string())
            } else {
                None
            }
        })
        .unwrap_or_default();

    if qrsig.is_empty() {
        return Err("无法获取QQ登录二维码密钥 / Cannot get QQ login QR key.".to_string());
    }

    // 从同一次请求中获取二维码图片字节
    let img_bytes = resp
        .bytes()
        .await
        .map_err(|e| format!("QQ二维码图片读取失败: {e}"))?;

    use base64::Engine;
    let img_b64 = base64::engine::general_purpose::STANDARD.encode(&img_bytes);
    let data_url = format!("data:image/png;base64,{img_b64}");

    Ok(QQMusicQrLoginDto {
        url: data_url,
        key: qrsig,
        cookies: all_cookies,
    })
}

/// 轮询 QQ 音乐二维码扫码状态
pub async fn check_qqmusic_qr(qrsig: &str, all_cookies: &str) -> Result<QQMusicQrCheckDto, String> {
    let ptqrtoken = qqmusic_hash33(qrsig).to_string();
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();

    // 从 all_cookies 中提取 pt_login_sig
    let login_sig = all_cookies
        .split(';')
        .find_map(|part| {
            let part = part.trim();
            if part.starts_with("pt_login_sig=") {
                Some(part.strip_prefix("pt_login_sig=").unwrap_or("").to_string())
            } else {
                None
            }
        })
        .unwrap_or_default();

    let url = format!(
        "https://ssl.ptlogin2.qq.com/ptqrlogin?u1=https%3A%2F%2Fy.qq.com%2F&ptqrtoken={ptqrtoken}&ptredirect=0&h=1&t=1&g=1&from_ui=1&ptlang=2052&action=0-0-{ts}&js_ver=20102616&js_type=1&login_sig={login_sig}&pt_uistyle=40&aid=716027609&daid=383&pt_3rd_aid=100497308&has_onekey=1&"
    );

    // 使用 Policy::none() 防止 reqwest 自动跟随重定向
    // ptqrlogin 登录成功时可能返回 302，reqwest 跟随重定向会丢失 302 的 Set-Cookie（uin, p_skey 等）
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| format!("Client build error: {e}"))?;
    let mut ptqrlogin_headers = qq_login_browser_headers();
    if let Ok(v) = "https://xui.ptlogin2.qq.com/cgi-bin/xlogin?appid=716027609&daid=383&pt_3rd_aid=100497308&style=33&s_url=https%3A%2F%2Fy.qq.com%2F".parse() {
        ptqrlogin_headers.insert("Referer", v);
    }
    let resp = client
        .get(&url)
        .headers(ptqrlogin_headers)
        .header("Cookie", all_cookies)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| format!("QQ登录轮询失败: {e}"))?;

    // 先收集 ptqrlogin 响应的 Set-Cookie 头（这些 cookie 对登录至关重要）
    let ptqrlogin_cookies: String = resp
        .headers()
        .get_all("set-cookie")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .filter_map(|v| v.split(';').next())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .collect::<Vec<_>>()
        .join("; ");

    // 本轮 poll 的最新累积 cookie（原 cookie + 本轮 Set-Cookie 合并）。
    // 返回给调用方回存 session map，保证下一次 poll 携带最新的
    // pt_login_sig / qrsig 等由服务器刷新的 cookie（session continuity）。
    let mut poll_entries = parse_cookie_header(all_cookies);
    poll_entries.extend(parse_cookie_header(&ptqrlogin_cookies));
    let current_cookies = merge_cookie_entries(poll_entries);

    let resp_status = resp.status();

    if resp_status == reqwest::StatusCode::FORBIDDEN {
        return Ok(QQMusicQrCheckDto {
            status: "failed".to_string(),
            cookie: None,
            cookies: Some(current_cookies),
            message: Some(
                "QQ 直连扫码当前不可用，请使用官方网页登录 / Direct QR is unavailable; use Official Sign-in."
                    .to_string(),
            ),
        });
    }

    // 检查是否为重定向响应（302/301）——登录成功时 QQ 服务器可能返回 302
    if resp_status.is_redirection() {
        let redirect_url = resp
            .headers()
            .get("location")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();

        #[cfg(debug_assertions)]
        eprintln!(
            "[QQMusic] QR login redirect: status={}, cookie_count={}",
            resp_status,
            ptqrlogin_cookies
                .split(';')
                .filter(|part| !part.trim().is_empty())
                .count()
        );

        if !redirect_url.is_empty() {
            // 302 重定向 = 登录成功，跟随重定向链获取完整 cookie
            let redirect_cookie =
                follow_qqmusic_login_redirect(&redirect_url, &ptqrlogin_cookies, all_cookies)
                    .await?;

            // 合并所有 cookie（同名覆盖，空值跳过，redirect_cookie 优先）
            let mut entries = parse_cookie_header(all_cookies);
            entries.extend(parse_cookie_header(&ptqrlogin_cookies));
            entries.extend(parse_cookie_header(&redirect_cookie));
            let merged_cookie = merge_cookie_entries(entries);

            #[cfg(debug_assertions)]
            eprintln!(
                "[QQMusic] QR login confirmed: credential_length={}, has_uin={}, has_signing_key={}",
                merged_cookie.len(),
                extract_cookie_value(&merged_cookie).is_some(),
                extract_qqmusic_signing_key(&merged_cookie).is_some()
            );

            return Ok(QQMusicQrCheckDto {
                status: "confirmed".to_string(),
                cookie: Some(merged_cookie.clone()),
                cookies: Some(merged_cookie),
                message: Some("登录成功 / Login successful.".to_string()),
            });
        }
        return Ok(QQMusicQrCheckDto {
            status: "waiting".to_string(),
            cookie: None,
            cookies: Some(current_cookies),
            message: Some("登录重定向但无目标URL，继续等待".to_string()),
        });
    }

    // 非重定向响应（200），读取文本并解析 ptuiCB 回调
    let text = resp
        .text()
        .await
        .map_err(|e| format!("QQ登录响应读取失败: {e}"))?;

    #[cfg(debug_assertions)]
    eprintln!(
        "[QQMusic] QR poll response: status={resp_status}, body_length={}",
        text.len()
    );
    eprintln!(
        "[QQMusic] ptqrlogin cookie传入: qrsig_len={}, all_cookies_len={}",
        qrsig.len(),
        all_cookies.len()
    );
    eprintln!(
        "[QQMusic] ptqrlogin login_sig={}",
        if login_sig.is_empty() {
            "(空/empty)"
        } else {
            "(有值/has value)"
        }
    );

    // 解析 ptuiCB 回调
    // ptuiCB('0','0','redirect_url','0','login success','nickname')
    // 状态码: 66=等待扫码, 67=已扫码待确认, 65=过期, 68=取消, 0=成功
    // 只信任 server 返回的状态；无法解析时保持等待（Unknown）。
    let qr_state = parse_ptui_state(&text);
    eprintln!("[QQAuth][QR_POLL] state={}", qr_state.as_str());

    match qr_state {
        QQQrState::Waiting => Ok(QQMusicQrCheckDto {
            status: "waiting".to_string(),
            cookie: None,
            cookies: Some(current_cookies.clone()),
            message: Some("等待扫码 / Waiting for scan.".to_string()),
        }),
        QQQrState::Scanned => Ok(QQMusicQrCheckDto {
            status: "scanned".to_string(),
            cookie: None,
            cookies: Some(current_cookies.clone()),
            message: Some("已扫码，请在手机上确认 / Scanned, please confirm on phone.".to_string()),
        }),
        QQQrState::Expired => Ok(QQMusicQrCheckDto {
            status: "expired".to_string(),
            cookie: None,
            cookies: Some(current_cookies.clone()),
            message: Some("二维码已过期，请重新生成 / QR code expired.".to_string()),
        }),
        QQQrState::Canceled => Ok(QQMusicQrCheckDto {
            status: "failed".to_string(),
            cookie: None,
            cookies: Some(current_cookies.clone()),
            message: Some("已取消登录 / Login canceled.".to_string()),
        }),
        QQQrState::Confirmed => {
            #[cfg(debug_assertions)]
            eprintln!(
                "[QQMusic] QR login callback confirmed: cookie_count={}",
                ptqrlogin_cookies
                    .split(';')
                    .filter(|part| !part.trim().is_empty())
                    .count()
            );

            // 登录成功 → 合并 ptqrlogin 响应 cookie + 跟随重定向获取的 cookie
            let redirect_cookie =
                follow_qqmusic_login_redirect(&text, &ptqrlogin_cookies, all_cookies).await?;

            // 合并所有 cookie: 原始 cookie + ptqrlogin cookie + 重定向 cookie
            let mut entries = parse_cookie_header(all_cookies);
            entries.extend(parse_cookie_header(&ptqrlogin_cookies));
            entries.extend(parse_cookie_header(&redirect_cookie));
            let merged_cookie = merge_cookie_entries(entries);

            #[cfg(debug_assertions)]
            eprintln!(
            "[QQMusic] QR login confirmed: credential_length={}, has_uin={}, has_signing_key={}",
            merged_cookie.len(),
            extract_cookie_value(&merged_cookie).is_some(),
            extract_qqmusic_signing_key(&merged_cookie).is_some()
        );

            Ok(QQMusicQrCheckDto {
                status: "confirmed".to_string(),
                cookie: Some(merged_cookie.clone()),
                cookies: Some(merged_cookie),
                message: Some("登录成功 / Login successful.".to_string()),
            })
        }
        // 未知响应不立即失败，继续等待扫码（避免临时网络问题导致二维码过早消失）。
        // 这里不猜测 expired——二维码是否过期只能由 server 的 65 状态决定。
        QQQrState::Unknown => Ok(QQMusicQrCheckDto {
            status: "waiting".to_string(),
            cookie: None,
            cookies: Some(current_cookies.clone()),
            message: Some(
                "登录状态暂时未知，继续等待 / Login status unknown; still waiting.".to_string(),
            ),
        }),
    }
}

/// 跟随 QQ 登录重定向 URL 获取 cookie
/// ptqrlogin_cookies: ptqrlogin 响应的 Set-Cookie 头
/// original_cookies: ptqrshow 响应的 cookie（包含 qrsig 等）
///
/// 手动跟随重定向链（最多 10 跳），在每一跳收集 Set-Cookie。
/// uin、p_skey 等关键认证 cookie 通常在重定向链的第 2-3 跳才设置，
/// 因此不能用 Policy::none() 只取第一层响应。
async fn follow_qqmusic_login_redirect(
    callback_text: &str,
    ptqrlogin_cookies: &str,
    original_cookies: &str,
) -> Result<String, String> {
    // 从 ptuiCB 回调中提取重定向 URL，或直接使用传入的 URL（302 重定向情况）
    let redirect_url = if callback_text.starts_with("http") {
        callback_text.to_string()
    } else {
        callback_text
            .split('\'')
            .nth(5)
            .filter(|s| !s.is_empty() && s != &"0")
            .unwrap_or("https://y.qq.com")
            .to_string()
    };

    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| format!("Client build error: {e}"))?;

    // 累积所有 cookie（原始 + ptqrlogin），用于每一跳发送
    let mut accumulated: Vec<String> = Vec::new();
    for part in original_cookies
        .split(';')
        .chain(ptqrlogin_cookies.split(';'))
    {
        let part = part.trim();
        if !part.is_empty() && !accumulated.iter().any(|a| a == part) {
            accumulated.push(part.to_string());
        }
    }

    // 手动跟随重定向链，最多 10 跳
    let mut current_url = parse_trusted_qqmusic_login_url(&redirect_url)?;
    for _hop in 0..10u32 {
        let cookie_header = accumulated.join("; ");
        let resp = client
            .get(current_url.clone())
            .header("User-Agent", QQMUSIC_UA)
            .header("Referer", "https://y.qq.com/")
            .header("Cookie", &cookie_header)
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await
            .map_err(|error| {
                if error.is_timeout() {
                    "QQ 登录跳转超时 / QQ login redirect timed out.".to_string()
                } else {
                    "QQ 登录跳转失败 / QQ login redirect failed.".to_string()
                }
            })?;

        // 收集本跳的 Set-Cookie（正确解析：value 可能含 `=`，attributes 剥离，
        // 空值删除 cookie 不清除有效旧值，同名以新值覆盖）
        let set_cookie_headers: Vec<String> = resp
            .headers()
            .get_all("set-cookie")
            .iter()
            .filter_map(|v| v.to_str().ok().map(ToString::to_string))
            .collect();
        let mut cookie_header = accumulated.join("; ");
        cookie_header = merge_set_cookie_header(&cookie_header, &set_cookie_headers);
        accumulated = cookie_header
            .split(';')
            .map(|part| part.trim().to_string())
            .filter(|part| !part.is_empty())
            .collect();

        // 记算本跳新增的 cookie 数量
        let set_cookie_count = resp.headers().get_all("set-cookie").iter().count();
        eprintln!(
            "[QQMusic] hop {_hop}: status={}, Set-Cookie headers: {}, accumulated cookies: {}",
            resp.status(),
            set_cookie_count,
            accumulated.len()
        );
        // 检查是否为重定向
        if resp.status().is_redirection() {
            if let Some(loc) = resp.headers().get("location") {
                let next_url = loc.to_str().unwrap_or("");
                if next_url.is_empty() {
                    break;
                }
                let resolved = current_url
                    .join(next_url)
                    .map_err(|_| "QQ 登录跳转地址无效 / Invalid QQ login redirect.".to_string())?;
                current_url = parse_trusted_qqmusic_login_url(resolved.as_str())?;
                continue;
            }
        }
        // 非重定向响应（200 OK），尝试从 HTML 中提取跳转 URL
        let body = resp.text().await.unwrap_or_default();

        let mut found_url: Option<String> = None;

        // 查找 meta refresh: content="0;url=..." 或 url=...
        if let Some(pos) = body.find("url=") {
            let after = &body[pos + 4..];
            let after = after.trim_start_matches('"').trim_start_matches('\'');
            let extracted = truncate_html_fragment(after);
            if parse_trusted_qqmusic_login_url(extracted).is_ok() {
                found_url = Some(extracted.to_string());
            }
        }

        // 查找 window.location / location.href / location.replace / top.location
        for pattern in &[
            "window.location=",
            "window.location.href=",
            "location.replace(",
            "location.href=",
            "top.location=",
        ] {
            if let Some(pos) = body.find(pattern) {
                let after = &body[pos + pattern.len()..];
                let after = after
                    .trim_start()
                    .trim_start_matches('"')
                    .trim_start_matches('\'');
                let extracted = truncate_html_fragment(after);
                if parse_trusted_qqmusic_login_url(extracted).is_ok() {
                    found_url = Some(extracted.to_string());
                    break;
                }
            }
        }

        if let Some(next_url) = found_url {
            current_url = parse_trusted_qqmusic_login_url(&next_url)?;
            continue;
        }

        break;
    }

    // 访问 y.qq.com 获取 QQ 音乐专用 cookie（如 qqmusic_key, qt 等）
    // 手动跟随重定向链（最多 5 跳），因为 qqmusic_key 可能在重定向中设置
    let mut yqq_url = parse_trusted_qqmusic_login_url("https://y.qq.com/")?;
    for _yhop in 0..5u32 {
        let cookie_header = accumulated.join("; ");
        let resp_y = client
            .get(yqq_url.clone())
            .header("User-Agent", QQMUSIC_UA)
            .header("Referer", "https://y.qq.com/")
            .header("Cookie", &cookie_header)
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await
            .map_err(|error| {
                if error.is_timeout() {
                    "QQ 音乐会话确认超时 / QQ Music session check timed out.".to_string()
                } else {
                    "QQ 音乐会话确认失败 / QQ Music session check failed.".to_string()
                }
            })?;

        let y_set_cookie_count = resp_y.headers().get_all("set-cookie").iter().count();
        eprintln!(
            "[QQMusic] y.qq.com hop {_yhop}: status={}, Set-Cookie headers: {}",
            resp_y.status(),
            y_set_cookie_count
        );

        let set_cookie_headers: Vec<String> = resp_y
            .headers()
            .get_all("set-cookie")
            .iter()
            .filter_map(|v| v.to_str().ok().map(ToString::to_string))
            .collect();
        let mut cookie_header = accumulated.join("; ");
        cookie_header = merge_set_cookie_header(&cookie_header, &set_cookie_headers);
        accumulated = cookie_header
            .split(';')
            .map(|part| part.trim().to_string())
            .filter(|part| !part.is_empty())
            .collect();

        if resp_y.status().is_redirection() {
            if let Some(loc) = resp_y.headers().get("location") {
                let next_url = loc.to_str().unwrap_or("");
                if next_url.is_empty() {
                    break;
                }
                let resolved = yqq_url.join(next_url).map_err(|_| {
                    "QQ 音乐会话跳转地址无效 / Invalid QQ Music session redirect.".to_string()
                })?;
                yqq_url = parse_trusted_qqmusic_login_url(resolved.as_str())?;
                continue;
            }
        }
        // 200 OK: 尝试从 HTML 中提取跳转 URL
        let y_body = resp_y.text().await.unwrap_or_default();

        let mut y_found_url: Option<String> = None;
        for pattern in &[
            "window.location=",
            "window.location.href=",
            "location.replace(",
            "location.href=",
            "top.location=",
        ] {
            if let Some(pos) = y_body.find(pattern) {
                let after = &y_body[pos + pattern.len()..];
                let after = after
                    .trim_start()
                    .trim_start_matches('"')
                    .trim_start_matches('\'');
                let extracted = truncate_html_fragment(after);
                if parse_trusted_qqmusic_login_url(extracted).is_ok() {
                    y_found_url = Some(extracted.to_string());
                    break;
                }
            }
        }
        if y_found_url.is_none() {
            if let Some(pos) = y_body.find("url=") {
                let after = &y_body[pos + 4..];
                let after = after.trim_start_matches('"').trim_start_matches('\'');
                let extracted = truncate_html_fragment(after);
                if parse_trusted_qqmusic_login_url(extracted).is_ok() {
                    y_found_url = Some(extracted.to_string());
                }
            }
        }
        if let Some(next_url) = y_found_url {
            yqq_url = parse_trusted_qqmusic_login_url(&next_url)?;
            continue;
        }
        break;
    }

    // 后备：如果仍未获取 qqmusic_key，尝试多种方式获取
    // 尝试调用 fcg_music_oauth_get_accesstoken.fcg 用 QQ Connect cookie 换取 qqmusic_key
    let has_qqmusic_key = accumulated.iter().any(|a| a.starts_with("qqmusic_key="));
    if !has_qqmusic_key {
        let cookie_str = accumulated.join("; ");
        // 尝试多种参数组合
        let oauth_urls = vec![
            "https://u.y.qq.com/cgi-bin/fcg_music_oauth_get_accesstoken.fcg?client_id=100497308&format=json&inCharset=utf8&outCharset=utf-8".to_string(),
            "https://u.y.qq.com/cgi-bin/fcg_music_oauth_get_accesstoken.fcg?client_id=100497308&grant_type=authorization_code&format=json".to_string(),
        ];
        for oauth_url in &oauth_urls {
            let oauth_resp = client
                .get(oauth_url)
                .header("Cookie", &cookie_str)
                .header("Referer", "https://y.qq.com/")
                .header("User-Agent", QQMUSIC_UA)
                .timeout(std::time::Duration::from_secs(10))
                .send()
                .await;
            if let Ok(or) = oauth_resp {
                let or_status = or.status();
                let or_text = or.text().await.unwrap_or_default();
                #[cfg(debug_assertions)]
                eprintln!(
                    "[QQMusic] OAuth response: status={or_status}, body_length={}",
                    or_text.len()
                );
                // 尝试从响应中提取 access_token 或 musickey
                if or_text.contains("access_token")
                    || or_text.contains("musickey")
                    || or_text.contains("qqmusic_key")
                {
                    // 尝试解析 JSON
                    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&or_text) {
                        // 尝试多个字段名
                        for field in &["access_token", "musickey", "qqmusic_key", "key", "token"] {
                            if let Some(val) = json
                                .get(field)
                                .and_then(|v| v.as_str())
                                .filter(|s| !s.is_empty())
                            {
                                accumulated.retain(|a| !a.starts_with("qqmusic_key="));
                                accumulated.push(format!("qqmusic_key={}", val));
                                break;
                            }
                        }
                        // 也检查 data 子对象
                        if let Some(data) = json.get("data") {
                            for field in
                                &["access_token", "musickey", "qqmusic_key", "key", "token"]
                            {
                                if let Some(val) = data
                                    .get(field)
                                    .and_then(|v| v.as_str())
                                    .filter(|s| !s.is_empty())
                                {
                                    accumulated.retain(|a| !a.starts_with("qqmusic_key="));
                                    accumulated.push(format!("qqmusic_key={}", val));
                                    break;
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // 也尝试 POST 方式调用 fcg_music_oauth_get_accesstoken.fcg
    let has_qqmusic_key = accumulated.iter().any(|a| a.starts_with("qqmusic_key="));
    if !has_qqmusic_key {
        let cookie_str = accumulated.join("; ");
        let oauth_body = serde_json::json!({
            "comm": build_qqmusic_comm(&ResolvedQQMusicSourceConfig {
                enabled: true,
                base_url: String::new(),
                token: Some(cookie_str.clone()),
            }),
            "req_1": {
                "module": "music.UserInfoServer",
                "method": "GetLoginInfo",
                "param": {}
            }
        });
        let oauth_post_resp = client
            .post("https://u.y.qq.com/cgi-bin/musicu.fcg")
            .header("Cookie", &cookie_str)
            .header("Referer", "https://y.qq.com/")
            .header("User-Agent", QQMUSIC_UA)
            .header("Origin", "https://y.qq.com")
            .json(&oauth_body)
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await;
        if let Ok(opr) = oauth_post_resp {
            let opr_status = opr.status();
            let opr_text = opr.text().await.unwrap_or_default();
            #[cfg(debug_assertions)]
            eprintln!(
                "[QQMusic] login-info response: status={opr_status}, body_length={}",
                opr_text.len()
            );
        }
    }

    // 检查是否有 access_token (来自 Implicit Grant 重定向)
    // access_token 可能在重定向 URL 的 fragment (#access_token=XXX) 中
    // 如果找到，尝试用作 qqmusic_key
    let has_qqmusic_key = accumulated.iter().any(|a| a.starts_with("qqmusic_key="));
    if !has_qqmusic_key {
        // 尝试按优先级使用其他 cookie 作为 qqmusic_key 替代：
        // 1. pt_oauth_token - QQ Connect OAuth token
        // 2. superkey - QQ登录 superkey
        // 3. p_skey - QQ Connect p_skey
        let mut fallback_key: Option<String> = None;

        for a in accumulated.iter() {
            if a.starts_with("pt_oauth_token=") {
                let val = a.strip_prefix("pt_oauth_token=").unwrap_or("");
                if !val.is_empty() {
                    fallback_key = Some(val.to_string());
                    break;
                }
            }
        }

        if fallback_key.is_none() {
            for a in accumulated.iter() {
                if a.starts_with("superkey=") {
                    let val = a.strip_prefix("superkey=").unwrap_or("");
                    if !val.is_empty() {
                        fallback_key = Some(val.to_string());
                        break;
                    }
                }
            }
        }

        if fallback_key.is_none() {
            for a in accumulated.iter() {
                if a.starts_with("p_skey=") {
                    let val = a.strip_prefix("p_skey=").unwrap_or("");
                    if !val.is_empty() {
                        fallback_key = Some(val.to_string());
                        break;
                    }
                }
            }
        }

        if let Some(key_val) = fallback_key {
            accumulated.push(format!("qqmusic_key={}", key_val));
        }
    }

    if !has_qqmusic_key {
        let cookie_header = accumulated.join("; ");

        // 方式A: 尝试调用 QQ Connect authorize 端点获取授权码
        // 方式A1: 尝试 Implicit Grant (response_type=token) - 不需要 client_secret
        let authorize_url = "https://graph.qq.com/oauth2.0/authorize?response_type=token&client_id=100497308&redirect_uri=https%3A%2F%2Fy.qq.com%2Fportal%2Fwx_redirect.html&state=qqmusic_login&scope=get_user_info";
        let noredirect_client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| "QQ 登录客户端初始化失败 / QQ login client failed.".to_string())?;
        let authorize_resp = noredirect_client
            .get(authorize_url)
            .header("Cookie", &cookie_header)
            .header("User-Agent", QQMUSIC_UA)
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await;

        if let Ok(resp) = authorize_resp {
            let status = resp.status();
            #[cfg(debug_assertions)]
            eprintln!("[QQMusic] authorize 响应状态: {status}");
            // 收集 Set-Cookie
            for cookie in resp.headers().get_all("set-cookie").iter() {
                if let Ok(s) = cookie.to_str() {
                    // 提取 cookie 名=值 部分
                    if let Some(eq_pos) = s.find('=') {
                        let _cookie_pair = &s[..s.find(';').unwrap_or(s.len())];
                        let name = &s[..eq_pos];
                        // 跳过空值和过期cookie
                        let value = &s[eq_pos + 1..s.find(';').unwrap_or(s.len())];
                        if !value.is_empty() && !value.contains("1970") {
                            let new_pair = format!("{}={}", name, value);
                            if !accumulated
                                .iter()
                                .any(|a| a.starts_with(&format!("{}=", name)))
                            {
                                accumulated.push(new_pair);
                            }
                        }
                    }
                }
            }
            if let Some(loc) = resp.headers().get("location") {
                let loc_str = loc.to_str().unwrap_or("");
                parse_trusted_qqmusic_login_url(loc_str)?;
                // 检查URL中是否包含 access_token (Implicit Grant, 在 fragment # 中)
                if loc_str.contains("access_token=") {
                    // 提取 access_token
                    let at_start = loc_str.find("access_token=").unwrap();
                    let at_end = loc_str[at_start..]
                        .find('&')
                        .map(|p| at_start + p)
                        .unwrap_or(loc_str.len());
                    let access_token = &loc_str[at_start + 13..at_end];
                    // 将 access_token 作为 qqmusic_key 使用
                    accumulated.retain(|a| !a.starts_with("qqmusic_key="));
                    accumulated.push(format!("qqmusic_key={}", access_token));
                }
                // 检查URL中是否包含 code 参数
                if loc_str.contains("code=") {
                    // 提取 code
                    if let Some(code_start) = loc_str.find("code=") {
                        let code_end = loc_str[code_start..]
                            .find('&')
                            .map(|p| code_start + p)
                            .unwrap_or(loc_str.len());
                        let code = &loc_str[code_start + 5..code_end];

                        // 尝试用 code 换取 access_token
                        let token_url = format!(
                            "https://graph.qq.com/oauth2.0/token?grant_type=authorization_code&client_id=100497308&client_secret=unused&code={}&redirect_uri=https%3A%2F%2Fy.qq.com%2Fportal%2Fwx_redirect.html",
                            code
                        );
                        let token_resp = client
                            .get(&token_url)
                            .header("Cookie", &cookie_header)
                            .header("User-Agent", QQMUSIC_UA)
                            .timeout(std::time::Duration::from_secs(10))
                            .send()
                            .await;
                        if let Ok(tr) = token_resp {
                            #[cfg(debug_assertions)]
                            eprintln!("[QQMusic] token exchange response: status={}", tr.status());
                        }
                    }
                } else {
                    // authorize 重定向到 fast_authorize 或其他中间 URL
                    // 跟随重定向链（最多 5 跳），寻找 code= 参数或 Set-Cookie 中的 qqmusic_key
                    let mut auth_url = parse_trusted_qqmusic_login_url(loc_str)?;
                    for auth_hop in 0..5u32 {
                        let auth_resp = noredirect_client
                            .get(auth_url.clone())
                            .header("Cookie", &cookie_header)
                            .header("User-Agent", QQMUSIC_UA)
                            .header("Referer", "https://graph.qq.com/")
                            .timeout(std::time::Duration::from_secs(10))
                            .send()
                            .await;
                        match auth_resp {
                            Ok(ar) => {
                                let ar_status = ar.status();
                                #[cfg(debug_assertions)]
                                eprintln!("[QQMusic] auth hop {auth_hop}: status={ar_status}");
                                // 收集 Set-Cookie
                                for sc in ar.headers().get_all("set-cookie").iter() {
                                    if let Ok(s) = sc.to_str() {
                                        if let Some(eq_pos) = s.find('=') {
                                            let name = &s[..eq_pos];
                                            let pair_end = s.find(';').unwrap_or(s.len());
                                            let value = &s[eq_pos + 1..pair_end];
                                            if !value.is_empty() && !value.contains("1970") {
                                                let new_pair = format!("{}={}", name, value);
                                                accumulated.retain(|a| {
                                                    !a.starts_with(&format!("{}=", name))
                                                });
                                                accumulated.push(new_pair);
                                            }
                                        }
                                    }
                                }
                                // 检查重定向
                                if ar_status.is_redirection() {
                                    if let Some(al) = ar.headers().get("location") {
                                        let al_str = al.to_str().unwrap_or("");
                                        let resolved_auth_url = auth_url.join(al_str).map_err(|_| {
                                            "QQ 授权跳转地址无效 / Invalid QQ authorization redirect."
                                                .to_string()
                                        })?;
                                        let trusted_auth_url = parse_trusted_qqmusic_login_url(
                                            resolved_auth_url.as_str(),
                                        )?;
                                        // 检查是否包含 code= 参数
                                        if al_str.contains("code=") {
                                            // 如果重定向到 y.qq.com，跟随它（可能设置 qqmusic_key）
                                            if al_str.contains("y.qq.com")
                                                || al_str.contains("wx_redirect")
                                            {
                                                let wx_url = trusted_auth_url.clone();
                                                let wx_resp = client
                                                    .get(wx_url)
                                                    .header("Cookie", &cookie_header)
                                                    .header("User-Agent", QQMUSIC_UA)
                                                    .header("Referer", "https://y.qq.com/")
                                                    .timeout(std::time::Duration::from_secs(10))
                                                    .send()
                                                    .await;
                                                if let Ok(wr) = wx_resp {
                                                    #[cfg(debug_assertions)]
                                                    eprintln!(
                                                        "[QQMusic]   wx_redirect status: {}",
                                                        wr.status()
                                                    );
                                                    for sc in
                                                        wr.headers().get_all("set-cookie").iter()
                                                    {
                                                        if let Ok(s) = sc.to_str() {
                                                            if let Some(eq_pos) = s.find('=') {
                                                                let name = &s[..eq_pos];
                                                                let pair_end =
                                                                    s.find(';').unwrap_or(s.len());
                                                                let value =
                                                                    &s[eq_pos + 1..pair_end];
                                                                if !value.is_empty()
                                                                    && !value.contains("1970")
                                                                {
                                                                    let new_pair = format!(
                                                                        "{}={}",
                                                                        name, value
                                                                    );
                                                                    accumulated.retain(|a| {
                                                                        !a.starts_with(&format!(
                                                                            "{}=",
                                                                            name
                                                                        ))
                                                                    });
                                                                    accumulated.push(new_pair);
                                                                }
                                                            }
                                                        }
                                                    }
                                                }
                                                break;
                                            }
                                        }
                                        // 继续跟随
                                        auth_url = trusted_auth_url;
                                        continue;
                                    }
                                }
                                // 非重定向，读取 body 检查是否有跳转或表单
                                let body = ar.text().await.unwrap_or_default();

                                // 搜索 access_token 在 body 中
                                if body.contains("access_token=") {
                                    if let Some(at_pos) = body.find("access_token=") {
                                        let at_end = body[at_pos..]
                                            .find('&')
                                            .map(|p| at_pos + p)
                                            .unwrap_or(body.len());
                                        let at_val = &body[at_pos + 13..at_end];
                                        if !at_val.is_empty() {
                                            accumulated.retain(|a| !a.starts_with("qqmusic_key="));
                                            accumulated.push(format!("qqmusic_key={}", at_val));
                                        }
                                    }
                                }

                                // 搜索 URL 跳转 (location.href, location.replace, window.location)
                                let mut found_auth_url: Option<String> = None;
                                for pattern in &[
                                    "location.href=",
                                    "location.replace(",
                                    "window.location=",
                                    "top.location=",
                                    "location.assign(",
                                ] {
                                    if let Some(pos) = body.find(pattern) {
                                        let after = &body[pos..];
                                        // 提取双引号中的 URL
                                        if let Some(q1) = after.find('"') {
                                            if let Some(q2) = after[q1 + 1..].find('"') {
                                                let url = &after[q1 + 1..q1 + 1 + q2];
                                                if (url.contains("http")
                                                    || url.contains("wx_redirect")
                                                    || url.contains("access_token")
                                                    || url.contains("code="))
                                                    && parse_trusted_qqmusic_login_url(url).is_ok()
                                                {
                                                    found_auth_url = Some(url.to_string());
                                                    break;
                                                }
                                            }
                                        }
                                    }
                                }

                                // 搜索 meta refresh
                                if found_auth_url.is_none() {
                                    if let Some(pos) = body.find("url=") {
                                        let after = &body[pos..];
                                        if let Some(end) = after.find('"') {
                                            let url = &after[4..end];
                                            if (url.contains("http")
                                                || url.contains("wx_redirect")
                                                || url.contains("access_token")
                                                || url.contains("code="))
                                                && parse_trusted_qqmusic_login_url(url).is_ok()
                                            {
                                                found_auth_url = Some(url.to_string());
                                            }
                                        }
                                    }
                                }

                                // 搜索 form action (授权确认表单)
                                if found_auth_url.is_none() {
                                    if let Some(pos) = body.find("action=") {
                                        let after = &body[pos..];
                                        if let Some(q1) = after.find('"') {
                                            if let Some(q2) = after[q1 + 1..].find('"') {
                                                let url = &after[q1 + 1..q1 + 1 + q2];
                                                if url.contains("http")
                                                    || url.contains("authorize")
                                                    || url.contains("token")
                                                {
                                                    let form_url = if url.starts_with("http") {
                                                        url.to_string()
                                                    } else if url.starts_with('/') {
                                                        format!("https://graph.qq.com{}", url)
                                                    } else {
                                                        format!("https://graph.qq.com/{}", url)
                                                    };
                                                    parse_trusted_qqmusic_login_url(&form_url)?;
                                                    // 提取所有 hidden input
                                                    let mut form_params = Vec::new();
                                                    let mut search_pos = 0;
                                                    while let Some(ip) =
                                                        body[search_pos..].find("<input")
                                                    {
                                                        let input_start = search_pos + ip;
                                                        let input_end = body[input_start..]
                                                            .find('>')
                                                            .map(|p| input_start + p)
                                                            .unwrap_or(body.len());
                                                        let input_tag =
                                                            &body[input_start..input_end];
                                                        if input_tag.contains("type=\"hidden\"") {
                                                            let mut name = "";
                                                            let mut value = "";
                                                            if let Some(np) =
                                                                input_tag.find("name=\"")
                                                            {
                                                                if let Some(ne) =
                                                                    input_tag[np + 6..].find('"')
                                                                {
                                                                    name = &input_tag
                                                                        [np + 6..np + 6 + ne];
                                                                }
                                                            }
                                                            if let Some(vp) =
                                                                input_tag.find("value=\"")
                                                            {
                                                                if let Some(ve) =
                                                                    input_tag[vp + 7..].find('"')
                                                                {
                                                                    value = &input_tag
                                                                        [vp + 7..vp + 7 + ve];
                                                                }
                                                            }
                                                            if !name.is_empty() {
                                                                form_params.push((
                                                                    name.to_string(),
                                                                    value.to_string(),
                                                                ));
                                                            }
                                                        }
                                                        search_pos = input_end + 1;
                                                    }
                                                    let mut form_url_with_params = form_url;
                                                    if !form_params.is_empty() {
                                                        form_url_with_params.push('?');
                                                        for (i, (k, v)) in
                                                            form_params.iter().enumerate()
                                                        {
                                                            if i > 0 {
                                                                form_url_with_params.push('&');
                                                            }
                                                            form_url_with_params
                                                                .push_str(&format!("{}={}", k, v));
                                                        }
                                                    }
                                                    parse_trusted_qqmusic_login_url(
                                                        &form_url_with_params,
                                                    )?;
                                                    found_auth_url = Some(form_url_with_params);
                                                }
                                            }
                                        }
                                    }
                                }

                                if let Some(next_url) = found_auth_url {
                                    auth_url = parse_trusted_qqmusic_login_url(&next_url)?;
                                    continue;
                                }
                                break;
                            }
                            Err(_) => {
                                break;
                            }
                        }
                    }
                }
            }
        }

        // 方式B: 尝试访问 player.html
        let player_resp = client
            .get("https://y.qq.com/portal/player.html")
            .header("User-Agent", QQMUSIC_UA)
            .header("Referer", "https://y.qq.com/")
            .header("Cookie", &cookie_header)
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await;

        if let Ok(resp_p) = player_resp {
            let p_set_cookie_count = resp_p.headers().get_all("set-cookie").iter().count();
            #[cfg(debug_assertions)]
            eprintln!(
                "[QQMusic] player.html: status={}, Set-Cookie headers: {}",
                resp_p.status(),
                p_set_cookie_count
            );
            let set_cookie_headers: Vec<String> = resp_p
                .headers()
                .get_all("set-cookie")
                .iter()
                .filter_map(|v| v.to_str().ok().map(ToString::to_string))
                .collect();
            let mut cookie_header = accumulated.join("; ");
            cookie_header = merge_set_cookie_header(&cookie_header, &set_cookie_headers);
            accumulated = cookie_header
                .split(';')
                .map(|part| part.trim().to_string())
                .filter(|part| !part.is_empty())
                .collect();
        }
    }

    let final_cookie = accumulated.join("; ");
    if final_cookie.is_empty() {
        return Err("无法获取QQ音乐登录凭据 / Cannot get QQ Music login cookie.".to_string());
    }

    #[cfg(debug_assertions)]
    eprintln!(
        "[QQMusic] login redirect complete: credential_length={}, has_uin={}, has_signing_key={}, has_superkey={}",
        final_cookie.len(),
        extract_cookie_value(&final_cookie).is_some(),
        extract_qqmusic_signing_key(&final_cookie).is_some(),
        final_cookie.contains("superkey=")
    );

    Ok(final_cookie)
}

// ── 用户歌单 / 喜欢列表 / VIP 状态 / 用户资料 ────────────────────────

/// QQ音乐用户歌单 DTO (对应 NeteaseUserPlaylistDto)
#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct QQMusicUserPlaylistDto {
    pub id: String,
    pub name: String,
    pub track_count: u32,
    pub creator_name: String,
    pub subscribed: bool,
    pub cover_url: String,
    pub description: String,
}

/// QQ音乐 VIP 状态 DTO (对应 NeteaseVipStatusDto)
#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct QQMusicVipStatusDto {
    pub is_member: bool,
    pub level: Option<String>,
    pub message: String,
    pub membership_known: bool,
}

/// QQ音乐用户资料 DTO (对应 NeteaseUserProfileDto)
#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct QQMusicUserProfileDto {
    pub logged_in: bool,
    pub nickname: Option<String>,
    pub user_id: Option<String>,
    pub avatar_url: Option<String>,
    pub vip: Option<QQMusicVipStatusDto>,
}

/// 获取 QQ音乐用户歌单列表
pub async fn fetch_qqmusic_user_playlists(
    config: &ResolvedQQMusicSourceConfig,
) -> Result<Vec<QQMusicUserPlaylistDto>, String> {
    let uin = resolve_qqmusic_uin_num(config);
    if uin == 0 {
        return Err("未登录QQ音乐 / Not logged in to QQ Music.".to_string());
    }

    let (_g_tk, _g_tk_new) = resolve_qqmusic_gtk(config);
    let body = serde_json::json!({
        "comm": build_qqmusic_comm(config),
        "req_1": {
            "module": "playlist.PlayListPlazaServer",
            "method": "GetUserPlayList",
            "param": {
                "uin": uin
            }
        }
    });

    let value =
        request_qqmusic_json_post(config, "https://u.y.qq.com/cgi-bin/musicu.fcg", &body).await?;

    let playlist_array = value
        .get("req_1")
        .and_then(|r| r.get("data"))
        .and_then(|d| d.get("myPlaylist"))
        .and_then(|p| p.as_array())
        .cloned()
        .unwrap_or_default();

    if playlist_array.is_empty() {
        // 尝试备用路径 v_playlist / playlist
        let alt = value
            .get("req_1")
            .and_then(|r| r.get("data"))
            .and_then(|d| d.get("v_playlist"))
            .and_then(|p| p.as_array())
            .cloned()
            .unwrap_or_default();
        if alt.is_empty() {
            return Err(
                "QQ音乐未返回用户歌单 / QQ Music did not return user playlists.".to_string(),
            );
        }
        return Ok(alt.iter().map(qqmusic_user_playlist_from_json).collect());
    }

    Ok(playlist_array
        .iter()
        .map(qqmusic_user_playlist_from_json)
        .collect())
}

/// 从 JSON 映射为 QQMusicUserPlaylistDto
fn qqmusic_user_playlist_from_json(value: &serde_json::Value) -> QQMusicUserPlaylistDto {
    let id = json_text(value.get("tid"))
        .or_else(|| json_text(value.get("dirid")))
        .or_else(|| json_text(value.get("disstid")))
        .unwrap_or_default();
    let name = json_text(value.get("title"))
        .or_else(|| json_text(value.get("dirName")))
        .or_else(|| json_text(value.get("dissname")))
        .unwrap_or_else(|| "未命名歌单".to_string());
    let track_count = value
        .get("songNum")
        .or_else(|| value.get("songnum"))
        .or_else(|| value.get("track_count"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;
    let creator_name = json_text(value.get("creatorName"))
        .or_else(|| json_text(value.get("nick")))
        .or_else(|| json_text(value.get("nickname")))
        .unwrap_or_else(|| "QQ用户".to_string());
    let subscribed = value
        .get("dirShow")
        .or_else(|| value.get("subscribed"))
        .and_then(|v| v.as_i64())
        .map(|v| v == 0)
        .unwrap_or(false);
    let cover_url = json_text(value.get("picUrl"))
        .or_else(|| json_text(value.get("logo")))
        .or_else(|| json_text(value.get("picurl")))
        .unwrap_or_default();
    let description = json_text(value.get("desc"))
        .or_else(|| json_text(value.get("introduction")))
        .unwrap_or_default();

    QQMusicUserPlaylistDto {
        id,
        name,
        track_count,
        creator_name,
        subscribed,
        cover_url,
        description,
    }
}

/// 获取 QQ音乐歌单详情 (包含歌曲列表)
pub async fn fetch_qqmusic_playlist(
    config: &ResolvedQQMusicSourceConfig,
    playlist_id: &str,
) -> Result<crate::SourcePlaylistDto, String> {
    // 使用 fcg_ucc_getcdinfo_byids_cp.fcg 获取歌单详情
    // 此 API 需要 Referer 头
    let mut extra_headers = HashMap::new();
    extra_headers.insert(
        "Referer",
        format!("https://y.qq.com/n/yqq/playlist/{}.html", playlist_id),
    );
    let text = request_qqmusic_text(
        config,
        "https://c.y.qq.com/qzone/fcg-bin/fcg_ucc_getcdinfo_byids_cp.fcg",
        &[
            ("type", "1"),
            ("json", "1"),
            ("utf8", "1"),
            ("onlysong", "0"),
            ("disstid", playlist_id),
            ("format", "json"),
        ],
        Some(&extra_headers),
    )
    .await?;

    let value: serde_json::Value =
        serde_json::from_str(&text).map_err(|_| "QQ音乐歌单详情解析失败".to_string())?;

    let cdlist = value
        .get("cdlist")
        .and_then(|c| c.as_array())
        .and_then(|arr| arr.first())
        .ok_or_else(|| "QQ音乐歌单数据为空 / QQ Music playlist data is empty.".to_string())?;

    let name = json_text(cdlist.get("dissname")).unwrap_or_else(|| "未知歌单".to_string());
    let description = json_text(cdlist.get("desc")).unwrap_or_default();

    let songlist = cdlist
        .get("songlist")
        .and_then(|s| s.as_array())
        .cloned()
        .unwrap_or_default();

    let tracks: Vec<SourceSongDto> = songlist
        .iter()
        .map(source_song_from_qqmusic_playlist_json)
        .collect();

    Ok(crate::SourcePlaylistDto {
        id: playlist_id.to_string(),
        name,
        description,
        source: "qqmusic".to_string(),
        tracks,
    })
}

/// 从歌单歌曲 JSON 映射为 SourceSongDto
fn source_song_from_qqmusic_playlist_json(value: &serde_json::Value) -> SourceSongDto {
    let songmid = json_text(value.get("songmid"))
        .or_else(|| json_text(value.get("mid")))
        .unwrap_or_default();
    let media_mid = qqmusic_media_mid(value);
    let songname = json_text(value.get("songname"))
        .or_else(|| json_text(value.get("name")))
        .unwrap_or_else(|| "Unknown Song".to_string());

    let artist = value
        .get("singer")
        .and_then(|s| s.as_array())
        .and_then(|arr| arr.first())
        .and_then(|s| s.get("name"))
        .and_then(|n| n.as_str())
        .unwrap_or("Unknown Artist")
        .to_string();

    let albumname = json_text(value.get("albumname"))
        .or_else(|| json_text(value.get("albumName")))
        .unwrap_or_else(|| "Unknown Album".to_string());
    let albummid = json_text(value.get("albummid"))
        .or_else(|| json_text(value.get("albumMid")))
        .unwrap_or_default();
    let cover_url = qqmusic_cover_url(&albummid);
    let interval = json_u64(value.get("interval")).unwrap_or(0);
    let source_url = if !songmid.is_empty() {
        Some(qqmusic_share_url(&songmid))
    } else {
        None
    };

    SourceSongDto {
        id: qqmusic_source_song_id(&songmid, media_mid.as_deref()),
        source: Some("qqmusic".to_string()),
        title: songname,
        artist,
        album: albumname,
        duration_seconds: interval,
        cover_url,
        playable_url: None,
        unavailable: false,
        unavailable_reason: None,
        bvid: None,
        aid: None,
        cid: None,
        uploader: None,
        danmaku_count: None,
        play_count: None,
        page_index: None,
        source_url,
    }
}

/// 获取 QQ音乐用户喜欢的歌曲 (从"我喜欢"歌单中获取)
pub async fn fetch_qqmusic_liked_songs(
    config: &ResolvedQQMusicSourceConfig,
    limit: u32,
) -> Result<Vec<SourceSongDto>, String> {
    // 先获取用户歌单列表，找到"我喜欢"歌单
    let playlists = fetch_qqmusic_user_playlists(config).await?;

    // 查找"我喜欢"歌单 (通常名称包含"我喜欢"或"我喜欢")
    let liked_playlist = playlists.iter().find(|p| {
        p.name.contains("我喜欢") || p.name.contains("我喜歡") || p.name.contains("Like")
    });

    let playlist_id = if let Some(p) = liked_playlist {
        p.id.clone()
    } else {
        // 如果没有找到"我喜欢"歌单，使用第一个歌单
        playlists
            .first()
            .map(|p| p.id.clone())
            .ok_or_else(|| "未找到可用的QQ音乐歌单 / No QQ Music playlist found.".to_string())?
    };

    // 获取歌单详情
    let playlist = fetch_qqmusic_playlist(config, &playlist_id).await?;

    // 限制返回数量
    let tracks = if limit > 0 && (limit as usize) < playlist.tracks.len() {
        playlist.tracks.into_iter().take(limit as usize).collect()
    } else {
        playlist.tracks
    };

    Ok(tracks)
}

/// 获取 QQ音乐 VIP 状态
pub async fn fetch_qqmusic_vip_status(
    config: &ResolvedQQMusicSourceConfig,
) -> Result<QQMusicVipStatusDto, String> {
    let uin = resolve_qqmusic_uin_num(config);
    if uin == 0 {
        return Ok(QQMusicVipStatusDto {
            is_member: false,
            level: None,
            message: "未登录 / Not logged in.".to_string(),
            membership_known: false,
        });
    }

    let uin_str = resolve_qqmusic_uin(config);
    let (g_tk, g_tk_new) = resolve_qqmusic_gtk(config);
    let qqmusic_key =
        extract_qqmusic_signing_key(config.token.as_deref().unwrap_or("")).unwrap_or_default();

    // 调试 / Debug: 诊断 VIP 查询失败原因
    let cookie_str = config.token.as_deref().unwrap_or("");
    #[cfg(debug_assertions)]
    eprintln!(
        "[QQMusic] membership query metadata: credential_length={}, has_uin={}",
        cookie_str.len(),
        uin > 0
    );
    eprintln!(
        "[QQMusic]   has qqmusic_key={}, has p_skey={}, has skey={}, has uin={}",
        extract_qqmusic_signing_key(cookie_str).is_some(),
        extract_cookie_raw(cookie_str, "p_skey").is_some(),
        extract_cookie_raw(cookie_str, "skey").is_some(),
        extract_cookie_raw(cookie_str, "uin").is_some()
    );

    // comm 对象必须包含完整字段，与 verify_qqmusic_session 一致，
    // 否则 API 可能返回 code=500005 导致无法获取 VIP 信息。
    let body = serde_json::json!({
        "comm": build_qqmusic_comm(config),
        "req_1": {
            "module": "userInfo.BaseUserInfoServer",
            "method": "get_user_baseinfo",
            "param": {
                "vec_uin": [uin]
            }
        }
    });

    let result =
        request_qqmusic_json_post(config, "https://u.y.qq.com/cgi-bin/musicu.fcg", &body).await;

    match result {
        Ok(value) => {
            let req_code = value
                .get("req_1")
                .and_then(|r| r.get("code"))
                .and_then(|c| c.as_i64())
                .unwrap_or(-1);

            // 尝试多种路径提取 baseInfo：
            // 1. req_1.data.map.<uin_str>.baseInfo
            // 2. req_1.data.map.<first_value>.baseInfo
            // 3. req_1.data.map.<uin_str>.info
            // 4. req_1.data.map.<first_value>.info
            let map_obj = value
                .get("req_1")
                .and_then(|r| r.get("data"))
                .and_then(|d| d.get("map"))
                .and_then(|m| m.as_object());

            // 获取 map entry 直接引用（不进入 baseInfo/info 子对象）
            // verify_qqmusic_session 成功在 map.<uin_str>.nick 找到 nick，
            // 说明 VIP 字段也直接在 map entry 上（与 nick 同级）。
            let map_entry = map_obj
                .and_then(|obj| obj.get(&uin_str))
                .or_else(|| map_obj.and_then(|obj| obj.values().next()));

            // 先直接在 map entry 上查找 VIP 字段（与 nick 同级）
            // 再回退到 baseInfo/info/vipInfo 子对象中查找
            let vip_type = map_entry
                .and_then(|v| v.get("vipType").and_then(|v| v.as_i64()))
                .or_else(|| map_entry.and_then(|v| v.get("viptype").and_then(|v| v.as_i64())))
                .or_else(|| map_entry.and_then(|v| v.get("vipLevel").and_then(|v| v.as_i64())))
                .or_else(|| map_entry.and_then(|v| v.get("superVipLevel").and_then(|v| v.as_i64())))
                .or_else(|| {
                    // 回退到 baseInfo 子对象
                    map_entry.and_then(|v| v.get("baseInfo")).and_then(|b| {
                        b.get("vipType")
                            .and_then(|v| v.as_i64())
                            .or_else(|| b.get("viptype").and_then(|v| v.as_i64()))
                            .or_else(|| b.get("vipLevel").and_then(|v| v.as_i64()))
                    })
                })
                .or_else(|| {
                    // 回退到 info 子对象
                    map_entry.and_then(|v| v.get("info")).and_then(|b| {
                        b.get("vipType")
                            .and_then(|v| v.as_i64())
                            .or_else(|| b.get("viptype").and_then(|v| v.as_i64()))
                            .or_else(|| b.get("vipLevel").and_then(|v| v.as_i64()))
                    })
                })
                .or_else(|| {
                    // 回退到 vipInfo 子对象
                    map_entry.and_then(|v| v.get("vipInfo")).and_then(|b| {
                        b.get("vipType")
                            .and_then(|v| v.as_i64())
                            .or_else(|| b.get("viptype").and_then(|v| v.as_i64()))
                    })
                })
                .unwrap_or(0);

            let is_vip_bool = map_entry
                .and_then(|v| v.get("isVip").and_then(|v| v.as_bool()))
                .or_else(|| {
                    map_entry
                        .and_then(|v| v.get("isVip").and_then(|v| v.as_i64()))
                        .map(|v| v > 0)
                })
                .or_else(|| {
                    map_entry
                        .and_then(|v| v.get("baseInfo"))
                        .and_then(|b| b.get("isVip").and_then(|v| v.as_bool()))
                })
                .unwrap_or(false);

            if let Some(_entry) = map_entry {
                // 找到了 map entry，使用提取到的 VIP 信息
                let is_member = vip_type > 0 || is_vip_bool;
                let level = match vip_type {
                    1 => Some("green".to_string()),
                    2 => Some("green_deluxe".to_string()),
                    3 => Some("super".to_string()),
                    _ if is_vip_bool => Some("green".to_string()),
                    _ => None,
                };
                let msg = match vip_type {
                    1 => "绿钻会员 / Green VIP.".to_string(),
                    2 => "豪华绿钻 / Deluxe Green VIP.".to_string(),
                    3 => "超级会员 / Super VIP.".to_string(),
                    _ if is_vip_bool => "绿钻会员 / Green VIP.".to_string(),
                    _ => "非会员 / Non-member.".to_string(),
                };
                eprintln!("[QQMusic] vip_status: vipType={vip_type}, isVip_bool={is_vip_bool}, is_member={is_member}");

                // 如果 req_code==0 但未找到 VIP 字段（vip_type=0 且 is_vip_bool=false），
                // 尝试 GetLoginInfo 回退获取更准确的 VIP 信息
                if req_code == 0 && !is_member {
                    eprintln!("[QQMusic] vip_status: req_code=0 但未检测到VIP字段，尝试 GetLoginInfo 回退...");
                    if let Some(vip) = try_get_vip_from_login_info(
                        config,
                        uin,
                        uin_str.as_str(),
                        g_tk,
                        g_tk_new,
                        &qqmusic_key,
                    )
                    .await
                    {
                        if vip.is_member {
                            return Ok(vip);
                        }
                    }
                }

                Ok(QQMusicVipStatusDto {
                    is_member,
                    level,
                    message: msg,
                    membership_known: true,
                })
            } else {
                // API returned a response but without map data.
                let cookie = config.token.as_deref().unwrap_or("");
                let has_key = extract_qqmusic_signing_key(cookie).is_some()
                    || extract_cookie_raw(cookie, "p_skey").is_some();
                eprintln!("[QQMusic] vip_status: no map entry found, req_code={req_code}, has_key={has_key}");

                // 尝试 GetLoginInfo 回退获取 VIP 信息
                let vip_fallback = try_get_vip_from_login_info(
                    config,
                    uin,
                    uin_str.as_str(),
                    g_tk,
                    g_tk_new,
                    &qqmusic_key,
                )
                .await;
                if let Some(vip) = vip_fallback {
                    return Ok(vip);
                }

                if req_code != 0 && has_key {
                    // API rejected our request format (e.g. 500005) —
                    // the user has a cookie but we could NOT verify
                    // membership. Report "unknown" (is_member: false +
                    // membership_known: false), never claim non-member
                    // or member without API confirmation.
                    Ok(QQMusicVipStatusDto {
                        is_member: false,
                        level: None,
                        message: "已登录，VIP状态未知 / Logged in, VIP status unknown.".to_string(),
                        membership_known: false,
                    })
                } else {
                    Ok(QQMusicVipStatusDto {
                        is_member: false,
                        level: None,
                        message: "无法获取VIP状态 / Cannot get VIP status.".to_string(),
                        membership_known: false,
                    })
                }
            }
        }
        Err(e) => {
            eprintln!("[QQMusic] vip_status API请求失败: {e}");
            // API request failed. If user has valid cookie (qqmusic_key),
            // they are logged in — we just can't verify VIP status.
            let cookie = config.token.as_deref().unwrap_or("");
            let has_key = extract_qqmusic_signing_key(cookie).is_some()
                || extract_cookie_raw(cookie, "p_skey").is_some();
            if has_key {
                // API request failed but a cookie exists. Membership is
                // UNKNOWN — do not claim the user is a member (or a
                // non-member) without an API confirmation.
                Ok(QQMusicVipStatusDto {
                    is_member: false,
                    level: None,
                    message: "已登录，VIP状态未知 / Logged in, VIP status unknown.".to_string(),
                    membership_known: false,
                })
            } else {
                Ok(QQMusicVipStatusDto {
                    is_member: false,
                    level: None,
                    message: "VIP状态查询失败 / VIP status query failed.".to_string(),
                    membership_known: false,
                })
            }
        }
    }
}

/// 通过 music.UserInfoServer.GetLoginInfo 尝试获取 VIP 状态（回退方案）
async fn try_get_vip_from_login_info(
    config: &ResolvedQQMusicSourceConfig,
    _uin: u64,
    _uin_str: &str,
    _g_tk: u32,
    _g_tk_new: u32,
    _qqmusic_key: &str,
) -> Option<QQMusicVipStatusDto> {
    let body = serde_json::json!({
        "comm": build_qqmusic_comm(config),
        "req_1": {
            "module": "music.UserInfoServer",
            "method": "GetLoginInfo",
            "param": {}
        }
    });

    let result =
        request_qqmusic_json_post(config, "https://u.y.qq.com/cgi-bin/musicu.fcg", &body).await;

    if let Ok(value) = result {
        // Log only metadata, never the raw response body: GetLoginInfo can
        // echo account fields (uin, nickname, avatar) and possibly session
        // cookies on error paths. Release logs must stay credential-free.
        #[cfg(debug_assertions)]
        eprintln!(
            "[QQMusic] vip_status GetLoginInfo回退: body_length={}",
            value.to_string().len()
        );

        // 检查 req_1.code
        let req_code = value
            .get("req_1")
            .and_then(|r| r.get("code"))
            .and_then(|c| c.as_i64())
            .unwrap_or(-1);
        eprintln!("[QQMusic] vip_status GetLoginInfo: req_1.code={req_code}");

        // 如果 API 返回错误（如 500005），不假装知道 VIP 状态，返回 None 让调用者处理
        if req_code != 0 {
            eprintln!("[QQMusic] vip_status GetLoginInfo: API错误，返回None让调用者fallback");
            return None;
        }

        // GetLoginInfo 响应中可能包含 VIP 信息
        let data = value.get("req_1").and_then(|r| r.get("data"));

        if let Some(data) = data {
            // 仅调试模式打印 data 的 key 列表（无值），release 保持安静。
            #[cfg(debug_assertions)]
            if let Some(obj) = data.as_object() {
                let keys: Vec<&String> = obj.keys().collect();
                eprintln!("[QQMusic] vip_status GetLoginInfo data keys = {:?}", keys);
            }

            // 尝试多种路径: req_1.data.isVip, req_1.data.vipType, req_1.data.userVipInfo.vipType
            // 也检查 user_baseinfo 子对象中的 VIP 字段
            let vip_type = data
                .get("vipType")
                .and_then(|v| v.as_i64())
                .or_else(|| data.get("viptype").and_then(|v| v.as_i64()))
                .or_else(|| data.get("vipLevel").and_then(|v| v.as_i64()))
                .or_else(|| {
                    data.get("userVipInfo")
                        .and_then(|v| v.get("vipType"))
                        .and_then(|v| v.as_i64())
                })
                .or_else(|| {
                    data.get("vipInfo")
                        .and_then(|v| v.get("vipType"))
                        .and_then(|v| v.as_i64())
                })
                .or_else(|| {
                    // user_baseinfo 子对象
                    data.get("user_baseinfo")
                        .and_then(|v| v.get("vipType"))
                        .and_then(|v| v.as_i64())
                })
                .or_else(|| {
                    data.get("user_baseinfo")
                        .and_then(|v| v.get("viptype"))
                        .and_then(|v| v.as_i64())
                })
                .unwrap_or(0);

            let is_vip_bool = data
                .get("isVip")
                .and_then(|v| v.as_bool())
                .or_else(|| data.get("isGreenVip").and_then(|v| v.as_bool()))
                .or_else(|| {
                    data.get("user_baseinfo")
                        .and_then(|v| v.get("isVip"))
                        .and_then(|v| v.as_bool())
                })
                .unwrap_or(false);

            let is_member = vip_type > 0 || is_vip_bool;
            // 只在确实找到了 VIP 字段时才返回结果
            // vip_type > 0 表示是会员，is_vip_bool 表示 isVip=true
            // 如果两者都为 false，可能是字段不存在而非真正的非会员，返回 None 让调用者处理
            if is_member {
                let level = match vip_type {
                    1 => Some("green".to_string()),
                    2 => Some("green_deluxe".to_string()),
                    3 => Some("super".to_string()),
                    _ if is_vip_bool => Some("green".to_string()),
                    _ => None,
                };
                let msg = match vip_type {
                    1 => "绿钻会员 / Green VIP.".to_string(),
                    2 => "豪华绿钻 / Deluxe Green VIP.".to_string(),
                    3 => "超级会员 / Super VIP.".to_string(),
                    _ if is_vip_bool => "绿钻会员 / Green VIP.".to_string(),
                    _ => "非会员 / Non-member.".to_string(),
                };
                eprintln!(
                    "[QQMusic] vip_status GetLoginInfo: vipType={vip_type}, isVip={is_vip_bool}"
                );
                return Some(QQMusicVipStatusDto {
                    is_member,
                    level,
                    message: msg,
                    membership_known: true,
                });
            }
        }
    }

    None
}

/// 获取 QQ音乐用户资料 (昵称、头像、VIP)
pub async fn fetch_qqmusic_user_profile(
    config: &ResolvedQQMusicSourceConfig,
) -> Result<QQMusicUserProfileDto, String> {
    let uin = resolve_qqmusic_uin_num(config);
    if uin == 0 {
        return Ok(QQMusicUserProfileDto {
            logged_in: false,
            nickname: None,
            user_id: None,
            avatar_url: None,
            vip: None,
        });
    }

    let (_g_tk, _g_tk_new) = resolve_qqmusic_gtk(config);
    let body = serde_json::json!({
        "comm": build_qqmusic_comm(config),
        "req_1": {
            "module": "userInfo.BaseUserInfoServer",
            "method": "get_user_baseinfo",
            "param": {
                "vec_uin": [uin]
            }
        }
    });

    let result =
        request_qqmusic_json_post(config, "https://u.y.qq.com/cgi-bin/musicu.fcg", &body).await;

    match result {
        Ok(value) => {
            // 先直接在 map entry 上查找（与 verify_session 一致），
            // 再回退到 baseInfo 子对象
            let map_entry = value
                .get("req_1")
                .and_then(|r| r.get("data"))
                .and_then(|d| d.get("map"))
                .and_then(|m| m.as_object())
                .and_then(|obj| obj.values().next());

            let info = map_entry.and_then(|v| v.get("baseInfo")).or(map_entry); // 如果 baseInfo 不存在，直接用 map entry

            if let Some(info) = info {
                let nickname = json_text(info.get("nick"))
                    .or_else(|| json_text(info.get("nickname")))
                    .or_else(|| Some("QQ音乐用户".to_string()));
                let avatar_url = json_text(info.get("pic"))
                    .or_else(|| json_text(info.get("avatar")))
                    .or_else(|| {
                        // 构造默认头像
                        Some(format!("https://q.qlogo.cn/g?b=qq&nk={uin}&s=100"))
                    });

                // 获取 VIP 状态
                let vip = fetch_qqmusic_vip_status(config).await.ok();

                Ok(QQMusicUserProfileDto {
                    logged_in: true,
                    nickname,
                    user_id: Some(uin.to_string()),
                    avatar_url,
                    vip,
                })
            } else {
                Ok(QQMusicUserProfileDto {
                    logged_in: true,
                    nickname: Some("QQ音乐用户".to_string()),
                    user_id: Some(uin.to_string()),
                    avatar_url: Some(format!("https://q.qlogo.cn/g?b=qq&nk={uin}&s=100")),
                    vip: None,
                })
            }
        }
        Err(_) => Ok(QQMusicUserProfileDto {
            logged_in: false,
            nickname: None,
            user_id: None,
            avatar_url: None,
            vip: None,
        }),
    }
}

// ── 测试 ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cookie_merge_last_value_wins_and_empty_deletes_are_skipped() {
        // Duplicate rule required by the auth P0: later values override
        // earlier ones in place, and a server-side deletion (empty value)
        // never wipes a valid value collected from another hop.
        let merged = merge_cookie_entries(parse_cookie_header(
            "qm_keyst=old; uin=1000; qm_keyst=; uin=; qm_keyst=new",
        ));
        assert_eq!(merged, "qm_keyst=new; uin=1000");
    }

    #[test]
    fn cookie_merge_strips_set_cookie_attributes_and_keeps_base64_padding() {
        // Values may contain '=' (base64 padding) and Set-Cookie attributes
        // (Expires/Path/Domain/HttpOnly/Secure/SameSite) must never leak into
        // the Cookie header we send.
        let merged = merge_set_cookie_header(
            "uin=o12345",
            &[
                "qm_keyst=abc==; Path=/; Domain=.qq.com; HttpOnly; Secure; SameSite=None; Expires=Wed, 21 Oct 2026 07:28:00 GMT".to_string(),
                "p_skey=; Expires=Thu, 01 Jan 1970 00:00:00 GMT".to_string(),
            ],
        );
        assert!(merged.contains("qm_keyst=abc=="));
        assert!(merged.contains("uin=o12345"));
        assert!(!merged.contains("Path=/"));
        assert!(!merged.contains("HttpOnly"));
        assert!(!merged.contains("p_skey="));
    }

    #[test]
    fn credential_completeness_requires_identity_plus_signing_key() {
        // QQ account identity alone (as often collected after a ptlogin
        // redirect) is NOT a QQ Music session; completeness gates bootstrap.
        assert!(!qqmusic_credential_is_complete("uin=o12345; skey=@abcDEF"));
        assert!(qqmusic_credential_is_complete("uin=o12345; qm_keyst=abc=="));
        assert!(qqmusic_credential_is_complete("uin=o12345; p_skey=toto"));
        assert!(!qqmusic_credential_is_complete("qm_keyst=abc=="));
    }

    #[test]
    fn test_sign_algorithm() {
        let data = r#"{"comm":{"ct":24}}"#;
        let sign = qqmusic_get_sign(data);
        assert!(!sign.is_empty());
        assert!(sign.starts_with("zzc"));
        assert_eq!(sign.len(), 19); // "zzc" + 16 hex chars
    }

    #[test]
    fn test_sign_consistent() {
        let data = "test_data";
        let sign1 = qqmusic_get_sign(data);
        let sign2 = qqmusic_get_sign(data);
        assert_eq!(sign1, sign2, "sign must be deterministic");
    }

    #[test]
    fn test_hash33() {
        let token = qqmusic_hash33("test_qrsig");
        assert!(token > 0);
        // hash33 should be deterministic
        assert_eq!(qqmusic_hash33("test_qrsig"), token);
    }

    #[test]
    fn test_cover_url() {
        assert_eq!(
            qqmusic_cover_url("003OUlho2HcRHC"),
            "https://y.gtimg.cn/music/photo_new/T002R300x300M000003OUlho2HcRHC.jpg"
        );
        assert_eq!(qqmusic_cover_url(""), "");
    }

    #[test]
    fn test_share_url() {
        assert_eq!(
            qqmusic_share_url("003OUlho2HcRHC"),
            "https://y.qq.com/n/ryqq/songDetail/003OUlho2HcRHC"
        );
    }

    #[test]
    fn test_classify_error() {
        assert_eq!(
            classify_qqmusic_playurl_reason("rate_limited"),
            "rate_limited"
        );
        assert_eq!(classify_qqmusic_playurl_reason("412"), "rate_limited");
        assert_eq!(
            classify_qqmusic_playurl_reason("vip required"),
            "vip_required"
        );
        assert_eq!(
            classify_qqmusic_playurl_reason("no copyright"),
            "no_copyright"
        );
        assert_eq!(
            classify_qqmusic_playurl_reason("region restricted"),
            "region_restricted"
        );
        assert_eq!(
            classify_qqmusic_playurl_reason("session expired"),
            "session_expired"
        );
        assert_eq!(
            classify_qqmusic_playurl_reason("sign invalid"),
            "sign_invalid"
        );
        assert_eq!(classify_qqmusic_playurl_reason("timeout"), "timeout");
        assert_eq!(
            classify_qqmusic_playurl_reason("unknown error"),
            "api_failed"
        );
    }

    #[test]
    fn test_quality_map() {
        assert_eq!(QQMUSIC_QUALITY_MAP[0], ("standard", "C400", "m4a"));
        assert_eq!(QQMUSIC_QUALITY_MAP[1], ("higher", "M500", "mp3"));
        assert_eq!(QQMUSIC_QUALITY_MAP[2], ("exhigh", "M800", "mp3"));
        assert_eq!(QQMUSIC_QUALITY_MAP[3], ("lossless", "F000", "flac"));
        assert_eq!(QQMUSIC_QUALITY_MAP[4], ("hires", "RS01", "flac"));
    }

    #[test]
    fn parses_complete_cookie_without_truncating_padded_values() {
        let cookie = "pt2gguin=; pt2gguin=o0012345; qm_keyst=secret-value==; display_name=夜色";

        assert_eq!(extract_cookie_value(cookie).as_deref(), Some("12345"));
        assert_eq!(
            extract_cookie_raw(cookie, "qm_keyst").as_deref(),
            Some("secret-value==")
        );
        assert!(qqmusic_credential_is_complete(cookie));
    }

    #[test]
    fn accepts_wechat_qqmusic_session_identity() {
        let cookie = "wxuin=o0098765; qm_keyst=wechat-session==; wxopenid=openid; wxunionid=unionid; wxrefresh_token=refresh";

        assert_eq!(extract_cookie_value(cookie).as_deref(), Some("98765"));
        assert!(qqmusic_credential_is_complete(cookie));
    }

    #[test]
    fn rejects_partial_wechat_qqmusic_session_identity() {
        let cookie = "wxuin=o0098765; qm_keyst=wechat-session==; wxunionid=unionid";

        assert!(!qqmusic_credential_is_complete(cookie));
    }

    #[test]
    fn builds_authenticated_comm_for_wechat_session() {
        let config = ResolvedQQMusicSourceConfig {
            enabled: true,
            base_url: QQMUSIC_DEFAULT_BASE_URL.to_string(),
            token: Some(
                "wxuin=o0098765; qm_keyst=wechat-session==; wxopenid=openid; wxunionid=unionid; wxrefresh_token=refresh"
                    .to_string(),
            ),
        };

        let comm = build_qqmusic_comm(&config);
        assert_eq!(
            comm.get("uin").and_then(|value| value.as_str()),
            Some("98765")
        );
        assert_eq!(
            comm.get("authst").and_then(|value| value.as_str()),
            Some("wechat-session==")
        );
        assert_eq!(
            comm.get("platform").and_then(|value| value.as_str()),
            Some("yqq.json")
        );
        assert!(comm
            .get("g_tk_new_20200303")
            .and_then(|value| value.as_u64())
            .is_some_and(|value| value > 0));
    }

    #[test]
    fn builds_modern_vkey_request_values_without_exposing_other_credentials() {
        let config = ResolvedQQMusicSourceConfig {
            enabled: true,
            base_url: QQMUSIC_DEFAULT_BASE_URL.to_string(),
            token: Some(
                "uin=o0012345; qm_keyst=session-key; psrf_qqaccess_token=private".to_string(),
            ),
        };
        let comm = build_qqmusic_vkey_comm(&config);

        assert_eq!(
            comm.get("uin").and_then(|value| value.as_str()),
            Some("12345")
        );
        assert_eq!(comm.get("ct").and_then(|value| value.as_i64()), Some(24));
        assert_eq!(comm.get("cv").and_then(|value| value.as_i64()), Some(0));
        assert_eq!(
            comm.get("authst").and_then(|value| value.as_str()),
            Some("session-key")
        );
        assert!(!comm.to_string().contains("private"));
        assert_eq!(
            qqmusic_modern_filenames("standard", "song-mid", None),
            vec![
                "C400song-midsong-mid.m4a".to_string(),
                "M500song-midsong-mid.mp3".to_string()
            ]
        );
        assert_eq!(
            qqmusic_modern_filenames("exhigh", "song-mid", Some("media-mid")),
            vec![
                "M800media-mid.mp3".to_string(),
                "C600media-mid.m4a".to_string()
            ]
        );
    }

    #[test]
    fn builds_trusted_https_urls_from_modern_vkey_response() {
        let response = serde_json::json!({
            "req": { "data": { "sip": ["http://isure.stream.qqmusic.qq.com/"] } }
        });
        let data = serde_json::json!({});
        let urls =
            qqmusic_vkey_candidate_urls(&response, &data, "C400song-midsong-mid.m4a?vkey=test");

        assert_eq!(
            urls,
            vec!["https://isure.stream.qqmusic.qq.com/C400song-midsong-mid.m4a?vkey=test"]
        );
    }

    #[test]
    fn preserves_media_mid_in_search_track_identity() {
        let song = source_song_from_qqmusic_search_json(&serde_json::json!({
            "songmid": "song-mid",
            "songname": "Rain",
            "singer": [{ "name": "Singer" }],
            "albumname": "Album",
            "albummid": "album-mid",
            "interval": 200,
            "file": { "media_mid": "media-mid" }
        }));

        assert_eq!(song.id, "song-mid|media-mid");
        assert_eq!(
            qqmusic_song_id_parts(&song.id),
            ("song-mid", Some("media-mid"))
        );
    }

    #[test]
    fn classifies_auth_failures_without_treating_cookie_presence_as_login() {
        assert_eq!(
            classify_qqmusic_auth_failure("network unavailable", false, false),
            QQMusicAuthState::SignedOut
        );
        assert_eq!(
            classify_qqmusic_auth_failure("missing signing key", true, false),
            QQMusicAuthState::Failed
        );
        assert_eq!(
            classify_qqmusic_auth_failure("session expired", true, true),
            QQMusicAuthState::Expired
        );
        assert_eq!(
            classify_qqmusic_auth_failure("network unavailable", true, true),
            QQMusicAuthState::Unknown
        );
    }

    #[test]
    fn accepts_only_official_qqmusic_endpoints() {
        assert_eq!(
            validate_qqmusic_base_url("https://c.y.qq.com/").as_deref(),
            Ok("https://c.y.qq.com")
        );
        assert!(validate_qqmusic_base_url("http://c.y.qq.com").is_err());
        assert!(validate_qqmusic_base_url("https://user:secret@c.y.qq.com").is_err());
        assert!(validate_qqmusic_base_url("https://c.y.qq.com:8443").is_err());
        assert!(validate_qqmusic_base_url("https://c.y.qq.com.example.test").is_err());
        assert!(validate_qqmusic_base_url("https://127.0.0.1").is_err());
        assert!(validate_qqmusic_base_url("https://localhost").is_err());

        assert!(is_trusted_qqmusic_webview_url(
            "https://y.qq.com/n/ryqq/profile"
        ));
        assert!(!is_trusted_qqmusic_webview_url(
            "https://y.qq.com.example.test/login"
        ));
    }

    #[test]
    fn accepts_only_qqmusic_media_hosts() {
        assert!(is_trusted_qqmusic_media_url(
            "https://y.gtimg.cn/music/photo_new/cover.jpg"
        ));
        assert!(is_trusted_qqmusic_media_url(
            "https://dl.stream.qqmusic.qq.com/file.m4a"
        ));
        assert!(!is_trusted_qqmusic_media_url(
            "https://y.gtimg.cn.example.test/file.m4a"
        ));
        assert!(!is_trusted_qqmusic_media_url("http://127.0.0.1/private"));
    }

    #[test]
    fn redacted_diagnostics_never_include_cookie_values() {
        let secret = "uin=o0012345; qm_keyst=do-not-print-this==; p_skey=private-value";
        let metadata = redacted_qqmusic_credential_metadata(true, Some(secret));
        let serialized = serde_json::to_string(&metadata).expect("serialize metadata");

        assert!(metadata["token_exists"].as_bool().unwrap_or(false));
        assert_eq!(metadata["token_length"].as_u64(), Some(secret.len() as u64));
        assert!(!serialized.contains("do-not-print-this"));
        assert!(!serialized.contains("private-value"));
        assert!(!serialized.contains("o0012345"));
    }
}
