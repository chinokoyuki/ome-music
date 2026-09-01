import { memo, useEffect, useMemo, useRef, useState } from "react";
import {
  FolderOpen,
  Heart,
  Loader2,
  MoreHorizontal,
  Share,
  Sparkles,
  ThumbsDown,
} from "lucide-react";
import clsx from "clsx";
import type { Track } from "../types/music";
import type { LyricLine } from "../features/lyrics/lyricsResolver";
import { resolveTrackCover } from "../features/artwork/resolveTrackCover";
import type { BilibiliDanmakuDebug, DanmakuItem } from "../features/musicSources/provider";
import { ArtworkImage } from "./ArtworkImage";
import { DanmakuAtmosphereLayer } from "./DanmakuAtmosphereLayer";
import { PLAYBACK_SPEEDS, type PlaybackSpeed } from "./PlayerControls";

interface NowPlayingHeroProps {
  track: Track | null;
  lyrics: LyricLine[];
  translatedLyrics: LyricLine[];
  currentLyricIndex: number;
  lyricWarning?: string | null;
  isPlaying: boolean;
  isLyricsLoading: boolean;
  videoAtmosphereSrc: string;
  progressSeconds: number;
  danmakuItems: DanmakuItem[];
  danmakuDebug?: BilibiliDanmakuDebug | null;
  isImporting: boolean;
  error: string | null;
  onImport: () => void;
  liked: boolean;
  onToggleLike: () => void;
  onLessLikeThis: () => void;
  onShare: () => string;
  // Current playback speed (drives the active checkmark in the More menu's
  // speed list) and a setter that applies an explicit speed directly — no
  // more blind cycling.
  playbackSpeed: PlaybackSpeed;
  onSelectSpeed: (speed: PlaybackSpeed) => void;
  // Click a lyric line to seek the audio to that line's start time. The
  // Lyrics Room is a stage, not a flat list — clicking a line should feel
  // like dropping the needle at that moment, then the room recenters.
  onSeekToLyric: (seconds: number) => void;
  moreOpen: boolean;
  onOpenMore: () => void;
  onCloseMore: () => void;
  // Any primary overlay (queue / settings / quick settings / search) being
  // open must retire the title dialog too, so translucent layers never stack.
  overlayOpen: boolean;
}

