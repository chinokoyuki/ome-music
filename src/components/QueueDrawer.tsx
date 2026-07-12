/* eslint-disable react-refresh/only-export-components -- preference helpers intentionally live with the queue drawer */
import { useEffect } from "react";
import type React from "react";
import { Heart, ListMusic, Sparkles, Trash2, X } from "lucide-react";
import clsx from "clsx";
import type { Track } from "../types/music";
import { resolveTrackCover } from "../features/artwork/resolveTrackCover";
import { formatDuration } from "../utils/format";
import { ArtworkImage } from "./ArtworkImage";

function sourceTagLabel(source: Track["source"]): string {
  switch (source) {
    case "netease":
      return "NetEase";
    case "bilibili":
      return "Bilibili";
    case "qqmusic":
      return "QQ音乐";
    case "local":
    default:
      return "Local";
  }
}

interface QueueDrawerProps {
  open: boolean;
  tracks: Track[];
  currentTrackId: string | null;
  isPlaying: boolean;
  qualityLabel: string;
  recommendSimilar: boolean;
  onClose: () => void;
  onPlay: (track: Track) => void;
  onRemove: (trackId: string) => void;
  onClear: () => void;
  onLikeAll: () => void;
  onToggleLike: (trackId: string) => void;
  onToggleRecommendSimilar: (value: boolean) => void;
}

