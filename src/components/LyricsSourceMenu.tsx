import { useEffect, useRef, useState } from "react";
import type React from "react";
import {
  Clock3,
  Cloud,
  FileMusic,
  FolderOpen,
  Radio,
  RefreshCw,
  Settings2,
  SlidersHorizontal,
  Wind,
} from "lucide-react";
import {
  BilibiliAccountSessionProvider,
  getBilibiliSourceConfig,
  getNeteaseSourceConfig,
  getQQMusicSourceConfig,
  type BilibiliLoginStatus,
  type BilibiliSourceConfig,
  type MusicSourceConfig,
  type NetEaseLoginStatus,
  type NetEasePlaybackDebug,
  type NetEaseServiceStatus,
  type PlayableUrlOptions,
  type QQMusicLoginStatus,
  type QQMusicSourceConfig,
} from "../features/musicSources/provider";
import type { Track } from "../types/music";
import { getDanmakuSettings, type DanmakuSettings } from "../features/danmaku/danmakuSettings";

interface LyricsSourceMenuProps {
  track: Track | null;
  localTrackCount: number;
  lyricOffsetMs: number;
  playbackDebug: NetEasePlaybackDebug | null;
  serviceStatus: NetEaseServiceStatus | null;
  loginStatus: NetEaseLoginStatus | null;
  bilibiliLoginStatus: BilibiliLoginStatus | null;
  qqmusicLoginStatus: QQMusicLoginStatus | null;
  playbackQuality: NonNullable<PlayableUrlOptions["level"]>;
  onReloadLyrics: () => void;
  onImportLyrics: () => void;
  onAdjustLyricOffset: (deltaMs: number) => void;
  onResetLyricOffset: () => void;
  onPlaybackQualityChange: (level: NonNullable<PlayableUrlOptions["level"]>) => void;
  onOpenSettings: () => void;
  onOpenAtmosphereSettings: () => void;
  open: boolean;
  onOpen: () => void;
  onClose: () => void;
}

const qualityOptions: Array<{ value: NonNullable<PlayableUrlOptions["level"]>; label: string }> = [
  { value: "hires", label: "Hi-Res" },
  { value: "lossless", label: "Lossless" },
  { value: "exhigh", label: "Extra High" },
  { value: "higher", label: "Higher" },
  { value: "standard", label: "Standard" },
];

const bilibiliAccount = new BilibiliAccountSessionProvider();