export function NowPlayingHero({
  track,
  lyrics,
  translatedLyrics,
  currentLyricIndex,
  lyricWarning,
  isPlaying,
  isLyricsLoading,
  videoAtmosphereSrc,
  progressSeconds,
  danmakuItems,
  danmakuDebug,
  isImporting,
  error,
  onImport,
  liked,
  onToggleLike,
  onLessLikeThis,
  onShare,
  playbackSpeed,
  onSelectSpeed,
  onSeekToLyric,
  moreOpen,
  onOpenMore,
  onCloseMore,
  overlayOpen,
}: NowPlayingHeroProps) {
  const [isTitleExpanded, setTitleExpanded] = useState(false);
  const [likePulse, setLikePulse] = useState(false);
  const [moreToast, setMoreToast] = useState<string | null>(null);
  const moreMenuRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    setTitleExpanded(false);
    onCloseMore();
  }, [onCloseMore, track?.id]);

  // A primary overlay opening elsewhere (queue / settings / quick settings /
  // search) retires the title dialog so translucent layers never stack.
  useEffect(() => {
    if (overlayOpen) setTitleExpanded(false);
  }, [overlayOpen]);

  useEffect(() => {
    if (!isTitleExpanded) return;
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape") setTitleExpanded(false);
    };
    window.addEventListener("keydown", closeOnEscape);
    return () => window.removeEventListener("keydown", closeOnEscape);
  }, [isTitleExpanded]);

  // Close the "more" menu on outside pointer / Esc — mirrors the Quick
  // Settings panel behavior so the two popovers feel consistent.
  useEffect(() => {
    if (!moreOpen) return;
    const handlePointerDown = (event: PointerEvent) => {
      if (!moreMenuRef.current?.contains(event.target as Node)) onCloseMore();
    };
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") onCloseMore();
    };
    window.addEventListener("pointerdown", handlePointerDown);
    window.addEventListener("keydown", handleKeyDown);
    return () => {
      window.removeEventListener("pointerdown", handlePointerDown);
      window.removeEventListener("keydown", handleKeyDown);
    };
  }, [moreOpen, onCloseMore]);

  // Auto-dismiss the "more" toast after a short delay — these are light
  // confirmations (share copied, less-like-this recorded), not modals.
  useEffect(() => {
    if (!moreToast) return;
    const timer = window.setTimeout(() => setMoreToast(null), 2400);
    return () => window.clearTimeout(timer);
  }, [moreToast]);

  const handleLike = () => {
    if (!liked) {
      // Brief pulse animation when liking — the heart grows then settles.
      setLikePulse(true);
      window.setTimeout(() => setLikePulse(false), 480);
    }
    onToggleLike();
  };

  const handleLessLikeThis = () => {
    onCloseMore();
    onLessLikeThis();
    setMoreToast("Got it — fewer like this.");
  };

  const handleShare = () => {
    onCloseMore();
    const message = onShare();
    setMoreToast(message);
  };

  if (!track) {
    return (
      <section className="relative flex min-h-[calc(100svh-7rem)] items-center justify-center px-8">
        <div className="max-w-xl text-center">
          <p className="mb-5 text-sm font-medium text-[#4a2108]/[0.38]">OME</p>
          <h1 className="text-5xl font-black leading-tight text-[#4a2108]/[0.82] sm:text-7xl">
            Let the record wake first
          </h1>
          <p className="mx-auto mt-6 max-w-md text-base leading-8 text-[#4a2108]/[0.42]">
            Choose a music folder, then let the room fill with cover light, sound, and words.
          </p>
          {error && <p className="mt-5 text-sm text-[#7a2d1c]/70">{error}</p>}
          <button
            type="button"
            onClick={onImport}
            disabled={isImporting}
            className="app-transition mt-10 inline-flex h-12 items-center justify-center gap-2 rounded-full bg-[#4a2108]/[0.86] px-6 text-sm font-semibold text-[#efe4d8] shadow-[0_18px_46px_rgba(74,33,8,0.22)] hover:scale-[1.015] hover:bg-[#4a2108] disabled:cursor-wait disabled:opacity-70"
          >
            {isImporting ? (
              <Loader2 className="h-4 w-4 animate-spin" />
            ) : (
              <FolderOpen className="h-4 w-4" />
            )}
            {isImporting ? "Waking the records" : "Choose Music Folder"}
          </button>
        </div>
      </section>
    );
  }

  const isBilibiliStage = track.source === "bilibili" && !isLyricsLoading && lyrics.length === 0;
  const artwork = resolveTrackCover(track);

  return (
    <section className="now-playing-stage relative mx-auto grid w-full max-w-[1780px] items-center overflow-visible">
      <div
        data-danmaku-safe-zone="left-visual"
        className="left-visual-stack relative z-10 flex min-w-0 flex-col items-center md:items-start"
      >
        <div className="record-sleeve relative">
          <ArtworkImage
            src={artwork.src}
            alt={track.album || track.title}
            source={track.source}
            className="h-full w-full rounded-[18px] object-cover"
          />
          {/* Like button — bottom-right of the cover. Doubles as a Taste
              Signal: tapping it records a liked/unliked event that feeds
              the listening-memory / radio scoring pipeline. Filled heart
              when liked, soft outline otherwise. */}
          <button
            type="button"
            onClick={handleLike}
            className={`app-transition absolute bottom-3 right-3 flex h-11 w-11 items-center justify-center rounded-full backdrop-blur-xl ${
              liked
                ? "bg-[#7a2d1c]/85 text-white shadow-[0_8px_24px_rgba(122,45,28,0.42)]"
                : "bg-white/55 text-[#4a2108]/55 hover:bg-white/75 hover:text-[#7a2d1c]"
            } ${likePulse ? "scale-[1.18]" : "scale-100"}`}
            aria-label={liked ? "Unlike" : "Like"}
            aria-pressed={liked}
            title={liked ? "Unlike / 取消喜欢" : "Like / 喜欢"}
          >
            <Heart className={`h-[18px] w-[18px] ${liked ? "fill-current" : ""}`} />
          </button>
        </div>

        <div className="mt-5 w-[min(70vw,390px)] text-center md:w-[min(29vw,420px)] md:text-left">
          <div className="flex items-start gap-2">
            <div
              role="button"
              tabIndex={0}
              onClick={() => setTitleExpanded((value) => !value)}
              onKeyDown={(event) => {
                if (event.key === "Enter" || event.key === " ") {
                  event.preventDefault();
                  setTitleExpanded((value) => !value);
                }
              }}
              className="block min-w-0 flex-1 cursor-pointer text-left"
              aria-expanded={isTitleExpanded}
            >
              <h1
                title={track.title}
                className="line-clamp-2 max-h-[2.12em] text-[clamp(1.35rem,2vw,2rem)] font-black leading-[1.06] text-[#4a2108]/88 transition-colors duration-300 hover:text-[#4a2108]"
              >
                {track.title}
              </h1>
            </div>
            {/* More button — hidden secondary actions. Opens a light glass
                popover with only the actions that actually work today:
                Less like this / Playback speed (explicit list) / Share.
                Half-finished entries (Add to playlist, View source) are
                intentionally NOT shown — an immersive player shouldn't
                parade disabled stubs. Kept small so it never competes with
                the cover or lyrics. */}
            <div ref={moreMenuRef} className="relative shrink-0">
              <button
                type="button"
                onClick={() => (moreOpen ? onCloseMore() : onOpenMore())}
                className={`app-transition flex h-9 w-9 items-center justify-center rounded-full text-[#4a2108]/40 hover:bg-[#4a2108]/[0.05] hover:text-[#4a2108]/70 ${
                  moreOpen ? "bg-[#4a2108]/[0.06] text-[#4a2108]/70" : ""
                }`}
                aria-label="More options"
                aria-expanded={moreOpen}
                title="More / 更多"
              >
                <MoreHorizontal className="h-[18px] w-[18px]" />
              </button>
              {moreOpen && (
                <div className="quick-settings-panel settings-scroll absolute bottom-11 right-0 z-50 max-h-[min(52vh,330px)] w-64 overflow-y-auto rounded-[20px] p-2">
                  <MoreMenuItem
                    icon={ThumbsDown}
                    label="Less like this"
                    sublabel="减少推荐"
                    onClick={handleLessLikeThis}
                  />
                  {/* Playback speed — explicit list, not blind cycling. The
                      current speed shows a checkmark; picking one applies it
                      immediately and closes the menu. */}
                  <div className="px-3 pb-1 pt-2">
                    <div className="flex items-center gap-2 text-[#4a2108]/50">
                      <Sparkles className="h-[15px] w-[15px]" />
                      <span className="text-[13px] font-bold text-[#4a2108]/82">
                        Playback speed
                      </span>
                      <span className="text-[11px] font-semibold text-[#4a2108]/38">播放速度</span>
                    </div>
                  </div>
                  <div className="grid grid-cols-3 gap-1 px-2 pb-1.5">
                    {PLAYBACK_SPEEDS.map((speed) => {
                      const active = speed === playbackSpeed;
                      const label = speed === 1 ? "1x" : `${speed}x`;
                      return (
                        <button
                          key={speed}
                          type="button"
                          onClick={() => {
                            onSelectSpeed(speed);
                            onCloseMore();
                          }}
                          aria-pressed={active}
                          className={`app-transition flex h-8 items-center justify-center rounded-full text-[12px] font-bold tabular-nums ${
                            active
                              ? "bg-[#4a2108]/85 text-white"
                              : "bg-[#4a2108]/[0.05] text-[#4a2108]/62 hover:bg-[#4a2108]/[0.1] hover:text-[#4a2108]/85"
                          }`}
                        >
                          {label}
                        </button>
                      );
                    })}
                  </div>
                  <MoreMenuItem icon={Share} label="Share" sublabel="分享" onClick={handleShare} />
                </div>
              )}
            </div>
          </div>
          <div className="mt-4 space-y-1.5 text-[13px] font-bold leading-5 text-[#4a2108]/[0.26]">
            <p>{track.artist}</p>
            <p>{track.album}</p>
          </div>
          {moreToast && (
            <p className="app-transition mt-3 rounded-full bg-[#4a2108]/[0.06] px-3 py-1.5 text-xs font-semibold text-[#4a2108]/60">
              {moreToast}
            </p>
          )}
        </div>
      </div>

      <div
        data-danmaku-safe-zone={isBilibiliStage ? undefined : "lyrics"}
        className={clsx(
          "right-atmosphere-column relative z-10 min-h-0 min-w-0",
          isBilibiliStage
            ? "flex w-full items-center overflow-hidden"
            : "lyric-stage h-full min-h-[58vh] overflow-visible py-10",
        )}
      >
        {!isBilibiliStage && (
          <div className="pointer-events-none absolute inset-x-0 top-0 h-28 bg-gradient-to-b from-[#d0c6ba]/38 to-transparent" />
        )}
        {!isBilibiliStage && (
          <div className="pointer-events-none absolute inset-x-0 bottom-0 h-28 bg-gradient-to-t from-[#d0c6ba]/38 to-transparent" />
        )}

        {isLyricsLoading ? (
          <div className="flex h-[58vh] items-center text-3xl font-black text-[#4a2108]/25">
            <Loader2 className="mr-3 h-6 w-6 animate-spin" />
            Finding the words
          </div>
        ) : lyrics.length > 0 ? (
          <LyricsRoom
            lyrics={lyrics}
            translatedLyrics={translatedLyrics}
            currentLyricIndex={currentLyricIndex}
            isPlaying={isPlaying}
            trackId={track.id}
            onSeekToLyric={onSeekToLyric}
          />
        ) : track.source === "bilibili" ? (
          <BilibiliVideoAtmosphere
            track={track}
            src={videoAtmosphereSrc}
            isPlaying={isPlaying}
            progressSeconds={progressSeconds}
            danmakuItems={danmakuItems}
            danmakuDebug={danmakuDebug}
          />
        ) : (
          <div className="flex h-[58vh] flex-col justify-center">
            <p className="max-w-[920px] text-balance text-5xl font-black leading-tight text-[#4a2108]/70 xl:text-7xl">
              No matched lyrics for this version.
            </p>
            <p className="mt-8 max-w-[720px] text-balance text-2xl font-black leading-tight text-[#4a2108]/25 xl:text-4xl">
              Let the room breathe for a moment.
            </p>
            {lyricWarning && (
              <p className="mt-6 max-w-md text-sm font-semibold text-[#4a2108]/35">
                {lyricWarning}
              </p>
            )}
          </div>
        )}
      </div>

      <div
        className={clsx(
          "fixed inset-0 z-30 bg-[#31180b]/[0.06] backdrop-blur-[1.5px] transition-opacity duration-300",
          isTitleExpanded ? "pointer-events-auto opacity-100" : "pointer-events-none opacity-0",
        )}
        onClick={() => setTitleExpanded(false)}
        aria-hidden={!isTitleExpanded}
      >
        <div
          data-danmaku-safe-zone="title-dialog"
          className={clsx(
            "title-reveal-panel fixed rounded-[20px] border border-white/18 bg-[#e4d2c4]/60 px-5 py-4 shadow-[0_22px_64px_rgba(74,33,8,0.18)] backdrop-blur-[18px] transition-[opacity,transform] duration-300",
            isTitleExpanded
              ? "translate-y-0 scale-100 opacity-100"
              : "translate-y-5 scale-[0.97] opacity-0",
          )}
          onClick={(event) => event.stopPropagation()}
        >
          <p className="text-[10px] font-black uppercase tracking-[0.2em] text-[#4a2108]/34">
            Now Playing
          </p>
          <p className="mt-2.5 text-balance text-[clamp(1.2rem,1.7vw,1.65rem)] font-extrabold leading-[1.1] text-[#4a2108]/88">
            {track.title}
          </p>
          <p className="mt-4 text-sm font-semibold text-[#4a2108]/42">
            {track.artist}
            {track.source === "bilibili" ? " · Bilibili" : ""} · {formatTime(track.durationSeconds)}
          </p>
        </div>
      </div>
    </section>
  );
}

