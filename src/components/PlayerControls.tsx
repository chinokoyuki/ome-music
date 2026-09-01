/* eslint-disable react-refresh/only-export-components -- playback constants are shared with the player shell */
import {
  ListMusic,
  Gauge,
  Pause,
  Play,
  Repeat,
  Repeat1,
  Shuffle,
  SkipBack,
  SkipForward,
  Sparkles,
  Volume2,
  VolumeX,
} from "lucide-react";
import type { LoopMode, PlaybackMode, Track } from "../types/music";
import { formatDuration } from "../utils/format";

// Allowed playback speeds (cycles forward). 1x must always be available and
// is the default — see App.tsx for persistence.
export const PLAYBACK_SPEEDS = [1, 1.25, 1.5, 2, 0.5, 0.75] as const;
export type PlaybackSpeed = (typeof PLAYBACK_SPEEDS)[number];

// Human-readable label for the current NetEase quality level. Surfaced on the
// progress bar so the user always knows what quality is actually playing.
export function qualityLabel(level: string | null | undefined): string {
  switch (level) {
    case "hires":
      return "Hi-Res";
    case "lossless":
      return "Lossless";
    case "exhigh":
      return "Ex High";
    case "higher":
      return "Higher";
    case "standard":
      return "Standard";
    default:
      return "";
  }
}

interface PlayerControlsProps {
  track: Track | null;
  isPlaying: boolean;
  progressSeconds: number;
  volume: number;
  shuffle: boolean;
  loopMode: LoopMode;
  playbackMode: PlaybackMode;
  playbackSpeed: PlaybackSpeed;
  qualityLevel: string | null | undefined;
  isQueueDrawerOpen: boolean;
  onTogglePlay: () => void;
  onNext: () => void;
  onPrevious: () => void;
  onCyclePlaybackMode: () => void;
  onCyclePlaybackSpeed: () => void;
  onToggleQueue: () => void;
  onSetProgress: (seconds: number) => void;
  onSetVolume: (volume: number) => void;
}