export function LyricsSourceMenu({
  track,
  localTrackCount,
  lyricOffsetMs,
  playbackDebug,
  serviceStatus,
  loginStatus,
  bilibiliLoginStatus: _bilibiliLoginStatus,
  qqmusicLoginStatus,
  playbackQuality,
  onReloadLyrics,
  onImportLyrics,
  onAdjustLyricOffset,
  onResetLyricOffset,
  onPlaybackQualityChange,
  onOpenSettings,
  onOpenAtmosphereSettings,
  open,
  onOpen,
  onClose,
}: LyricsSourceMenuProps) {
  const rootRef = useRef<HTMLDivElement | null>(null);
  const [neteaseConfig, setNeteaseConfig] = useState<MusicSourceConfig | null>(null);
  const [bilibiliConfig, setBilibiliConfig] = useState<BilibiliSourceConfig | null>(null);
  const [bilibiliStatus, setBilibiliStatus] = useState<BilibiliLoginStatus | null>(null);
  const [qqmusicConfig, setQQMusicConfig] = useState<QQMusicSourceConfig | null>(null);
  const [danmakuSettings, setDanmakuSettings] = useState<DanmakuSettings>(() =>
    getDanmakuSettings(),
  );

  useEffect(() => {
    if (!open) return;
    const handlePointerDown = (event: PointerEvent) => {
      if (!rootRef.current?.contains(event.target as Node)) onClose();
    };
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("pointerdown", handlePointerDown);
    window.addEventListener("keydown", handleKeyDown);
    return () => {
      window.removeEventListener("pointerdown", handlePointerDown);
      window.removeEventListener("keydown", handleKeyDown);
    };
  }, [onClose, open]);

  useEffect(() => {
    const handleDanmakuSettings = (event: Event) => {
      const custom = event as CustomEvent<DanmakuSettings>;
      setDanmakuSettings(custom.detail ?? getDanmakuSettings());
    };
    window.addEventListener("ome:danmaku-settings", handleDanmakuSettings);
    window.addEventListener("storage", handleDanmakuSettings);
    return () => {
      window.removeEventListener("ome:danmaku-settings", handleDanmakuSettings);
      window.removeEventListener("storage", handleDanmakuSettings);
    };
  }, []);

  useEffect(() => {
    if (!open) return;
    let cancelled = false;
    void Promise.allSettled([
      getNeteaseSourceConfig(),
      getBilibiliSourceConfig(),
      bilibiliAccount.getLoginStatus(),
      getQQMusicSourceConfig(),
    ]).then(([netease, bilibili, status, qqmusic]) => {
      if (cancelled) return;
      if (netease.status === "fulfilled") setNeteaseConfig(netease.value);
      if (bilibili.status === "fulfilled") setBilibiliConfig(bilibili.value);
      if (status.status === "fulfilled") setBilibiliStatus(status.value);
      if (qqmusic.status === "fulfilled") setQQMusicConfig(qqmusic.value);
    });
    return () => {
      cancelled = true;
    };
  }, [open]);

  const currentSource = sourceLabel(track?.source);
  const sourceReady =
    track?.source === "local" ||
    (track?.source === "netease" && Boolean(serviceStatus?.running || playbackDebug?.hasUrl)) ||
    (track?.source === "bilibili" && Boolean(bilibiliConfig?.enabled)) ||
    (track?.source === "qqmusic" && Boolean(qqmusicConfig?.enabled));

  return (
    <div
      ref={rootRef}
      data-danmaku-safe-zone="settings"
      // UI subtraction (v0.3.7): the top-right Quick Settings trigger used to
      // be a solid bordered shadowed button always shouting from the corner.
      // It now rests faint (opacity ~45%) and only brightens on hover/focus,
      // so the corner stops competing with the cover/lyrics focal point.
      // Position, size and behavior are unchanged.
      className="fixed right-8 top-8 z-50 opacity-45 transition-opacity duration-500 ease-out hover:opacity-100 focus-within:opacity-100"
    >
      <button
        type="button"
        onClick={() => (open ? onClose() : onOpen())}
        className="quick-settings-trigger app-transition relative flex h-9 w-9 items-center justify-center rounded-[12px] border border-white/10 bg-white/[0.06] text-[#4a2108]/34 shadow-[0_6px_18px_rgba(74,33,8,0.05)] backdrop-blur-2xl hover:bg-white/[0.2] hover:text-[#4a2108]/68"
        aria-label="Quick Settings"
        aria-expanded={open}
      >
        <span
          className={`absolute right-1.5 top-1.5 h-1.5 w-1.5 rounded-full ${sourceReady ? "bg-[#638052]/70" : "bg-[#8b6f5d]/35"}`}
        />
        <SlidersHorizontal className="h-4 w-4" />
      </button>

      {open && (
        <div
          className="quick-settings-panel settings-scroll mt-3 max-h-[calc(100svh-6rem)] w-[min(360px,calc(100vw-3rem))] touch-pan-y overflow-y-auto overscroll-contain rounded-[26px] p-5"
          onWheel={(event) => event.stopPropagation()}
        >
          <header className="flex items-start justify-between gap-4">
            <div className="min-w-0">
              <p className="text-[10px] font-black uppercase tracking-[0.2em] text-[#4a2108]/28">
                Quick Settings
              </p>
              <h2 className="mt-1.5 truncate text-[17px] font-bold text-[#4a2108]/82">
                {track?.title ?? "A quiet room"}
              </h2>
              <p className="mt-1 text-xs font-semibold text-[#4a2108]/34">{currentSource}</p>
            </div>
            <span
              className={`mt-1 h-2 w-2 shrink-0 rounded-full ${sourceReady ? "bg-[#638052]/75 shadow-[0_0_14px_rgba(99,128,82,0.42)]" : "bg-[#8b6f5d]/35"}`}
            />
          </header>

          <QuickSection title="Lyrics">
            <div className="grid grid-cols-2 gap-2">
              <QuickAction
                icon={RefreshCw}
                label="Search again"
                sublabel="Search again"
                onClick={onReloadLyrics}
              />
              <QuickAction
                icon={FileMusic}
                label="Choose .lrc"
                sublabel="Choose .lrc"
                onClick={onImportLyrics}
              />
            </div>
            <div className="mt-2.5 rounded-[16px] bg-[#4a2108]/[0.045] p-2">
              <div className="mb-2 flex items-center gap-2 px-1.5 text-[11px] font-bold text-[#4a2108]/38">
                <Clock3 className="h-3.5 w-3.5" />
                <span>Timing</span>
                <span className="ml-auto tabular-nums">
                  {lyricOffsetMs > 0 ? "+" : ""}
                  {lyricOffsetMs}ms
                </span>
              </div>
              <div className="grid grid-cols-3 gap-1.5">
                <button
                  type="button"
                  onClick={() => onAdjustLyricOffset(-500)}
                  className="quick-settings-pill"
                >
                  -500ms
                </button>
                <button type="button" onClick={onResetLyricOffset} className="quick-settings-pill">
                  Reset
                </button>
                <button
                  type="button"
                  onClick={() => onAdjustLyricOffset(500)}
                  className="quick-settings-pill"
                >
                  +500ms
                </button>
              </div>
            </div>
          </QuickSection>

          <QuickSection title="Sources">
            <div className="space-y-1.5">
              <SourceStatusRow
                icon={FolderOpen}
                name="Local Music"
                status={`${localTrackCount} tracks`}
                ready
              />
              <SourceStatusRow
                icon={Cloud}
                name="NetEase"
                status={neteaseStatusLabel({ serviceStatus, loginStatus, neteaseConfig })}
                ready={Boolean(serviceStatus?.running && neteaseConfig?.enabled)}
              />
              <SourceStatusRow
                icon={Radio}
                name="Bilibili"
                status={
                  bilibiliStatus?.loggedIn
                    ? "Connected"
                    : bilibiliConfig?.enabled
                      ? "Public content available"
                      : "Off"
                }
                ready={Boolean(bilibiliConfig?.enabled)}
              />
              <SourceStatusRow
                icon={Cloud}
                name="QQ音乐"
                status={
                  qqmusicLoginStatus?.loggedIn
                    ? "已连接"
                    : qqmusicConfig?.enabled
                      ? "公共内容可用"
                      : "关闭"
                }
                ready={Boolean(qqmusicConfig?.enabled)}
              />
            </div>
          </QuickSection>

          <QuickSection title="Playback">
            <label className="flex items-center justify-between gap-4 rounded-[16px] bg-[#4a2108]/[0.045] px-3 py-2.5">
              <span className="text-xs font-bold text-[#4a2108]/48">Preferred quality</span>
              <select
                value={playbackQuality}
                onChange={(event) =>
                  onPlaybackQualityChange(
                    event.target.value as NonNullable<PlayableUrlOptions["level"]>,
                  )
                }
                className="rounded-full bg-[#4a2108]/[0.07] px-3 py-1.5 text-xs font-bold text-[#4a2108]/62 outline-none"
              >
                {qualityOptions.map((option) => (
                  <option key={option.value} value={option.value}>
                    {option.label}
                  </option>
                ))}
              </select>
            </label>
          </QuickSection>

          <QuickSection title="Atmosphere">
            <button
              type="button"
              onClick={() => {
                onClose();
                onOpenAtmosphereSettings();
              }}
              className="app-transition flex w-full items-center gap-3 rounded-[16px] bg-[#4a2108]/[0.045] px-3 py-2.5 text-left hover:bg-[#4a2108]/[0.085]"
            >
              <Wind className="h-4 w-4 shrink-0 text-[#4a2108]/42" />
              <span className="min-w-0 flex-1">
                <span className="block text-xs font-bold text-[#4a2108]/62">Danmaku mood</span>
                <span className="block truncate text-[10px] text-[#4a2108]/28">
                  {danmakuLabel(danmakuSettings)}
                </span>
              </span>
              <span
                className={`h-1.5 w-1.5 shrink-0 rounded-full ${danmakuSettings.enabled ? "bg-[#638052]/70" : "bg-[#8b6f5d]/28"}`}
              />
            </button>
          </QuickSection>

          <button
            type="button"
            onClick={() => {
              onClose();
              onOpenSettings();
            }}
            className="app-transition mt-4 flex w-full items-center gap-3 rounded-[16px] bg-[#4a2108]/[0.065] px-3 py-3 text-left text-[#4a2108]/58 hover:bg-[#4a2108]/[0.1] hover:text-[#4a2108]/82"
          >
            <Settings2 className="h-4 w-4" />
            <span className="flex-1 text-xs font-bold">Open full settings / More Settings</span>
          </button>
        </div>
      )}
    </div>
  );
}

