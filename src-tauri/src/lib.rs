use std::{
    collections::{HashMap, HashSet},
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

use base64::{engine::general_purpose, Engine as _};
use lofty::{prelude::*, probe::Probe};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};
use tauri::path::BaseDirectory;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_dialog::DialogExt;
use walkdir::WalkDir;

pub mod qqmusic;
use qqmusic::{
    delete_qqmusic_token, ensure_qqmusic_source_enabled, fetch_qqmusic_liked_songs,
    fetch_qqmusic_lyrics, fetch_qqmusic_playable_url, fetch_qqmusic_playlist,
    fetch_qqmusic_song_metadata, fetch_qqmusic_user_playlists, fetch_qqmusic_user_profile,
    fetch_qqmusic_vip_status, load_qqmusic_source_config, proxy_qqmusic_playback,
    proxy_qqmusic_search_covers, proxy_qqmusic_track_covers, read_qqmusic_token,
    resolve_qqmusic_source_config, save_qqmusic_source_config_to_db, save_qqmusic_token,
    search_qqmusic, test_qqmusic_connection, QQMUSIC_DEFAULT_BASE_URL, QQMUSIC_UA,
};

const AUDIO_EXTENSIONS: &[&str] = &["mp3", "flac", "wav", "m4a"];
const INITIAL_SCHEMA: &str = include_str!("../migrations/001_initial_schema.sql");
const MIGRATION_002: &str = include_str!("../migrations/002_mood_note_rename_and_indexes.sql");
const MIGRATION_003: &str = include_str!("../migrations/003_authorized_music_directories.sql");
const LLM_KEYRING_SERVICE: &str = "ome.music.provider";
const LLM_KEYRING_ACCOUNT: &str = "local";
const NETEASE_KEYRING_SERVICE: &str = "ome.music.source.netease";
const NETEASE_KEYRING_ACCOUNT: &str = "local";
const BILIBILI_KEYRING_SERVICE: &str = "ome.music.source.bilibili";
const BILIBILI_KEYRING_ACCOUNT: &str = "local";
const BILIBILI_DEFAULT_BASE_URL: &str = "https://api.bilibili.com";
const BILIBILI_BROWSER_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

pub struct AppState {
    db: Mutex<Connection>,
    media_proxy: Mutex<HashMap<String, MediaProxyEntry>>,
    managed_netease_api: Option<ManagedNeteaseApiRuntime>,
    pub managed_netease_child: Arc<Mutex<Option<Child>>>,
}

#[derive(Clone, Debug)]
struct ManagedNeteaseApiRuntime {
    node_exe: PathBuf,
    app_js: PathBuf,
}

#[derive(Clone)]
struct MediaProxyEntry {
    urls: Vec<String>,
    kind: &'static str,
    expires_at: std::time::SystemTime,
}