export function PlayerControls({
  track,
  isPlaying,
  progressSeconds,
  volume,
  playbackMode,
  playbackSpeed,
  qualityLevel,
  isQueueDrawerOpen,
  onTogglePlay,
  onNext,
  onPrevious,
  onCyclePlaybackMode,
  onCyclePlaybackSpeed,
  onToggleQueue,
  onSetProgress,
  onSetVolume,
}: PlayerControlsProps) {
  if (!track) {
    return null;
  }

  const maxSeconds = Math.max(1, track.durationSeconds);
  const progress = Math.min(progressSeconds, maxSeconds);
  const remaining = Math.max(0, maxSeconds - progress);
  const isMuted = volume <= 0;
  const speedLabel = playbackSpeed === 1 ? "1x" : `${playbackSpeed}x`;
  const badge = qualityLabel(qualityLevel);
  const modeMeta = playbackModeMeta(playbackMode);

  return (
    <footer
      data-danmaku-safe-zone="controls"
      className="player-dock-controls fixed bottom-[clamp(1.9rem,4.8vh,4rem)] z-30 text-[#4a2108]"
    >
      {/* Dock progress: kept inside the left player column so the room no
          longer reads as a full-width transport bar. */}
      <div className="mb-5">
        <input
          type="range"
          min={0}
          max={maxSeconds}
          value={progress}
          onChange={(event) => onSetProgress(Number(event.target.value))}
          className="cinema-progress"
          aria-label="Playback progress"
        />
        <div className="mt-3 flex items-center justify-between gap-3 text-xs font-semibold tabular-nums text-[#4a2108]/[0.48]">
          <span>{formatDuration(progress)}</span>
          {badge && (
            <span className="rounded-full bg-[#4a2108]/[0.07] px-2 py-0.5 text-[10px] font-bold tracking-wide text-[#4a2108]/55">
              {badge}
            </span>
          )}
          <span>-{formatDuration(remaining)}</span>
        </div>
      </div>

      <div className="grid grid-cols-[1fr_auto_1fr] items-center">
        {/* Left: playback mode + non-default speed indicator. */}
        <div className="flex items-center justify-start gap-1.5">
          <button
            type="button"
            onClick={onCyclePlaybackMode}
            className={`player-icon-button ${playbackMode !== "loop" ? "is-active" : ""}`}
            aria-label={modeMeta.label}
            title={modeMeta.label}
          >
            <modeMeta.Icon className="h-[17px] w-[17px]" />
          </button>
          {playbackSpeed !== 1 && (
            <button
              type="button"
              onClick={onCyclePlaybackSpeed}
              className="player-icon-button is-active gap-1.5 px-2.5"
              aria-label={`Playback speed ${speedLabel}`}
              title={`Playback speed ${speedLabel}`}
            >
              <Gauge className="h-[15px] w-[15px]" />
              <span className="text-[11px] font-bold tabular-nums">{speedLabel}</span>
            </button>
          )}
        </div>

        {/* Center: transport controls, the visual anchor of the dock. */}
        <div className="flex items-center justify-center gap-5">
          <button
            type="button"
            onClick={onPrevious}
            className="player-icon-button"
            aria-label="Previous track"
            title="Previous"
          >
            <SkipBack className="h-[18px] w-[18px] fill-current" />
          </button>
          <button
            type="button"
            onClick={onTogglePlay}
            className="player-main-button"
            aria-label={isPlaying ? "Pause" : "Play"}
            title={isPlaying ? "Pause" : "Play"}
          >
            {isPlaying ? (
              <Pause className="h-5 w-5 fill-current" />
            ) : (
              <Play className="ml-0.5 h-5 w-5 fill-current" />
            )}
          </button>
          <button
            type="button"
            onClick={onNext}
            className="player-icon-button"
            aria-label="Next track"
            title="Next"
          >
            <SkipForward className="h-[18px] w-[18px] fill-current" />
          </button>
        </div>

        {/* Right: queue. Volume sits below like the reference player. */}
        <div className="flex items-center justify-end">
          <button
            type="button"
            onClick={onToggleQueue}
            className={`player-icon-button ${isQueueDrawerOpen ? "is-active" : ""}`}
            aria-label="Toggle queue"
            title="Queue / 播放队列"
          >
            <ListMusic className="h-[17px] w-[17px]" />
          </button>
        </div>
      </div>

      <div className="mt-8 flex items-center gap-3">
        <button
          type="button"
          onClick={() => onSetVolume(isMuted ? 0.72 : 0)}
          className="player-volume-button"
          aria-label={isMuted ? "Unmute" : "Mute"}
          title={isMuted ? "Unmute" : "Mute"}
        >
          {isMuted ? <VolumeX className="h-3.5 w-3.5" /> : <Volume2 className="h-3.5 w-3.5" />}
        </button>
        <input
          type="range"
          min={0}
          max={1}
          step={0.01}
          value={volume}
          onChange={(event) => onSetVolume(Number(event.target.value))}
          className="cinema-range flex-1"
          aria-label="Volume"
        />
      </div>
    </footer>
  );
}

function playbackModeMeta(mode: PlaybackMode): { Icon: typeof Repeat; label: string } {
  switch (mode) {
    case "curator":
      // Curator mode: taste-based next-track recommendations. Icon is a soft
      // sparkle, NOT anything labelled as a model — the room is a music room, not a
      // tech demo.
      return { Icon: Sparkles, label: "Radio / 私人电台" };
    case "repeat-one":
      return { Icon: Repeat1, label: "Repeat One / 单曲循环" };
    case "shuffle":
      return { Icon: Shuffle, label: "Shuffle / 随机播放" };
    case "loop":
    default:
      return { Icon: Repeat, label: "Loop / 列表循环" };
  }
}