// Lyrics Room — the right-side lyric stage. Not a flat scrolling list:
// the current line is the spatial center (sharp, large, ~full opacity),
// nearby lines drift and blur like close echoes, and far lines dissolve
// into the air with a gentle rotate so they read as atmosphere. Click any
// line to drop the needle at that moment; the auto-scroll effect below
// then recenters the stage on the new current line.
//
// Depth tiers (per spec):
//   current  — opacity ~0.96, no blur, scale 1, no rotate
//   nearby   — opacity ~0.42, blur 3px, scale 0.9, subtle fan drift
//   far      — opacity fading to ~0.16, blur 7px, scale 0.8, gentle rotate
// Motion is slow (duration-700) and eased so line changes feel like
// breathing, never jittery. No 3D transforms — just opacity / blur /
// scale / a hair of rotate, which is enough to suggest depth.
const LYRIC_ROOM_PRESETS = {
  calm: {
    curveX: 26,
    verticalGap: 58,
    rotate: 2.4,
    depth: 44,
    blur: 1.8,
    minScale: 0.76,
    minOpacity: 0.12,
  },
  arcRoom: {
    curveX: 46,
    verticalGap: 74,
    rotate: 4.2,
    depth: 62,
    blur: 2.4,
    minScale: 0.68,
    minOpacity: 0.08,
  },
  dream: {
    curveX: 54,
    verticalGap: 78,
    rotate: 5.1,
    depth: 74,
    blur: 3.1,
    minScale: 0.66,
    minOpacity: 0.07,
  },
  stage: {
    curveX: 38,
    verticalGap: 70,
    rotate: 3.4,
    depth: 84,
    blur: 2.1,
    minScale: 0.7,
    minOpacity: 0.1,
  },
} as const;