export function QueueDrawer({
  open,
  tracks,
  currentTrackId,
  isPlaying,
  qualityLabel,
  recommendSimilar,
  onClose,
  onPlay,
  onRemove,
  onClear,
  onLikeAll,
  onToggleLike,
  onToggleRecommendSimilar,
}: QueueDrawerProps) {
  useEffect(() => {
    if (!open) return;
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [open, onClose]);

  const count = tracks.length;

  return (
    <div className="pointer-events-none fixed inset-0 z-40">
      <button
        type="button"
        aria-label="Close queue"
        tabIndex={open ? 0 : -1}
        onClick={onClose}
        className={clsx(
          "absolute inset-0 bg-[#120b08]/14 backdrop-blur-[1px] transition-opacity duration-300",
          open ? "pointer-events-auto opacity-100" : "opacity-0",
        )}
      />

      <aside
        role="dialog"
        aria-label="Queue / 播放队列"
        aria-hidden={!open}
        className={clsx(
          "pointer-events-auto absolute right-0 top-0 flex h-full w-[min(420px,92vw)] flex-col border-l border-white/12 bg-[#231914]/82 text-[#efe4d8] shadow-[0_30px_86px_rgba(38,18,10,0.34)] backdrop-blur-xl transition-transform duration-300 ease-out",
          open ? "translate-x-0" : "translate-x-full",
        )}
      >
        <div className="flex items-center justify-between px-5 pb-3 pt-5">
          <div className="flex items-center gap-2.5">
            <ListMusic className="h-[18px] w-[18px] text-[#efe4d8]/55" />
            <h2 className="text-sm font-bold tracking-wide">Queue</h2>
            <span className="rounded-full bg-white/[0.08] px-2 py-0.5 text-[10px] font-bold tabular-nums text-[#efe4d8]/60">
              {count}
            </span>
          </div>
          <div className="flex items-center gap-1.5">
            <button
              type="button"
              onClick={onLikeAll}
              disabled={count === 0}
              className="app-transition flex h-8 items-center gap-1.5 rounded-full bg-white/[0.06] px-3 text-[11px] font-semibold text-[#efe4d8]/75 hover:bg-white/[0.12] hover:text-[#efe4d8] disabled:cursor-not-allowed disabled:opacity-40"
              title="Like all / 全部喜欢"
            >
              <Heart className="h-[13px] w-[13px]" />
              Like all
            </button>
            <button
              type="button"
              onClick={onClear}
              disabled={count === 0}
              className="app-transition flex h-8 items-center gap-1.5 rounded-full bg-white/[0.04] px-3 text-[11px] font-semibold text-[#efe4d8]/55 hover:bg-[#7a2d1c]/40 hover:text-[#efe4d8] disabled:cursor-not-allowed disabled:opacity-30"
              title="Stop and reset playback. Your library stays safe. / 停止并清空队列，曲库保留"
            >
              <Trash2 className="h-[13px] w-[13px]" />
              Clear
            </button>
            <button
              type="button"
              onClick={onClose}
              className="app-transition flex h-8 w-8 items-center justify-center rounded-full text-[#efe4d8]/55 hover:bg-white/[0.08] hover:text-[#efe4d8]"
              aria-label="Close"
            >
              <X className="h-[16px] w-[16px]" />
            </button>
          </div>
        </div>
        <p className="px-5 pb-3 text-[11px] font-medium text-[#efe4d8]/40">播放列表 / 播放队列</p>

        <div className="settings-scroll min-h-0 flex-1 overflow-y-auto px-2.5 pb-4">
          {count === 0 ? (
            <div className="flex h-40 flex-col items-center justify-center px-6 text-center">
              <p className="text-sm font-semibold text-[#efe4d8]/45">The queue is quiet.</p>
              <p className="mt-1.5 text-xs text-[#efe4d8]/30">队列是空的，挑一首歌放进来吧。</p>
            </div>
          ) : (
            <ul className="space-y-0.5">
              {tracks.map((track) => {
                const isCurrent = track.id === currentTrackId;
                const unavailable = track.filePath.startsWith("unavailable:");
                const canRetrySource =
                  unavailable &&
                  (Boolean(track.sourceId) ||
                    track.filePath.startsWith("unavailable:netease:") ||
                    track.filePath.startsWith("unavailable:bilibili:"));
                const artwork = resolveTrackCover(track);
                const handleRowKeyDown = (event: React.KeyboardEvent<HTMLDivElement>) => {
                  if (event.key === "Enter" || event.key === " ") {
                    event.preventDefault();
                    onPlay(track);
                  }
                };

                return (
                  <li key={track.id}>
                    <div
                      role="button"
                      tabIndex={0}
                      onClick={() => onPlay(track)}
                      onKeyDown={handleRowKeyDown}
                      className={clsx(
                        "group relative flex cursor-pointer items-center gap-3 rounded-[14px] px-2.5 py-2 outline-none transition-colors duration-200 focus-visible:bg-white/[0.07]",
                        isCurrent ? "bg-white/[0.08]" : "hover:bg-white/[0.04]",
                      )}
                    >
                      <div
                        className="relative h-11 w-11 shrink-0 overflow-hidden rounded-[10px]"
                        title={
                          unavailable
                            ? canRetrySource
                              ? "Retry this source"
                              : "Unavailable"
                            : `Play ${track.title}`
                        }
                      >
                        <ArtworkImage
                          src={artwork.src}
                          alt={track.album || track.title}
                          source={track.source}
                          className="h-full w-full object-cover"
                        />
                        {isCurrent && isPlaying && (
                          <span className="absolute inset-0 flex items-center justify-center bg-black/30">
                            <span className="flex items-end gap-[2px]">
                              <span className="h-2 w-[2px] animate-[pulse_1.1s_ease-in-out_infinite] rounded-full bg-white/85" />
                              <span className="h-3 w-[2px] animate-[pulse_1.1s_ease-in-out_infinite_0.2s] rounded-full bg-white/85" />
                              <span className="h-1.5 w-[2px] animate-[pulse_1.1s_ease-in-out_infinite_0.4s] rounded-full bg-white/85" />
                            </span>
                          </span>
                        )}
                      </div>

                      <div className="min-w-0 flex-1 text-left">
                        <p
                          className={clsx(
                            "truncate text-[13px] font-semibold leading-tight",
                            isCurrent ? "text-[#efe4d8]" : "text-[#efe4d8]/82",
                          )}
                        >
                          {track.title}
                        </p>
                        <p className="mt-0.5 truncate text-[11px] text-[#efe4d8]/45">
                          {track.artist}
                        </p>
                        <div className="mt-1 flex items-center gap-1.5">
                          <span className="rounded-full bg-white/[0.06] px-1.5 py-px text-[9px] font-bold uppercase tracking-wide text-[#efe4d8]/55">
                            {sourceTagLabel(track.source)}
                          </span>
                          {isCurrent && qualityLabel && (
                            <span className="rounded-full bg-[#7a2d1c]/45 px-1.5 py-px text-[9px] font-bold uppercase tracking-wide text-[#efe4d8]/85">
                              {qualityLabel}
                            </span>
                          )}
                          {unavailable && (
                            <span className="text-[9px] font-bold uppercase tracking-wide text-[#e8a08f]/75">
                              {canRetrySource ? "Retry source" : "Unavailable"}
                            </span>
                          )}
                        </div>
                      </div>

                      <span className="hidden shrink-0 text-[11px] font-semibold tabular-nums text-[#efe4d8]/35 sm:block">
                        {formatDuration(track.durationSeconds)}
                      </span>

                      <button
                        type="button"
                        onClick={(event) => {
                          event.stopPropagation();
                          onToggleLike(track.id);
                        }}
                        className={clsx(
                          "app-transition flex h-8 w-8 shrink-0 items-center justify-center rounded-full",
                          track.liked
                            ? "text-[#e8a08f]"
                            : "text-[#efe4d8]/30 opacity-0 hover:bg-white/[0.08] hover:text-[#efe4d8]/70 group-hover:opacity-100",
                        )}
                        aria-label={track.liked ? "Unlike" : "Like"}
                        aria-pressed={track.liked}
                        title={track.liked ? "Unlike / 取消喜欢" : "Like / 喜欢"}
                      >
                        <Heart
                          className={clsx("h-[14px] w-[14px]", track.liked && "fill-current")}
                        />
                      </button>

                      <button
                        type="button"
                        onClick={(event) => {
                          event.stopPropagation();
                          onRemove(track.id);
                        }}
                        className="app-transition flex h-8 w-8 shrink-0 items-center justify-center rounded-full text-[#efe4d8]/30 opacity-0 hover:bg-[#7a2d1c]/40 hover:text-[#efe4d8] group-hover:opacity-100"
                        aria-label="Remove from queue"
                        title="Skip this track. Library kept. / 跳过此曲，曲库保留"
                      >
                        <X className="h-[14px] w-[14px]" />
                      </button>
                    </div>
                  </li>
                );
              })}
            </ul>
          )}
        </div>

        <div className="border-t border-white/[0.06] px-5 py-3.5">
          <button
            type="button"
            onClick={() => onToggleRecommendSimilar(!recommendSimilar)}
            className="flex w-full items-center justify-between"
            aria-pressed={recommendSimilar}
          >
            <span className="flex items-center gap-2 text-[11px] font-semibold text-[#efe4d8]/55">
              <Sparkles className="h-[13px] w-[13px]" />
              Recommend similar
            </span>
            <span
              className={clsx(
                "relative h-[18px] w-8 rounded-full transition-colors duration-200",
                recommendSimilar ? "bg-[#7a2d1c]/70" : "bg-white/[0.12]",
              )}
            >
              <span
                className={clsx(
                  "absolute top-[2px] h-[14px] w-[14px] rounded-full bg-white/90 transition-transform duration-200",
                  recommendSimilar ? "translate-x-[16px]" : "translate-x-[2px]",
                )}
              />
            </span>
          </button>
          <p className="mt-1.5 text-[10px] text-[#efe4d8]/30">在队列末尾延续相近的听感。</p>
        </div>
      </aside>
    </div>
  );
}

export const RECOMMEND_SIMILAR_KEY = "ome.queue.recommendSimilar";

export function loadRecommendSimilar(): boolean {
  try {
    return window.localStorage.getItem(RECOMMEND_SIMILAR_KEY) === "1";
  } catch {
    return false;
  }
}

export function saveRecommendSimilar(value: boolean): void {
  try {
    window.localStorage.setItem(RECOMMEND_SIMILAR_KEY, value ? "1" : "0");
  } catch {
    // Non-critical preference.
  }
}