const MEDIA_PROXY_TTL: std::time::Duration = std::time::Duration::from_secs(60 * 60);
const MEDIA_PROXY_MAX_ENTRIES: usize = 256;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalAudioFile {
    path: String,
    extension: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TrackDto {
    id: String,
    title: String,
    artist: String,
    album: String,
    duration_seconds: u64,
    file_path: String,
    source: String,
    source_id: Option<String>,
    unavailable_reason: Option<String>,
    cover_url: String,
    genres: Vec<String>,
    moods: Vec<String>,
    language: String,
    year: Option<i32>,
    play_count: u32,
    skip_count: u32,
    liked: bool,
    imported_at: String,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ProfileRank {
    label: String,
    weight: f64,
    confidence: f64,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct HourPreference {
    hour: u8,
    weight: f64,
    confidence: f64,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ScoreConfidence {
    score: f64,
    confidence: f64,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct UserProfileDto {
    favorite_artists: Vec<ProfileRank>,
    favorite_albums: Vec<ProfileRank>,
    favorite_genres: Vec<ProfileRank>,
    favorite_moods: Vec<ProfileRank>,
    preferred_listening_hours: Vec<HourPreference>,
    night_listening_preference: ScoreConfidence,
    skip_patterns: Vec<ProfileRank>,
    repeat_patterns: Vec<ProfileRank>,
    liked_song_patterns: Vec<ProfileRank>,
    exploration_score: ScoreConfidence,
    calm_music_preference: ScoreConfidence,
    energetic_music_preference: ScoreConfidence,
    event_count: u32,
    confidence: f64,
    is_learning: bool,
    updated_at: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MoodEntryDto {
    id: String,
    date: String,
    mood: String,
    mood_signal: String,
    note: String,
    desired_vibe: Option<String>,
    private_tags: Vec<String>,
    recommended_track_ids: Vec<String>,
    created_at: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveMoodEntryPayload {
    date: String,
    mood: String,
    mood_signal: String,
    note: String,
    desired_vibe: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportResult {
    directory: Option<String>,
    imported_count: usize,
    skipped_count: usize,
    tracks: Vec<TrackDto>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackEventPayload {
    track_id: String,
    event_type: PlaybackEventType,
    position_seconds: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LlmProviderConfigDto {
    provider_name: String,
    base_url: String,
    model: String,
    masked_api_key: String,
    has_api_key: bool,
    configured: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveLlmProviderConfigPayload {
    provider_name: String,
    base_url: String,
    model: String,
    api_key: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FetchLlmModelsPayload {
    base_url: String,
    api_key: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LlmModelListResponse {
    models: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LlmTextRequestPayload {
    purpose: String,
    system_prompt: String,
    user_prompt: String,
    max_tokens: Option<u32>,
    temperature: Option<f32>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LlmTextResponse {
    provider_name: String,
    model: String,
    text: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpeechTranscriptionPayload {
    audio_base64: String,
    mime_type: String,
    model: Option<String>,
    language: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpeechTranscriptionResponse {
    text: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpeechSynthesisPayload {
    text: String,
    model: Option<String>,
    voice: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpeechSynthesisResponse {
    audio_data_url: String,
    mime_type: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavePlaylistAnalysisPayload {
    playlist_id: String,
    playlist_name: String,
    track_count: u32,
    mode: String,
    provider: String,
    chunk_results_json: String,
    final_result_json: String,
    report_json: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistAnalysisRunDto {
    id: String,
    playlist_id: String,
    playlist_name: String,
    track_count: u32,
    mode: String,
    provider: String,
    chunk_results_json: String,
    final_result_json: String,
    report_json: String,
    created_at: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NeteaseSourceConfigDto {
    enabled: bool,
    base_url: String,
    has_token: bool,
    masked_token: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BilibiliSourceConfigDto {
    enabled: bool,
    base_url: String,
    has_token: bool,
    masked_token: String,
    search_scope: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NeteaseServiceStatusDto {
    running: bool,
    started: bool,
    base_url: String,
    message: String,
    node_available: bool,
    api_package_found: bool,
    stage: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveNeteaseSourceConfigPayload {
    enabled: bool,
    base_url: String,
    token: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveBilibiliSourceConfigPayload {
    enabled: bool,
    base_url: Option<String>,
    token: Option<String>,
    search_scope: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BilibiliLoginStatusDto {
    logged_in: bool,
    expired: bool,
    nickname: Option<String>,
    user_id: Option<String>,
    message: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NeteaseQrLoginDto {
    key: String,
    qr_url: String,
    qr_img: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NeteaseQrCheckPayload {
    key: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NeteaseCookiePayload {
    cookie: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NeteasePasswordLoginPayload {
    account: String,
    password: String,
    country_code: Option<String>,
    login_type: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NeteaseSmsPayload {
    phone: String,
    country_code: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NeteaseSmsLoginPayload {
    phone: String,
    code: String,
    country_code: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceWebLoginPayload {
    source: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NeteaseLoginStatusDto {
    logged_in: bool,
    expired: bool,
    login_status_known: bool,
    has_cookie: bool,
    masked_cookie: String,
    nickname: Option<String>,
    user_id: Option<String>,
    avatar_url: Option<String>,
    message: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NeteaseVipStatusDto {
    is_member: bool,
    level: Option<String>,
    message: String,
    // Distinguishes "we asked NetEase and the account is not a member" from
    // "the /vip/info endpoint failed, so we genuinely don't know". Without this
    // flag, an API failure surfaces as `is_member: false` and the UI shows
    // "Non-member", which misleads users into thinking their membership lapsed.
    membership_known: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NeteaseUserProfileDto {
    logged_in: bool,
    nickname: Option<String>,
    user_id: Option<String>,
    avatar_url: Option<String>,
    vip: Option<NeteaseVipStatusDto>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NeteaseQrCheckDto {
    status: String,
    code: i64,
    message: String,
    login_status: Option<NeteaseLoginStatusDto>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BilibiliQrCheckDto {
    status: String,
    code: i64,
    message: String,
    login_status: Option<BilibiliLoginStatusDto>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceSearchPayload {
    query: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourcePlaylistPayload {
    playlist_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceSongPayload {
    song_id: String,
    level: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SourceSongDto {
    id: String,
    source: Option<String>,
    title: String,
    artist: String,
    album: String,
    duration_seconds: u64,
    cover_url: String,
    playable_url: Option<String>,
    unavailable: bool,
    unavailable_reason: Option<String>,
    bvid: Option<String>,
    aid: Option<String>,
    cid: Option<String>,
    uploader: Option<String>,
    danmaku_count: Option<u64>,
    play_count: Option<u64>,
    page_index: Option<u32>,
    source_url: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourcePlaylistDto {
    id: String,
    name: String,
    description: String,
    source: String,
    tracks: Vec<SourceSongDto>,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct NeteaseUserPlaylistDto {
    id: String,
    name: String,
    track_count: u32,
    creator_name: String,
    subscribed: bool,
    cover_url: String,
    description: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NeteaseTasteSyncPayload {
    include_liked_songs: Option<bool>,
    include_playlists: Option<bool>,
    playlist_ids: Option<Vec<String>>,
    liked_limit: Option<u32>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TasteNotesDto {
    id: String,
    source: String,
    track_count: u32,
    playlist_count: u32,
    music_personality: String,
    favorite_artists: Vec<String>,
    favorite_albums: Vec<String>,
    favorite_languages: Vec<String>,
    favorite_moods: Vec<String>,
    favorite_scenes: Vec<String>,
    hidden_patterns: Vec<String>,
    recommendation_strategy: String,
    confidence: f64,
    updated_at: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NeteaseTasteSyncResultDto {
    liked_count: u32,
    playlist_count: u32,
    imported_track_count: u32,
    analyzed_track_count: u32,
    taste_notes: TasteNotesDto,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceLyricsDto {
    song_id: String,
    lyrics: String,
    translated_lyrics: String,
    source: String,
    cache_key: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DanmakuItemDto {
    id: String,
    source: String,
    cid: String,
    time: f64,
    text: String,
    mode: String,
    color: String,
    font_size: String,
    timestamp: String,
    user_hash: String,
    weight: f64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DanmakuResponseDto {
    source: String,
    id: String,
    cid: String,
    cache_key: String,
    items: Vec<DanmakuItemDto>,
    debug: DanmakuDebugDto,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DanmakuDebugDto {
    bvid: String,
    aid: Option<String>,
    cid: String,
    danmaku_request_url: String,
    raw_danmaku_loaded: bool,
    raw_danmaku_length: usize,
    parsed_danmaku_count: usize,
    first_danmaku_time: Option<f64>,
    from_cache: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DanmakuPayload {
    source: String,
    id: String,
    cid: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct StorageBucketDto {
    label: String,
    bytes: u64,
    display_size: String,
    path: String,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct StorageReportDto {
    app_cache: StorageBucketDto,
    webview_cache: StorageBucketDto,
    cover_cache: StorageBucketDto,
    lyrics_cache: StorageBucketDto,
    logs: StorageBucketDto,
    database: StorageBucketDto,
    total_cache_bytes: u64,
    total_cache_display_size: String,
    generated_at: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClearStoragePayload {
    kind: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolveLyricsPayload {
    track: TrackDto,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveLyricOffsetPayload {
    cache_key: String,
    offset_ms: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportTrackLyricsPayload {
    track: TrackDto,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedLyricsDto {
    cache_key: String,
    source: String,
    lyrics: String,
    translated_lyrics: String,
    confidence: f64,
    warning: Option<String>,
    offset_ms: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayableUrlDto {
    song_id: String,
    url: Option<String>,
    video_url: Option<String>,
    unavailable: bool,
    reason: Option<String>,
    debug: Option<NeteasePlaybackDebugDto>,
    #[serde(skip_serializing)]
    audio_candidates: Vec<String>,
    #[serde(skip_serializing)]
    video_candidates: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BilibiliImportResultDto {
    tracks: Vec<TrackDto>,
    playback: PlayableUrlDto,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct NeteasePlaybackAttemptDto {
    level: String,
    endpoint: String,
    response_code: Option<i64>,
    has_url: bool,
    returned_level: Option<String>,
    reason: Option<String>,
    message: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct NeteasePlaybackDebugDto {
    is_logged_in: bool,
    has_cookie: bool,
    masked_cookie: String,
    user_id: Option<String>,
    vip_status: Option<String>,
    requested_song_id: String,
    requested_level: String,
    endpoint: String,
    response_code: Option<i64>,
    has_url: bool,
    returned_level: Option<String>,
    fee: Option<i64>,
    privilege: Option<String>,
    reason: Option<String>,
    message: Option<String>,
    attempts: Vec<NeteasePlaybackAttemptDto>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceConnectionDto {
    ok: bool,
    message: String,
}

#[derive(Debug, Clone)]
struct StoredLlmProviderConfig {
    provider_name: String,
    base_url: String,
    model: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
enum PlaybackEventType {
    Play,
    Pause,
    Skip,
    Completed,
    Liked,
    Unliked,
    Replayed,
}

impl PlaybackEventType {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Play => "play",
            Self::Pause => "pause",
            Self::Skip => "skip",
            Self::Completed => "completed",
            Self::Liked => "liked",
            Self::Unliked => "unliked",
            Self::Replayed => "replayed",
        }
    }
}

#[derive(Debug)]
struct ParsedTrack {
    id: String,
    title: String,
    artist: String,
    album: String,
    duration_seconds: u64,
    file_path: String,
    file_extension: String,
    source: String,
    source_id: Option<String>,
    unavailable_reason: Option<String>,
    cover_url: Option<String>,
    genres: Vec<String>,
    moods: Vec<String>,
    calm_score: f64,
    energetic_score: f64,
    year: Option<i32>,
}

struct ProfileEvent {
    track_id: String,
    event_type: String,
    hour_of_day: u8,
    weight: f64,
    artist: String,
    album: String,
    genres: Vec<String>,
    moods: Vec<String>,
    calm_score: f64,
    energetic_score: f64,
}

#[tauri::command]
fn get_app_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[tauri::command]
fn get_storage_report(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<StorageReportDto, String> {
    build_storage_report(&app, &state)
}

#[tauri::command]
fn clear_storage_bucket(
    app: AppHandle,
    state: State<'_, AppState>,
    payload: ClearStoragePayload,
) -> Result<StorageReportDto, String> {
    match payload.kind.as_str() {
        "appCache" => {
            for path in app_cache_paths(&app)? {
                clear_path_contents(&path)?;
            }
            for path in webview_cache_paths(&app)? {
                clear_path_contents(&path)?;
            }
        }
        "coverCache" => {
            for path in cover_cache_paths(&app)? {
                clear_path_contents(&path)?;
            }
        }
        "lyricsCache" => {
            let db = state.db.lock().map_err(|error| error.to_string())?;
            db.execute("DELETE FROM lyrics_cache", [])
                .map_err(|error| error.to_string())?;
        }
        "logs" => {
            for path in log_paths(&app)? {
                clear_path_contents(&path)?;
            }
            for path in app_log_files(&app)? {
                remove_file_if_exists(&path)?;
            }
        }
        _ => return Err("Unknown storage bucket.".to_string()),
    }

    build_storage_report(&app, &state)
}

#[tauri::command]
fn export_storage_diagnostics(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let report = build_storage_report(&app, &state)?;
    Ok(format!(
        "Ome Music Storage Diagnostics\nGenerated: {}\n\nApp cache: {}\nWebView cache: {}\nCover cache: {}\nLyrics cache: {}\nLogs: {}\nDatabase: {}\nTotal cache: {}\n\nNotes:\n- Local music files are referenced by path and are not copied into the app cache.\n- NetEase tracks are streamed by default.\n- SQLite is reserved for metadata, preferences, playback events, and small text caches.\n",
        report.generated_at,
        report.app_cache.display_size,
        report.webview_cache.display_size,
        report.cover_cache.display_size,
        report.lyrics_cache.display_size,
        report.logs.display_size,
        report.database.display_size,
        report.total_cache_display_size
    ))
}

#[tauri::command]
fn list_tracks(state: State<'_, AppState>) -> Result<Vec<TrackDto>, String> {
    let db = state.db.lock().map_err(|error| error.to_string())?;
    let mut tracks = load_tracks(&db)?;
    drop(db);
    proxy_bilibili_track_covers(&state, &mut tracks)?;
    proxy_netease_track_covers(&state, &mut tracks)?;
    proxy_qqmusic_track_covers(&state, &mut tracks)?;
    Ok(tracks)
}

#[tauri::command]
fn get_user_profile(state: State<'_, AppState>) -> Result<UserProfileDto, String> {
    let db = state.db.lock().map_err(|error| error.to_string())?;
    refresh_user_profile(&db)
}

#[tauri::command]
fn get_today_mood_entry(state: State<'_, AppState>) -> Result<Option<MoodEntryDto>, String> {
    let db = state.db.lock().map_err(|error| error.to_string())?;
    load_today_mood_entry(&db)
}

#[tauri::command]
fn list_mood_entries(
    state: State<'_, AppState>,
    limit: Option<u32>,
) -> Result<Vec<MoodEntryDto>, String> {
    let db = state.db.lock().map_err(|error| error.to_string())?;
    load_mood_entries(&db, limit.unwrap_or(30))
}

#[tauri::command]
fn save_mood_entry(
    state: State<'_, AppState>,
    payload: SaveMoodEntryPayload,
) -> Result<MoodEntryDto, String> {
    let db = state.db.lock().map_err(|error| error.to_string())?;
    save_mood_entry_to_db(&db, payload)?;
    load_today_mood_entry(&db)?
        .ok_or_else(|| "Could not read today's mood entry after saving.".to_string())
}

#[tauri::command]
async fn import_music_folder(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<ImportResult, String> {
    let folder_path = app
        .dialog()
        .file()
        .set_title("Choose a local music folder")
        .blocking_pick_folder();

    let Some(folder_path) = folder_path else {
        let db = state.db.lock().map_err(|error| error.to_string())?;
        return Ok(ImportResult {
            directory: None,
            imported_count: 0,
            skipped_count: 0,
            tracks: load_tracks(&db)?,
        });
    };

    let folder_path = folder_path
        .as_path()
        .ok_or_else(|| "Could not read the selected folder path.".to_string())?
        .to_path_buf();

    let directory_label = folder_path.to_string_lossy().to_string();

    // Save the user-selected directory to the authorized registry so future
    // scan_music_directory calls can validate against it.
    {
        let db = state.db.lock().map_err(|error| error.to_string())?;
        db.execute(
            "INSERT OR IGNORE INTO authorized_music_directories (directory_path) VALUES (?1)",
            params![&directory_label],
        )
        .map_err(|error| error.to_string())?;
    }

    // Grant the chosen folder to Tauri's asset protocol scope at runtime so
    // convertFileSrc() can actually load files from it. Without this grant only
    // $HOME/Music is playable; user-picked folders (D:\Music, USB, Desktop)
    // would scan successfully but fail at playback. The grant is recursive so
    // nested album folders are covered. The static tauri.conf.json scope stays
    // locked down — we never open $HOME/** or the whole disk.
    if let Err(error) = app
        .asset_protocol_scope()
        .allow_directory(&folder_path, true)
    {
        eprintln!(
            "could not authorize asset scope for {}: {error}",
            folder_path.display()
        );
    }

    // Discovery + tag parsing is CPU/IO heavy. Run it off the async executor so the
    // Tauri command loop stays responsive; only the DB upserts touch the connection.
    let discovered = tauri::async_runtime::spawn_blocking(move || {
        discover_audio_files(&folder_path)
            .into_iter()
            .map(|file_path| (file_path.clone(), parse_track(&file_path)))
            .collect::<Vec<_>>()
    })
    .await
    .map_err(|error| error.to_string())?;

    let mut imported_count = 0;
    let mut skipped_count = 0;
    let db = state.db.lock().map_err(|error| error.to_string())?;

    for (file_path, parsed) in discovered {
        match parsed.and_then(|track| upsert_track(&db, &track).map(|_| track)) {
            Ok(_) => imported_count += 1,
            Err(error) => {
                eprintln!("skipped audio file {}: {error}", file_path.display());
                skipped_count += 1;
            }
        }
    }

    Ok(ImportResult {
        directory: Some(directory_label),
        imported_count,
        skipped_count,
        tracks: load_tracks(&db)?,
    })
}

#[tauri::command]
fn scan_music_directory(
    path: String,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Vec<LocalAudioFile>, String> {
    // Security: reject paths that have not been authorized by the user through
    // the system folder picker (import_music_folder) or that are not under the
    // default Music directory.
    let db = state.db.lock().map_err(|error| error.to_string())?;
    let authorized: Vec<String> = db
        .prepare("SELECT directory_path FROM authorized_music_directories")
        .map_err(|error| error.to_string())?
        .query_map([], |row| row.get(0))
        .map_err(|error| error.to_string())?
        .filter_map(Result::ok)
        .collect();
    drop(db);

    let requested = Path::new(&path);
    let music_dir = app.path().audio_dir().map_err(|error| error.to_string())?;
    // A path is authorized only if it is equal to or nested under an
    // authorized directory, or under the default Music directory. We never
    // authorize ancestors of authorized directories.
    let is_authorized = authorized.iter().any(|dir| requested.starts_with(dir))
        || requested.starts_with(&music_dir);

    if !is_authorized {
        return Err(
            "This folder has not been authorized by the user. / 该目录未经用户授权。".to_string(),
        );
    }

    Ok(discover_audio_files(Path::new(&path))
        .into_iter()
        .filter_map(|path| {
            let extension = path.extension()?.to_string_lossy().to_lowercase();
            Some(LocalAudioFile {
                path: path.to_string_lossy().to_string(),
                extension,
            })
        })
        .collect())
}

#[tauri::command]
fn get_netease_source_config(state: State<'_, AppState>) -> Result<NeteaseSourceConfigDto, String> {
    let db = state.db.lock().map_err(|error| error.to_string())?;
    load_netease_source_config(&db)
}

#[tauri::command]
fn save_netease_source_config(
    state: State<'_, AppState>,
    payload: SaveNeteaseSourceConfigPayload,
) -> Result<NeteaseSourceConfigDto, String> {
    let db = state.db.lock().map_err(|error| error.to_string())?;
    save_netease_source_config_to_db(&db, payload)?;
    load_netease_source_config(&db)
}

#[tauri::command]
fn get_bilibili_source_config(
    state: State<'_, AppState>,
) -> Result<BilibiliSourceConfigDto, String> {
    let db = state.db.lock().map_err(|error| error.to_string())?;
    load_bilibili_source_config(&db)
}

#[tauri::command]
fn save_bilibili_source_config(
    state: State<'_, AppState>,
    payload: SaveBilibiliSourceConfigPayload,
) -> Result<BilibiliSourceConfigDto, String> {
    let db = state.db.lock().map_err(|error| error.to_string())?;
    save_bilibili_source_config_to_db(&db, payload)?;
    load_bilibili_source_config(&db)
}

#[tauri::command]
async fn test_bilibili_source_connection(
    state: State<'_, AppState>,
    payload: SaveBilibiliSourceConfigPayload,
) -> Result<SourceConnectionDto, String> {
    let config = resolve_bilibili_source_config(&state, Some(payload))?;
    let value = request_bilibili_json(&config, "/x/web-interface/nav", &[]).await?;
    let ok = value
        .get("code")
        .and_then(|value| value.as_i64())
        .unwrap_or(-1)
        == 0;
    Ok(SourceConnectionDto {
        ok,
        message: if ok {
            "Connected. Bilibili is ready.".to_string()
        } else {
            "Bilibili 暂不可用 / Bilibili source is not available".to_string()
        },
    })
}

#[tauri::command]
async fn import_bilibili_cookie(
    state: State<'_, AppState>,
    payload: NeteaseCookiePayload,
) -> Result<BilibiliLoginStatusDto, String> {
    let cookie = payload.cookie.trim();
    if cookie.is_empty() {
        return Err("Cookie is required.".to_string());
    }
    save_bilibili_token(cookie)?;
    let config = resolve_bilibili_source_config(&state, None)?;
    fetch_bilibili_login_status(&config).await
}

#[tauri::command]
async fn create_bilibili_qr_login() -> Result<NeteaseQrLoginDto, String> {
    let response = reqwest::Client::new()
        .get("https://passport.bilibili.com/x/passport-login/web/qrcode/generate")
        .header("User-Agent", BILIBILI_BROWSER_USER_AGENT)
        .header("Referer", "https://www.bilibili.com/")
        .header("Origin", "https://www.bilibili.com")
        .header("Accept", "application/json, text/plain, */*")
        .timeout(std::time::Duration::from_secs(12))
        .send()
        .await
        .map_err(|error| format!("Could not create a Bilibili sign-in code. {error}"))?;
    let value = response
        .json::<serde_json::Value>()
        .await
        .map_err(|_| "Could not read the Bilibili sign-in code.".to_string())?;
    let data = value
        .get("data")
        .ok_or_else(|| "Could not create a Bilibili sign-in code.".to_string())?;
    let key = json_text(data.get("qrcode_key"))
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "Could not create a Bilibili sign-in code.".to_string())?;
    let qr_url = json_text(data.get("url"))
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "Could not create a Bilibili sign-in code.".to_string())?;
    Ok(NeteaseQrLoginDto {
        key,
        qr_url,
        qr_img: String::new(),
    })
}

#[tauri::command]
async fn check_bilibili_qr_login(
    state: State<'_, AppState>,
    payload: NeteaseQrCheckPayload,
) -> Result<BilibiliQrCheckDto, String> {
    let response = reqwest::Client::new()
        .get("https://passport.bilibili.com/x/passport-login/web/qrcode/poll")
        .query(&[("qrcode_key", payload.key.as_str())])
        .header("User-Agent", BILIBILI_BROWSER_USER_AGENT)
        .header("Referer", "https://www.bilibili.com/")
        .header("Origin", "https://www.bilibili.com")
        .header("Accept", "application/json, text/plain, */*")
        .timeout(std::time::Duration::from_secs(12))
        .send()
        .await
        .map_err(|error| format!("Could not check the Bilibili sign-in code. {error}"))?;

    let set_cookie = cookie_header_from_response(&response);
    let value = response
        .json::<serde_json::Value>()
        .await
        .map_err(|_| "Could not read the Bilibili sign-in status.".to_string())?;
    let data = value.get("data").unwrap_or(&value);
    let code = data
        .get("code")
        .and_then(|value| value.as_i64())
        .unwrap_or(-1);
    let status = match code {
        0 => "success",
        86090 => "confirmed",
        86101 => "waiting",
        86038 => "expired",
        _ => "unknown",
    }
    .to_string();
    let message = json_text(data.get("message")).unwrap_or_else(|| match code {
        0 => "Connected to Bilibili.".to_string(),
        86090 => "Scanned. Confirm on your phone.".to_string(),
        86101 => "Waiting for scan.".to_string(),
        86038 => "This sign-in code has expired.".to_string(),
        _ => "Could not confirm this sign-in code.".to_string(),
    });

    let login_status = if code == 0 {
        let cookie = set_cookie.or_else(|| {
            json_text(data.get("url")).and_then(|url| bilibili_cookie_from_login_url(&url))
        });
        let cookie = cookie.ok_or_else(|| "Bilibili confirmed the scan but did not return a session. Please create a new code.".to_string())?;
        save_bilibili_token(&cookie)?;
        let refreshed_config = resolve_bilibili_source_config(&state, None)?;
        Some(fetch_bilibili_login_status(&refreshed_config).await?)
    } else {
        None
    };

    Ok(BilibiliQrCheckDto {
        status,
        code,
        message,
        login_status,
    })
}

#[tauri::command]
async fn get_bilibili_login_status(
    state: State<'_, AppState>,
) -> Result<BilibiliLoginStatusDto, String> {
    let config = resolve_bilibili_source_config(&state, None)?;
    fetch_bilibili_login_status(&config).await
}

#[tauri::command]
fn logout_bilibili(state: State<'_, AppState>) -> Result<BilibiliLoginStatusDto, String> {
    let db = state.db.lock().map_err(|error| error.to_string())?;
    delete_bilibili_token()?;
    Ok(BilibiliLoginStatusDto {
        logged_in: false,
        expired: false,
        nickname: None,
        user_id: None,
        message: if load_bilibili_source_config(&db)?.enabled {
            "Signed out.".to_string()
        } else {
            "Bilibili source is off.".to_string()
        },
    })
}

#[tauri::command]
async fn search_bilibili_songs(
    state: State<'_, AppState>,
    payload: SourceSearchPayload,
) -> Result<Vec<SourceSongDto>, String> {
    let query = payload.query.trim();
    if query.is_empty() {
        return Ok(Vec::new());
    }
    let config = resolve_bilibili_source_config(&state, None)?;
    let mut songs = search_bilibili_videos(&config, query).await?;
    proxy_bilibili_song_covers(&state, &mut songs)?;
    Ok(songs)
}

#[tauri::command]
async fn get_bilibili_song_metadata(
    state: State<'_, AppState>,
    payload: SourceSongPayload,
) -> Result<SourceSongDto, String> {
    let config = resolve_bilibili_source_config(&state, None)?;
    let mut song = fetch_bilibili_video_metadata(&config, &payload.song_id).await?;
    proxy_bilibili_song_cover(&state, &mut song)?;
    Ok(song)
}

#[tauri::command]
async fn get_bilibili_playable_url(
    state: State<'_, AppState>,
    payload: SourceSongPayload,
) -> Result<PlayableUrlDto, String> {
    let config = resolve_bilibili_source_config(&state, None)?;
    let mut playback = fetch_bilibili_playable_url(&config, &payload.song_id).await?;
    proxy_bilibili_playback(&state, &mut playback)?;
    Ok(playback)
}

#[tauri::command]
async fn import_bilibili_song(
    state: State<'_, AppState>,
    payload: SourceSongPayload,
) -> Result<BilibiliImportResultDto, String> {
    let config = resolve_bilibili_source_config(&state, None)?;
    let mut song = fetch_bilibili_video_metadata(&config, &payload.song_id).await?;
    let mut playback = fetch_bilibili_playable_url(&config, &song.id)
        .await
        .unwrap_or_else(|_| bilibili_unavailable_playable(&song.id, "playurl_failed"));
    song.playable_url = playback.url.clone();
    song.unavailable = playback.unavailable;
    song.unavailable_reason = playback.reason.clone();
    let db = state.db.lock().map_err(|error| error.to_string())?;
    let track = parsed_track_from_source_song(&song);
    upsert_track(&db, &track)?;
    let mut tracks = load_tracks(&db)?;
    drop(db);
    proxy_bilibili_track_covers(&state, &mut tracks)?;
    proxy_bilibili_playback(&state, &mut playback)?;
    Ok(BilibiliImportResultDto { tracks, playback })
}

#[tauri::command]
async fn get_bilibili_danmaku(
    state: State<'_, AppState>,
    payload: DanmakuPayload,
) -> Result<DanmakuResponseDto, String> {
    let config = resolve_bilibili_source_config(&state, None)?;
    let bvid = bilibili_song_id_parts(&payload.id).0;
    let cid = payload
        .cid
        .clone()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| bilibili_song_id_parts(&payload.id).1)
        .ok_or_else(|| "Danmaku needs a cid.".to_string())?;
    let cache_key = danmaku_cache_key(&payload.source, &cid);
    if let Some(cached) = load_cached_danmaku(&state, &cache_key)? {
        let debug = DanmakuDebugDto {
            bvid,
            aid: None,
            cid: cid.clone(),
            danmaku_request_url: format!("cache:{cache_key}"),
            raw_danmaku_loaded: true,
            raw_danmaku_length: 0,
            parsed_danmaku_count: cached.len(),
            first_danmaku_time: cached.first().map(|item| item.time),
            from_cache: true,
        };
        return Ok(DanmakuResponseDto {
            source: payload.source,
            id: payload.id,
            cid,
            cache_key,
            items: cached,
            debug,
        });
    }
    let fetched = fetch_bilibili_danmaku_items(&config, &cid).await?;
    let items = fetched.items;
    save_cached_danmaku(&state, &cache_key, &payload.source, &cid, &items)?;
    prune_danmaku_cache(&state)?;
    let debug = DanmakuDebugDto {
        bvid,
        aid: None,
        cid: cid.clone(),
        danmaku_request_url: fetched.request_url,
        raw_danmaku_loaded: true,
        raw_danmaku_length: fetched.raw_length,
        parsed_danmaku_count: items.len(),
        first_danmaku_time: items.first().map(|item| item.time),
        from_cache: false,
    };
    Ok(DanmakuResponseDto {
        source: payload.source,
        id: payload.id,
        cid,
        cache_key,
        items,
        debug,
    })
}

#[tauri::command]
fn clear_danmaku_cache(state: State<'_, AppState>) -> Result<(), String> {
    let db = state.db.lock().map_err(|error| error.to_string())?;
    db.execute("DELETE FROM danmaku_cache", [])
        .map_err(|error| error.to_string())?;
    Ok(())
}

// ── QQ Music Tauri Commands ───────────────────────────────────────────

#[tauri::command]
fn get_qqmusic_source_config(
    state: State<'_, AppState>,
) -> Result<qqmusic::QQMusicSourceConfigDto, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    load_qqmusic_source_config(&db)
}

#[tauri::command]
fn save_qqmusic_source_config(
    state: State<'_, AppState>,
    payload: qqmusic::SaveQQMusicSourceConfigPayload,
) -> Result<qqmusic::QQMusicSourceConfigDto, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    save_qqmusic_source_config_to_db(&db, payload)?;
    load_qqmusic_source_config(&db)
}

#[tauri::command]
async fn test_qqmusic_source_connection(
    _state: State<'_, AppState>,
    payload: qqmusic::SaveQQMusicSourceConfigPayload,
) -> Result<SourceConnectionDto, String> {
    // 优先用 payload 中的 base_url 测试连接
    if let Some(base_url) = payload.base_url.as_deref().filter(|v| !v.trim().is_empty()) {
        let client = reqwest::Client::new();
        let resp = client
            .get(base_url)
            .header("User-Agent", QQMUSIC_UA)
            .timeout(std::time::Duration::from_secs(8))
            .send()
            .await
            .map_err(|e| format!("无法连接: {e}"))?;
        return Ok(SourceConnectionDto {
            ok: resp.status().is_success(),
            message: if resp.status().is_success() {
                "Connected. QQ音乐来源已就绪。".to_string()
            } else {
                format!("状态码 {}", resp.status().as_u16())
            },
        });
    }
    match test_qqmusic_connection().await {
        Ok(msg) => Ok(SourceConnectionDto {
            ok: true,
            message: msg,
        }),
        Err(e) => Ok(SourceConnectionDto {
            ok: false,
            message: e,
        }),
    }
}

#[tauri::command]
async fn import_qqmusic_token(
    state: State<'_, AppState>,
    payload: NeteaseCookiePayload,
) -> Result<qqmusic::QQMusicLoginStatusDto, String> {
    let cookie = payload.cookie.trim();
    if cookie.is_empty() {
        return Err("QQ音乐 Cookie 为空 / QQ Music cookie is empty.".to_string());
    }
    eprintln!(
        "[QQMusic] import_token: cookie length={}, has qqmusic_key={}, has uin={}",
        cookie.len(),
        cookie.contains("qqmusic_key"),
        cookie.contains("uin=")
    );
    save_qqmusic_token(cookie).map_err(|e| {
        eprintln!("[QQMusic] ❌ import_token save failed: {}", e);
        e
    })?;
    eprintln!("[QQMusic] ✅ import_token saved to keyring, verifying readback...");
    match read_qqmusic_token() {
        Some(saved) => eprintln!(
            "[QQMusic] ✅ import_token readback OK, length={}",
            saved.len()
        ),
        None => eprintln!("[QQMusic] ❌ import_token readback FAILED!"),
    }

    // 登录成功后自动启用来源，避免重开设置面板后丢失登录态
    if let Ok(db) = state.db.lock() {
        if let Err(e) = ensure_qqmusic_source_enabled(&db) {
            eprintln!("[QQMusic] ⚠️ import_token: ensure_enabled failed: {}", e);
        }
    }

    // 立即验证 Cookie 是否有效
    let config = qqmusic::ResolvedQQMusicSourceConfig {
        enabled: true,
        base_url: QQMUSIC_DEFAULT_BASE_URL.to_string(),
        token: Some(cookie.to_string()),
    };
    match qqmusic::verify_qqmusic_session(&config).await {
        Ok((uin, nickname)) => Ok(qqmusic::QQMusicLoginStatusDto {
            logged_in: true,
            uin,
            nickname,
            avatar_url: String::new(),
            vip_type: "none".to_string(),
            message: "QQ音乐已连接 / QQ Music connected.".to_string(),
        }),
        Err(e) => {
            // 验证失败但保留 token，让用户可以重试检测登录
            // 安全网：即使 verify_session 失败，如果 cookie 中仍含有效凭据
            // （有 qqmusic_key 且有 uin），仍认为已登录
            let cookie_str = cookie;
            let has_qqmusic_key = cookie_str.contains("qqmusic_key=")
                || cookie_str.contains("p_skey=")
                || cookie_str.contains("superkey=");
            let has_uin = cookie_str.contains("uin=")
                || cookie_str.contains("pt2gguin=")
                || cookie_str.contains("superuin=");
            eprintln!(
                "[QQMusic] import_token: verify failed, safety net: has_key={}, has_uin={}",
                has_qqmusic_key, has_uin
            );
            if has_qqmusic_key && has_uin {
                let uin_str = qqmusic::resolve_qqmusic_uin(&config);
                eprintln!(
                    "[QQMusic] import_token: safety net PASSED, returning logged_in=true, uin={}",
                    uin_str
                );
                Ok(qqmusic::QQMusicLoginStatusDto {
                    logged_in: true,
                    uin: uin_str,
                    nickname: "QQ音乐用户".to_string(),
                    avatar_url: String::new(),
                    vip_type: "none".to_string(),
                    message:
                        "QQ音乐已连接（API验证跳过） / QQ Music connected (API verify skipped)."
                            .to_string(),
                })
            } else {
                eprintln!("[QQMusic] import_token: safety net FAILED, returning logged_in=false");
                Ok(qqmusic::QQMusicLoginStatusDto {
                    logged_in: false,
                    uin: String::new(),
                    nickname: String::new(),
                    avatar_url: String::new(),
                    vip_type: "none".to_string(),
                    message: format!(
                        "验证失败: {e} / Verification failed. Cookie saved, retry check login."
                    ),
                })
            }
        }
    }
}

#[tauri::command]
async fn create_qqmusic_qr_login() -> Result<qqmusic::QQMusicQrLoginDto, String> {
    qqmusic::create_qqmusic_qr().await
}

#[tauri::command]
async fn check_qqmusic_qr_login(
    state: State<'_, AppState>,
    payload: QQMusicQrCheckPayload,
) -> Result<qqmusic::QQMusicQrCheckDto, String> {
    let result = qqmusic::check_qqmusic_qr(&payload.key, &payload.cookies).await?;
    // 登录成功时保存 cookie 并自动启用来源
    if result.status == "confirmed" {
        if let Some(ref cookie) = result.cookie {
            save_qqmusic_token(cookie)?;
            // 确保来源在 DB 中标记为启用
            if let Ok(db) = state.db.lock() {
                if let Err(e) = ensure_qqmusic_source_enabled(&db) {
                    eprintln!("[QQMusic] ⚠️ qr_login: ensure_enabled failed: {}", e);
                }
            }
        }
    }
    Ok(result)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct QQMusicQrCheckPayload {
    key: String,
    cookies: String,
}

#[tauri::command]
async fn get_qqmusic_login_status(
    _state: State<'_, AppState>,
) -> Result<qqmusic::QQMusicLoginStatusDto, String> {
    let token = read_qqmusic_token();
    eprintln!(
        "[QQMusic] login_status: token exists={}, length={}",
        token.is_some(),
        token.as_deref().map(|s| s.len()).unwrap_or(0)
    );
    if let Some(ref t) = token {
        eprintln!(
            "[QQMusic] login_status: contains qqmusic_key={}, contains uin={}, preview={}",
            t.contains("qqmusic_key="),
            t.contains("uin="),
            &t[..t.len().min(200)]
        );
    }
    let Some(ref cookie) = token else {
        return Ok(qqmusic::QQMusicLoginStatusDto {
            logged_in: false,
            uin: String::new(),
            nickname: String::new(),
            avatar_url: String::new(),
            vip_type: "none".to_string(),
            message: "未登录 / Not signed in.".to_string(),
        });
    };

    let config = qqmusic::ResolvedQQMusicSourceConfig {
        enabled: true,
        base_url: QQMUSIC_DEFAULT_BASE_URL.to_string(),
        token: Some(cookie.clone()),
    };
    match qqmusic::verify_qqmusic_session(&config).await {
        Ok((uin, nickname)) => Ok(qqmusic::QQMusicLoginStatusDto {
            logged_in: true,
            uin,
            nickname,
            avatar_url: String::new(),
            vip_type: "none".to_string(),
            message: "QQ音乐已连接 / QQ Music connected.".to_string(),
        }),
        Err(e) => {
            // 验证失败但保留 token，让用户可以重试
            // 安全网：即使 verify_session 失败，如果 cookie 中仍含有效凭据
            // （uin > 0 且有 qqmusic_key），仍认为已登录
            let cookie_str = cookie.as_str();
            let has_qqmusic_key = cookie_str.contains("qqmusic_key=")
                || cookie_str.contains("p_skey=")
                || cookie_str.contains("superkey=");
            // 检查 cookie 中是否有 uin 相关字段
            let has_uin = cookie_str.contains("uin=")
                || cookie_str.contains("pt2gguin=")
                || cookie_str.contains("superuin=");
            eprintln!("[QQMusic] login_status: verify failed, but checking cookie: has_key={has_qqmusic_key}, has_uin={has_uin}");
            if has_qqmusic_key && has_uin {
                // cookie 中有有效凭据，认为已登录（API 验证失败可能是格式问题）
                let uin_str = qqmusic::resolve_qqmusic_uin(&config);
                Ok(qqmusic::QQMusicLoginStatusDto {
                    logged_in: true,
                    uin: uin_str,
                    nickname: "QQ音乐用户".to_string(),
                    avatar_url: String::new(),
                    vip_type: "none".to_string(),
                    message:
                        "QQ音乐已连接（API验证跳过） / QQ Music connected (API verify skipped)."
                            .to_string(),
                })
            } else {
                Ok(qqmusic::QQMusicLoginStatusDto {
                    logged_in: false,
                    uin: String::new(),
                    nickname: String::new(),
                    avatar_url: String::new(),
                    vip_type: "none".to_string(),
                    message: format!("验证失败: {e} / Verification failed. Token retained."),
                })
            }
        }
    }
}

#[tauri::command]
fn logout_qqmusic() -> Result<qqmusic::QQMusicLoginStatusDto, String> {
    delete_qqmusic_token()?;
    Ok(qqmusic::QQMusicLoginStatusDto {
        logged_in: false,
        uin: String::new(),
        nickname: String::new(),
        avatar_url: String::new(),
        vip_type: "none".to_string(),
        message: "已登出 / Signed out.".to_string(),
    })
}

#[tauri::command]
async fn search_qqmusic_songs(
    state: State<'_, AppState>,
    payload: qqmusic::QQMusicSearchPayload,
) -> Result<Vec<SourceSongDto>, String> {
    let query = payload.query.trim();
    if query.is_empty() {
        return Ok(Vec::new());
    }
    let config = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        resolve_qqmusic_source_config(&db)?
    };
    let page = payload.page.unwrap_or(1);
    let page_size = payload.page_size.unwrap_or(20);
    let mut songs = search_qqmusic(&config, query, page, page_size).await?;
    proxy_qqmusic_search_covers(state.inner(), &mut songs)?;
    Ok(songs)
}

#[tauri::command]
async fn get_qqmusic_song_metadata(
    state: State<'_, AppState>,
    payload: qqmusic::QQMusicSourceSongPayload,
) -> Result<SourceSongDto, String> {
    let config = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        resolve_qqmusic_source_config(&db)?
    };
    let mut song = fetch_qqmusic_song_metadata(&config, &payload.song_id).await?;
    if is_proxyable_remote_url(&song.cover_url) {
        song.cover_url = register_media_proxy(state.inner(), &song.cover_url, "image")?;
    }
    Ok(song)
}

#[tauri::command]
async fn get_qqmusic_playable_url(
    state: State<'_, AppState>,
    payload: qqmusic::QQMusicPlayablePayload,
) -> Result<PlayableUrlDto, String> {
    let config = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        resolve_qqmusic_source_config(&db)?
    };
    let mut playback =
        fetch_qqmusic_playable_url(&config, &payload.song_id, payload.quality.as_deref()).await?;
    proxy_qqmusic_playback(state.inner(), &mut playback)?;
    Ok(playback)
}

#[tauri::command]
async fn import_qqmusic_song(
    state: State<'_, AppState>,
    payload: qqmusic::QQMusicSourceSongPayload,
) -> Result<Vec<TrackDto>, String> {
    let config = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        resolve_qqmusic_source_config(&db)?
    };

    let mut song = fetch_qqmusic_song_metadata(&config, &payload.song_id).await?;
    // 尝试获取播放链接但不阻塞导入
    let _playback = fetch_qqmusic_playable_url(&config, &payload.song_id, Some("standard"))
        .await
        .ok();
    if let Some(ref pb) = _playback {
        if let Some(ref url) = pb.url {
            song.playable_url = Some(url.clone());
        }
        song.unavailable = pb.unavailable;
        song.unavailable_reason = pb.reason.clone();
    }

    let db = state.db.lock().map_err(|e| e.to_string())?;
    let track = parsed_track_from_source_song(&song);
    upsert_track(&db, &track)?;
    let mut tracks = load_tracks(&db)?;
    drop(db);
    proxy_qqmusic_track_covers(state.inner(), &mut tracks)?;
    proxy_bilibili_track_covers(&state, &mut tracks)?;
    proxy_netease_track_covers(&state, &mut tracks)?;
    Ok(tracks)
}

#[tauri::command]
async fn get_qqmusic_lyrics(
    state: State<'_, AppState>,
    payload: qqmusic::QQMusicSourceSongPayload,
) -> Result<SourceLyricsDto, String> {
    let config = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        resolve_qqmusic_source_config(&db)?
    };
    let (lyrics, trans) = fetch_qqmusic_lyrics(&config, &payload.song_id).await?;
    Ok(SourceLyricsDto {
        song_id: payload.song_id.clone(),
        lyrics,
        translated_lyrics: trans,
        source: "qqmusic".to_string(),
        cache_key: format!("qqmusic:{}:lrc", payload.song_id),
    })
}

// ── QQ Music Playlist / VIP / UserProfile Commands ───────────────────

#[tauri::command]
async fn get_qqmusic_user_playlists(
    state: State<'_, AppState>,
) -> Result<Vec<qqmusic::QQMusicUserPlaylistDto>, String> {
    let config = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        resolve_qqmusic_source_config(&db)?
    };
    let mut playlists = fetch_qqmusic_user_playlists(&config).await?;
    // Proxy cover URLs
    for p in playlists.iter_mut() {
        if is_proxyable_remote_url(&p.cover_url) {
            p.cover_url = register_media_proxy(state.inner(), &p.cover_url, "image")?;
        }
    }
    Ok(playlists)
}

#[tauri::command]
async fn get_qqmusic_playlist(
    state: State<'_, AppState>,
    payload: SourcePlaylistPayload,
) -> Result<SourcePlaylistDto, String> {
    let config = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        resolve_qqmusic_source_config(&db)?
    };
    let mut playlist = fetch_qqmusic_playlist(&config, &payload.playlist_id).await?;
    proxy_qqmusic_search_covers(state.inner(), &mut playlist.tracks)?;
    Ok(playlist)
}

#[tauri::command]
async fn import_qqmusic_playlist(
    state: State<'_, AppState>,
    payload: SourcePlaylistPayload,
) -> Result<SourcePlaylistDto, String> {
    let config = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        resolve_qqmusic_source_config(&db)?
    };
    let mut playlist = fetch_qqmusic_playlist(&config, &payload.playlist_id).await?;
    let db = state.db.lock().map_err(|e| e.to_string())?;
    import_source_playlist_to_db(&db, &playlist)?;
    drop(db);
    proxy_qqmusic_search_covers(state.inner(), &mut playlist.tracks)?;
    Ok(playlist)
}

#[tauri::command]
async fn get_qqmusic_liked_songs(
    state: State<'_, AppState>,
    limit: Option<u32>,
) -> Result<Vec<SourceSongDto>, String> {
    let config = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        resolve_qqmusic_source_config(&db)?
    };
    let mut songs = fetch_qqmusic_liked_songs(&config, limit.unwrap_or(100)).await?;
    proxy_qqmusic_search_covers(state.inner(), &mut songs)?;
    Ok(songs)
}

#[tauri::command]
fn qqmusic_debug_dump(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    Ok(qqmusic::debug_dump_qqmusic(&db))
}

#[tauri::command]
async fn get_qqmusic_vip_status(
    _state: State<'_, AppState>,
) -> Result<qqmusic::QQMusicVipStatusDto, String> {
    // VIP 状态查询属于账号信息查询，与 get_qqmusic_login_status 同源：
    // 直接读取 keyring 中的 cookie 即可，不应依赖 source 是否启用。
    // 否则会出现"已登录但 VIP 状态未知"的不一致状态。
    let token = read_qqmusic_token();
    let Some(ref cookie) = token else {
        return Ok(qqmusic::QQMusicVipStatusDto {
            is_member: false,
            level: None,
            message: "未登录 / Not signed in.".to_string(),
            membership_known: false,
        });
    };
    let config = qqmusic::ResolvedQQMusicSourceConfig {
        enabled: true,
        base_url: QQMUSIC_DEFAULT_BASE_URL.to_string(),
        token: Some(cookie.clone()),
    };
    fetch_qqmusic_vip_status(&config).await
}

#[tauri::command]
async fn get_qqmusic_user_profile(
    state: State<'_, AppState>,
) -> Result<qqmusic::QQMusicUserProfileDto, String> {
    // 同 get_qqmusic_vip_status：账号资料查询不依赖 source 启用状态。
    let token = read_qqmusic_token();
    let Some(ref cookie) = token else {
        return Ok(qqmusic::QQMusicUserProfileDto {
            logged_in: false,
            nickname: None,
            user_id: None,
            avatar_url: None,
            vip: None,
        });
    };
    let config = qqmusic::ResolvedQQMusicSourceConfig {
        enabled: true,
        base_url: QQMUSIC_DEFAULT_BASE_URL.to_string(),
        token: Some(cookie.clone()),
    };
    let mut profile = fetch_qqmusic_user_profile(&config).await?;
    // Proxy avatar URL
    if let Some(ref avatar) = profile.avatar_url {
        if is_proxyable_remote_url(avatar) {
            profile.avatar_url = Some(register_media_proxy(state.inner(), avatar, "image")?);
        }
    }
    Ok(profile)
}

#[tauri::command]
async fn ensure_netease_api_service(
    state: State<'_, AppState>,
) -> Result<NeteaseServiceStatusDto, String> {
    let base_url = {
        let db = state.db.lock().map_err(|error| error.to_string())?;
        load_netease_source_config(&db)?.base_url
    };
    ensure_local_netease_api_service(
        &base_url,
        state.managed_netease_api.clone(),
        &state.managed_netease_child,
    )
    .await
}

#[tauri::command]
async fn create_netease_qr_login(state: State<'_, AppState>) -> Result<NeteaseQrLoginDto, String> {
    let config = resolve_netease_source_config(&state, None)?;
    let key_timestamp = current_timestamp_ms().to_string();
    let key_value = request_netease_json(
        &config,
        "/login/qr/key",
        &[("timestamp", key_timestamp.as_str())],
    )
    .await
    .map_err(|error| {
        format!("无法连接网易云 API 服务。 / Could not reach the NetEase API. {error}")
    })?;
    let key = key_value
        .get("data")
        .and_then(|data| data.get("unikey"))
        .and_then(|value| value.as_str())
        .ok_or_else(|| {
            "网易云 API 未返回登录密钥，请稍后重试。 / The NetEase API did not return a login key."
                .to_string()
        })?
        .to_string();
    let create_timestamp = current_timestamp_ms().to_string();
    let create_value = request_netease_json(
        &config,
        "/login/qr/create",
        &[
            ("key", key.as_str()),
            ("qrimg", "true"),
            ("timestamp", create_timestamp.as_str()),
        ],
    )
    .await
    .map_err(|error| format!("无法生成二维码。 / Could not generate the QR code. {error}"))?;
    let data = create_value.get("data").ok_or_else(|| {
        "网易云 API 未返回二维码数据。 / The NetEase API did not return QR data.".to_string()
    })?;

    Ok(NeteaseQrLoginDto {
        key,
        qr_url: data
            .get("qrurl")
            .and_then(|value| value.as_str())
            .unwrap_or("")
            .to_string(),
        qr_img: data
            .get("qrimg")
            .and_then(|value| value.as_str())
            .unwrap_or("")
            .to_string(),
    })
}

#[tauri::command]
async fn check_netease_qr_login(
    state: State<'_, AppState>,
    payload: NeteaseQrCheckPayload,
) -> Result<NeteaseQrCheckDto, String> {
    let config = resolve_netease_source_config(&state, None)?;
    let timestamp = current_timestamp_ms().to_string();
    let response = request_netease_json_response(
        &config,
        "/login/qr/check",
        &[
            ("key", payload.key.as_str()),
            ("timestamp", timestamp.as_str()),
        ],
    )
    .await?;
    let value = response.value;
    let code = value
        .get("code")
        .and_then(|value| value.as_i64())
        .unwrap_or(0);
    let message = json_text(value.get("message")).unwrap_or_default();

    let status = match code {
        800 => "expired",
        801 => "waiting",
        802 => "confirmed",
        803 => "success",
        _ => "unknown",
    }
    .to_string();

    let login_status = if code == 803 {
        let cookie = extract_netease_login_cookie(&value, response.set_cookie.as_deref());
        if let Some(cookie) = cookie {
            let saved_cookie = save_and_verify_netease_session(&cookie)?;
            let mut refreshed_config = resolve_netease_source_config(&state, None)?;
            refreshed_config.token = Some(saved_cookie);
            let status = fetch_netease_login_status(&refreshed_config).await?;
            if !status.logged_in {
                return Err(status.message);
            }
            Some(status)
        } else {
            return Err("扫码登录成功但未能获取会话凭证，请重试。 / Sign-in confirmed but the session cookie was missing.".to_string());
        }
    } else {
        None
    };

    Ok(NeteaseQrCheckDto {
        status,
        code,
        message,
        login_status,
    })
}

#[tauri::command]
async fn import_netease_cookie(
    state: State<'_, AppState>,
    payload: NeteaseCookiePayload,
) -> Result<NeteaseLoginStatusDto, String> {
    let cookie = payload.cookie.trim();
    if cookie.is_empty() {
        return Err("Cookie is required.".to_string());
    }
    let saved_cookie = save_and_verify_netease_session(cookie)?;
    let mut config = resolve_netease_source_config(&state, None)?;
    config.token = Some(saved_cookie);
    fetch_netease_login_status(&config).await
}

#[tauri::command]
async fn login_netease_with_password(
    state: State<'_, AppState>,
    payload: NeteasePasswordLoginPayload,
) -> Result<NeteaseLoginStatusDto, String> {
    let config = resolve_netease_source_config(&state, None)?;
    let account = payload.account.trim().replace(' ', "");
    let password = payload.password.trim();
    if account.is_empty() || password.is_empty() {
        return Err("Account and password are required.".to_string());
    }

    let country_code = payload
        .country_code
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("86");
    let login_type = payload.login_type.as_deref().unwrap_or("");
    let looks_like_phone = account
        .chars()
        .all(|character| character.is_ascii_digit() || character == '+');

    let md5_password = format!("{:x}", md5::compute(password.as_bytes()));

    let response = if login_type == "email" || (!looks_like_phone && account.contains('@')) {
        request_netease_json_response(
            &config,
            "/login",
            &[
                ("email", account.as_str()),
                ("md5_password", md5_password.as_str()),
            ],
        )
        .await?
    } else {
        request_netease_json_response(
            &config,
            "/login/cellphone",
            &[
                ("phone", account.as_str().trim_start_matches('+')),
                ("countrycode", country_code.trim_start_matches('+')),
                ("md5_password", md5_password.as_str()),
            ],
        )
        .await?
    };

    complete_netease_session_from_response(&state, response).await
}

#[tauri::command]
async fn request_netease_sms_code(
    state: State<'_, AppState>,
    payload: NeteaseSmsPayload,
) -> Result<SourceConnectionDto, String> {
    let config = resolve_netease_source_config(&state, None)?;
    let phone = payload.phone.trim().replace(' ', "");
    if phone.is_empty() {
        return Err("Phone number is required.".to_string());
    }
    let country_code = payload
        .country_code
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("86");
    let value = request_netease_json(
        &config,
        "/captcha/sent",
        &[
            ("phone", phone.as_str()),
            ("ctcode", country_code.trim_start_matches('+')),
        ],
    )
    .await?;
    let code = value
        .get("code")
        .and_then(|value| value.as_i64())
        .unwrap_or(0);
    if code == 200 {
        Ok(SourceConnectionDto {
            ok: true,
            message: "Code sent. Please check your phone.".to_string(),
        })
    } else {
        Err(friendly_netease_login_error(&value))
    }
}

#[tauri::command]
async fn login_netease_with_sms_code(
    state: State<'_, AppState>,
    payload: NeteaseSmsLoginPayload,
) -> Result<NeteaseLoginStatusDto, String> {
    let config = resolve_netease_source_config(&state, None)?;
    let phone = payload.phone.trim().replace(' ', "");
    let code = payload.code.trim();
    if phone.is_empty() || code.is_empty() {
        return Err("Phone number and verification code are required.".to_string());
    }
    let country_code = payload
        .country_code
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("86");
    let response = request_netease_json_response(
        &config,
        "/login/cellphone",
        &[
            ("phone", phone.as_str()),
            ("countrycode", country_code.trim_start_matches('+')),
            ("captcha", code),
        ],
    )
    .await?;

    complete_netease_session_from_response(&state, response).await
}

#[tauri::command]
fn open_source_web_login(payload: SourceWebLoginPayload) -> Result<SourceConnectionDto, String> {
    let source = payload.source.trim().to_ascii_lowercase();
    let url = match source.as_str() {
        "netease" => "https://music.163.com/#/login",
        "bilibili" => "https://passport.bilibili.com/login",
        "qqmusic" => "https://y.qq.com/",
        _ => return Err("This music source does not support secure web login yet.".to_string()),
    };
    open_url_with_system(url)?;
    Ok(SourceConnectionDto {
        ok: true,
        message: "Secure login page opened. Complete it there, then import Cookie if needed."
            .to_string(),
    })
}

/// 打开 QQ 音乐 webview 登录窗口
#[cfg(windows)]
async fn get_webview2_all_cookies(app: &tauri::AppHandle) -> Result<String, String> {
    use webview2_com::{
        take_pwstr, GetCookiesCompletedHandler,
        Microsoft::Web::WebView2::Win32::{ICoreWebView2Cookie, ICoreWebView2_2},
    };
    use windows::core::{Interface, HSTRING, PWSTR};

    let window = app
        .get_webview_window("qqmusic-login")
        .ok_or_else(|| "qqmusic-login window not found".to_string())?;

    let (tx, rx) = tokio::sync::oneshot::channel::<Result<String, String>>();
    let tx = std::sync::Mutex::new(Some(tx));

    window.with_webview(move |webview| {
        unsafe {
            let controller = webview.controller();

            // 获取 ICoreWebView2
            let core = match controller.CoreWebView2() {
                Ok(c) => c,
                Err(e) => {
                    if let Some(tx) = tx.lock().unwrap().take() {
                        let _ = tx.send(Err(format!("CoreWebView2 failed: {}", e)));
                    }
                    return;
                }
            };

            // 转换为 ICoreWebView2_2 以访问 CookieManager
            let core2 = match Interface::cast::<ICoreWebView2_2>(&core) {
                Ok(c) => c,
                Err(e) => {
                    if let Some(tx) = tx.lock().unwrap().take() {
                        let _ = tx.send(Err(format!("cast ICoreWebView2_2 failed: {}", e)));
                    }
                    return;
                }
            };

            // 获取 CookieManager
            let manager = match core2.CookieManager() {
                Ok(m) => m,
                Err(e) => {
                    if let Some(tx) = tx.lock().unwrap().take() {
                        let _ = tx.send(Err(format!("CookieManager failed: {}", e)));
                    }
                    return;
                }
            };

            eprintln!("[QQMusic] WebView2 CookieManager obtained, getting cookies for https://y.qq.com/");

            let uri = HSTRING::from("https://y.qq.com/");
            let tx_inner = std::sync::Arc::new(tx);

            let tx_for_callback = tx_inner.clone();
            let result = GetCookiesCompletedHandler::wait_for_async_operation(
                Box::new(move |handler| {
                    manager.GetCookies(&uri, &handler)?;
                    Ok(())
                }),
                Box::new(move |hresult, list| {
                    hresult?;
                    match list {
                        Some(list) => {
                            let mut count: u32 = 0;
                            list.Count(&mut count)?;
                            eprintln!("[QQMusic] WebView2 CookieManager: {} cookies found", count);
                            let mut cookies = vec![];
                            for i in 0..count {
                                let cookie: ICoreWebView2Cookie = list.GetValueAtIndex(i)?;
                                let mut name = PWSTR::null();
                                let mut value = PWSTR::null();
                                let mut domain = PWSTR::null();
                                cookie.Name(&mut name)?;
                                cookie.Value(&mut value)?;
                                cookie.Domain(&mut domain)?;
                                let n = take_pwstr(name);
                                let v = take_pwstr(value);
                                let d = take_pwstr(domain);
                                eprintln!("[QQMusic]   cookie[{}]: {}={} (domain={})", i, n, &v[..v.len().min(30)], d);
                                cookies.push(format!("{}={}", n, v));
                            }
                            let cookie_str = cookies.join("; ");
                            eprintln!("[QQMusic] WebView2 cookie string length: {}", cookie_str.len());
                            if let Some(tx) = tx_for_callback.lock().unwrap().take() {
                                let _ = tx.send(Ok(cookie_str));
                            }
                        }
                        None => {
                            eprintln!("[QQMusic] WebView2 CookieManager returned no cookies");
                            if let Some(tx) = tx_for_callback.lock().unwrap().take() {
                                let _ = tx.send(Err("No cookies returned".to_string()));
                            }
                        }
                    }
                    Ok(())
                }),
            );

            if let Err(e) = result {
                eprintln!("[QQMusic] WebView2 wait_for_async_operation error: {}", e);
                if let Some(tx) = tx_inner.lock().unwrap().take() {
                    let _ = tx.send(Err(format!("wait_for_async_operation failed: {}", e)));
                }
            }
        }
    }).map_err(|e| format!("with_webview failed: {}", e))?;

    rx.await
        .map_err(|e| format!("channel recv failed: {}", e))?
}

/// 用户在窗口中登录后，点击"提取Cookie"按钮获取 cookie
/// 使用 initialization_script 在页面加载前注入 JS，绕过 CSP 限制
#[tauri::command]
async fn open_qqmusic_webview_login(app: tauri::AppHandle) -> Result<(), String> {
    use tauri::WebviewWindowBuilder;

    // 如果窗口已存在，聚焦它
    if let Some(window) = app.get_webview_window("qqmusic-login") {
        window
            .set_focus()
            .map_err(|e| format!("Failed to focus window: {}", e))?;
        return Ok(());
    }

    // initialization_script 在页面 JS 加载前由 WebView2 引擎注入，不受页面 CSP 限制
    // 用 setInterval 每 100ms 持续将 document.cookie 写入 title，防止 SPA 覆盖
    let init_script = r#"(function() {
        if (window.__qqCookieInterval) clearInterval(window.__qqCookieInterval);
        window.__qqCookieInterval = setInterval(function() {
            try {
                var c = document.cookie || '';
                document.title = 'QQMUSIC_COOKIE_START' + c + 'QQMUSIC_COOKIE_END';
            } catch(e) {
                document.title = 'QQMUSIC_COOKIE_ERROR:' + e.message;
            }
        }, 200);
    })()"#;

    let _window = WebviewWindowBuilder::new(
        &app,
        "qqmusic-login",
        tauri::WebviewUrl::External("https://y.qq.com/".parse().unwrap()),
    )
    .title("QQ音乐登录 - 登录后点击下方'提取Cookie'按钮")
    .inner_size(1000.0, 700.0)
    .initialization_script(init_script)
    .build()
    .map_err(|e| format!("Failed to create window: {}", e))?;

    Ok(())
}

/// 从 QQ 音乐 webview 窗口提取 cookie
/// 综合方案：eval() 读取 document.cookie + localStorage + fetch API 验证
#[tauri::command]
async fn extract_qqmusic_webview_cookie(app: tauri::AppHandle) -> Result<String, String> {
    // 方案0: 尝试使用 WebView2 CookieManager API 获取所有 cookies（包括 HttpOnly）
    eprintln!("[QQMusic] 尝试 WebView2 CookieManager API...");
    match get_webview2_all_cookies(&app).await {
        Ok(cookie) => {
            eprintln!("[QQMusic] WebView2 CookieManager 成功! cookie length={}, has qqmusic_key={}, has skey={}",
                cookie.len(), cookie.contains("qqmusic_key"), cookie.contains("skey="));
            if !cookie.is_empty() {
                // 直接保存到 keyring，不依赖前端二次调用 importCookie
                match save_qqmusic_token(&cookie) {
                    Ok(()) => {
                        eprintln!(
                            "[QQMusic] ✅ Cookie已直接保存到keyring (WebView2), length={}",
                            cookie.len()
                        );
                        // 验证保存是否成功
                        match read_qqmusic_token() {
                            Some(saved) => eprintln!(
                                "[QQMusic] ✅ keyring读回验证成功, length={}",
                                saved.len()
                            ),
                            None => eprintln!("[QQMusic] ❌ keyring读回失败! token未保存!"),
                        }
                        // 自动启用来源
                        if let Some(state) = app.try_state::<AppState>() {
                            if let Ok(db) = state.db.lock() {
                                if let Err(e) = ensure_qqmusic_source_enabled(&db) {
                                    eprintln!(
                                        "[QQMusic] ⚠️ webview_extract: ensure_enabled failed: {}",
                                        e
                                    );
                                }
                            }
                        }
                    }
                    Err(e) => eprintln!("[QQMusic] ❌ save_qqmusic_token失败: {}", e),
                }
                return Ok(cookie);
            }
        }
        Err(e) => {
            eprintln!(
                "[QQMusic] WebView2 CookieManager 失败: {}，回退到 eval+TCP beacon 方案",
                e
            );
        }
    }

    let window = app
        .get_webview_window("qqmusic-login")
        .ok_or("QQ音乐登录窗口未打开，请先点击'打开QQ音乐登录页'按钮")?;

    let current_url = window.url().map(|u| u.to_string()).unwrap_or_default();
    eprintln!("[QQMusic] webview extract: current URL = {}", current_url);

    // ── 综合提取：document.cookie + localStorage + fetch API 验证 ──
    // 用 eval() 执行 JS，结果写入 document.title，然后通过 window.title() 读取
    // JS 同时启动 fetch() 异步验证，第二次读取 title 时获取 fetch 结果
    let js_extract = r#"try {
        var cookie = document.cookie || '';
        var lsData = {};
        try {
            for (var i = 0; i < localStorage.length; i++) {
                var key = localStorage.key(i);
                if (key) {
                    var lk = key.toLowerCase();
                    if (lk.indexOf('qqmusic') >= 0 || lk.indexOf('qm_keyst') >= 0 || lk.indexOf('music_key') >= 0 || lk.indexOf('uin') >= 0) {
                        lsData[key] = localStorage.getItem(key);
                    }
                }
            }
        } catch(e) {}
        var info = JSON.stringify({cookie: cookie, localStorage: lsData, url: window.location.href, hasQQMusicKey: cookie.indexOf('qqmusic_key') >= 0});
        document.title = 'QQMUSIC_EXTRACT_START' + info + 'QQMUSIC_EXTRACT_END';
    } catch(e) {
        document.title = 'QQMUSIC_COOKIE_ERROR:' + e.message;
    }"#;

    for attempt in 1..=5u32 {
        if let Err(e) = window.eval(js_extract) {
            eprintln!("[QQMusic] eval attempt {} failed: {}", attempt, e);
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

        let title = window
            .title()
            .map_err(|e| format!("Failed to get title: {}", e))?;
        eprintln!(
            "[QQMusic] eval attempt {}: title length={}, preview={}",
            attempt,
            title.len(),
            &title[..title.len().min(300)]
        );

        if let Some(stripped) = title.strip_prefix("QQMUSIC_COOKIE_ERROR:") {
            return Err(format!("JavaScript 错误: {}", stripped));
        }

        if let Some(start) = title.find("QQMUSIC_EXTRACT_START") {
            if let Some(end) = title.find("QQMUSIC_EXTRACT_END") {
                let json_str = &title[start + "QQMUSIC_EXTRACT_START".len()..end];
                eprintln!("[QQMusic] extract JSON length={}", json_str.len());

                // 解析 JSON
                if let Ok(info) = serde_json::from_str::<serde_json::Value>(json_str) {
                    let cookie = info["cookie"].as_str().unwrap_or("");
                    let has_qqmusic_key = info["hasQQMusicKey"].as_bool().unwrap_or(false);
                    let url = info["url"].as_str().unwrap_or("");
                    let ls = &info["localStorage"];

                    eprintln!(
                        "[QQMusic] extract: URL={}, cookie length={}, has qqmusic_key={}",
                        url,
                        cookie.len(),
                        has_qqmusic_key
                    );
                    eprintln!(
                        "[QQMusic] extract: cookie (前300)={}",
                        &cookie[..cookie.len().min(300)]
                    );
                    eprintln!(
                        "[QQMusic] extract: localStorage keys={}",
                        ls.as_object()
                            .map(|o| o.keys().map(|k| k.as_str()).collect::<Vec<_>>().join(", "))
                            .unwrap_or_default()
                    );

                    if !cookie.is_empty() && has_qqmusic_key {
                        eprintln!("[QQMusic] extract: SUCCESS - cookie contains qqmusic_key");
                        match save_qqmusic_token(cookie) {
                            Ok(()) => eprintln!(
                                "[QQMusic] ✅ Cookie已保存到keyring (eval+title), length={}",
                                cookie.len()
                            ),
                            Err(e) => eprintln!("[QQMusic] ❌ save_qqmusic_token失败: {}", e),
                        }
                        return Ok(cookie.to_string());
                    }
                    if !cookie.is_empty() && attempt >= 3 {
                        // 即使没有 qqmusic_key，也返回 cookie（可能 qqmusic_key 在其他位置）
                        eprintln!(
                            "[QQMusic] extract: returning cookie without qqmusic_key (attempt {})",
                            attempt
                        );
                        match save_qqmusic_token(cookie) {
                            Ok(()) => eprintln!(
                                "[QQMusic] ✅ Cookie已保存到keyring (eval fallback), length={}",
                                cookie.len()
                            ),
                            Err(e) => eprintln!("[QQMusic] ❌ save_qqmusic_token失败: {}", e),
                        }
                        return Ok(cookie.to_string());
                    }
                } else {
                    eprintln!(
                        "[QQMusic] extract: JSON parse failed, raw (前300)={}",
                        &json_str[..json_str.len().min(300)]
                    );
                }
            }
        }

        // 也检查旧格式
        if let Some(start) = title.find("QQMUSIC_COOKIE_START") {
            if let Some(end) = title.find("QQMUSIC_COOKIE_END") {
                let cookie = &title[start + "QQMUSIC_COOKIE_START".len()..end];
                eprintln!(
                    "[QQMusic] old-format cookie: length={}, has qqmusic_key={}",
                    cookie.len(),
                    cookie.contains("qqmusic_key")
                );
                if !cookie.is_empty() && cookie.contains("qqmusic_key") {
                    match save_qqmusic_token(cookie) {
                        Ok(()) => eprintln!(
                            "[QQMusic] ✅ Cookie已保存到keyring (old format), length={}",
                            cookie.len()
                        ),
                        Err(e) => eprintln!("[QQMusic] ❌ save_qqmusic_token失败: {}", e),
                    }
                    return Ok(cookie.to_string());
                }
            }
        }
    }

    eprintln!("[QQMusic] method1 (eval+title) failed, trying method2 (TCP beacon)");

    // ── 方案2: 本地 TCP 服务器 + Image beacon ──
    let listener = std::net::TcpListener::bind("127.0.0.1:0")
        .map_err(|e| format!("Failed to bind TCP listener: {}", e))?;
    let port = listener
        .local_addr()
        .map_err(|e| format!("Failed to get local addr: {}", e))?
        .port();
    eprintln!(
        "[QQMusic] TCP beacon server listening on 127.0.0.1:{}",
        port
    );

    listener
        .set_nonblocking(true)
        .map_err(|e| format!("Failed to set non-blocking: {}", e))?;

    // 用 eval() 执行 JS：通过 Image beacon 发送 document.cookie
    // Image 加载不受 CORS 限制，可以跨域发送请求
    // 用 eval() 执行 JS：通过 Image beacon 发送 document.cookie
    // 使用 localhost 而非 127.0.0.1：Chromium 将 http://localhost 视为可信源，豁免混合内容阻止
    // Image 加载不受 CORS 限制，可以跨域发送请求
    let js_beacon = format!(
        r#"try {{
            var c = document.cookie || '';
            var img = new Image();
            img.src = 'http://localhost:{}/?cookie=' + encodeURIComponent(c);
        }} catch(e) {{
            var img2 = new Image();
            img2.src = 'http://localhost:{}/?error=' + encodeURIComponent(e.message);
        }}"#,
        port, port
    );

    window
        .eval(&js_beacon)
        .map_err(|e| format!("eval beacon failed: {}", e))?;

    // 等待 TCP 连接（最多 5 秒）
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if tokio::time::Instant::now() >= deadline {
            eprintln!("[QQMusic] method2 timed out waiting for beacon");
            break;
        }

        match listener.accept() {
            Ok((mut stream, addr)) => {
                eprintln!("[QQMusic] TCP beacon received from {}", addr);
                use std::io::Read;
                let mut buf = [0u8; 8192];
                stream.set_nonblocking(false).ok();
                let n = stream.read(&mut buf).unwrap_or(0);
                let request = String::from_utf8_lossy(&buf[..n]);
                eprintln!(
                    "[QQMusic] TCP request: {}",
                    &request[..request.len().min(500)]
                );

                if let Some(pos) = request.find("/?cookie=") {
                    let rest = &request[pos + "/?cookie=".len()..];
                    let encoded: String = rest
                        .chars()
                        .take_while(|&c| c != ' ' && c != '\n' && c != '\r')
                        .collect();
                    let cookie = urlencoding_decode(&encoded);
                    eprintln!(
                        "[QQMusic] method2 cookie: length={}, has qqmusic_key={}",
                        cookie.len(),
                        cookie.contains("qqmusic_key")
                    );
                    if !cookie.is_empty() {
                        let response = "HTTP/1.1 200 OK

Content-Length: 2



OK";
                        use std::io::Write;
                        stream.write_all(response.as_bytes()).ok();
                        // 直接保存到 keyring
                        match save_qqmusic_token(&cookie) {
                            Ok(()) => eprintln!(
                                "[QQMusic] ✅ Cookie已保存到keyring (TCP beacon), length={}",
                                cookie.len()
                            ),
                            Err(e) => eprintln!("[QQMusic] ❌ save_qqmusic_token失败: {}", e),
                        }
                        return Ok(cookie);
                    }
                }
                if let Some(pos) = request.find("/?error=") {
                    let rest = &request[pos + "/?error=".len()..];
                    let encoded: String = rest
                        .chars()
                        .take_while(|&c| c != ' ' && c != '\n' && c != '\r')
                        .collect();
                    let err_msg = urlencoding_decode(&encoded);
                    eprintln!("[QQMusic] beacon JS error: {}", err_msg);
                }
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
            Err(e) => {
                eprintln!("[QQMusic] TCP accept error: {}", e);
                break;
            }
        }
    }

    Err("未能从窗口提取 Cookie。可能原因：1) 页面尚未加载完成；2) Cookie 为 HttpOnly 类型，JS 无法读取；3) 页面 CSP 阻止了请求。请确保已在窗口中登录QQ音乐后再点提取".to_string())
}

/// URL 解码辅助函数
fn urlencoding_decode(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '%' {
            let h = chars.next();
            let l = chars.next();
            if let (Some(h), Some(l)) = (h, l) {
                if let (Some(hv), Some(lv)) = (h.to_digit(16), l.to_digit(16)) {
                    result.push((hv * 16 + lv) as u8 as char);
                    continue;
                }
            }
            result.push(c);
        } else if c == '+' {
            result.push(' ');
        } else {
            result.push(c);
        }
    }
    result
}

/// 关闭 QQ 音乐 webview 登录窗口
#[tauri::command]
async fn close_qqmusic_webview_login(app: tauri::AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("qqmusic-login") {
        window
            .close()
            .map_err(|e| format!("Failed to close window: {}", e))?;
    }
    Ok(())
}

#[tauri::command]
fn open_external_url(payload: ExternalUrlPayload) -> Result<(), String> {
    let url = payload.url.trim();
    if !url.starts_with("https://") {
        return Err("Only HTTPS links can be opened from Ome Music.".to_string());
    }
    open_url_with_system(url)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalUrlPayload {
    url: String,
}

#[tauri::command]
async fn get_netease_login_status(
    state: State<'_, AppState>,
) -> Result<NeteaseLoginStatusDto, String> {
    let config = resolve_netease_source_config(&state, None)?;
    fetch_netease_login_status(&config).await
}

#[tauri::command]
async fn refresh_netease_login(
    state: State<'_, AppState>,
) -> Result<NeteaseLoginStatusDto, String> {
    let mut config = resolve_netease_source_config(&state, None)?;
    if let Ok(response) = request_netease_json_response(&config, "/login/refresh", &[]).await {
        if let Some(updated_cookie) =
            persist_merged_netease_cookie(config.token.as_deref(), response.set_cookie.as_deref())?
        {
            config.token = Some(updated_cookie);
        }
    }
    fetch_netease_login_status(&config).await
}

#[tauri::command]
fn logout_netease(state: State<'_, AppState>) -> Result<NeteaseLoginStatusDto, String> {
    let db = state.db.lock().map_err(|error| error.to_string())?;
    delete_netease_token()?;
    Ok(NeteaseLoginStatusDto {
        logged_in: false,
        expired: false,
        login_status_known: true,
        has_cookie: false,
        masked_cookie: String::new(),
        nickname: None,
        user_id: None,
        avatar_url: None,
        message: if load_netease_source_config(&db)?.enabled {
            "Signed out.".to_string()
        } else {
            "Music source is not enabled.".to_string()
        },
    })
}

#[tauri::command]
async fn get_netease_vip_status(state: State<'_, AppState>) -> Result<NeteaseVipStatusDto, String> {
    let config = resolve_netease_source_config(&state, None)?;
    fetch_netease_vip_status(&config).await
}

#[tauri::command]
async fn get_netease_user_profile(
    state: State<'_, AppState>,
) -> Result<NeteaseUserProfileDto, String> {
    let config = resolve_netease_source_config(&state, None)?;
    let login = fetch_netease_login_status(&config).await?;
    let vip = if login.logged_in {
        Some(fetch_netease_vip_status(&config).await?)
    } else {
        None
    };

    Ok(NeteaseUserProfileDto {
        logged_in: login.logged_in,
        nickname: login.nickname,
        user_id: login.user_id,
        avatar_url: login.avatar_url,
        vip,
    })
}

#[tauri::command]
async fn test_netease_source_connection(
    state: State<'_, AppState>,
    payload: SaveNeteaseSourceConfigPayload,
) -> Result<SourceConnectionDto, String> {
    let config = resolve_netease_source_config(&state, Some(payload))?;
    let value = request_netease_json(
        &config,
        "/search",
        &[("keywords", "test"), ("limit", "1"), ("type", "1")],
    )
    .await?;
    let count = value
        .get("result")
        .and_then(|result| result.get("songs"))
        .and_then(|songs| songs.as_array())
        .map(|songs| songs.len())
        .unwrap_or(0);

    Ok(SourceConnectionDto {
        ok: true,
        message: if count > 0 {
            "Connected. The source is ready.".to_string()
        } else {
            "Connected, but no songs were returned.".to_string()
        },
    })
}

#[tauri::command]
async fn search_netease_songs(
    state: State<'_, AppState>,
    payload: SourceSearchPayload,
) -> Result<Vec<SourceSongDto>, String> {
    let query = payload.query.trim();
    if query.is_empty() {
        return Ok(Vec::new());
    }

    let config = resolve_netease_source_config(&state, None)?;
    let value = match request_netease_json(
        &config,
        "/cloudsearch",
        &[("keywords", query), ("limit", "20"), ("type", "1")],
    )
    .await
    {
        Ok(value) => value,
        Err(_) => {
            request_netease_json(
                &config,
                "/search",
                &[("keywords", query), ("limit", "20"), ("type", "1")],
            )
            .await?
        }
    };
    let mut songs: Vec<SourceSongDto> = value
        .get("result")
        .and_then(|result| result.get("songs"))
        .and_then(|songs| songs.as_array())
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(source_song_from_search_json)
        .collect();
    // Route every cover through the ome-media:// proxy so the WebView can render
    // it regardless of protocol (http vs https) or CDN cross-origin behaviour.
    proxy_netease_song_covers(&state, &mut songs)?;
    Ok(songs)
}

#[tauri::command]
async fn get_netease_playlist(
    state: State<'_, AppState>,
    payload: SourcePlaylistPayload,
) -> Result<SourcePlaylistDto, String> {
    let config = resolve_netease_source_config(&state, None)?;
    let mut playlist = fetch_netease_playlist(&config, &payload.playlist_id).await?;
    proxy_netease_song_covers(&state, &mut playlist.tracks)?;
    Ok(playlist)
}

#[tauri::command]
async fn get_netease_liked_songs(
    state: State<'_, AppState>,
    limit: Option<u32>,
) -> Result<Vec<SourceSongDto>, String> {
    let config = resolve_netease_source_config(&state, None)?;
    let login = fetch_netease_login_status(&config).await?;
    let user_id = login
        .user_id
        .filter(|value| login.logged_in && !login.expired && !value.trim().is_empty())
        .ok_or_else(|| "Sign in to your music source to try again.".to_string())?;
    let ids = fetch_netease_liked_song_ids(&config, &user_id, limit.unwrap_or(100)).await?;
    let mut songs = fetch_netease_songs_by_ids(&config, &ids).await?;
    proxy_netease_song_covers(&state, &mut songs)?;
    Ok(songs)
}

#[tauri::command]
async fn get_netease_user_playlists(
    state: State<'_, AppState>,
) -> Result<Vec<NeteaseUserPlaylistDto>, String> {
    let config = resolve_netease_source_config(&state, None)?;
    let login = fetch_netease_login_status(&config).await?;
    let user_id = login
        .user_id
        .filter(|value| login.logged_in && !login.expired && !value.trim().is_empty())
        .ok_or_else(|| "Sign in to your music source to try again.".to_string())?;
    fetch_netease_user_playlists(&config, &user_id).await
}

#[tauri::command]
async fn import_netease_playlist(
    state: State<'_, AppState>,
    payload: SourcePlaylistPayload,
) -> Result<SourcePlaylistDto, String> {
    let config = resolve_netease_source_config(&state, None)?;
    let mut playlist = fetch_netease_playlist(&config, &payload.playlist_id).await?;
    let db = state.db.lock().map_err(|error| error.to_string())?;
    import_source_playlist_to_db(&db, &playlist)?;
    drop(db);
    proxy_netease_song_covers(&state, &mut playlist.tracks)?;
    Ok(playlist)
}

#[tauri::command]
async fn import_netease_song(
    state: State<'_, AppState>,
    payload: SourceSongPayload,
) -> Result<Vec<TrackDto>, String> {
    let config = resolve_netease_source_config(&state, None)?;
    let song = fetch_netease_song_metadata(&config, &payload.song_id).await?;
    let db = state.db.lock().map_err(|error| error.to_string())?;
    let track = parsed_track_from_source_song(&song);
    upsert_track(&db, &track)?;
    let mut tracks = load_tracks(&db)?;
    drop(db);
    proxy_netease_track_covers(&state, &mut tracks)?;
    proxy_bilibili_track_covers(&state, &mut tracks)?;
    Ok(tracks)
}

#[tauri::command]
async fn sync_netease_listening_memory(
    state: State<'_, AppState>,
    payload: NeteaseTasteSyncPayload,
) -> Result<NeteaseTasteSyncResultDto, String> {
    let config = resolve_netease_source_config(&state, None)?;
    let login = fetch_netease_login_status(&config).await?;
    let user_id = login
        .user_id
        .filter(|value| login.logged_in && !login.expired && !value.trim().is_empty())
        .ok_or_else(|| "Sign in to your music source to try again.".to_string())?;

    let include_liked = payload.include_liked_songs.unwrap_or(true);
    let include_playlists = payload.include_playlists.unwrap_or(false);
    let liked_limit = payload.liked_limit.unwrap_or(100).clamp(1, 500);
    let mut liked_count = 0_u32;
    let mut playlist_count = 0_u32;
    let mut imported_track_ids = HashSet::new();

    if include_liked {
        let ids = fetch_netease_liked_song_ids(&config, &user_id, liked_limit).await?;
        let songs = fetch_netease_songs_by_ids(&config, &ids).await?;
        liked_count = songs.len() as u32;
        let db = state.db.lock().map_err(|error| error.to_string())?;
        for song in songs {
            let track = parsed_track_from_source_song(&song);
            upsert_track(&db, &track)?;
            mark_track_liked_by_id(&db, &track.id, true)?;
            imported_track_ids.insert(track.id);
        }
    }

    if include_playlists {
        let playlist_ids = match payload.playlist_ids {
            Some(ids) if !ids.is_empty() => ids,
            _ => fetch_netease_user_playlists(&config, &user_id)
                .await?
                .into_iter()
                .map(|playlist| playlist.id)
                .collect::<Vec<_>>(),
        };

        for playlist_id in playlist_ids
            .into_iter()
            .filter(|id| !id.trim().is_empty())
            .take(30)
        {
            let playlist = fetch_netease_playlist(&config, &playlist_id).await?;
            let db = state.db.lock().map_err(|error| error.to_string())?;
            import_source_playlist_to_db(&db, &playlist)?;
            for song in &playlist.tracks {
                imported_track_ids.insert(stable_id(&format!("netease:{}", song.id)));
            }
            playlist_count += 1;
        }
    }

    let db = state.db.lock().map_err(|error| error.to_string())?;
    let taste_notes = save_taste_notes(&db, "netease")?;

    Ok(NeteaseTasteSyncResultDto {
        liked_count,
        playlist_count,
        imported_track_count: imported_track_ids.len() as u32,
        analyzed_track_count: taste_notes.track_count,
        taste_notes,
    })
}

#[tauri::command]
fn get_latest_taste_notes(
    state: State<'_, AppState>,
    source: Option<String>,
) -> Result<Option<TasteNotesDto>, String> {
    let db = state.db.lock().map_err(|error| error.to_string())?;
    load_latest_taste_notes(&db, source.as_deref().unwrap_or("netease"))
}

#[tauri::command]
async fn get_netease_song_metadata(
    state: State<'_, AppState>,
    payload: SourceSongPayload,
) -> Result<SourceSongDto, String> {
    let config = resolve_netease_source_config(&state, None)?;
    let mut song = fetch_netease_song_metadata(&config, &payload.song_id).await?;
    let raw_cover_url = song.cover_url.clone();
    repair_netease_track_cover(&state, &payload.song_id, &raw_cover_url)?;
    proxy_netease_song_cover(&state, &mut song)?;
    Ok(song)
}

fn repair_netease_track_cover(
    state: &State<'_, AppState>,
    song_id: &str,
    cover_url: &str,
) -> Result<(), String> {
    let normalized_cover = cover_url.trim();
    if normalized_cover.is_empty()
        || normalized_cover.starts_with("data:image/svg+xml")
        || normalized_cover.starts_with("ome-media:")
        || normalized_cover.contains("ome-media.localhost")
    {
        return Ok(());
    }

    let db = state.db.lock().map_err(|error| error.to_string())?;
    db.execute(
        "UPDATE tracks
         SET cover_url = ?1, updated_at = CURRENT_TIMESTAMP
         WHERE source = 'netease'
           AND (source_id = ?2 OR file_path = ?3 OR file_path = ?4 OR id = ?5)
           AND (cover_url IS NULL OR cover_url = '' OR cover_url LIKE 'data:image/svg+xml%')",
        params![
            normalized_cover,
            song_id,
            format!("netease:{song_id}"),
            format!("unavailable:netease:{song_id}"),
            stable_id(&format!("netease:{song_id}"))
        ],
    )
    .map_err(|error| error.to_string())?;

    Ok(())
}

#[tauri::command]
async fn get_netease_playable_url(
    state: State<'_, AppState>,
    payload: SourceSongPayload,
) -> Result<PlayableUrlDto, String> {
    let config = resolve_netease_source_config(&state, None)?;
    fetch_netease_playable_url_with_level(
        &config,
        &payload.song_id,
        payload.level.as_deref(),
        Some(state.inner()),
    )
    .await
}

#[tauri::command]
async fn get_netease_lyrics(
    state: State<'_, AppState>,
    payload: SourceSongPayload,
) -> Result<SourceLyricsDto, String> {
    let config = resolve_netease_source_config(&state, None)?;
    let value =
        request_netease_json(&config, "/lyric", &[("id", payload.song_id.as_str())]).await?;
    let lyrics = value
        .get("lrc")
        .and_then(|lrc| lrc.get("lyric"))
        .and_then(|lyric| lyric.as_str())
        .unwrap_or("")
        .to_string();
    let translated_lyrics = value
        .get("tlyric")
        .and_then(|lrc| lrc.get("lyric"))
        .and_then(|lyric| lyric.as_str())
        .unwrap_or("")
        .to_string();

    Ok(SourceLyricsDto {
        cache_key: lyric_cache_key("netease", &payload.song_id, "lrc"),
        song_id: payload.song_id,
        source: "netease".to_string(),
        lyrics,
        translated_lyrics,
    })
}

#[tauri::command]
async fn resolve_track_lyrics(
    state: State<'_, AppState>,
    payload: ResolveLyricsPayload,
) -> Result<ResolvedLyricsDto, String> {
    let track = payload.track;
    let source = if track.source.trim().is_empty() {
        infer_track_source(&track)
    } else {
        track.source.clone()
    };
    let source_id = track
        .source_id
        .clone()
        .or_else(|| infer_track_source_id(&track))
        .unwrap_or_else(|| stable_id(&track.file_path));
    let cache_key = lyric_cache_key(&source, &source_id, "lrc");

    if let Some(cached) = load_cached_lyrics(&state, &cache_key)? {
        return Ok(cached);
    }

    let resolved = if source == "netease" {
        let config = resolve_netease_source_config(&state, None)?;
        let value = request_netease_json(&config, "/lyric", &[("id", source_id.as_str())]).await?;
        let lyrics = value
            .get("lrc")
            .and_then(|lrc| lrc.get("lyric"))
            .and_then(|lyric| lyric.as_str())
            .unwrap_or("")
            .to_string();
        let translated_lyrics = value
            .get("tlyric")
            .and_then(|lrc| lrc.get("lyric"))
            .and_then(|lyric| lyric.as_str())
            .unwrap_or("")
            .to_string();
        let warning = if lyrics.trim().is_empty() {
            Some("No matched lyrics for this version.".to_string())
        } else {
            None
        };
        ResolvedLyricsDto {
            cache_key,
            source,
            lyrics,
            translated_lyrics,
            confidence: if warning.is_some() { 0.0 } else { 1.0 },
            warning,
            offset_ms: 0,
        }
    } else if source == "bilibili" {
        ResolvedLyricsDto {
            cache_key,
            source,
            lyrics: String::new(),
            translated_lyrics: String::new(),
            confidence: 0.0,
            warning: Some("No matched lyrics for this version.".to_string()),
            offset_ms: 0,
        }
    } else if source == "qqmusic" {
        let config = {
            let db = state.db.lock().map_err(|e| e.to_string())?;
            resolve_qqmusic_source_config(&db)?
        };
        match fetch_qqmusic_lyrics(&config, &source_id).await {
            Ok((lyrics, translated_lyrics)) => {
                let warning = if lyrics.trim().is_empty() {
                    Some("No matched lyrics for this version.".to_string())
                } else {
                    None
                };
                ResolvedLyricsDto {
                    cache_key,
                    source,
                    lyrics,
                    translated_lyrics,
                    confidence: if warning.is_some() { 0.0 } else { 0.9 },
                    warning,
                    offset_ms: 0,
                }
            }
            Err(e) => ResolvedLyricsDto {
                cache_key,
                source,
                lyrics: String::new(),
                translated_lyrics: String::new(),
                confidence: 0.0,
                warning: Some(e),
                offset_ms: 0,
            },
        }
    } else {
        resolve_local_track_lyrics(track, source, source_id, cache_key)?
    };

    save_cached_lyrics(&state, &resolved)?;
    with_saved_lyric_offset(&state, resolved)
}

#[tauri::command]
fn save_lyric_offset(
    state: State<'_, AppState>,
    payload: SaveLyricOffsetPayload,
) -> Result<(), String> {
    let offset_key = lyric_offset_key_from_cache_key(&payload.cache_key);
    let db = state.db.lock().map_err(|error| error.to_string())?;
    db.execute(
        "INSERT INTO lyric_offsets (cache_key, offset_ms, updated_at)
         VALUES (?1, ?2, CURRENT_TIMESTAMP)
         ON CONFLICT(cache_key) DO UPDATE SET
           offset_ms = excluded.offset_ms,
           updated_at = CURRENT_TIMESTAMP",
        params![offset_key, payload.offset_ms],
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

#[tauri::command]
fn import_track_lyrics(
    app: AppHandle,
    state: State<'_, AppState>,
    payload: ImportTrackLyricsPayload,
) -> Result<ResolvedLyricsDto, String> {
    let Some(path) = app
        .dialog()
        .file()
        .set_title("Choose an .lrc file")
        .add_filter("LRC lyrics", &["lrc"])
        .blocking_pick_file()
    else {
        return Err("No lyrics file selected.".to_string());
    };

    let lyrics_path = path
        .into_path()
        .map_err(|_| "Could not read that lyrics file.".to_string())?;
    let lyrics = fs::read_to_string(&lyrics_path)
        .map_err(|_| "Could not read that lyrics file.".to_string())?;
    if lyrics.trim().is_empty() {
        return Err("That lyrics file is empty.".to_string());
    }

    let track = payload.track;
    let source = if track.source.trim().is_empty() {
        infer_track_source(&track)
    } else {
        track.source.clone()
    };
    let source_id = track
        .source_id
        .clone()
        .or_else(|| infer_track_source_id(&track))
        .unwrap_or_else(|| stable_id(&track.file_path));
    let cache_key = lyric_cache_key(&source, &source_id, "lrc");

    let resolved = ResolvedLyricsDto {
        cache_key,
        source,
        lyrics,
        translated_lyrics: String::new(),
        confidence: 0.88,
        warning: Some("Lyrics may not match this version.".to_string()),
        offset_ms: 0,
    };
    save_cached_lyrics(&state, &resolved)?;
    with_saved_lyric_offset(&state, resolved)
}

#[tauri::command]
fn record_playback_event(
    state: State<'_, AppState>,
    payload: PlaybackEventPayload,
) -> Result<(), String> {
    let db = state.db.lock().map_err(|error| error.to_string())?;
    insert_playback_event(&db, &payload)?;
    update_track_counters(&db, &payload)?;
    refresh_user_profile(&db).map(|_| ())
}

#[tauri::command]
fn set_track_liked(
    state: State<'_, AppState>,
    track_id: String,
    liked: bool,
    position_seconds: Option<u64>,
) -> Result<Vec<TrackDto>, String> {
    let db = state.db.lock().map_err(|error| error.to_string())?;
    db.execute(
        "UPDATE tracks SET liked = ?1, updated_at = CURRENT_TIMESTAMP WHERE id = ?2",
        params![bool_to_int(liked), &track_id],
    )
    .map_err(|error| error.to_string())?;

    let payload = PlaybackEventPayload {
        track_id,
        event_type: if liked {
            PlaybackEventType::Liked
        } else {
            PlaybackEventType::Unliked
        },
        position_seconds: position_seconds.unwrap_or(0),
    };
    insert_playback_event(&db, &payload)?;
    update_track_counters(&db, &payload)?;
    refresh_user_profile(&db)?;
    load_tracks(&db)
}

#[tauri::command]
fn get_llm_provider_config(state: State<'_, AppState>) -> Result<LlmProviderConfigDto, String> {
    let db = state.db.lock().map_err(|error| error.to_string())?;
    load_llm_provider_config(&db)
}

#[tauri::command]
fn save_llm_provider_config(
    state: State<'_, AppState>,
    payload: SaveLlmProviderConfigPayload,
) -> Result<LlmProviderConfigDto, String> {
    let db = state.db.lock().map_err(|error| error.to_string())?;
    save_llm_provider_config_to_db(&db, payload)?;
    load_llm_provider_config(&db)
}

#[tauri::command]
async fn fetch_llm_models(payload: FetchLlmModelsPayload) -> Result<LlmModelListResponse, String> {
    let base_url = payload.base_url.trim();
    if base_url.is_empty() {
        return Err("Base URL is required before fetching models.".to_string());
    }

    let api_key = payload
        .api_key
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .or_else(read_llm_api_key)
        .ok_or_else(|| "API Key is required before fetching models.".to_string())?;

    let endpoint = openai_compatible_models_endpoint(base_url);
    let response = reqwest::Client::new()
        .get(endpoint)
        .bearer_auth(api_key)
        .send()
        .await
        .map_err(|error| format!("Could not reach the model source. {error}"))?;
    let status = response.status();

    if !status.is_success() {
        return Err(format!(
            "Could not fetch models ({status}). Please check the Base URL and key."
        ));
    }

    let value: serde_json::Value = response.json().await.map_err(|error| error.to_string())?;
    let mut models = value
        .get("data")
        .and_then(|data| data.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.get("id").and_then(|id| id.as_str()))
                .map(str::trim)
                .filter(|id| !id.is_empty())
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    models.sort();
    models.dedup();

    if models.is_empty() {
        return Err("The model source responded, but no models were found.".to_string());
    }

    Ok(LlmModelListResponse { models })
}

#[tauri::command]
async fn generate_llm_text(
    state: State<'_, AppState>,
    payload: LlmTextRequestPayload,
) -> Result<LlmTextResponse, String> {
    let config = {
        let db = state.db.lock().map_err(|error| error.to_string())?;
        load_stored_llm_provider_config(&db)?
            .ok_or_else(|| "Music understanding provider is not configured.".to_string())?
    };
    let api_key = read_llm_api_key()
        .ok_or_else(|| "Music understanding credential is missing.".to_string())?;

    if config.provider_name.trim().is_empty()
        || config.base_url.trim().is_empty()
        || config.model.trim().is_empty()
    {
        return Err("Music understanding provider is incomplete.".to_string());
    }

    let endpoint = openai_compatible_completion_endpoint(&config.base_url);
    let body = serde_json::json!({
        "model": config.model,
        "messages": [
            { "role": "system", "content": payload.system_prompt },
            { "role": "user", "content": payload.user_prompt }
        ],
        "temperature": payload.temperature.unwrap_or(0.65),
        "max_tokens": payload.max_tokens.unwrap_or(480)
    });

    let client = reqwest::Client::new();
    let response = client
        .post(endpoint)
        .bearer_auth(api_key)
        .json(&body)
        .send()
        .await
        .map_err(|error| error.to_string())?;
    let status = response.status();

    if !status.is_success() {
        let details = response.text().await.unwrap_or_default();
        return Err(format!(
            "Music understanding request failed ({status}): {details}"
        ));
    }

    let value: serde_json::Value = response.json().await.map_err(|error| error.to_string())?;
    let text = value
        .get("choices")
        .and_then(|choices| choices.get(0))
        .and_then(|choice| choice.get("message"))
        .and_then(|message| message.get("content"))
        .and_then(|content| content.as_str())
        .map(str::trim)
        .filter(|content| !content.is_empty())
        .ok_or_else(|| "Music understanding provider returned an empty response.".to_string())?
        .to_string();

    {
        let db = state.db.lock().map_err(|error| error.to_string())?;
        if let Err(error) =
            insert_llm_request_audit(&db, &config.provider_name, &payload.purpose, &text)
        {
            eprintln!("llm audit log failed: {error}");
        }
    }

    Ok(LlmTextResponse {
        provider_name: config.provider_name,
        model: config.model,
        text,
    })
}

#[tauri::command]
async fn transcribe_speech_audio(
    state: State<'_, AppState>,
    payload: SpeechTranscriptionPayload,
) -> Result<SpeechTranscriptionResponse, String> {
    let config = {
        let db = state.db.lock().map_err(|error| error.to_string())?;
        load_stored_llm_provider_config(&db)?
            .ok_or_else(|| "Curator source is not configured.".to_string())?
    };
    let api_key =
        read_llm_api_key().ok_or_else(|| "Curator source credential is missing.".to_string())?;
    let model = payload
        .model
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("FunAudioLLM/SenseVoiceSmall");
    let audio_bytes = decode_base64_payload(&payload.audio_base64)?;
    let mime_type = payload.mime_type.trim();
    let file_name = format!("curator-listening.{}", audio_extension_from_mime(mime_type));
    let mut form = reqwest::multipart::Form::new()
        .text("model", model.to_string())
        .text("response_format", "json")
        .part(
            "file",
            reqwest::multipart::Part::bytes(audio_bytes)
                .file_name(file_name)
                .mime_str(if mime_type.is_empty() {
                    "audio/webm"
                } else {
                    mime_type
                })
                .map_err(|error| error.to_string())?,
        );

    if let Some(language) = payload
        .language
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        form = form.text("language", language.to_string());
    }

    let response = reqwest::Client::new()
        .post(openai_compatible_transcription_endpoint(&config.base_url))
        .bearer_auth(api_key)
        .multipart(form)
        .send()
        .await
        .map_err(|error| format!("Could not reach the curator source. {error}"))?;
    let status = response.status();

    if !status.is_success() {
        let details = response.text().await.unwrap_or_default();
        return Err(format!(
            "The curator could not hear that ({status}). {details}"
        ));
    }

    let value: serde_json::Value = response.json().await.map_err(|error| error.to_string())?;
    let text = value
        .get("text")
        .and_then(|text| text.as_str())
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .ok_or_else(|| "The curator could not make out enough words.".to_string())?
        .to_string();

    Ok(SpeechTranscriptionResponse { text })
}

#[tauri::command]
async fn synthesize_curator_speech(
    state: State<'_, AppState>,
    payload: SpeechSynthesisPayload,
) -> Result<SpeechSynthesisResponse, String> {
    let config = {
        let db = state.db.lock().map_err(|error| error.to_string())?;
        load_stored_llm_provider_config(&db)?
            .ok_or_else(|| "Curator source is not configured.".to_string())?
    };
    let api_key =
        read_llm_api_key().ok_or_else(|| "Curator source credential is missing.".to_string())?;
    let input = payload.text.trim();
    if input.is_empty() {
        return Err("There is nothing for the curator to say.".to_string());
    }
    let model = payload
        .model
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("FunAudioLLM/CosyVoice2-0.5B");
    let voice = payload
        .voice
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("FunAudioLLM/CosyVoice2-0.5B:alex");
    let body = serde_json::json!({
        "model": model,
        "voice": voice,
        "input": input,
        "response_format": "mp3"
    });
    let response = reqwest::Client::new()
        .post(openai_compatible_speech_endpoint(&config.base_url))
        .bearer_auth(api_key)
        .json(&body)
        .send()
        .await
        .map_err(|error| format!("Could not reach the curator voice source. {error}"))?;
    let status = response.status();
    let mime_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("audio/mpeg")
        .split(';')
        .next()
        .unwrap_or("audio/mpeg")
        .trim()
        .to_string();

    if !status.is_success() {
        let details = response.text().await.unwrap_or_default();
        return Err(format!(
            "The curator voice is unavailable ({status}). {details}"
        ));
    }

    let bytes = response.bytes().await.map_err(|error| error.to_string())?;
    Ok(SpeechSynthesisResponse {
        audio_data_url: format!(
            "data:{mime_type};base64,{}",
            general_purpose::STANDARD.encode(bytes)
        ),
        mime_type,
    })
}

#[tauri::command]
fn save_playlist_analysis_result(
    state: State<'_, AppState>,
    payload: SavePlaylistAnalysisPayload,
) -> Result<PlaylistAnalysisRunDto, String> {
    let db = state.db.lock().map_err(|error| error.to_string())?;
    insert_playlist_analysis_run(&db, payload)
}

#[tauri::command]
fn get_latest_playlist_analysis(
    state: State<'_, AppState>,
    playlist_id: String,
) -> Result<Option<PlaylistAnalysisRunDto>, String> {
    let db = state.db.lock().map_err(|error| error.to_string())?;
    load_latest_playlist_analysis_run(&db, &playlist_id)
}

fn discover_audio_files(root: &Path) -> Vec<PathBuf> {
    WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .filter_map(|entry| {
            let extension = entry.path().extension()?.to_string_lossy().to_lowercase();
            AUDIO_EXTENSIONS
                .contains(&extension.as_str())
                .then(|| entry.path().to_path_buf())
        })
        .collect()
}

fn parse_track(path: &Path) -> Result<ParsedTrack, String> {
    let tagged_file = Probe::open(path)
        .map_err(|error| error.to_string())?
        .read()
        .map_err(|error| error.to_string())?;

    let tag = tagged_file
        .primary_tag()
        .or_else(|| tagged_file.first_tag());
    let file_stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("Untitled");
    let title = tag
        .and_then(|tag| tag.title())
        .map(|value| value.to_string())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| file_stem.to_string());
    let artist = tag
        .and_then(|tag| tag.artist())
        .map(|value| value.to_string())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "Unknown Artist".to_string());
    let album = tag
        .and_then(|tag| tag.album())
        .map(|value| value.to_string())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "Unknown Album".to_string());
    let genres = tag
        .and_then(|tag| tag.genre())
        .map(|value| split_genres(&value))
        .unwrap_or_default();
    let (moods, calm_score, energetic_score) =
        infer_song_features(&title, &artist, &album, &genres);
    let cover_url = tag.and_then(read_cover_data_url);

    Ok(ParsedTrack {
        id: stable_id(&path.to_string_lossy()),
        title,
        artist,
        album,
        duration_seconds: tagged_file.properties().duration().as_secs(),
        file_path: path.to_string_lossy().to_string(),
        file_extension: path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_lowercase(),
        source: "local".to_string(),
        source_id: Some(stable_id(&path.to_string_lossy())),
        unavailable_reason: None,
        cover_url,
        genres,
        moods,
        calm_score,
        energetic_score,
        year: None,
    })
}

fn split_genres(value: &str) -> Vec<String> {
    value
        .split([';', ',', '/'])
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn read_cover_data_url(tag: &lofty::tag::Tag) -> Option<String> {
    let picture = tag.pictures().first()?;
    let mime = picture
        .mime_type()
        .map(|mime| mime.as_str())
        .unwrap_or("image/jpeg");
    let encoded = general_purpose::STANDARD.encode(picture.data());
    Some(format!("data:{mime};base64,{encoded}"))
}

fn upsert_track(db: &Connection, track: &ParsedTrack) -> Result<(), String> {
    let artist_id = stable_id(&format!("artist:{}", track.artist));
    let album_id = stable_id(&format!("album:{}:{}", track.artist, track.album));
    let genres_json = serde_json::to_string(&track.genres).map_err(|error| error.to_string())?;

    db.execute(
        "INSERT OR IGNORE INTO artists (id, name, genres_json) VALUES (?1, ?2, ?3)",
        params![&artist_id, &track.artist, &genres_json],
    )
    .map_err(|error| error.to_string())?;

    db.execute(
        "INSERT OR IGNORE INTO albums (id, title, artist_id, year, cover_path) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![&album_id, &track.album, &artist_id, track.year, track.cover_url.as_deref()],
    )
    .map_err(|error| error.to_string())?;

    db.execute(
        "INSERT INTO tracks (
            id, title, artist_id, album_id, duration_seconds, file_path, file_extension,
            source, source_id, unavailable_reason, cover_url, genres_json, moods_json, language, year, imported_at, updated_at
          )
          VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, '[]', 'unknown', ?13, CURRENT_TIMESTAMP, CURRENT_TIMESTAMP)
          ON CONFLICT(id) DO UPDATE SET
            title = excluded.title,
            artist_id = excluded.artist_id,
            album_id = excluded.album_id,
            duration_seconds = excluded.duration_seconds,
            file_path = excluded.file_path,
            file_extension = excluded.file_extension,
            source = excluded.source,
            source_id = excluded.source_id,
            unavailable_reason = excluded.unavailable_reason,
            cover_url = COALESCE(excluded.cover_url, tracks.cover_url),
            genres_json = excluded.genres_json,
            year = excluded.year,
            updated_at = CURRENT_TIMESTAMP",
        params![
            &track.id,
            &track.title,
            &artist_id,
            &album_id,
            track.duration_seconds as i64,
            &track.file_path,
            &track.file_extension,
            &track.source,
            track.source_id.as_deref(),
            track.unavailable_reason.as_deref(),
            track.cover_url.as_deref(),
            serde_json::to_string(&track.genres).map_err(|error| error.to_string())?,
            track.year
        ],
    )
    .map_err(|error| error.to_string())?;

    upsert_song_features(db, track)?;

    Ok(())
}

fn mark_track_liked_by_id(db: &Connection, track_id: &str, liked: bool) -> Result<(), String> {
    db.execute(
        "UPDATE tracks SET liked = ?1, updated_at = CURRENT_TIMESTAMP WHERE id = ?2",
        params![bool_to_int(liked), track_id],
    )
    .map_err(|error| error.to_string())?;

    Ok(())
}

fn upsert_song_features(db: &Connection, track: &ParsedTrack) -> Result<(), String> {
    db.execute(
        "INSERT INTO song_features (
            track_id, artist_name, album_title, genres_json, moods_json,
            calm_score, energetic_score, exploration_seed, updated_at
          )
          VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, CURRENT_TIMESTAMP)
          ON CONFLICT(track_id) DO UPDATE SET
            artist_name = excluded.artist_name,
            album_title = excluded.album_title,
            genres_json = excluded.genres_json,
            moods_json = excluded.moods_json,
            calm_score = excluded.calm_score,
            energetic_score = excluded.energetic_score,
            exploration_seed = excluded.exploration_seed,
            updated_at = CURRENT_TIMESTAMP",
        params![
            &track.id,
            &track.artist,
            &track.album,
            serde_json::to_string(&track.genres).map_err(|error| error.to_string())?,
            serde_json::to_string(&track.moods).map_err(|error| error.to_string())?,
            clamp01(track.calm_score),
            clamp01(track.energetic_score),
            0.5_f64
        ],
    )
    .map_err(|error| error.to_string())?;

    Ok(())
}

fn load_tracks(db: &Connection) -> Result<Vec<TrackDto>, String> {
    let mut statement = db
        .prepare(
            "SELECT
              tracks.id,
              tracks.title,
              COALESCE(artists.name, 'Unknown Artist') AS artist,
              COALESCE(albums.title, 'Unknown Album') AS album,
              tracks.duration_seconds,
              tracks.file_path,
              tracks.source,
              tracks.source_id,
              tracks.unavailable_reason,
              tracks.cover_url,
              tracks.genres_json,
              tracks.moods_json,
              tracks.language,
              tracks.year,
              tracks.play_count,
              tracks.skip_count,
              tracks.liked,
              tracks.imported_at
            FROM tracks
            LEFT JOIN artists ON artists.id = tracks.artist_id
            LEFT JOIN albums ON albums.id = tracks.album_id
            ORDER BY tracks.imported_at DESC, tracks.title COLLATE NOCASE ASC",
        )
        .map_err(|error| error.to_string())?;

    let rows = statement
        .query_map([], |row| {
            let id: String = row.get(0)?;
            let genres_json: String = row.get(10)?;
            let moods_json: String = row.get(11)?;
            let cover_url: Option<String> = row.get(9)?;

            Ok(TrackDto {
                id: id.clone(),
                title: row.get(1)?,
                artist: row.get(2)?,
                album: row.get(3)?,
                duration_seconds: row.get::<_, i64>(4)?.max(0) as u64,
                file_path: row.get(5)?,
                source: row.get(6)?,
                source_id: row.get(7)?,
                unavailable_reason: row.get(8)?,
                cover_url: cover_url.unwrap_or_else(|| fallback_cover_url(&id)),
                genres: parse_json_array(&genres_json),
                moods: parse_json_array(&moods_json),
                language: row.get(12)?,
                year: row.get(13)?,
                play_count: row.get::<_, i64>(14)?.max(0) as u32,
                skip_count: row.get::<_, i64>(15)?.max(0) as u32,
                liked: row.get::<_, i64>(16)? == 1,
                imported_at: row.get(17)?,
            })
        })
        .map_err(|error| error.to_string())?;

    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())
}

fn insert_playback_event(db: &Connection, payload: &PlaybackEventPayload) -> Result<(), String> {
    let event_id = stable_id(&format!(
        "event:{}:{}:{}:{}",
        payload.track_id,
        payload.event_type.as_str(),
        payload.position_seconds,
        chrono_like_timestamp()
    ));
    let event_weight = event_weight(&payload.event_type);

    db.execute(
        "INSERT INTO playback_events (id, track_id, event_type, position_seconds, context_json)
         VALUES (?1, ?2, ?3, ?4, '{}')",
        params![
            &event_id,
            &payload.track_id,
            payload.event_type.as_str(),
            payload.position_seconds as i64
        ],
    )
    .map_err(|error| error.to_string())?;

    db.execute(
        "INSERT INTO listening_events (
            id, track_id, event_type, position_seconds, hour_of_day, event_weight
          )
          VALUES (?1, ?2, ?3, ?4, CAST(strftime('%H', 'now', 'localtime') AS INTEGER), ?5)",
        params![
            &event_id,
            &payload.track_id,
            payload.event_type.as_str(),
            payload.position_seconds as i64,
            event_weight
        ],
    )
    .map_err(|error| error.to_string())?;

    Ok(())
}

fn update_track_counters(db: &Connection, payload: &PlaybackEventPayload) -> Result<(), String> {
    let sql = match payload.event_type {
        PlaybackEventType::Play => {
            "UPDATE tracks SET play_count = play_count + 1, updated_at = CURRENT_TIMESTAMP WHERE id = ?1"
        }
        PlaybackEventType::Skip => {
            "UPDATE tracks SET skip_count = skip_count + 1, updated_at = CURRENT_TIMESTAMP WHERE id = ?1"
        }
        PlaybackEventType::Completed => {
            "UPDATE tracks SET completed_count = completed_count + 1, updated_at = CURRENT_TIMESTAMP WHERE id = ?1"
        }
        PlaybackEventType::Replayed => {
            "UPDATE tracks SET replay_count = replay_count + 1, updated_at = CURRENT_TIMESTAMP WHERE id = ?1"
        }
        PlaybackEventType::Pause | PlaybackEventType::Liked | PlaybackEventType::Unliked => {
            "UPDATE tracks SET updated_at = CURRENT_TIMESTAMP WHERE id = ?1"
        }
    };

    db.execute(sql, params![&payload.track_id])
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn initialize_database(app: &tauri::App) -> Result<Connection, String> {
    let app_data_dir = app
        .path()
        .app_data_dir()
        .map_err(|error| error.to_string())?;
    fs::create_dir_all(&app_data_dir).map_err(|error| error.to_string())?;
    let db_path = app_data_dir.join("ome_music.sqlite3");
    let db = Connection::open(db_path).map_err(|error| error.to_string())?;
    // Enforce foreign keys on this connection so legacy ON DELETE CASCADE rules fire.
    db.execute_batch("PRAGMA foreign_keys = ON;")
        .map_err(|error| error.to_string())?;
    db.execute_batch(INITIAL_SCHEMA)
        .map_err(|error| error.to_string())?;
    ensure_mood_entries_note_text(&db)?;
    db.execute_batch(MIGRATION_002)
        .map_err(|error| error.to_string())?;
    db.execute_batch(MIGRATION_003)
        .map_err(|error| error.to_string())?;
    ensure_track_columns(&db)?;
    ensure_profile_tables(&db)?;
    ensure_llm_provider_tables(&db)?;
    ensure_music_source_tables(&db)?;
    ensure_lyrics_tables(&db)?;
    Ok(db)
}

fn ensure_mood_entries_note_text(db: &Connection) -> Result<(), String> {
    // Rename the legacy `note_encrypted` column to `note_text` once. The journal has
    // never actually stored ciphertext; the old name misled readers into assuming
    // encryption that did not exist.
    let has_old: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info('mood_entries') WHERE name = 'note_encrypted')",
            [],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    let has_new: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info('mood_entries') WHERE name = 'note_text')",
            [],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    if has_old && !has_new {
        db.execute(
            "ALTER TABLE mood_entries RENAME COLUMN note_encrypted TO note_text",
            [],
        )
        .map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn build_storage_report(
    app: &AppHandle,
    state: &State<'_, AppState>,
) -> Result<StorageReportDto, String> {
    let app_cache_bytes = sum_paths(&app_cache_paths(app)?);
    let webview_cache_bytes = sum_paths(&webview_cache_paths(app)?);
    let cover_cache_bytes = sum_paths(&cover_cache_paths(app)?);
    let log_bytes = sum_paths(&log_paths(app)?) + sum_paths(&app_log_files(app)?);
    let database_path = app
        .path()
        .app_data_dir()
        .map_err(|error| error.to_string())?
        .join("ome_music.sqlite3");
    let database_bytes = file_size(&database_path);
    let lyrics_cache_bytes = {
        let db = state.db.lock().map_err(|error| error.to_string())?;
        db.query_row(
            "SELECT COALESCE(SUM(LENGTH(lyrics) + LENGTH(translated_lyrics)), 0) FROM lyrics_cache",
            [],
            |row| row.get::<_, i64>(0),
        )
        .unwrap_or(0)
        .max(0) as u64
    };
    let total_cache_bytes =
        app_cache_bytes + webview_cache_bytes + cover_cache_bytes + lyrics_cache_bytes + log_bytes;

    Ok(StorageReportDto {
        app_cache: storage_bucket(
            "应用缓存 / App Cache",
            app_cache_bytes,
            app_cache_root(app)?,
        ),
        webview_cache: storage_bucket(
            "WebView 缓存 / WebView Cache",
            webview_cache_bytes,
            local_app_root(),
        ),
        cover_cache: storage_bucket(
            "封面缓存 / Cover Cache",
            cover_cache_bytes,
            cover_cache_root(app)?,
        ),
        lyrics_cache: storage_bucket(
            "歌词缓存 / Lyrics Cache",
            lyrics_cache_bytes,
            "SQLite: lyrics_cache".to_string(),
        ),
        logs: storage_bucket("日志 / Logs", log_bytes, log_root(app)?),
        database: storage_bucket(
            "数据库 / Database",
            database_bytes,
            database_path.to_string_lossy().to_string(),
        ),
        total_cache_bytes,
        total_cache_display_size: format_bytes(total_cache_bytes),
        generated_at: now_timestamp(),
    })
}

fn storage_bucket(label: &str, bytes: u64, path: String) -> StorageBucketDto {
    StorageBucketDto {
        label: label.to_string(),
        bytes,
        display_size: format_bytes(bytes),
        path,
    }
}

fn app_cache_root(app: &AppHandle) -> Result<String, String> {
    Ok(app
        .path()
        .app_data_dir()
        .map_err(|error| error.to_string())?
        .join("cache")
        .to_string_lossy()
        .to_string())
}

fn cover_cache_root(app: &AppHandle) -> Result<String, String> {
    Ok(app
        .path()
        .app_data_dir()
        .map_err(|error| error.to_string())?
        .join("cover-cache")
        .to_string_lossy()
        .to_string())
}

fn log_root(app: &AppHandle) -> Result<String, String> {
    Ok(app
        .path()
        .app_data_dir()
        .map_err(|error| error.to_string())?
        .join("logs")
        .to_string_lossy()
        .to_string())
}

fn app_cache_paths(app: &AppHandle) -> Result<Vec<PathBuf>, String> {
    let app_data = app
        .path()
        .app_data_dir()
        .map_err(|error| error.to_string())?;
    Ok(["cache", "tmp", "temp"]
        .into_iter()
        .map(|name| app_data.join(name))
        .collect())
}

fn cover_cache_paths(app: &AppHandle) -> Result<Vec<PathBuf>, String> {
    let app_data = app
        .path()
        .app_data_dir()
        .map_err(|error| error.to_string())?;
    Ok(["cover-cache", "covers", "image-cache", "artwork-cache"]
        .into_iter()
        .map(|name| app_data.join(name))
        .collect())
}

fn log_paths(app: &AppHandle) -> Result<Vec<PathBuf>, String> {
    let app_data = app
        .path()
        .app_data_dir()
        .map_err(|error| error.to_string())?;
    Ok(vec![app_data.join("logs")])
}

fn app_log_files(app: &AppHandle) -> Result<Vec<PathBuf>, String> {
    let app_data = app
        .path()
        .app_data_dir()
        .map_err(|error| error.to_string())?;
    let mut files = Vec::new();
    if app_data.exists() {
        for entry in fs::read_dir(&app_data).map_err(|error| error.to_string())? {
            let path = entry.map_err(|error| error.to_string())?.path();
            if path.extension().and_then(|value| value.to_str()) == Some("log") {
                files.push(path);
            }
        }
    }
    Ok(files)
}

fn webview_cache_paths(_app: &AppHandle) -> Result<Vec<PathBuf>, String> {
    let mut paths = Vec::new();
    if let Some(local_root) = env_child_path("LOCALAPPDATA", "com.ome.music") {
        paths.push(local_root.join("EBWebView").join("Default").join("Cache"));
        paths.push(
            local_root
                .join("EBWebView")
                .join("Default")
                .join("Code Cache"),
        );
        paths.push(local_root.join("EBWebView").join("GPUCache"));
        paths.push(local_root.join("EBWebView").join("GrShaderCache"));
        paths.push(local_root.join("EBWebView").join("DawnCache"));
        paths.push(local_root.join("EBWebView").join("DawnGraphiteCache"));
        paths.push(local_root.join("EBWebView").join("DawnWebGPUCache"));
        paths.push(local_root.join("EBWebView").join("component_crx_cache"));
    }
    if let Some(legacy_root) = env_child_path("APPDATA", "ome") {
        paths.push(legacy_root.join("Cache"));
        paths.push(legacy_root.join("GPUCache"));
        paths.push(legacy_root.join("Code Cache"));
        paths.push(legacy_root.join("DawnGraphiteCache"));
        paths.push(legacy_root.join("DawnWebGPUCache"));
    }
    Ok(paths)
}

fn local_app_root() -> String {
    env_child_path("LOCALAPPDATA", "com.ome.music")
        .map(|path| path.to_string_lossy().to_string())
        .unwrap_or_else(|| "Local app data".to_string())
}

fn env_child_path(var_name: &str, child: &str) -> Option<PathBuf> {
    std::env::var_os(var_name)
        .map(PathBuf::from)
        .map(|path| path.join(child))
}

fn sum_paths(paths: &[PathBuf]) -> u64 {
    paths.iter().map(|path| path_size(path)).sum()
}

fn path_size(path: &Path) -> u64 {
    if !path.exists() {
        return 0;
    }
    if path.is_file() {
        return file_size(path);
    }

    WalkDir::new(path)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .filter_map(|entry| entry.metadata().ok().map(|metadata| metadata.len()))
        .sum()
}

fn file_size(path: &Path) -> u64 {
    fs::metadata(path)
        .map(|metadata| metadata.len())
        .unwrap_or(0)
}

fn clear_path_contents(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }
    if path.is_file() {
        return remove_file_if_exists(path);
    }

    for entry in fs::read_dir(path).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let child = entry.path();
        if child.is_dir() {
            fs::remove_dir_all(&child).map_err(|error| error.to_string())?;
        } else {
            remove_file_if_exists(&child)?;
        }
    }
    Ok(())
}

fn remove_file_if_exists(path: &Path) -> Result<(), String> {
    if path.exists() {
        fs::remove_file(path).map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn format_bytes(bytes: u64) -> String {
    let bytes = bytes as f64;
    if bytes >= 1024.0 * 1024.0 * 1024.0 {
        format!("{:.2} GB", bytes / 1024.0 / 1024.0 / 1024.0)
    } else if bytes >= 1024.0 * 1024.0 {
        format!("{:.2} MB", bytes / 1024.0 / 1024.0)
    } else if bytes >= 1024.0 {
        format!("{:.2} KB", bytes / 1024.0)
    } else {
        format!("{:.0} B", bytes)
    }
}

fn ensure_track_columns(db: &Connection) -> Result<(), String> {
    let columns = [
        ("file_extension", "TEXT NOT NULL DEFAULT ''"),
        ("cover_url", "TEXT"),
        ("liked", "INTEGER NOT NULL DEFAULT 0"),
        ("play_count", "INTEGER NOT NULL DEFAULT 0"),
        ("skip_count", "INTEGER NOT NULL DEFAULT 0"),
        ("completed_count", "INTEGER NOT NULL DEFAULT 0"),
        ("replay_count", "INTEGER NOT NULL DEFAULT 0"),
        ("source", "TEXT NOT NULL DEFAULT 'local'"),
        ("source_id", "TEXT"),
        ("unavailable_reason", "TEXT"),
    ];

    for (name, definition) in columns {
        let exists: Option<i64> = db
            .query_row(
                "SELECT 1 FROM pragma_table_info('tracks') WHERE name = ?1",
                params![name],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| error.to_string())?;

        if exists.is_none() {
            db.execute(
                &format!("ALTER TABLE tracks ADD COLUMN {name} {definition}"),
                [],
            )
            .map_err(|error| error.to_string())?;
        }
    }

    Ok(())
}

fn ensure_profile_tables(db: &Connection) -> Result<(), String> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS listening_events (
          id TEXT PRIMARY KEY,
          track_id TEXT NOT NULL,
          event_type TEXT NOT NULL CHECK (event_type IN ('play', 'pause', 'skip', 'completed', 'liked', 'unliked', 'replayed')),
          position_seconds INTEGER NOT NULL DEFAULT 0,
          hour_of_day INTEGER NOT NULL DEFAULT 0,
          event_weight REAL NOT NULL DEFAULT 0,
          created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
          FOREIGN KEY (track_id) REFERENCES tracks(id) ON DELETE CASCADE
        );
        CREATE TABLE IF NOT EXISTS song_features (
          track_id TEXT PRIMARY KEY,
          artist_name TEXT NOT NULL DEFAULT 'Unknown Artist',
          album_title TEXT NOT NULL DEFAULT 'Unknown Album',
          genres_json TEXT NOT NULL DEFAULT '[]',
          moods_json TEXT NOT NULL DEFAULT '[]',
          calm_score REAL NOT NULL DEFAULT 0,
          energetic_score REAL NOT NULL DEFAULT 0,
          exploration_seed REAL NOT NULL DEFAULT 0,
          updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
          FOREIGN KEY (track_id) REFERENCES tracks(id) ON DELETE CASCADE
        );
        CREATE TABLE IF NOT EXISTS user_profile (
          id TEXT PRIMARY KEY CHECK (id = 'local'),
          event_count INTEGER NOT NULL DEFAULT 0,
          confidence REAL NOT NULL DEFAULT 0,
          favorite_artists_json TEXT NOT NULL DEFAULT '[]',
          favorite_albums_json TEXT NOT NULL DEFAULT '[]',
          favorite_genres_json TEXT NOT NULL DEFAULT '[]',
          favorite_moods_json TEXT NOT NULL DEFAULT '[]',
          preferred_listening_hours_json TEXT NOT NULL DEFAULT '[]',
          night_listening_preference_json TEXT NOT NULL DEFAULT '{}',
          skip_patterns_json TEXT NOT NULL DEFAULT '[]',
          repeat_patterns_json TEXT NOT NULL DEFAULT '[]',
          liked_song_patterns_json TEXT NOT NULL DEFAULT '[]',
          exploration_score_json TEXT NOT NULL DEFAULT '{}',
          calm_music_preference_json TEXT NOT NULL DEFAULT '{}',
          energetic_music_preference_json TEXT NOT NULL DEFAULT '{}',
          is_learning INTEGER NOT NULL DEFAULT 1,
          updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
        );
        CREATE INDEX IF NOT EXISTS idx_listening_events_track_id ON listening_events(track_id);
        CREATE INDEX IF NOT EXISTS idx_listening_events_created_at ON listening_events(created_at);
        CREATE INDEX IF NOT EXISTS idx_listening_events_event_type ON listening_events(event_type);",
    )
    .map_err(|error| error.to_string())
}

fn ensure_llm_provider_tables(db: &Connection) -> Result<(), String> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS llm_provider_settings (
          id TEXT PRIMARY KEY CHECK (id = 'local'),
          provider_name TEXT NOT NULL DEFAULT '',
          base_url TEXT NOT NULL DEFAULT '',
          model TEXT NOT NULL DEFAULT '',
          key_ref TEXT NOT NULL DEFAULT 'local',
          created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
          updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
        );
        CREATE TABLE IF NOT EXISTS playlist_analysis_runs (
          id TEXT PRIMARY KEY,
          playlist_id TEXT NOT NULL,
          playlist_name TEXT NOT NULL,
          track_count INTEGER NOT NULL DEFAULT 0,
          mode TEXT NOT NULL CHECK (mode IN ('direct', 'layered')),
          provider TEXT NOT NULL DEFAULT 'local',
          chunk_results_json TEXT NOT NULL DEFAULT '[]',
          final_result_json TEXT NOT NULL DEFAULT '{}',
          report_json TEXT NOT NULL DEFAULT '{}',
          created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
        );
        CREATE INDEX IF NOT EXISTS idx_playlist_analysis_runs_playlist_id ON playlist_analysis_runs(playlist_id);
        CREATE INDEX IF NOT EXISTS idx_playlist_analysis_runs_created_at ON playlist_analysis_runs(created_at);
        CREATE TABLE IF NOT EXISTS taste_notes (
          id TEXT PRIMARY KEY,
          source TEXT NOT NULL,
          track_count INTEGER NOT NULL DEFAULT 0,
          playlist_count INTEGER NOT NULL DEFAULT 0,
          notes_json TEXT NOT NULL DEFAULT '{}',
          created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
          updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
        );
        CREATE INDEX IF NOT EXISTS idx_taste_notes_source ON taste_notes(source);",
    )
    .map_err(|error| error.to_string())
}

fn ensure_music_source_tables(db: &Connection) -> Result<(), String> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS music_source_settings (
          id TEXT PRIMARY KEY,
          enabled INTEGER NOT NULL DEFAULT 0,
          base_url TEXT NOT NULL DEFAULT '',
          token_ref TEXT NOT NULL DEFAULT '',
          metadata_json TEXT NOT NULL DEFAULT '{}',
          created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
          updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
        );
        CREATE TABLE IF NOT EXISTS danmaku_cache (
          cache_key TEXT PRIMARY KEY,
          source TEXT NOT NULL,
          source_id TEXT NOT NULL,
          items_json TEXT NOT NULL DEFAULT '[]',
          bytes INTEGER NOT NULL DEFAULT 0,
          updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
        );
        CREATE INDEX IF NOT EXISTS idx_danmaku_cache_updated_at ON danmaku_cache(updated_at);",
    )
    .map_err(|error| error.to_string())?;
    let _ = db.execute(
        "ALTER TABLE music_source_settings ADD COLUMN metadata_json TEXT NOT NULL DEFAULT '{}'",
        [],
    );
    Ok(())
}

fn ensure_lyrics_tables(db: &Connection) -> Result<(), String> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS lyrics_cache (
          cache_key TEXT PRIMARY KEY,
          source TEXT NOT NULL,
          source_id TEXT NOT NULL,
          lyric_type TEXT NOT NULL DEFAULT 'lrc',
          lyrics TEXT NOT NULL DEFAULT '',
          translated_lyrics TEXT NOT NULL DEFAULT '',
          confidence REAL NOT NULL DEFAULT 1,
          warning TEXT,
          updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
        );
        CREATE TABLE IF NOT EXISTS lyric_offsets (
          cache_key TEXT PRIMARY KEY,
          offset_ms INTEGER NOT NULL DEFAULT 0,
          updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
        );",
    )
    .map_err(|error| error.to_string())
}

fn load_llm_provider_config(db: &Connection) -> Result<LlmProviderConfigDto, String> {
    let stored = load_stored_llm_provider_config(db)?;
    let has_api_key = read_llm_api_key().is_some();
    let masked_api_key = if has_api_key {
        "••••••••••••".to_string()
    } else {
        "".to_string()
    };

    Ok(match stored {
        Some(config) => {
            let configured = !config.provider_name.trim().is_empty()
                && !config.base_url.trim().is_empty()
                && !config.model.trim().is_empty()
                && has_api_key;

            LlmProviderConfigDto {
                provider_name: config.provider_name,
                base_url: config.base_url,
                model: config.model,
                masked_api_key,
                has_api_key,
                configured,
            }
        }
        None => LlmProviderConfigDto {
            provider_name: "".to_string(),
            base_url: "".to_string(),
            model: "".to_string(),
            masked_api_key,
            has_api_key,
            configured: false,
        },
    })
}

fn load_stored_llm_provider_config(
    db: &Connection,
) -> Result<Option<StoredLlmProviderConfig>, String> {
    db.query_row(
        "SELECT provider_name, base_url, model FROM llm_provider_settings WHERE id = 'local'",
        [],
        |row| {
            Ok(StoredLlmProviderConfig {
                provider_name: row.get(0)?,
                base_url: row.get(1)?,
                model: row.get(2)?,
            })
        },
    )
    .optional()
    .map_err(|error| error.to_string())
}

fn save_llm_provider_config_to_db(
    db: &Connection,
    payload: SaveLlmProviderConfigPayload,
) -> Result<(), String> {
    let provider_name = payload.provider_name.trim();
    let base_url = payload.base_url.trim().trim_end_matches('/');
    let model = payload.model.trim();

    if provider_name.is_empty() || base_url.is_empty() || model.is_empty() {
        return Err("Provider Name, Base URL and Model are required.".to_string());
    }

    if let Some(api_key) = payload
        .api_key
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        save_llm_api_key(api_key)?;
    }

    db.execute(
        "INSERT INTO llm_provider_settings (
          id, provider_name, base_url, model, key_ref, created_at, updated_at
        )
        VALUES ('local', ?1, ?2, ?3, 'local', CURRENT_TIMESTAMP, CURRENT_TIMESTAMP)
        ON CONFLICT(id) DO UPDATE SET
          provider_name = excluded.provider_name,
          base_url = excluded.base_url,
          model = excluded.model,
          updated_at = CURRENT_TIMESTAMP",
        params![provider_name, base_url, model],
    )
    .map_err(|error| error.to_string())?;

    Ok(())
}

fn save_llm_api_key(api_key: &str) -> Result<(), String> {
    // Reject plaintext fallback: secrets must live in the OS keyring. If the keyring is
    // unavailable we surface the error so the caller can warn the listener instead of
    // silently writing recoverable base64 to disk.
    keyring::Entry::new(LLM_KEYRING_SERVICE, LLM_KEYRING_ACCOUNT)
        .map_err(|error| error.to_string())?
        .set_password(api_key)
        .map_err(|error| error.to_string())?;

    // Clean up any legacy plaintext fallback from older builds.
    if let Some(path) = llm_api_key_fallback_path() {
        let _ = fs::remove_file(path);
    }
    Ok(())
}

fn read_llm_api_key() -> Option<String> {
    keyring::Entry::new(LLM_KEYRING_SERVICE, LLM_KEYRING_ACCOUNT)
        .ok()
        .and_then(|entry| entry.get_password().ok())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .or_else(read_llm_api_key_fallback)
}

fn read_llm_api_key_fallback() -> Option<String> {
    let path = llm_api_key_fallback_path()?;
    let encoded = fs::read_to_string(path).ok()?;
    let bytes = general_purpose::STANDARD.decode(encoded.trim()).ok()?;
    String::from_utf8(bytes)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn llm_api_key_fallback_path() -> Option<PathBuf> {
    let current_dir = std::env::current_dir().ok()?;
    let app_data = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .map(|path| path.join("com.ome.music"));
    let candidates = [
        app_data.unwrap_or_else(|| current_dir.join("PersonalConfig")),
        current_dir.join("PersonalConfig"),
        current_dir
            .parent()
            .map(|parent| parent.join("PersonalConfig"))
            .unwrap_or_else(|| current_dir.join("PersonalConfig")),
    ];
    candidates
        .into_iter()
        .find(|path| path.exists())
        .or_else(|| Some(current_dir.join("PersonalConfig")))
        .map(|dir| dir.join("curator_source.local"))
}

#[derive(Debug, Clone)]
struct ResolvedNeteaseSourceConfig {
    enabled: bool,
    base_url: String,
    token: Option<String>,
    managed_api: Option<ManagedNeteaseApiRuntime>,
    managed_netease_child: Arc<Mutex<Option<Child>>>,
}

fn load_netease_source_config(db: &Connection) -> Result<NeteaseSourceConfigDto, String> {
    let stored = db
        .query_row(
            "SELECT enabled, base_url FROM music_source_settings WHERE id = 'netease'",
            [],
            |row| Ok((row.get::<_, i64>(0)? == 1, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    let token = read_netease_token();
    let has_token = token.is_some();
    let masked_token = token
        .as_deref()
        .map(mask_netease_cookie)
        .unwrap_or_default();

    Ok(match stored {
        Some((enabled, base_url)) => NeteaseSourceConfigDto {
            enabled,
            base_url,
            has_token,
            masked_token,
        },
        None => NeteaseSourceConfigDto {
            enabled: true,
            base_url: "http://127.0.0.1:3000".to_string(),
            has_token,
            masked_token,
        },
    })
}
fn save_netease_source_config_to_db(
    db: &Connection,
    payload: SaveNeteaseSourceConfigPayload,
) -> Result<(), String> {
    let base_url = payload.base_url.trim().trim_end_matches('/');
    if payload.enabled && base_url.is_empty() {
        return Err("API Base URL is required when NetEase Cloud Music is enabled.".to_string());
    }

    if let Some(token) = payload
        .token
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        save_netease_token(token)?;
    }

    db.execute(
        "INSERT INTO music_source_settings (id, enabled, base_url, token_ref, created_at, updated_at)
         VALUES ('netease', ?1, ?2, 'local', CURRENT_TIMESTAMP, CURRENT_TIMESTAMP)
         ON CONFLICT(id) DO UPDATE SET
           enabled = excluded.enabled,
           base_url = excluded.base_url,
           token_ref = excluded.token_ref,
           updated_at = CURRENT_TIMESTAMP",
        params![bool_to_int(payload.enabled), base_url],
    )
    .map_err(|error| error.to_string())?;

    Ok(())
}

fn resolve_netease_source_config(
    state: &State<'_, AppState>,
    override_payload: Option<SaveNeteaseSourceConfigPayload>,
) -> Result<ResolvedNeteaseSourceConfig, String> {
    let managed_api = state.managed_netease_api.clone();
    let managed_netease_child = state.managed_netease_child.clone();
    if let Some(payload) = override_payload {
        return Ok(ResolvedNeteaseSourceConfig {
            enabled: payload.enabled,
            base_url: payload.base_url.trim().trim_end_matches('/').to_string(),
            token: payload
                .token
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToString::to_string)
                .or_else(read_netease_token),
            managed_api,
            managed_netease_child,
        });
    }

    let db = state.db.lock().map_err(|error| error.to_string())?;
    let config = load_netease_source_config(&db)?;
    if !config.enabled {
        return Err("NetEase Cloud Music is not enabled.".to_string());
    }
    if config.base_url.trim().is_empty() {
        return Err("NetEase Cloud Music API Base URL is missing.".to_string());
    }

    Ok(ResolvedNeteaseSourceConfig {
        enabled: config.enabled,
        base_url: config.base_url,
        token: read_netease_token(),
        managed_api,
        managed_netease_child,
    })
}

fn save_netease_token(token: &str) -> Result<(), String> {
    let normalized = normalize_cookie_header(token);
    if normalized.is_empty() {
        return Err("The NetEase session credential was empty.".to_string());
    }

    let fallback_result = write_netease_token_fallback(&normalized);
    let keyring_result = match keyring::Entry::new(NETEASE_KEYRING_SERVICE, NETEASE_KEYRING_ACCOUNT)
    {
        Ok(entry) => match entry.set_password(&normalized) {
            Ok(()) => Ok(()),
            Err(error) => {
                let _ = entry.delete_credential();
                Err(error.to_string())
            }
        },
        Err(error) => Err(error.to_string()),
    };

    // Mirror the credential to the legacy plaintext fallback file too.
    // We used to delete this file after a successful keyring write, but that
    // created a real-world P0: when the OS keyring service transiently fails
    // to read (Windows Credential Manager restart, Linux Secret Service
    // contention, etc.), `read_netease_token` had no fallback and returned
    // None — which the playback path then reported as `not_logged_in`,
    // surfacing "Sign in needed" even though the user was signed in. Keeping
    // the mirror lets a keyring read failure degrade gracefully instead of
    // being misclassified as "user is not signed in".
    if keyring_result.is_ok() || fallback_result.is_ok() {
        return Ok(());
    }

    Err("Could not store the NetEase session on this device.".to_string())
}

fn save_and_verify_netease_session(token: &str) -> Result<String, String> {
    save_netease_token(token)?;
    let saved = read_netease_token().ok_or_else(|| {
        "Sign-in confirmed, but Ome Music could not restore the session. Please try again."
            .to_string()
    })?;
    if !cookie_has_music_u(&saved) {
        return Err(
            "Sign-in confirmed, but the returned NetEase session was incomplete. Please try again."
                .to_string(),
        );
    }
    Ok(saved)
}

fn write_netease_token_fallback(normalized: &str) -> Result<(), String> {
    let path = netease_token_fallback_path()
        .ok_or_else(|| "Could not locate the local NetEase session store.".to_string())?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let encoded = general_purpose::STANDARD.encode(normalized.as_bytes());
    fs::write(&path, encoded).map_err(|error| error.to_string())
}

fn cookie_has_music_u(cookie: &str) -> bool {
    cookie
        .split(';')
        .any(|part| part.trim().starts_with("MUSIC_U="))
}

fn persist_merged_netease_cookie(
    current_cookie: Option<&str>,
    set_cookie: Option<&str>,
) -> Result<Option<String>, String> {
    let Some(merged_cookie) = merge_cookie_headers(current_cookie, set_cookie) else {
        return Ok(None);
    };
    let had_marker = current_cookie.is_some_and(cookie_has_music_u);
    let keeps_marker = cookie_has_music_u(&merged_cookie);
    if had_marker && !keeps_marker {
        return Ok(current_cookie.map(ToString::to_string));
    }
    if current_cookie.is_some_and(|cookie| normalize_cookie_header(cookie) == merged_cookie) {
        return Ok(Some(merged_cookie));
    }
    save_netease_token(&merged_cookie)?;
    Ok(Some(merged_cookie))
}

fn read_netease_token() -> Option<String> {
    // The OS keyring (Windows Credential Manager / macOS Keychain / Linux
    // Secret Service) can return a transient error while its backing service
    // restarts or under access-contention. A single failed read used to flip
    // `has_token` to false for that one playback call, surfacing as
    // "Sign in needed" even though the user was signed in. Retry a couple of
    // times with a short backoff before concluding the credential is absent.
    let mut last_value: Option<String> = None;
    for attempt in 0..3u32 {
        let entry_result = keyring::Entry::new(NETEASE_KEYRING_SERVICE, NETEASE_KEYRING_ACCOUNT);
        if let Ok(entry) = entry_result {
            match entry.get_password() {
                Ok(value) => {
                    last_value = Some(value);
                    break;
                }
                Err(err) => {
                    // Only retry on NoEntry / back-end errors; an auth error
                    // is a hard negative we should not paper over.
                    let is_no_entry = matches!(err, keyring::Error::NoEntry);
                    if is_no_entry {
                        // Credential genuinely absent — don't retry.
                        return read_netease_token_fallback();
                    }
                    if attempt < 2 {
                        std::thread::sleep(std::time::Duration::from_millis(80));
                        continue;
                    }
                }
            }
        } else if attempt < 2 {
            std::thread::sleep(std::time::Duration::from_millis(80));
            continue;
        }
    }

    last_value
        .map(|value| normalize_cookie_header(&value))
        .filter(|value| !value.is_empty())
        .or_else(read_netease_token_fallback)
}

fn delete_netease_token() -> Result<(), String> {
    if let Ok(entry) = keyring::Entry::new(NETEASE_KEYRING_SERVICE, NETEASE_KEYRING_ACCOUNT) {
        let _ = entry.delete_credential();
    }
    if let Some(path) = netease_token_fallback_path() {
        let _ = fs::remove_file(path);
    }
    Ok(())
}

fn read_netease_token_fallback() -> Option<String> {
    let path = netease_token_fallback_path()?;
    let encoded = fs::read_to_string(path).ok()?;
    let bytes = general_purpose::STANDARD.decode(encoded.trim()).ok()?;
    String::from_utf8(bytes)
        .ok()
        .map(|value| normalize_cookie_header(&value))
        .filter(|value| !value.is_empty())
}

fn netease_token_fallback_path() -> Option<PathBuf> {
    let current_dir = std::env::current_dir().ok()?;
    let candidates = [
        current_dir.join("PersonalConfig"),
        current_dir
            .parent()
            .map(|parent| parent.join("PersonalConfig"))
            .unwrap_or_else(|| current_dir.join("PersonalConfig")),
    ];
    candidates
        .into_iter()
        .find(|path| path.exists())
        .or_else(|| Some(current_dir.join("PersonalConfig")))
        .map(|dir| dir.join("netease_session.local"))
}

struct ResolvedBilibiliSourceConfig {
    base_url: String,
    token: Option<String>,
    search_scope: String,
}

fn load_bilibili_source_config(db: &Connection) -> Result<BilibiliSourceConfigDto, String> {
    let stored = db
        .query_row(
            "SELECT enabled, base_url, metadata_json FROM music_source_settings WHERE id = 'bilibili'",
            [],
            |row| Ok((row.get::<_, i64>(0)? == 1, row.get::<_, String>(1)?, row.get::<_, String>(2)?)),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    let has_token = read_bilibili_token().is_some();
    let masked_token = if has_token {
        "••••••••••••".to_string()
    } else {
        "".to_string()
    };

    Ok(match stored {
        Some((enabled, base_url, metadata_json)) => BilibiliSourceConfigDto {
            enabled,
            base_url: if base_url.trim().is_empty() {
                BILIBILI_DEFAULT_BASE_URL.to_string()
            } else {
                base_url
            },
            has_token,
            masked_token,
            search_scope: bilibili_search_scope_from_metadata(&metadata_json),
        },
        None => BilibiliSourceConfigDto {
            enabled: false,
            base_url: BILIBILI_DEFAULT_BASE_URL.to_string(),
            has_token,
            masked_token,
            search_scope: "music".to_string(),
        },
    })
}

fn save_bilibili_source_config_to_db(
    db: &Connection,
    payload: SaveBilibiliSourceConfigPayload,
) -> Result<(), String> {
    let base_url = payload
        .base_url
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(BILIBILI_DEFAULT_BASE_URL)
        .trim_end_matches('/')
        .to_string();
    if payload.enabled && !base_url.starts_with("https://") && !base_url.starts_with("http://") {
        return Err("API Base URL is not valid.".to_string());
    }

    if let Some(token) = payload
        .token
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        save_bilibili_token(token)?;
    }

    let search_scope = normalize_bilibili_search_scope(payload.search_scope.as_deref());
    let metadata_json = serde_json::json!({ "searchScope": search_scope }).to_string();
    db.execute(
        "INSERT INTO music_source_settings (id, enabled, base_url, token_ref, metadata_json, created_at, updated_at)
         VALUES ('bilibili', ?1, ?2, 'local', ?3, CURRENT_TIMESTAMP, CURRENT_TIMESTAMP)
         ON CONFLICT(id) DO UPDATE SET
           enabled = excluded.enabled,
           base_url = excluded.base_url,
           token_ref = excluded.token_ref,
           metadata_json = excluded.metadata_json,
           updated_at = CURRENT_TIMESTAMP",
        params![bool_to_int(payload.enabled), base_url, metadata_json],
    )
    .map_err(|error| error.to_string())?;

    Ok(())
}

fn resolve_bilibili_source_config(
    state: &State<'_, AppState>,
    override_payload: Option<SaveBilibiliSourceConfigPayload>,
) -> Result<ResolvedBilibiliSourceConfig, String> {
    if let Some(payload) = override_payload {
        return Ok(ResolvedBilibiliSourceConfig {
            base_url: payload
                .base_url
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or(BILIBILI_DEFAULT_BASE_URL)
                .trim_end_matches('/')
                .to_string(),
            token: payload
                .token
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToString::to_string)
                .or_else(read_bilibili_token),
            search_scope: normalize_bilibili_search_scope(payload.search_scope.as_deref()),
        });
    }

    let db = state.db.lock().map_err(|error| error.to_string())?;
    let config = load_bilibili_source_config(&db)?;
    if !config.enabled {
        return Err("Bilibili source is off.".to_string());
    }

    Ok(ResolvedBilibiliSourceConfig {
        base_url: config.base_url,
        token: read_bilibili_token(),
        search_scope: config.search_scope,
    })
}

fn save_bilibili_token(token: &str) -> Result<(), String> {
    // Reject plaintext fallback: Bilibili sessions must live in the OS keyring.
    keyring::Entry::new(BILIBILI_KEYRING_SERVICE, BILIBILI_KEYRING_ACCOUNT)
        .map_err(|error| error.to_string())?
        .set_password(token)
        .map_err(|error| error.to_string())?;

    // Clean up any legacy plaintext fallback from older builds.
    if let Some(path) = bilibili_token_fallback_path() {
        let _ = fs::remove_file(path);
    }
    Ok(())
}

fn read_bilibili_token() -> Option<String> {
    keyring::Entry::new(BILIBILI_KEYRING_SERVICE, BILIBILI_KEYRING_ACCOUNT)
        .ok()
        .and_then(|entry| entry.get_password().ok())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .or_else(read_bilibili_token_fallback)
}

fn delete_bilibili_token() -> Result<(), String> {
    if let Ok(entry) = keyring::Entry::new(BILIBILI_KEYRING_SERVICE, BILIBILI_KEYRING_ACCOUNT) {
        let _ = entry.delete_credential();
    }
    if let Some(path) = bilibili_token_fallback_path() {
        let _ = fs::remove_file(path);
    }
    Ok(())
}

fn read_bilibili_token_fallback() -> Option<String> {
    let path = bilibili_token_fallback_path()?;
    let encoded = fs::read_to_string(path).ok()?;
    let bytes = general_purpose::STANDARD.decode(encoded.trim()).ok()?;
    String::from_utf8(bytes)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn bilibili_token_fallback_path() -> Option<PathBuf> {
    let current_dir = std::env::current_dir().ok()?;
    let candidates = [
        current_dir.join("PersonalConfig"),
        current_dir
            .parent()
            .map(|parent| parent.join("PersonalConfig"))
            .unwrap_or_else(|| current_dir.join("PersonalConfig")),
    ];
    candidates
        .into_iter()
        .find(|path| path.exists())
        .or_else(|| Some(current_dir.join("PersonalConfig")))
        .map(|dir| dir.join("bilibili_session.local"))
}

fn bilibili_search_scope_from_metadata(metadata_json: &str) -> String {
    serde_json::from_str::<serde_json::Value>(metadata_json)
        .ok()
        .and_then(|value| {
            value
                .get("searchScope")
                .and_then(|scope| scope.as_str())
                .map(ToString::to_string)
        })
        .map(|value| normalize_bilibili_search_scope(Some(&value)))
        .unwrap_or_else(|| "music".to_string())
}

fn normalize_bilibili_search_scope(scope: Option<&str>) -> String {
    match scope.unwrap_or("music").trim().to_lowercase().as_str() {
        "vocaloid" => "vocaloid".to_string(),
        "live" => "live".to_string(),
        "cover" => "cover".to_string(),
        "mv" => "mv".to_string(),
        "all" => "all".to_string(),
        _ => "music".to_string(),
    }
}

async fn request_bilibili_json(
    config: &ResolvedBilibiliSourceConfig,
    path: &str,
    query: &[(&str, &str)],
) -> Result<serde_json::Value, String> {
    let text = request_bilibili_text(config, path, query).await?;
    serde_json::from_str::<serde_json::Value>(&text)
        .map_err(|_| "Bilibili 响应解析失败 / Failed to parse Bilibili response".to_string())
}

fn cookie_header_from_response(response: &reqwest::Response) -> Option<String> {
    let cookies = response
        .headers()
        .get_all(reqwest::header::SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .filter_map(|value| value.split(';').next())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    if cookies.is_empty() {
        None
    } else {
        Some(cookies.join("; "))
    }
}

fn normalize_cookie_header(cookie: &str) -> String {
    const COOKIE_ATTRIBUTES: &[&str] = &[
        "path", "domain", "expires", "max-age", "httponly", "secure", "samesite", "priority",
    ];

    let mut seen = HashSet::new();
    cookie
        .split(';')
        .filter_map(|part| {
            let trimmed = part.trim();
            if trimmed.is_empty() {
                return None;
            }
            let (name, value) = trimmed.split_once('=')?;
            let name = name.trim();
            let value = value.trim();
            if name.is_empty() || value.is_empty() {
                return None;
            }
            if COOKIE_ATTRIBUTES
                .iter()
                .any(|attribute| name.eq_ignore_ascii_case(attribute))
            {
                return None;
            }
            let key = name.to_ascii_lowercase();
            if !seen.insert(key) {
                return None;
            }
            Some(format!("{name}={value}"))
        })
        .collect::<Vec<_>>()
        .join("; ")
}

fn merge_cookie_headers(primary: Option<&str>, secondary: Option<&str>) -> Option<String> {
    let mut order: Vec<String> = Vec::new();
    let mut values: HashMap<String, (String, String)> = HashMap::new();

    for cookie in [primary, secondary].into_iter().flatten() {
        let normalized = normalize_cookie_header(cookie);
        for part in normalized.split(';') {
            let trimmed = part.trim();
            if trimmed.is_empty() {
                continue;
            }
            let Some((name, value)) = trimmed.split_once('=') else {
                continue;
            };
            let name = name.trim();
            let value = value.trim();
            if name.is_empty() || value.is_empty() {
                continue;
            }
            let key = name.to_ascii_lowercase();
            if !values.contains_key(&key) {
                order.push(key.clone());
            }
            values.insert(key, (name.to_string(), value.to_string()));
        }
    }

    let merged = order
        .into_iter()
        .filter_map(|key| values.get(&key).cloned())
        .map(|(name, value)| format!("{name}={value}"))
        .collect::<Vec<_>>()
        .join("; ");

    if merged.is_empty() {
        None
    } else {
        Some(merged)
    }
}

fn bilibili_cookie_from_login_url(url: &str) -> Option<String> {
    const COOKIE_NAMES: &[&str] = &[
        "SESSDATA",
        "bili_jct",
        "DedeUserID",
        "DedeUserID__ckMd5",
        "sid",
    ];
    let query = url.split_once('?')?.1;
    let cookies = query
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .filter(|(name, value)| COOKIE_NAMES.contains(name) && !value.is_empty())
        .map(|(name, value)| format!("{name}={value}"))
        .collect::<Vec<_>>();
    if cookies.is_empty() {
        None
    } else {
        Some(cookies.join("; "))
    }
}

async fn request_bilibili_text(
    config: &ResolvedBilibiliSourceConfig,
    path: &str,
    query: &[(&str, &str)],
) -> Result<String, String> {
    let url = if path.starts_with("http://") || path.starts_with("https://") {
        path.to_string()
    } else {
        format!("{}{}", config.base_url.trim_end_matches('/'), path)
    };
    let is_search_request = path.contains("/search/");
    let request_cookie = resolve_bilibili_request_cookie(config).await;
    let client = reqwest::Client::new();
    let mut request = client
        .get(url)
        .query(query)
        .header(
            "User-Agent",
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36",
        )
        .header("Accept", "application/json, text/plain, */*")
        .header("Accept-Language", "zh-CN,zh;q=0.9,en;q=0.8")
        .header(
            "Referer",
            if is_search_request { "https://search.bilibili.com/" } else { "https://www.bilibili.com/" },
        )
        .timeout(std::time::Duration::from_secs(12));

    if is_search_request {
        request = request
            .header("Origin", "https://search.bilibili.com")
            .header("Sec-Fetch-Dest", "empty")
            .header("Sec-Fetch-Mode", "cors")
            .header("Sec-Fetch-Site", "same-site");
    }

    if let Some(cookie) = request_cookie
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        request = request.header("Cookie", cookie);
    }

    let response = request.send().await.map_err(|error| {
        format!("Bilibili 请求失败：{error} / Bilibili request failed: {error}")
    })?;
    let status = response.status();
    if status.as_u16() == 412 {
        return Err(
            "Bilibili paused this search request. Please wait a moment and try again.".to_string(),
        );
    }
    if !status.is_success() {
        return Err(format!("Bilibili is unavailable just now ({status})."));
    }
    let text = response.text().await.map_err(|error| error.to_string())?;
    let trimmed = text.trim_start();
    if trimmed.starts_with("<!DOCTYPE") || trimmed.starts_with("<html") {
        return Err(
            "Bilibili asked for web verification. Please wait a moment and search again."
                .to_string(),
        );
    }
    Ok(text)
}

async fn resolve_bilibili_request_cookie(config: &ResolvedBilibiliSourceConfig) -> Option<String> {
    let existing = config
        .token
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if existing.is_some_and(|cookie| {
        has_cookie_name(cookie, "buvid3") && has_cookie_name(cookie, "buvid4")
    }) {
        return existing.map(ToString::to_string);
    }

    let device_cookie = fetch_bilibili_device_cookie().await;
    let merged = merge_bilibili_cookies(existing, device_cookie.as_deref());
    if existing.is_some_and(|cookie| has_cookie_name(cookie, "SESSDATA")) {
        if let Some(cookie) = merged.as_deref() {
            let _ = save_bilibili_token(cookie);
        }
    }
    merged
}

async fn fetch_bilibili_device_cookie() -> Option<String> {
    let value = reqwest::Client::new()
        .get("https://api.bilibili.com/x/frontend/finger/spi")
        .header(
            "User-Agent",
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36",
        )
        .header("Accept", "application/json, text/plain, */*")
        .header("Referer", "https://www.bilibili.com/")
        .timeout(std::time::Duration::from_secs(8))
        .send()
        .await
        .ok()?
        .json::<serde_json::Value>()
        .await
        .ok()?;
    let data = value.get("data")?;
    let buvid3 = json_text(data.get("b_3")).filter(|value| !value.is_empty())?;
    let buvid4 = json_text(data.get("b_4")).filter(|value| !value.is_empty())?;
    Some(format!("buvid3={buvid3}; buvid4={buvid4}"))
}

fn merge_bilibili_cookies(existing: Option<&str>, additional: Option<&str>) -> Option<String> {
    let mut values = Vec::<(String, String)>::new();
    for cookie in [existing, additional].into_iter().flatten() {
        for pair in cookie
            .split(';')
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            let Some((name, value)) = pair.split_once('=') else {
                continue;
            };
            let name = name.trim();
            if name.is_empty() || value.trim().is_empty() {
                continue;
            }
            if let Some(found) = values
                .iter_mut()
                .find(|(stored_name, _)| stored_name.eq_ignore_ascii_case(name))
            {
                found.1 = value.trim().to_string();
            } else {
                values.push((name.to_string(), value.trim().to_string()));
            }
        }
    }
    if values.is_empty() {
        None
    } else {
        Some(
            values
                .into_iter()
                .map(|(name, value)| format!("{name}={value}"))
                .collect::<Vec<_>>()
                .join("; "),
        )
    }
}

fn has_cookie_name(cookie: &str, expected: &str) -> bool {
    cookie
        .split(';')
        .filter_map(|pair| pair.trim().split_once('='))
        .any(|(name, _)| name.trim().eq_ignore_ascii_case(expected))
}

async fn fetch_bilibili_login_status(
    config: &ResolvedBilibiliSourceConfig,
) -> Result<BilibiliLoginStatusDto, String> {
    if config.token.is_none() {
        return Ok(BilibiliLoginStatusDto {
            logged_in: false,
            expired: false,
            nickname: None,
            user_id: None,
            message: "Bilibili is available for public content. Import Cookie for private access."
                .to_string(),
        });
    }

    let value = request_bilibili_json(config, "/x/web-interface/nav", &[]).await?;
    let data = value.get("data").unwrap_or(&value);
    let logged_in = data
        .get("isLogin")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    let nickname = data
        .get("uname")
        .and_then(|value| value.as_str())
        .map(ToString::to_string);
    let user_id = Some(json_id(data.get("mid"))).filter(|value| !value.is_empty() && value != "0");

    Ok(BilibiliLoginStatusDto {
        logged_in,
        expired: !logged_in,
        nickname,
        user_id,
        message: if logged_in {
            "Connected to Bilibili.".to_string()
        } else {
            "Reconnect Bilibili to try again.".to_string()
        },
    })
}

async fn search_bilibili_videos(
    config: &ResolvedBilibiliSourceConfig,
    query: &str,
) -> Result<Vec<SourceSongDto>, String> {
    if let Some(id) = extract_bilibili_id(query) {
        return fetch_bilibili_video_metadata(config, &id)
            .await
            .map(|song| vec![song]);
    }

    let scoped_query = bilibili_scoped_query(query, &config.search_scope);
    let value = match request_bilibili_json(
        config,
        "/x/web-interface/search/type",
        &[
            ("search_type", "video"),
            ("keyword", scoped_query.as_str()),
            ("page", "1"),
            ("page_size", "12"),
        ],
    )
    .await
    {
        Ok(value) => value,
        Err(primary_error) => request_bilibili_wbi_search(config, &scoped_query)
            .await
            .map_err(|fallback_error| format!("{primary_error} {fallback_error}"))?,
    };

    let code = value
        .get("code")
        .and_then(|value| value.as_i64())
        .unwrap_or(-1);
    if code != 0 {
        let message = json_text(value.get("message"))
            .unwrap_or_else(|| "Please try again in a moment.".to_string());
        return Err(format!(
            "Bilibili search is unavailable just now. {message}"
        ));
    }
    let results = value
        .get("data")
        .and_then(|data| data.get("result"))
        .and_then(|result| result.as_array())
        .cloned()
        .unwrap_or_default();

    Ok(results
        .iter()
        .map(source_song_from_bilibili_search_json)
        .collect())
}

async fn request_bilibili_wbi_search(
    config: &ResolvedBilibiliSourceConfig,
    keyword: &str,
) -> Result<serde_json::Value, String> {
    let nav = request_bilibili_json(config, "/x/web-interface/nav", &[]).await?;
    let wbi_img = nav
        .get("data")
        .and_then(|data| data.get("wbi_img"))
        .ok_or_else(|| "Bilibili search verification is unavailable just now.".to_string())?;
    let img_key = wbi_key_from_url(json_text(wbi_img.get("img_url")).as_deref())
        .ok_or_else(|| "Bilibili search verification is unavailable just now.".to_string())?;
    let sub_key = wbi_key_from_url(json_text(wbi_img.get("sub_url")).as_deref())
        .ok_or_else(|| "Bilibili search verification is unavailable just now.".to_string())?;
    let mixin_key = bilibili_mixin_key(&format!("{img_key}{sub_key}"));
    let wts = (current_timestamp_ms() / 1000).to_string();
    let sanitized_keyword = keyword.replace(['!', '\'', '(', ')', '*'], "");
    let query = format!(
        "keyword={}&page=1&page_size=12&search_type=video&wts={}",
        encode_url_component(&sanitized_keyword),
        wts
    );
    let w_rid = format!("{:x}", md5::compute(format!("{query}{mixin_key}")));
    let url =
        format!("https://api.bilibili.com/x/web-interface/wbi/search/type?{query}&w_rid={w_rid}");
    request_bilibili_json(config, &url, &[]).await
}

fn wbi_key_from_url(url: Option<&str>) -> Option<String> {
    url?.rsplit('/')
        .next()?
        .split('.')
        .next()
        .map(ToString::to_string)
        .filter(|value| !value.is_empty())
}

fn bilibili_mixin_key(raw_key: &str) -> String {
    const MIXIN_KEY_ENC_TAB: &[usize] = &[
        46, 47, 18, 2, 53, 8, 23, 32, 15, 50, 10, 31, 58, 3, 45, 35, 27, 43, 5, 49, 33, 9, 42, 19,
        29, 28, 14, 39, 12, 38, 41, 13, 37, 48, 7, 16, 24, 55, 40, 61, 26, 17, 0, 1, 60, 51, 30, 4,
        22, 25, 54, 21, 56, 59, 6, 63, 57, 62, 11, 36, 20, 34, 44, 52,
    ];
    let chars = raw_key.chars().collect::<Vec<_>>();
    MIXIN_KEY_ENC_TAB
        .iter()
        .filter_map(|index| chars.get(*index))
        .take(32)
        .collect()
}

fn encode_url_component(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.as_bytes() {
        if byte.is_ascii_alphanumeric() || matches!(*byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(*byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

fn bilibili_scoped_query(query: &str, scope: &str) -> String {
    let extra = match scope {
        "vocaloid" => " vocaloid",
        "live" => " live 现场",
        "cover" => " cover 翻唱",
        "mv" => " MV",
        "all" => "",
        _ => " 音乐",
    };
    format!("{query}{extra}")
}

async fn fetch_bilibili_video_metadata(
    config: &ResolvedBilibiliSourceConfig,
    bvid_or_aid: &str,
) -> Result<SourceSongDto, String> {
    let clean_id =
        extract_bilibili_id(bvid_or_aid).unwrap_or_else(|| bvid_or_aid.trim().to_string());
    let (bvid_part, cid_hint) = bilibili_song_id_parts(&clean_id);
    let mut query = Vec::new();
    if bvid_part.to_lowercase().starts_with("av") {
        query.push((
            "aid",
            bvid_part
                .trim_start_matches("av")
                .trim_start_matches("AV")
                .to_string(),
        ));
    } else if bvid_part.chars().all(|ch| ch.is_ascii_digit()) {
        query.push(("aid", bvid_part.clone()));
    } else {
        query.push(("bvid", bvid_part.clone()));
    }
    let query_refs = query
        .iter()
        .map(|(key, value)| (*key, value.as_str()))
        .collect::<Vec<_>>();
    let value = request_bilibili_json(config, "/x/web-interface/view", &query_refs).await?;
    if value
        .get("code")
        .and_then(|value| value.as_i64())
        .unwrap_or(-1)
        != 0
    {
        return Err("This Bilibili track is unavailable from the current source.".to_string());
    }
    let data = value
        .get("data")
        .ok_or_else(|| "This Bilibili track is unavailable from the current source.".to_string())?;
    Ok(source_song_from_bilibili_view_json(data, cid_hint))
}

async fn fetch_bilibili_playable_url(
    config: &ResolvedBilibiliSourceConfig,
    song_id: &str,
) -> Result<PlayableUrlDto, String> {
    let metadata = fetch_bilibili_video_metadata(config, song_id).await?;
    let bvid = metadata.bvid.clone().unwrap_or_else(|| song_id.to_string());
    let cid = metadata
        .cid
        .clone()
        .ok_or_else(|| "audio_stream_missing".to_string())?;
    let value = request_bilibili_json(
        config,
        "/x/player/playurl",
        &[
            ("bvid", bvid.as_str()),
            ("cid", cid.as_str()),
            ("fnval", "4048"),
            ("fourk", "1"),
        ],
    )
    .await
    .map_err(|_| "playurl_failed".to_string())?;

    let code = value
        .get("code")
        .and_then(|value| value.as_i64())
        .unwrap_or(-1);
    if code != 0 {
        return Ok(bilibili_unavailable_playable(
            song_id,
            classify_bilibili_playurl_reason(&value),
        ));
    }
    let data = value.get("data").unwrap_or(&value);
    let audio_candidates = data
        .get("dash")
        .and_then(|dash| dash.get("audio"))
        .and_then(|audio| audio.as_array())
        .and_then(|audio| {
            audio.iter().max_by_key(|item| {
                item.get("bandwidth")
                    .and_then(|value| value.as_u64())
                    .unwrap_or(0)
            })
        })
        .map(bilibili_media_url_candidates)
        .or_else(|| {
            data.get("durl")
                .and_then(|items| items.as_array())
                .and_then(|items| items.first())
                .and_then(|item| item.get("url"))
                .and_then(|url| url.as_str())
                .map(|url| vec![url.to_string()])
        })
        .unwrap_or_default();

    let video_candidates = data
        .get("dash")
        .and_then(|dash| dash.get("video"))
        .and_then(|video| video.as_array())
        .and_then(|video| {
            video
                .iter()
                .filter(|item| item.get("codecid").and_then(|value| value.as_u64()) == Some(7))
                .filter(|item| {
                    item.get("height")
                        .and_then(|value| value.as_u64())
                        .unwrap_or(0)
                        <= 720
                })
                .max_by_key(|item| {
                    item.get("bandwidth")
                        .and_then(|value| value.as_u64())
                        .unwrap_or(0)
                })
                .or_else(|| {
                    video
                        .iter()
                        .filter(|item| {
                            item.get("height")
                                .and_then(|value| value.as_u64())
                                .unwrap_or(0)
                                <= 720
                        })
                        .min_by_key(|item| {
                            item.get("bandwidth")
                                .and_then(|value| value.as_u64())
                                .unwrap_or(u64::MAX)
                        })
                })
        })
        .map(bilibili_media_url_candidates)
        .unwrap_or_default();

    match audio_candidates.first().cloned() {
        Some(url) if !url.trim().is_empty() => Ok(PlayableUrlDto {
            song_id: song_id.to_string(),
            url: Some(url),
            video_url: video_candidates.first().cloned(),
            unavailable: false,
            reason: None,
            debug: None,
            audio_candidates,
            video_candidates,
        }),
        _ => Ok(bilibili_unavailable_playable(
            song_id,
            "audio_stream_missing",
        )),
    }
}

fn bilibili_media_url_candidates(item: &serde_json::Value) -> Vec<String> {
    let mut urls = Vec::new();
    if let Some(url) = item
        .get("baseUrl")
        .or_else(|| item.get("base_url"))
        .and_then(|value| value.as_str())
    {
        urls.push(url.to_string());
    }
    if let Some(backups) = item
        .get("backupUrl")
        .or_else(|| item.get("backup_url"))
        .and_then(|value| value.as_array())
    {
        urls.extend(
            backups
                .iter()
                .filter_map(|value| value.as_str())
                .map(ToString::to_string),
        );
    }
    urls.retain(|url| !url.trim().is_empty());
    urls.sort_by_key(|url| {
        let lower = url.to_ascii_lowercase();
        if lower.contains("mcdn") || lower.contains("akamaized") {
            1
        } else {
            0
        }
    });
    urls.dedup();
    urls
}

fn bilibili_unavailable_playable(song_id: &str, reason: &str) -> PlayableUrlDto {
    PlayableUrlDto {
        song_id: song_id.to_string(),
        url: None,
        video_url: None,
        unavailable: true,
        reason: Some(reason.to_string()),
        debug: None,
        audio_candidates: Vec::new(),
        video_candidates: Vec::new(),
    }
}

fn classify_bilibili_playurl_reason(value: &serde_json::Value) -> &'static str {
    let message = value
        .get("message")
        .or_else(|| value.get("msg"))
        .and_then(|value| value.as_str())
        .unwrap_or("")
        .to_lowercase();
    if message.contains("login") || message.contains("登录") {
        "not_logged_in"
    } else if message.contains("vip") || message.contains("会员") || message.contains("付费") {
        "vip_required"
    } else if message.contains("copyright") || message.contains("版权") {
        "no_copyright"
    } else if message.contains("region") || message.contains("地区") {
        "region_restricted"
    } else if message.contains("不存在") || message.contains("下架") {
        "video_removed"
    } else {
        "playurl_failed"
    }
}

struct BilibiliDanmakuFetch {
    items: Vec<DanmakuItemDto>,
    request_url: String,
    raw_length: usize,
}

async fn fetch_bilibili_danmaku_items(
    config: &ResolvedBilibiliSourceConfig,
    cid: &str,
) -> Result<BilibiliDanmakuFetch, String> {
    let primary_url = format!(
        "{}/x/v1/dm/list.so?oid={cid}",
        config.base_url.trim_end_matches('/')
    );
    if let Ok(xml) = request_bilibili_danmaku_xml(config, &primary_url).await {
        let items = parse_bilibili_danmaku_xml(cid, &xml);
        if !items.is_empty() {
            return Ok(BilibiliDanmakuFetch {
                items,
                request_url: primary_url,
                raw_length: xml.len(),
            });
        }
    }

    let fallback_url = format!("https://comment.bilibili.com/{cid}.xml");
    let xml = request_bilibili_danmaku_xml(config, &fallback_url).await?;
    let items = parse_bilibili_danmaku_xml(cid, &xml);
    if items.is_empty() {
        return Err("No danmaku was returned for this video.".to_string());
    }
    Ok(BilibiliDanmakuFetch {
        items,
        request_url: fallback_url,
        raw_length: xml.len(),
    })
}

async fn request_bilibili_danmaku_xml(
    config: &ResolvedBilibiliSourceConfig,
    url: &str,
) -> Result<String, String> {
    // Bilibili labels this response as deflate but sends a raw DEFLATE stream.
    // Disable reqwest's zlib decoder and handle both wire formats explicitly.
    let client = reqwest::Client::builder()
        .no_gzip()
        .no_brotli()
        .no_deflate()
        .build()
        .map_err(|error| error.to_string())?;
    let mut request = client
        .get(url)
        .header(
            "User-Agent",
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36",
        )
        .header("Accept", "application/xml,text/xml,*/*")
        .header("Referer", "https://www.bilibili.com/")
        .timeout(std::time::Duration::from_secs(12));

    if let Some(cookie) = resolve_bilibili_request_cookie(config).await {
        request = request.header("Cookie", cookie);
    }

    let response = request
        .send()
        .await
        .map_err(|error| format!("弹幕请求失败：{error} / Danmaku request failed: {error}"))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!(
            "Bilibili danmaku is unavailable just now ({status})."
        ));
    }
    let encoding = response
        .headers()
        .get(reqwest::header::CONTENT_ENCODING)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();
    let bytes = response.bytes().await.map_err(|error| error.to_string())?;

    let decoded = if encoding.contains("deflate") {
        decode_deflate_payload(&bytes)?
    } else {
        bytes.to_vec()
    };
    Ok(String::from_utf8_lossy(&decoded).into_owned())
}

fn decode_deflate_payload(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let mut raw = flate2::read::DeflateDecoder::new(bytes);
    let mut decoded = Vec::new();
    if raw.read_to_end(&mut decoded).is_ok() {
        return Ok(decoded);
    }

    let mut zlib = flate2::read::ZlibDecoder::new(bytes);
    let mut decoded = Vec::new();
    zlib.read_to_end(&mut decoded)
        .map_err(|error| format!("Could not decode danmaku: {error}"))?;
    Ok(decoded)
}

fn parse_bilibili_danmaku_xml(cid: &str, xml: &str) -> Vec<DanmakuItemDto> {
    let mut items = Vec::new();
    let mut cursor = 0_usize;
    while let Some(start_rel) = xml[cursor..].find("<d p=\"") {
        let start = cursor + start_rel + 6;
        let Some(end_attr_rel) = xml[start..].find('"') else {
            break;
        };
        let attr = &xml[start..start + end_attr_rel];
        let text_start = start + end_attr_rel + 2;
        let Some(text_end_rel) = xml[text_start..].find("</d>") else {
            break;
        };
        let raw_text = &xml[text_start..text_start + text_end_rel];
        cursor = text_start + text_end_rel + 4;

        let parts = attr.split(',').collect::<Vec<_>>();
        let text = decode_xml_text(raw_text);
        if text.trim().is_empty() || text.chars().count() > 42 {
            continue;
        }
        let time = parts
            .first()
            .and_then(|value| value.parse::<f64>().ok())
            .unwrap_or(0.0);
        let mode = parts.get(1).copied().unwrap_or("").to_string();
        let font_size = parts.get(2).copied().unwrap_or("").to_string();
        let color = parts.get(3).copied().unwrap_or("").to_string();
        let timestamp = parts.get(4).copied().unwrap_or("").to_string();
        let user_hash = parts.get(6).copied().unwrap_or("").to_string();
        let id = stable_id(&format!("bilibili:{cid}:{time}:{user_hash}:{text}"));
        items.push(DanmakuItemDto {
            id,
            source: "bilibili".to_string(),
            cid: cid.to_string(),
            time,
            text,
            mode,
            color,
            font_size,
            timestamp,
            user_hash,
            weight: 1.0,
        });
    }
    items.sort_by(|left, right| {
        left.time
            .partial_cmp(&right.time)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    dedupe_danmaku(items)
}

fn dedupe_danmaku(items: Vec<DanmakuItemDto>) -> Vec<DanmakuItemDto> {
    let mut seen = HashSet::new();
    items
        .into_iter()
        .filter(|item| {
            let key = format!("{}:{:.0}", item.text, item.time);
            seen.insert(key)
        })
        .take(5000)
        .collect()
}

fn decode_xml_text(value: &str) -> String {
    value
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
        .trim()
        .to_string()
}

fn danmaku_cache_key(source: &str, cid: &str) -> String {
    format!("{}:{}:danmaku", source.trim().to_lowercase(), cid.trim())
}

fn load_cached_danmaku(
    state: &State<'_, AppState>,
    cache_key: &str,
) -> Result<Option<Vec<DanmakuItemDto>>, String> {
    let db = state.db.lock().map_err(|error| error.to_string())?;
    let raw = db
        .query_row(
            "SELECT items_json FROM danmaku_cache WHERE cache_key = ?1",
            params![cache_key],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    Ok(raw
        .and_then(|value| serde_json::from_str::<Vec<DanmakuItemDto>>(&value).ok())
        .filter(|items| !items.is_empty()))
}

fn save_cached_danmaku(
    state: &State<'_, AppState>,
    cache_key: &str,
    source: &str,
    cid: &str,
    items: &[DanmakuItemDto],
) -> Result<(), String> {
    let items_json = serde_json::to_string(items).map_err(|error| error.to_string())?;
    let db = state.db.lock().map_err(|error| error.to_string())?;
    db.execute(
        "INSERT INTO danmaku_cache (cache_key, source, source_id, items_json, bytes, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, CURRENT_TIMESTAMP)
         ON CONFLICT(cache_key) DO UPDATE SET
           items_json = excluded.items_json,
           bytes = excluded.bytes,
           updated_at = CURRENT_TIMESTAMP",
        params![cache_key, source, cid, items_json, items_json.len() as i64],
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

fn prune_danmaku_cache(state: &State<'_, AppState>) -> Result<(), String> {
    const DANMAKU_CACHE_LIMIT_BYTES: i64 = 50 * 1024 * 1024;
    let db = state.db.lock().map_err(|error| error.to_string())?;
    let total: i64 = db
        .query_row(
            "SELECT COALESCE(SUM(bytes), 0) FROM danmaku_cache",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);
    if total <= DANMAKU_CACHE_LIMIT_BYTES {
        return Ok(());
    }
    db.execute(
        "DELETE FROM danmaku_cache
         WHERE cache_key IN (
           SELECT cache_key FROM danmaku_cache
           ORDER BY updated_at ASC
           LIMIT 20
         )",
        [],
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

fn extract_bilibili_id(value: &str) -> Option<String> {
    let trimmed = value.trim();
    for token in trimmed.split(|ch: char| ch.is_whitespace() || ch == '/' || ch == '?' || ch == '&')
    {
        if token.starts_with("BV") && token.len() >= 10 {
            return Some(
                token
                    .trim_matches(|ch: char| !ch.is_ascii_alphanumeric())
                    .to_string(),
            );
        }
        if token.starts_with("av") && token[2..].chars().all(|ch| ch.is_ascii_digit()) {
            return Some(token.to_string());
        }
    }
    None
}

fn bilibili_song_id_parts(song_id: &str) -> (String, Option<String>) {
    if let Some((left, right)) = song_id.split_once(':') {
        (left.to_string(), Some(right.to_string()))
    } else {
        (song_id.to_string(), None)
    }
}

fn source_song_from_bilibili_search_json(value: &serde_json::Value) -> SourceSongDto {
    let bvid = json_text(value.get("bvid")).unwrap_or_else(|| json_id(value.get("aid")));
    let title = clean_bilibili_title(
        &json_text(value.get("title")).unwrap_or_else(|| "Bilibili Track".to_string()),
    );
    let uploader = json_text(value.get("author")).unwrap_or_else(|| "Bilibili".to_string());
    let (artist, album) = infer_bilibili_artist_album(&title, &uploader);
    let cover_url = normalize_bilibili_image_url(json_text(value.get("pic")).unwrap_or_default());
    SourceSongDto {
        id: bvid.clone(),
        source: Some("bilibili".to_string()),
        title,
        artist,
        album,
        duration_seconds: parse_bilibili_duration(value.get("duration")).unwrap_or(0),
        cover_url,
        playable_url: None,
        unavailable: false,
        unavailable_reason: None,
        bvid: Some(bvid),
        aid: Some(json_id(value.get("aid"))).filter(|value| !value.is_empty()),
        cid: None,
        uploader: Some(uploader),
        danmaku_count: value
            .get("danmaku")
            .or_else(|| value.get("video_review"))
            .and_then(|value| value.as_u64()),
        play_count: value.get("play").and_then(|value| value.as_u64()),
        page_index: Some(0),
        source_url: json_text(value.get("arcurl")),
    }
}

fn source_song_from_bilibili_view_json(
    value: &serde_json::Value,
    cid_hint: Option<String>,
) -> SourceSongDto {
    let bvid = json_text(value.get("bvid")).unwrap_or_else(|| json_id(value.get("aid")));
    let page = value
        .get("pages")
        .and_then(|pages| pages.as_array())
        .and_then(|pages| {
            cid_hint
                .as_ref()
                .and_then(|cid| pages.iter().find(|page| json_id(page.get("cid")) == *cid))
                .or_else(|| pages.first())
        });
    let cid = cid_hint
        .or_else(|| page.map(|page| json_id(page.get("cid"))))
        .filter(|value| !value.is_empty());
    let title = clean_bilibili_title(
        &json_text(value.get("title")).unwrap_or_else(|| "Bilibili Track".to_string()),
    );
    let uploader = value
        .get("owner")
        .and_then(|owner| owner.get("name"))
        .and_then(|name| name.as_str())
        .unwrap_or("Bilibili")
        .to_string();
    let (artist, album) = infer_bilibili_artist_album(&title, &uploader);
    SourceSongDto {
        id: match cid.as_deref() {
            Some(cid) => format!("{bvid}:{cid}"),
            None => bvid.clone(),
        },
        source: Some("bilibili".to_string()),
        title,
        artist,
        album,
        duration_seconds: page
            .and_then(|page| page.get("duration"))
            .and_then(|duration| duration.as_u64())
            .or_else(|| value.get("duration").and_then(|duration| duration.as_u64()))
            .unwrap_or(0),
        cover_url: normalize_bilibili_image_url(json_text(value.get("pic")).unwrap_or_default()),
        playable_url: None,
        unavailable: false,
        unavailable_reason: None,
        bvid: Some(bvid.clone()),
        aid: Some(json_id(value.get("aid"))).filter(|value| !value.is_empty()),
        cid,
        uploader: Some(uploader),
        danmaku_count: value
            .get("stat")
            .and_then(|stat| stat.get("danmaku"))
            .and_then(|value| value.as_u64()),
        play_count: value
            .get("stat")
            .and_then(|stat| stat.get("view"))
            .and_then(|value| value.as_u64()),
        page_index: page
            .and_then(|page| page.get("page"))
            .and_then(|value| value.as_u64())
            .map(|value| value.saturating_sub(1) as u32),
        source_url: Some(format!("https://www.bilibili.com/video/{bvid}")),
    }
}

fn clean_bilibili_title(title: &str) -> String {
    title
        .replace("<em class=\"keyword\">", "")
        .replace("</em>", "")
        .replace("&quot;", "\"")
        .replace("&amp;", "&")
        .trim()
        .to_string()
}

fn infer_bilibili_artist_album(title: &str, uploader: &str) -> (String, String) {
    let normalized = title.replace(['《', '》'], " - ");
    let separators = [" - ", "-", "—", "|", "｜"];
    for separator in separators {
        if let Some((left, right)) = normalized.split_once(separator) {
            let left = clean_bilibili_title(left).trim().to_string();
            let right = clean_bilibili_title(right).trim().to_string();
            if !left.is_empty() && !right.is_empty() {
                if left.chars().count() <= 24 {
                    return (left, right);
                }
                return (right, "Bilibili".to_string());
            }
        }
    }
    (uploader.to_string(), "Bilibili".to_string())
}

fn normalize_bilibili_image_url(value: String) -> String {
    let normalized = value.replace("\\u002F", "/").trim().to_string();
    if normalized.starts_with("//") {
        format!("https:{normalized}")
    } else if normalized.starts_with("http://") {
        normalized.replacen("http://", "https://", 1)
    } else {
        normalized
    }
}

fn normalize_netease_image_url(value: String) -> String {
    let normalized = value.trim().to_string();
    if normalized.is_empty() {
        return normalized;
    }
    if normalized.starts_with("//") {
        format!("https:{normalized}")
    } else if normalized.starts_with("http://") {
        normalized.replacen("http://", "https://", 1)
    } else if !normalized.starts_with("https://") && !normalized.starts_with("data:") {
        String::new()
    } else {
        normalized
    }
}

// 安全截取字符串预览，避免 UTF-8 字符边界 panic
// Safe string preview that avoids UTF-8 char boundary panics
fn safe_str_preview(s: &str, n: usize) -> &str {
    if s.len() <= n {
        return s;
    }
    let mut end = n;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

fn safe_url_preview(url: &str, n: usize) -> &str {
    safe_str_preview(url, n)
}

// Only remote http(s) URLs need proxying. Local/inline covers (data:, blob:,
// file:), already-proxied ome-media:// URLs, and the SVG fallback cover
// produced by `fallback_cover_url` must be passed through untouched so the
// WebView can render them directly. Routing a data: URI through reqwest would
// break the default placeholder for songs with no cover art.
fn is_proxyable_remote_url(url: &str) -> bool {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return false;
    }
    let lower = trimmed.to_ascii_lowercase();
    if lower.starts_with("data:")
        || lower.starts_with("blob:")
        || lower.starts_with("file:")
        || lower.starts_with("ome-media:")
        || lower.contains("ome-media.localhost")
    {
        return false;
    }
    lower.starts_with("http://") || lower.starts_with("https://")
}

fn proxy_bilibili_song_cover(
    state: &State<'_, AppState>,
    song: &mut SourceSongDto,
) -> Result<(), String> {
    if is_proxyable_remote_url(&song.cover_url) {
        song.cover_url = register_media_proxy(state.inner(), &song.cover_url, "image")?;
    }
    Ok(())
}

// NetEase album covers (p1.music.126.net) are usually public HTTPS, but in some
// regions / for some songs the API returns http:// or protocol-relative URLs that
// the WebView CSP refuses, or the CDN returns 403 for cross-origin requests. Route
// every NetEase cover through the ome-media:// proxy so covers always render.
fn proxy_netease_song_covers(
    state: &State<'_, AppState>,
    songs: &mut [SourceSongDto],
) -> Result<(), String> {
    for song in songs.iter_mut().filter(|song| {
        song.source.as_deref() == Some("netease") || song.source.as_deref().is_none()
    }) {
        if !is_proxyable_remote_url(&song.cover_url) {
            continue;
        }
        song.cover_url = register_media_proxy(state.inner(), &song.cover_url, "image")?;
    }
    Ok(())
}

fn proxy_bilibili_song_covers(
    state: &State<'_, AppState>,
    songs: &mut [SourceSongDto],
) -> Result<(), String> {
    for song in songs {
        proxy_bilibili_song_cover(state, song)?;
    }
    Ok(())
}

fn proxy_bilibili_track_covers(
    state: &State<'_, AppState>,
    tracks: &mut [TrackDto],
) -> Result<(), String> {
    for track in tracks.iter_mut().filter(|track| track.source == "bilibili") {
        if is_proxyable_remote_url(&track.cover_url) {
            track.cover_url = register_media_proxy(state.inner(), &track.cover_url, "image")?;
        }
    }
    Ok(())
}

// NetEase library rows already store stable https://p*.music.126.net cover URLs.
// Keep those URLs for queue/history rendering instead of replacing them with a
// short-lived runtime proxy token. The proxy is still useful for search results
// and playable media, but queue/history artwork should survive reloads and
// should not fall back to the OME placeholder if a transient proxy request fails.
fn proxy_netease_track_covers(
    state: &State<'_, AppState>,
    tracks: &mut [TrackDto],
) -> Result<(), String> {
    let _ = state;
    let _ = tracks;
    Ok(())
}

fn proxy_netease_song_cover(
    state: &State<'_, AppState>,
    song: &mut SourceSongDto,
) -> Result<(), String> {
    if is_proxyable_remote_url(&song.cover_url) {
        song.cover_url = register_media_proxy(state.inner(), &song.cover_url, "image")?;
    }
    Ok(())
}

fn proxy_bilibili_playback(
    state: &State<'_, AppState>,
    playback: &mut PlayableUrlDto,
) -> Result<(), String> {
    if !playback.audio_candidates.is_empty() {
        playback.url = Some(register_media_proxy_candidates(
            state.inner(),
            playback.audio_candidates.clone(),
            "audio",
        )?);
    } else if let Some(url) = playback.url.as_deref() {
        playback.url = Some(register_media_proxy(state.inner(), url, "audio")?);
    }
    if !playback.video_candidates.is_empty() {
        playback.video_url = Some(register_media_proxy_candidates(
            state.inner(),
            playback.video_candidates.clone(),
            "video",
        )?);
    } else if let Some(url) = playback.video_url.as_deref() {
        playback.video_url = Some(register_media_proxy(state.inner(), url, "video")?);
    }
    Ok(())
}

fn register_media_proxy(state: &AppState, url: &str, kind: &'static str) -> Result<String, String> {
    register_media_proxy_candidates(state, vec![url.to_string()], kind)
}

fn register_media_proxy_candidates(
    state: &AppState,
    urls: Vec<String>,
    kind: &'static str,
) -> Result<String, String> {
    if urls.is_empty() {
        return Err("Media source is unavailable.".to_string());
    }
    // Token must be unguessable: combine the URL hash with a process-wide counter
    // and the current monotonic time so consecutive registrations differ even for
    // identical inputs. SHA-1 is fine here as a mixing function (not a password).
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let nonce = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let salt = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let token = stable_id(&format!("{kind}:{nonce}:{salt}:{}", urls.join("|")));

    let mut registry = state
        .media_proxy
        .lock()
        .map_err(|error| error.to_string())?;
    // Evict expired entries and enforce a soft LRU cap.
    let now = std::time::SystemTime::now();
    registry.retain(|_, entry| now < entry.expires_at);
    if registry.len() >= MEDIA_PROXY_MAX_ENTRIES {
        // Drop the oldest entry by expiry time.
        let oldest_key = registry
            .iter()
            .min_by_key(|(_, entry)| entry.expires_at)
            .map(|(key, _)| key.clone());
        if let Some(key) = oldest_key {
            registry.remove(&key);
        }
    }
    registry.insert(
        token.clone(),
        MediaProxyEntry {
            urls,
            kind,
            expires_at: now + MEDIA_PROXY_TTL,
        },
    );

    #[cfg(target_os = "windows")]
    return Ok(format!("http://ome-media.localhost/{token}"));
    #[cfg(not(target_os = "windows"))]
    return Ok(format!("ome-media://localhost/{token}"));
}

async fn respond_bilibili_media(
    app: AppHandle,
    request: tauri::http::Request<Vec<u8>>,
) -> tauri::http::Response<Vec<u8>> {
    let token = request.uri().path().trim_start_matches('/');
    let entry = app
        .state::<AppState>()
        .media_proxy
        .lock()
        .ok()
        .and_then(|registry| registry.get(token).cloned())
        .filter(|entry| entry.expires_at > std::time::SystemTime::now());
    let Some(entry) = entry else {
        return media_proxy_error(tauri::http::StatusCode::NOT_FOUND, "Media link expired.");
    };

    let client = match reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::limited(6))
        .build()
    {
        Ok(client) => client,
        Err(_) => {
            return media_proxy_error(
                tauri::http::StatusCode::BAD_GATEWAY,
                "Could not open media source.",
            )
        }
    };
    let is_head = request.method() == tauri::http::Method::HEAD;
    // Bilibili media CDNs commonly reject HEAD even though ranged GET is supported.
    let requested_range = request
        .headers()
        .get(tauri::http::header::RANGE)
        .and_then(|value| value.to_str().ok())
        .map(ToString::to_string)
        .or_else(|| is_head.then(|| "bytes=0-0".to_string()))
        .or_else(|| (entry.kind != "image").then(|| "bytes=0-4194303".to_string()));

    let mut response = None;
    for url in &entry.urls {
        let lower_url = url.to_ascii_lowercase();
        // Only Bilibili CDNs (bilivideo, bilivideo.com, hdslb.com, akamaized.net
        // paths owned by Bilibili) require the Bilibili Referer/Origin pair. NetEase
        // music CDNs (music.126.net) and image CDNs do not need — and should not
        // receive — these headers.
        let is_bilibili_host = lower_url.contains("bilibili")
            || lower_url.contains("bilivideo")
            || lower_url.contains("hdslb.com")
            || lower_url.contains("akamaized.net");
        // QQ 音乐 CDN (aqqmusic.tc.qq.com, ws.stream.qqmusic.qq.com) 需要 Referer
        let is_qqmusic_host = lower_url.contains("qq.com")
            || lower_url.contains("tc.qq.com")
            || lower_url.contains("stream.qqmusic");
        let mut remote = client
            .get(url)
            .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 Chrome/131.0.0.0 Safari/537.36");
        if is_bilibili_host {
            remote = remote
                .header("Referer", "https://www.bilibili.com/")
                .header("Origin", "https://www.bilibili.com");
        }
        if is_qqmusic_host {
            remote = remote
                .header("Referer", "https://y.qq.com/")
                .header("Origin", "https://y.qq.com")
                .header("Accept", "*/*")
                .header("Accept-Encoding", "identity")
                .header("Cache-Control", "no-cache");
        }
        if let Some(range) = requested_range.as_deref() {
            remote = remote.header(reqwest::header::RANGE, range);
        }
        if let Ok(candidate) = remote.send().await {
            let status = candidate.status();
            let url_preview = safe_url_preview(url, 100);
            eprintln!(
                "[MediaProxy] {} status={} (url_len={})",
                url_preview,
                status,
                url.len()
            );
            if status.is_success() {
                response = Some(candidate);
                break;
            }
            // 非 2xx：打印响应体帮助诊断
            if !status.is_success() {
                if let Ok(body) = candidate.text().await {
                    let body_preview = safe_str_preview(&body, 200);
                    eprintln!("[MediaProxy] error body: {}", body_preview);
                }
            }
        } else {
            eprintln!("[MediaProxy] {} request failed", safe_url_preview(url, 100));
        }
    }
    let Some(response) = response else {
        return media_proxy_error(
            tauri::http::StatusCode::BAD_GATEWAY,
            "Media source is unavailable.",
        );
    };
    let status = tauri::http::StatusCode::from_u16(response.status().as_u16())
        .unwrap_or(tauri::http::StatusCode::BAD_GATEWAY);
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(ToString::to_string);
    let content_range = response
        .headers()
        .get(reqwest::header::CONTENT_RANGE)
        .and_then(|value| value.to_str().ok())
        .map(ToString::to_string);
    let accept_ranges = response
        .headers()
        .get(reqwest::header::ACCEPT_RANGES)
        .and_then(|value| value.to_str().ok())
        .map(ToString::to_string);
    let remote_content_length = response.content_length();
    let body = if is_head {
        Vec::new()
    } else {
        match response.bytes().await {
            Ok(bytes) => bytes.to_vec(),
            Err(_) => {
                return media_proxy_error(
                    tauri::http::StatusCode::BAD_GATEWAY,
                    "Media stream was interrupted.",
                )
            }
        }
    };

    let content_length = if is_head {
        remote_content_length.unwrap_or(0) as usize
    } else {
        body.len()
    };
    let mut builder = tauri::http::Response::builder()
        .status(status)
        .header(tauri::http::header::CACHE_CONTROL, "private, max-age=300")
        .header(
            tauri::http::header::CONTENT_LENGTH,
            content_length.to_string(),
        );
    let resolved_content_type = match content_type.as_deref() {
        None | Some("application/octet-stream") if entry.kind == "video" => {
            Some("video/mp4".to_string())
        }
        None | Some("application/octet-stream") if entry.kind == "audio" => {
            Some("audio/mp4".to_string())
        }
        _ => content_type,
    };
    if let Some(value) = resolved_content_type {
        builder = builder.header(tauri::http::header::CONTENT_TYPE, value);
    }
    if let Some(value) = content_range {
        builder = builder.header(tauri::http::header::CONTENT_RANGE, value);
    }
    if let Some(value) = accept_ranges {
        builder = builder.header(tauri::http::header::ACCEPT_RANGES, value);
    }
    builder.body(body).unwrap_or_else(|_| {
        media_proxy_error(
            tauri::http::StatusCode::BAD_GATEWAY,
            "Media response failed.",
        )
    })
}

fn media_proxy_error(
    status: tauri::http::StatusCode,
    message: &str,
) -> tauri::http::Response<Vec<u8>> {
    tauri::http::Response::builder()
        .status(status)
        .header(
            tauri::http::header::CONTENT_TYPE,
            "text/plain; charset=utf-8",
        )
        .body(message.as_bytes().to_vec())
        .unwrap_or_else(|_| {
            tauri::http::Response::builder()
                .status(status)
                .body(Vec::new())
                .expect("empty media proxy error body must build")
        })
}

fn parse_bilibili_duration(value: Option<&serde_json::Value>) -> Option<u64> {
    match value? {
        serde_json::Value::Number(number) => number.as_u64(),
        serde_json::Value::String(text) => {
            let parts = text
                .split(':')
                .filter_map(|part| part.parse::<u64>().ok())
                .collect::<Vec<_>>();
            match parts.as_slice() {
                [minutes, seconds] => Some(minutes * 60 + seconds),
                [hours, minutes, seconds] => Some(hours * 3600 + minutes * 60 + seconds),
                _ => None,
            }
        }
        _ => None,
    }
}

fn resolve_managed_netease_api_runtime(app: &tauri::App) -> Option<ManagedNeteaseApiRuntime> {
    let candidates = [
        (
            "resources/node/node.exe",
            "resources/netease-runtime/node_modules/NeteaseCloudMusicApi/app.js",
        ),
        (
            "node/node.exe",
            "netease-runtime/node_modules/NeteaseCloudMusicApi/app.js",
        ),
    ];

    candidates.into_iter().find_map(|(node_path, app_path)| {
        let node_exe = app
            .path()
            .resolve(node_path, BaseDirectory::Resource)
            .ok()?;
        let app_js = app.path().resolve(app_path, BaseDirectory::Resource).ok()?;
        if node_exe.exists() && app_js.exists() {
            Some(ManagedNeteaseApiRuntime { node_exe, app_js })
        } else {
            None
        }
    })
}

async fn ensure_local_netease_api_service(
    base_url: &str,
    managed_api: Option<ManagedNeteaseApiRuntime>,
    child_handle: &Mutex<Option<Child>>,
) -> Result<NeteaseServiceStatusDto, String> {
    let base_url = normalize_netease_base_url(base_url);
    if !is_local_netease_base_url(&base_url) {
        return Ok(NeteaseServiceStatusDto {
            running: false,
            started: false,
            base_url,
            message: "External music source selected.".to_string(),
            node_available: false,
            api_package_found: false,
            stage: "not_started".to_string(),
        });
    }

    let managed_api_found = managed_api.is_some();

    #[cfg(debug_assertions)]
    let api_entry = find_netease_api_entry();
    #[cfg(not(debug_assertions))]
    let api_entry: Option<PathBuf> = None;

    #[cfg(debug_assertions)]
    let system_node_available = is_node_available();
    #[cfg(not(debug_assertions))]
    let system_node_available = false;

    #[cfg(debug_assertions)]
    let npx_available = is_npx_available();
    #[cfg(not(debug_assertions))]
    let npx_available = false;

    let api_package_found = api_entry.is_some();
    let node_available = managed_api_found || system_node_available;

    if is_netease_api_reachable(&base_url).await {
        return Ok(NeteaseServiceStatusDto {
            running: true,
            started: false,
            base_url,
            message: "Music source is awake.".to_string(),
            node_available,
            api_package_found: api_package_found || managed_api_found || npx_available,
            stage: "ready".to_string(),
        });
    }

    if !node_available {
        return Ok(NeteaseServiceStatusDto {
            running: false,
            started: false,
            base_url,
            message: "The built-in NetEase runtime was not found. Please reinstall Ome Music, or set an external music source in Settings.".to_string(),
            node_available: false,
            api_package_found,
            stage: "failed".to_string(),
        });
    }

    if !managed_api_found && !api_package_found && !npx_available {
        return Ok(NeteaseServiceStatusDto {
            running: false,
            started: false,
            base_url,
            message: "The NetEase music source is not ready. Please reinstall Ome Music, or set an external music source in Settings.".to_string(),
            node_available: true,
            api_package_found: false,
            stage: "failed".to_string(),
        });
    }

    let port = netease_base_url_port(&base_url).unwrap_or(3000).to_string();
    let mut command = if let Some(runtime) = managed_api {
        let mut command = Command::new(runtime.node_exe);
        command.arg(runtime.app_js);
        command
    } else if let Some(app_js) = api_entry {
        let working_dir = app_js
            .parent()
            .map(Path::to_path_buf)
            .ok_or_else(|| "NetEase Cloud Music source folder was not found.".to_string())?;
        let mut command = Command::new("node");
        command.arg(&app_js).current_dir(working_dir);
        command
    } else {
        let mut command = Command::new(npx_command());
        command.arg("--yes").arg("NeteaseCloudMusicApi@4.32.0");
        command
    };

    command
        .env("PORT", port)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    suppress_command_window(&mut command);

    // Reuse a still-running managed child so repeated calls do not orphan or
    // duplicate the bundled node.exe process. Clear any handle whose child has
    // already exited so a fresh process can be spawned.
    let mut existing_child_alive = false;
    if let Ok(mut guard) = child_handle.lock() {
        let needs_clear = match guard.as_mut() {
            Some(child) => matches!(child.try_wait(), Ok(Some(_)) | Err(_)),
            None => false,
        };
        if needs_clear {
            *guard = None;
        }
        existing_child_alive = guard.is_some();
    }

    if !existing_child_alive {
        let child = command
            .spawn()
            .map_err(|error| format!("Could not wake the music source. {error}"))?;
        if let Ok(mut guard) = child_handle.lock() {
            *guard = Some(child);
        }
    }

    let attempts = if managed_api_found || api_package_found {
        32
    } else {
        70
    };
    for _ in 0..attempts {
        if is_netease_api_reachable(&base_url).await {
            return Ok(NeteaseServiceStatusDto {
                running: true,
                started: true,
                base_url,
                message: if managed_api_found {
                    "Music source is ready.".to_string()
                } else if api_package_found {
                    "Music source is awake.".to_string()
                } else {
                    "Music source is awake through npm.".to_string()
                },
                node_available,
                api_package_found: managed_api_found || api_package_found || npx_available,
                stage: "ready".to_string(),
            });
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }

    Ok(NeteaseServiceStatusDto {
        running: false,
        started: true,
        base_url,
        message: if managed_api_found {
            "Music source is starting. Please try again in a moment.".to_string()
        } else if api_package_found {
            "Music source is still warming up.".to_string()
        } else {
            "Music source is being prepared through npm. The first launch can take a little longer."
                .to_string()
        },
        node_available,
        api_package_found: managed_api_found || api_package_found || npx_available,
        stage: "waiting_health".to_string(),
    })
}

#[cfg(debug_assertions)]
fn is_node_available() -> bool {
    let mut command = Command::new("node");
    command.arg("--version");
    suppress_command_window(&mut command);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .is_ok_and(|output| output.status.success())
}

#[cfg(debug_assertions)]
fn is_npx_available() -> bool {
    let mut command = Command::new(npx_command());
    command.arg("--version");
    suppress_command_window(&mut command);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .is_ok_and(|output| output.status.success())
}

#[cfg(target_os = "windows")]
fn npx_command() -> &'static str {
    "npx.cmd"
}

#[cfg(not(target_os = "windows"))]
fn npx_command() -> &'static str {
    "npx"
}

async fn is_netease_api_reachable(base_url: &str) -> bool {
    let endpoint = format!(
        "{}/login/status?timestamp={}",
        base_url.trim_end_matches('/'),
        current_timestamp_ms()
    );
    reqwest::Client::new()
        .get(endpoint)
        .timeout(std::time::Duration::from_secs(2))
        .send()
        .await
        .map(|response| response.status().is_success())
        .unwrap_or(false)
}

fn normalize_netease_base_url(base_url: &str) -> String {
    let trimmed = base_url.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        "http://127.0.0.1:3000".to_string()
    } else {
        trimmed.to_string()
    }
}

fn is_local_netease_base_url(base_url: &str) -> bool {
    let lower = base_url.to_lowercase();
    lower.contains("127.0.0.1") || lower.contains("localhost")
}

fn netease_base_url_port(base_url: &str) -> Option<u16> {
    let after_scheme = base_url.split("://").nth(1).unwrap_or(base_url);
    let host_port = after_scheme.split('/').next().unwrap_or(after_scheme);
    let port = host_port.rsplit_once(':')?.1;
    port.parse::<u16>().ok()
}

#[cfg(target_os = "windows")]
fn suppress_command_window(command: &mut Command) {
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(target_os = "windows"))]
fn suppress_command_window(_command: &mut Command) {}

#[cfg(debug_assertions)]
fn find_netease_api_entry() -> Option<PathBuf> {
    let mut roots = Vec::new();
    if let Ok(current_dir) = std::env::current_dir() {
        roots.push(current_dir.clone());
        if let Some(parent) = current_dir.parent() {
            roots.push(parent.to_path_buf());
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(exe_dir) = exe.parent() {
            roots.push(exe_dir.to_path_buf());
            if let Some(parent) = exe_dir.parent() {
                roots.push(parent.to_path_buf());
            }
            if let Some(grandparent) = exe_dir.parent().and_then(Path::parent) {
                roots.push(grandparent.to_path_buf());
            }
        }
    }

    roots.sort();
    roots.dedup();

    roots
        .into_iter()
        .flat_map(|root| {
            [
                root.join("node_modules")
                    .join("NeteaseCloudMusicApi")
                    .join("app.js"),
                root.join("..")
                    .join("node_modules")
                    .join("NeteaseCloudMusicApi")
                    .join("app.js"),
            ]
        })
        .find(|path| path.exists())
}

struct NeteaseJsonResponse {
    value: serde_json::Value,
    set_cookie: Option<String>,
}

async fn request_netease_json(
    config: &ResolvedNeteaseSourceConfig,
    path: &str,
    query: &[(&str, &str)],
) -> Result<serde_json::Value, String> {
    request_netease_json_response(config, path, query)
        .await
        .map(|response| response.value)
}

async fn request_netease_json_response(
    config: &ResolvedNeteaseSourceConfig,
    path: &str,
    query: &[(&str, &str)],
) -> Result<NeteaseJsonResponse, String> {
    if !config.enabled {
        return Err("NetEase Cloud Music is not enabled.".to_string());
    }

    // Try to start the local NeteaseCloudMusicApi service when pointing at a local URL.
    // Surface the preflight reason here so later request errors do not hide a missing
    // bundled runtime, a blocked port, or an unavailable external source.
    if is_local_netease_base_url(&config.base_url) {
        let service_status = ensure_local_netease_api_service(
            &config.base_url,
            config.managed_api.clone(),
            &config.managed_netease_child,
        )
        .await?;
        if !service_status.running {
            return Err(format!(
                "NetEase Cloud Music source is not ready. {}",
                service_status.message
            ));
        }
    }

    let endpoint = format!("{}{}", config.base_url.trim_end_matches('/'), path);
    let endpoint_for_error = endpoint.clone();
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| e.to_string())?;
    let mut query_items: Vec<(String, String)> = query
        .iter()
        .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
        .collect();
    if !query_items.iter().any(|(key, _)| key == "timestamp") {
        query_items.push(("timestamp".to_string(), current_timestamp_ms().to_string()));
    }
    if let Some(token) = config.token.as_deref() {
        query_items.push(("cookie".to_string(), token.to_string()));
    }
    let mut request = client.get(endpoint).query(&query_items);
    if let Some(token) = config.token.as_deref() {
        request = request.header(reqwest::header::COOKIE, token);
    }

    let response = request
        .send()
        .await
        .map_err(|error| {
            // reqwest 错误的 Display 可能包含完整 URL（含 cookie/password 等 query 参数），
            // 不能直接透传，只报告连接类型和端点路径。
            let error_kind = if error.is_connect() {
                "connection refused"
            } else if error.is_timeout() {
                "timeout"
            } else {
                "network error"
            };
            format!(
                "无法连接网易云 API 服务 ({endpoint_for_error})。 / Could not reach the NetEase API. {error_kind}"
            )
        })?;
    let status = response.status();
    let set_cookie = cookie_header_from_response(&response);
    let text = response.text().await.map_err(|error| error.to_string())?;
    let value = serde_json::from_str::<serde_json::Value>(&text)
        .map_err(|_| format!("NetEase Cloud Music responded with {status}."))?;

    if !status.is_success() {
        let message = json_text(value.get("message"))
            .or_else(|| json_text(value.get("msg")))
            .unwrap_or_else(|| format!("NetEase Cloud Music responded with {status}."));
        return Err(message);
    }

    Ok(NeteaseJsonResponse { value, set_cookie })
}

async fn complete_netease_session_from_response(
    state: &State<'_, AppState>,
    response: NeteaseJsonResponse,
) -> Result<NeteaseLoginStatusDto, String> {
    let value = response.value;
    let code = value
        .get("code")
        .and_then(|value| value.as_i64())
        .unwrap_or(0);
    if code != 200 && code != 803 {
        return Err(friendly_netease_login_error(&value));
    }

    let cookie = extract_netease_login_cookie(&value, response.set_cookie.as_deref())
        .ok_or_else(|| {
            "登录成功但未能获取会话凭证，请尝试扫码登录。 / Sign-in succeeded but no session credential was returned. Try scan login.".to_string()
        })?;
    let saved_cookie = save_and_verify_netease_session(&cookie)?;

    let mut refreshed_config = resolve_netease_source_config(state, None)?;
    refreshed_config.token = Some(saved_cookie);
    let status = fetch_netease_login_status(&refreshed_config).await?;
    if !status.logged_in {
        return Err(status.message);
    }
    Ok(status)
}

fn friendly_netease_login_error(value: &serde_json::Value) -> String {
    let code = value
        .get("code")
        .and_then(|value| value.as_i64())
        .unwrap_or(0);
    let message = json_text(value.get("message"))
        .or_else(|| json_text(value.get("msg")))
        .unwrap_or_default();
    let lower = message.to_ascii_lowercase();

    if lower.contains("captcha") || message.contains("验证码") {
        return "A verification code is required.".to_string();
    }
    if lower.contains("password") || message.contains("密码") || code == 502 {
        return "The account or password may be incorrect.".to_string();
    }
    if lower.contains("risk") || message.contains("安全") || message.contains("风控") {
        return "Security verification is required. Try secure web login.".to_string();
    }
    if !message.trim().is_empty() {
        return message;
    }
    "Could not sign in to this music source.".to_string()
}

fn extract_netease_login_cookie(
    value: &serde_json::Value,
    set_cookie: Option<&str>,
) -> Option<String> {
    let body_cookie = value
        .get("cookie")
        .and_then(|cookie| cookie.as_str())
        .or_else(|| {
            value
                .get("data")
                .and_then(|data| data.get("cookie"))
                .and_then(|cookie| cookie.as_str())
        });
    merge_cookie_headers(body_cookie, set_cookie)
}

fn open_url_with_system(url: &str) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        Command::new("rundll32.exe")
            .args(["url.dll,FileProtocolHandler", url])
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .map_err(|error| format!("Could not open secure login page. {error}"))?;
        return Ok(());
    }

    #[cfg(target_os = "macos")]
    {
        Command::new("open")
            .arg(url)
            .spawn()
            .map_err(|error| format!("Could not open secure login page. {error}"))?;
        return Ok(());
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        Command::new("xdg-open")
            .arg(url)
            .spawn()
            .map_err(|error| format!("Could not open secure login page. {error}"))?;
        return Ok(());
    }

    #[allow(unreachable_code)]
    Err("Secure web login is not supported on this system.".to_string())
}

async fn fetch_netease_playlist(
    config: &ResolvedNeteaseSourceConfig,
    playlist_id: &str,
) -> Result<SourcePlaylistDto, String> {
    let value = request_netease_json(config, "/playlist/detail", &[("id", playlist_id)]).await?;
    let playlist = value
        .get("playlist")
        .ok_or_else(|| "Playlist was not found.".to_string())?;
    let name = json_text(playlist.get("name"))
        .unwrap_or_else(|| format!("NetEase Playlist {playlist_id}"));
    let description = json_text(playlist.get("description")).unwrap_or_default();
    let tracks_json = playlist
        .get("tracks")
        .and_then(|tracks| tracks.as_array())
        .cloned()
        .unwrap_or_default();
    let mut tracks = Vec::new();

    for item in tracks_json.iter().take(500) {
        let song = source_song_from_playlist_json(item);
        tracks.push(song);
    }

    let _ = hydrate_netease_song_covers(config, &mut tracks).await;

    Ok(SourcePlaylistDto {
        id: playlist_id.to_string(),
        name,
        description,
        source: "netease".to_string(),
        tracks,
    })
}

async fn fetch_netease_liked_song_ids(
    config: &ResolvedNeteaseSourceConfig,
    user_id: &str,
    limit: u32,
) -> Result<Vec<String>, String> {
    let value = request_netease_json(config, "/likelist", &[("uid", user_id)]).await?;
    let ids = value
        .get("ids")
        .and_then(|ids| ids.as_array())
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|value| {
            value
                .as_i64()
                .map(|number| number.to_string())
                .or_else(|| value.as_u64().map(|number| number.to_string()))
                .or_else(|| value.as_str().map(ToString::to_string))
        })
        .take(limit as usize)
        .collect::<Vec<_>>();

    Ok(ids)
}

async fn fetch_netease_songs_by_ids(
    config: &ResolvedNeteaseSourceConfig,
    ids: &[String],
) -> Result<Vec<SourceSongDto>, String> {
    let mut songs = Vec::new();

    for chunk in ids.chunks(100) {
        if chunk.is_empty() {
            continue;
        }

        let ids_json = format!("[{}]", chunk.join(","));
        let value =
            request_netease_json(config, "/song/detail", &[("ids", ids_json.as_str())]).await?;
        let chunk_songs = value
            .get("songs")
            .and_then(|items| items.as_array())
            .cloned()
            .unwrap_or_default();

        songs.extend(chunk_songs.iter().map(source_song_from_playlist_json));
    }

    Ok(songs)
}

async fn hydrate_netease_song_covers(
    config: &ResolvedNeteaseSourceConfig,
    songs: &mut [SourceSongDto],
) -> Result<(), String> {
    let missing_ids = songs
        .iter()
        .filter(|song| song.cover_url.trim().is_empty() && !song.id.trim().is_empty())
        .map(|song| song.id.clone())
        .collect::<Vec<_>>();

    if missing_ids.is_empty() {
        return Ok(());
    }

    let details = fetch_netease_songs_by_ids(config, &missing_ids).await?;
    let covers_by_id = details
        .into_iter()
        .filter(|song| !song.cover_url.trim().is_empty())
        .map(|song| (song.id, song.cover_url))
        .collect::<HashMap<_, _>>();

    for song in songs.iter_mut() {
        if song.cover_url.trim().is_empty() {
            if let Some(cover_url) = covers_by_id.get(&song.id) {
                song.cover_url = cover_url.clone();
            }
        }
    }

    Ok(())
}

async fn fetch_netease_user_playlists(
    config: &ResolvedNeteaseSourceConfig,
    user_id: &str,
) -> Result<Vec<NeteaseUserPlaylistDto>, String> {
    let value = request_netease_json(
        config,
        "/user/playlist",
        &[("uid", user_id), ("limit", "1000")],
    )
    .await?;
    let playlists = value
        .get("playlist")
        .and_then(|items| items.as_array())
        .cloned()
        .unwrap_or_default();

    Ok(playlists
        .iter()
        .map(netease_user_playlist_from_json)
        .collect())
}

async fn fetch_netease_song_metadata(
    config: &ResolvedNeteaseSourceConfig,
    song_id: &str,
) -> Result<SourceSongDto, String> {
    let value = request_netease_json(config, "/song/detail", &[("ids", song_id)]).await?;
    let songs = value
        .get("songs")
        .and_then(|songs| songs.as_array())
        .ok_or_else(|| "Song metadata was not found.".to_string())?;
    let mut song = songs
        .first()
        .map(source_song_from_playlist_json)
        .ok_or_else(|| "Song metadata was not found.".to_string())?;
    match fetch_netease_playable_url(config, song_id).await {
        Ok(playable) => {
            song.playable_url = playable.url;
            song.unavailable = playable.unavailable;
            song.unavailable_reason = playable.reason;
        }
        Err(_) => {
            song.playable_url = None;
            song.unavailable = true;
            song.unavailable_reason = Some("api_failed".to_string());
        }
    }
    Ok(song)
}

async fn fetch_netease_playable_url(
    config: &ResolvedNeteaseSourceConfig,
    song_id: &str,
) -> Result<PlayableUrlDto, String> {
    fetch_netease_playable_url_with_level(config, song_id, None, None).await
}

async fn fetch_netease_playable_url_with_level(
    config: &ResolvedNeteaseSourceConfig,
    song_id: &str,
    requested_level: Option<&str>,
    proxy_state: Option<&AppState>,
) -> Result<PlayableUrlDto, String> {
    // P0 fix: `resolve_netease_source_config` may have read the keyring at a
    // transient-failure moment and returned `token: None`, even though the user
    // is actually signed in. Before concluding anything about login state,
    // re-read the keyring once more here. This closes the gap where the
    // settings page showed "Signed in" (snapshot taken earlier) but playback
    // said "Sign in needed" (keyring read failed at this moment).
    let effective_token = config.token.clone().or_else(read_netease_token);
    let has_token = effective_token.is_some();
    // Build a local config with the recovered token so downstream requests
    // actually carry the cookie.
    let effective_config = ResolvedNeteaseSourceConfig {
        token: effective_token,
        ..config.clone()
    };
    let config = &effective_config;

    let requested_level = requested_level
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(if has_token { "hires" } else { "standard" });
    let login_status = fetch_netease_login_status(config).await.ok();
    // Distinguish "login status known" from "login status unknown (API failed)".
    // When the status API is unreachable we must NOT pretend the cookie is a valid
    // session: surface a cookie_expired reason so the user can re-sign-in, and still
    // attempt playback with the cookie we have (NetEase sometimes serves playable
    // URLs even when /login/status is briefly unavailable).
    let login_status_known = login_status.is_some();
    let is_logged_in = login_status
        .as_ref()
        .map(|status| status.logged_in && !status.expired)
        .unwrap_or(false);
    let user_profile = login_status;
    let vip_status = fetch_netease_vip_status(config).await.ok();
    let is_member = vip_status.as_ref().is_some_and(|status| status.is_member);
    let mut levels = if has_token {
        ordered_netease_levels(requested_level)
    } else {
        vec!["standard".to_string()]
    };
    levels.dedup();

    let mut attempts = Vec::new();
    let mut final_debug = default_netease_playback_debug(
        config,
        song_id,
        requested_level,
        is_logged_in,
        user_profile
            .as_ref()
            .and_then(|status| status.user_id.clone()),
        vip_status.as_ref().map(|status| {
            if status.is_member {
                status.level.clone().unwrap_or_else(|| "active".to_string())
            } else {
                "inactive".to_string()
            }
        }),
    );

    for level in levels {
        let endpoint = "/song/url/v1";
        let value = match request_netease_json(
            config,
            endpoint,
            &[("id", song_id), ("level", level.as_str())],
        )
        .await
        {
            Ok(value) => value,
            Err(error) => {
                let attempt = NeteasePlaybackAttemptDto {
                    level: level.clone(),
                    endpoint: endpoint.to_string(),
                    response_code: None,
                    has_url: false,
                    returned_level: None,
                    reason: Some("api_failed".to_string()),
                    message: Some(error),
                };
                attempts.push(attempt);
                continue;
            }
        };

        let item = first_playable_item(&value);
        let url = playback_url_from_item(&item);
        let trial_only = is_netease_trial_only(&item, url.is_some(), is_member);
        let response_code = item
            .get("code")
            .or_else(|| value.get("code"))
            .and_then(|value| value.as_i64());
        let returned_level = json_text(item.get("level"));
        let fee = item.get("fee").and_then(|value| value.as_i64());
        let privilege = json_text(item.get("type")).or_else(|| json_text(item.get("encodeType")));
        let message = netease_playback_message(&value, &item);
        let reason = if url.is_none() || trial_only {
            Some(if trial_only {
                classify_netease_trial_reason(config, is_logged_in, is_member, fee)
            } else {
                classify_netease_unavailable_reason(config, &value, &item)
            })
        } else {
            None
        };

        let attempt = NeteasePlaybackAttemptDto {
            level: level.clone(),
            endpoint: endpoint.to_string(),
            response_code,
            has_url: url.is_some() && !trial_only,
            returned_level: returned_level.clone(),
            reason: reason.clone(),
            message: message.clone(),
        };
        attempts.push(attempt);

        final_debug.endpoint = endpoint.to_string();
        final_debug.response_code = response_code;
        final_debug.has_url = url.is_some() && !trial_only;
        final_debug.returned_level = returned_level;
        final_debug.fee = fee;
        final_debug.privilege = privilege;
        final_debug.reason = reason.clone();
        final_debug.message = message;

        if let Some(url) = url {
            if !trial_only {
                // NetEase audio CDNs are commonly served over plain http://
                // (m*.music.126.net), which the WebView CSP `media-src` blocks.
                // Route every successful URL through the ome-media:// proxy so the
                // audio element can load it, and so we can attach the right
                // headers/Referer for the NetEase CDN.
                let proxied_url = match proxy_state {
                    Some(state) => {
                        register_media_proxy(state, &url, "audio").unwrap_or_else(|_| url.clone())
                    }
                    None => url.clone(),
                };
                final_debug.attempts = attempts;
                final_debug.has_url = true;
                final_debug.reason = None;
                return Ok(PlayableUrlDto {
                    song_id: song_id.to_string(),
                    unavailable: false,
                    reason: None,
                    url: Some(proxied_url),
                    video_url: None,
                    debug: Some(final_debug),
                    audio_candidates: Vec::new(),
                    video_candidates: Vec::new(),
                });
            }
            // 试听片段不返回，继续尝试更低音质 level
        }
    }

    if final_debug.reason.is_none() {
        // Order matters: not_logged_in (no cookie at all) → cookie_expired
        // (cookie present but status API confirms it is no longer a session) →
        // url_null (we have a cookie and could not determine session state, but
        // the API still did not return a playable URL).
        final_debug.reason = Some(
            if !has_token {
                "not_logged_in"
            } else if login_status_known && !is_logged_in {
                "cookie_expired"
            } else {
                "url_null"
            }
            .to_string(),
        );
    }
    final_debug.attempts = attempts;

    Ok(PlayableUrlDto {
        song_id: song_id.to_string(),
        unavailable: true,
        reason: final_debug.reason.clone(),
        url: None,
        video_url: None,
        debug: Some(final_debug),
        audio_candidates: Vec::new(),
        video_candidates: Vec::new(),
    })
}

async fn fetch_netease_login_status(
    config: &ResolvedNeteaseSourceConfig,
) -> Result<NeteaseLoginStatusDto, String> {
    let Some(initial_token) = config.token.clone() else {
        return Ok(NeteaseLoginStatusDto {
            logged_in: false,
            expired: false,
            login_status_known: true,
            has_cookie: false,
            masked_cookie: String::new(),
            nickname: None,
            user_id: None,
            avatar_url: None,
            message: "Sign in to your music source to try again.".to_string(),
        });
    };

    let mut current_config = config.clone();
    let mut current_token = initial_token;
    let mut last_error: Option<String> = None;
    let mut status_known = false;

    for endpoint in ["/login/status", "/user/account"] {
        match request_netease_json_response(&current_config, endpoint, &[]).await {
            Ok(response) => {
                status_known = true;
                if let Some(updated_cookie) = persist_merged_netease_cookie(
                    Some(&current_token),
                    response.set_cookie.as_deref(),
                )? {
                    current_token = updated_cookie;
                    current_config.token = Some(current_token.clone());
                }

                let value = response.value;
                let data = value.get("data").unwrap_or(&value);
                let profile = data.get("profile").or_else(|| value.get("profile"));
                let account = data.get("account").or_else(|| value.get("account"));
                let nickname = profile
                    .and_then(|profile| profile.get("nickname"))
                    .and_then(|value| value.as_str())
                    .map(ToString::to_string);
                let user_id_value = account
                    .and_then(|account| account.get("id"))
                    .or_else(|| profile.and_then(|profile| profile.get("userId")));
                let user_id = Some(json_id(user_id_value)).filter(|value| !value.is_empty());
                let avatar_url = profile
                    .and_then(|profile| profile.get("avatarUrl"))
                    .and_then(|value| value.as_str())
                    .map(ToString::to_string);
                let logged_in = nickname.is_some() || user_id.is_some();

                if logged_in {
                    let has_cookie = cookie_has_music_u(&current_token);
                    return Ok(NeteaseLoginStatusDto {
                        logged_in: true,
                        expired: false,
                        login_status_known: true,
                        has_cookie,
                        masked_cookie: mask_netease_cookie(&current_token),
                        nickname,
                        user_id,
                        avatar_url,
                        message: "Connected.".to_string(),
                    });
                }

                last_error =
                    json_text(value.get("message")).or_else(|| json_text(value.get("msg")));
            }
            Err(error) => {
                last_error = Some(error);
            }
        }
    }

    Ok(NeteaseLoginStatusDto {
        logged_in: false,
        expired: status_known,
        login_status_known: status_known,
        has_cookie: cookie_has_music_u(&current_token),
        masked_cookie: mask_netease_cookie(&current_token),
        nickname: None,
        user_id: None,
        avatar_url: None,
        message: last_error.unwrap_or_else(|| {
            if status_known {
                "Your session has expired. Please reconnect NetEase Cloud Music.".to_string()
            } else {
                "Could not verify the NetEase session. Please try again in a moment.".to_string()
            }
        }),
    })
}

fn default_netease_playback_debug(
    config: &ResolvedNeteaseSourceConfig,
    song_id: &str,
    requested_level: &str,
    is_logged_in: bool,
    user_id: Option<String>,
    vip_status: Option<String>,
) -> NeteasePlaybackDebugDto {
    NeteasePlaybackDebugDto {
        is_logged_in,
        has_cookie: config.token.is_some(),
        masked_cookie: config
            .token
            .as_deref()
            .map(mask_netease_cookie)
            .unwrap_or_default(),
        user_id,
        vip_status,
        requested_song_id: song_id.to_string(),
        requested_level: requested_level.to_string(),
        endpoint: "/song/url/v1".to_string(),
        response_code: None,
        has_url: false,
        returned_level: None,
        fee: None,
        privilege: None,
        reason: None,
        message: None,
        attempts: Vec::new(),
    }
}

fn ordered_netease_levels(requested_level: &str) -> Vec<String> {
    const ORDER: [&str; 5] = ["hires", "lossless", "exhigh", "higher", "standard"];
    let requested = if ORDER.contains(&requested_level) {
        requested_level
    } else {
        "hires"
    };
    let mut levels = vec![requested.to_string()];
    if let Some(index) = ORDER.iter().position(|level| *level == requested) {
        for level in ORDER.iter().skip(index + 1) {
            levels.push((*level).to_string());
        }
    }
    if !levels.iter().any(|level| level == "standard") {
        levels.push("standard".to_string());
    }
    levels
}

fn first_playable_item(value: &serde_json::Value) -> serde_json::Value {
    value
        .get("data")
        .and_then(|data| data.as_array())
        .and_then(|items| items.first())
        .cloned()
        .unwrap_or_default()
}

fn playback_url_from_item(item: &serde_json::Value) -> Option<String> {
    item.get("url")
        .and_then(|url| url.as_str())
        .map(str::trim)
        .filter(|url| !url.is_empty())
        .map(ToString::to_string)
}

fn is_netease_trial_only(
    item: &serde_json::Value,
    has_playable_url: bool,
    is_member: bool,
) -> bool {
    if has_playable_url && is_member {
        return false;
    }

    let has_trial_info = item
        .get("freeTrialInfo")
        .is_some_and(|value| !value.is_null());
    let free_trial_privilege = item.get("freeTrialPrivilege");
    let listen_type = free_trial_privilege
        .and_then(|value| value.get("listenType"))
        .and_then(|value| value.as_i64())
        .unwrap_or(0);
    has_trial_info || listen_type > 0
}

fn netease_playback_message(value: &serde_json::Value, item: &serde_json::Value) -> Option<String> {
    json_text(value.get("message"))
        .or_else(|| json_text(value.get("msg")))
        .or_else(|| json_text(item.get("msg")))
}

fn classify_netease_trial_reason(
    config: &ResolvedNeteaseSourceConfig,
    is_logged_in: bool,
    is_member: bool,
    fee: Option<i64>,
) -> String {
    if config.token.is_none() {
        return "not_logged_in".to_string();
    }
    if !is_logged_in {
        return "cookie_expired".to_string();
    }
    if matches!(fee, Some(1) | Some(4)) && !is_member {
        return "vip_required".to_string();
    }
    "trial_only".to_string()
}

fn mask_netease_cookie(cookie: &str) -> String {
    let music_u = cookie
        .split(';')
        .find_map(|part| {
            let part = part.trim();
            part.strip_prefix("MUSIC_U=").map(str::trim)
        })
        .filter(|value| !value.is_empty());

    match music_u {
        Some(value) if value.len() > 8 => {
            format!("MUSIC_U={}****{}", &value[..4], &value[value.len() - 4..])
        }
        Some(_) => "MUSIC_U=****".to_string(),
        None if cookie.len() > 8 => format!("{}****{}", &cookie[..4], &cookie[cookie.len() - 4..]),
        None => "****".to_string(),
    }
}

async fn fetch_netease_vip_status(
    config: &ResolvedNeteaseSourceConfig,
) -> Result<NeteaseVipStatusDto, String> {
    let has_token = config.token.is_some();
    if !has_token {
        return Ok(NeteaseVipStatusDto {
            is_member: false,
            level: None,
            message: "Sign in to view membership status.".to_string(),
            // Known: the user is not signed in, so there is no membership to
            // report. This is NOT an "unknown" state.
            membership_known: true,
        });
    }

    let value = match request_netease_json(config, "/vip/info", &[]).await {
        Ok(value) => Ok(value),
        Err(_) => request_netease_json(config, "/vip/info/v2", &[]).await,
    };

    match value {
        Ok(value) => {
            let data = value.get("data").unwrap_or(&value);
            let associator = data.get("associator").or_else(|| data.get("data"));
            let vip_type = data
                .get("vipType")
                .or_else(|| data.get("redVipLevel"))
                .or_else(|| associator.and_then(|a| a.get("vipType")))
                .or_else(|| associator.and_then(|a| a.get("redVipLevel")))
                .and_then(|value| value.as_i64())
                .unwrap_or(0);
            let is_member = data
                .get("isMember")
                .and_then(|value| value.as_bool())
                .or_else(|| {
                    associator
                        .and_then(|a| a.get("isMember"))
                        .and_then(|v| v.as_bool())
                })
                .unwrap_or(vip_type > 0);
            Ok(NeteaseVipStatusDto {
                is_member,
                level: if vip_type > 0 {
                    Some(vip_type.to_string())
                } else {
                    None
                },
                message: if is_member {
                    "Membership is active.".to_string()
                } else {
                    "No active membership found for this session.".to_string()
                },
                // The API answered, so the membership verdict is authoritative.
                membership_known: true,
            })
        }
        Err(_) => Ok(NeteaseVipStatusDto {
            is_member: false,
            level: None,
            message: "Membership status is unavailable right now.".to_string(),
            // Both /vip/info and /vip/info/v2 failed — we cannot tell whether
            // the account is a member, so the UI must NOT label it "Non-member".
            membership_known: false,
        }),
    }
}

fn classify_netease_unavailable_reason(
    config: &ResolvedNeteaseSourceConfig,
    value: &serde_json::Value,
    item: &serde_json::Value,
) -> String {
    let message = json_text(value.get("message"))
        .or_else(|| json_text(value.get("msg")))
        .or_else(|| json_text(item.get("msg")))
        .unwrap_or_default()
        .to_lowercase();
    let code = item
        .get("code")
        .or_else(|| value.get("code"))
        .and_then(|value| value.as_i64())
        .unwrap_or(0);
    let fee = item
        .get("fee")
        .and_then(|value| value.as_i64())
        .unwrap_or(0);
    let mentions_login = message.contains("login")
        || message.contains("cookie")
        || message.contains("\u{767b}\u{5f55}");
    let mentions_vip = message.contains("vip") || message.contains("\u{4f1a}\u{5458}");
    let mentions_expired = message.contains("expired")
        || message.contains("session")
        || message.contains("\u{5931}\u{6548}");
    let mentions_copyright = message.contains("copyright") || message.contains("\u{7248}\u{6743}");
    let mentions_region = message.contains("region") || message.contains("\u{5730}\u{533a}");
    let mentions_removed = message.contains("removed") || message.contains("\u{4e0b}\u{67b6}");

    if config.token.is_none() && (mentions_vip || mentions_login || fee == 1 || fee == 4) {
        return "not_logged_in".to_string();
    }
    if mentions_expired || (config.token.is_some() && mentions_login) || code == 301 {
        return "cookie_expired".to_string();
    }
    if mentions_vip || fee == 1 || fee == 4 {
        return "vip_required".to_string();
    }
    if mentions_copyright || code == -110 {
        return "no_copyright".to_string();
    }
    if mentions_region || code == 403 {
        return "region_restricted".to_string();
    }
    if mentions_removed || code == 404 {
        return "song_removed".to_string();
    }

    if config.token.is_none()
        && (message.contains("vip")
            || message.contains("login")
            || message.contains("cookie")
            || message.contains("会员")
            || message.contains("登录")
            || fee == 1
            || fee == 4)
    {
        return "not_logged_in".to_string();
    }
    if message.contains("expired")
        || message.contains("session")
        || message.contains("失效")
        || (config.token.is_some()
            && (message.contains("login")
                || message.contains("cookie")
                || message.contains("登录")))
        || code == 301
    {
        return "cookie_expired".to_string();
    }
    if message.contains("vip") || message.contains("会员") || fee == 1 || fee == 4 {
        return "vip_required".to_string();
    }
    if message.contains("copyright") || message.contains("版权") || code == -110 {
        return "no_copyright".to_string();
    }
    if message.contains("region") || message.contains("地区") || code == 403 {
        return "region_restricted".to_string();
    }
    if message.contains("removed") || message.contains("下架") || code == 404 {
        return "song_removed".to_string();
    }
    if code != 200 && code != 0 {
        return "api_failed".to_string();
    }
    "url_null".to_string()
}

fn lyric_cache_key(source: &str, source_id: &str, lyric_type: &str) -> String {
    format!("{source}:{source_id}:{lyric_type}")
}

fn lyric_offset_key(source: &str, source_id: &str) -> String {
    format!("{source}:{source_id}")
}

fn lyric_offset_key_from_cache_key(cache_key: &str) -> String {
    let mut parts = cache_key.splitn(3, ':');
    match (parts.next(), parts.next()) {
        (Some(source), Some(source_id)) => lyric_offset_key(source, source_id),
        _ => cache_key.to_string(),
    }
}

fn infer_track_source(track: &TrackDto) -> String {
    if track.file_path.starts_with("unavailable:netease:") {
        "netease".to_string()
    } else {
        "local".to_string()
    }
}

fn infer_track_source_id(track: &TrackDto) -> Option<String> {
    track
        .file_path
        .strip_prefix("unavailable:netease:")
        .map(ToString::to_string)
        .or_else(|| {
            if track.source == "local" || !track.file_path.starts_with("http") {
                Some(stable_id(&track.file_path))
            } else {
                None
            }
        })
}

fn load_cached_lyrics(
    state: &State<'_, AppState>,
    cache_key: &str,
) -> Result<Option<ResolvedLyricsDto>, String> {
    let db = state.db.lock().map_err(|error| error.to_string())?;
    db.query_row(
        "SELECT
           lyrics_cache.cache_key,
           lyrics_cache.source,
           lyrics_cache.lyrics,
           lyrics_cache.translated_lyrics,
           lyrics_cache.confidence,
           lyrics_cache.warning,
           COALESCE(lyric_offsets.offset_ms, 0)
         FROM lyrics_cache
         LEFT JOIN lyric_offsets ON lyric_offsets.cache_key = lyrics_cache.cache_key
         WHERE lyrics_cache.cache_key = ?1",
        params![cache_key],
        |row| {
            Ok(ResolvedLyricsDto {
                cache_key: row.get(0)?,
                source: row.get(1)?,
                lyrics: row.get(2)?,
                translated_lyrics: row.get(3)?,
                confidence: row.get(4)?,
                warning: row.get(5)?,
                offset_ms: row.get(6)?,
            })
        },
    )
    .optional()
    .map_err(|error| error.to_string())
}

fn save_cached_lyrics(
    state: &State<'_, AppState>,
    resolved: &ResolvedLyricsDto,
) -> Result<(), String> {
    let mut parts = resolved.cache_key.splitn(3, ':');
    let source = parts.next().unwrap_or(&resolved.source);
    let source_id = parts.next().unwrap_or("");
    let lyric_type = parts.next().unwrap_or("lrc");
    let db = state.db.lock().map_err(|error| error.to_string())?;
    db.execute(
        "INSERT INTO lyrics_cache (
            cache_key, source, source_id, lyric_type, lyrics, translated_lyrics, confidence, warning, updated_at
         )
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, CURRENT_TIMESTAMP)
         ON CONFLICT(cache_key) DO UPDATE SET
            lyrics = excluded.lyrics,
            translated_lyrics = excluded.translated_lyrics,
            confidence = excluded.confidence,
            warning = excluded.warning,
            updated_at = CURRENT_TIMESTAMP",
        params![
            &resolved.cache_key,
            source,
            source_id,
            lyric_type,
            &resolved.lyrics,
            &resolved.translated_lyrics,
            resolved.confidence,
            resolved.warning.as_deref(),
        ],
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

fn with_saved_lyric_offset(
    state: &State<'_, AppState>,
    mut resolved: ResolvedLyricsDto,
) -> Result<ResolvedLyricsDto, String> {
    let db = state.db.lock().map_err(|error| error.to_string())?;
    resolved.offset_ms = db
        .query_row(
            "SELECT offset_ms FROM lyric_offsets WHERE cache_key = ?1",
            params![lyric_offset_key_from_cache_key(&resolved.cache_key)],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())?
        .unwrap_or(0);
    Ok(resolved)
}

fn resolve_local_track_lyrics(
    track: TrackDto,
    source: String,
    source_id: String,
    cache_key: String,
) -> Result<ResolvedLyricsDto, String> {
    if let Some(lyrics) = read_embedded_lrc_lyrics(&track.file_path) {
        return Ok(ResolvedLyricsDto {
            cache_key,
            source,
            lyrics,
            translated_lyrics: String::new(),
            confidence: 0.98,
            warning: None,
            offset_ms: 0,
        });
    }

    if let Some(path) = sidecar_lrc_path(&track.file_path) {
        if let Ok(lyrics) = fs::read_to_string(&path) {
            if !lyrics.trim().is_empty() {
                return Ok(ResolvedLyricsDto {
                    cache_key,
                    source,
                    lyrics,
                    translated_lyrics: String::new(),
                    confidence: 0.94,
                    warning: None,
                    offset_ms: 0,
                });
            }
        }
    }

    if let Some((lyrics, confidence)) = find_fuzzy_lrc_for_track(&track) {
        return Ok(ResolvedLyricsDto {
            cache_key,
            source,
            lyrics,
            translated_lyrics: String::new(),
            confidence,
            warning: Some("Lyrics may not match this version.".to_string()),
            offset_ms: 0,
        });
    }

    Ok(ResolvedLyricsDto {
        cache_key: lyric_cache_key(&source, &source_id, "lrc"),
        source,
        lyrics: String::new(),
        translated_lyrics: String::new(),
        confidence: 0.0,
        warning: Some("No matched lyrics for this version.".to_string()),
        offset_ms: 0,
    })
}

fn read_embedded_lrc_lyrics(file_path: &str) -> Option<String> {
    if file_path.starts_with("http") || file_path.starts_with("unavailable:") {
        return None;
    }
    let tagged_file = Probe::open(file_path).ok()?.read().ok()?;
    let tag = tagged_file
        .primary_tag()
        .or_else(|| tagged_file.first_tag())?;
    let lyrics = tag
        .get_string(ItemKey::Lyrics)
        .or_else(|| tag.get_string(ItemKey::UnsyncLyrics))?
        .trim()
        .to_string();
    if lyrics.is_empty() || !looks_like_lrc(&lyrics) {
        return None;
    }
    Some(lyrics)
}

fn find_fuzzy_lrc_for_track(track: &TrackDto) -> Option<(String, f64)> {
    if track.file_path.starts_with("http") || track.file_path.starts_with("unavailable:") {
        return None;
    }
    let path = Path::new(&track.file_path);
    let parent = path.parent()?;
    let mut best: Option<(PathBuf, f64)> = None;

    for entry in fs::read_dir(parent).ok()?.flatten() {
        let candidate = entry.path();
        let is_lrc = candidate
            .extension()
            .and_then(|value| value.to_str())
            .map(|value| value.eq_ignore_ascii_case("lrc"))
            .unwrap_or(false);
        if !is_lrc {
            continue;
        }
        let stem = candidate
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or_default();
        let score = lyric_match_confidence(stem, track);
        if score >= 0.68
            && best
                .as_ref()
                .map(|(_, best_score)| score > *best_score)
                .unwrap_or(true)
        {
            best = Some((candidate, score));
        }
    }

    let (path, score) = best?;
    let lyrics = fs::read_to_string(path).ok()?;
    if lyrics.trim().is_empty() || !looks_like_lrc(&lyrics) {
        return None;
    }
    Some((lyrics, score))
}

fn looks_like_lrc(value: &str) -> bool {
    value.contains(":[") || value.contains("[00:") || value.contains("[0:")
}

fn lyric_match_confidence(candidate_name: &str, track: &TrackDto) -> f64 {
    let candidate = normalize_match_text(candidate_name);
    let title = normalize_match_text(&track.title);
    let artist = normalize_match_text(&track.artist);
    let album = normalize_match_text(&track.album);

    let title_score = token_overlap_score(&candidate, &title);
    let artist_score = token_overlap_score(&candidate, &artist);
    let album_score = token_overlap_score(&candidate, &album);
    (title_score * 0.72 + artist_score * 0.2 + album_score * 0.08).min(1.0)
}

fn normalize_match_text(value: &str) -> String {
    value
        .to_lowercase()
        .chars()
        .map(|ch| {
            if ch.is_alphanumeric() || ch.is_whitespace() {
                ch
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn token_overlap_score(candidate: &str, expected: &str) -> f64 {
    if expected.is_empty() || candidate.is_empty() {
        return 0.0;
    }
    if candidate == expected || candidate.contains(expected) || expected.contains(candidate) {
        return 1.0;
    }
    let expected_tokens: HashSet<&str> = expected.split_whitespace().collect();
    if expected_tokens.is_empty() {
        return 0.0;
    }
    let candidate_tokens: HashSet<&str> = candidate.split_whitespace().collect();
    let overlap = expected_tokens.intersection(&candidate_tokens).count();
    overlap as f64 / expected_tokens.len() as f64
}

fn sidecar_lrc_path(file_path: &str) -> Option<PathBuf> {
    if file_path.starts_with("http") || file_path.starts_with("unavailable:") {
        return None;
    }
    let path = Path::new(file_path);
    let with_lrc = path.with_extension("lrc");
    if with_lrc.exists() {
        return Some(with_lrc);
    }
    None
}

fn import_source_playlist_to_db(
    db: &Connection,
    playlist: &SourcePlaylistDto,
) -> Result<(), String> {
    let playlist_db_id = stable_id(&format!("source:{}:{}", playlist.source, playlist.id));

    db.execute(
        "INSERT INTO playlists (id, name, description, source, created_at, updated_at)
         VALUES (?1, ?2, ?3, 'imported', CURRENT_TIMESTAMP, CURRENT_TIMESTAMP)
         ON CONFLICT(id) DO UPDATE SET
           name = excluded.name,
           description = excluded.description,
           updated_at = CURRENT_TIMESTAMP",
        params![&playlist_db_id, &playlist.name, &playlist.description],
    )
    .map_err(|error| error.to_string())?;

    db.execute(
        "DELETE FROM playlist_tracks WHERE playlist_id = ?1",
        params![&playlist_db_id],
    )
    .map_err(|error| error.to_string())?;

    for (index, song) in playlist.tracks.iter().enumerate() {
        let track = parsed_track_from_source_song(song);
        upsert_track(db, &track)?;
        db.execute(
            "INSERT OR REPLACE INTO playlist_tracks (playlist_id, track_id, position, added_at)
             VALUES (?1, ?2, ?3, CURRENT_TIMESTAMP)",
            params![&playlist_db_id, &track.id, index as i64],
        )
        .map_err(|error| error.to_string())?;
    }

    Ok(())
}

fn parsed_track_from_source_song(song: &SourceSongDto) -> ParsedTrack {
    let source = song.source.as_deref().unwrap_or("netease");
    let source_id = if source == "bilibili" {
        song.bvid
            .as_ref()
            .zip(song.cid.as_ref())
            .map(|(bvid, cid)| format!("{bvid}:{cid}"))
            .unwrap_or_else(|| song.id.clone())
    } else {
        song.id.clone()
    };
    ParsedTrack {
        id: stable_id(&format!("{source}:{source_id}")),
        title: song.title.clone(),
        artist: song.artist.clone(),
        album: song.album.clone(),
        duration_seconds: song.duration_seconds,
        file_path: song
            .playable_url
            .clone()
            .unwrap_or_else(|| format!("unavailable:{source}:{source_id}")),
        file_extension: source.to_string(),
        source: source.to_string(),
        source_id: Some(source_id),
        unavailable_reason: song.unavailable_reason.clone(),
        cover_url: if song.cover_url.is_empty() {
            None
        } else {
            Some(song.cover_url.clone())
        },
        genres: Vec::new(),
        moods: Vec::new(),
        calm_score: 0.45,
        energetic_score: 0.45,
        year: None,
    }
}

fn source_song_from_search_json(value: &serde_json::Value) -> SourceSongDto {
    let id = json_id(value.get("id"));
    let title = json_text(value.get("name")).unwrap_or_else(|| "Unknown Song".to_string());
    let artist = value
        .get("artists")
        .or_else(|| value.get("ar"))
        .and_then(|artists| artists.as_array())
        .and_then(|artists| artists.first())
        .and_then(|artist| artist.get("name"))
        .and_then(|name| name.as_str())
        .unwrap_or("Unknown Artist")
        .to_string();
    let album_value = value.get("album").or_else(|| value.get("al"));
    let album = album_value
        .and_then(|album| album.get("name"))
        .and_then(|name| name.as_str())
        .unwrap_or("Unknown Album")
        .to_string();
    let cover_url = netease_cover_url_from_song_json(value);
    let duration_seconds = value
        .get("duration")
        .or_else(|| value.get("dt"))
        .and_then(|duration| duration.as_u64())
        .unwrap_or(0)
        / 1000;

    SourceSongDto {
        id,
        source: Some("netease".to_string()),
        title,
        artist,
        album,
        duration_seconds,
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
        source_url: None,
    }
}

fn source_song_from_playlist_json(value: &serde_json::Value) -> SourceSongDto {
    let id = json_id(value.get("id"));
    let title = json_text(value.get("name")).unwrap_or_else(|| "Unknown Song".to_string());
    let artist = value
        .get("ar")
        .or_else(|| value.get("artists"))
        .and_then(|artists| artists.as_array())
        .and_then(|artists| artists.first())
        .and_then(|artist| artist.get("name"))
        .and_then(|name| name.as_str())
        .unwrap_or("Unknown Artist")
        .to_string();
    let album_value = value.get("al").or_else(|| value.get("album"));
    let album = album_value
        .and_then(|album| album.get("name"))
        .and_then(|name| name.as_str())
        .unwrap_or("Unknown Album")
        .to_string();
    let cover_url = netease_cover_url_from_song_json(value);
    let duration_seconds = value
        .get("dt")
        .or_else(|| value.get("duration"))
        .and_then(|duration| duration.as_u64())
        .unwrap_or(0)
        / 1000;

    SourceSongDto {
        id,
        source: Some("netease".to_string()),
        title,
        artist,
        album,
        duration_seconds,
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
        source_url: None,
    }
}

fn netease_cover_url_from_song_json(value: &serde_json::Value) -> String {
    let album_value = value.get("al").or_else(|| value.get("album"));
    let candidates = [
        album_value.and_then(|album| album.get("picUrl")),
        album_value.and_then(|album| album.get("blurPicUrl")),
        value.get("picUrl"),
        value.get("coverUrl"),
        value.get("cover"),
    ];

    candidates
        .into_iter()
        .filter_map(json_text)
        .map(normalize_netease_image_url)
        .find(|url| !url.trim().is_empty())
        .unwrap_or_default()
}

fn netease_user_playlist_from_json(value: &serde_json::Value) -> NeteaseUserPlaylistDto {
    let id = json_id(value.get("id"));
    let name = json_text(value.get("name")).unwrap_or_else(|| "Untitled Playlist".to_string());
    let creator_name = value
        .get("creator")
        .and_then(|creator| creator.get("nickname"))
        .and_then(|name| name.as_str())
        .unwrap_or("")
        .to_string();
    let track_count = value
        .get("trackCount")
        .or_else(|| value.get("track_count"))
        .and_then(|count| count.as_u64())
        .unwrap_or(0) as u32;
    let cover_url = value
        .get("coverImgUrl")
        .or_else(|| value.get("coverUrl"))
        .and_then(|url| url.as_str())
        .unwrap_or("")
        .to_string();
    let description = json_text(value.get("description")).unwrap_or_default();
    let subscribed = value
        .get("subscribed")
        .and_then(|subscribed| subscribed.as_bool())
        .unwrap_or(false);

    NeteaseUserPlaylistDto {
        id,
        name,
        track_count,
        creator_name,
        subscribed,
        cover_url,
        description,
    }
}

fn json_id(value: Option<&serde_json::Value>) -> String {
    value
        .and_then(|value| {
            value
                .as_str()
                .map(ToString::to_string)
                .or_else(|| value.as_i64().map(|number| number.to_string()))
                .or_else(|| value.as_u64().map(|number| number.to_string()))
        })
        .unwrap_or_default()
}

fn json_text(value: Option<&serde_json::Value>) -> Option<String> {
    value
        .and_then(|value| value.as_str())
        .map(ToString::to_string)
}

fn openai_compatible_completion_endpoint(base_url: &str) -> String {
    let trimmed = base_url.trim().trim_end_matches('/');

    if trimmed.ends_with("/chat/completions") {
        trimmed.to_string()
    } else {
        format!("{trimmed}/chat/completions")
    }
}

fn openai_compatible_models_endpoint(base_url: &str) -> String {
    let trimmed = base_url.trim().trim_end_matches('/');

    if trimmed.ends_with("/models") {
        trimmed.to_string()
    } else {
        format!("{trimmed}/models")
    }
}

fn openai_compatible_transcription_endpoint(base_url: &str) -> String {
    let trimmed = base_url.trim().trim_end_matches('/');

    if trimmed.ends_with("/audio/transcriptions") {
        trimmed.to_string()
    } else {
        format!("{trimmed}/audio/transcriptions")
    }
}

fn openai_compatible_speech_endpoint(base_url: &str) -> String {
    let trimmed = base_url.trim().trim_end_matches('/');

    if trimmed.ends_with("/audio/speech") {
        trimmed.to_string()
    } else {
        format!("{trimmed}/audio/speech")
    }
}

fn decode_base64_payload(value: &str) -> Result<Vec<u8>, String> {
    let payload = value
        .split_once(',')
        .map(|(_, data)| data)
        .unwrap_or(value)
        .trim();
    general_purpose::STANDARD
        .decode(payload)
        .map_err(|error| format!("Audio payload could not be read. {error}"))
}

fn audio_extension_from_mime(mime_type: &str) -> &'static str {
    match mime_type.split(';').next().unwrap_or("").trim() {
        "audio/mpeg" | "audio/mp3" => "mp3",
        "audio/mp4" | "audio/x-m4a" => "m4a",
        "audio/wav" | "audio/wave" => "wav",
        "audio/ogg" => "ogg",
        _ => "webm",
    }
}

fn insert_llm_request_audit(
    db: &Connection,
    provider: &str,
    purpose: &str,
    response_summary: &str,
) -> Result<(), String> {
    let normalized_purpose = match purpose {
        "playlist_analysis" => "playlist_analysis",
        "mood_recommendation" => "mood_recommendation",
        _ => "scene_recommendation",
    };
    let summary = response_summary.chars().take(300).collect::<String>();

    db.execute(
        "INSERT INTO ai_requests (id, provider, purpose, metadata_sent_json, response_summary, created_at)
         VALUES (?1, ?2, ?3, '{}', ?4, CURRENT_TIMESTAMP)",
        params![stable_id(&format!("llm:{}:{}:{}", provider, normalized_purpose, now_timestamp())), provider, normalized_purpose, summary],
    )
    .map_err(|error| error.to_string())?;

    Ok(())
}

fn insert_playlist_analysis_run(
    db: &Connection,
    payload: SavePlaylistAnalysisPayload,
) -> Result<PlaylistAnalysisRunDto, String> {
    let mode = match payload.mode.as_str() {
        "layered" => "layered",
        _ => "direct",
    };
    let id = stable_id(&format!(
        "playlist-analysis:{}:{}:{}",
        payload.playlist_id,
        mode,
        now_timestamp()
    ));

    db.execute(
        "INSERT INTO playlist_analysis_runs (
          id, playlist_id, playlist_name, track_count, mode, provider,
          chunk_results_json, final_result_json, report_json, created_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, CURRENT_TIMESTAMP)",
        params![
            id,
            payload.playlist_id,
            payload.playlist_name,
            payload.track_count as i64,
            mode,
            payload.provider,
            payload.chunk_results_json,
            payload.final_result_json,
            payload.report_json,
        ],
    )
    .map_err(|error| error.to_string())?;

    load_playlist_analysis_run_by_id(db, &id)?
        .ok_or_else(|| "Saved playlist interpretation could not be loaded.".to_string())
}

fn load_latest_playlist_analysis_run(
    db: &Connection,
    playlist_id: &str,
) -> Result<Option<PlaylistAnalysisRunDto>, String> {
    db.query_row(
        "SELECT id, playlist_id, playlist_name, track_count, mode, provider,
          chunk_results_json, final_result_json, report_json, created_at
         FROM playlist_analysis_runs
         WHERE playlist_id = ?1
         ORDER BY created_at DESC, id DESC
         LIMIT 1",
        params![playlist_id],
        playlist_analysis_run_from_row,
    )
    .optional()
    .map_err(|error| error.to_string())
}

fn load_playlist_analysis_run_by_id(
    db: &Connection,
    id: &str,
) -> Result<Option<PlaylistAnalysisRunDto>, String> {
    db.query_row(
        "SELECT id, playlist_id, playlist_name, track_count, mode, provider,
          chunk_results_json, final_result_json, report_json, created_at
         FROM playlist_analysis_runs
         WHERE id = ?1",
        params![id],
        playlist_analysis_run_from_row,
    )
    .optional()
    .map_err(|error| error.to_string())
}

fn playlist_analysis_run_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<PlaylistAnalysisRunDto> {
    let track_count: i64 = row.get(3)?;

    Ok(PlaylistAnalysisRunDto {
        id: row.get(0)?,
        playlist_id: row.get(1)?,
        playlist_name: row.get(2)?,
        track_count: track_count.max(0) as u32,
        mode: row.get(4)?,
        provider: row.get(5)?,
        chunk_results_json: row.get(6)?,
        final_result_json: row.get(7)?,
        report_json: row.get(8)?,
        created_at: row.get(9)?,
    })
}

fn save_taste_notes(db: &Connection, source: &str) -> Result<TasteNotesDto, String> {
    let mut tracks = load_tracks(db)?
        .into_iter()
        .filter(|track| track.source == source)
        .collect::<Vec<_>>();

    tracks.sort_by(|a, b| {
        b.liked
            .cmp(&a.liked)
            .then_with(|| b.play_count.cmp(&a.play_count))
            .then_with(|| a.title.cmp(&b.title))
    });

    let playlist_count = count_source_playlists(db, source)?;
    let notes = build_taste_notes(source, &tracks, playlist_count);
    let notes_json = serde_json::to_string(&notes).map_err(|error| error.to_string())?;

    db.execute(
        "INSERT INTO taste_notes (id, source, track_count, playlist_count, notes_json, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, CURRENT_TIMESTAMP, CURRENT_TIMESTAMP)
         ON CONFLICT(id) DO UPDATE SET
           source = excluded.source,
           track_count = excluded.track_count,
           playlist_count = excluded.playlist_count,
           notes_json = excluded.notes_json,
           updated_at = CURRENT_TIMESTAMP",
        params![
            &notes.id,
            &notes.source,
            notes.track_count as i64,
            notes.playlist_count as i64,
            notes_json,
        ],
    )
    .map_err(|error| error.to_string())?;

    load_latest_taste_notes(db, source)?
        .ok_or_else(|| "Taste notes could not be loaded.".to_string())
}

fn load_latest_taste_notes(db: &Connection, source: &str) -> Result<Option<TasteNotesDto>, String> {
    let row = db
        .query_row(
            "SELECT notes_json, updated_at FROM taste_notes WHERE source = ?1 ORDER BY updated_at DESC LIMIT 1",
            params![source],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(|error| error.to_string())?;

    match row {
        Some((notes_json, updated_at)) => {
            let mut notes = serde_json::from_str::<TasteNotesDto>(&notes_json)
                .map_err(|error| error.to_string())?;
            notes.updated_at = updated_at;
            Ok(Some(notes))
        }
        None => Ok(None),
    }
}

fn count_source_playlists(db: &Connection, source: &str) -> Result<u32, String> {
    let count = db
        .query_row(
            "SELECT COUNT(DISTINCT playlists.id)
             FROM playlists
             JOIN playlist_tracks ON playlist_tracks.playlist_id = playlists.id
             JOIN tracks ON tracks.id = playlist_tracks.track_id
             WHERE tracks.source = ?1",
            params![source],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|error| error.to_string())?;

    Ok(count.max(0) as u32)
}

fn build_taste_notes(source: &str, tracks: &[TrackDto], playlist_count: u32) -> TasteNotesDto {
    let favorite_artists = top_track_labels(tracks, 8, |track| track.artist.clone());
    let favorite_albums = top_track_labels(tracks, 6, |track| track.album.clone());
    let favorite_languages = top_track_labels(tracks, 4, inferred_language_label);
    let favorite_moods = top_inferred_moods(tracks, 5);
    let favorite_scenes = inferred_scenes_from_moods(&favorite_moods);
    let top_artist = favorite_artists
        .first()
        .cloned()
        .unwrap_or_else(|| "a few familiar voices".to_string());
    let top_mood = favorite_moods
        .first()
        .cloned()
        .unwrap_or_else(|| "quiet after-hours listening".to_string());
    let confidence = round2(clamp01(tracks.len() as f64 / 120.0));

    let hidden_patterns = if tracks.is_empty() {
        vec!["Your listening memory is waiting for the first records to arrive.".to_string()]
    } else {
        vec![
            format!(
                "You tend to return to {top_artist} when the room asks for something familiar."
            ),
            format!("The collection leans toward {top_mood}, with room for a few brighter turns."),
            if playlist_count > 1 {
                "Several playlists point to a taste built by scenes rather than strict genres."
                    .to_string()
            } else {
                "The first layer is clear enough; more playlists will make the memory warmer."
                    .to_string()
            },
        ]
    };

    TasteNotesDto {
        id: stable_id(&format!("taste:{source}")),
        source: source.to_string(),
        track_count: tracks.len() as u32,
        playlist_count,
        music_personality: if tracks.is_empty() {
            "No listening memory yet. Bring in liked songs or playlists first.".to_string()
        } else {
            format!("A private shelf with {top_artist} near the front, built for {top_mood}.")
        },
        favorite_artists,
        favorite_albums,
        favorite_languages,
        favorite_moods,
        favorite_scenes,
        hidden_patterns,
        recommendation_strategy: if tracks.is_empty() {
            "Start with your liked songs, then let playlists add context.".to_string()
        } else {
            "Begin with familiar voices, then introduce adjacent tracks with the same late-room temperature.".to_string()
        },
        confidence,
        updated_at: now_timestamp(),
    }
}

fn top_track_labels<F>(tracks: &[TrackDto], limit: usize, label: F) -> Vec<String>
where
    F: Fn(&TrackDto) -> String,
{
    let mut counts: HashMap<String, f64> = HashMap::new();
    for track in tracks {
        let value = label(track).trim().to_string();
        if value.is_empty() || value == "Unknown Artist" || value == "Unknown Album" {
            continue;
        }
        let weight = if track.liked {
            2.5
        } else {
            1.0 + (track.play_count as f64 * 0.18)
        };
        *counts.entry(value).or_insert(0.0) += weight;
    }

    sorted_label_counts(&counts, limit)
}

fn top_inferred_moods(tracks: &[TrackDto], limit: usize) -> Vec<String> {
    let mut counts: HashMap<String, f64> = HashMap::new();
    for track in tracks {
        let moods = if track.moods.is_empty() {
            infer_moods_from_track(track)
        } else {
            track.moods.clone()
        };
        let weight = if track.liked { 2.0 } else { 1.0 };
        for mood in moods {
            *counts.entry(mood).or_insert(0.0) += weight;
        }
    }

    let labels = sorted_label_counts(&counts, limit);
    if labels.is_empty() {
        vec!["late-night calm".to_string()]
    } else {
        labels
    }
}

fn infer_moods_from_track(track: &TrackDto) -> Vec<String> {
    let text = format!("{} {} {}", track.title, track.artist, track.album).to_lowercase();
    let mut moods = Vec::new();

    if text.contains("夜") || text.contains("晚") || text.contains("moon") || text.contains("night")
    {
        moods.push("late-night calm".to_string());
    }
    if text.contains("雨")
        || text.contains("泪")
        || text.contains("sad")
        || text.contains("blue")
        || text.contains("lonely")
    {
        moods.push("rainy melancholy".to_string());
    }
    if text.contains("爱") || text.contains("love") || text.contains("romance") {
        moods.push("warm romance".to_string());
    }
    if text.contains("舞")
        || text.contains("dance")
        || text.contains("party")
        || text.contains("快乐")
    {
        moods.push("bright motion".to_string());
    }

    if moods.is_empty() {
        moods.push(if track.liked {
            "familiar warmth".to_string()
        } else {
            "soft discovery".to_string()
        });
    }

    moods
}

fn inferred_language_label(track: &TrackDto) -> String {
    if track.language != "unknown" && !track.language.trim().is_empty() {
        return track.language.clone();
    }

    let text = format!("{} {} {}", track.title, track.artist, track.album);
    if text
        .chars()
        .any(|ch| ('\u{4e00}'..='\u{9fff}').contains(&ch))
    {
        "Mandarin / Chinese".to_string()
    } else if text.is_ascii() {
        "English / International".to_string()
    } else {
        "International".to_string()
    }
}

fn inferred_scenes_from_moods(moods: &[String]) -> Vec<String> {
    let joined = moods.join(" ").to_lowercase();
    let mut scenes = Vec::new();

    if joined.contains("night") || joined.contains("calm") || joined.contains("warm") {
        scenes.push("Late-night radio".to_string());
    }
    if joined.contains("rain") || joined.contains("melancholy") {
        scenes.push("Rain on the window".to_string());
    }
    if joined.contains("motion") || joined.contains("bright") {
        scenes.push("Walking through daylight".to_string());
    }
    if joined.contains("romance") {
        scenes.push("A softer room".to_string());
    }
    if scenes.is_empty() {
        scenes.push("Private listening".to_string());
    }

    scenes.into_iter().take(4).collect()
}

fn sorted_label_counts(counts: &HashMap<String, f64>, limit: usize) -> Vec<String> {
    let mut entries = counts.iter().collect::<Vec<_>>();
    entries.sort_by(|a, b| {
        b.1.partial_cmp(a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.cmp(b.0))
    });
    entries
        .into_iter()
        .take(limit)
        .map(|(label, _)| label.clone())
        .collect()
}

fn save_mood_entry_to_db(db: &Connection, payload: SaveMoodEntryPayload) -> Result<(), String> {
    let entry_id = stable_id(&format!("mood:{}", payload.date));
    let private_tags = serde_json::to_string(&vec![payload.mood_signal.clone()])
        .map_err(|error| error.to_string())?;
    let recommended_track_ids = "[]";

    db.execute(
        "INSERT INTO mood_entries (
          id, entry_date, mood, note_text, private_tags_json,
          recommended_track_ids_json, created_at, updated_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, CURRENT_TIMESTAMP, CURRENT_TIMESTAMP)
        ON CONFLICT(entry_date) DO UPDATE SET
          mood = excluded.mood,
          note_text = excluded.note_text,
          private_tags_json = excluded.private_tags_json,
          recommended_track_ids_json = excluded.recommended_track_ids_json,
          updated_at = CURRENT_TIMESTAMP",
        params![
            entry_id,
            payload.date,
            payload.mood,
            payload.note,
            json_string(&serde_json::json!({
                "moodSignal": payload.mood_signal,
                "desiredVibe": payload.desired_vibe,
                "tags": serde_json::from_str::<Vec<String>>(&private_tags).unwrap_or_default()
            }))?,
            recommended_track_ids
        ],
    )
    .map_err(|error| error.to_string())?;

    Ok(())
}

fn load_today_mood_entry(db: &Connection) -> Result<Option<MoodEntryDto>, String> {
    let today = local_date_string();
    let mut entries = load_mood_entries_by_where(db, "WHERE entry_date = ?1", params![today], 1)?;
    Ok(entries.pop())
}

fn load_mood_entries(db: &Connection, limit: u32) -> Result<Vec<MoodEntryDto>, String> {
    load_mood_entries_by_where(
        db,
        "ORDER BY entry_date DESC LIMIT ?1",
        params![limit.clamp(1, 120) as i64],
        limit,
    )
}

fn load_mood_entries_by_where<P: rusqlite::Params>(
    db: &Connection,
    clause: &str,
    params: P,
    _limit: u32,
) -> Result<Vec<MoodEntryDto>, String> {
    let sql = format!(
        "SELECT id, entry_date, mood, note_text, private_tags_json, recommended_track_ids_json, created_at
         FROM mood_entries {clause}"
    );
    let mut statement = db.prepare(&sql).map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params, |row| {
            let private_json: String = row.get(4)?;
            let recommended_json: String = row.get(5)?;
            let (mood_signal, desired_vibe, private_tags) = parse_mood_private_json(&private_json);

            Ok(MoodEntryDto {
                id: row.get(0)?,
                date: row.get(1)?,
                mood: row.get(2)?,
                mood_signal,
                note: row.get::<_, Option<String>>(3)?.unwrap_or_default(),
                desired_vibe,
                private_tags,
                recommended_track_ids: parse_json_array(&recommended_json),
                created_at: row.get(6)?,
            })
        })
        .map_err(|error| error.to_string())?;

    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())
}

fn parse_mood_private_json(value: &str) -> (String, Option<String>, Vec<String>) {
    let parsed = serde_json::from_str::<serde_json::Value>(value).unwrap_or_default();
    let mood_signal = parsed
        .get("moodSignal")
        .and_then(|value| value.as_str())
        .unwrap_or("calm")
        .to_string();
    let desired_vibe = parsed
        .get("desiredVibe")
        .and_then(|value| value.as_str())
        .map(ToOwned::to_owned);
    let tags = parsed
        .get("tags")
        .and_then(|value| value.as_array())
        .map(|values| {
            values
                .iter()
                .filter_map(|value| value.as_str().map(ToOwned::to_owned))
                .collect()
        })
        .unwrap_or_default();

    (mood_signal, desired_vibe, tags)
}

fn refresh_user_profile(db: &Connection) -> Result<UserProfileDto, String> {
    let events = load_profile_events(db)?;
    let profile = compute_user_profile(&events);
    save_user_profile(db, &profile)?;
    Ok(profile)
}

fn load_profile_events(db: &Connection) -> Result<Vec<ProfileEvent>, String> {
    let mut statement = db
        .prepare(
            "SELECT
              listening_events.track_id,
              listening_events.event_type,
              listening_events.hour_of_day,
              listening_events.event_weight,
              COALESCE(artists.name, song_features.artist_name, 'Unknown Artist') AS artist,
              COALESCE(albums.title, song_features.album_title, 'Unknown Album') AS album,
              COALESCE(song_features.genres_json, tracks.genres_json, '[]') AS genres_json,
              COALESCE(song_features.moods_json, tracks.moods_json, '[]') AS moods_json,
              COALESCE(song_features.calm_score, 0) AS calm_score,
              COALESCE(song_features.energetic_score, 0) AS energetic_score
            FROM listening_events
            INNER JOIN tracks ON tracks.id = listening_events.track_id
            LEFT JOIN artists ON artists.id = tracks.artist_id
            LEFT JOIN albums ON albums.id = tracks.album_id
            LEFT JOIN song_features ON song_features.track_id = tracks.id
            ORDER BY listening_events.created_at DESC
            LIMIT 500",
        )
        .map_err(|error| error.to_string())?;

    let rows = statement
        .query_map([], |row| {
            let genres_json: String = row.get(6)?;
            let moods_json: String = row.get(7)?;
            Ok(ProfileEvent {
                track_id: row.get(0)?,
                event_type: row.get(1)?,
                hour_of_day: row.get::<_, i64>(2)?.clamp(0, 23) as u8,
                weight: row.get(3)?,
                artist: row.get(4)?,
                album: row.get(5)?,
                genres: parse_json_array(&genres_json),
                moods: parse_json_array(&moods_json),
                calm_score: row.get::<_, f64>(8)?.clamp(0.0, 1.0),
                energetic_score: row.get::<_, f64>(9)?.clamp(0.0, 1.0),
            })
        })
        .map_err(|error| error.to_string())?;

    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())
}

fn compute_user_profile(events: &[ProfileEvent]) -> UserProfileDto {
    let event_count = events.len() as u32;
    let positive_events = events.iter().filter(|event| event.weight > 0.0).count() as u32;
    let confidence = confidence_from_count(event_count, 16);
    let is_learning = event_count < 6 || positive_events < 3;

    if is_learning {
        return UserProfileDto {
            favorite_artists: Vec::new(),
            favorite_albums: Vec::new(),
            favorite_genres: Vec::new(),
            favorite_moods: Vec::new(),
            preferred_listening_hours: Vec::new(),
            night_listening_preference: ScoreConfidence {
                score: 0.0,
                confidence,
            },
            skip_patterns: Vec::new(),
            repeat_patterns: Vec::new(),
            liked_song_patterns: Vec::new(),
            exploration_score: ScoreConfidence {
                score: 0.0,
                confidence,
            },
            calm_music_preference: ScoreConfidence {
                score: 0.0,
                confidence,
            },
            energetic_music_preference: ScoreConfidence {
                score: 0.0,
                confidence,
            },
            event_count,
            confidence,
            is_learning,
            updated_at: "CURRENT_TIMESTAMP".to_string(),
        };
    }

    let mut artist_scores = HashMap::new();
    let mut album_scores = HashMap::new();
    let mut genre_scores = HashMap::new();
    let mut mood_scores = HashMap::new();
    let mut skip_scores = HashMap::new();
    let mut repeat_scores = HashMap::new();
    let mut liked_scores = HashMap::new();
    let mut hour_scores: HashMap<u8, f64> = HashMap::new();
    let mut played_tracks = HashSet::new();
    let mut play_event_count = 0_u32;
    let mut night_events = 0_u32;
    let mut time_events = 0_u32;
    let mut calm_sum = 0.0;
    let mut energetic_sum = 0.0;
    let mut feature_weight_sum = 0.0;

    for event in events {
        if event.weight > 0.0 {
            add_score(&mut artist_scores, &event.artist, event.weight);
            add_score(&mut album_scores, &event.album, event.weight);
            for genre in &event.genres {
                add_score(&mut genre_scores, genre, event.weight);
            }
            for mood in &event.moods {
                add_score(&mut mood_scores, mood, event.weight);
            }
            *hour_scores.entry(event.hour_of_day).or_insert(0.0) += 1.0;
            time_events += 1;
            calm_sum += event.calm_score * event.weight;
            energetic_sum += event.energetic_score * event.weight;
            feature_weight_sum += event.weight;
        }

        if event.event_type == "play" {
            played_tracks.insert(event.track_id.clone());
            play_event_count += 1;
        }

        if is_night_hour(event.hour_of_day) && event.weight > 0.0 {
            night_events += 1;
        }

        if event.event_type == "skip" {
            add_score(&mut skip_scores, &event.artist, 1.0);
            for genre in &event.genres {
                add_score(&mut skip_scores, genre, 0.7);
            }
        }

        if event.event_type == "replayed" {
            add_score(&mut repeat_scores, &event.artist, 1.0);
            add_score(&mut repeat_scores, &event.album, 0.8);
        }

        if event.event_type == "liked" {
            add_score(&mut liked_scores, &event.artist, 1.0);
            for genre in &event.genres {
                add_score(&mut liked_scores, genre, 0.8);
            }
            for mood in &event.moods {
                add_score(&mut liked_scores, mood, 0.8);
            }
        }
    }

    let night_score = if time_events == 0 {
        0.0
    } else {
        night_events as f64 / time_events as f64
    };
    let exploration_score = if play_event_count == 0 {
        0.0
    } else {
        played_tracks.len() as f64 / play_event_count as f64
    };
    let calm_score = if feature_weight_sum == 0.0 {
        0.0
    } else {
        calm_sum / feature_weight_sum
    };
    let energetic_score = if feature_weight_sum == 0.0 {
        0.0
    } else {
        energetic_sum / feature_weight_sum
    };

    UserProfileDto {
        favorite_artists: top_profile_ranks(&artist_scores, 4, confidence),
        favorite_albums: top_profile_ranks(&album_scores, 4, confidence),
        favorite_genres: top_profile_ranks(&genre_scores, 4, confidence),
        favorite_moods: top_profile_ranks(&mood_scores, 4, confidence),
        preferred_listening_hours: top_hour_preferences(&hour_scores, 4, confidence),
        night_listening_preference: ScoreConfidence {
            score: clamp01(night_score),
            confidence: confidence_from_count(time_events, 12),
        },
        skip_patterns: top_profile_ranks(
            &skip_scores,
            4,
            confidence_from_count(skip_scores.len() as u32, 3),
        ),
        repeat_patterns: top_profile_ranks(
            &repeat_scores,
            4,
            confidence_from_count(repeat_scores.len() as u32, 3),
        ),
        liked_song_patterns: top_profile_ranks(
            &liked_scores,
            4,
            confidence_from_count(liked_scores.len() as u32, 3),
        ),
        exploration_score: ScoreConfidence {
            score: clamp01(exploration_score),
            confidence: confidence_from_count(play_event_count, 8),
        },
        calm_music_preference: ScoreConfidence {
            score: clamp01(calm_score),
            confidence: confidence_from_count(feature_weight_sum.round() as u32, 8),
        },
        energetic_music_preference: ScoreConfidence {
            score: clamp01(energetic_score),
            confidence: confidence_from_count(feature_weight_sum.round() as u32, 8),
        },
        event_count,
        confidence,
        is_learning,
        updated_at: "CURRENT_TIMESTAMP".to_string(),
    }
}

fn save_user_profile(db: &Connection, profile: &UserProfileDto) -> Result<(), String> {
    db.execute(
        "INSERT INTO user_profile (
          id, event_count, confidence, favorite_artists_json, favorite_albums_json,
          favorite_genres_json, favorite_moods_json, preferred_listening_hours_json,
          night_listening_preference_json, skip_patterns_json, repeat_patterns_json,
          liked_song_patterns_json, exploration_score_json, calm_music_preference_json,
          energetic_music_preference_json, is_learning, updated_at
        )
        VALUES ('local', ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, CURRENT_TIMESTAMP)
        ON CONFLICT(id) DO UPDATE SET
          event_count = excluded.event_count,
          confidence = excluded.confidence,
          favorite_artists_json = excluded.favorite_artists_json,
          favorite_albums_json = excluded.favorite_albums_json,
          favorite_genres_json = excluded.favorite_genres_json,
          favorite_moods_json = excluded.favorite_moods_json,
          preferred_listening_hours_json = excluded.preferred_listening_hours_json,
          night_listening_preference_json = excluded.night_listening_preference_json,
          skip_patterns_json = excluded.skip_patterns_json,
          repeat_patterns_json = excluded.repeat_patterns_json,
          liked_song_patterns_json = excluded.liked_song_patterns_json,
          exploration_score_json = excluded.exploration_score_json,
          calm_music_preference_json = excluded.calm_music_preference_json,
          energetic_music_preference_json = excluded.energetic_music_preference_json,
          is_learning = excluded.is_learning,
          updated_at = CURRENT_TIMESTAMP",
        params![
            profile.event_count as i64,
            profile.confidence,
            json_string(&profile.favorite_artists)?,
            json_string(&profile.favorite_albums)?,
            json_string(&profile.favorite_genres)?,
            json_string(&profile.favorite_moods)?,
            json_string(&profile.preferred_listening_hours)?,
            json_string(&profile.night_listening_preference)?,
            json_string(&profile.skip_patterns)?,
            json_string(&profile.repeat_patterns)?,
            json_string(&profile.liked_song_patterns)?,
            json_string(&profile.exploration_score)?,
            json_string(&profile.calm_music_preference)?,
            json_string(&profile.energetic_music_preference)?,
            bool_to_int(profile.is_learning)
        ],
    )
    .map_err(|error| error.to_string())?;

    Ok(())
}

fn parse_json_array(value: &str) -> Vec<String> {
    serde_json::from_str(value).unwrap_or_default()
}

fn infer_song_features(
    title: &str,
    artist: &str,
    album: &str,
    genres: &[String],
) -> (Vec<String>, f64, f64) {
    let text = format!(
        "{} {} {} {}",
        title.to_lowercase(),
        artist.to_lowercase(),
        album.to_lowercase(),
        genres.join(" ").to_lowercase()
    );
    let calm_keywords = [
        "ambient",
        "acoustic",
        "ballad",
        "chill",
        "classical",
        "dream",
        "folk",
        "jazz",
        "lo-fi",
        "lofi",
        "piano",
        "sleep",
        "soft",
    ];
    let energetic_keywords = [
        "dance",
        "edm",
        "electronic",
        "funk",
        "hard",
        "hip-hop",
        "house",
        "metal",
        "pop",
        "punk",
        "rock",
        "techno",
        "trap",
    ];
    let calm_hits = calm_keywords
        .iter()
        .filter(|keyword| text.contains(**keyword))
        .count() as f64;
    let energetic_hits = energetic_keywords
        .iter()
        .filter(|keyword| text.contains(**keyword))
        .count() as f64;
    let calm_score = (calm_hits / 3.0).min(1.0);
    let energetic_score = (energetic_hits / 3.0).min(1.0);
    let mut moods = Vec::new();

    if calm_score >= 0.34 {
        moods.push("calm".to_string());
    }
    if energetic_score >= 0.34 {
        moods.push("energetic".to_string());
    }
    if text.contains("focus") || text.contains("study") || text.contains("work") {
        moods.push("focused".to_string());
    }
    if text.contains("dream") || text.contains("night") {
        moods.push("dreamy".to_string());
    }
    if moods.is_empty() {
        moods.push("unknown".to_string());
    }

    (moods, calm_score, energetic_score)
}

fn event_weight(event_type: &PlaybackEventType) -> f64 {
    match event_type {
        PlaybackEventType::Play => 0.5,
        PlaybackEventType::Completed => 1.4,
        PlaybackEventType::Liked => 2.0,
        PlaybackEventType::Replayed => 1.8,
        PlaybackEventType::Pause => 0.0,
        PlaybackEventType::Unliked => -0.8,
        PlaybackEventType::Skip => -1.0,
    }
}

fn add_score(map: &mut HashMap<String, f64>, key: &str, score: f64) {
    let key = key.trim();
    if key.is_empty() || key == "unknown" || key == "Unknown Artist" || key == "Unknown Album" {
        return;
    }
    *map.entry(key.to_string()).or_insert(0.0) += score;
}

fn top_profile_ranks(
    map: &HashMap<String, f64>,
    limit: usize,
    global_confidence: f64,
) -> Vec<ProfileRank> {
    let mut values = map
        .iter()
        .filter(|(_, score)| **score > 0.0)
        .map(|(label, score)| (label.clone(), *score))
        .collect::<Vec<_>>();
    values.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    let max_score = values
        .first()
        .map(|(_, score)| *score)
        .unwrap_or(1.0)
        .max(1.0);

    values
        .into_iter()
        .take(limit)
        .map(|(label, score)| ProfileRank {
            label,
            weight: round2(clamp01(score / max_score)),
            confidence: round2(clamp01(global_confidence * (score / max_score).sqrt())),
        })
        .collect()
}

fn top_hour_preferences(
    map: &HashMap<u8, f64>,
    limit: usize,
    global_confidence: f64,
) -> Vec<HourPreference> {
    let mut values = map
        .iter()
        .map(|(hour, score)| (*hour, *score))
        .collect::<Vec<_>>();
    values.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    let max_score = values
        .first()
        .map(|(_, score)| *score)
        .unwrap_or(1.0)
        .max(1.0);

    values
        .into_iter()
        .take(limit)
        .map(|(hour, score)| HourPreference {
            hour,
            weight: round2(clamp01(score / max_score)),
            confidence: round2(clamp01(global_confidence * (score / max_score).sqrt())),
        })
        .collect()
}

fn is_night_hour(hour: u8) -> bool {
    hour >= 22 || hour <= 4
}

fn confidence_from_count(count: u32, target: u32) -> f64 {
    if target == 0 {
        return 0.0;
    }
    round2((count as f64 / target as f64).min(1.0))
}

fn clamp01(value: f64) -> f64 {
    value.clamp(0.0, 1.0)
}

fn round2(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

fn json_string<T: Serialize>(value: &T) -> Result<String, String> {
    serde_json::to_string(value).map_err(|error| error.to_string())
}

fn stable_id(input: &str) -> String {
    let mut hasher = Sha1::new();
    hasher.update(input.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn bool_to_int(value: bool) -> i64 {
    if value {
        1
    } else {
        0
    }
}

fn chrono_like_timestamp() -> String {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos().to_string())
        .unwrap_or_else(|_| "0".to_string())
}

fn local_date_string() -> String {
    // SQLite localtime gives us a stable date string without adding a date-time crate.
    let db = Connection::open_in_memory();
    db.ok()
        .and_then(|db| {
            db.query_row("SELECT date('now', 'localtime')", [], |row| {
                row.get::<_, String>(0)
            })
            .ok()
        })
        .unwrap_or_else(|| "1970-01-01".to_string())
}

fn now_timestamp() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().to_string())
        .unwrap_or_else(|_| "0".to_string())
}

fn current_timestamp_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default()
}

fn fallback_cover_url(seed: &str) -> String {
    let colors = ["ff4778", "5be7ff", "adff6b", "ff775c"];
    let color =
        colors[seed.as_bytes().first().copied().unwrap_or_default() as usize % colors.len()];
    let svg = format!(
        "<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 512 512'><rect width='512' height='512' fill='%2308090d'/><circle cx='160' cy='170' r='150' fill='%23{color}' opacity='.75'/><circle cx='360' cy='350' r='190' fill='%235be7ff' opacity='.28'/><path d='M168 338c88 54 195 27 236-56' stroke='white' stroke-opacity='.72' stroke-width='26' fill='none' stroke-linecap='round'/></svg>"
    );
    format!(
        "data:image/svg+xml;base64,{}",
        general_purpose::STANDARD.encode(svg)
    )
}

#[cfg(test)]
mod tests {
    use super::{
        bilibili_cookie_from_login_url, bilibili_mixin_key, encode_url_component,
        extract_netease_login_cookie, has_cookie_name, merge_bilibili_cookies,
        normalize_bilibili_image_url, parse_bilibili_danmaku_xml, request_bilibili_danmaku_xml,
        wbi_key_from_url, ResolvedBilibiliSourceConfig,
    };

    #[test]
    fn extracts_only_session_cookie_values_from_bilibili_login_url() {
        let session_key = "SESSDATA";
        let csrf_key = "bili_jct";
        let login_url = format!(
            "https://www.bilibili.com/?DedeUserID=42&{session_key}=abc%2Cdef&{csrf_key}=csrf&refresh_token=private"
        );
        let cookie = bilibili_cookie_from_login_url(&login_url).expect("session cookie");

        assert!(cookie.contains("DedeUserID=42"));
        assert!(cookie.contains(&format!("{session_key}=abc%2Cdef")));
        assert!(cookie.contains(&format!("{csrf_key}=csrf")));
        assert!(!cookie.contains("refresh_token"));
    }

    #[test]
    fn rejects_login_url_without_session_values() {
        assert!(
            bilibili_cookie_from_login_url("https://www.bilibili.com/?refresh_token=private")
                .is_none()
        );
    }

    #[test]
    fn merges_device_identity_into_the_signed_in_session() {
        let session_cookie = format!("{}=session; {}=csrf", "SESSDATA", "bili_jct");
        let cookie = merge_bilibili_cookies(
            Some(session_cookie.as_str()),
            Some("buvid3=device-three; buvid4=device-four"),
        )
        .expect("merged cookie");

        assert!(has_cookie_name(&cookie, "SESSDATA"));
        assert!(has_cookie_name(&cookie, "buvid3"));
        assert!(has_cookie_name(&cookie, "buvid4"));
        assert_eq!(cookie.matches("buvid3=").count(), 1);
    }

    #[test]
    fn extracts_netease_qr_cookie_from_nested_data() {
        let value = serde_json::json!({
            "code": 803,
            "data": {
                "cookie": "MUSIC_U=session-token; __csrf=csrf-token; Path=/; HttpOnly"
            }
        });
        let cookie = extract_netease_login_cookie(&value, Some("NMTID=device-token; Path=/"))
            .expect("login cookie");

        assert!(cookie.contains("MUSIC_U=session-token"));
        assert!(cookie.contains("__csrf=csrf-token"));
        assert!(cookie.contains("NMTID=device-token"));
        assert!(!cookie.to_ascii_lowercase().contains("httponly"));
    }

    #[test]
    fn preserves_netease_music_u_when_merging_response_cookies() {
        let value = serde_json::json!({
            "code": 803,
            "cookie": "MUSIC_U=session-token; __csrf=old-csrf"
        });
        let cookie = extract_netease_login_cookie(&value, Some("__csrf=new-csrf; Path=/"))
            .expect("merged cookie");

        assert!(cookie.contains("MUSIC_U=session-token"));
        assert!(cookie.contains("__csrf=new-csrf"));
        assert_eq!(cookie.matches("MUSIC_U=").count(), 1);
    }

    #[test]
    fn prepares_bilibili_wbi_keys_and_utf8_query_values() {
        assert_eq!(
            wbi_key_from_url(Some("https://i0.hdslb.com/bfs/wbi/abc123.png")).as_deref(),
            Some("abc123")
        );
        assert_eq!(
            encode_url_component("反乌托邦 音乐"),
            "%E5%8F%8D%E4%B9%8C%E6%89%98%E9%82%A6%20%E9%9F%B3%E4%B9%90"
        );
        assert_eq!(
            bilibili_mixin_key("abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-_=")
                .chars()
                .count(),
            32
        );
    }

    #[test]
    fn parses_bilibili_danmaku_timeline_and_decodes_text() {
        let xml = r#"<i><d p="1.25,1,25,16777215,0,0,user-a,1">hello &amp; night</d><d p="8.5,1,25,16777215,0,0,user-b,2">晚安</d></i>"#;
        let items = parse_bilibili_danmaku_xml("12345", xml);

        assert_eq!(items.len(), 2);
        assert_eq!(items[0].time, 1.25);
        assert_eq!(items[0].text, "hello & night");
        assert_eq!(items[1].cid, "12345");
    }

    #[test]
    fn normalizes_bilibili_cover_urls_for_webview_images() {
        assert_eq!(
            normalize_bilibili_image_url("//i0.hdslb.com/bfs/archive/cover.jpg".to_string()),
            "https://i0.hdslb.com/bfs/archive/cover.jpg"
        );
        assert_eq!(
            normalize_bilibili_image_url("http://i1.hdslb.com/bfs/archive/cover.jpg".to_string()),
            "https://i1.hdslb.com/bfs/archive/cover.jpg"
        );
    }

    #[test]
    #[ignore = "live Bilibili diagnostic"]
    fn fetches_live_bilibili_danmaku_xml() {
        let config = ResolvedBilibiliSourceConfig {
            base_url: "https://api.bilibili.com".to_string(),
            token: None,
            search_scope: "music".to_string(),
        };
        let xml = tauri::async_runtime::block_on(request_bilibili_danmaku_xml(
            &config,
            "https://comment.bilibili.com/38240650238.xml",
        ))
        .expect("live danmaku response");
        let items = parse_bilibili_danmaku_xml("38240650238", &xml);
        assert!(
            !items.is_empty(),
            "received {} bytes but parsed no items",
            xml.len()
        );
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    if let Err(error) = run_inner() {
        eprintln!("Ome Music failed to start: {error}");
        std::process::exit(1);
    }
}

// Re-apply every persisted authorized music folder to Tauri's asset protocol
// scope on startup. The asset scope is process-local (not persisted), so
// without this restore step a restart would break playback for any previously
// picked folder outside $HOME/Music — the registry row would still exist, the
// tracks would still be in the DB, but convertFileSrc() URLs would be rejected.
// This keeps the static tauri.conf.json scope locked down; only folders the
// user explicitly chose (and that survived in the DB) become playable again.
fn restore_authorized_asset_scope(app: &tauri::AppHandle, db: &rusqlite::Connection) {
    let Ok(mut statement) = db.prepare("SELECT directory_path FROM authorized_music_directories")
    else {
        return;
    };
    let Ok(rows) = statement.query_map([], |row| row.get::<_, String>(0)) else {
        return;
    };
    let scope = app.asset_protocol_scope();
    for path in rows.filter_map(Result::ok) {
        let dir = std::path::PathBuf::from(path);
        if let Err(error) = scope.allow_directory(&dir, true) {
            eprintln!(
                "could not restore asset scope for {}: {error}",
                dir.display()
            );
        }
    }
}

fn run_inner() -> Result<(), Box<dyn std::error::Error>> {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .register_asynchronous_uri_scheme_protocol("ome-media", |context, request, responder| {
            let app = context.app_handle().clone();
            tauri::async_runtime::spawn(async move {
                responder.respond(respond_bilibili_media(app, request).await);
            });
        })
        .setup(|app| {
            let db = initialize_database(app).map_err(std::io::Error::other)?;
            // Re-grant every previously authorized music folder to the asset
            // protocol scope. The scope is process-local and does not persist,
            // so without this restore step every restart would silently break
            // playback for any folder the user had picked before (e.g. D:\Music
            // or a USB drive) even though the registry row is still in the DB.
            restore_authorized_asset_scope(app.handle(), &db);
            let managed_netease_api = resolve_managed_netease_api_runtime(app);
            app.manage(AppState {
                db: Mutex::new(db),
                media_proxy: Mutex::new(HashMap::new()),
                managed_netease_api,
                managed_netease_child: Arc::new(Mutex::new(None)),
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { .. } = event {
                // Only clean up the managed node.exe runtime when the MAIN window
                // is closed — not when auxiliary windows (e.g. qqmusic-login)
                // are closed. Closing those should NOT kill the netease API
                // service or cause the app to exit.
                let label = window.label().to_string();
                if label == "main" {
                    let app_handle = window.app_handle();
                    let state = app_handle.state::<AppState>();
                    if let Ok(mut guard) = state.managed_netease_child.lock() {
                        if let Some(mut child) = guard.take() {
                            let _ = child.kill();
                        }
                    };
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_app_version,
            get_storage_report,
            clear_storage_bucket,
            export_storage_diagnostics,
            list_tracks,
            get_user_profile,
            get_today_mood_entry,
            list_mood_entries,
            save_mood_entry,
            import_music_folder,
            scan_music_directory,
            ensure_netease_api_service,
            get_netease_source_config,
            save_netease_source_config,
            get_bilibili_source_config,
            save_bilibili_source_config,
            test_bilibili_source_connection,
            import_bilibili_cookie,
            create_bilibili_qr_login,
            check_bilibili_qr_login,
            get_bilibili_login_status,
            logout_bilibili,
            search_bilibili_songs,
            get_bilibili_song_metadata,
            get_bilibili_playable_url,
            import_bilibili_song,
            get_bilibili_danmaku,
            clear_danmaku_cache,
            create_netease_qr_login,
            check_netease_qr_login,
            import_netease_cookie,
            login_netease_with_password,
            request_netease_sms_code,
            login_netease_with_sms_code,
            open_source_web_login,
            open_external_url,
            get_netease_login_status,
            refresh_netease_login,
            logout_netease,
            get_netease_vip_status,
            get_netease_user_profile,
            test_netease_source_connection,
            search_netease_songs,
            get_netease_playlist,
            get_netease_liked_songs,
            get_netease_user_playlists,
            import_netease_playlist,
            import_netease_song,
            sync_netease_listening_memory,
            get_latest_taste_notes,
            get_netease_song_metadata,
            get_netease_playable_url,
            get_netease_lyrics,
            resolve_track_lyrics,
            save_lyric_offset,
            import_track_lyrics,
            record_playback_event,
            set_track_liked,
            get_llm_provider_config,
            save_llm_provider_config,
            fetch_llm_models,
            generate_llm_text,
            transcribe_speech_audio,
            synthesize_curator_speech,
            save_playlist_analysis_result,
            get_latest_playlist_analysis,
            get_qqmusic_source_config,
            save_qqmusic_source_config,
            test_qqmusic_source_connection,
            import_qqmusic_token,
            create_qqmusic_qr_login,
            check_qqmusic_qr_login,
            get_qqmusic_login_status,
            logout_qqmusic,
            open_qqmusic_webview_login,
            extract_qqmusic_webview_cookie,
            close_qqmusic_webview_login,
            search_qqmusic_songs,
            get_qqmusic_song_metadata,
            get_qqmusic_playable_url,
            import_qqmusic_song,
            get_qqmusic_lyrics,
            get_qqmusic_user_playlists,
            get_qqmusic_playlist,
            import_qqmusic_playlist,
            get_qqmusic_liked_songs,
            get_qqmusic_vip_status,
            qqmusic_debug_dump,
            get_qqmusic_user_profile
        ])
        .run(tauri::generate_context!())?;
    Ok(())
}