function QuickSection({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <section className="mt-4">
      <p className="mb-2 text-[10px] font-black uppercase tracking-[0.16em] text-[#4a2108]/26">
        {title}
      </p>
      {children}
    </section>
  );
}

function QuickAction({
  icon: Icon,
  label,
  sublabel,
  onClick,
}: {
  icon: typeof RefreshCw;
  label: string;
  sublabel: string;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className="app-transition flex items-center gap-2.5 rounded-[16px] bg-[#4a2108]/[0.045] px-3 py-2.5 text-left hover:bg-[#4a2108]/[0.085]"
    >
      <Icon className="h-4 w-4 shrink-0 text-[#4a2108]/42" />
      <span className="min-w-0">
        <span className="block text-xs font-bold text-[#4a2108]/62">{label}</span>
        <span className="block truncate text-[10px] text-[#4a2108]/28">{sublabel}</span>
      </span>
    </button>
  );
}

function SourceStatusRow({
  icon: Icon,
  name,
  status,
  ready,
}: {
  icon: typeof Cloud;
  name: string;
  status: string;
  ready: boolean;
}) {
  return (
    <div className="flex items-center gap-3 rounded-[14px] px-2.5 py-2 text-[#4a2108]/54">
      <Icon className="h-4 w-4 shrink-0 opacity-60" />
      <span className="flex-1 text-xs font-bold">{name}</span>
      <span className="text-[10px] font-semibold text-[#4a2108]/32">{status}</span>
      <span
        className={`h-1.5 w-1.5 rounded-full ${ready ? "bg-[#638052]/70" : "bg-[#8b6f5d]/28"}`}
      />
    </div>
  );
}