const DEFAULT_LYRIC_ROOM_PRESET = LYRIC_ROOM_PRESETS.arcRoom;

const LyricsRoom = memo(function LyricsRoom({
  lyrics,
  translatedLyrics,
  currentLyricIndex,
  isPlaying,
  trackId,
  onSeekToLyric,
}: {
  lyrics: LyricLine[];
  translatedLyrics: LyricLine[];
  currentLyricIndex: number;
  isPlaying: boolean;
  trackId: string;
  onSeekToLyric: (seconds: number) => void;
}) {
  const scrollRef = useRef<HTMLDivElement | null>(null);
  const preset = DEFAULT_LYRIC_ROOM_PRESET;
  // Hidden Lyric Tools — translation toggle only for now (romanization /
  // word-by-word are present but disabled: their data isn't parsed yet).
  // Persisted so the user's reveal preference survives across tracks.
  const [translationVisible, setTranslationVisible] = useState<boolean>(
    () => window.localStorage.getItem("ome.lyrics.translationVisible") === "1",
  );

  const toggleTranslation = () => {
    setTranslationVisible((prev) => {
      const next = !prev;
      window.localStorage.setItem("ome.lyrics.translationVisible", next ? "1" : "0");
      return next;
    });
  };

  // Recenter the stage on the current line — smooth while playing, instant
  // when paused or right after a track switch so the room snaps to attention
  // without a long slide.
  useEffect(() => {
    const container = scrollRef.current;
    const activeLine = container?.querySelector<HTMLElement>(
      `[data-lyric-index="${currentLyricIndex}"]`,
    );
    if (!container || !activeLine) return;
    const targetTop =
      activeLine.offsetTop - container.clientHeight / 2 + activeLine.clientHeight / 2;
    container.scrollTo({ top: Math.max(0, targetTop), behavior: isPlaying ? "smooth" : "auto" });
  }, [currentLyricIndex, isPlaying, trackId]);

  // Match the current line's translation. Translated LRC usually shares the
  // main LRC's timestamp structure, so an index match is the honest default;
  // if the translated track has fewer lines, we fall back to a time search so
  // we still surface the right line rather than nothing.
  const currentTranslatedText = (() => {
    if (!translationVisible || translatedLyrics.length === 0) return null;
    const byIndex = translatedLyrics[currentLyricIndex];
    if (byIndex?.text) return byIndex.text;
    const current = lyrics[currentLyricIndex];
    if (!current) return null;
    let closest: LyricLine | null = null;
    let bestDelta = Infinity;
    for (const line of translatedLyrics) {
      const delta = Math.abs(line.startTime - current.startTime);
      if (delta < bestDelta) {
        bestDelta = delta;
        closest = line;
      }
    }
    return closest?.text ?? null;
  })();

  // Window size around the active lyric line. Only this slice is rendered as
  // DOM; out-of-window lines become invisible spacer rows that keep the scroll
  // extent stable. This bounds DOM nodes + composited layers for very long
  // lyrics (300+ lines) while the visual arc stage stays identical.
  const LYRIC_WINDOW_RADIUS = 12;
  const LYRIC_ROW_HEIGHT_PX = 136; // mirrors .lyric-room-line min-height (8.5rem)

  const visualWindow = useMemo(() => {
    if (lyrics.length === 0) {
      return { start: 0, end: 0, beforeLines: 0, afterLines: 0, lines: [] as LyricLine[] };
    }
    const start = Math.max(0, currentLyricIndex - LYRIC_WINDOW_RADIUS);
    const end = Math.min(lyrics.length, currentLyricIndex + LYRIC_WINDOW_RADIUS + 1);
    return {
      start,
      end,
      beforeLines: start,
      afterLines: lyrics.length - end,
      lines: lyrics.slice(start, end),
    };
  }, [currentLyricIndex, lyrics]);

  const visualLines = useMemo(
    () =>
      visualWindow.lines.map((line, offsetInWindow) => {
        const index = visualWindow.start + offsetInWindow;
        const offset = index - currentLyricIndex;
        const distance = Math.abs(offset);
        const signed = offset === 0 ? 0 : Math.sign(offset);
        const cappedDistance = Math.min(distance, 6);
        const arcDirection = signed * (1 - Math.min(distance, 5) * 0.05);
        const curveX = signed * Math.min(220, Math.pow(cappedDistance, 1.24) * preset.curveX);
        const curveY = offset * preset.verticalGap;
        const zDepth = -Math.min(360, Math.pow(cappedDistance, 1.15) * preset.depth);
        const rotateZ = signed * Math.min(18, Math.pow(cappedDistance, 0.96) * preset.rotate);
        const rotateX = signed * Math.min(10, cappedDistance * 1.25);
        const scale = Math.max(preset.minScale, 1 - cappedDistance * 0.075);
        const opacity =
          distance === 0
            ? 1
            : Math.max(preset.minOpacity, 0.66 - Math.pow(cappedDistance, 1.04) * 0.12);
        const blur =
          distance === 0 ? 0 : Math.min(14, Math.pow(cappedDistance, 1.08) * preset.blur);

        return {
          line,
          index,
          isCurrent: distance === 0,
          opacity,
          blur,
          transform: `translate3d(${curveX}px, ${curveY}px, ${zDepth}px) rotateX(${rotateX}deg) rotateZ(${-rotateZ * arcDirection}deg) scale(${scale})`,
        };
      }),
    [currentLyricIndex, preset, visualWindow],
  );

  return (
    <>
      <div
        ref={scrollRef}
        className="lyrics-scroll h-[58vh] touch-pan-y overflow-y-auto overflow-x-visible overscroll-contain px-2 py-[24vh] pr-8"
        onWheel={(event) => event.stopPropagation()}
      >
        {visualWindow.beforeLines > 0 && (
          <div
            aria-hidden="true"
            style={{ height: visualWindow.beforeLines * LYRIC_ROW_HEIGHT_PX }}
            className="w-full shrink-0"
          />
        )}
        {visualLines.map(({ line, index, isCurrent, opacity, blur, transform }) => {
          return (
            <button
              type="button"
              key={line.id}
              data-lyric-index={index}
              onClick={() => onSeekToLyric(line.startTime)}
              style={{
                transform,
                opacity,
                filter: blur > 0 ? `blur(${blur}px)` : undefined,
              }}
              className={clsx(
                "lyric-room-line app-transition m-0 flex min-h-[8.5rem] w-full origin-center cursor-pointer items-center overflow-visible border-0 bg-transparent px-1 py-4 text-left text-balance text-4xl font-black leading-[1.14] duration-700 ease-out lg:text-5xl xl:text-7xl",
                isCurrent ? "text-[#4a2108]" : "text-[#4a2108] hover:text-[#4a2108]/70",
              )}
              title={`Jump to ${formatTime(line.startTime)}`}
              aria-current={isCurrent ? "true" : undefined}
            >
              <span className="lyric-room-line-inner flex flex-col overflow-visible">
                <span>{line.text}</span>
                {isCurrent && currentTranslatedText && (
                  <span className="mt-3 text-lg font-semibold leading-snug text-[#4a2108]/55 lg:text-xl xl:text-2xl">
                    {currentTranslatedText}
                  </span>
                )}
              </span>
            </button>
          );
        })}
        {visualWindow.afterLines > 0 && (
          <div
            aria-hidden="true"
            style={{ height: visualWindow.afterLines * LYRIC_ROW_HEIGHT_PX }}
            className="w-full shrink-0"
          />
        )}
      </div>

      {/* Hidden Lyric Tools — sit at the bottom-right corner of the stage.
          Default opacity 0; only reveal when the pointer enters this small
          hotspot. They never compete with the lyric stage on first glance.
          Translation is live; Romanization / Word-by-word render disabled
          until their data pipelines exist. */}
      <div className="group/tool pointer-events-auto absolute bottom-5 right-5 z-20 flex flex-col items-end gap-1.5 p-2">
        <div className="flex items-center gap-1 rounded-full border border-[#4a2108]/10 bg-white/45 px-1 py-1 opacity-0 shadow-[0_8px_24px_rgba(74,33,8,0.08)] backdrop-blur-md transition-all duration-300 group-hover/tool:opacity-100">
          <HiddenToolButton
            label="译"
            active={translationVisible}
            onClick={toggleTranslation}
            title="Translation / 翻译"
          />
          <HiddenToolButton label="音" disabled title="Romanization / 罗马音 (暂不可用)" />
          <HiddenToolButton label="逐字" disabled title="Word-by-word / 逐字 (暂不可用)" />
        </div>
        {translationVisible && !currentTranslatedText && (
          <p className="rounded-full bg-[#4a2108]/[0.05] px-2.5 py-1 text-[10px] font-semibold text-[#4a2108]/45 opacity-0 transition-opacity duration-300 group-hover/tool:opacity-100">
            暂无翻译 · No translation for this version
          </p>
        )}
      </div>
    </>
  );
});

