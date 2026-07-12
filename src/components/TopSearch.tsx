import { useEffect, useMemo, useRef, useState } from "react";
import type React from "react";
import { ChevronDown, Loader2, Music2, RefreshCw, Search, Settings, X } from "lucide-react";
import {
  BilibiliMusicProvider,
  ensureNeteaseApiService,
  getBilibiliSourceConfig,
  getNeteaseSourceConfig,
  getQQMusicSourceConfig,
  NetEaseMusicProvider,
  QQMusicProvider,
  waitForNeteaseServiceReady,
  type MusicSourceSong,
} from "../features/musicSources/provider";
import type { Track } from "../types/music";
import { resolveTrackCover } from "../features/artwork/resolveTrackCover";
import { formatDuration } from "../utils/format";
import { ArtworkImage } from "./ArtworkImage";

const neteaseProvider = new NetEaseMusicProvider();
const bilibiliProvider = new BilibiliMusicProvider();
const qqmusicProvider = new QQMusicProvider();

// Client-side page size: providers already return enough rows; keep the UI light.
const SOURCE_PAGE_SIZE = 6;
interface TopSearchProps {
  tracks: Track[];
  onPlayLocal: (track: Track) => void;
  onPlayNetEase: (song: MusicSourceSong) => Promise<void | boolean>;
  onPlayBilibili: (song: MusicSourceSong) => Promise<void | boolean>;
  onPlayQQMusic: (song: MusicSourceSong) => Promise<void | boolean>;
  onOpenSettings?: () => void;
}