function sourceLabel(source?: string): string {
  if (source === "netease") return "\u7f51\u6613\u4e91\u97f3\u4e50 / NetEase Cloud Music";
  if (source === "bilibili") return "Bilibili Video Atmosphere";
  if (source === "qqmusic") return "QQ\u97f3\u4e50";
  return "\u672c\u5730\u97f3\u4e50 / Local Library";
}

function neteaseStatusLabel({
  serviceStatus,
  loginStatus,
  neteaseConfig,
}: {
  serviceStatus: NetEaseServiceStatus | null;
  loginStatus: NetEaseLoginStatus | null;
  neteaseConfig: MusicSourceConfig | null;
}): string {
  // Carefully separate four distinct states:
  //   1. Source enabled?
  //   2. Service (managed runtime) ready?
  //   3. User signed in (loginStatus.loggedIn && !expired)?
  //   4. Session expired (loginStatus.expired)?
  // "Available" only means the API service is reachable; it must NOT be
  // mistaken for "signed in". Previously a service-ready state masked a missing
  // login, so the search page told users to sign in even when they were signed in.
  if (!neteaseConfig?.enabled) return "\u5173\u95ed";
  if (serviceStatus && !serviceStatus.running && !serviceStatus.apiPackageFound)
    return "\u9700\u91cd\u65b0\u5b89\u88c5";
  if (loginStatus?.expired) return "\u767b\u5f55\u5df2\u8fc7\u671f";
  if (loginStatus?.loggedIn) return "\u5df2\u767b\u5f55";
  if (serviceStatus?.running) return "\u672a\u767b\u5f55";
  return "\u53ef\u7528";
}

function danmakuLabel(settings: DanmakuSettings): string {
  if (!settings.enabled) return "\u5173\u95ed / Off";
  const mode =
    settings.displayMode === "video"
      ? "\u89c6\u9891\u6c1b\u56f4"
      : settings.displayMode === "ambient"
        ? "\u5168\u5c40\u6c1b\u56f4"
        : "\u5173\u95ed";
  const intensity =
    settings.emotionalIntensity === "quiet"
      ? "\u5b89\u9759"
      : settings.emotionalIntensity === "expressive"
        ? "\u9c9c\u660e"
        : "\u5e73\u8861";
  return `${mode} \u00b7 ${intensity} \u00b7 ${Math.round(settings.opacity * 100)}%`;
}