// A single hidden-tool pill. Compact, calm — just a one-character label so
// the cluster reads as a whisper, not a toolbar. Active state is a soft fill;
// disabled is greyed and non-interactive (its data isn't wired yet).
function HiddenToolButton({
  label,
  active,
  disabled,
  onClick,
  title,
}: {
  label: string;
  active?: boolean;
  disabled?: boolean;
  onClick?: () => void;
  title: string;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      title={title}
      aria-pressed={active}
      className={clsx(
        "app-transition flex h-7 min-w-7 items-center justify-center rounded-full px-2 text-[11px] font-bold",
        disabled
          ? "cursor-not-allowed text-[#4a2108]/25"
          : active
            ? "bg-[#4a2108]/85 text-white"
            : "text-[#4a2108]/45 hover:bg-[#4a2108]/[0.06] hover:text-[#4a2108]/75",
      )}
    >
      {label}
    </button>
  );
}

// A single row in the "more" popover. Light, glassy, quiet — never a heavy
// context menu. Disabled items render greyed out so the user can see the shape
// of future features without being able to click them yet.
function MoreMenuItem({
  icon: Icon,
  label,
  sublabel,
  onClick,
  disabled,
}: {
  icon: typeof Heart;
  label: string;
  sublabel: string;
  onClick?: () => void;
  disabled?: boolean;
}) {
  const displaySublabel =
    label === "Less like this" ? "减少推荐" : label === "Share" ? "分享" : sublabel;

  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      className="app-transition source-menu-row flex w-full items-center gap-3 rounded-[14px] px-3 py-2 text-left disabled:cursor-not-allowed disabled:opacity-35"
    >
      <Icon className="h-[15px] w-[15px] shrink-0 text-[#4a2108]/50" />
      <span className="min-w-0 flex-1">
        <span className="block truncate text-[13px] font-bold text-[#4a2108]/82">{label}</span>
        <span className="block truncate text-[11px] font-semibold text-[#4a2108]/38">
          {displaySublabel}
        </span>
      </span>
    </button>
  );
}

