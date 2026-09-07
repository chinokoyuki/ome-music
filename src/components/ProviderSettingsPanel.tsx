import {
  BookOpen,
  ChevronDown,
  Cloud,
  Database,
  FileDown,
  HardDrive,
  KeyRound,
  Library,
  ListMusic,
  Loader2,
  Mic2,
  Music2,
  QrCode,
  Radio,
  RefreshCw,
  Save,
  Settings2,
  ShieldCheck,
  SlidersHorizontal,
  Sparkles,
  Trash2,
  Volume2,
  X,
  type LucideIcon,
} from "lucide-react";
import clsx from "clsx";
import type { ReactNode } from "react";
import { useCallback, useEffect, useRef, useState } from "react";
import {
  fetchLlmModels,
  getLlmProviderConfig,
  saveLlmProviderConfig,
  type LlmProviderConfig,
} from "../features/llm/provider";
import {
  getSpeechProviderConfig,
  listSpeechVoices,
  saveSpeechProviderConfig,
  type SpeechProviderConfig,
  type SpeechVoiceOption,
} from "../features/speech/provider";
import {
  BilibiliAccountSessionProvider,
  BilibiliMusicProvider,
  ensureNeteaseApiService,
  getBilibiliSourceConfig,
  getNeteaseSourceConfig,
  getQQMusicSourceConfig,
  NetEaseAccountSessionProvider,
  NetEaseMusicProvider,
  QQMusicAccountSessionProvider,
  refreshLocalTracksAfterSourceImport,
  saveBilibiliSourceConfig,
  saveNeteaseSourceConfig,
  saveQQMusicSourceConfig,
  testBilibiliSourceConnection,
  testNeteaseSourceConnection,
  type BilibiliDanmakuDebug,
  type BilibiliLoginStatus,
  type BilibiliSourceConfig,
  type NetEaseLoginStatus,
  type NetEasePlaybackDebug,
  type NetEaseQrLogin,
  type NetEaseServiceStage,
  type NetEaseServiceStatus,
  type NetEaseVipStatus,
  type QQMusicLoginStatus,
  type QQMusicLoginFlow,
  type QQMusicVipStatus,
  type QQMusicQrLogin,
  type MusicSourceConfig,
  type NetEaseUserPlaylist,
  type PlayableUrlOptions,
  type TasteNotes,
} from "../features/musicSources/provider";
import {
  defaultDanmakuSettings,
  getDanmakuSettings,
  saveDanmakuSettings,
  type DanmakuSettings,
} from "../features/danmaku/danmakuSettings";
import {
  clearStorageBucket,
  exportStorageDiagnostics,
  getStorageReport,
  type StorageBucketKind,
  type StorageReport,
} from "../features/storage/storageApi";

import { ArtworkImage } from "./ArtworkImage";
import type { Track } from "../types/music";

interface ProviderSettingsPanelProps {
  open: boolean;
  focus?: "all" | "music" | "atmosphere";
  playbackQuality: NonNullable<PlayableUrlOptions["level"]>;
  onPlaybackQualityChange: (level: NonNullable<PlayableUrlOptions["level"]>) => void;
  onClose: () => void;
  onLibraryChanged?: (tracks: Track[]) => void;
  onRestartOnboarding?: () => void;
  // Live login-state propagation. The panel owns its own login refresh (QR,
  // password, SMS, cookie import), but the rest of the app must NOT wait until
  // the panel closes to see the new session state — otherwise playback / search
  // gating / onboarding still treat the user as signed out right after a scan.
  onNetEaseLoginChanged?: (status: NetEaseLoginStatus | null) => void;
  onBilibiliLoginChanged?: (status: BilibiliLoginStatus | null) => void;
  // Last NetEase playback resolve diagnostics + last Bilibili danmaku fetch
  // diagnostics, surfaced in the Advanced section so failures are never silent.
  neteasePlaybackDebug?: NetEasePlaybackDebug | null;
  bilibiliDanmakuDebug?: BilibiliDanmakuDebug | null;
}

type SettingsSection =
  "overview" | "sources" | "curator" | "playback" | "atmosphere" | "storage" | "advanced" | "guide";

const settingsSections: Array<{
  id: SettingsSection;
  title: string;
  subtitle: string;
  icon: LucideIcon;
}> = [
  { id: "overview", title: "快速开始", subtitle: "Quick Setup", icon: Radio },
  { id: "sources", title: "音乐来源", subtitle: "Music Sources", icon: Cloud },
  { id: "playback", title: "播放", subtitle: "Playback", icon: Volume2 },
  { id: "atmosphere", title: "弹幕氛围", subtitle: "Atmosphere", icon: SlidersHorizontal },
  { id: "curator", title: "鉴赏家与声音", subtitle: "Curator & Voice", icon: KeyRound },
  { id: "storage", title: "存储", subtitle: "Storage", icon: HardDrive },
  { id: "advanced", title: "高级", subtitle: "Advanced", icon: Settings2 },
  { id: "guide", title: "使用指南", subtitle: "Guide", icon: BookOpen },
];

const emptyConfig: LlmProviderConfig = {
  providerName: "",
  baseUrl: "",
  model: "",
  maskedApiKey: "",
  hasApiKey: false,
  configured: false,
};

const emptySpeechConfig: SpeechProviderConfig = {
  sttProvider: "curator",
  ttsProvider: "curator",
  voice: "FunAudioLLM/CosyVoice2-0.5B:alex",
  languageDetection: true,
  sttModel: "FunAudioLLM/SenseVoiceSmall",
  ttsModel: "FunAudioLLM/CosyVoice2-0.5B",
};

const emptyMusicSourceConfig: MusicSourceConfig = {
  enabled: false,
  baseUrl: "http://127.0.0.1:3000",
  hasToken: false,
  maskedToken: "",
};

const neteaseProvider = new NetEaseMusicProvider();
const neteaseAuthProvider = new NetEaseAccountSessionProvider();
const bilibiliProvider = new BilibiliMusicProvider();
const bilibiliAuthProvider = new BilibiliAccountSessionProvider();
const qqmusicAuthProvider = new QQMusicAccountSessionProvider();

// QR 会话状态机：waiting 等待扫码 / scanned 已扫描待确认 / expired 服务端判定过期 / timeout 客户端兜底超时
type QrSessionStatus = "waiting" | "scanned" | "expired" | "timeout";

// max-life 兜底：服务端不会主动告知超时，需客户端在 QR 失效前停止轮询并保留弹窗供用户重新生成。
// 网易云二维码官方有效期约 180s；Bilibili 二维码约 200s。
const NETEASE_QR_MAX_LIFE_MS = 180_000;
const BILIBILI_QR_MAX_LIFE_MS = 200_000;

// Playlist Shelf persistence — records which NetEase playlists have been
// imported into the local library and when, so the shelf can surface
// "Imported" state and a "last synced" stamp across sessions without a
// re-fetch. Best-effort: localStorage failures are swallowed so a quota
// error never blocks the import itself.
const PLAYLIST_SHELF_KEY = "ome.playlistShelf.imported";
const LIKED_SYNC_KEY = "ome.playlistShelf.likedSyncedAt";

interface ImportedPlaylistRecord {
  id: string;
  name: string;
  trackCount: number;
  importedAt: string; // ISO timestamp
}

// Per-playlist import progress. Replaces the single boolean
// `isImportingPlaylist` for the shelf cards so each row reports its own
// state (reading / imported N / failed) instead of a global lock.
type PlaylistImportState =
  | { status: "reading" }
  | { status: "imported"; trackCount: number; at: string }
  | { status: "failed"; reason: string; at: string };

function loadImportedPlaylistRecords(): Record<string, ImportedPlaylistRecord> {
  try {
    const raw = window.localStorage.getItem(PLAYLIST_SHELF_KEY);
    if (!raw) return {};
    const parsed = JSON.parse(raw) as Record<string, ImportedPlaylistRecord>;
    return parsed && typeof parsed === "object" ? parsed : {};
  } catch {
    return {};
  }
}

function saveImportedPlaylistRecords(records: Record<string, ImportedPlaylistRecord>) {
  try {
    window.localStorage.setItem(PLAYLIST_SHELF_KEY, JSON.stringify(records));
  } catch {
    // ignore quota / serialization errors — shelf stamps are best-effort
  }
}

function loadLikedSyncedAt(): string | null {
  try {
    return window.localStorage.getItem(LIKED_SYNC_KEY);
  } catch {
    return null;
  }
}

function saveLikedSyncedAt(iso: string) {
  try {
    window.localStorage.setItem(LIKED_SYNC_KEY, iso);
  } catch {
    // ignore
  }
}

// Heuristic: NetEase returns the user's liked-songs playlist ("我喜欢的音乐")
// as the first owned (non-subscribed) playlist in /user/playlist. We surface
// it distinctly in the shelf so the user sees their Liked collection as a
// first-class card. When login info is available we additionally confirm the
// creator matches the logged-in user; otherwise we trust the NetEase ordering
// convention.
function isLikedSongsPlaylist(
  playlist: NetEaseUserPlaylist,
  index: number,
  loginStatus: NetEaseLoginStatus | null,
): boolean {
  if (playlist.subscribed) return false;
  if (index !== 0) return false;
  const owner = loginStatus?.nickname || loginStatus?.userId;
  if (!owner) return true;
  return playlist.creatorName === owner;
}

// Soft relative-time label for the shelf stamps ("just now", "3m ago", ...).
function formatRelativeTime(iso: string | null): string | null {
  if (!iso) return null;
  const then = Date.parse(iso);
  if (Number.isNaN(then)) return null;
  const diffMs = Date.now() - then;
  const sec = Math.floor(diffMs / 1000);
  if (sec < 45) return "just now";
  if (sec < 3600) return `${Math.floor(sec / 60)}m ago`;
  if (sec < 86400) return `${Math.floor(sec / 3600)}h ago`;
  return `${Math.floor(sec / 86400)}d ago`;
}

// A single shelf card. The shelf is a card grid rather than a backend
// table: cover, name, count, a quiet sync-status line that reports the
// live per-playlist progress (reading / imported N / failed), and a
// single primary action that flips between Import / Re-sync. The Liked
// collection gets a soft ring + badge so it reads as first-class.
function PlaylistShelfCard({
  playlist,
  liked,
  state,
  imported,
  disabled,
  onImport,
}: {
  playlist: NetEaseUserPlaylist;
  liked: boolean;
  state?: PlaylistImportState;
  imported?: ImportedPlaylistRecord;
  disabled: boolean;
  onImport: () => void;
}) {
  const isReading = state?.status === "reading";
  const isFailed = state?.status === "failed";
  const isImported = state?.status === "imported" || (!state && !!imported);
  const liveImported = state?.status === "imported" ? state : null;
  const importedCount = liveImported?.trackCount ?? imported?.trackCount ?? 0;
  const importedLabel = formatRelativeTime(liveImported?.at ?? imported?.importedAt ?? null);
  const failedReason = state?.status === "failed" ? state.reason : "";

  return (
    <div
      className={clsx(
        "flex flex-col gap-2 rounded-[18px] bg-white/[0.04] p-3",
        liked && "ring-1 ring-white/12",
      )}
    >
      <div className="flex items-start gap-3">
        <ArtworkImage
          src={playlist.coverUrl}
          alt={playlist.name}
          source="netease"
          className="h-12 w-12 shrink-0 rounded-[10px] object-cover"
        />
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-1.5">
            {liked && (
              <span className="shrink-0 rounded-full bg-[#7a2d1c]/45 px-1.5 py-0.5 text-[9px] font-bold uppercase tracking-wide text-white/74">
                Liked
              </span>
            )}
            <p className="truncate text-sm font-semibold text-white/82">{playlist.name}</p>
          </div>
          <p className="truncate text-xs text-white/38">
            {playlist.trackCount} songs
            {playlist.creatorName ? ` · ${playlist.creatorName}` : ""}
          </p>
        </div>
      </div>

      {/* Status line — quiet per-row feedback. NOTE: "Imported locally" is
          deliberately honest: this is a one-way local import from NetEase,
          NOT a two-way cloud sync. The stamp reflects when the local
          library last received this playlist's tracks. */}
      <div className="flex items-center justify-between gap-2">
        <span className="min-w-0 truncate text-[11px] font-medium text-white/40">
          {isReading
            ? "Reading playlist…"
            : isFailed
              ? `Failed · ${failedReason}`
              : isImported
                ? `Imported locally · ${importedCount} songs${importedLabel ? ` · ${importedLabel}` : ""}`
                : liked
                  ? "Not imported yet"
                  : "NetEase"}
        </span>
        <button
          type="button"
          onClick={onImport}
          disabled={isReading || disabled}
          className="app-transition flex shrink-0 items-center gap-1.5 rounded-full bg-white/[0.08] px-3 py-1.5 text-xs font-semibold text-white/64 hover:bg-white/[0.13] hover:text-white disabled:cursor-wait disabled:opacity-45"
        >
          {isReading && <Loader2 className="h-3 w-3 animate-spin" />}
          {isReading ? "Reading" : isImported ? "Re-import" : "Import"}
        </button>
      </div>
    </div>
  );
}