export function TopSearch({
  tracks,
  onPlayLocal,
  onPlayNetEase,
  onPlayBilibili,
  onPlayQQMusic,
  onOpenSettings,
}: TopSearchProps) {
  const rootRef = useRef<HTMLDivElement | null>(null);
  const bilibiliPlayRequestRef = useRef<number>(0);
  const closeSoonTimerRef = useRef<number | null>(null);
  const [query, setQuery] = useState("");
  const [isOpen, setOpen] = useState(false);
  const [neteaseEnabled, setNeteaseEnabled] = useState(false);
  const [bilibiliEnabled, setBilibiliEnabled] = useState(false);
  const [neteaseResults, setNeteaseResults] = useState<MusicSourceSong[]>([]);
  const [bilibiliResults, setBilibiliResults] = useState<MusicSourceSong[]>([]);
  const [isSearchingSource, setSearchingSource] = useState(false);
  const [isSearchingBilibili, setSearchingBilibili] = useState(false);
  const [neteaseMessage, setNeteaseMessage] = useState<string | null>(null);
  const [bilibiliMessage, setBilibiliMessage] = useState<string | null>(null);
  const [qqmusicEnabled, setQQMusicEnabled] = useState(false);
  const [qqmusicResults, setQQMusicResults] = useState<MusicSourceSong[]>([]);
  const [isSearchingQQMusic, setSearchingQQMusic] = useState(false);
  const [qqmusicMessage, setQQMusicMessage] = useState<string | null>(null);
  const [playingBilibiliId, setPlayingBilibiliId] = useState<string | null>(null);
  const [neteaseVisible, setNeteaseVisible] = useState(SOURCE_PAGE_SIZE);
  const [bilibiliVisible, setBilibiliVisible] = useState(SOURCE_PAGE_SIZE);
  const [qqmusicVisible, setQQMusicVisible] = useState(SOURCE_PAGE_SIZE);
  // NetEase can still be warming up right after QR login or first launch.
  const [neteaseServiceStarting, setNeteaseServiceStarting] = useState(false);
  const [neteaseServiceError, setNeteaseServiceError] = useState<string | null>(null);
  // Increment this token to retry the source warm-up search effect.
  const [neteaseRetryToken, setNeteaseRetryToken] = useState(0);

  const localResults = useMemo(() => {
    const needle = query.trim().toLowerCase();
    if (!needle) return tracks.slice(0, 8);
    return tracks
      .filter((track) =>
        `${track.title} ${track.artist} ${track.album}`.toLowerCase().includes(needle),
      )
      .slice(0, 8);
  }, [query, tracks]);

  useEffect(() => {
    let cancelled = false;
    refreshSourceConfig()
      .then((config) => {
        if (!cancelled) setNeteaseEnabled(config.enabled);
      })
      .catch(() => {
        if (!cancelled) setNeteaseEnabled(false);
      });
    getBilibiliSourceConfig()
      .then((config) => {
        if (!cancelled) setBilibiliEnabled(config.enabled);
      })
      .catch(() => {
        if (!cancelled) setBilibiliEnabled(false);
      });
    getQQMusicSourceConfig()
      .then((config) => {
        if (!cancelled) setQQMusicEnabled(config.enabled);
      })
      .catch(() => {
        if (!cancelled) setQQMusicEnabled(false);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const refreshSourceConfig = () => getNeteaseSourceConfig();

  useEffect(() => {
    const term = query.trim();
    setNeteaseMessage(null);
    setNeteaseServiceError(null);

    if (!isOpen || !neteaseEnabled || term.length < 2) {
      setNeteaseResults([]);
      setNeteaseServiceStarting(false);
      return;
    }

    // Reset visible result count for every new search.
    setNeteaseVisible(SOURCE_PAGE_SIZE);

    let cancelled = false;
    const timer = window.setTimeout(async () => {
      // NetEase may still be warming up right after QR login or first launch.
      // Wait briefly in the UI instead of failing silently.
      try {
        const status = await ensureNeteaseApiService();
        if (cancelled) return;
        if (status.stage !== "ready") {
          setNeteaseServiceStarting(true);
          const waited = await waitForNeteaseServiceReady();
          if (cancelled) return;
          setNeteaseServiceStarting(false);
          if (waited.stage !== "ready") {
            setNeteaseResults([]);
            setNeteaseServiceError(
              waited.message || "网易云音乐源暂时不可用 / NetEase source is unavailable right now.",
            );
            return;
          }
        }
      } catch (error) {
        if (cancelled) return;
        setNeteaseServiceStarting(false);
        setNeteaseResults([]);
        setNeteaseServiceError(readNeteaseServiceError(error));
        return;
      }

      if (cancelled) return;
      setSearchingSource(true);
      neteaseProvider
        .searchSongs(term)
        .then((songs) => {
          // Keep all returned rows; the UI handles light client-side paging.
          if (!cancelled) setNeteaseResults(songs);
        })
        .catch((error) => {
          if (!cancelled) {
            setNeteaseResults([]);
            setNeteaseMessage(readNeteaseSearchError(error));
          }
        })
        .finally(() => {
          if (!cancelled) setSearchingSource(false);
        });
    }, 260);

    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [isOpen, neteaseEnabled, query, neteaseRetryToken]);

  useEffect(() => {
    const term = query.trim();
    setBilibiliMessage(null);

    if (!isOpen || !bilibiliEnabled || term.length < 2) {
      setBilibiliResults([]);
      return;
    }

    setBilibiliVisible(SOURCE_PAGE_SIZE);

    let cancelled = false;
    const timer = window.setTimeout(() => {
      setSearchingBilibili(true);
      bilibiliProvider
        .searchSongs(term)
        .then((songs) => {
          if (!cancelled) setBilibiliResults(songs);
        })
        .catch((error) => {
          if (!cancelled) {
            setBilibiliResults([]);
            setBilibiliMessage(readSourceError(error));
          }
        })
        .finally(() => {
          if (!cancelled) setSearchingBilibili(false);
        });
    }, 480);

    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [bilibiliEnabled, isOpen, query]);

  // QQ 音乐搜索
  useEffect(() => {
    const term = query.trim();
    setQQMusicMessage(null);

    if (!isOpen || !qqmusicEnabled || term.length < 2) {
      setQQMusicResults([]);
      return;
    }

    setQQMusicVisible(SOURCE_PAGE_SIZE);

    let cancelled = false;
    const timer = window.setTimeout(() => {
      setSearchingQQMusic(true);
      qqmusicProvider
        .searchSongs(term)
        .then((songs) => {
          if (!cancelled) setQQMusicResults(songs);
        })
        .catch((error) => {
          if (!cancelled) {
            setQQMusicResults([]);
            setQQMusicMessage(readSourceError(error, "qqmusic"));
          }
        })
        .finally(() => {
          if (!cancelled) setSearchingQQMusic(false);
        });
    }, 600);

    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [qqmusicEnabled, isOpen, query]);

  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") setOpen(false);
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, []);

  useEffect(
    () => () => {
      if (closeSoonTimerRef.current !== null) window.clearTimeout(closeSoonTimerRef.current);
    },
    [],
  );

  const closeSoon = () => {
    if (closeSoonTimerRef.current !== null) window.clearTimeout(closeSoonTimerRef.current);
    closeSoonTimerRef.current = window.setTimeout(() => {
      closeSoonTimerRef.current = null;
      if (!rootRef.current?.contains(document.activeElement)) setOpen(false);
    }, 120);
  };

  return (
    <div
      ref={rootRef}
      data-danmaku-safe-zone="search"
      // UI subtraction (v0.3.7): the search bar is the largest always-on chrome
      // competing with the cover/lyrics focal point. It now stays faint at rest
      // (opacity ~55%) and only brightens when the user approaches it — hover
      // or focus-within. Position, width and behavior are unchanged, so
      // discoverability and the open-results flow stay intact.
      className="fixed left-1/2 top-5 z-40 w-[min(34vw,560px)] min-w-[300px] max-w-[calc(100vw-8rem)] -translate-x-1/2 opacity-55 transition-opacity duration-500 ease-out hover:opacity-100 focus-within:opacity-100 max-md:left-6 max-md:right-20 max-md:w-auto max-md:min-w-0 max-md:max-w-none max-md:translate-x-0"
    >
      <div className="search-glass flex h-10 items-center gap-2.5 rounded-full px-3.5">
        <Search className="h-3.5 w-3.5 shrink-0 text-[#4a2108]/28" />
        <input
          value={query}
          onChange={(event) => setQuery(event.target.value)}
          onFocus={() => {
            setOpen(true);
            void refreshSourceConfig()
              .then((config) => setNeteaseEnabled(config.enabled))
              .catch(() => setNeteaseEnabled(false));
            void getBilibiliSourceConfig()
              .then((config) => setBilibiliEnabled(config.enabled))
              .catch(() => setBilibiliEnabled(false));
            void getQQMusicSourceConfig()
              .then((config) => setQQMusicEnabled(config.enabled))
              .catch(() => setQQMusicEnabled(false));
          }}
          onBlur={closeSoon}
          placeholder="Find a song for this moment..."
          className="h-full min-w-0 flex-1 bg-transparent text-[13px] font-semibold text-[#4a2108]/68 outline-none placeholder:text-[#4a2108]/24"
        />
        {query && (
          <button
            type="button"
            onMouseDown={(event) => event.preventDefault()}
            onClick={() => {
              setQuery("");
              setNeteaseResults([]);
              setBilibiliResults([]);
            }}
            className="app-transition flex h-7 w-7 items-center justify-center rounded-full text-[#4a2108]/38 hover:bg-[#4a2108]/10 hover:text-[#4a2108]/72"
            aria-label="Clear search"
          >
            <X className="h-3.5 w-3.5" />
          </button>
        )}
      </div>

      {isOpen && query.trim().length > 0 && (
        <div
          data-testid="search-results"
          className="search-popover search-results-scroll mt-4 min-h-0 max-h-[min(62svh,calc(100svh-6.75rem),520px)] touch-pan-y overflow-y-auto overscroll-contain rounded-[24px] pb-3 pl-3 pr-2 pt-5"
          onWheel={(event) => event.stopPropagation()}
        >
          <SearchGroup title="Local Library">
            {localResults.length ? (
              localResults.map((track) => {
                const artwork = resolveTrackCover(track);
                return (
                  <button
                    key={track.id}
                    type="button"
                    aria-label={`Play ${track.title} by ${track.artist}`}
                    onMouseDown={(event) => event.preventDefault()}
                    onClick={() => {
                      onPlayLocal(track);
                      setOpen(false);
                    }}
                    className="search-result-row"
                  >
                    <ArtworkImage
                      src={artwork.src}
                      alt={track.album}
                      source={track.source}
                      className="h-10 w-10 shrink-0 rounded-[10px] object-cover shadow-[0_10px_24px_rgba(74,33,8,0.18)]"
                    />
                    <ResultText title={track.title} subtitle={track.artist} />
                  </button>
                );
              })
            ) : (
              <EmptyLine text="No local match yet." />
            )}
          </SearchGroup>

          {query.trim().length >= 2 && (
            <SearchGroup title="NetEase Cloud Music">
              {!neteaseEnabled ? (
                <SourceDisabledHint
                  label="网易云音乐源未启用 / NetEase source is off"
                  onOpenSettings={onOpenSettings}
                />
              ) : neteaseServiceStarting ? (
                <div className="flex items-center gap-2 px-2 py-3 text-xs font-semibold text-[#4a2108]/46">
                  <Loader2 className="h-3.5 w-3.5 animate-spin" />
                  正在启动网易云音乐源 / Starting NetEase music source...
                  <span className="ml-auto text-[10px] font-semibold text-[#4a2108]/32">
                    第一次启动可能需要几秒，本地音乐可以立即播放
                  </span>
                </div>
              ) : neteaseServiceError ? (
                <div className="flex flex-col gap-2 px-2 py-3">
                  <p className="text-xs font-semibold text-[#4a2108]/56">{neteaseServiceError}</p>
                  <button
                    type="button"
                    onMouseDown={(event) => event.preventDefault()}
                    onClick={() => {
                      setNeteaseServiceError(null);
                      setNeteaseRetryToken((value) => value + 1);
                    }}
                    className="app-transition inline-flex w-fit items-center gap-1.5 rounded-full bg-[#4a2108]/8 px-2.5 py-1 text-[11px] font-bold text-[#4a2108]/62 hover:bg-[#4a2108]/14 hover:text-[#4a2108]/82"
                  >
                    <RefreshCw className="h-3 w-3" />
                    重试 / Retry
                  </button>
                </div>
              ) : isSearchingSource ? (
                <div className="flex items-center gap-2 px-2 py-3 text-xs font-semibold text-[#4a2108]/38">
                  <Loader2 className="h-3.5 w-3.5 animate-spin" />
                  Searching NetEase Cloud Music...
                </div>
              ) : neteaseResults.length ? (
                <>
                  {neteaseResults.slice(0, neteaseVisible).map((song) => (
                    <button
                      key={song.id}
                      type="button"
                      onMouseDown={(event) => event.preventDefault()}
                      onClick={async () => {
                        // Only dismiss the search popover when playback
                        // actually started. On failure the import returns
                        // false (and the real reason is surfaced via the
                        // playback notice / library error), so we keep the
                        // results visible for the user to pick another track
                        // instead of silently swallowing the click.
                        setOpen(false);
                        const ok = await onPlayNetEase(song);
                        if (ok) {
                          setOpen(false);
                        } else {
                          setNeteaseMessage(
                            "Couldn't play this track — see the notice for the reason. / 无法播放，请查看提示。",
                          );
                        }
                      }}
                      className="search-result-row"
                    >
                      <ArtworkImage
                        src={song.coverUrl}
                        alt={song.album || song.title}
                        source="netease"
                        className="h-10 w-10 shrink-0 rounded-[10px] object-cover shadow-[0_10px_24px_rgba(74,33,8,0.18)]"
                      />
                      <ResultText title={song.title} subtitle={`${song.artist} · ${song.album}`} />
                      {song.unavailable && (
                        <span className="ml-auto text-xs font-semibold text-[#4a2108]/30">
                          unavailable
                        </span>
                      )}
                    </button>
                  ))}
                  {neteaseResults.length > neteaseVisible && (
                    <ShowMoreButton
                      onClick={() => setNeteaseVisible((v) => v + SOURCE_PAGE_SIZE)}
                    />
                  )}
                  {neteaseMessage && (
                    <p className="px-1 pt-1 text-xs font-semibold text-[#7a2d1c]/72">
                      {neteaseMessage}
                    </p>
                  )}
                </>
              ) : (
                <EmptyLine text={neteaseMessage ?? "No outside match yet."} />
              )}
            </SearchGroup>
          )}

          {query.trim().length >= 2 && (
            <SearchGroup title="Bilibili">
              {!bilibiliEnabled ? (
                <SourceDisabledHint
                  label="Bilibili 音乐源未启用 / Bilibili source is off"
                  onOpenSettings={onOpenSettings}
                />
              ) : isSearchingBilibili ? (
                <div className="flex items-center gap-2 px-2 py-3 text-xs font-semibold text-[#4a2108]/38">
                  <Loader2 className="h-3.5 w-3.5 animate-spin" />
                  Searching Bilibili...
                </div>
              ) : bilibiliResults.length ? (
                <>
                  {bilibiliResults.slice(0, bilibiliVisible).map((song) => (
                    <button
                      key={song.id}
                      type="button"
                      onMouseDown={(event) => event.preventDefault()}
                      onClick={async () => {
                        const requestId = bilibiliPlayRequestRef.current + 1;
                        bilibiliPlayRequestRef.current = requestId;
                        setPlayingBilibiliId(song.id);
                        setOpen(false);
                        const started = await onPlayBilibili(song);
                        if (bilibiliPlayRequestRef.current !== requestId) return;
                        setPlayingBilibiliId(null);
                        if (started) setOpen(false);
                      }}
                      className="search-result-row"
                    >
                      <ArtworkImage
                        src={song.coverUrl}
                        alt={song.title}
                        source="bilibili"
                        className="h-10 w-10 shrink-0 rounded-[10px] object-cover shadow-[0_10px_24px_rgba(74,33,8,0.18)]"
                      />
                      <ResultText
                        title={song.title}
                        subtitle={`${song.uploader ?? song.artist} · ${formatDuration(song.durationSeconds)}`}
                      />
                      {playingBilibiliId === song.id ? (
                        <Loader2 className="ml-auto h-3.5 w-3.5 shrink-0 animate-spin text-[#4a2108]/38" />
                      ) : typeof song.danmakuCount === "number" ? (
                        <span className="ml-auto text-xs font-semibold text-[#4a2108]/30">
                          {compactNumber(song.danmakuCount)} danmaku
                        </span>
                      ) : null}
                    </button>
                  ))}
                  {bilibiliResults.length > bilibiliVisible && (
                    <ShowMoreButton
                      onClick={() => setBilibiliVisible((v) => v + SOURCE_PAGE_SIZE)}
                    />
                  )}
                </>
              ) : (
                <EmptyLine text={bilibiliMessage ?? "No Bilibili match yet."} />
              )}
            </SearchGroup>
          )}

          {query.trim().length >= 2 && (
            <SearchGroup title="QQ音乐">
              {!qqmusicEnabled ? (
                <SourceDisabledHint
                  label="QQ音乐源未启用 / QQ Music source is off"
                  onOpenSettings={onOpenSettings}
                />
              ) : isSearchingQQMusic ? (
                <div className="flex items-center gap-2 px-2 py-3 text-xs font-semibold text-[#4a2108]/38">
                  <Loader2 className="h-3.5 w-3.5 animate-spin" />
                  Searching QQ音乐…
                </div>
              ) : qqmusicResults.length ? (
                <>
                  {qqmusicResults.slice(0, qqmusicVisible).map((song) => (
                    <button
                      key={song.id}
                      type="button"
                      onMouseDown={(event) => event.preventDefault()}
                      onClick={async () => {
                        const started = await onPlayQQMusic(song);
                        if (started) {
                          setOpen(false);
                        } else {
                          setQQMusicMessage(
                            "无法播放这首歌，请查看提示后重试 / Couldn't play this track. Check the notice and try again.",
                          );
                        }
                      }}
                      className="search-result-row"
                    >
                      {song.coverUrl ? (
                        <ArtworkImage
                          src={song.coverUrl}
                          alt={song.album}
                          source="qqmusic"
                          className="h-10 w-10 shrink-0 rounded-[10px] object-cover shadow-[0_10px_24px_rgba(74,33,8,0.18)]"
                        />
                      ) : (
                        <div className="flex h-10 w-10 items-center justify-center rounded-[10px] bg-[#4a2108]/10 text-[#4a2108]/38">
                          <Music2 className="h-4 w-4" />
                        </div>
                      )}
                      <ResultText title={song.title} subtitle={`${song.artist} · ${song.album}`} />
                      {song.unavailable && (
                        <span className="ml-auto text-xs font-semibold text-[#4a2108]/30">
                          unavailable
                        </span>
                      )}
                    </button>
                  ))}
                  {qqmusicResults.length > qqmusicVisible && (
                    <ShowMoreButton
                      onClick={() => setQQMusicVisible((v) => v + SOURCE_PAGE_SIZE)}
                    />
                  )}
                </>
              ) : (
                <EmptyLine text={qqmusicMessage ?? "No QQ Music match yet."} />
              )}
            </SearchGroup>
          )}
        </div>
      )}
    </div>
  );
}

function compactNumber(value: number): string {
  if (value >= 10000) return `${(value / 1000).toFixed(0)}k`;
  return `${value}`;
}

function readSourceError(error: unknown, source: "bilibili" | "qqmusic" = "bilibili"): string {
  const message = error instanceof Error ? error.message : String(error ?? "");
  if (message.includes("web verification") || message.includes("paused this search")) {
    return source === "bilibili"
      ? "Bilibili needs a brief pause. Try this search again in a moment."
      : "QQ音乐需要网页验证，请稍后再试 / QQ Music needs web verification, please try later.";
  }
  return source === "bilibili"
    ? "Bilibili 搜索失败，请稍后再试 / Bilibili search failed, please try again."
    : `QQ音乐搜索失败: ${message || "请稍后再试"} / QQ Music search failed.`;
}

function readNeteaseSearchError(error: unknown): string {
  const message = error instanceof Error ? error.message : String(error ?? "");
  if (
    message.includes("Could not reach") ||
    message.includes("Could not reach") ||
    message.includes("\u65e0\u6cd5\u8fde\u63a5") ||
    message.includes("ECONNREFUSED") ||
    message.includes("connect to") ||
    message.includes("Network Error")
  ) {
    return "NetEase Cloud Music is not ready yet. Try again shortly or reconnect the music source in Settings.";
  }
  return message || "NetEase search failed, please try again.";
}

function readNeteaseServiceError(error: unknown): string {
  const message = error instanceof Error ? error.message : String(error ?? "");
  if (!message) {
    return "网易云音乐源暂时无法启动 / Could not start the NetEase source.";
  }
  return message;
}

function SearchGroup({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div className="py-2">
      <p className="px-2 pb-2 text-[11px] font-black uppercase tracking-[0.18em] text-[#4a2108]/28">
        {title}
      </p>
      <div className="space-y-1">{children}</div>
    </div>
  );
}

function ResultText({ title, subtitle }: { title: string; subtitle: string }) {
  return (
    <span className="min-w-0 text-left">
      <span className="block truncate text-sm font-black text-[#4a2108]/78">{title}</span>
      <span className="block truncate text-xs font-semibold text-[#4a2108]/36">{subtitle}</span>
    </span>
  );
}

function EmptyLine({ text }: { text: string }) {
  return <p className="px-2 py-3 text-xs font-semibold text-[#4a2108]/34">{text}</p>;
}

function SourceDisabledHint({
  label,
  onOpenSettings,
}: {
  label: string;
  onOpenSettings?: () => void;
}) {
  return (
    <div className="flex items-center justify-between gap-3 px-2 py-3">
      <span className="text-xs font-semibold text-[#4a2108]/38">{label}</span>
      {onOpenSettings && (
        <button
          type="button"
          onMouseDown={(event) => event.preventDefault()}
          onClick={onOpenSettings}
          className="app-transition inline-flex items-center gap-1.5 rounded-full bg-[#4a2108]/8 px-2.5 py-1 text-[11px] font-bold text-[#4a2108]/62 hover:bg-[#4a2108]/14 hover:text-[#4a2108]/82"
        >
          <Settings className="h-3 w-3" />
          启用 / Enable
        </button>
      )}
    </div>
  );
}

function ShowMoreButton({ onClick }: { onClick: () => void }) {
  // A restrained "Show more": no total count noise, just a quiet way to keep browsing.
  return (
    <button
      type="button"
      onMouseDown={(event) => event.preventDefault()}
      onClick={onClick}
      className="app-transition mt-1 flex w-full items-center justify-center gap-1.5 rounded-full px-2 py-2 text-[11px] font-bold text-[#4a2108]/42 hover:bg-[#4a2108]/6 hover:text-[#4a2108]/68"
    >
      更多 / Show more
      <ChevronDown className="h-3 w-3" />
    </button>
  );
}