function BilibiliVideoAtmosphere({
  track,
  src,
  isPlaying,
  progressSeconds,
  danmakuItems,
  danmakuDebug,
}: {
  track: Track;
  src: string;
  isPlaying: boolean;
  progressSeconds: number;
  danmakuItems: DanmakuItem[];
  danmakuDebug?: BilibiliDanmakuDebug | null;
}) {
  const videoRef = useRef<HTMLVideoElement | null>(null);
  const [videoReady, setVideoReady] = useState(false);
  const [videoFailed, setVideoFailed] = useState(false);

  useEffect(() => {
    const video = videoRef.current;
    setVideoReady(false);
    setVideoFailed(false);
    if (!video) return;
    video.pause();
    video.removeAttribute("src");
    if (src) video.src = src;
    video.load();
  }, [src]);

  useEffect(() => {
    const video = videoRef.current;
    if (!video || !src || videoFailed) return;
    if (isPlaying) void video.play().catch(() => undefined);
    else video.pause();
  }, [isPlaying, src, videoFailed]);

  useEffect(() => {
    const video = videoRef.current;
    if (
      !video ||
      !src ||
      video.readyState < 2 ||
      Math.abs(video.currentTime - progressSeconds) < 0.85
    )
      return;
    video.currentTime = progressSeconds;
  }, [progressSeconds, src]);

  return (
    <div className="bilibili-atmosphere-stage relative aspect-video w-full overflow-hidden rounded-[28px] bg-[#32190f]/10 shadow-[0_30px_86px_rgba(74,33,8,0.14)]">
      <ArtworkImage
        src={resolveTrackCover(track).src}
        alt=""
        source="bilibili"
        className="absolute inset-0 h-full w-full scale-[1.03] object-cover opacity-45 blur-[3px] saturate-75"
      />
      {src && !videoFailed && (
        <video
          ref={videoRef}
          muted
          playsInline
          preload="metadata"
          onCanPlay={() => setVideoReady(true)}
          onError={() => {
            setVideoReady(false);
            setVideoFailed(true);
          }}
          onLoadedMetadata={(event) => {
            event.currentTarget.currentTime = Math.min(
              progressSeconds,
              Math.max(0, event.currentTarget.duration || progressSeconds),
            );
          }}
          className={clsx(
            "absolute inset-0 z-[1] h-full w-full object-cover blur-[1.8px] saturate-[0.86] contrast-[0.94] sepia-[0.12] transition-opacity duration-700",
            videoReady ? "opacity-[0.74]" : "opacity-0",
          )}
        />
      )}
      <div className="pointer-events-none absolute inset-0 z-10 bg-[linear-gradient(180deg,rgba(245,230,214,0.08),rgba(74,38,21,0.25)),radial-gradient(circle_at_center,transparent_34%,rgba(45,20,11,0.36)_100%)]" />
      <DanmakuAtmosphereLayer
        items={danmakuItems}
        currentTime={progressSeconds}
        isPlaying={isPlaying}
        hasLyrics={false}
        trackId={track.id}
        debug={danmakuDebug}
      />
      <div className="pointer-events-none absolute bottom-6 left-7 right-7 z-30 flex items-end justify-between gap-4">
        <p className="text-[10px] font-black uppercase tracking-[0.22em] text-white/56">
          Bilibili ·{" "}
          {!src
            ? "Cover Atmosphere"
            : videoReady
              ? "Video Atmosphere"
              : videoFailed
                ? "Cover Atmosphere"
                : "Preparing Atmosphere"}
        </p>
        {videoFailed && (
          <p className="text-[10px] font-semibold text-white/42">Video stream unavailable</p>
        )}
      </div>
    </div>
  );
}

function formatTime(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds <= 0) return "--:--";
  const minutes = Math.floor(seconds / 60);
  return `${minutes}:${Math.floor(seconds % 60)
    .toString()
    .padStart(2, "0")}`;
}