export function ProviderSettingsPanel({
  open,
  focus = "all",
  playbackQuality,
  onPlaybackQualityChange,
  onClose,
  onLibraryChanged,
  onRestartOnboarding,
  onNetEaseLoginChanged,
  onBilibiliLoginChanged,
  neteasePlaybackDebug = null,
  bilibiliDanmakuDebug = null,
}: ProviderSettingsPanelProps) {
  const [activeSection, setActiveSection] = useState<SettingsSection>(
    focus === "music" ? "sources" : focus === "atmosphere" ? "atmosphere" : "overview",
  );
  const [config, setConfig] = useState<LlmProviderConfig>(emptyConfig);
  const [providerName, setProviderName] = useState("");
  const [baseUrl, setBaseUrl] = useState("");
  const [model, setModel] = useState("");
  const [apiKey, setApiKey] = useState("");
  const [availableModels, setAvailableModels] = useState<string[]>([]);
  const [speechConfig, setSpeechConfig] = useState<SpeechProviderConfig>(emptySpeechConfig);
  const [voices, setVoices] = useState<SpeechVoiceOption[]>([]);
  const [musicSourceConfig, setMusicSourceConfig] =
    useState<MusicSourceConfig>(emptyMusicSourceConfig);
  const [bilibiliConfig, setBilibiliConfig] = useState<BilibiliSourceConfig>({
    enabled: false,
    baseUrl: "https://api.bilibili.com",
    hasToken: false,
    maskedToken: "",
    searchScope: "music",
  });
  const [neteaseEnabled, setNeteaseEnabled] = useState(false);
  const [neteaseBaseUrl, setNeteaseBaseUrl] = useState("");
  const [neteaseToken, setNeteaseToken] = useState("");
  const [neteaseLoginType, setNeteaseLoginType] = useState<"phone" | "email">("phone");
  const [neteaseAccount, setNeteaseAccount] = useState("");
  const [neteasePassword, setNeteasePassword] = useState("");
  const [neteasePhone, setNeteasePhone] = useState("");
  const [neteaseCountryCode, setNeteaseCountryCode] = useState("86");
  const [neteaseSmsCode, setNeteaseSmsCode] = useState("");
  const [smsCooldown, setSmsCooldown] = useState(0);
  const [isOtherLoginOpen, setOtherLoginOpen] = useState(false);
  const [bilibiliEnabled, setBilibiliEnabled] = useState(false);
  const [bilibiliBaseUrl, setBilibiliBaseUrl] = useState("https://api.bilibili.com");
  const [bilibiliToken, setBilibiliToken] = useState("");
  const [bilibiliSearchScope, setBilibiliSearchScope] =
    useState<BilibiliSourceConfig["searchScope"]>("music");
  const [bilibiliLoginStatus, setBilibiliLoginStatus] = useState<BilibiliLoginStatus | null>(null);
  const [bilibiliQr, setBilibiliQr] = useState<NetEaseQrLogin | null>(null);
  const [isBilibiliOtherLoginOpen, setBilibiliOtherLoginOpen] = useState(false);
  // QQ Music state
  const [qqmusicEnabled, setQQMusicEnabled] = useState(false);
  const [qqmusicBaseUrl, setQQMusicBaseUrl] = useState("https://c.y.qq.com");
  const [qqmusicToken, setQQMusicToken] = useState("");
  const [qqmusicLoginStatus, setQQMusicLoginStatus] = useState<QQMusicLoginStatus | null>(null);
  const [qqmusicMsg, setQQMusicMsg] = useState<string | null>(null);
  // QQ Music QR / VIP / Playlists
  const [qqmusicQr, setQQMusicQr] = useState<QQMusicQrLogin | null>(null);
  const [qqmusicQrStatus, setQQMusicQrStatus] = useState<string>("");
  // QR generation guard: incremented on every "New Code"; any async poll
  // response whose generation is stale is dropped (see poll effect).
  const qqmusicQrGenerationRef = useRef(0);
  const [qqmusicWebLoginMode, setQQMusicWebLoginMode] = useState<"qq" | "wechat" | null>(null);
  // Backend login-flow state machine (sole auth authority): polled while the
  // official sign-in window is open. Auto-finalizes on success — the user
  // never has to press "Complete Connection" or "Check Login" themselves.
  const [qqmusicLoginFlow, setQQMusicLoginFlow] = useState<QQMusicLoginFlow | null>(null);
  // Prevents an unmounted panel or a re-opened login window from consuming a
  // stale flow status (same generation-guard pattern as the QR poll).
  const qqmusicWebLoginGenerationRef = useRef(0);
  // Whether the login flow already reported `authenticated` for this window
  // (used to auto-close the flow UI exactly once).
  const qqmusicFlowAuthedRef = useRef(false);
  const [, setQQMusicVipStatus] = useState<QQMusicVipStatus | null>(null);
  const [, setQQMusicUserPlaylists] = useState<NetEaseUserPlaylist[]>([]);
  const [isCreatingQQMusicQr, setCreatingQQMusicQr] = useState(false);
  const [isCheckingQQMusicLogin, setCheckingQQMusicLogin] = useState(false);
  const [danmakuSettings, setDanmakuSettings] = useState<DanmakuSettings>(defaultDanmakuSettings);
  const [neteasePlaylistId, setNeteasePlaylistId] = useState("");
  const [neteaseUserPlaylists, setNeteaseUserPlaylists] = useState<NetEaseUserPlaylist[]>([]);
  const [tasteNotes, setTasteNotes] = useState<TasteNotes | null>(null);
  const [neteaseLoginStatus, setNeteaseLoginStatus] = useState<NetEaseLoginStatus | null>(null);
  const [neteaseVipStatus, setNeteaseVipStatus] = useState<NetEaseVipStatus | null>(null);
  const [neteaseQr, setNeteaseQr] = useState<NetEaseQrLogin | null>(null);
  // NetEase 本地服务启动状态：扫码后立即搜索时，前端可显示 stage 并在失败后重试。
  const [neteaseServiceStatus, setNeteaseServiceStatus] = useState<NetEaseServiceStatus | null>(
    null,
  );
  const [isRefreshingNeteaseService, setRefreshingNeteaseService] = useState(false);
  const [neteaseServiceRetryToken, setNeteaseServiceRetryToken] = useState(0);
  // QR 会话状态与起始时间：用于在弹窗内就近显示动态状态、过期/超时后保留弹窗供用户重新生成。
  const [neteaseQrStatus, setNeteaseQrStatus] = useState<QrSessionStatus>("waiting");
  const neteaseQrStartedAtRef = useRef<number>(0);
  const [bilibiliQrStatus, setBilibiliQrStatus] = useState<QrSessionStatus>("waiting");
  const bilibiliQrStartedAtRef = useRef<number>(0);
  const [isLoading, setIsLoading] = useState(false);
  const [isSaving, setIsSaving] = useState(false);
  const [isFetchingModels, setIsFetchingModels] = useState(false);
  const [isTestingSource, setTestingSource] = useState(false);
  const [isCheckingLogin, setCheckingLogin] = useState(false);
  const [isCreatingQr, setCreatingQr] = useState(false);
  const [isImportingPlaylist, setImportingPlaylist] = useState(false);
  const [isLoadingUserPlaylists, setLoadingUserPlaylists] = useState(false);
  const [isSyncingMemory, setSyncingMemory] = useState(false);
  // Playlist Shelf state. `importedRecords` is the persisted cache of
  // playlists already imported into the local library (survives restarts);
  // `playlistStates` is the live per-playlist progress state machine that
  // drives the card's reading / imported N / failed feedback. `likedSyncedAt`
  // is the persisted stamp for the last "Sync Liked" run.
  const [importedRecords, setImportedRecords] = useState<Record<string, ImportedPlaylistRecord>>(
    () => loadImportedPlaylistRecords(),
  );
  const [playlistStates, setPlaylistStates] = useState<Record<string, PlaylistImportState>>({});
  const [likedSyncedAt, setLikedSyncedAt] = useState<string | null>(() => loadLikedSyncedAt());
  const [isTestingBilibili, setTestingBilibili] = useState(false);
  const [isCheckingBilibili, setCheckingBilibili] = useState(false);
  const [isCreatingBilibiliQr, setCreatingBilibiliQr] = useState(false);
  const [isPasswordLogin, setPasswordLogin] = useState(false);
  const [isSendingSms, setSendingSms] = useState(false);
  const [isSmsLogin, setSmsLogin] = useState(false);
  const [openingWebLoginSource, setOpeningWebLoginSource] = useState<
    "netease" | "bilibili" | "qqmusic" | null
  >(null);
  const [message, setMessage] = useState<string | null>(null);
  const [modelMessage, setModelMessage] = useState<string | null>(null);
  const [sourceMessage, setSourceMessage] = useState<string | null>(null);
  const [isStorageOpen, setStorageOpen] = useState(false);
  const [storageReport, setStorageReport] = useState<StorageReport | null>(null);
  const [storageMessage, setStorageMessage] = useState<string | null>(null);
  const [clearingStorageKind, setClearingStorageKind] = useState<StorageBucketKind | null>(null);

  // Live login-state propagation. Every login path inside the panel
  // (QR scan, password, SMS, cookie import, manual refresh) funnels through
  // setNeteaseLoginStatus / setBilibiliLoginStatus, so a single effect per
  // source mirrors the fresh state into the App shell immediately. This is the
  // fix for "settings shows signed in, but playback says Sign in needed right
  // after a scan" — the App shell no longer waits until panel close to learn
  // the session changed.
  useEffect(() => {
    onNetEaseLoginChanged?.(neteaseLoginStatus);
  }, [neteaseLoginStatus, onNetEaseLoginChanged]);

  useEffect(() => {
    onBilibiliLoginChanged?.(bilibiliLoginStatus);
  }, [bilibiliLoginStatus, onBilibiliLoginChanged]);

  // Live config mirror used by stable callbacks so their identities never
  // change and the main config-loading effect does not re-run after a load
  // completes (re-running cleared user-pasted tokens mid-edit).
  const musicSourceConfigRef = useRef<MusicSourceConfig>(musicSourceConfig);
  useEffect(() => {
    musicSourceConfigRef.current = musicSourceConfig;
  }, [musicSourceConfig]);

  const refreshNetEaseAccountStatus = useCallback(async () => {
    setCheckingLogin(true);
    try {
      const [login, vip] = await Promise.all([
        neteaseAuthProvider.getLoginStatus(),
        neteaseAuthProvider.getVipStatus(),
      ]);
      setNeteaseLoginStatus(login);
      setNeteaseVipStatus(vip);
      const savedMusicSourceConfig = await getNeteaseSourceConfig();
      setMusicSourceConfig(savedMusicSourceConfig);
    } catch (error) {
      const latest = musicSourceConfigRef.current;
      setNeteaseLoginStatus({
        loggedIn: false,
        expired: false,
        loginStatusKnown: false,
        hasCookie: latest.hasToken,
        maskedCookie: latest.maskedToken,
        message: readError(error),
      });
      setNeteaseVipStatus(null);
    } finally {
      setCheckingLogin(false);
    }
  }, []);

  const refreshBilibiliStatus = useCallback(async () => {
    setCheckingBilibili(true);
    try {
      const status = await bilibiliAuthProvider.getLoginStatus();
      setBilibiliLoginStatus(status);
    } catch (error) {
      setBilibiliLoginStatus({
        loggedIn: false,
        expired: true,
        message: readError(error),
      });
    } finally {
      setCheckingBilibili(false);
    }
  }, []);

  useEffect(() => {
    if (!open) return;

    setActiveSection(
      focus === "music" ? "sources" : focus === "atmosphere" ? "atmosphere" : "overview",
    );

    let cancelled = false;
    setIsLoading(true);
    setMessage(null);
    setModelMessage(null);
    setAvailableModels([]);
    setSpeechConfig(getSpeechProviderConfig());
    setSourceMessage(null);
    setDanmakuSettings(getDanmakuSettings());
    setStorageMessage(null);
    setNeteaseUserPlaylists([]);
    setNeteaseQr(null);
    setNeteaseQrStatus("waiting");
    neteaseQrStartedAtRef.current = 0;
    setBilibiliQr(null);
    setBilibiliQrStatus("waiting");
    bilibiliQrStartedAtRef.current = 0;
    setNeteaseLoginStatus(null);
    setNeteaseVipStatus(null);
    setNeteasePassword("");
    setNeteaseSmsCode("");
    setSmsCooldown(0);
    setTasteNotes(null);
    void listSpeechVoices().then(setVoices);
    void neteaseProvider
      .getLatestTasteNotes()
      .then(setTasteNotes)
      .catch(() => setTasteNotes(null));
    void refreshStorageReport();
    void (async () => {
      const [neteaseResult, bilibiliResult, qqmusicResult, curatorResult] =
        await Promise.allSettled([
          getNeteaseSourceConfig(),
          getBilibiliSourceConfig(),
          getQQMusicSourceConfig(),
          getLlmProviderConfig(),
        ]);
      if (cancelled) return;

      if (neteaseResult.status === "fulfilled") {
        const loaded = neteaseResult.value;
        setMusicSourceConfig(loaded);
        setNeteaseEnabled(loaded.enabled);
        setNeteaseBaseUrl(loaded.baseUrl);
        setNeteaseToken("");
        if (loaded.enabled) void refreshNetEaseAccountStatus();
      }

      if (bilibiliResult.status === "fulfilled") {
        const loaded = bilibiliResult.value;
        setBilibiliConfig(loaded);
        setBilibiliEnabled(loaded.enabled);
        setBilibiliBaseUrl(loaded.baseUrl);
        setBilibiliSearchScope(loaded.searchScope);
        setBilibiliToken("");
        if (loaded.enabled) void refreshBilibiliStatus();
      }

      if (qqmusicResult.status === "fulfilled") {
        const loaded = qqmusicResult.value;
        setQQMusicEnabled(loaded.enabled);
        setQQMusicBaseUrl(loaded.baseUrl);
        setQQMusicToken("");
        if (loaded.enabled) {
          void qqmusicAuthProvider
            .getLoginStatus()
            .then((status) => {
              if (!cancelled) setQQMusicLoginStatus(status);
              if (!cancelled && status.loggedIn) {
                void qqmusicAuthProvider
                  .getMembershipStatus()
                  .then((vip) => {
                    if (!cancelled) setQQMusicVipStatus(vip);
                  })
                  .catch(() => undefined);
              }
            })
            .catch((error) => {
              if (!cancelled) {
                setQQMusicLoginStatus({
                  loggedIn: false,
                  credentialPresent: false,
                  status: "failed",
                  uin: "",
                  nickname: "",
                  avatarUrl: "",
                  vipType: "none",
                  message: readError(error),
                });
              }
            });
        }
      }

      if (curatorResult.status === "fulfilled") {
        const loaded = curatorResult.value;
        setConfig(loaded);
        setProviderName(loaded.providerName);
        setBaseUrl(loaded.baseUrl);
        setModel(loaded.model);
        setApiKey("");
      }

      const firstFailure = [neteaseResult, bilibiliResult, qqmusicResult, curatorResult].find(
        (result) => result.status === "rejected",
      );
      if (firstFailure?.status === "rejected") setMessage(readError(firstFailure.reason));
      setIsLoading(false);
    })();

    return () => {
      cancelled = true;
    };
  }, [focus, open, refreshBilibiliStatus, refreshNetEaseAccountStatus]);

  useEffect(() => {
    if (!open || !neteaseQr) return;
    // 已进入终态（过期/超时）后停止轮询，但保留弹窗让用户点「重新生成」。
    if (neteaseQrStatus === "expired" || neteaseQrStatus === "timeout") return;

    let cancelled = false;
    let inFlight = false;
    let errorCount = 0;
    const poll = async () => {
      if (inFlight || cancelled) return;
      inFlight = true;
      try {
        // 客户端 max-life 兜底：服务端仅在 QR 真正过期时才回 expired，
        // 但若网络异常或服务端长时间不返回 expired，需在失效前主动停止。
        const elapsed = Date.now() - neteaseQrStartedAtRef.current;
        if (elapsed > NETEASE_QR_MAX_LIFE_MS) {
          setNeteaseQrStatus("timeout");
          setSourceMessage("二维码已超时，请重新生成 / QR timed out, regenerate to try again.");
          return;
        }

        const result = await neteaseAuthProvider.checkQrLoginStatus(neteaseQr.key);
        if (cancelled) return;
        errorCount = 0;

        // 同步本地状态机：confirmed（已扫描待确认）映射为 scanned，其余维持。
        if (result.status === "confirmed") {
          setNeteaseQrStatus("scanned");
        } else if (result.status === "waiting") {
          setNeteaseQrStatus("waiting");
        }
        // unknown 状态保持当前 UI 状态不变，不覆盖 sourceMessage

        if (result.status !== "unknown") {
          setSourceMessage(qrStatusMessage(result.status));
        }
        if (result.loginStatus) {
          setNeteaseLoginStatus(result.loginStatus);
          const [vip, savedMusicSourceConfig] = await Promise.all([
            neteaseAuthProvider.getVipStatus(),
            getNeteaseSourceConfig(),
          ]);
          if (cancelled) return;
          setNeteaseVipStatus(vip);
          setMusicSourceConfig(savedMusicSourceConfig);
          setNeteaseQr(null);
          setNeteaseQrStatus("waiting");
          setSourceMessage(
            result.loginStatus.loggedIn
              ? "Connected to NetEase Cloud Music."
              : result.loginStatus.message,
          );
        } else if (result.status === "expired") {
          // 不关闭弹窗，就地保留并提示用户重新生成。
          setNeteaseQrStatus("expired");
        }
      } catch (error) {
        if (!cancelled) {
          errorCount += 1;
          if (errorCount >= 5) {
            setNeteaseQrStatus("timeout");
            setSourceMessage("网络异常，请重新生成二维码 / Network error, regenerate the QR code.");
          } else {
            setSourceMessage(`Could not check the sign-in code. ${readError(error)}`);
          }
        }
      } finally {
        inFlight = false;
      }
    };

    void poll();
    const timer = window.setInterval(poll, 2200);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, [open, neteaseQr, neteaseQrStatus]);

  useEffect(() => {
    if (!open || !bilibiliQr) return;
    // 已进入终态（过期/超时）后停止轮询，但保留弹窗让用户点「重新生成」。
    if (bilibiliQrStatus === "expired" || bilibiliQrStatus === "timeout") return;

    let cancelled = false;
    let inFlight = false;
    let errorCount = 0;
    const poll = async () => {
      if (cancelled || inFlight) return;
      inFlight = true;
      try {
        // 客户端 max-life 兜底：与 NetEase 同样的防御逻辑。
        const elapsed = Date.now() - bilibiliQrStartedAtRef.current;
        if (elapsed > BILIBILI_QR_MAX_LIFE_MS) {
          setBilibiliQrStatus("timeout");
          setSourceMessage("二维码已超时，请重新生成 / QR timed out, regenerate to try again.");
          return;
        }

        const result = await bilibiliAuthProvider.checkQrLoginStatus(bilibiliQr.key);
        if (cancelled) return;
        errorCount = 0;

        if (result.status === "confirmed") {
          setBilibiliQrStatus("scanned");
        } else if (result.status === "waiting") {
          setBilibiliQrStatus("waiting");
        }
        // unknown 状态保持当前 UI 状态不变，不覆盖 sourceMessage

        if (result.status !== "unknown") {
          setSourceMessage(qrStatusMessage(result.status));
        }
        if (result.loginStatus) {
          const savedConfig = await getBilibiliSourceConfig();
          if (cancelled) return;
          setBilibiliLoginStatus(result.loginStatus);
          setBilibiliConfig(savedConfig);
          setBilibiliQr(null);
          setBilibiliQrStatus("waiting");
          setSourceMessage(
            result.loginStatus.loggedIn
              ? "Bilibili 已连接 / Connected."
              : result.loginStatus.message,
          );
        } else if (result.status === "expired") {
          // 不关闭弹窗，就地保留并提示用户重新生成。
          setBilibiliQrStatus("expired");
        }
      } catch (error) {
        if (!cancelled) {
          errorCount += 1;
          if (errorCount >= 5) {
            setBilibiliQrStatus("timeout");
            setSourceMessage("网络异常，请重新生成二维码 / Network error, regenerate the QR code.");
          } else {
            setSourceMessage(`无法检查 Bilibili 登录状态 / ${readError(error)}`);
          }
        }
      } finally {
        inFlight = false;
      }
    };

    void poll();
    const timer = window.setInterval(poll, 2200);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, [bilibiliQr, open, bilibiliQrStatus]);

  useEffect(() => {
    if (smsCooldown <= 0) return;
    const timer = window.setInterval(() => setSmsCooldown((value) => Math.max(0, value - 1)), 1000);
    return () => window.clearInterval(timer);
  }, [smsCooldown]);

  // NetEase 本地服务状态轮询：仅在来源面板且网易云启用时进行。
  // 到达 ready / failed 终态后停止轮询，避免后台空跑；用户点重试会通过 retryToken 触发重启。
  useEffect(() => {
    if (!open || activeSection !== "sources" || !neteaseEnabled) return;
    let cancelled = false;
    let intervalId: number | null = null;
    const stopPolling = () => {
      if (intervalId !== null) {
        window.clearInterval(intervalId);
        intervalId = null;
      }
    };
    const poll = async () => {
      try {
        const status = await ensureNeteaseApiService();
        if (cancelled) return;
        setNeteaseServiceStatus(status);
        if (status.stage === "ready" || status.stage === "failed") {
          stopPolling();
        }
      } catch (error) {
        if (cancelled) return;
        setNeteaseServiceStatus({
          running: false,
          started: false,
          baseUrl: "",
          message: readError(error),
          nodeAvailable: false,
          apiPackageFound: false,
          stage: "failed",
        });
        stopPolling();
      }
    };
    void poll();
    intervalId = window.setInterval(poll, 2200);
    return () => {
      cancelled = true;
      stopPolling();
    };
  }, [open, activeSection, neteaseEnabled, neteaseServiceRetryToken]);

  useEffect(() => {
    if (!open || activeSection !== "storage") return;
    void getStorageReport()
      .then(setStorageReport)
      .catch((error) =>
        setStorageMessage(`无法读取存储状态 / Could not read storage. ${readError(error)}`),
      );
  }, [activeSection, open]);

  // QQ Music QR polling lives in an effect (with cleanup), mirroring the
  // NetEase/Bilibili QR flows. This guarantees the poll chain dies when the
  // settings panel unmounts, that a repeated "generate" replaces the old QR
  // session, and that terminal states (confirmed / expired / failed) stop the
  // poll immediately — no zombie timeout chain on an unmounted component and
  // no misleading "QR expired" after a failed verification.
  //
  // QR expiry is decided ONLY by the server's 65 status: the frontend never
  // fakes "expired" from a local timer. A locally elapsed poll window just
  // keeps waiting ("polling"), and transient network errors back off and
  // retry instead of destroying the QR.
  // NOTE: must stay ABOVE the `if (!open) return null` guard — hooks after a
  // conditional early-return violate the rules of hooks.
  useEffect(() => {
    if (!open || !qqmusicQr) return;
    if (
      qqmusicQrStatus === "expired" ||
      qqmusicQrStatus === "failed" ||
      qqmusicQrStatus === "confirmed"
    )
      return;

    // Generation guard: every "New Code" bumps this ref through
    // qqmusicQrGenerationRef (in createQQMusicQr). Any async response from
    // an older QR generation is dropped, so a stale poll can never update
    // the new QR's state or overwrite a successful login.
    const generation = qqmusicQrGenerationRef.current;
    let cancelled = false;
    let inFlight = false;
    let consecutiveErrors = 0;

    // Backoff schedule for transient errors: 2s, 2s, 3s, 5s, then capped 5s.
    const backoffDelayMs = () => {
      const delays = [2000, 2000, 3000, 5000];
      return delays[Math.min(consecutiveErrors, delays.length - 1)];
    };

    let timer: number | undefined;

    const scheduleNext = (delayMs: number) => {
      timer = window.setTimeout(() => void poll(), delayMs);
    };

    const poll = async () => {
      if (cancelled || inFlight) return;
      if (qqmusicQrGenerationRef.current !== generation) return;
      inFlight = true;
      try {
        const result = await qqmusicAuthProvider.checkQrLoginStatus(qqmusicQr.key);
        if (cancelled) return;
        if (qqmusicQrGenerationRef.current !== generation) return;
        consecutiveErrors = 0;

        if (result.status === "confirmed") {
          const loginStatus = result.loginStatus;
          if (loginStatus) {
            setQQMusicQrStatus("confirmed");
            setQQMusicLoginStatus(loginStatus);
            setQQMusicMsg(describeQQMusicAuthState(loginStatus, qqmusicEnabled, "detail"));
            if (loginStatus.loggedIn) {
              // Fetch VIP status
              try {
                const vip = await qqmusicAuthProvider.getMembershipStatus();
                if (cancelled) return;
                setQQMusicVipStatus(vip);
              } catch {}
            }
          } else {
            // Rust returned confirmed but the session could not be verified.
            // This is terminal — stop polling and show the real reason
            // instead of retrying indefinitely.
            setQQMusicQrStatus("failed");
            setQQMusicMsg(
              result.message || "登录确认失败，请重新扫码 / Login confirmation failed. Scan again.",
            );
          }
          return;
        }
        if (result.status === "scanned") {
          setQQMusicQrStatus("scanned");
        } else if (result.status === "waiting") {
          setQQMusicQrStatus("waiting");
        } else if (result.status === "expired") {
          // Server-confirmed expiry: the QR is genuinely dead.
          setQQMusicQrStatus("expired");
          return;
        } else if (result.status === "failed") {
          // Backend terminal failure (canceled / rate_limited / network
          // verdict from a definitive source). Surface the real reason.
          setQQMusicQrStatus("failed");
          setQQMusicMsg(result.message || "QR login failed.");
          return;
        }
        scheduleNext(1500);
      } catch (pollErr) {
        if (cancelled) return;
        if (qqmusicQrGenerationRef.current !== generation) return;
        // Transient network failure: back off and keep polling the SAME QR.
        // Never convert a network error into "expired".
        consecutiveErrors += 1;
        if (consecutiveErrors === 1) {
          setQQMusicMsg(`轮询中: ${readError(pollErr)} / Polling: ${readError(pollErr)}`);
        } else {
          setQQMusicMsg(
            `网络波动，继续等待: ${readError(pollErr)} / Transient error, still waiting.`,
          );
        }
        scheduleNext(backoffDelayMs());
      } finally {
        inFlight = false;
      }
    };

    void poll();
    return () => {
      cancelled = true;
      if (timer !== undefined) window.clearTimeout(timer);
    };
  }, [open, qqmusicQr, qqmusicQrStatus, qqmusicEnabled]);

  // 官方 WebView 登录自动收口：后端 watcher 是登录状态的唯一权威，前端只在
  // 登录窗口打开期间轮询展示其状态机，authenticated 时自动刷新登录态并收起
  // 登录 UI——用户扫码确认后无需再点“完成连接 / Check Login”。
  // NOTE: must stay ABOVE the `if (!open) return null` guard — hooks after a
  // conditional early-return violate the rules of hooks.
  useEffect(() => {
    if (!open || !qqmusicWebLoginMode) {
      return;
    }
    const generation = ++qqmusicWebLoginGenerationRef.current;
    qqmusicFlowAuthedRef.current = false;
    let cancelled = false;
    const poll = async () => {
      try {
        const flow = await qqmusicAuthProvider.getLoginFlow();
        if (cancelled || qqmusicWebLoginGenerationRef.current !== generation) return;
        setQQMusicLoginFlow(flow);
        if (flow.status === "authenticated" && !qqmusicFlowAuthedRef.current) {
          qqmusicFlowAuthedRef.current = true;
          // Session already persisted by the backend; refresh the panel's
          // view of it so status/avatar/文案 switch immediately.
          try {
            const status = await qqmusicAuthProvider.getLoginStatus();
            if (!cancelled && qqmusicWebLoginGenerationRef.current === generation) {
              setQQMusicLoginStatus(status);
            }
          } catch {
            /* status refresh is cosmetic; keep the flow's success state */
          }
          await qqmusicAuthProvider.closeWebviewLogin().catch(() => {});
          if (!cancelled) {
            setQQMusicWebLoginMode(null);
            setQQMusicMsg(flow.message || "QQ音乐已连接 / QQ Music connected.");
          }
        } else if (flow.status === "canceled") {
          if (!cancelled) setQQMusicWebLoginMode(null);
        } else if (flow.status === "failed") {
          // Surface the backend's real reason (never a value — masked only).
          if (!cancelled && flow.message) setQQMusicMsg(flow.message);
        }
      } catch {
        /* transient backend error: keep polling, never fake a state */
      }
    };
    void poll();
    const timer = window.setInterval(() => void poll(), 2000);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, [open, qqmusicWebLoginMode]);

  if (!open) {
    return null;
  }

  const fetchModels = async () => {
    setIsFetchingModels(true);
    setMessage(null);
    setModelMessage(null);

    try {
      const models = await fetchLlmModels({
        baseUrl,
        apiKey: apiKey.trim() || undefined,
      });
      setAvailableModels(models);
      if (!model && models[0]) {
        setModel(models[0]);
      }
      setModelMessage(models.length === 1 ? "Found 1 model." : `Found ${models.length} models.`);
    } catch (error) {
      setAvailableModels([]);
      setModelMessage(`Could not fetch models. ${readError(error)}`);
    } finally {
      setIsFetchingModels(false);
    }
  };

  const save = async () => {
    setIsSaving(true);
    setMessage(null);

    try {
      const savedConfig = await saveLlmProviderConfig({
        providerName,
        baseUrl,
        model,
        apiKey: apiKey.trim() || undefined,
      });
      const savedSpeechConfig = saveSpeechProviderConfig(speechConfig);
      setConfig(savedConfig);
      setSpeechConfig(savedSpeechConfig);
      setProviderName(savedConfig.providerName);
      setBaseUrl(savedConfig.baseUrl);
      setModel(savedConfig.model);
      setApiKey("");
      setMessage(
        savedConfig.configured
          ? "Saved. The curator will use this source first."
          : "Saved. Add a key when you are ready.",
      );
    } catch (error) {
      setMessage(readError(error));
    } finally {
      setIsSaving(false);
    }
  };

  const testSource = async () => {
    setTestingSource(true);
    setSourceMessage(null);

    try {
      const result = await testNeteaseSourceConnection({
        enabled: neteaseEnabled,
        baseUrl: neteaseBaseUrl,
        token: neteaseToken.trim() || undefined,
      });
      setSourceMessage(result);
    } catch (error) {
      setSourceMessage(`Could not connect. ${readError(error)}`);
    } finally {
      setTestingSource(false);
    }
  };

  const saveSourceDraft = async (options?: { token?: string; enabled?: boolean }) => {
    // Source-scoped save: never writes Bilibili, QQ Music or curator settings.
    const savedMusicSourceConfig = await saveNeteaseSourceConfig({
      enabled: options?.enabled ?? neteaseEnabled,
      baseUrl: neteaseBaseUrl,
      token: options?.token,
    });
    setMusicSourceConfig(savedMusicSourceConfig);
    setNeteaseEnabled(savedMusicSourceConfig.enabled);
    setNeteaseBaseUrl(savedMusicSourceConfig.baseUrl);
    if (options?.token !== undefined) setNeteaseToken("");
    return savedMusicSourceConfig;
  };

  const saveBilibiliDraft = async (options?: { token?: string; enabled?: boolean }) => {
    // 统一的 Bilibili 草稿保存入口：与 saveSourceDraft 同样的语义。
    const savedConfig = await saveBilibiliSourceConfig({
      enabled: options?.enabled ?? bilibiliEnabled,
      baseUrl: bilibiliBaseUrl,
      token: options?.token,
      searchScope: bilibiliSearchScope,
    });
    setBilibiliConfig(savedConfig);
    setBilibiliEnabled(savedConfig.enabled);
    setBilibiliBaseUrl(savedConfig.baseUrl);
    setBilibiliSearchScope(savedConfig.searchScope);
    if (options?.token !== undefined) setBilibiliToken("");
    return savedConfig;
  };

  // QQ Music source settings are saved independently from every other section.
  const saveQQMusicDraft = async (options?: { token?: string; enabled?: boolean }) => {
    const savedConfig = await saveQQMusicSourceConfig({
      enabled: options?.enabled ?? qqmusicEnabled,
      baseUrl: qqmusicBaseUrl,
      token: options?.token,
    });
    setQQMusicEnabled(savedConfig.enabled);
    setQQMusicBaseUrl(savedConfig.baseUrl);
    if (options?.token !== undefined) setQQMusicToken("");
    return savedConfig;
  };

  const updateNeteaseEnabled = async (enabled: boolean) => {
    const previous = neteaseEnabled;
    setNeteaseEnabled(enabled);
    setSourceMessage(null);
    try {
      await saveSourceDraft({ enabled });
    } catch (error) {
      setNeteaseEnabled(previous);
      setSourceMessage(`无法更新网易云来源 / Could not update NetEase. ${readError(error)}`);
    }
  };

  const updateBilibiliEnabled = async (enabled: boolean) => {
    const previous = bilibiliEnabled;
    setBilibiliEnabled(enabled);
    setSourceMessage(null);
    try {
      await saveBilibiliDraft({ enabled });
    } catch (error) {
      setBilibiliEnabled(previous);
      setSourceMessage(`无法更新 Bilibili 来源 / Could not update Bilibili. ${readError(error)}`);
    }
  };

  const updateQQMusicEnabled = async (enabled: boolean) => {
    const previous = qqmusicEnabled;
    setQQMusicEnabled(enabled);
    setQQMusicMsg(null);
    try {
      await saveQQMusicDraft({ enabled });
    } catch (error) {
      setQQMusicEnabled(previous);
      setQQMusicMsg(`无法更新 QQ 音乐来源 / Could not update QQ Music. ${readError(error)}`);
    }
  };

  const testBilibiliSource = async () => {
    setTestingBilibili(true);
    setSourceMessage(null);
    try {
      const result = await testBilibiliSourceConnection({
        enabled: bilibiliEnabled,
        baseUrl: bilibiliBaseUrl,
        token: bilibiliToken.trim() || undefined,
        searchScope: bilibiliSearchScope,
      });
      setSourceMessage(result);
    } catch (error) {
      setSourceMessage(`Bilibili 连接测试失败 / Bilibili test failed: ${readError(error)}`);
    } finally {
      setTestingBilibili(false);
    }
  };

  const importBilibiliCookie = async () => {
    if (!bilibiliToken.trim()) return;
    setCheckingBilibili(true);
    setSourceMessage(null);
    try {
      // 复用 saveBilibiliDraft，token 由它统一保存并清空输入框
      await saveBilibiliDraft({ token: bilibiliToken.trim() });
      const status = await bilibiliAuthProvider.getLoginStatus();
      setBilibiliLoginStatus(status);
      setSourceMessage(status.loggedIn ? "Connected to Bilibili." : status.message);
    } catch (error) {
      setSourceMessage(`Could not connect Bilibili. ${readError(error)}`);
    } finally {
      setCheckingBilibili(false);
    }
  };

  const createBilibiliQrLogin = async () => {
    setCreatingBilibiliQr(true);
    setSourceMessage(null);
    try {
      await saveBilibiliDraft();
      const qr = await bilibiliAuthProvider.createQrLogin();
      bilibiliQrStartedAtRef.current = Date.now();
      setBilibiliQrStatus("waiting");
      setBilibiliQr(qr);
      setSourceMessage(
        "请使用哔哩哔哩扫码，手机确认后会自动连接。 / Scan with Bilibili and confirm on your phone.",
      );
    } catch (error) {
      setSourceMessage(`无法创建 Bilibili 登录二维码 / ${readError(error)}`);
    } finally {
      setCreatingBilibiliQr(false);
    }
  };

  const logoutBilibili = async () => {
    setCheckingBilibili(true);
    setSourceMessage(null);
    try {
      const status = await bilibiliAuthProvider.logout();
      const savedConfig = await getBilibiliSourceConfig();
      setBilibiliConfig(savedConfig);
      setBilibiliLoginStatus(status);
      setBilibiliQr(null);
      setSourceMessage("Bilibili disconnected.");
    } catch (error) {
      setSourceMessage(`Could not disconnect Bilibili. ${readError(error)}`);
    } finally {
      setCheckingBilibili(false);
    }
  };

  const updateDanmakuSettings = (patch: Partial<DanmakuSettings>) => {
    setDanmakuSettings((value) => saveDanmakuSettings({ ...value, ...patch }));
  };

  const clearDanmaku = async () => {
    setSourceMessage(null);
    try {
      await bilibiliProvider.clearDanmakuCache();
      setSourceMessage("弹幕缓存已清理 / Danmaku cache cleared.");
    } catch (error) {
      setSourceMessage(`Could not clear danmaku cache. ${readError(error)}`);
    }
  };

  const createQrLogin = async () => {
    setCreatingQr(true);
    setSourceMessage(null);
    try {
      await saveSourceDraft();
      const qr = await neteaseAuthProvider.createQrLogin();
      neteaseQrStartedAtRef.current = Date.now();
      setNeteaseQrStatus("waiting");
      setNeteaseQr(qr);
      setSourceMessage(
        "Scan the code with NetEase Cloud Music. This page will connect automatically.",
      );
    } catch (error) {
      setSourceMessage(`Could not create a sign-in code. ${readError(error)}`);
    } finally {
      setCreatingQr(false);
    }
  };

  const importCookie = async () => {
    if (!neteaseToken.trim()) return;
    setCheckingLogin(true);
    setSourceMessage(null);
    try {
      await saveSourceDraft();
      const status = await neteaseAuthProvider.importCookie(neteaseToken.trim());
      setNeteaseLoginStatus(status);
      setNeteaseToken("");
      const vip = await neteaseAuthProvider.getVipStatus();
      setNeteaseVipStatus(vip);
      const savedMusicSourceConfig = await getNeteaseSourceConfig();
      setMusicSourceConfig(savedMusicSourceConfig);
      setSourceMessage(status.loggedIn ? "Connected to NetEase Cloud Music." : status.message);
    } catch (error) {
      setSourceMessage(`Could not import this session. ${readError(error)}`);
    } finally {
      setCheckingLogin(false);
    }
  };

  const finishNetEaseSignIn = async (status: NetEaseLoginStatus) => {
    setNeteaseLoginStatus(status);
    const [vip, savedMusicSourceConfig] = await Promise.all([
      neteaseAuthProvider.getVipStatus(),
      getNeteaseSourceConfig(),
    ]);
    setNeteaseVipStatus(vip);
    setMusicSourceConfig(savedMusicSourceConfig);
    setSourceMessage(status.loggedIn ? "已连接 / Connected." : status.message);
  };

  const loginNetEaseWithPassword = async () => {
    if (!neteaseAccount.trim() || !neteasePassword.trim()) return;
    setPasswordLogin(true);
    setSourceMessage(null);
    try {
      await saveSourceDraft();
      const status = await neteaseAuthProvider.loginWithPassword({
        account: neteaseAccount.trim(),
        password: neteasePassword,
        countryCode: neteaseCountryCode.trim() || "86",
        loginType: neteaseLoginType,
      });
      setNeteasePassword("");
      await finishNetEaseSignIn(status);
    } catch (error) {
      setNeteasePassword("");
      setSourceMessage(loginErrorMessage(error));
    } finally {
      setPasswordLogin(false);
    }
  };

  const requestNetEaseSmsCode = async () => {
    if (!neteasePhone.trim()) return;
    setSendingSms(true);
    setSourceMessage(null);
    try {
      await saveSourceDraft();
      const result = await neteaseAuthProvider.requestSmsCode({
        phone: neteasePhone.trim(),
        countryCode: neteaseCountryCode.trim() || "86",
      });
      setSmsCooldown(60);
      setSourceMessage(result.message);
    } catch (error) {
      setSourceMessage(loginErrorMessage(error));
    } finally {
      setSendingSms(false);
    }
  };

  const loginNetEaseWithSms = async () => {
    if (!neteasePhone.trim() || !neteaseSmsCode.trim()) return;
    setSmsLogin(true);
    setSourceMessage(null);
    try {
      await saveSourceDraft();
      const status = await neteaseAuthProvider.loginWithSmsCode({
        phone: neteasePhone.trim(),
        code: neteaseSmsCode.trim(),
        countryCode: neteaseCountryCode.trim() || "86",
      });
      setNeteaseSmsCode("");
      await finishNetEaseSignIn(status);
    } catch (error) {
      setSourceMessage(loginErrorMessage(error));
    } finally {
      setSmsLogin(false);
    }
  };

  const openSecureWebLogin = async (source: "netease" | "bilibili") => {
    setOpeningWebLoginSource(source);
    setSourceMessage(null);
    try {
      if (source === "netease") {
        await saveSourceDraft();
        const result = await neteaseAuthProvider.openSecureWebLogin();
        setSourceMessage(`${result.message} 登录后可导入 Cookie / Import Cookie after signing in.`);
      } else {
        await saveBilibiliDraft();
        const result = await bilibiliAuthProvider.openSecureWebLogin();
        setSourceMessage(`${result.message} 登录后可导入 Cookie / Import Cookie after signing in.`);
      }
    } catch (error) {
      setSourceMessage(readError(error));
    } finally {
      setOpeningWebLoginSource(null);
    }
  };

  // ── QQ Music QR Login ───────────────────────────────────────────
  const createQQMusicQr = async () => {
    setCreatingQQMusicQr(true);
    setQQMusicMsg(null);
    try {
      // Bump the generation BEFORE swapping in the new QR so any in-flight
      // poll from the previous QR is ignored (stale-response guard).
      qqmusicQrGenerationRef.current += 1;
      const qr = await qqmusicAuthProvider.createQrLogin();
      setQQMusicQr(qr);
      setQQMusicQrStatus("waiting");
    } catch (error) {
      setQQMusicMsg(readError(error));
    } finally {
      setCreatingQQMusicQr(false);
    }
  };

  const refreshLogin = async () => {
    setCheckingLogin(true);
    setSourceMessage(null);
    try {
      const status = await neteaseAuthProvider.refreshLogin();
      setNeteaseLoginStatus(status);
      const [vip, savedMusicSourceConfig] = await Promise.all([
        neteaseAuthProvider.getVipStatus(),
        getNeteaseSourceConfig(),
      ]);
      setNeteaseVipStatus(vip);
      setMusicSourceConfig(savedMusicSourceConfig);
      setSourceMessage(status.message);
    } catch (error) {
      setSourceMessage(`Could not refresh the session. ${readError(error)}`);
    } finally {
      setCheckingLogin(false);
    }
  };

  const logoutSource = async () => {
    setCheckingLogin(true);
    setSourceMessage(null);
    try {
      const status = await neteaseAuthProvider.logout();
      setNeteaseLoginStatus(status);
      setNeteaseVipStatus(null);
      const savedMusicSourceConfig = await getNeteaseSourceConfig();
      setMusicSourceConfig(savedMusicSourceConfig);
      setSourceMessage("Signed out from NetEase Cloud Music.");
    } catch (error) {
      setSourceMessage(`Could not sign out. ${readError(error)}`);
    } finally {
      setCheckingLogin(false);
    }
  };

  const importPlaylistById = async (playlistId: string) => {
    const trimmedPlaylistId = playlistId.trim();
    if (!trimmedPlaylistId) return;
    setImportingPlaylist(true);
    // Mark this row as "reading" so the shelf card shows live per-playlist
    // progress instead of the old global lock. Other rows stay actionable.
    setPlaylistStates((prev) => ({ ...prev, [trimmedPlaylistId]: { status: "reading" } }));
    setSourceMessage(null);

    try {
      await saveSourceDraft();
      const playlist = await neteaseProvider.importPlaylist(trimmedPlaylistId);
      const tracks = await refreshLocalTracksAfterSourceImport();
      onLibraryChanged?.(tracks);
      const at = new Date().toISOString();
      setPlaylistStates((prev) => ({
        ...prev,
        [trimmedPlaylistId]: { status: "imported", trackCount: playlist.tracks.length, at },
      }));
      setImportedRecords((prev) => {
        const next: Record<string, ImportedPlaylistRecord> = {
          ...prev,
          [trimmedPlaylistId]: {
            id: trimmedPlaylistId,
            name: playlist.name,
            trackCount: playlist.tracks.length,
            importedAt: at,
          },
        };
        saveImportedPlaylistRecords(next);
        return next;
      });
      setSourceMessage(`Imported ${playlist.tracks.length} songs from "${playlist.name}".`);
    } catch (error) {
      const reason = readError(error);
      setPlaylistStates((prev) => ({
        ...prev,
        [trimmedPlaylistId]: { status: "failed", reason, at: new Date().toISOString() },
      }));
      setSourceMessage(`Import failed. ${reason}`);
    } finally {
      setImportingPlaylist(false);
    }
  };

  const importPlaylist = async () => {
    await importPlaylistById(neteasePlaylistId);
  };

  const loadUserPlaylists = async () => {
    setLoadingUserPlaylists(true);
    setSourceMessage(null);

    try {
      await saveSourceDraft();
      const playlists = await neteaseProvider.getUserPlaylists();
      setNeteaseUserPlaylists(playlists);
      setSourceMessage(
        playlists.length ? `Found ${playlists.length} playlists.` : "No playlists found.",
      );
    } catch (error) {
      setNeteaseUserPlaylists([]);
      setSourceMessage(`Could not load playlists. ${readError(error)}`);
    } finally {
      setLoadingUserPlaylists(false);
    }
  };

  const syncListeningMemory = async (includePlaylists = false) => {
    setSyncingMemory(true);
    setSourceMessage(null);

    try {
      await saveSourceDraft();
      const result = await neteaseProvider.syncListeningMemory({
        includeLikedSongs: true,
        includePlaylists,
        playlistIds:
          includePlaylists && neteaseUserPlaylists.length
            ? neteaseUserPlaylists.map((playlist) => playlist.id)
            : undefined,
        likedLimit: 200,
      });
      setTasteNotes(result.tasteNotes);
      const tracks = await refreshLocalTracksAfterSourceImport();
      onLibraryChanged?.(tracks);
      const syncedAt = new Date().toISOString();
      setLikedSyncedAt(syncedAt);
      saveLikedSyncedAt(syncedAt);
      setSourceMessage(
        includePlaylists
          ? `Imported ${result.likedCount} liked songs and ${result.playlistCount} playlists into this library. Taste notes are ready.`
          : `Imported ${result.likedCount} liked songs into this library. Taste notes are ready.`,
      );
    } catch (error) {
      setSourceMessage(`Could not sync listening memory. ${readError(error)}`);
    } finally {
      setSyncingMemory(false);
    }
  };

  const refreshStorageReport = async () => {
    try {
      const report = await getStorageReport();
      setStorageReport(report);
    } catch (error) {
      setStorageMessage(`无法读取存储状态 / Could not read storage. ${readError(error)}`);
    }
  };

  const clearStorage = async (kind: StorageBucketKind) => {
    setClearingStorageKind(kind);
    setStorageMessage(null);
    try {
      const report = await clearStorageBucket(kind);
      setStorageReport(report);
      setStorageMessage("已清理所选缓存 / Selected cache cleared.");
    } catch (error) {
      setStorageMessage(`清理失败 / Could not clear cache. ${readError(error)}`);
    } finally {
      setClearingStorageKind(null);
    }
  };

  const exportDiagnostics = async () => {
    setStorageMessage(null);
    try {
      const text = await exportStorageDiagnostics();
      const blob = new Blob([text], { type: "text/plain;charset=utf-8" });
      const url = URL.createObjectURL(blob);
      const anchor = document.createElement("a");
      anchor.href = url;
      anchor.download = `ome-storage-diagnostics-${new Date().toISOString().slice(0, 10)}.txt`;
      anchor.click();
      URL.revokeObjectURL(url);
      setStorageMessage("诊断报告已导出 / Diagnostics exported.");
    } catch (error) {
      setStorageMessage(`导出失败 / Could not export diagnostics. ${readError(error)}`);
    }
  };

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/38 px-5 backdrop-blur-2xl"
      role="dialog"
      aria-modal="true"
      aria-label="设置 / Settings"
      onClick={onClose}
      onKeyDown={(event) => {
        if (event.key === "Escape") onClose();
      }}
    >
      <section
        className="settings-panel flex h-[90vh] max-h-[90vh] w-full max-w-6xl flex-col overflow-hidden rounded-[28px] border border-white/[0.08] bg-[#17120f]/90 shadow-[0_34px_110px_rgba(0,0,0,0.52)]"
        onClick={(event) => event.stopPropagation()}
      >
        <div className="flex shrink-0 items-start justify-between gap-5 px-6 pb-5 pt-6">
          <div>
            <p className="text-xs font-semibold uppercase tracking-[0.28em] text-white/34">
              Ome Settings
            </p>
            <h2 className="mt-2 text-3xl font-semibold text-white sm:text-4xl">全局设置</h2>
            <p className="mt-2 max-w-lg text-sm leading-6 text-white/42">
              让来源、声音与播放习惯各归其位。
            </p>
          </div>
          <button
            type="button"
            onClick={onClose}
            className="app-transition flex h-10 w-10 shrink-0 items-center justify-center rounded-full bg-white/[0.06] text-white/58 hover:bg-white/[0.12] hover:text-white"
            aria-label="Close curator settings"
          >
            <X className="h-4 w-4" />
          </button>
        </div>

        <div className="grid min-h-0 flex-1 md:grid-cols-[210px_minmax(0,1fr)]">
          <nav className="settings-nav hidden min-h-0 overflow-y-auto overscroll-contain border-r border-white/[0.06] px-3 pb-5 md:block">
            <div className="space-y-1">
              {settingsSections.map((section) => {
                const Icon = section.icon;
                return (
                  <button
                    key={section.id}
                    type="button"
                    onClick={() => setActiveSection(section.id)}
                    className={clsx(
                      "app-transition flex w-full items-center gap-3 rounded-[14px] px-3 py-2.5 text-left",
                      activeSection === section.id
                        ? "bg-white/[0.1] text-white"
                        : "text-white/48 hover:bg-white/[0.055] hover:text-white/78",
                    )}
                  >
                    <Icon className="h-4 w-4 shrink-0" />
                    <span className="min-w-0">
                      <span className="block text-sm font-semibold">{section.title}</span>
                      <span className="block text-[10px] uppercase tracking-[0.12em] opacity-50">
                        {section.subtitle}
                      </span>
                    </span>
                  </button>
                );
              })}
            </div>
          </nav>

          <div className="settings-scroll min-h-0 overflow-y-auto overscroll-contain px-5 pb-6 md:px-6">
            <div className="sticky top-0 z-10 -mx-1 mb-4 bg-[#17120f]/90 px-1 pb-3 pt-1 backdrop-blur-xl md:hidden">
              <select
                value={activeSection}
                onChange={(event) => setActiveSection(event.target.value as SettingsSection)}
                className="settings-input appearance-none"
              >
                {settingsSections.map((section) => (
                  <option
                    key={section.id}
                    value={section.id}
                    className="bg-graphite-950 text-white"
                  >
                    {section.title} / {section.subtitle}
                  </option>
                ))}
              </select>
            </div>
            {isLoading ? (
              <div className="flex h-64 items-center justify-center text-white/52">
                <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                Loading settings
              </div>
            ) : (
              <div className="space-y-4">
                {activeSection === "overview" && (
                  <div className="space-y-5">
                    <SettingsIntro
                      title="快速开始"
                      subtitle="Connect a source, confirm playback, then leave the rest for later."
                    />
                    <div className="settings-surface space-y-1">
                      <GuideStep
                        number="01"
                        title="连接音乐来源"
                        subtitle="Required"
                        detail="本地音乐、网易云或 Bilibili，至少启用一个来源。"
                        required
                      />
                      <GuideStep
                        number="02"
                        title="确认播放音质"
                        subtitle="Required"
                        detail="先播放一首歌；需要时再调整音质。"
                        required
                      />
                      <GuideStep
                        number="03"
                        title="鉴赏家与声音"
                        subtitle="Optional"
                        detail="私人选曲和语音均为可选，不影响基础播放。"
                      />
                    </div>
                    <div className="grid gap-3 sm:grid-cols-2 xl:grid-cols-4">
                      <StatusTile
                        icon={Library}
                        title="Local Library"
                        subtitle="本地音乐"
                        value="Ready"
                      />
                      <StatusTile
                        icon={Cloud}
                        title="NetEase"
                        subtitle="网易云"
                        value={
                          neteaseLoginStatus?.loggedIn
                            ? "Signed in"
                            : neteaseEnabled
                              ? "Ready"
                              : "Off"
                        }
                        muted={!neteaseEnabled}
                      />
                      <StatusTile
                        icon={Music2}
                        title="Bilibili"
                        subtitle="B站"
                        value={
                          bilibiliLoginStatus?.loggedIn
                            ? "Signed in"
                            : bilibiliEnabled
                              ? "Ready"
                              : "Off"
                        }
                        muted={!bilibiliEnabled}
                      />
                      <StatusTile
                        icon={Music2}
                        title="QQ Music"
                        subtitle="QQ音乐"
                        value={describeQQMusicAuthState(
                          qqmusicLoginStatus,
                          qqmusicEnabled,
                          "compact",
                        )}
                        muted={!qqmusicEnabled}
                      />
                      <StatusTile
                        icon={Radio}
                        title="Curator"
                        subtitle="鉴赏家"
                        value={config.configured ? "Connected" : "Local"}
                        muted={!config.configured}
                      />
                    </div>
                    <div className="grid gap-3 sm:grid-cols-2 xl:grid-cols-3">
                      <OverviewLink
                        icon={Cloud}
                        title="管理音乐来源"
                        subtitle="Music Sources"
                        onClick={() => setActiveSection("sources")}
                      />
                      <OverviewLink
                        icon={Volume2}
                        title="调整播放音质"
                        subtitle="Playback"
                        onClick={() => setActiveSection("playback")}
                      />
                      <OverviewLink
                        icon={KeyRound}
                        title="鉴赏家与声音"
                        subtitle="Curator & Voice"
                        onClick={() => setActiveSection("curator")}
                      />
                    </div>
                  </div>
                )}

                {activeSection === "curator" && (
                  <>
                    <SettingsIntro title="鉴赏家与声音" subtitle="Curator & Voice" />
                    <SectionLabel icon={KeyRound} title="Music Understanding" subtitle="音乐理解" />
                    <Field label="Provider Name / 供应商">
                      <input
                        value={providerName}
                        onChange={(event) => setProviderName(event.target.value)}
                        placeholder="Custom Provider / DeepSeek / Local Gateway"
                        className="settings-input"
                      />
                    </Field>

                    <Field label="Base URL / 接入地址">
                      <input
                        value={baseUrl}
                        onChange={(event) => {
                          setBaseUrl(event.target.value);
                          setAvailableModels([]);
                          setModelMessage(null);
                        }}
                        placeholder="https://provider.example/v1"
                        className="settings-input"
                      />
                    </Field>

                    <Field label="API Key / 密钥">
                      <input
                        value={apiKey}
                        onChange={(event) => setApiKey(event.target.value)}
                        placeholder={
                          config.hasApiKey
                            ? `${config.maskedApiKey} saved; leave blank to keep it`
                            : "Stored securely on this device"
                        }
                        type="password"
                        className="settings-input"
                        autoComplete="off"
                      />
                    </Field>

                    <Field label="Model / 模型">
                      <div className="grid gap-3 sm:grid-cols-[1fr_auto]">
                        <input
                          value={model}
                          onChange={(event) => setModel(event.target.value)}
                          placeholder="Select or type a model"
                          className="settings-input"
                          list="ome-curator-models"
                        />
                        <datalist id="ome-curator-models">
                          {availableModels.map((modelId) => (
                            <option key={modelId} value={modelId} />
                          ))}
                        </datalist>
                        <button
                          type="button"
                          onClick={fetchModels}
                          disabled={
                            isFetchingModels ||
                            !baseUrl.trim() ||
                            (!apiKey.trim() && !config.hasApiKey)
                          }
                          className="app-transition inline-flex h-12 items-center justify-center gap-2 rounded-[18px] bg-white/[0.08] px-4 text-sm font-semibold text-white/78 hover:bg-white/[0.13] hover:text-white disabled:cursor-not-allowed disabled:opacity-45"
                        >
                          {isFetchingModels ? (
                            <Loader2 className="h-4 w-4 animate-spin" />
                          ) : (
                            <RefreshCw className="h-4 w-4" />
                          )}
                          Test & Fetch / 测试并获取
                        </button>
                      </div>
                    </Field>
                  </>
                )}

                {activeSection === "curator" && modelMessage && (
                  <p className="rounded-[18px] bg-white/[0.045] px-4 py-3 text-sm leading-6 text-white/54">
                    {modelMessage}
                  </p>
                )}

                {activeSection === "curator" && (
                  <SectionLabel icon={Mic2} title="Voice Booth" subtitle="声音间" />
                )}

                {activeSection === "curator" && (
                  <div className="grid gap-4 pt-2 sm:grid-cols-2">
                    <Field label="STT Provider / 听写来源">
                      <select
                        value={speechConfig.sttProvider}
                        onChange={(event) =>
                          setSpeechConfig((value) => ({
                            ...value,
                            sttProvider:
                              event.target.value === "browser" || event.target.value === "off"
                                ? event.target.value
                                : "curator",
                          }))
                        }
                        className="settings-input appearance-none"
                      >
                        <option value="curator" className="bg-graphite-950 text-white">
                          Curator source / 鉴赏来源
                        </option>
                        <option value="off" className="bg-graphite-950 text-white">
                          Text only / 仅文字
                        </option>
                        <option value="browser" className="bg-graphite-950 text-white">
                          System microphone / 系统麦克风
                        </option>
                      </select>
                    </Field>

                    <Field label="TTS Provider / 朗读来源">
                      <select
                        value={speechConfig.ttsProvider}
                        onChange={(event) =>
                          setSpeechConfig((value) => ({
                            ...value,
                            ttsProvider:
                              event.target.value === "browser" || event.target.value === "off"
                                ? event.target.value
                                : "curator",
                          }))
                        }
                        className="settings-input appearance-none"
                      >
                        <option value="curator" className="bg-graphite-950 text-white">
                          Curator source / 鉴赏来源
                        </option>
                        <option value="off" className="bg-graphite-950 text-white">
                          Silent replies / 静音回复
                        </option>
                        <option value="browser" className="bg-graphite-950 text-white">
                          System voice / 系统声音
                        </option>
                      </select>
                    </Field>
                  </div>
                )}

                {activeSection === "curator" && (
                  <div className="grid gap-4 sm:grid-cols-2">
                    <Field label="STT Model / 听写模型">
                      <input
                        value={speechConfig.sttModel ?? ""}
                        onChange={(event) =>
                          setSpeechConfig((value) => ({ ...value, sttModel: event.target.value }))
                        }
                        placeholder="FunAudioLLM/SenseVoiceSmall"
                        className="settings-input"
                        disabled={speechConfig.sttProvider !== "curator"}
                      />
                    </Field>

                    <Field label="TTS Model / 朗读模型">
                      <input
                        value={speechConfig.ttsModel ?? ""}
                        onChange={(event) =>
                          setSpeechConfig((value) => ({ ...value, ttsModel: event.target.value }))
                        }
                        placeholder="FunAudioLLM/CosyVoice2-0.5B"
                        className="settings-input"
                        disabled={speechConfig.ttsProvider !== "curator"}
                      />
                    </Field>
                  </div>
                )}

                {activeSection === "curator" && (
                  <div className="grid gap-4 sm:grid-cols-[1fr_auto] sm:items-end">
                    <Field label="Voice / 声线">
                      <select
                        value={speechConfig.voice}
                        onChange={(event) =>
                          setSpeechConfig((value) => ({ ...value, voice: event.target.value }))
                        }
                        className="settings-input appearance-none"
                        disabled={speechConfig.ttsProvider === "off"}
                      >
                        <option
                          value="FunAudioLLM/CosyVoice2-0.5B:alex"
                          className="bg-graphite-950 text-white"
                        >
                          Vintage British default / 复古英伦
                        </option>
                        {voices.map((voice) => (
                          <option
                            key={voice.id}
                            value={voice.id}
                            className="bg-graphite-950 text-white"
                          >
                            {voice.name} · {voice.lang}
                          </option>
                        ))}
                      </select>
                    </Field>

                    <label className="app-transition flex h-12 items-center gap-3 rounded-[18px] bg-white/[0.055] px-4 text-sm text-white/62 hover:bg-white/[0.08]">
                      <input
                        type="checkbox"
                        checked={speechConfig.languageDetection}
                        onChange={(event) =>
                          setSpeechConfig((value) => ({
                            ...value,
                            languageDetection: event.target.checked,
                          }))
                        }
                        className="h-4 w-4 accent-white"
                      />
                      Language Detection / 语言识别
                    </label>
                  </div>
                )}

                {activeSection === "playback" && (
                  <div className="space-y-5">
                    <SettingsIntro
                      title="播放与声音"
                      subtitle="Playback that stays out of the way."
                    />
                    <div className="settings-surface space-y-4">
                      <SectionLabel icon={Volume2} title="Playback Quality" subtitle="播放音质" />
                      <div className="grid gap-2 sm:grid-cols-5">
                        {(["standard", "higher", "exhigh", "lossless", "hires"] as const).map(
                          (quality) => (
                            <button
                              key={quality}
                              type="button"
                              onClick={() => onPlaybackQualityChange(quality)}
                              className={clsx(
                                "app-transition rounded-[14px] px-3 py-3 text-sm font-semibold capitalize",
                                playbackQuality === quality
                                  ? "bg-white text-[#211813]"
                                  : "bg-white/[0.055] text-white/52 hover:bg-white/[0.1] hover:text-white",
                              )}
                            >
                              {quality}
                            </button>
                          ),
                        )}
                      </div>
                      <p className="text-xs leading-5 text-white/34">
                        音质会按当前来源与账号权限自动回落，不中断播放。
                      </p>
                    </div>
                    <div className="settings-surface grid gap-3 sm:grid-cols-2">
                      <StatusLine
                        label="Local Playback / 本地播放"
                        value="Direct from your library"
                      />
                      <StatusLine
                        label="Voice / 声音"
                        value={
                          speechConfig.ttsProvider === "off"
                            ? "Text only"
                            : speechConfig.ttsProvider === "browser"
                              ? "System voice"
                              : "Curator voice"
                        }
                      />
                    </div>
                  </div>
                )}

                {activeSection === "atmosphere" && (
                  <div className="space-y-5">
                    <SettingsIntro
                      title="弹幕氛围"
                      subtitle="A quiet layer around the music, never over it."
                    />
                    <DanmakuSettingsCard
                      settings={danmakuSettings}
                      onChange={updateDanmakuSettings}
                      onClear={clearDanmaku}
                    />
                  </div>
                )}

                {activeSection === "advanced" && (
                  <div className="space-y-5">
                    <SettingsIntro title="高级设置" subtitle="Diagnostics and quiet maintenance." />
                    <div className="settings-surface">
                      <p className="text-sm font-semibold text-white/74">Local-first / 本地优先</p>
                      <p className="mt-2 text-xs leading-6 text-white/36">
                        本地曲目只保存路径，不复制音频；流媒体默认不下载整首歌曲；敏感配置不会在界面中明文展示。
                        Local songs are never copied; streaming is not downloaded by default;
                        sensitive values stay masked.
                      </p>
                    </div>
                    <MusicSourceDebugCard
                      neteasePlaybackDebug={neteasePlaybackDebug}
                      neteaseServiceStatus={neteaseServiceStatus}
                      neteaseLoginStatus={neteaseLoginStatus}
                      bilibiliLoginStatus={bilibiliLoginStatus}
                      bilibiliDanmakuDebug={bilibiliDanmakuDebug}
                      musicSourceConfig={musicSourceConfig}
                    />
                  </div>
                )}

                {activeSection === "guide" && (
                  <div className="space-y-5">
                    <SettingsIntro
                      title="使用指南"
                      subtitle="How to keep the room quiet and the music first."
                    />
                    <div className="settings-surface space-y-4">
                      <GuideTopic title="导入本地音乐" subtitle="Import local music">
                        在主界面顶部搜索框聚焦后，点击出现的「Choose Music
                        Folder」按钮，选择一个音频文件夹。Ome Music 只记录文件路径，不复制原始音频。
                      </GuideTopic>
                      <GuideTopic title="连接音乐源" subtitle="Connect a music source">
                        前往「音乐来源」连接网易云或 Bilibili。网易云支持扫码与 Cookie
                        导入；Bilibili
                        公共内容可直接使用，登录后可访问更多内容。凭据保存在系统钥匙串，不会明文落盘。
                      </GuideTopic>
                      <GuideTopic title="搜索与播放" subtitle="Search and play">
                        顶部搜索框同时检索本地库与已连接的远端源。点击结果即可播放；远端曲目会先解析可播放地址，失败时会给出原因。
                      </GuideTopic>
                      <GuideTopic title="歌词与偏移" subtitle="Lyrics and timing">
                        默认自动匹配歌词。在快捷设置（右上齿轮）可重新匹配、导入 .lrc 文件，或以
                        ±500ms 微调时间轴。
                      </GuideTopic>
                      <GuideTopic title="弹幕氛围" subtitle="Danmaku atmosphere">
                        Bilibili
                        曲目可显示视频氛围层与弹幕。弹幕会避让封面、标题、歌词核心区与播放控件。在「弹幕氛围」中调整模式、密度、速度与情绪强度。
                      </GuideTopic>
                      <GuideTopic title="Ome Radio 与鉴赏家" subtitle="Radio and curator">
                        左栏 Ome Radio 依据听歌记忆生成私人电台；右栏 DJ
                        鉴赏家可语音或文字点歌、生成歌单。DJ
                        始终以克制的英文回应，工具调用执行音乐操作。
                      </GuideTopic>
                    </div>
                    <div className="settings-surface">
                      <p className="text-sm font-semibold text-white/74">Music First.</p>
                      <p className="mt-2 text-xs leading-6 text-white/36">
                        所有高级能力都在背后工作，不抢主视觉。当不确定时，让它保持安静。
                      </p>
                    </div>
                    {onRestartOnboarding && (
                      <button
                        type="button"
                        onClick={() => {
                          onRestartOnboarding();
                          onClose();
                        }}
                        className="app-transition flex w-full items-center gap-3 rounded-[16px] bg-white/[0.04] px-4 py-3 text-left text-white/52 hover:bg-white/[0.08] hover:text-white/82"
                      >
                        <Sparkles className="h-4 w-4 shrink-0" />
                        <span className="flex-1 text-xs font-bold">
                          重新查看新手引导 / Replay onboarding
                        </span>
                        <ChevronDown className="h-3.5 w-3.5 -rotate-90 opacity-50" />
                      </button>
                    )}
                  </div>
                )}

                {activeSection === "sources" && (
                  <div className="space-y-4">
                    <SettingsIntro
                      title="音乐来源"
                      subtitle="Every shelf you listen from, in one place."
                    />
                    <div className="settings-surface flex items-center justify-between gap-4">
                      <div className="flex items-center gap-3">
                        <span className="flex h-9 w-9 items-center justify-center rounded-full bg-white/[0.07] text-white/64">
                          <Library className="h-4 w-4" />
                        </span>
                        <div>
                          <p className="text-sm font-semibold text-white/84">
                            本地音乐 / Local Library
                          </p>
                          <p className="mt-1 text-xs text-white/36">
                            通过主界面搜索框导入，不复制原始文件。
                          </p>
                        </div>
                      </div>
                      <span className="quick-settings-pill">Ready</span>
                    </div>
                    <div className="settings-surface space-y-4">
                      <div className="flex items-center justify-between gap-4">
                        <div>
                          <div className="flex items-center gap-3">
                            <span className="flex h-9 w-9 items-center justify-center rounded-full bg-white/[0.07] text-white/64">
                              <Cloud className="h-4 w-4" />
                            </span>
                            <div>
                              <h3 className="text-base font-semibold text-white">
                                NetEase Cloud Music / 网易云音乐
                              </h3>
                              <p className="mt-1 text-sm leading-6 text-white/42">
                                Bring in playlists and playable songs. 导入歌单与可播放曲目。
                              </p>
                            </div>
                          </div>
                        </div>
                        <label className="flex items-center gap-3 text-sm text-white/64">
                          <input
                            type="checkbox"
                            checked={neteaseEnabled}
                            onChange={(event) => void updateNeteaseEnabled(event.target.checked)}
                            className="h-4 w-4 accent-white"
                          />
                          Enable NetEase / 启用网易云
                        </label>
                      </div>

                      <Field label="API Base URL / 来源地址">
                        <input
                          value={neteaseBaseUrl}
                          onChange={(event) => setNeteaseBaseUrl(event.target.value)}
                          placeholder="http://127.0.0.1:3000"
                          className="settings-input"
                        />
                      </Field>

                      <Field label="Cookie / Token / 登录凭据">
                        <input
                          value={neteaseToken}
                          onChange={(event) => setNeteaseToken(event.target.value)}
                          placeholder={
                            musicSourceConfig.hasToken
                              ? `${musicSourceConfig.maskedToken} saved; leave blank to keep it`
                              : "Optional"
                          }
                          type="password"
                          className="settings-input"
                          autoComplete="off"
                        />
                      </Field>

                      <div className="grid gap-3 sm:grid-cols-3">
                        <button
                          type="button"
                          onClick={() => {
                            void saveSourceDraft()
                              .then(() =>
                                setSourceMessage("网易云来源设置已保存 / NetEase source saved."),
                              )
                              .catch((error) =>
                                setSourceMessage(
                                  `无法保存网易云来源 / Could not save NetEase. ${readError(error)}`,
                                ),
                              );
                          }}
                          disabled={neteaseEnabled && !neteaseBaseUrl.trim()}
                          className="app-transition inline-flex h-11 items-center justify-center gap-2 rounded-full bg-white/[0.08] px-4 text-sm font-semibold text-white/72 hover:bg-white/[0.13] hover:text-white disabled:cursor-not-allowed disabled:opacity-45"
                        >
                          <Save className="h-4 w-4" />
                          保存来源 / Save Source
                        </button>
                        <button
                          type="button"
                          onClick={createQrLogin}
                          disabled={isCreatingQr || !neteaseEnabled || !neteaseBaseUrl.trim()}
                          className="app-transition inline-flex h-11 items-center justify-center gap-2 rounded-full bg-white/[0.08] px-4 text-sm font-semibold text-white/72 hover:bg-white/[0.13] hover:text-white disabled:cursor-not-allowed disabled:opacity-45"
                        >
                          {isCreatingQr ? (
                            <Loader2 className="h-4 w-4 animate-spin" />
                          ) : (
                            <QrCode className="h-4 w-4" />
                          )}
                          {neteaseQr ? "重新生成 / New Code" : "扫码登录 / QR Login"}
                        </button>
                        <button
                          type="button"
                          onClick={importCookie}
                          disabled={isCheckingLogin || !neteaseEnabled || !neteaseToken.trim()}
                          className="app-transition inline-flex h-11 items-center justify-center gap-2 rounded-full bg-white/[0.08] px-4 text-sm font-semibold text-white/72 hover:bg-white/[0.13] hover:text-white disabled:cursor-not-allowed disabled:opacity-45"
                        >
                          {isCheckingLogin ? (
                            <Loader2 className="h-4 w-4 animate-spin" />
                          ) : (
                            <ShieldCheck className="h-4 w-4" />
                          )}
                          Import Cookie / 导入凭据
                        </button>
                      </div>

                      {neteaseQr && (
                        <div className="grid gap-4 rounded-[20px] bg-white/[0.04] p-4 sm:grid-cols-[128px_1fr]">
                          {neteaseQr.qrImg ? (
                            <img
                              src={neteaseQr.qrImg}
                              alt="NetEase QR Code"
                              className="h-32 w-32 rounded-[16px] bg-white p-2"
                            />
                          ) : (
                            <div className="flex h-32 w-32 items-center justify-center rounded-[16px] bg-white/[0.06] text-xs text-white/40">
                              二维码加载失败 / QR unavailable
                            </div>
                          )}
                          <div className="flex flex-col justify-center">
                            <p className="text-sm font-semibold text-white/80">
                              Scan with NetEase / 使用网易云扫码
                            </p>
                            <p className="mt-2 text-sm leading-6 text-white/42">
                              {neteaseQrStatus === "waiting" && "等待扫码确认… / Waiting for scan…"}
                              {neteaseQrStatus === "scanned" &&
                                "已扫描，请在手机上确认 / Scanned. Confirm on your phone."}
                              {neteaseQrStatus === "expired" &&
                                "二维码已过期，请重新生成 / QR code expired. Regenerate to try again."}
                              {neteaseQrStatus === "timeout" &&
                                "等待超时，请重新生成 / Timed out. Regenerate to try again."}
                            </p>
                            {(neteaseQrStatus === "expired" || neteaseQrStatus === "timeout") && (
                              <button
                                type="button"
                                onClick={createQrLogin}
                                disabled={isCreatingQr || !neteaseEnabled || !neteaseBaseUrl.trim()}
                                className="app-transition mt-3 inline-flex h-9 w-fit items-center justify-center gap-2 rounded-full bg-white/[0.1] px-4 text-xs font-semibold text-white/72 hover:bg-white/[0.18] hover:text-white disabled:cursor-not-allowed disabled:opacity-45"
                              >
                                {isCreatingQr ? (
                                  <Loader2 className="h-3.5 w-3.5 animate-spin" />
                                ) : (
                                  <RefreshCw className="h-3.5 w-3.5" />
                                )}
                                重新生成 / Regenerate
                              </button>
                            )}
                          </div>
                        </div>
                      )}

                      <div className="rounded-[20px] bg-white/[0.035]">
                        <button
                          type="button"
                          onClick={() => setOtherLoginOpen((value) => !value)}
                          className="app-transition flex w-full items-center justify-between gap-3 px-4 py-3 text-left hover:bg-white/[0.035]"
                        >
                          <span>
                            <span className="block text-sm font-semibold text-white/78">
                              其他登录方式 / Other sign-in methods
                            </span>
                            <span className="mt-1 block text-xs text-white/34">
                              扫码优先；账号、短信和网页登录作为备用。
                            </span>
                          </span>
                          <ChevronDown
                            className={`h-4 w-4 text-white/40 transition ${isOtherLoginOpen ? "rotate-180" : ""}`}
                          />
                        </button>

                        {isOtherLoginOpen && (
                          <div className="space-y-4 border-t border-white/[0.06] p-4">
                            <div className="grid gap-3 sm:grid-cols-[0.7fr_1fr_1fr_auto]">
                              <Field label="Type / 类型">
                                <select
                                  value={neteaseLoginType}
                                  onChange={(event) =>
                                    setNeteaseLoginType(event.target.value as "phone" | "email")
                                  }
                                  className="settings-input appearance-none"
                                >
                                  <option value="phone" className="bg-graphite-950 text-white">
                                    Phone / 手机
                                  </option>
                                  <option value="email" className="bg-graphite-950 text-white">
                                    Email / 邮箱
                                  </option>
                                </select>
                              </Field>
                              <Field label="Account / 账号">
                                <input
                                  value={neteaseAccount}
                                  onChange={(event) => setNeteaseAccount(event.target.value)}
                                  placeholder={
                                    neteaseLoginType === "phone"
                                      ? "Phone number / 手机号"
                                      : "Email / 邮箱"
                                  }
                                  className="settings-input"
                                  autoComplete="username"
                                />
                              </Field>
                              <Field label="Password / 密码">
                                <input
                                  value={neteasePassword}
                                  onChange={(event) => setNeteasePassword(event.target.value)}
                                  placeholder="Only used once / 不会保存"
                                  type="password"
                                  className="settings-input"
                                  autoComplete="current-password"
                                />
                              </Field>
                              <button
                                type="button"
                                onClick={loginNetEaseWithPassword}
                                disabled={
                                  isPasswordLogin ||
                                  !neteaseEnabled ||
                                  !neteaseAccount.trim() ||
                                  !neteasePassword.trim()
                                }
                                className="app-transition mt-6 inline-flex h-12 items-center justify-center gap-2 rounded-[18px] bg-white/[0.08] px-4 text-sm font-semibold text-white/70 hover:bg-white/[0.13] hover:text-white disabled:cursor-not-allowed disabled:opacity-45"
                              >
                                {isPasswordLogin ? (
                                  <Loader2 className="h-4 w-4 animate-spin" />
                                ) : (
                                  <KeyRound className="h-4 w-4" />
                                )}
                                Sign in / 登录
                              </button>
                            </div>

                            <div className="grid gap-3 sm:grid-cols-[0.45fr_1fr_1fr_auto_auto]">
                              <Field label="Code / 区号">
                                <input
                                  value={neteaseCountryCode}
                                  onChange={(event) => setNeteaseCountryCode(event.target.value)}
                                  placeholder="86"
                                  className="settings-input"
                                />
                              </Field>
                              <Field label="Phone / 手机号">
                                <input
                                  value={neteasePhone}
                                  onChange={(event) => setNeteasePhone(event.target.value)}
                                  placeholder="138****1234"
                                  className="settings-input"
                                  autoComplete="tel"
                                />
                              </Field>
                              <Field label="SMS Code / 短信验证码">
                                <input
                                  value={neteaseSmsCode}
                                  onChange={(event) => setNeteaseSmsCode(event.target.value)}
                                  placeholder="Enter code / 输入验证码"
                                  className="settings-input"
                                  inputMode="numeric"
                                />
                              </Field>
                              <button
                                type="button"
                                onClick={requestNetEaseSmsCode}
                                disabled={
                                  isSendingSms ||
                                  smsCooldown > 0 ||
                                  !neteaseEnabled ||
                                  !neteasePhone.trim()
                                }
                                className="app-transition mt-6 inline-flex h-12 items-center justify-center gap-2 rounded-[18px] bg-white/[0.08] px-4 text-sm font-semibold text-white/70 hover:bg-white/[0.13] hover:text-white disabled:cursor-not-allowed disabled:opacity-45"
                              >
                                {isSendingSms ? (
                                  <Loader2 className="h-4 w-4 animate-spin" />
                                ) : (
                                  <RefreshCw className="h-4 w-4" />
                                )}
                                {smsCooldown > 0 ? `${smsCooldown}s` : "Send / 发送"}
                              </button>
                              <button
                                type="button"
                                onClick={loginNetEaseWithSms}
                                disabled={
                                  isSmsLogin ||
                                  !neteaseEnabled ||
                                  !neteasePhone.trim() ||
                                  !neteaseSmsCode.trim()
                                }
                                className="app-transition mt-6 inline-flex h-12 items-center justify-center gap-2 rounded-[18px] bg-white/[0.08] px-4 text-sm font-semibold text-white/70 hover:bg-white/[0.13] hover:text-white disabled:cursor-not-allowed disabled:opacity-45"
                              >
                                {isSmsLogin ? (
                                  <Loader2 className="h-4 w-4 animate-spin" />
                                ) : (
                                  <ShieldCheck className="h-4 w-4" />
                                )}
                                Verify / 验证
                              </button>
                            </div>

                            <div className="h-5 text-xs text-[color:var(--settings-text-muted)]">
                              {smsCooldown > 0
                                ? `验证码已发送至 ${maskPhone(neteasePhone)} / Code sent`
                                : "输入短信验证码 / Enter verification code"}
                            </div>

                            <div className="flex flex-wrap items-center gap-2">
                              <button
                                type="button"
                                onClick={() => openSecureWebLogin("netease")}
                                disabled={openingWebLoginSource === "netease" || !neteaseEnabled}
                                className="app-transition inline-flex h-10 items-center justify-center gap-2 rounded-full bg-white/[0.07] px-4 text-xs font-semibold text-white/62 hover:bg-white/[0.12] hover:text-white disabled:cursor-not-allowed disabled:opacity-45"
                              >
                                {openingWebLoginSource === "netease" ? (
                                  <Loader2 className="h-4 w-4 animate-spin" />
                                ) : (
                                  <Cloud className="h-4 w-4" />
                                )}
                                Secure Web Login / 使用网页登录
                              </button>
                              <p className="text-xs leading-5 text-white/34">
                                密码和验证码不会保存；复杂安全验证请使用官方网页登录。
                              </p>
                            </div>
                          </div>
                        )}
                      </div>

                      <div className="grid gap-3 rounded-[20px] bg-white/[0.035] p-4 sm:grid-cols-2">
                        <StatusLine
                          label="Login / 登录"
                          value={
                            neteaseLoginStatus?.loggedIn
                              ? `Signed in${neteaseLoginStatus.nickname ? ` as ${neteaseLoginStatus.nickname}` : ""}`
                              : neteaseLoginStatus?.expired
                                ? "Session expired — please re-sign in"
                                : neteaseLoginStatus?.hasCookie || musicSourceConfig.hasToken
                                  ? "Not signed in (cookie stored)"
                                  : "Signed out"
                          }
                        />
                        <StatusLine
                          label="Membership / 会员"
                          value={
                            neteaseVipStatus?.isMember
                              ? `Member${neteaseVipStatus.level ? ` (${neteaseVipStatus.level})` : ""}`
                              : neteaseVipStatus && neteaseVipStatus.membershipKnown === false
                                ? "Unknown / 会员状态未知"
                                : neteaseVipStatus
                                  ? "Non-member"
                                  : "Unknown — sign in to check"
                          }
                        />
                        <div className="flex gap-2 sm:col-span-2">
                          <button
                            type="button"
                            onClick={refreshLogin}
                            disabled={isCheckingLogin || !neteaseEnabled}
                            className="app-transition inline-flex h-9 items-center justify-center gap-2 rounded-full bg-white/[0.07] px-3 text-xs font-semibold text-white/58 hover:bg-white/[0.12] hover:text-white disabled:cursor-not-allowed disabled:opacity-45"
                          >
                            Refresh
                          </button>
                          <button
                            type="button"
                            onClick={logoutSource}
                            disabled={
                              isCheckingLogin || !neteaseEnabled || !musicSourceConfig.hasToken
                            }
                            className="app-transition inline-flex h-9 items-center justify-center gap-2 rounded-full bg-white/[0.07] px-3 text-xs font-semibold text-white/58 hover:bg-white/[0.12] hover:text-white disabled:cursor-not-allowed disabled:opacity-45"
                          >
                            Logout
                          </button>
                        </div>
                      </div>

                      <NetEaseServiceStatusCard
                        status={neteaseServiceStatus}
                        isRefreshing={isRefreshingNeteaseService}
                        onRetry={() => {
                          setRefreshingNeteaseService(true);
                          // 通过 retryToken 重启轮询；ensureNeteaseApiService 会由 effect 立即调用一次。
                          setNeteaseServiceRetryToken((value) => value + 1);
                          // 异步再拉取一次以同步刷新态，与 effect 内的初次 poll 等价但确保 isRefreshing 立即被清掉。
                          void ensureNeteaseApiService()
                            .then((status) => setNeteaseServiceStatus(status))
                            .catch((error) =>
                              setNeteaseServiceStatus({
                                running: false,
                                started: false,
                                baseUrl: "",
                                message: readError(error),
                                nodeAvailable: false,
                                apiPackageFound: false,
                                stage: "failed",
                              }),
                            )
                            .finally(() => setRefreshingNeteaseService(false));
                        }}
                      />

                      <BilibiliSourceSettings
                        config={bilibiliConfig}
                        enabled={bilibiliEnabled}
                        baseUrl={bilibiliBaseUrl}
                        token={bilibiliToken}
                        searchScope={bilibiliSearchScope}
                        loginStatus={bilibiliLoginStatus}
                        qr={bilibiliQr}
                        qrStatus={bilibiliQrStatus}
                        isTesting={isTestingBilibili}
                        isChecking={isCheckingBilibili}
                        isCreatingQr={isCreatingBilibiliQr}
                        otherLoginOpen={isBilibiliOtherLoginOpen}
                        onEnabledChange={(enabled) => void updateBilibiliEnabled(enabled)}
                        onBaseUrlChange={setBilibiliBaseUrl}
                        onTokenChange={setBilibiliToken}
                        onSearchScopeChange={setBilibiliSearchScope}
                        onTest={testBilibiliSource}
                        onSave={saveBilibiliDraft}
                        onImportCookie={importBilibiliCookie}
                        onCreateQr={createBilibiliQrLogin}
                        onOtherLoginToggle={() => setBilibiliOtherLoginOpen((value) => !value)}
                        onSecureWebLogin={() => openSecureWebLogin("bilibili")}
                        onLogout={logoutBilibili}
                        openingWebLogin={openingWebLoginSource === "bilibili"}
                      />

                      {/* ── QQ Music Source Card ── */}
                      <div className="space-y-4 rounded-[20px] bg-white/[0.035] p-4">
                        <div className="flex items-center justify-between gap-3">
                          <div className="flex items-center gap-3">
                            <span className="flex h-9 w-9 items-center justify-center rounded-full bg-white/[0.07] text-white/64">
                              <Music2 className="h-4 w-4" />
                            </span>
                            <div>
                              <p className="text-sm font-semibold text-white/84">
                                QQ音乐 / QQ Music
                              </p>
                              <p className="mt-1 text-xs text-white/36">
                                {describeQQMusicAuthState(
                                  qqmusicLoginStatus,
                                  qqmusicEnabled,
                                  "detail",
                                )}
                              </p>
                            </div>
                          </div>
                          <label className="relative inline-flex cursor-pointer items-center">
                            <input
                              type="checkbox"
                              className="peer sr-only"
                              checked={qqmusicEnabled}
                              onChange={(event) => void updateQQMusicEnabled(event.target.checked)}
                            />
                            <div className="h-6 w-11 rounded-full bg-white/15 peer-checked:bg-white/40 peer-focus:ring-2 peer-focus:ring-white/20" />
                            <div className="absolute left-0.5 top-0.5 h-5 w-5 rounded-full bg-white/80 transition-transform peer-checked:translate-x-5" />
                          </label>
                        </div>

                        {/* Cookie 导入为高级备用方式；输入框与导入按钮都在 Advanced 区。 */}

                        {!qqmusicEnabled && (
                          <p className="rounded-[16px] bg-white/[0.04] px-4 py-3 text-xs leading-5 text-white/45">
                            请先开启 QQ 音乐来源再登录。登录不会自动启用该来源。 / Enable the QQ
                            Music source above before signing in. Signing in never enables a source
                            automatically.
                          </p>
                        )}

                        {/* Official embedded sign-in. Credentials remain inside Rust/WebView2. */}
                        <div className="space-y-3 rounded-[20px] bg-white/[0.035] p-4">
                          <div>
                            <p className="text-sm font-semibold text-white/80">
                              登录 QQ 音乐 / Official Sign-in
                            </p>
                            <p className="mt-1 text-xs leading-5 text-white/38">
                              将在 Ome Music 的安全登录窗口中打开 QQ
                              官方页面；二维码、扫码与确认全部由官方页面完成，Ome Music
                              不读取密码。登录确认后会自动完成连接 / Sign-in happens on the official
                              page inside a secure app-owned window; the connection completes
                              automatically after you confirm on your phone.
                            </p>
                          </div>
                          <div className="grid gap-3 sm:grid-cols-2">
                            <button
                              type="button"
                              disabled={!qqmusicEnabled}
                              onClick={async () => {
                                setQQMusicMsg(null);
                                try {
                                  await qqmusicAuthProvider.openWebviewLogin("qq");
                                  setQQMusicWebLoginMode("qq");
                                  setQQMusicMsg(
                                    "官方窗口已打开：请扫码并确认，登录后会自动完成连接 / The official window is open: scan, confirm, and the connection completes automatically.",
                                  );
                                } catch (error) {
                                  setQQMusicMsg(readError(error));
                                }
                              }}
                              className="app-transition inline-flex h-11 items-center justify-center gap-2 rounded-full bg-white/[0.08] px-4 text-sm font-semibold text-white/72 hover:bg-white/[0.13] hover:text-white disabled:cursor-not-allowed disabled:opacity-45"
                            >
                              <QrCode className="h-4 w-4" />
                              登录 QQ 音乐 / Official Login
                            </button>
                            <button
                              type="button"
                              disabled={!qqmusicEnabled}
                              onClick={async () => {
                                setQQMusicMsg(null);
                                try {
                                  await qqmusicAuthProvider.openWebviewLogin("wechat");
                                  setQQMusicWebLoginMode("wechat");
                                  setQQMusicMsg(
                                    "官方窗口已打开：请选择微信扫码并确认，登录后会自动完成连接 / Choose WeChat QR in the official window; the connection completes automatically.",
                                  );
                                } catch (error) {
                                  setQQMusicMsg(readError(error));
                                }
                              }}
                              className="app-transition inline-flex h-11 items-center justify-center gap-2 rounded-full bg-white/[0.08] px-4 text-sm font-semibold text-white/72 hover:bg-white/[0.13] hover:text-white disabled:cursor-not-allowed disabled:opacity-45"
                            >
                              <QrCode className="h-4 w-4" />
                              微信扫码 / WeChat QR
                            </button>
                          </div>
                          {qqmusicWebLoginMode && (
                            <>
                              {/* Backend flow state machine — the sole auth
                                  authority. Progress text only; the values are
                                  masked counts/booleans from Rust. */}
                              <div
                                className="rounded-xl bg-white/[0.04] px-3 py-2 text-xs leading-5 text-white/55"
                                data-qqmusic-login-flow
                              >
                                {qqmusicLoginFlow
                                  ? `${
                                      qqmusicLoginFlow.status === "authenticated"
                                        ? "已连接 / Authenticated"
                                        : qqmusicLoginFlow.status === "verifying"
                                          ? "正在确认 QQ 音乐账号… / Confirming the QQ Music account…"
                                          : qqmusicLoginFlow.status === "waiting_for_user"
                                            ? "等待扫码确认… / Waiting for the scan to be confirmed…"
                                            : qqmusicLoginFlow.status === "failed"
                                              ? "未通过验证 / Not verified yet"
                                              : qqmusicLoginFlow.status === "canceled"
                                                ? "登录窗口已关闭 / Sign-in window closed"
                                                : "正在打开官方窗口… / Opening the official window…"
                                    } · cookies=${qqmusicLoginFlow.cookieCount} · uin=${qqmusicLoginFlow.uinPresent} · musicKey=${qqmusicLoginFlow.signingKeyPresent}${qqmusicLoginFlow.message ? ` · ${qqmusicLoginFlow.message}` : ""}`
                                  : "正在打开官方窗口… / Opening the official window…"}
                              </div>
                              <div className="grid gap-3 sm:grid-cols-2">
                                <button
                                  type="button"
                                  onClick={async () => {
                                    setQQMusicMsg(null);
                                    setCheckingQQMusicLogin(true);
                                    try {
                                      const status =
                                        await qqmusicAuthProvider.importWebviewSession();
                                      setQQMusicLoginStatus(status);
                                      setQQMusicMsg(
                                        describeQQMusicAuthState(status, qqmusicEnabled, "detail"),
                                      );
                                      if (
                                        status.loggedIn ||
                                        (status.credentialPresent &&
                                          ["credential_present", "unknown"].includes(status.status))
                                      ) {
                                        await qqmusicAuthProvider.closeWebviewLogin();
                                        setQQMusicWebLoginMode(null);
                                      }
                                    } catch (error) {
                                      setQQMusicMsg(readError(error));
                                    } finally {
                                      setCheckingQQMusicLogin(false);
                                    }
                                  }}
                                  disabled={isCheckingQQMusicLogin}
                                  className="app-transition inline-flex h-11 items-center justify-center gap-2 rounded-full bg-emerald-600/20 px-4 text-sm font-semibold text-emerald-200 hover:bg-emerald-600/30 disabled:cursor-wait disabled:opacity-45"
                                >
                                  {isCheckingQQMusicLogin ? (
                                    <Loader2 className="h-4 w-4 animate-spin" />
                                  ) : (
                                    <ShieldCheck className="h-4 w-4" />
                                  )}
                                  完成连接 / Complete Connection
                                </button>
                                <button
                                  type="button"
                                  onClick={async () => {
                                    try {
                                      await qqmusicAuthProvider.closeWebviewLogin();
                                      setQQMusicWebLoginMode(null);
                                      setQQMusicMsg("已关闭登录窗口 / Sign-in window closed.");
                                    } catch (error) {
                                      setQQMusicMsg(readError(error));
                                    }
                                  }}
                                  className="app-transition inline-flex h-11 items-center justify-center rounded-full bg-white/[0.08] px-4 text-sm font-semibold text-white/68 hover:bg-white/[0.13] hover:text-white"
                                >
                                  关闭窗口 / Close Window
                                </button>
                              </div>
                              <p className="text-[11px] leading-4 text-white/30">
                                正常情况下登录确认后会自动完成连接；“完成连接”仅作为页面未自动跳转时的备用
                                / The connection normally completes automatically; “Complete
                                Connection” is only a fallback for pages that do not navigate.
                              </p>
                            </>
                          )}
                        </div>

                        {/* 高级：Cookie 导入 + Direct QR（实验性）。
                            Direct QR（自建 ptlogin 链路）在部分网络环境被 QQ
                            服务端以 403 拒绝；它不是主登录入口，失败时引导到
                            上方官方登录。403 是“不可用”，绝不显示为“已过期”。 */}
                        <div className="space-y-3 rounded-[20px] bg-white/[0.02] p-4">
                          <p className="text-xs font-semibold uppercase tracking-wide text-white/40">
                            Advanced / 高级
                          </p>
                          <Field label="Cookie / 登录凭据">
                            <textarea
                              value={qqmusicToken}
                              onChange={(e) => setQQMusicToken(e.target.value)}
                              placeholder="粘贴 Cookie 粘贴导入 / Paste Cookie to import…"
                              className="settings-input min-h-24 resize-y py-3 font-mono text-xs leading-5"
                              autoComplete="off"
                              spellCheck={false}
                            />
                          </Field>
                          <div className="grid gap-3 sm:grid-cols-2">
                            <button
                              type="button"
                              onClick={createQQMusicQr}
                              disabled={isCreatingQQMusicQr || !qqmusicEnabled}
                              className="app-transition inline-flex h-11 items-center justify-center gap-2 rounded-full bg-white/[0.06] px-4 text-sm font-semibold text-white/60 hover:bg-white/[0.12] hover:text-white disabled:cursor-not-allowed disabled:opacity-45"
                            >
                              {isCreatingQQMusicQr ? (
                                <Loader2 className="h-4 w-4 animate-spin" />
                              ) : (
                                <QrCode className="h-4 w-4" />
                              )}
                              {qqmusicQr
                                ? "重新生成 / New Code"
                                : "Direct QR（实验性）/ Experimental"}
                            </button>
                            <button
                              type="button"
                              disabled={!qqmusicToken.trim() || !qqmusicEnabled}
                              onClick={async () => {
                                setQQMusicMsg(null);
                                try {
                                  const status = await qqmusicAuthProvider.importCookie(
                                    qqmusicToken.trim(),
                                  );
                                  setQQMusicToken("");
                                  setQQMusicLoginStatus(status);
                                  setQQMusicMsg(
                                    describeQQMusicAuthState(status, qqmusicEnabled, "detail"),
                                  );
                                } catch (error) {
                                  setQQMusicMsg(readError(error));
                                }
                              }}
                              className="app-transition inline-flex h-11 items-center justify-center gap-2 rounded-full bg-white/[0.06] px-4 text-sm font-semibold text-white/60 hover:bg-white/[0.12] hover:text-white disabled:cursor-not-allowed disabled:opacity-45"
                            >
                              <ShieldCheck className="h-4 w-4" />
                              导入 Cookie / Import
                            </button>
                          </div>

                          {/* QR 码展示区（实验性 Direct QR） */}
                          {qqmusicQr && qqmusicQrStatus !== "confirmed" && (
                            <div className="grid gap-4 rounded-[20px] bg-white/[0.04] p-4 sm:grid-cols-[128px_1fr]">
                              {qqmusicQrStatus !== "failed" ? (
                                qqmusicQr.url ? (
                                  <img
                                    src={qqmusicQr.url}
                                    alt="QQ Music QR Code"
                                    className="h-32 w-32 rounded-[16px] bg-white p-2"
                                  />
                                ) : (
                                  <div className="flex h-32 w-32 items-center justify-center rounded-[16px] bg-white/[0.06] text-xs text-white/40">
                                    二维码加载失败 / QR unavailable
                                  </div>
                                )
                              ) : (
                                <div className="flex h-32 w-32 items-center justify-center rounded-[16px] bg-[#7a2d1c]/25 text-xs font-semibold text-[#e8a08f]">
                                  不可用 / Unavailable
                                </div>
                              )}
                              <div className="flex flex-col justify-center">
                                <p className="text-sm font-semibold text-white/80">
                                  Direct QR（实验性）/ Experimental
                                </p>
                                <p className="mt-2 text-sm leading-6 text-white/42">
                                  {qqmusicQrStatus === "waiting" &&
                                    "等待扫码确认… / Waiting for scan…"}
                                  {qqmusicQrStatus === "scanned" &&
                                    "已扫描，请在手机上确认 / Scanned. Confirm on your phone."}
                                  {qqmusicQrStatus === "expired" &&
                                    "二维码已过期，请重新生成 / QR code expired. Regenerate to try again."}
                                  {qqmusicQrStatus === "timeout" &&
                                    "二维码已超时，请重新生成 / QR timed out. Regenerate to try again."}
                                  {qqmusicQrStatus === "failed" &&
                                    "当前网络环境无法使用直接扫码（QQ 服务端拒绝），请使用上方官方登录 / Direct QR is unavailable on this network; use Official Sign-in above."}
                                </p>
                                <div className="flex flex-wrap gap-2">
                                  {qqmusicQrStatus === "failed" && (
                                    <button
                                      type="button"
                                      onClick={async () => {
                                        setQQMusicMsg(null);
                                        try {
                                          await qqmusicAuthProvider.openWebviewLogin("qq");
                                          setQQMusicWebLoginMode("qq");
                                          setQQMusicMsg(
                                            "官方窗口已打开：请扫码并确认，登录后会自动完成连接 / The official window is open: scan, confirm, and the connection completes automatically.",
                                          );
                                        } catch (error) {
                                          setQQMusicMsg(readError(error));
                                        }
                                      }}
                                      className="app-transition mt-3 inline-flex h-9 w-fit items-center justify-center gap-2 rounded-full bg-emerald-600/20 px-4 text-xs font-semibold text-emerald-200 hover:bg-emerald-600/30"
                                    >
                                      <QrCode className="h-3.5 w-3.5" />
                                      使用官方登录 / Use Official Sign-in
                                    </button>
                                  )}
                                  {(qqmusicQrStatus === "expired" ||
                                    qqmusicQrStatus === "timeout" ||
                                    qqmusicQrStatus === "failed") && (
                                    <button
                                      type="button"
                                      onClick={createQQMusicQr}
                                      disabled={isCreatingQQMusicQr}
                                      className="app-transition mt-3 inline-flex h-9 w-fit items-center justify-center gap-2 rounded-full bg-white/[0.1] px-4 text-xs font-semibold text-white/72 hover:bg-white/[0.18] hover:text-white disabled:cursor-not-allowed disabled:opacity-45"
                                    >
                                      {isCreatingQQMusicQr ? (
                                        <Loader2 className="h-3.5 w-3.5 animate-spin" />
                                      ) : (
                                        <RefreshCw className="h-3.5 w-3.5" />
                                      )}
                                      重新生成 / Regenerate
                                    </button>
                                  )}
                                </div>
                              </div>
                            </div>
                          )}
                        </div>
                        {/* 状态检测 + 登出 */}
                        <div className="grid gap-3 sm:grid-cols-2">
                          <button
                            type="button"
                            disabled={isCheckingQQMusicLogin}
                            onClick={async () => {
                              setQQMusicMsg(null);
                              setCheckingQQMusicLogin(true);
                              try {
                                const status = await qqmusicAuthProvider.getLoginStatus();
                                setQQMusicLoginStatus(status);
                                setQQMusicMsg(
                                  describeQQMusicAuthState(status, qqmusicEnabled, "detail"),
                                );
                              } catch (e) {
                                setQQMusicMsg(readError(e));
                              } finally {
                                setCheckingQQMusicLogin(false);
                              }
                            }}
                            className="app-transition inline-flex h-11 items-center justify-center gap-2 rounded-full bg-white/[0.08] px-4 text-sm font-semibold text-white/72 hover:bg-white/[0.13] hover:text-white disabled:cursor-not-allowed disabled:opacity-45"
                          >
                            {isCheckingQQMusicLogin ? (
                              <Loader2 className="h-4 w-4 animate-spin" />
                            ) : (
                              <ShieldCheck className="h-4 w-4" />
                            )}
                            检测登录 / Check Login
                          </button>
                          {qqmusicLoginStatus?.credentialPresent && (
                            <button
                              type="button"
                              onClick={async () => {
                                const status = await qqmusicAuthProvider.logout();
                                setQQMusicLoginStatus(status);
                                setQQMusicVipStatus(null);
                                setQQMusicUserPlaylists([]);
                                setQQMusicMsg("已登出 / Signed out.");
                              }}
                              className="app-transition inline-flex h-11 items-center justify-center gap-2 rounded-full bg-white/[0.08] px-4 text-sm font-semibold text-white/72 hover:bg-white/[0.13] hover:text-white"
                            >
                              登出 / Sign Out
                            </button>
                          )}
                        </div>

                        {/* 操作反馈 */}
                        {qqmusicMsg && (
                          <p className="rounded-[14px] bg-white/[0.05] px-3 py-2 text-xs leading-5 text-white/54">
                            {qqmusicMsg}
                          </p>
                        )}
                      </div>

                      <div className="space-y-3 rounded-[20px] bg-white/[0.035] p-4">
                        <div className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
                          <div>
                            <div className="flex items-center gap-2">
                              <Library className="h-4 w-4 text-white/46" />
                              <h4 className="text-sm font-semibold text-white/82">
                                Playlist Shelf / 歌单架
                              </h4>
                            </div>
                            <p className="mt-1 text-sm leading-6 text-white/42">
                              Bring liked songs and playlists into the local library.
                              将喜欢和歌单收进本地。
                            </p>
                          </div>
                          <div className="flex shrink-0 flex-wrap gap-2">
                            <button
                              type="button"
                              onClick={() => syncListeningMemory(false)}
                              disabled={
                                isSyncingMemory || !neteaseEnabled || !neteaseLoginStatus?.loggedIn
                              }
                              className="app-transition inline-flex h-9 items-center justify-center gap-2 rounded-full bg-white/[0.07] px-3 text-xs font-semibold text-white/62 hover:bg-white/[0.12] hover:text-white disabled:cursor-not-allowed disabled:opacity-45"
                            >
                              {isSyncingMemory ? (
                                <Loader2 className="h-3.5 w-3.5 animate-spin" />
                              ) : (
                                <Music2 className="h-3.5 w-3.5" />
                              )}
                              Sync Liked / 同步喜欢
                            </button>
                            <button
                              type="button"
                              onClick={loadUserPlaylists}
                              disabled={
                                isLoadingUserPlaylists ||
                                !neteaseEnabled ||
                                !neteaseLoginStatus?.loggedIn
                              }
                              className="app-transition inline-flex h-9 items-center justify-center gap-2 rounded-full bg-white/[0.07] px-3 text-xs font-semibold text-white/62 hover:bg-white/[0.12] hover:text-white disabled:cursor-not-allowed disabled:opacity-45"
                            >
                              {isLoadingUserPlaylists ? (
                                <Loader2 className="h-3.5 w-3.5 animate-spin" />
                              ) : (
                                <ListMusic className="h-3.5 w-3.5" />
                              )}
                              Load Playlists / 读取歌单
                            </button>
                          </div>
                        </div>

                        {/* Local import stamps — one-way local import from
                            NetEase into this library, NOT a cloud sync. Quiet
                            single line so the user knows whether the last
                            "Sync Liked" finished and how many playlists are
                            already in the local library. */}
                        {((likedSyncedAt && formatRelativeTime(likedSyncedAt)) ||
                          Object.keys(importedRecords).length > 0) && (
                          <p className="text-xs text-white/34">
                            {likedSyncedAt && formatRelativeTime(likedSyncedAt)
                              ? `Liked imported ${formatRelativeTime(likedSyncedAt)}`
                              : "Liked not imported yet"}
                            {Object.keys(importedRecords).length > 0
                              ? ` · ${Object.keys(importedRecords).length} playlist${
                                  Object.keys(importedRecords).length === 1 ? "" : "s"
                                } imported locally`
                              : ""}
                          </p>
                        )}

                        {tasteNotes && (
                          <div className="rounded-[18px] bg-black/10 px-4 py-3">
                            <p className="text-sm font-semibold text-white/74">
                              {tasteNotes.musicPersonality}
                            </p>
                            <p className="mt-2 text-xs leading-5 text-white/38">
                              {tasteNotes.trackCount} songs remembered / 已记住{" "}
                              {tasteNotes.trackCount} 首
                              {tasteNotes.playlistCount
                                ? `, ${tasteNotes.playlistCount} playlists interpreted`
                                : ""}
                              {tasteNotes.favoriteArtists[0]
                                ? ` - ${tasteNotes.favoriteArtists.slice(0, 3).join(", ")}`
                                : ""}
                            </p>
                          </div>
                        )}

                        {/* Full shelf — no slice cap. Cards render in a
                            scrollable grid so all playlists stay reachable
                            without becoming a backend-style table. */}
                        {neteaseUserPlaylists.length > 0 && (
                          <div className="settings-scroll grid max-h-[22rem] gap-2 overflow-y-auto pr-1 sm:grid-cols-2">
                            {neteaseUserPlaylists.map((playlist, index) => (
                              <PlaylistShelfCard
                                key={playlist.id}
                                playlist={playlist}
                                liked={isLikedSongsPlaylist(playlist, index, neteaseLoginStatus)}
                                state={playlistStates[playlist.id]}
                                imported={importedRecords[playlist.id]}
                                disabled={!neteaseEnabled || !neteaseLoginStatus?.loggedIn}
                                onImport={() => importPlaylistById(playlist.id)}
                              />
                            ))}
                          </div>
                        )}
                      </div>

                      <button
                        type="button"
                        onClick={testSource}
                        disabled={isTestingSource || !neteaseEnabled || !neteaseBaseUrl.trim()}
                        className="app-transition inline-flex h-11 items-center justify-center gap-2 rounded-full bg-white/[0.08] px-4 text-sm font-semibold text-white/72 hover:bg-white/[0.13] hover:text-white disabled:cursor-not-allowed disabled:opacity-45"
                      >
                        {isTestingSource ? (
                          <Loader2 className="h-4 w-4 animate-spin" />
                        ) : (
                          <RefreshCw className="h-4 w-4" />
                        )}
                        Test Connection / 测试连接
                      </button>

                      <div className="grid gap-3 sm:grid-cols-[1fr_auto]">
                        <input
                          value={neteasePlaylistId}
                          onChange={(event) => setNeteasePlaylistId(event.target.value)}
                          placeholder="Playlist ID / 歌单 ID"
                          className="settings-input"
                        />
                        <button
                          type="button"
                          onClick={importPlaylist}
                          disabled={
                            isImportingPlaylist || !neteaseEnabled || !neteasePlaylistId.trim()
                          }
                          className="app-transition inline-flex h-12 items-center justify-center gap-2 rounded-[18px] bg-white/[0.08] px-4 text-sm font-semibold text-white/72 hover:bg-white/[0.13] hover:text-white disabled:cursor-not-allowed disabled:opacity-45"
                        >
                          {isImportingPlaylist ? (
                            <Loader2 className="h-4 w-4 animate-spin" />
                          ) : (
                            <ListMusic className="h-4 w-4" />
                          )}
                          Import Playlist / 导入歌单
                        </button>
                      </div>

                      {sourceMessage && (
                        <p className="rounded-[18px] bg-white/[0.045] px-4 py-3 text-sm leading-6 text-white/54">
                          {sourceMessage}
                        </p>
                      )}
                    </div>
                  </div>
                )}

                {activeSection === "storage" && (
                  <StoragePanel
                    isOpen={isStorageOpen}
                    onToggle={() => {
                      setStorageOpen((value) => !value);
                      void refreshStorageReport();
                    }}
                    report={storageReport}
                    clearingKind={clearingStorageKind}
                    message={storageMessage}
                    onClear={clearStorage}
                    onExportDiagnostics={exportDiagnostics}
                  />
                )}

                {(activeSection === "sources" || activeSection === "curator") && (
                  <div className="flex flex-col gap-3 pt-3 sm:flex-row sm:items-center sm:justify-between">
                    <div className="text-sm text-white/44">
                      {activeSection === "sources"
                        ? neteaseLoginStatus?.loggedIn
                          ? "Signed in to NetEase Cloud Music. 网易云账号已登录。"
                          : musicSourceConfig.hasToken
                            ? "Local service ready, sign-in unverified. 本地服务已就绪，登录状态未确认。"
                            : "Local library stays ready. 本地曲库保持可用。"
                        : config.configured
                          ? "Current source is connected. 当前来源已连接。"
                          : "Local text is used until connected. 未连接时使用本地文本。"}
                    </div>
                    <button
                      type="button"
                      onClick={save}
                      disabled={isSaving}
                      className="app-transition inline-flex h-11 items-center justify-center gap-2 rounded-full bg-white px-5 text-sm font-semibold text-graphite-950 hover:scale-[1.02] disabled:cursor-wait disabled:opacity-70"
                    >
                      {isSaving ? (
                        <Loader2 className="h-4 w-4 animate-spin" />
                      ) : (
                        <Save className="h-4 w-4" />
                      )}
                      Save / 保存
                    </button>
                  </div>
                )}
                {message && (
                  <p className="rounded-[18px] bg-white/[0.05] px-4 py-3 text-sm leading-6 text-white/54">
                    {message}
                  </p>
                )}
              </div>
            )}
          </div>
        </div>
      </section>
    </div>
  );
}

function BilibiliSourceSettings({
  config,
  enabled,
  baseUrl,
  token,
  searchScope,
  loginStatus,
  qr,
  qrStatus,
  isTesting,
  isChecking,
  isCreatingQr,
  otherLoginOpen,
  onEnabledChange,
  onBaseUrlChange,
  onTokenChange,
  onSearchScopeChange,
  onTest,
  onSave,
  onImportCookie,
  onCreateQr,
  onOtherLoginToggle,
  onSecureWebLogin,
  onLogout,
  openingWebLogin,
}: {
  config: BilibiliSourceConfig;
  enabled: boolean;
  baseUrl: string;
  token: string;
  searchScope: BilibiliSourceConfig["searchScope"];
  loginStatus: BilibiliLoginStatus | null;
  qr: NetEaseQrLogin | null;
  qrStatus: QrSessionStatus;
  isTesting: boolean;
  isChecking: boolean;
  isCreatingQr: boolean;
  otherLoginOpen: boolean;
  onEnabledChange: (enabled: boolean) => void;
  onBaseUrlChange: (value: string) => void;
  onTokenChange: (value: string) => void;
  onSearchScopeChange: (value: BilibiliSourceConfig["searchScope"]) => void;
  onTest: () => void;
  onSave: () => void;
  onImportCookie: () => void;
  onCreateQr: () => void;
  onOtherLoginToggle: () => void;
  onSecureWebLogin: () => void;
  onLogout: () => void;
  openingWebLogin: boolean;
}) {
  return (
    <div className="space-y-4 rounded-[20px] bg-white/[0.035] p-4">
      <div className="flex items-center justify-between gap-4">
        <div>
          <h4 className="text-sm font-semibold text-white/84">Bilibili / B站音乐源</h4>
          <p className="mt-1 text-xs leading-5 text-white/38">只播放音频，弹幕作为轻量氛围层。</p>
        </div>
        <label className="flex items-center gap-3 text-sm text-white/64">
          <input
            type="checkbox"
            checked={enabled}
            onChange={(event) => onEnabledChange(event.target.checked)}
            className="h-4 w-4 accent-white"
          />
          Enable / 开启
        </label>
      </div>

      <div className="grid gap-3 sm:grid-cols-2">
        <Field label="API Base URL / 来源地址">
          <input
            value={baseUrl}
            onChange={(event) => onBaseUrlChange(event.target.value)}
            placeholder="https://api.bilibili.com"
            className="settings-input"
          />
        </Field>
        <Field label="Search Scope / 搜索范围">
          <select
            value={searchScope}
            onChange={(event) =>
              onSearchScopeChange(event.target.value as BilibiliSourceConfig["searchScope"])
            }
            className="settings-input appearance-none"
          >
            <option value="music" className="bg-graphite-950 text-white">
              Music / 音乐
            </option>
            <option value="vocaloid" className="bg-graphite-950 text-white">
              Vocaloid
            </option>
            <option value="live" className="bg-graphite-950 text-white">
              Live / 现场
            </option>
            <option value="cover" className="bg-graphite-950 text-white">
              Cover / 翻唱
            </option>
            <option value="mv" className="bg-graphite-950 text-white">
              MV
            </option>
            <option value="all" className="bg-graphite-950 text-white">
              All / 全部
            </option>
          </select>
        </Field>
      </div>

      <div className="space-y-3 rounded-[18px] bg-white/[0.035] p-4">
        <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
          <div>
            <p className="text-sm font-semibold text-white/82">扫码登录 / QR Sign-in</p>
            <p className="mt-1 text-xs leading-5 text-white/36">
              推荐方式。手机确认后，登录状态会自动安全保存。
            </p>
          </div>
          <button
            type="button"
            onClick={onCreateQr}
            disabled={isCreatingQr || !enabled}
            className="app-transition inline-flex h-10 shrink-0 items-center justify-center gap-2 rounded-full bg-white/[0.09] px-4 text-sm font-semibold text-white/74 hover:bg-white/[0.14] hover:text-white disabled:cursor-not-allowed disabled:opacity-45"
          >
            {isCreatingQr ? (
              <Loader2 className="h-4 w-4 animate-spin" />
            ) : (
              <QrCode className="h-4 w-4" />
            )}
            {qr ? "重新生成 / New Code" : "扫码登录 / Scan"}
          </button>
        </div>

        {qr && (
          <div className="grid gap-4 rounded-[16px] bg-black/[0.08] p-3 sm:grid-cols-[128px_1fr]">
            {qr.qrImg ? (
              <img
                src={qr.qrImg}
                alt="Bilibili sign-in QR code"
                className="h-32 w-32 rounded-[14px] bg-white p-2"
              />
            ) : (
              <div className="flex h-32 w-32 items-center justify-center rounded-[14px] bg-white/[0.06] text-xs text-white/40">
                二维码加载失败 / QR unavailable
              </div>
            )}
            <div className="flex flex-col justify-center">
              <p className="text-sm font-semibold text-white/78">
                使用哔哩哔哩扫码 / Scan with Bilibili
              </p>
              <p className="mt-2 text-xs leading-5 text-white/38">
                {qrStatus === "waiting" && "等待扫码确认… / Waiting for scan…"}
                {qrStatus === "scanned" &&
                  "已扫描，请在手机上确认 / Scanned. Confirm on your phone."}
                {qrStatus === "expired" &&
                  "二维码已过期，请重新生成 / QR code expired. Regenerate to try again."}
                {qrStatus === "timeout" &&
                  "等待超时，请重新生成 / Timed out. Regenerate to try again."}
              </p>
              {(qrStatus === "expired" || qrStatus === "timeout") && (
                <button
                  type="button"
                  onClick={onCreateQr}
                  disabled={isCreatingQr || !enabled}
                  className="app-transition mt-3 inline-flex h-9 w-fit items-center justify-center gap-2 rounded-full bg-white/[0.1] px-4 text-xs font-semibold text-white/72 hover:bg-white/[0.18] hover:text-white disabled:cursor-not-allowed disabled:opacity-45"
                >
                  {isCreatingQr ? (
                    <Loader2 className="h-3.5 w-3.5 animate-spin" />
                  ) : (
                    <RefreshCw className="h-3.5 w-3.5" />
                  )}
                  重新生成 / Regenerate
                </button>
              )}
            </div>
          </div>
        )}

        <button
          type="button"
          onClick={onOtherLoginToggle}
          className="app-transition flex w-full items-center justify-between gap-3 rounded-[14px] px-2 py-2 text-left hover:bg-white/[0.035]"
        >
          <span>
            <span className="block text-sm font-semibold text-white/68">
              网页安全登录 / Secure Web Login
            </span>
            <span className="mt-1 block text-xs text-white/32">
              Bilibili account/password and SMS verification are handled through Secure Web Login.
            </span>
          </span>
          <ChevronDown
            className={`h-4 w-4 text-white/36 transition ${otherLoginOpen ? "rotate-180" : ""}`}
          />
        </button>

        {otherLoginOpen && (
          <div className="grid gap-3">
            <SourceButton
              icon={ShieldCheck}
              label="打开 Bilibili 安全登录 / Open Secure Web Login"
              loading={openingWebLogin}
              disabled={openingWebLogin || !enabled}
              onClick={onSecureWebLogin}
            />
            <p className="text-xs text-white/32">
              Ome Music does not collect your Bilibili password or SMS code. All verification is
              completed on the official Bilibili page.
            </p>
          </div>
        )}
      </div>

      <Field label="Cookie / 登录凭据">
        <input
          value={token}
          onChange={(event) => onTokenChange(event.target.value)}
          placeholder={
            config.hasToken ? `${config.maskedToken} saved; leave blank to keep it` : "Optional"
          }
          type="password"
          className="settings-input"
          autoComplete="off"
        />
      </Field>

      <div className="grid gap-3 sm:grid-cols-4">
        <SourceButton
          icon={RefreshCw}
          label="Test / 测试"
          loading={isTesting}
          disabled={isTesting || !enabled}
          onClick={onTest}
        />
        <SourceButton icon={Save} label="Save / 保存" onClick={onSave} />
        <SourceButton
          icon={ShieldCheck}
          label="Import / 导入"
          loading={isChecking}
          disabled={isChecking || !enabled || !token.trim()}
          onClick={onImportCookie}
        />
        <SourceButton
          label="Logout / 退出"
          disabled={isChecking || !config.hasToken}
          onClick={onLogout}
        />
      </div>

      <p className="text-xs leading-5 text-white/34">
        Cookie 仅作为高级备用方式，内容不会明文显示。Cookie import is an advanced fallback only.
      </p>

      <StatusLine
        label="Bilibili Status / B站状态"
        value={
          loginStatus?.loggedIn
            ? `Connected${loginStatus.nickname ? ` as ${loginStatus.nickname}` : ""}`
            : (loginStatus?.message ?? "Public content is available.")
        }
      />
    </div>
  );
}

// 存储管理面板：将原本散落在主面板 JSX 中的 ~130 行存储 UI 收敛为单一组件。
// 状态仍在主面板持有 —— 让 storageReport 在切换 section 之间持久，避免来回切换时重新拉取。
function StoragePanel({
  isOpen,
  onToggle,
  report,
  clearingKind,
  message,
  onClear,
  onExportDiagnostics,
}: {
  isOpen: boolean;
  onToggle: () => void;
  report: StorageReport | null;
  clearingKind: StorageBucketKind | null;
  message: string | null;
  onClear: (kind: StorageBucketKind) => void;
  onExportDiagnostics: () => void;
}) {
  return (
    <div className="space-y-4">
      <SettingsIntro title="存储管理" subtitle="See what Ome keeps, and clear only what is safe." />
      <div className="settings-surface">
        <button
          type="button"
          onClick={onToggle}
          className="app-transition flex w-full items-center justify-between gap-4 px-4 py-4 text-left hover:bg-white/[0.035]"
        >
          <div className="flex items-center gap-3">
            <span className="flex h-9 w-9 items-center justify-center rounded-full bg-white/[0.07] text-white/64">
              <HardDrive className="h-4 w-4" />
            </span>
            <div>
              <h3 className="text-sm font-semibold text-white/84">存储管理 / Storage</h3>
              <p className="mt-1 text-xs text-white/36">
                {report
                  ? `缓存 ${report.totalCacheDisplaySize} · 数据库 ${report.database.displaySize}`
                  : "查看缓存大小，不删除音乐文件。"}
              </p>
            </div>
          </div>
          <ChevronDown
            className={`h-4 w-4 text-white/42 transition ${isOpen ? "rotate-180" : ""}`}
          />
        </button>

        {isOpen && (
          <div className="space-y-4 border-t border-white/[0.06] p-4">
            {report ? (
              <div className="grid gap-3 md:grid-cols-2">
                <StorageRow
                  icon={HardDrive}
                  label="应用缓存 / App Cache"
                  value={report.appCache.displaySize}
                />
                <StorageRow
                  icon={HardDrive}
                  label="WebView 缓存 / WebView Cache"
                  value={`${report.webviewCache.displaySize} (自动管理)`}
                />
                <StorageRow
                  icon={Music2}
                  label="封面缓存 / Cover Cache"
                  value={report.coverCache.displaySize}
                />
                <StorageRow
                  icon={ListMusic}
                  label="歌词缓存 / Lyrics Cache"
                  value={report.lyricsCache.displaySize}
                />
                <StorageRow icon={FileDown} label="日志 / Logs" value={report.logs.displaySize} />
                <StorageRow
                  icon={Database}
                  label="数据库 / Database"
                  value={report.database.displaySize}
                />
              </div>
            ) : (
              <p className="text-sm text-white/42">正在读取存储状态 / Reading storage state...</p>
            )}

            <div className="grid gap-2 md:grid-cols-5">
              <StorageAction
                label="清理应用缓存"
                sublabel="Clear App Cache"
                kind="appCache"
                activeKind={clearingKind}
                onClick={onClear}
              />
              <StorageAction
                label="清理封面缓存"
                sublabel="Clear Cover Cache"
                kind="coverCache"
                activeKind={clearingKind}
                onClick={onClear}
              />
              <StorageAction
                label="清理歌词缓存"
                sublabel="Clear Lyrics Cache"
                kind="lyricsCache"
                activeKind={clearingKind}
                onClick={onClear}
              />
              <StorageAction
                label="清理日志"
                sublabel="Clear Logs"
                kind="logs"
                activeKind={clearingKind}
                onClick={onClear}
              />
              <button
                type="button"
                onClick={onExportDiagnostics}
                className="app-transition inline-flex min-h-12 flex-col items-center justify-center rounded-[18px] bg-white/[0.07] px-3 py-2 text-xs font-semibold text-white/62 hover:bg-white/[0.12] hover:text-white"
              >
                <span>导出诊断</span>
                <span className="text-[10px] text-white/36">Export Diagnostics</span>
              </button>
            </div>

            <p className="text-xs leading-5 text-white/34">
              音乐缓存默认 0MB；网易云歌曲保持流式播放；本地音乐只保存路径，不复制文件。 Music cache
              is off by default. Local songs are never copied here.
            </p>
            {message && (
              <p className="rounded-[16px] bg-white/[0.045] px-3 py-2 text-xs leading-5 text-white/52">
                {message}
              </p>
            )}
          </div>
        )}
      </div>
    </div>
  );
}

function SettingsIntro({ title, subtitle }: { title: string; subtitle: string }) {
  return (
    <div className="pb-1">
      <h3 className="text-2xl font-semibold text-white/92">{title}</h3>
      <p className="mt-1.5 text-sm text-white/38">{subtitle}</p>
    </div>
  );
}

function OverviewLink({
  icon: Icon,
  title,
  subtitle,
  onClick,
}: {
  icon: LucideIcon;
  title: string;
  subtitle: string;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className="app-transition flex min-h-20 items-center gap-3 rounded-[18px] bg-white/[0.045] p-4 text-left hover:bg-white/[0.085]"
    >
      <span className="flex h-9 w-9 shrink-0 items-center justify-center rounded-full bg-white/[0.07] text-white/60">
        <Icon className="h-4 w-4" />
      </span>
      <span className="min-w-0">
        <span className="block text-sm font-semibold text-white/78">{title}</span>
        <span className="mt-1 block text-xs text-white/32">{subtitle}</span>
      </span>
    </button>
  );
}

function GuideStep({
  number,
  title,
  subtitle,
  detail,
  required = false,
}: {
  number: string;
  title: string;
  subtitle: string;
  detail: string;
  required?: boolean;
}) {
  return (
    <div className="grid gap-3 border-b border-white/[0.055] py-4 last:border-b-0 sm:grid-cols-[48px_minmax(0,1fr)_auto] sm:items-center">
      <span className="font-serif text-lg text-white/30">{number}</span>
      <div>
        <div className="flex flex-wrap items-baseline gap-x-2 gap-y-1">
          <p className="text-sm font-semibold text-white/82">{title}</p>
          <p className="text-[10px] uppercase tracking-[0.12em] text-white/30">{subtitle}</p>
        </div>
        <p className="mt-1 text-xs leading-5 text-white/42">{detail}</p>
      </div>
      <span
        className={clsx(
          "w-fit rounded-full px-2.5 py-1 text-[10px] font-semibold",
          required ? "bg-white text-[#211813]" : "bg-white/[0.06] text-white/38",
        )}
      >
        {required ? "必需" : "可选"}
      </span>
    </div>
  );
}

function DanmakuSettingsCard({
  settings,
  onChange,
  onClear,
}: {
  settings: DanmakuSettings;
  onChange: (patch: Partial<DanmakuSettings>) => void;
  onClear: () => void;
}) {
  return (
    <div className="settings-surface space-y-5">
      <div className="flex items-center justify-between gap-4">
        <div>
          <h4 className="text-sm font-semibold text-white/82">Danmaku Atmosphere / 弹幕氛围</h4>
          <p className="mt-1 text-xs leading-5 text-white/36">
            歌词优先，弹幕只像空气里飘过的情绪。
          </p>
        </div>
        <label className="flex items-center gap-3 text-sm text-white/64">
          <input
            type="checkbox"
            checked={settings.displayMode !== "off"}
            onChange={(event) =>
              onChange({
                enabled: event.target.checked,
                displayMode: event.target.checked
                  ? settings.displayMode === "off"
                    ? "ambient"
                    : settings.displayMode
                  : "off",
              })
            }
            className="h-4 w-4 accent-white"
          />
          Enable / 开启
        </label>
      </div>
      <div>
        <p className="mb-2 text-xs font-medium text-white/42">显示模式 / Display Mode</p>
        <div className="grid grid-cols-3 gap-1 rounded-[16px] bg-black/15 p-1">
          {(
            [
              ["off", "关闭", "Off"],
              ["video", "仅视频", "Video Only"],
              ["ambient", "全局氛围", "Ambient"],
            ] as const
          ).map(([mode, title, subtitle]) => (
            <button
              key={mode}
              type="button"
              onClick={() => onChange({ displayMode: mode, enabled: mode !== "off" })}
              className={clsx(
                "app-transition rounded-[13px] px-3 py-2.5 text-left",
                settings.displayMode === mode
                  ? "bg-white/12 text-white shadow-[0_8px_22px_rgba(0,0,0,0.16)]"
                  : "text-white/38 hover:bg-white/[0.055] hover:text-white/68",
              )}
            >
              <span className="block text-xs font-semibold">{title}</span>
              <span className="mt-0.5 block text-[10px] opacity-55">{subtitle}</span>
            </button>
          ))}
        </div>
      </div>
      <div className="grid gap-3 sm:grid-cols-3">
        <Field label={`Opacity / 透明度 ${(settings.opacity * 100).toFixed(0)}%`}>
          <input
            type="range"
            min="0.12"
            max="0.72"
            step="0.01"
            value={settings.opacity}
            onChange={(event) => onChange({ opacity: Number(event.target.value) })}
            className="accent-range"
          />
        </Field>
        <Field label="Density / 密度">
          <select
            value={settings.density}
            onChange={(event) =>
              onChange({ density: event.target.value as DanmakuSettings["density"] })
            }
            className="settings-input appearance-none"
          >
            <option value="low" className="bg-graphite-950 text-white">
              Low / 低
            </option>
            <option value="medium" className="bg-graphite-950 text-white">
              Medium / 中
            </option>
            <option value="high" className="bg-graphite-950 text-white">
              High / 高
            </option>
          </select>
        </Field>
        <Field label="Speed / 速度">
          <select
            value={settings.speed}
            onChange={(event) =>
              onChange({ speed: event.target.value as DanmakuSettings["speed"] })
            }
            className="settings-input appearance-none"
          >
            <option value="slow" className="bg-graphite-950 text-white">
              Slow / 慢
            </option>
            <option value="normal" className="bg-graphite-950 text-white">
              Normal / 正常
            </option>
            <option value="fast" className="bg-graphite-950 text-white">
              Fast / 快
            </option>
          </select>
        </Field>
      </div>
      <div className="grid gap-4 lg:grid-cols-2">
        <DanmakuChoiceGroup
          label="字体大小 / Font Size"
          value={settings.fontSize}
          options={[
            ["small", "小", "Small"],
            ["medium", "中", "Medium"],
            ["large", "大", "Large"],
          ]}
          onChange={(fontSize) => onChange({ fontSize })}
        />
        <DanmakuChoiceGroup
          label="情绪等级 / Emotional Intensity"
          value={settings.emotionalIntensity}
          options={[
            ["quiet", "安静", "Quiet"],
            ["balanced", "平衡", "Balanced"],
            ["expressive", "鲜明", "Expressive"],
          ]}
          onChange={(emotionalIntensity) => onChange({ emotionalIntensity })}
        />
      </div>
      <DanmakuChoiceGroup
        label="运动风格 / Motion Style"
        value={settings.motionStyle}
        options={[
          ["arc", "弧线", "Arc"],
          ["classic", "经典", "Classic"],
          ["drift", "漂移", "Drift"],
          ["meteor", "流星", "Meteor"],
          ["float", "浮游", "Float"],
          ["pulse", "呼吸", "Pulse"],
          ["mixed", "混合", "Mixed"],
        ]}
        columns="sm:grid-cols-3 lg:grid-cols-7"
        onChange={(motionStyle) => onChange({ motionStyle })}
      />
      <DanmakuChoiceGroup
        label="出现方式 / Entrance Style"
        value={settings.entranceStyle}
        options={[
          ["fade", "淡入", "Fade"],
          ["slide", "滑入", "Slide"],
          ["soft-rise", "轻升", "Soft Rise"],
          ["glow-drift", "微光", "Glow Drift"],
        ]}
        columns="sm:grid-cols-4"
        onChange={(entranceStyle) => onChange({ entranceStyle })}
      />
      <DanmakuChoiceGroup
        label="运动方向 / Direction"
        value={settings.direction}
        options={[
          ["rtl", "从右向左", "RTL"],
          ["ltr", "从左向右", "LTR"],
          ["mixed", "双向混合", "Mixed"],
        ]}
        columns="sm:grid-cols-3"
        onChange={(direction) => onChange({ direction })}
      />
      <div className="grid gap-3 sm:grid-cols-4">
        <ToggleLine
          label="Filter repeated / 过滤重复"
          checked={settings.filterRepeated}
          onChange={(checked) => onChange({ filterRepeated: checked })}
        />
        <ToggleLine
          label="Hide long / 屏蔽过长"
          checked={settings.hideLongComments}
          onChange={(checked) => onChange({ hideLongComments: checked })}
        />
        <ToggleLine
          label="Avoid lyrics / 避开歌词"
          checked={settings.avoidLyricsArea}
          onChange={(checked) => onChange({ avoidLyricsArea: checked })}
        />
        <button
          type="button"
          onClick={onClear}
          className="app-transition inline-flex h-9 items-center justify-center gap-2 rounded-full bg-white/[0.07] px-3 text-xs font-semibold text-white/58 hover:bg-white/[0.12] hover:text-white"
        >
          <Trash2 className="h-3.5 w-3.5" />
          Clear Cache / 清理
        </button>
      </div>
    </div>
  );
}

function DanmakuChoiceGroup<T extends string>({
  label,
  value,
  options,
  columns = "grid-cols-3",
  onChange,
}: {
  label: string;
  value: T;
  options: ReadonlyArray<readonly [T, string, string]>;
  columns?: string;
  onChange: (value: T) => void;
}) {
  return (
    <div>
      <p className="mb-2 text-xs font-medium text-white/42">{label}</p>
      <div className={clsx("grid gap-1 rounded-[16px] bg-black/15 p-1", columns)}>
        {options.map(([option, title, subtitle]) => (
          <button
            key={option}
            type="button"
            onClick={() => onChange(option)}
            className={clsx(
              "app-transition min-w-0 rounded-[13px] px-2.5 py-2.5 text-left",
              value === option
                ? "bg-white/12 text-white shadow-[0_8px_22px_rgba(0,0,0,0.16)]"
                : "text-white/38 hover:bg-white/[0.055] hover:text-white/68",
            )}
          >
            <span className="block truncate text-xs font-semibold">{title}</span>
            <span className="mt-0.5 block truncate text-[10px] opacity-55">{subtitle}</span>
          </button>
        ))}
      </div>
    </div>
  );
}

function SourceButton({
  icon: Icon,
  label,
  loading = false,
  disabled = false,
  onClick,
}: {
  icon?: LucideIcon;
  label: string;
  loading?: boolean;
  disabled?: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      className="app-transition inline-flex h-10 items-center justify-center gap-2 rounded-full bg-white/[0.08] px-3 text-xs font-semibold text-[color:var(--settings-text-secondary)] hover:bg-white/[0.13] hover:text-[color:var(--settings-text-primary)] disabled:cursor-not-allowed disabled:text-[color:var(--settings-text-disabled)]"
    >
      {loading ? (
        <Loader2 className="h-4 w-4 animate-spin" />
      ) : Icon ? (
        <Icon className="h-4 w-4" />
      ) : null}
      {label}
    </button>
  );
}

function GuideTopic({
  title,
  subtitle,
  children,
}: {
  title: string;
  subtitle: string;
  children: ReactNode;
}) {
  return (
    <div className="rounded-[18px] border border-white/[0.05] bg-white/[0.025] p-4">
      <p className="text-sm font-semibold text-white/82">{title}</p>
      <p className="mt-0.5 text-[11px] font-medium uppercase tracking-wider text-white/32">
        {subtitle}
      </p>
      <p className="mt-2 text-xs leading-6 text-white/52">{children}</p>
    </div>
  );
}

function ToggleLine({
  label,
  checked,
  onChange,
}: {
  label: string;
  checked: boolean;
  onChange: (checked: boolean) => void;
}) {
  return (
    <label className="flex items-center gap-3 text-xs text-white/52">
      <input
        type="checkbox"
        checked={checked}
        onChange={(event) => onChange(event.target.checked)}
        className="h-4 w-4 accent-white"
      />
      {label}
    </label>
  );
}

function StatusTile({
  icon: Icon,
  title,
  subtitle,
  value,
  muted = false,
}: {
  icon: LucideIcon;
  title: string;
  subtitle: string;
  value: string;
  muted?: boolean;
}) {
  return (
    <div className="app-transition rounded-[24px] border border-white/[0.06] bg-white/[0.045] p-4 shadow-[0_18px_42px_rgba(0,0,0,0.16)] hover:bg-white/[0.065]">
      <div className="flex items-start justify-between gap-4">
        <span className="flex h-11 w-11 items-center justify-center rounded-full bg-white/[0.075] text-white/68">
          <Icon className="h-4 w-4" />
        </span>
        <span
          className={`rounded-full px-3 py-1 text-xs font-semibold ${muted ? "bg-white/[0.055] text-white/36" : "bg-white text-[#17120f]"}`}
        >
          {value}
        </span>
      </div>
      <p className="mt-4 text-sm font-semibold text-white/82">{title}</p>
      <p className="mt-1 text-xs text-white/36">{subtitle}</p>
    </div>
  );
}

function SectionLabel({
  icon: Icon,
  title,
  subtitle,
}: {
  icon: LucideIcon;
  title: string;
  subtitle: string;
}) {
  return (
    <div className="flex items-center gap-3 rounded-[22px] border border-white/[0.06] bg-white/[0.035] px-4 py-3">
      <span className="flex h-9 w-9 items-center justify-center rounded-full bg-white/[0.07] text-white/64">
        <Icon className="h-4 w-4" />
      </span>
      <div>
        <h3 className="text-sm font-semibold text-white/84">{title}</h3>
        <p className="text-xs text-white/36">{subtitle}</p>
      </div>
    </div>
  );
}

function StatusLine({ label, value }: { label: string; value: string }) {
  return (
    <div>
      <p className="text-xs font-medium text-[color:var(--settings-text-muted)]">{label}</p>
      <p className="mt-1 text-sm font-semibold text-[color:var(--settings-text-secondary)]">
        {value}
      </p>
    </div>
  );
}

// Music source diagnostics card. Surfaces the last NetEase playback resolve and
// the last Bilibili danmaku fetch alongside source/service/login state, so a
// failure never stays silent and the user can copy a masked diagnostic text to
// share with support. Cookies/tokens are already masked by the backend; we never
// print raw credentials here.
function MusicSourceDebugCard({
  neteasePlaybackDebug,
  neteaseServiceStatus,
  neteaseLoginStatus,
  bilibiliLoginStatus,
  bilibiliDanmakuDebug,
  musicSourceConfig,
}: {
  neteasePlaybackDebug: NetEasePlaybackDebug | null;
  neteaseServiceStatus: NetEaseServiceStatus | null;
  neteaseLoginStatus: NetEaseLoginStatus | null;
  bilibiliLoginStatus: BilibiliLoginStatus | null;
  bilibiliDanmakuDebug: BilibiliDanmakuDebug | null;
  musicSourceConfig: MusicSourceConfig;
}) {
  const neteaseStage = neteaseServiceStatus?.stage ?? "not_started";
  const triedLevels = neteasePlaybackDebug?.attempts
    ?.map((attempt) => attempt.level)
    .filter(Boolean)
    .join(" → ");

  const lines: string[] = [
    "=== Ome Music · Music Source Diagnostics ===",
    "",
    "[NetEase Source]",
    `  enabled         : ${musicSourceConfig.enabled ? "yes" : "no"}`,
    `  service stage   : ${neteaseStage}`,
    `  service running : ${neteaseServiceStatus?.running ? "yes" : "no"}`,
    `  base url        : ${musicSourceConfig.baseUrl || "(unset)"}`,
    `  has cookie      : ${
      (neteaseLoginStatus?.hasCookie ?? musicSourceConfig.hasToken) ? "yes" : "no"
    }`,
    `  masked cookie   : ${
      neteaseLoginStatus?.maskedCookie || musicSourceConfig.maskedToken || "(none)"
    }`,
    `  status known    : ${neteaseLoginStatus?.loginStatusKnown === false ? "no" : "yes"}`,
    `  login           : ${
      neteaseLoginStatus?.loggedIn
        ? `signed in${neteaseLoginStatus.nickname ? ` as ${neteaseLoginStatus.nickname}` : ""}`
        : neteaseLoginStatus?.expired
          ? "expired"
          : neteaseLoginStatus?.hasCookie || musicSourceConfig.hasToken
            ? "cookie present, status unknown"
            : "signed out"
    }`,
    "",
    "[NetEase Last Playback Resolve]",
    neteasePlaybackDebug
      ? `  song id         : ${neteasePlaybackDebug.requestedSongId}`
      : "  (no playback resolve recorded yet)",
    neteasePlaybackDebug ? `  requested level : ${neteasePlaybackDebug.requestedLevel}` : null,
    neteasePlaybackDebug ? `  endpoint        : ${neteasePlaybackDebug.endpoint}` : null,
    neteasePlaybackDebug
      ? `  response code   : ${neteasePlaybackDebug.responseCode ?? "(none)"}`
      : null,
    neteasePlaybackDebug
      ? `  has playable url: ${neteasePlaybackDebug.hasUrl ? "yes" : "no"}`
      : null,
    neteasePlaybackDebug
      ? `  returned level  : ${neteasePlaybackDebug.returnedLevel ?? "(none)"}`
      : null,
    neteasePlaybackDebug
      ? `  vip status      : ${neteasePlaybackDebug.vipStatus ?? "(none)"}`
      : null,
    triedLevels ? `  tried levels    : ${triedLevels}` : null,
    neteasePlaybackDebug ? `  reason code     : ${neteasePlaybackDebug.reason ?? "(none)"}` : null,
    neteasePlaybackDebug ? `  message         : ${neteasePlaybackDebug.message ?? "(none)"}` : null,
    "",
    "[Bilibili Source]",
    `  login           : ${
      bilibiliLoginStatus?.loggedIn
        ? `signed in${bilibiliLoginStatus.nickname ? ` as ${bilibiliLoginStatus.nickname}` : ""}`
        : bilibiliLoginStatus
          ? "signed out"
          : "unknown"
    }`,
    bilibiliDanmakuDebug ? `  bvid            : ${bilibiliDanmakuDebug.bvid}` : null,
    bilibiliDanmakuDebug ? `  cid             : ${bilibiliDanmakuDebug.cid || "(none)"}` : null,
    bilibiliDanmakuDebug ? `  danmaku count   : ${bilibiliDanmakuDebug.parsedDanmakuCount}` : null,
    bilibiliDanmakuDebug
      ? `  from cache      : ${bilibiliDanmakuDebug.fromCache ? "yes" : "no"}`
      : null,
    bilibiliDanmakuDebug ? `  error           : ${bilibiliDanmakuDebug.error ?? "(none)"}` : null,
  ].filter((line): line is string => line !== null);

  const diagnosticsText = lines.join("\n");

  const handleCopy = () => {
    void navigator.clipboard.writeText(diagnosticsText).catch(() => {
      /* clipboard may be unavailable; the visible block still serves the user */
    });
  };

  return (
    <div className="settings-surface space-y-3">
      <div className="flex items-center justify-between gap-3">
        <div>
          <p className="text-sm font-semibold text-white/74">
            音乐源诊断 / Music Source Diagnostics
          </p>
          <p className="mt-1 text-xs leading-5 text-white/36">
            最近一次网易云播放解析与 Bilibili 弹幕拉取的诊断摘要。Cookie / Token
            已脱敏，可安全复制。
          </p>
        </div>
        <button
          type="button"
          onClick={handleCopy}
          className="app-transition inline-flex h-8 shrink-0 items-center justify-center gap-1.5 rounded-full bg-white/[0.08] px-3 text-[11px] font-semibold text-white/72 hover:bg-white/[0.13] hover:text-white"
        >
          复制 / Copy
        </button>
      </div>
      <pre className="max-h-72 overflow-auto whitespace-pre-wrap break-words rounded-2xl bg-black/30 p-3 font-mono text-[11px] leading-5 text-white/56">
        {diagnosticsText}
      </pre>
    </div>
  );
}

// NetEase 本地服务状态卡：在「音乐来源」面板中显示 stage 与轻量进度指示。
// 终态用单色小点区分，进行中用旋转 loader；失败时显示 message 与「重试 / Retry」。
function NetEaseServiceStatusCard({
  status,
  isRefreshing,
  onRetry,
}: {
  status: NetEaseServiceStatus | null;
  isRefreshing: boolean;
  onRetry: () => void;
}) {
  const stage: NetEaseServiceStage = status?.stage ?? "not_started";
  const label = neteaseServiceStageLabel(stage);
  const isPending =
    stage === "checking_runtime" ||
    stage === "checking_api" ||
    stage === "starting_service" ||
    stage === "waiting_health";
  const isReady = stage === "ready";
  const isFailed = stage === "failed";

  return (
    <div className="rounded-[20px] bg-white/[0.035] p-4">
      <div className="flex items-center justify-between gap-3">
        <div className="flex min-w-0 items-center gap-2">
          {isPending ? (
            <Loader2 className="h-3.5 w-3.5 shrink-0 animate-spin text-white/52" />
          ) : (
            <span
              className={`h-1.5 w-1.5 shrink-0 rounded-full ${
                isReady ? "bg-emerald-300/80" : isFailed ? "bg-rose-300/80" : "bg-white/30"
              }`}
            />
          )}
          <p className="truncate text-xs font-semibold text-white/64">{label}</p>
        </div>
        {isFailed && (
          <button
            type="button"
            onClick={onRetry}
            disabled={isRefreshing}
            className="app-transition inline-flex h-8 shrink-0 items-center justify-center gap-1.5 rounded-full bg-white/[0.08] px-3 text-[11px] font-semibold text-white/72 hover:bg-white/[0.13] hover:text-white disabled:cursor-not-allowed disabled:opacity-45"
          >
            {isRefreshing ? (
              <Loader2 className="h-3 w-3 animate-spin" />
            ) : (
              <RefreshCw className="h-3 w-3" />
            )}
            重试 / Retry
          </button>
        )}
      </div>
      {isFailed && status?.message ? (
        <p className="mt-2 text-[11px] leading-5 text-white/38">{status.message}</p>
      ) : null}
    </div>
  );
}

function neteaseServiceStageLabel(stage: NetEaseServiceStage): string {
  switch (stage) {
    case "not_started":
      return "未启动 / Not started";
    case "checking_runtime":
      return "正在检查运行环境 / Checking runtime...";
    case "checking_api":
      return "正在检查 API / Checking API files...";
    case "starting_service":
      return "正在启动网易云服务 / Starting NetEase service...";
    case "waiting_health":
      return "正在等待服务响应 / Waiting for service...";
    case "ready":
      // Honest label: this stage only means the local managed runtime is
      // listening — it says nothing about whether the user is signed in.
      // The Login status line above is the source of truth for the session.
      return "本地服务已就绪 / Local service ready (sign in to play)";
    case "failed":
      return "启动失败 / Failed to start";
    default:
      return "未启动 / Not started";
  }
}

function StorageRow({
  icon: Icon,
  label,
  value,
}: {
  icon: LucideIcon;
  label: string;
  value: string;
}) {
  return (
    <div className="flex items-center justify-between gap-3 rounded-[18px] bg-white/[0.04] px-3 py-2.5">
      <div className="flex min-w-0 items-center gap-2">
        <Icon className="h-4 w-4 shrink-0 text-white/38" />
        <span className="truncate text-xs font-semibold text-white/58">{label}</span>
      </div>
      <span className="shrink-0 text-xs font-semibold text-white/78">{value}</span>
    </div>
  );
}

function StorageAction({
  label,
  sublabel,
  kind,
  activeKind,
  onClick,
}: {
  label: string;
  sublabel: string;
  kind: StorageBucketKind;
  activeKind: StorageBucketKind | null;
  onClick: (kind: StorageBucketKind) => void;
}) {
  const isActive = activeKind === kind;
  return (
    <button
      type="button"
      onClick={() => onClick(kind)}
      disabled={Boolean(activeKind)}
      className="app-transition inline-flex min-h-12 flex-col items-center justify-center rounded-[18px] bg-white/[0.07] px-3 py-2 text-xs font-semibold text-white/62 hover:bg-white/[0.12] hover:text-white disabled:cursor-wait disabled:opacity-45"
    >
      {isActive ? (
        <Loader2 className="mb-1 h-3.5 w-3.5 animate-spin" />
      ) : (
        <Trash2 className="mb-1 h-3.5 w-3.5" />
      )}
      <span>{label}</span>
      <span className="text-[10px] text-white/36">{sublabel}</span>
    </button>
  );
}

function qrStatusMessage(status: string): string {
  switch (status) {
    case "waiting":
      return "等待扫码 / Waiting for scan";
    case "confirmed":
      return "已扫码，请在手机上确认 / Scanned. Please confirm on your phone.";
    case "success":
      return "登录成功 / Connected";
    case "expired":
      return "二维码已过期 / QR code expired. Refresh to try again.";
    case "unknown":
      return "状态未知，正在重试 / Status unknown, retrying…";
    default:
      return "Could not confirm the sign-in status.";
  }
}

function Field({ label, children }: { label: string; children: ReactNode }) {
  return (
    <label className="block">
      <span className="mb-2 block text-xs font-medium text-[color:var(--settings-text-muted)]">
        {label}
      </span>
      {children}
    </label>
  );
}

function describeQQMusicAuthState(
  status: QQMusicLoginStatus | null,
  enabled: boolean,
  mode: "compact" | "detail",
): string {
  if (status?.loggedIn && status.status === "authenticated") {
    if (mode === "compact") return "Signed in";
    return `已连接${status.nickname ? ` · ${status.nickname}` : ""} / Signed in`;
  }
  if (status?.status === "expired") {
    return mode === "compact" ? "Reconnect" : "登录已过期，请重新连接 / Session expired";
  }
  if (
    status?.credentialPresent &&
    ["credential_present", "verifying", "unknown"].includes(status.status)
  ) {
    return mode === "compact"
      ? "Session saved"
      : status.message ||
          "官方会话已保存，账号权限将在播放时确认 / Official session saved; account access will be confirmed during playback";
  }
  if (status?.status === "failed") {
    return mode === "compact" ? "Check sign-in" : status.message;
  }
  if (!enabled) return mode === "compact" ? "Off" : "未启用 / Disabled";
  return mode === "compact" ? "Ready" : "公共内容可用 / Public content available";
}

function readError(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function loginErrorMessage(error: unknown): string {
  const message = readError(error);
  if (/captcha|security|verification|verify|risk|device/i.test(message)) {
    return "需要完成安全验证 / Additional verification is required. 请使用官方网页登录继续。";
  }
  if (/sms|code required/i.test(message)) {
    return "需要短信验证 / A verification code is required.";
  }
  if (/expired|session/i.test(message)) {
    return "登录会话已失效 / Your session has expired. Please sign in again.";
  }
  if (/password|account|credential|incorrect/i.test(message)) {
    return "账号或密码可能不正确 / The account or password may be incorrect.";
  }
  return message;
}

function maskPhone(phone: string): string {
  const trimmed = phone.trim();
  if (trimmed.length < 7) return trimmed || "your phone";
  return `${trimmed.slice(0, 3)}****${trimmed.slice(-4)}`;
}
