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

/// 安全截断字符串到指定字节长度，避免在 UTF-8 字符中间切分导致 panic
/// Safely truncate a string to a byte length, avoiding panics on UTF-8 char boundaries
fn safe_preview(s: &str, n: usize) -> &str {
    if s.len() <= n {
        return s;
    }
    let mut end = n;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

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
    pub uin: String,
    pub nickname: String,
    pub avatar_url: String,
    pub vip_type: String, // "none" | "green" | "super"
    pub message: String,
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
fn extract_cookie_raw(cookie: &str, name: &str) -> Option<String> {
    let prefix = format!("{}=", name);
    for part in cookie.split(';') {
        let part = part.trim();
        if part.to_lowercase().starts_with(&prefix.to_lowercase()) {
            if let Some(val) = part.split('=').nth(1) {
                let val = val.trim();
                if !val.is_empty() {
                    return Some(val.to_string());
                }
            }
        }
    }
    None
}

/// 提取 QQ 音乐签名 key / Extract QQ Music signing key
/// 现代 QQ 音乐 web 使用 qm_keyst 字段（非 qqmusic_key）。
/// Modern QQ Music web uses qm_keyst (not qqmusic_key).
/// 优先 qm_keyst，回退到 qqmusic_key 以兼容旧 cookie。
fn extract_qqmusic_signing_key(cookie: &str) -> Option<String> {
    extract_cookie_raw(cookie, "qm_keyst").or_else(|| extract_cookie_raw(cookie, "qqmusic_key"))
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

    eprintln!(
        "[QQMusic] g_tk={g_tk}, g_tk_new_20200303={g_tk_new} (key_new='{}', key_old='{}')",
        &key_new[..key_new.len().min(20)],
        &key_old[..key_old.len().min(20)]
    );
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
    serde_json::json!({
        "uin": uin_str,
        "format": "json",
        "ct": 24,
        "cv": 0
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
    let client = reqwest::Client::new();
    let mut req = client.get(url).query(query);

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
        .map_err(|e| format!("QQ音乐请求失败 / QQ Music request failed: {e}"))?;

    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|e| format!("QQ音乐响应读取失败 / Failed to read QQ Music response: {e}"))?;

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
    let client = reqwest::Client::new();

    let body_str = serde_json::to_string(body).unwrap_or_default();

    // 简化 URL：仅 format=json + data 参数
    // QQ音乐 musicu.fcg 非加密接口仅需 format 和 data，
    // 额外的 g_tk/loginUin/hostUin 等参数会导致部分 API 返回 500005。
    // Simplified URL: only format=json + data param.
    // The non-encrypted musicu.fcg endpoint only needs format and data;
    // extra params like g_tk/loginUin/hostUin cause some APIs to return 500005.
    let encoded_body = urlencoding::encode(&body_str);
    let full_url = format!("{}?format=json&data={}", url, encoded_body);
    eprintln!("[QQMusic] GET URL length: {}", full_url.len());
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

    eprintln!("[QQMusic] request URL: {}", url);
    eprintln!("[QQMusic] request body: {}", safe_preview(&body_str, 500));

    let response = req
        .timeout(std::time::Duration::from_secs(12))
        .send()
        .await
        .map_err(|e| format!("QQ音乐请求失败 / QQ Music request failed: {e}"))?;

    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|e| format!("QQ音乐响应读取失败 / Failed to read QQ Music response: {e}"))?;

    eprintln!(
        "[QQMusic] response status: {status}, body length: {}",
        text.len()
    );
    eprintln!("[QQMusic] response body: {}", safe_preview(&text, 500));

    if status == 412 || status == 503 {
        return Err("rate_limited: 请求过于频繁，请稍后再试 / Rate limited.".to_string());
    }
    if text.trim_start().starts_with("<!DOCTYPE") || text.trim_start().starts_with("<html") {
        return Err("QQ音乐需要网页验证 / QQ Music requires web verification.".to_string());
    }

    serde_json::from_str::<serde_json::Value>(&text)
        .map_err(|_| format!("QQ音乐 JSON 解析失败 / Failed to parse QQ Music JSON: {text}"))
}

/// 发送 QQ 音乐 API 请求（匿名，不发送 Cookie）
pub async fn request_qqmusic_json_post_anon(
    url: &str,
    body: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    let client = reqwest::Client::new();
    let body_str = serde_json::to_string(body).unwrap_or_default();
    let encoded_body = urlencoding::encode(&body_str);
    let full_url = format!("{}?format=json&data={}", url, encoded_body);
    let mut req = client.get(&full_url);
    req = req.header("Referer", "https://y.qq.com/");
    req = req.header("Origin", "https://y.qq.com");
    req = req.header("User-Agent", QQMUSIC_UA);
    req = req.header("Accept", "application/json, text/plain, */*");

    let response = req
        .timeout(std::time::Duration::from_secs(12))
        .send()
        .await
        .map_err(|e| format!("QQ音乐请求失败 / QQ Music request failed: {e}"))?;

    let text = response
        .text()
        .await
        .map_err(|e| format!("QQ音乐响应读取失败 / Failed to read QQ Music response: {e}"))?;

    if text.trim_start().starts_with("<!DOCTYPE") || text.trim_start().starts_with("<html") {
        return Err("QQ音乐需要网页验证 / QQ Music requires web verification.".to_string());
    }

    serde_json::from_str::<serde_json::Value>(&text)
        .map_err(|_| format!("QQ音乐 JSON 解析失败 / Failed to parse QQ Music JSON: {text}"))
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
        id: songmid.clone(),
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
    songmid: &str,
) -> Result<SourceSongDto, String> {
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
    let cover_url = qqmusic_cover_url(&albummid);
    let interval = json_u64(info.get("interval")).unwrap_or(0);

    Ok(SourceSongDto {
        id: songmid.to_string(),
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

/// 获取播放链接 (vKey 签名)
/// 支持音质降级：如果请求的音质失败（API错误或空purl），
/// 自动尝试更低音质直到 standard。
pub async fn fetch_qqmusic_playable_url(
    config: &ResolvedQQMusicSourceConfig,
    songmid: &str,
    quality: Option<&str>,
) -> Result<PlayableUrlDto, String> {
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

        eprintln!("[QQMusic] playback: trying quality={q}, filename={filename}, songmid={songmid}, uin={uin}");

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
                let resp_str = serde_json::to_string(&value).unwrap_or_default();
                eprintln!(
                    "[QQMusic] playback 完整响应: {}",
                    safe_preview(&resp_str, 500)
                );

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

                    // 提取 testfile2g 用于 CDN 连通性诊断（不加入候选列表，仅日志）
                    // Extract testfile2g for CDN connectivity diagnosis (log only, not added to candidates)
                    if let Some(testfile) = data
                        .get("testfile2g")
                        .and_then(|t| t.as_str())
                        .filter(|s| !s.is_empty())
                    {
                        let testfile_clean: String = testfile
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
                        if !testfile_clean.is_empty() {
                            let sip_for_test = data
                                .get("sip")
                                .and_then(|s| s.as_array())
                                .and_then(|arr| arr.first())
                                .and_then(|s| s.as_str())
                                .unwrap_or("http://aqqmusic.tc.qq.com");
                            let sip_clean_test: String = sip_for_test
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
                                .collect::<String>()
                                .trim_end_matches('/')
                                .to_string();
                            let test_url = format!(
                                "{}/{}",
                                sip_clean_test,
                                testfile_clean.trim_start_matches('/')
                            );
                            eprintln!(
                                "[QQMusic] playback: CDN 连通性诊断(不播放) testfile={}",
                                safe_preview(&test_url, 120)
                            );
                        }
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
                    eprintln!("[QQMusic] playback quality={q}: all sips={:?}", sip_array);

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
                    eprintln!(
                        "[QQMusic] playback quality={q}: cleaned purl (len={}): {:?}",
                        purl_clean.len(),
                        purl_clean
                    );

                    // 构建播放 URL：对每个 sip 都生成一个 URL
                    // Build URL for each sip
                    if purl_clean.starts_with("http") {
                        // purl 已是完整 URL（旧格式）
                        eprintln!(
                            "[QQMusic] playback: ✅ quality={q} URL(purl自带)={}",
                            safe_preview(&purl_clean, 100)
                        );
                        if !collected_urls.contains(&purl_clean) {
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
                            eprintln!(
                                "[QQMusic] playback: ✅ quality={q} URL(sip={})={}",
                                safe_preview(sip, 40),
                                safe_preview(&url, 100)
                            );
                            if !collected_urls.contains(&url) {
                                collected_urls.push(url);
                            }
                        }
                    } else {
                        // 无 sip：用默认 host
                        let url = format!(
                            "http://ws.stream.qqmusic.qq.com/{}",
                            purl_clean.trim_start_matches('/')
                        );
                        eprintln!(
                            "[QQMusic] playback: ✅ quality={q} URL(default)={}",
                            safe_preview(&url, 100)
                        );
                        if !collected_urls.contains(&url) {
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
                        if !collected_urls.contains(&alt) {
                            collected_urls.push(alt);
                        }
                    }
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
            song_id: songmid.to_string(),
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

            eprintln!("[QQMusic] playback: 尝试匿名访问 {q} 音质 (filename={filename})...");
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
                            collected_urls.push(url);
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
            song_id: songmid.to_string(),
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
        song_id: songmid.to_string(),
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
    songmid: &str,
) -> Result<(String, String), String> {
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
    // 按优先级依次尝试：uin → pt2gguin → superuin
    // 注意：Set-Cookie 可能先发空值再发实际值（如 pt2gguin=; pt2gguin=o1747846382），
    // 所以对每个名字遍历所有匹配，跳过空值继续找。
    for name in &["uin", "pt2gguin", "superuin"] {
        let prefix = format!("{}=", name);
        for part in cookie.split(';') {
            let part = part.trim();
            if !part.to_lowercase().starts_with(&prefix) {
                continue;
            }
            if let Some(val) = part.split('=').nth(1) {
                let cleaned = val
                    .trim()
                    .trim_start_matches('o')
                    .trim_start_matches('0')
                    .to_string();
                if !cleaned.is_empty() {
                    eprintln!("[QQMusic] extract_cookie_value: matched '{name}' → {cleaned}");
                    return Some(cleaned);
                }
            }
        }
    }
    eprintln!(
        "[QQMusic] extract_cookie_value: no uin found in cookie (len={})",
        cookie.len()
    );
    // 调试：输出所有含 "uin" 的 cookie 段
    for part in cookie.split(';') {
        let part = part.trim();
        if part.to_lowercase().contains("uin") {
            eprintln!("[QQMusic]   cookie part with 'uin': {part}");
        }
    }
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
        if is_proxyable_remote_url(&song.cover_url) {
            song.cover_url = register_media_proxy(state, &song.cover_url, "image")?;
        }
    }
    Ok(())
}

/// 为 DB 中的 QQ 音乐 track 封面注册代理
pub fn proxy_qqmusic_track_covers(
    state: &AppState,
    tracks: &mut [crate::TrackDto],
) -> Result<(), String> {
    for track in tracks.iter_mut().filter(|t| t.source == "qqmusic") {
        if is_proxyable_remote_url(&track.cover_url) {
            track.cover_url = register_media_proxy(state, &track.cover_url, "image")?;
        }
    }
    Ok(())
}

/// 为播放链接注册媒体代理 (复用 ome-media://)
pub fn proxy_qqmusic_playback(
    state: &AppState,
    playback: &mut PlayableUrlDto,
) -> Result<(), String> {
    let mut candidates: Vec<String> = Vec::new();
    if let Some(ref url) = playback.url {
        candidates.push(url.clone());
    }
    candidates.extend(playback.audio_candidates.iter().cloned());

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
            base_url,
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
    let base_url = payload
        .base_url
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or(QQMUSIC_DEFAULT_BASE_URL)
        .trim_end_matches('/')
        .to_string();

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

/// 登录成功后自动启用 QQ 音乐来源。
/// 所有登录流程（QR 扫码、Cookie 导入、WebView 提取）在保存 cookie 到 keyring 后，
/// 都应调用此函数确保 DB 中 enabled=1，否则用户重开设置面板后会看到"已禁用"状态，
/// 且 resolve_qqmusic_source_config 会返回错误导致播放/VIP 查询全部失败。
pub fn ensure_qqmusic_source_enabled(db: &Connection) -> Result<(), String> {
    db.execute(
        "INSERT INTO music_source_settings (id, enabled, base_url, token_ref, created_at, updated_at)
         VALUES ('qqmusic', 1, ?1, 'local', CURRENT_TIMESTAMP, CURRENT_TIMESTAMP)
         ON CONFLICT(id) DO UPDATE SET
           enabled = 1,
           token_ref = 'local',
           updated_at = CURRENT_TIMESTAMP",
        params![QQMUSIC_DEFAULT_BASE_URL],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

// ── 测试连接 ──────────────────────────────────────────────────────────

/// 诊断：返回 cookie 和提取值的详细信息
pub fn debug_dump_qqmusic(db: &Connection) -> serde_json::Value {
    let config = load_qqmusic_source_config(db);
    let token = read_qqmusic_token();

    let cookie_str = token.as_deref().unwrap_or("");
    let uin_str = resolve_qqmusic_uin(&ResolvedQQMusicSourceConfig {
        enabled: true,
        base_url: String::new(),
        token: token.clone(),
    });
    let uin_num = resolve_qqmusic_uin_num(&ResolvedQQMusicSourceConfig {
        enabled: true,
        base_url: String::new(),
        token: token.clone(),
    });
    let (g_tk, g_tk_new) = resolve_qqmusic_gtk(&ResolvedQQMusicSourceConfig {
        enabled: true,
        base_url: String::new(),
        token: token.clone(),
    });
    let qqmusic_key = extract_qqmusic_signing_key(cookie_str).unwrap_or_default();
    let p_skey = extract_cookie_raw(cookie_str, "p_skey").unwrap_or_default();
    let superkey = extract_cookie_raw(cookie_str, "superkey").unwrap_or_default();

    serde_json::json!({
        "config_enabled": config.as_ref().map(|c| c.enabled).unwrap_or(false),
        "token_exists": token.is_some(),
        "token_length": cookie_str.len(),
        "token_preview": safe_preview(cookie_str, 200),
        "contains_qqmusic_key": cookie_str.contains("qqmusic_key="),
        "contains_uin": cookie_str.contains("uin="),
        "contains_pt2gguin": cookie_str.contains("pt2gguin="),
        "contains_p_skey": cookie_str.contains("p_skey="),
        "uin_str": uin_str,
        "uin_num": uin_num,
        "g_tk": g_tk,
        "g_tk_new": g_tk_new,
        "qqmusic_key_length": qqmusic_key.len(),
        "qqmusic_key_preview": safe_preview(&qqmusic_key, 30),
        "p_skey_length": p_skey.len(),
        "superkey_length": superkey.len(),
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
    eprintln!(
        "[QQMusic] verify_session: uin_str={uin_str}, uin_num={uin_num}, cookie len={}",
        cookie_str.len()
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
            let resp_summary = serde_json::to_string(value).unwrap_or_default();
            eprintln!(
                "[QQMusic] verify_session 方式1 API响应: {}",
                &resp_summary[..resp_summary.len().min(500)]
            );

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

                eprintln!("[QQMusic] verify_session 方式1成功, nickname={nickname}");
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
            let resp_summary = serde_json::to_string(value).unwrap_or_default();
            eprintln!(
                "[QQMusic] verify_session 方式2 API响应: {}",
                &resp_summary[..resp_summary.len().min(500)]
            );

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
            let resp_summary = serde_json::to_string(value).unwrap_or_default();
            eprintln!(
                "[QQMusic] verify_session 方式3 API响应: {}",
                &resp_summary[..resp_summary.len().min(500)]
            );
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
                    .unwrap_or("");
                let avatar_url = value
                    .get("req_1")
                    .and_then(|r| r.get("data"))
                    .and_then(|d| d.get("user_baseinfo"))
                    .and_then(|u| u.get("pic"))
                    .and_then(|p| p.as_str())
                    .unwrap_or("");
                return Ok((nickname.to_string(), avatar_url.to_string()));
            }
            eprintln!("[QQMusic] verify_session 方式3 req_1.code={}", req_code);
        }
        Err(e) => {
            eprintln!("[QQMusic] verify_session 方式3错误: {e}");
        }
    }

    // 方式4: 所有 API 验证均失败（通常 500005），但 cookie 中包含有效 uin 和 qqmusic_key/p_skey，
    // 说明用户确实已登录，只是 musicu.fcg API 不接受我们的请求格式。
    // 此时仍接受登录，后续搜索/播放等操作使用 Cookie 头认证。
    // 此回退放在 match 外部，确保即使方式3的 HTTP 请求本身失败（Err）也能触发。
    if uin_num > 0 {
        let has_key = !qqmusic_key.is_empty()
            || extract_cookie_raw(cookie_str, "p_skey").is_some()
            || extract_cookie_raw(cookie_str, "superkey").is_some();
        if has_key {
            eprintln!("[QQMusic] verify_session 方式4: API验证失败但cookie含有效凭据(uin={uin_num}, has_key={has_key})，接受登录");
            return Ok((uin_str, "QQ音乐用户".to_string()));
        }
    }

    Err(format!(
        "QQ音乐验证失败(uin={uin_str})，所有验证方式均失败 / All verification methods failed."
    ))
}

// ── QR 登录 ──────────────────────────────────────────────────────────

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

    // 合并 xlogin + ptqrshow 的所有 cookie
    let mut all_cookie_parts: Vec<&str> = Vec::new();
    for part in xlogin_cookies.split(';').chain(ptqrshow_cookies.split(';')) {
        let part = part.trim();
        if !part.is_empty() && !all_cookie_parts.contains(&part) {
            all_cookie_parts.push(part);
        }
    }
    let all_cookies = all_cookie_parts.join("; ");

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

    let resp_status = resp.status();

    // 检查是否为重定向响应（302/301）——登录成功时 QQ 服务器可能返回 302
    if resp_status.is_redirection() {
        let redirect_url = resp
            .headers()
            .get("location")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();

        eprintln!("[QQMusic] ptqrlogin 302 重定向: location={redirect_url}");
        eprintln!("[QQMusic] ptqrlogin Set-Cookie: {ptqrlogin_cookies}");

        if !redirect_url.is_empty() {
            // 302 重定向 = 登录成功，跟随重定向链获取完整 cookie
            let redirect_cookie =
                follow_qqmusic_login_redirect(&redirect_url, &ptqrlogin_cookies, all_cookies)
                    .await?;

            // 合并所有 cookie（redirect_cookie 已包含全部，这里去重合并）
            let mut all_parts: Vec<&str> = Vec::new();
            for part in all_cookies
                .split(';')
                .chain(ptqrlogin_cookies.split(';'))
                .chain(redirect_cookie.split(';'))
            {
                let part = part.trim();
                if !part.is_empty() && !all_parts.contains(&part) {
                    all_parts.push(part);
                }
            }
            let merged_cookie = all_parts.join("; ");

            eprintln!(
                "[QQMusic] 最终 cookie uin 提取测试: {:?}",
                extract_cookie_value(&merged_cookie)
            );
            eprintln!(
                "[QQMusic] 最终 cookie (前500字符): {}",
                &merged_cookie[..merged_cookie.len().min(500)]
            );

            return Ok(QQMusicQrCheckDto {
                status: "confirmed".to_string(),
                cookie: Some(merged_cookie),
                message: Some("登录成功 / Login successful.".to_string()),
            });
        }
        return Ok(QQMusicQrCheckDto {
            status: "waiting".to_string(),
            cookie: None,
            message: Some("登录重定向但无目标URL，继续等待".to_string()),
        });
    }

    // 非重定向响应（200），读取文本并解析 ptuiCB 回调
    let text = resp
        .text()
        .await
        .map_err(|e| format!("QQ登录响应读取失败: {e}"))?;

    // 调试 / Debug: 打印 ptqrlogin 原始响应，便于诊断扫码无反应问题
    eprintln!("[QQMusic] ptqrlogin HTTP status: {resp_status}");
    eprintln!(
        "[QQMusic] ptqrlogin 原始响应 (前500字符): {}",
        &text[..text.len().min(500)]
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
    // 状态码: 66=等待扫码, 67=已扫码待确认, 65=过期, 0=成功
    // 兼容单引号和双引号两种格式 / Compatible with both single and double quote formats
    let has_66 = text.contains("'66'") || text.contains("\"66\"") || text.contains("(66,");
    let has_67 = text.contains("'67'") || text.contains("\"67\"") || text.contains("(67,");
    let has_65 = text.contains("'65'") || text.contains("\"65\"") || text.contains("(65,");
    let has_0 = text.contains("'0'") || text.contains("\"0\"");
    eprintln!("[QQMusic] ptqrlogin 状态匹配: 66={has_66}, 67={has_67}, 65={has_65}, 0={has_0}");

    if has_66 {
        return Ok(QQMusicQrCheckDto {
            status: "waiting".to_string(),
            cookie: None,
            message: Some("等待扫码 / Waiting for scan.".to_string()),
        });
    }
    if has_67 {
        return Ok(QQMusicQrCheckDto {
            status: "scanned".to_string(),
            cookie: None,
            message: Some("已扫码，请在手机上确认 / Scanned, please confirm on phone.".to_string()),
        });
    }
    if has_65 {
        return Ok(QQMusicQrCheckDto {
            status: "expired".to_string(),
            cookie: None,
            message: Some("二维码已过期，请重新生成 / QR code expired.".to_string()),
        });
    }
    if has_0 {
        eprintln!(
            "[QQMusic] ptqrlogin 200 成功: {}",
            &text[..text.len().min(300)]
        );
        eprintln!("[QQMusic] ptqrlogin Set-Cookie: {ptqrlogin_cookies}");

        // 登录成功 → 合并 ptqrlogin 响应 cookie + 跟随重定向获取的 cookie
        let redirect_cookie =
            follow_qqmusic_login_redirect(&text, &ptqrlogin_cookies, all_cookies).await?;

        // 合并所有 cookie: 原始 cookie + ptqrlogin cookie + 重定向 cookie
        let mut all_parts: Vec<&str> = Vec::new();
        for part in all_cookies
            .split(';')
            .chain(ptqrlogin_cookies.split(';'))
            .chain(redirect_cookie.split(';'))
        {
            let part = part.trim();
            if !part.is_empty() && !all_parts.contains(&part) {
                all_parts.push(part);
            }
        }
        let merged_cookie = all_parts.join("; ");

        eprintln!(
            "[QQMusic] 最终 cookie uin 提取测试: {:?}",
            extract_cookie_value(&merged_cookie)
        );
        eprintln!(
            "[QQMusic] 最终 cookie (前500字符): {}",
            &merged_cookie[..merged_cookie.len().min(500)]
        );

        return Ok(QQMusicQrCheckDto {
            status: "confirmed".to_string(),
            cookie: Some(merged_cookie),
            message: Some("登录成功 / Login successful.".to_string()),
        });
    }

    // 未知响应不立即失败，继续等待扫码（避免临时网络问题导致二维码过早消失）
    Ok(QQMusicQrCheckDto {
        status: "waiting".to_string(),
        cookie: None,
        message: Some(format!("未知登录状态，继续等待: {text}")),
    })
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

    eprintln!("[QQMusic] 重定向 URL: {redirect_url}");

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
    let mut current_url = redirect_url.to_string();
    for _hop in 0..10u32 {
        let cookie_header = accumulated.join("; ");
        let resp = client
            .get(&current_url)
            .header("User-Agent", QQMUSIC_UA)
            .header("Referer", "https://y.qq.com/")
            .header("Cookie", &cookie_header)
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await
            .map_err(|e| format!("登录重定向请求失败(hop {_hop}): {e}"))?;

        // 收集本跳的 Set-Cookie
        for v in resp.headers().get_all("set-cookie").iter() {
            if let Ok(s) = v.to_str() {
                if let Some(name_value) = s.split(';').next() {
                    let name_value = name_value.trim();
                    if !name_value.is_empty() {
                        // 提取 cookie 值，跳过空值（删除 cookie，如 p_skey=; Expires=1970）
                        // 服务器会先设真实值再发空值删除其他域的同名 cookie，
                        // 如果不跳过空值，会用空值覆盖之前收集的有效值
                        let cookie_value = name_value.split('=').nth(1).unwrap_or("");
                        if cookie_value.is_empty() {
                            continue;
                        }
                        let cookie_name = name_value.split('=').next().unwrap_or("");
                        accumulated.retain(|a| !a.starts_with(&format!("{cookie_name}=")));
                        accumulated.push(name_value.to_string());
                    }
                }
            }
        }

        // 记算本跳新增的 cookie 数量
        let set_cookie_count = resp.headers().get_all("set-cookie").iter().count();
        eprintln!(
            "[QQMusic] hop {_hop}: status={}, Set-Cookie headers: {}, accumulated cookies: {}",
            resp.status(),
            set_cookie_count,
            accumulated.len()
        );
        if set_cookie_count > 0 {
            for v in resp.headers().get_all("set-cookie").iter() {
                if let Ok(s) = v.to_str() {
                    eprintln!(
                        "[QQMusic]   hop {_hop} Set-Cookie: {}",
                        &s[..s.len().min(200)]
                    );
                }
            }
        }

        // 检查是否为重定向
        if resp.status().is_redirection() {
            if let Some(loc) = resp.headers().get("location") {
                let next_url = loc.to_str().unwrap_or("");
                eprintln!(
                    "[QQMusic] hop {_hop} Location: {}",
                    &next_url[..next_url.len().min(1000)]
                );
                if next_url.is_empty() {
                    break;
                }
                // 处理相对 URL
                current_url = if next_url.starts_with("http") {
                    next_url.to_string()
                } else if next_url.starts_with('/') {
                    // 从当前 URL 提取 origin
                    let origin = current_url.split('/').take(3).collect::<Vec<_>>().join("/");
                    format!("{origin}{next_url}")
                } else {
                    next_url.to_string()
                };
                continue;
            }
        }
        // 非重定向响应（200 OK），尝试从 HTML 中提取跳转 URL
        let body = resp.text().await.unwrap_or_default();
        let body_preview = &body[..body.len().min(2000)];
        eprintln!("[QQMusic] hop {} 200 body(前2000): {}", _hop, body_preview);
        if body.contains("postMessage") || body.contains("qclogin") {
            eprintln!("[QQMusic] hop {} 发现 postMessage/qclogin!", _hop);
            for kw in &["postMessage", "qclogin", "access_token", "code="] {
                if let Some(pos) = body.find(kw) {
                    let s = pos.saturating_sub(100);
                    let e = (pos + 200).min(body.len());
                    eprintln!("[QQMusic] hop {} '{}' ctx: {}", _hop, kw, &body[s..e]);
                }
            }
        }

        let mut found_url: Option<String> = None;

        // 查找 meta refresh: content="0;url=..." 或 url=...
        if let Some(pos) = body.find("url=") {
            let after = &body[pos + 4..];
            let after = after.trim_start_matches('"').trim_start_matches('\'');
            let end = after
                .find(['"', '\'', '>', ';'])
                .unwrap_or(after.len().min(500));
            let extracted = &after[..end];
            if extracted.starts_with("http") {
                eprintln!(
                    "[QQMusic] hop {} HTML跳转URL: {}",
                    _hop,
                    &extracted[..extracted.len().min(300)]
                );
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
                let end = after
                    .find(['"', '\'', ')', ';'])
                    .unwrap_or(after.len().min(500));
                let extracted = &after[..end];
                if extracted.starts_with("http") {
                    eprintln!(
                        "[QQMusic] hop {} JS跳转URL: {}",
                        _hop,
                        &extracted[..extracted.len().min(300)]
                    );
                    found_url = Some(extracted.to_string());
                    break;
                }
            }
        }

        if let Some(next_url) = found_url {
            current_url = next_url;
            continue;
        }

        break;
    }

    // 访问 y.qq.com 获取 QQ 音乐专用 cookie（如 qqmusic_key, qt 等）
    // 手动跟随重定向链（最多 5 跳），因为 qqmusic_key 可能在重定向中设置
    let mut yqq_url = "https://y.qq.com/".to_string();
    for _yhop in 0..5u32 {
        let cookie_header = accumulated.join("; ");
        let resp_y = client
            .get(&yqq_url)
            .header("User-Agent", QQMUSIC_UA)
            .header("Referer", "https://y.qq.com/")
            .header("Cookie", &cookie_header)
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await
            .map_err(|e| format!("Cookie fetch from y.qq.com failed(hop {_yhop}): {e}"))?;

        let y_set_cookie_count = resp_y.headers().get_all("set-cookie").iter().count();
        eprintln!(
            "[QQMusic] y.qq.com hop {_yhop}: status={}, Set-Cookie headers: {}",
            resp_y.status(),
            y_set_cookie_count
        );

        for v in resp_y.headers().get_all("set-cookie").iter() {
            if let Ok(s) = v.to_str() {
                eprintln!(
                    "[QQMusic]   y.qq.com hop {_yhop} Set-Cookie: {}",
                    &s[..s.len().min(200)]
                );
                if let Some(name_value) = s.split(';').next() {
                    let name_value = name_value.trim();
                    if !name_value.is_empty() {
                        let cookie_value = name_value.split('=').nth(1).unwrap_or("");
                        if cookie_value.is_empty() {
                            continue;
                        }
                        let cookie_name = name_value.split('=').next().unwrap_or("");
                        accumulated.retain(|a| !a.starts_with(&format!("{cookie_name}=")));
                        accumulated.push(name_value.to_string());
                    }
                }
            }
        }

        if resp_y.status().is_redirection() {
            if let Some(loc) = resp_y.headers().get("location") {
                let next_url = loc.to_str().unwrap_or("");
                eprintln!(
                    "[QQMusic] y.qq.com hop {_yhop} Location: {}",
                    &next_url[..next_url.len().min(200)]
                );
                if next_url.is_empty() {
                    break;
                }
                yqq_url = if next_url.starts_with("http") {
                    next_url.to_string()
                } else if next_url.starts_with('/') {
                    let origin = yqq_url.split('/').take(3).collect::<Vec<_>>().join("/");
                    format!("{origin}{next_url}")
                } else {
                    next_url.to_string()
                };
                continue;
            }
        }
        // 200 OK: 尝试从 HTML 中提取跳转 URL
        let y_body = resp_y.text().await.unwrap_or_default();
        let y_body_preview = &y_body[..y_body.len().min(2000)];
        eprintln!(
            "[QQMusic] y.qq.com hop {_yhop} 200 body(前500): {}",
            y_body_preview
        );

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
                let end = after
                    .find(['"', '\'', ')', ';'])
                    .unwrap_or(after.len().min(500));
                let extracted = &after[..end];
                if extracted.starts_with("http") {
                    eprintln!(
                        "[QQMusic] y.qq.com hop {_yhop} JS跳转URL: {}",
                        &extracted[..extracted.len().min(300)]
                    );
                    y_found_url = Some(extracted.to_string());
                    break;
                }
            }
        }
        if y_found_url.is_none() {
            if let Some(pos) = y_body.find("url=") {
                let after = &y_body[pos + 4..];
                let after = after.trim_start_matches('"').trim_start_matches('\'');
                let end = after
                    .find(['"', '\'', '>', ';'])
                    .unwrap_or(after.len().min(500));
                let extracted = &after[..end];
                if extracted.starts_with("http") {
                    eprintln!(
                        "[QQMusic] y.qq.com hop {_yhop} HTML跳转URL: {}",
                        &extracted[..extracted.len().min(300)]
                    );
                    y_found_url = Some(extracted.to_string());
                }
            }
        }
        if let Some(next_url) = y_found_url {
            yqq_url = next_url;
            continue;
        }
        break;
    }

    // 后备：如果仍未获取 qqmusic_key，尝试多种方式获取
    // 尝试调用 fcg_music_oauth_get_accesstoken.fcg 用 QQ Connect cookie 换取 qqmusic_key
    let has_qqmusic_key = accumulated.iter().any(|a| a.starts_with("qqmusic_key="));
    if !has_qqmusic_key {
        eprintln!("[QQMusic] 尝试 fcg_music_oauth_get_accesstoken.fcg 端点...");
        let cookie_str = accumulated.join("; ");
        // 尝试多种参数组合
        let oauth_urls = vec![
            format!("https://u.y.qq.com/cgi-bin/fcg_music_oauth_get_accesstoken.fcg?client_id=100497308&format=json&inCharset=utf8&outCharset=utf-8"),
            format!("https://u.y.qq.com/cgi-bin/fcg_music_oauth_get_accesstoken.fcg?client_id=100497308&grant_type=authorization_code&format=json"),
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
                eprintln!(
                    "[QQMusic] oauth_get_accesstoken: status={or_status}, body={}",
                    &or_text[..or_text.len().min(500)]
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
                                eprintln!(
                                    "[QQMusic] 从 oauth 端点获取 {} = {}...",
                                    field,
                                    &val[..val.len().min(20)]
                                );
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
                                    eprintln!(
                                        "[QQMusic] 从 oauth data 获取 {} = {}...",
                                        field,
                                        &val[..val.len().min(20)]
                                    );
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
        eprintln!("[QQMusic] 尝试 POST fcg_music_oauth_get_accesstoken.fcg...");
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
            eprintln!(
                "[QQMusic] GetLoginInfo: status={opr_status}, body={}",
                &opr_text[..opr_text.len().min(500)]
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
        let mut fallback_source = "";

        for a in accumulated.iter() {
            if a.starts_with("pt_oauth_token=") {
                let val = a.strip_prefix("pt_oauth_token=").unwrap_or("");
                if !val.is_empty() {
                    fallback_key = Some(val.to_string());
                    fallback_source = "pt_oauth_token";
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
                        fallback_source = "superkey";
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
                        fallback_source = "p_skey";
                        break;
                    }
                }
            }
        }

        if let Some(key_val) = fallback_key {
            eprintln!("[QQMusic] 尝试用 {} 作为 qqmusic_key 替代", fallback_source);
            accumulated.push(format!("qqmusic_key={}", key_val));
        }
    }

    if !has_qqmusic_key {
        let cookie_header = accumulated.join("; ");

        // 方式A: 尝试调用 QQ Connect authorize 端点获取授权码
        eprintln!("[QQMusic] qqmusic_key 未获取，尝试 QQ Connect authorize 端点");
        // 方式A1: 尝试 Implicit Grant (response_type=token) - 不需要 client_secret
        let authorize_url = "https://graph.qq.com/oauth2.0/authorize?response_type=token&client_id=100497308&redirect_uri=https%3A%2F%2Fy.qq.com%2Fportal%2Fwx_redirect.html&state=qqmusic_login&scope=get_user_info";
        let noredirect_client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        let authorize_resp = noredirect_client
            .get(authorize_url)
            .header("Cookie", &cookie_header)
            .header("User-Agent", QQMUSIC_UA)
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await;

        if let Ok(resp) = authorize_resp {
            let status = resp.status();
            eprintln!("[QQMusic] authorize 响应状态: {status}");
            // 打印所有响应头
            for (k, v) in resp.headers().iter() {
                eprintln!(
                    "[QQMusic] authorize header {}: {}",
                    k,
                    v.to_str().unwrap_or("<binary>")
                );
            }
            // 收集 Set-Cookie
            for cookie in resp.headers().get_all("set-cookie").iter() {
                if let Ok(s) = cookie.to_str() {
                    eprintln!("[QQMusic] authorize Set-Cookie: {}", &s[..s.len().min(200)]);
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
                eprintln!(
                    "[QQMusic] authorize 重定向URL: {}",
                    &loc_str[..loc_str.len().min(1000)]
                );
                // 检查URL中是否包含 access_token (Implicit Grant, 在 fragment # 中)
                if loc_str.contains("access_token=") {
                    eprintln!("[QQMusic] authorize URL 包含 access_token (Implicit Grant)!");
                    // 提取 access_token
                    let at_start = loc_str.find("access_token=").unwrap();
                    let at_end = loc_str[at_start..]
                        .find('&')
                        .map(|p| at_start + p)
                        .unwrap_or(loc_str.len());
                    let access_token = &loc_str[at_start + 13..at_end];
                    eprintln!(
                        "[QQMusic] 提取到 access_token: {}...",
                        &access_token[..access_token.len().min(20)]
                    );
                    // 将 access_token 作为 qqmusic_key 使用
                    accumulated.retain(|a| !a.starts_with("qqmusic_key="));
                    accumulated.push(format!("qqmusic_key={}", access_token));
                }
                // 检查URL中是否包含 code 参数
                if loc_str.contains("code=") {
                    eprintln!("[QQMusic] authorize URL 包含 code 参数!");
                    // 提取 code
                    if let Some(code_start) = loc_str.find("code=") {
                        let code_end = loc_str[code_start..]
                            .find('&')
                            .map(|p| code_start + p)
                            .unwrap_or(loc_str.len());
                        let code = &loc_str[code_start + 5..code_end];
                        eprintln!("[QQMusic] 提取到授权码: {}...", &code[..code.len().min(20)]);

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
                            let token_text = tr.text().await.unwrap_or_default();
                            eprintln!(
                                "[QQMusic] token 响应: {}",
                                &token_text[..token_text.len().min(500)]
                            );
                        }
                    }
                } else {
                    // authorize 重定向到 fast_authorize 或其他中间 URL
                    // 跟随重定向链（最多 5 跳），寻找 code= 参数或 Set-Cookie 中的 qqmusic_key
                    eprintln!("[QQMusic] authorize 重定向不含 code，跟随重定向链...");
                    let mut auth_url = loc_str.to_string();
                    for auth_hop in 0..5u32 {
                        let auth_resp = noredirect_client
                            .get(&auth_url)
                            .header("Cookie", &cookie_header)
                            .header("User-Agent", QQMUSIC_UA)
                            .header("Referer", "https://graph.qq.com/")
                            .timeout(std::time::Duration::from_secs(10))
                            .send()
                            .await;
                        match auth_resp {
                            Ok(ar) => {
                                let ar_status = ar.status();
                                eprintln!("[QQMusic] auth hop {auth_hop}: status={ar_status}");
                                // 打印所有响应头
                                for (hk, hv) in ar.headers().iter() {
                                    eprintln!(
                                        "[QQMusic]   auth hop {auth_hop} header {}: {}",
                                        hk,
                                        hv.to_str().unwrap_or("<binary>")
                                    );
                                }
                                // 收集 Set-Cookie
                                for sc in ar.headers().get_all("set-cookie").iter() {
                                    if let Ok(s) = sc.to_str() {
                                        eprintln!(
                                            "[QQMusic]   auth hop {auth_hop} Set-Cookie: {}",
                                            &s[..s.len().min(300)]
                                        );
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
                                                eprintln!(
                                                    "[QQMusic]   ✅ 收集 cookie: {}={}",
                                                    name,
                                                    &value[..value.len().min(50)]
                                                );
                                            }
                                        }
                                    }
                                }
                                // 检查重定向
                                if ar_status.is_redirection() {
                                    if let Some(al) = ar.headers().get("location") {
                                        let al_str = al.to_str().unwrap_or("");
                                        eprintln!(
                                            "[QQMusic]   auth hop {auth_hop} Location: {}",
                                            &al_str[..al_str.len().min(1000)]
                                        );
                                        // 检查是否包含 code= 参数
                                        if al_str.contains("code=") {
                                            eprintln!("[QQMusic]   ✅ 发现 code= 参数!");
                                            // 如果重定向到 y.qq.com，跟随它（可能设置 qqmusic_key）
                                            if al_str.contains("y.qq.com")
                                                || al_str.contains("wx_redirect")
                                            {
                                                eprintln!("[QQMusic]   跟随到 y.qq.com 获取 qqmusic_key...");
                                                let wx_url = if al_str.starts_with("http") {
                                                    al_str.to_string()
                                                } else {
                                                    format!("https://y.qq.com{}", al_str)
                                                };
                                                let wx_resp = client
                                                    .get(&wx_url)
                                                    .header("Cookie", &cookie_header)
                                                    .header("User-Agent", QQMUSIC_UA)
                                                    .header("Referer", "https://y.qq.com/")
                                                    .timeout(std::time::Duration::from_secs(10))
                                                    .send()
                                                    .await;
                                                if let Ok(wr) = wx_resp {
                                                    eprintln!(
                                                        "[QQMusic]   wx_redirect status: {}",
                                                        wr.status()
                                                    );
                                                    for sc in
                                                        wr.headers().get_all("set-cookie").iter()
                                                    {
                                                        if let Ok(s) = sc.to_str() {
                                                            eprintln!("[QQMusic]   wx_redirect Set-Cookie: {}", &s[..s.len().min(300)]);
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
                                                                    eprintln!("[QQMusic]   ✅ 收集 cookie: {}={}", name, &value[..value.len().min(50)]);
                                                                }
                                                            }
                                                        }
                                                    }
                                                }
                                                break;
                                            }
                                        }
                                        // 继续跟随
                                        auth_url = if al_str.starts_with("http") {
                                            al_str.to_string()
                                        } else if al_str.starts_with('/') {
                                            "https://graph.qq.com".to_string() + al_str
                                        } else {
                                            al_str.to_string()
                                        };
                                        continue;
                                    }
                                }
                                // 非重定向，读取 body 检查是否有跳转或表单
                                let body = ar.text().await.unwrap_or_default();
                                eprintln!(
                                    "[QQMusic]   auth hop {auth_hop} 200 body(前2000): {}",
                                    &body[..body.len().min(2000)]
                                );

                                // 搜索 access_token 在 body 中
                                if body.contains("access_token=") {
                                    eprintln!("[QQMusic]   ✅ body 含 access_token!");
                                    if let Some(at_pos) = body.find("access_token=") {
                                        let at_end = body[at_pos..]
                                            .find('&')
                                            .map(|p| at_pos + p)
                                            .unwrap_or(body.len());
                                        let at_val = &body[at_pos + 13..at_end];
                                        if !at_val.is_empty() {
                                            eprintln!(
                                                "[QQMusic]   提取 access_token: {}...",
                                                &at_val[..at_val.len().min(30)]
                                            );
                                            accumulated.retain(|a| !a.starts_with("qqmusic_key="));
                                            accumulated.push(format!("qqmusic_key={}", at_val));
                                        }
                                    }
                                }

                                // 搜索 code= 参数
                                if body.contains("code=") {
                                    eprintln!("[QQMusic]   ✅ body 含 code=!");
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
                                                if url.contains("http")
                                                    || url.contains("wx_redirect")
                                                    || url.contains("access_token")
                                                    || url.contains("code=")
                                                {
                                                    eprintln!(
                                                        "[QQMusic]   ✅ 发现 JS 跳转: {}",
                                                        &url[..url.len().min(500)]
                                                    );
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
                                            if url.contains("http")
                                                || url.contains("wx_redirect")
                                                || url.contains("access_token")
                                                || url.contains("code=")
                                            {
                                                eprintln!(
                                                    "[QQMusic]   ✅ 发现 meta refresh URL: {}",
                                                    &url[..url.len().min(500)]
                                                );
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
                                                    eprintln!(
                                                        "[QQMusic]   ✅ 发现 form action: {}",
                                                        &url[..url.len().min(500)]
                                                    );
                                                    let form_url = if url.starts_with("http") {
                                                        url.to_string()
                                                    } else if url.starts_with('/') {
                                                        format!("https://graph.qq.com{}", url)
                                                    } else {
                                                        format!("https://graph.qq.com/{}", url)
                                                    };
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
                                                    eprintln!(
                                                        "[QQMusic]   form hidden params: {:?}",
                                                        &form_params
                                                    );
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
                                                    eprintln!(
                                                        "[QQMusic]   提交 form: {}",
                                                        &form_url_with_params
                                                            [..form_url_with_params
                                                                .len()
                                                                .min(1000)]
                                                    );
                                                    found_auth_url = Some(form_url_with_params);
                                                }
                                            }
                                        }
                                    }
                                }

                                if let Some(next_url) = found_auth_url {
                                    auth_url = next_url;
                                    continue;
                                }
                                break;
                            }
                            Err(e) => {
                                eprintln!("[QQMusic]   auth hop {auth_hop} 错误: {e}");
                                break;
                            }
                        }
                    }
                }
            }
        }

        // 方式B: 尝试访问 player.html
        eprintln!("[QQMusic] 尝试访问 player.html");
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
            eprintln!(
                "[QQMusic] player.html: status={}, Set-Cookie headers: {}",
                resp_p.status(),
                p_set_cookie_count
            );
            for v in resp_p.headers().get_all("set-cookie").iter() {
                if let Ok(s) = v.to_str() {
                    eprintln!(
                        "[QQMusic]   player.html Set-Cookie: {}",
                        &s[..s.len().min(200)]
                    );
                    if let Some(name_value) = s.split(';').next() {
                        let name_value = name_value.trim();
                        if !name_value.is_empty() {
                            let cookie_value = name_value.split('=').nth(1).unwrap_or("");
                            if cookie_value.is_empty() {
                                continue;
                            }
                            let cookie_name = name_value.split('=').next().unwrap_or("");
                            accumulated.retain(|a| !a.starts_with(&format!("{cookie_name}=")));
                            accumulated.push(name_value.to_string());
                        }
                    }
                }
            }
        }
    }

    let final_cookie = accumulated.join("; ");
    if final_cookie.is_empty() {
        return Err("无法获取QQ音乐登录凭据 / Cannot get QQ Music login cookie.".to_string());
    }

    eprintln!(
        "[QQMusic] follow_redirect 完成, uin 提取测试: {:?}",
        extract_cookie_value(&final_cookie)
    );
    let pskey_val = final_cookie
        .split(';')
        .find_map(|p| {
            let p = p.trim();
            if p.starts_with("p_skey=") {
                Some(p)
            } else {
                None
            }
        })
        .unwrap_or("");
    eprintln!(
        "[QQMusic] follow_redirect: p_skey 实际值: {}",
        &pskey_val[..pskey_val.len().min(80)]
    );
    eprintln!(
        "[QQMusic] follow_redirect: 含 superkey={}",
        final_cookie.contains("superkey=")
    );
    eprintln!(
        "[QQMusic] follow_redirect: 含 qqmusic_key={}",
        final_cookie.contains("qqmusic_key=")
    );
    eprintln!(
        "[QQMusic] follow_redirect final cookie (前500字符): {}",
        &final_cookie[..final_cookie.len().min(500)]
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
        id: songmid.clone(),
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
    eprintln!(
        "[QQMusic] vip_status 诊断: uin_num={uin}, uin_str='{uin_str}', cookie_len={}",
        cookie_str.len()
    );
    eprintln!(
        "[QQMusic]   has qqmusic_key={}, has p_skey={}, has skey={}, has uin={}",
        extract_qqmusic_signing_key(cookie_str).is_some(),
        extract_cookie_raw(cookie_str, "p_skey").is_some(),
        extract_cookie_raw(cookie_str, "skey").is_some(),
        extract_cookie_raw(cookie_str, "uin").is_some()
    );
    eprintln!(
        "[QQMusic]   cookie (前200字符): {}",
        safe_preview(cookie_str, 200)
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
            let resp_summary = serde_json::to_string(&value).unwrap_or_default();
            eprintln!(
                "[QQMusic] vip_status API响应: {}",
                &resp_summary[..resp_summary.len().min(800)]
            );

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

            // 打印 map entry 的所有 key 和 VIP 相关字段用于调试
            if let Some(entry) = map_entry {
                if let Some(obj) = entry.as_object() {
                    let keys: Vec<&String> = obj.keys().collect();
                    eprintln!("[QQMusic] vip_status: map entry keys = {:?}", keys);
                    for key in &[
                        "vipType",
                        "viptype",
                        "vipLevel",
                        "isVip",
                        "isVip2",
                        "greenVipLevel",
                        "superVipLevel",
                        "baseInfo",
                        "info",
                        "vipInfo",
                        "nick",
                    ] {
                        if let Some(val) = obj.get(*key) {
                            let val_str = serde_json::to_string(val).unwrap_or_default();
                            eprintln!(
                                "[QQMusic]   map entry['{}'] = {}",
                                key,
                                &val_str[..val_str.len().min(200)]
                            );
                        }
                    }
                }
            }

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
                    // API rejected our request format (e.g. 500005),
                    // but user has valid cookie — don't claim non-member.
                    Ok(QQMusicVipStatusDto {
                        is_member: true,
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
                Ok(QQMusicVipStatusDto {
                    is_member: true, // Don't claim non-member; user is logged in
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
        let resp_summary = serde_json::to_string(&value).unwrap_or_default();
        eprintln!(
            "[QQMusic] vip_status GetLoginInfo回退响应: {}",
            &resp_summary[..resp_summary.len().min(800)]
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
            // 打印 data 的所有 key 用于调试
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
}
