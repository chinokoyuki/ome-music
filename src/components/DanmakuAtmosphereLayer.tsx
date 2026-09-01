import { useEffect, useRef, useState, type CSSProperties } from "react";
import type { BilibiliDanmakuDebug, DanmakuItem } from "../features/musicSources/provider";
import {
  getDanmakuSettings,
  type DanmakuMotionStyle,
  type DanmakuSettings,
} from "../features/danmaku/danmakuSettings";

interface DanmakuAtmosphereLayerProps {
  items: DanmakuItem[];
  currentTime: number;
  isPlaying: boolean;
  hasLyrics: boolean;
  trackId: string | null;
  debug?: BilibiliDanmakuDebug | null;
}

interface FloatingDanmaku {
  key: string;
  text: string;
  top: number;
  duration: number;
  delay: number;
  direction: "rtl" | "ltr";
  motion: Exclude<DanmakuMotionStyle, "mixed">;
}

export function DanmakuAtmosphereLayer({
  items,
  currentTime,
  isPlaying,
  hasLyrics,
  trackId,
  debug,
}: DanmakuAtmosphereLayerProps) {
  const [settings, setSettings] = useState<DanmakuSettings>(() => getDanmakuSettings());
  const [floating, setFloating] = useState<FloatingDanmaku[]>([]);
  const [showDebug, setShowDebug] = useState(() => {
    const isDevelopment = Boolean(
      (import.meta as ImportMeta & { env?: { DEV?: boolean } }).env?.DEV,
    );
    return (
      isDevelopment &&
      (new URLSearchParams(window.location.search).get("danmakuDebug") === "1" ||
        window.localStorage.getItem("ome.debug.danmaku") === "1")
    );
  });
  const lastTimeRef = useRef(0);
  const lastSpawnTimeRef = useRef(-10);
  const displayInVideo = settings.enabled && settings.displayMode === "video";

  useEffect(() => {
    const handleSettings = (event: Event) => {
      const custom = event as CustomEvent<DanmakuSettings>;
      setSettings(custom.detail ?? getDanmakuSettings());
    };
    window.addEventListener("ome:danmaku-settings", handleSettings);
    window.addEventListener("storage", handleSettings);
    return () => {
      window.removeEventListener("ome:danmaku-settings", handleSettings);
      window.removeEventListener("storage", handleSettings);
    };
  }, []);

  useEffect(() => {
    const isDevelopment = Boolean(
      (import.meta as ImportMeta & { env?: { DEV?: boolean } }).env?.DEV,
    );
    if (!isDevelopment) return;
    const toggleDebug = (event: KeyboardEvent) => {
      if (!(event.ctrlKey && event.shiftKey && event.key.toLowerCase() === "d")) return;
      setShowDebug((current) => {
        const next = !current;
        window.localStorage.setItem("ome.debug.danmaku", next ? "1" : "0");
        return next;
      });
    };
    window.addEventListener("keydown", toggleDebug);
    return () => window.removeEventListener("keydown", toggleDebug);
  }, []);

  useEffect(() => {
    setFloating([]);
    lastTimeRef.current = 0;
    lastSpawnTimeRef.current = -10;
  }, [items, trackId]);

  useEffect(() => {
    if (!displayInVideo) setFloating([]);
  }, [displayInVideo]);

  // Pause = silence: clear freeze-framed comments the moment playback stops
  // so the video atmosphere reads clean during a pause (see Global layer).
  useEffect(() => {
    if (!isPlaying) setFloating([]);
  }, [isPlaying]);

  useEffect(() => {
    if (!displayInVideo || !isPlaying || items.length === 0) return;

    const previousTime = lastTimeRef.current;
    const didSeek = currentTime < previousTime - 0.45 || currentTime > previousTime + 1.8;
    const fromTime = didSeek ? Math.max(0, currentTime - 1.2) : previousTime;
    if (didSeek) {
      setFloating([]);
      lastSpawnTimeRef.current = currentTime - 10;
    }

    const spawnInterval =
      settings.density === "high" ? 0.45 : settings.density === "medium" ? 0.9 : 1.6;
    const maxNewItems = settings.density === "high" ? 3 : settings.density === "medium" ? 2 : 1;
    const candidates = items
      .filter((item) => item.time > fromTime && item.time <= currentTime + 0.55)
      .filter((item) => !settings.hideLongComments || item.text.length <= 24)
      .slice(0, maxNewItems);
    lastTimeRef.current = currentTime;
    if (!candidates.length) return;
    if (!didSeek && currentTime - lastSpawnTimeRef.current < spawnInterval) return;
    lastSpawnTimeRef.current = currentTime;

    const additions = candidates.map((item, index): FloatingDanmaku => {
      const seed = stableNumber(`${trackId ?? "track"}:${item.id}`);
      const avoidCenter = settings.avoidLyricsArea && hasLyrics;
      const topBase = avoidCenter ? (seed % 2 === 0 ? 11 : 72) : 16 + (seed % 60);
      const direction =
        settings.direction === "mixed" ? (seed % 2 === 0 ? "rtl" : "ltr") : settings.direction;
      const motion = resolveMotionStyle(settings.motionStyle, seed);
      const durationBase =
        settings.speed === "fast" ? 7.6 : settings.speed === "normal" ? 10.5 : 14;
      const duration = durationBase * motionDurationFactor(motion);
      return {
        key: item.id,
        text: item.text,
        top: Math.max(8, Math.min(78, topBase + index * 5)),
        duration: duration + (seed % 4),
        delay: (seed % 40) / 100,
        direction,
        motion,
      };
    });

    setFloating((current) => {
      const existing = new Set(current.map((item) => item.key));
      const merged = [...current, ...additions.filter((item) => !existing.has(item.key))];
      const limit = settings.density === "high" ? 16 : settings.density === "medium" ? 10 : 6;
      return merged.slice(-limit);
    });
  }, [currentTime, displayInVideo, hasLyrics, isPlaying, items, settings, trackId]);

  const layerVisible = displayInVideo && floating.length > 0;

  if (!displayInVideo && !showDebug) return null;

  return (
    <div
      className="danmaku-layer pointer-events-none absolute inset-0 z-20 overflow-hidden rounded-[22px]"
      aria-hidden="true"
    >
      <div
        className="absolute inset-0"
        style={{
          opacity:
            (hasLyrics
              ? Math.min(0.42, settings.opacity * 0.82)
              : Math.max(0.56, settings.opacity)) * intensityOpacity(settings.emotionalIntensity),
        }}
      >
        {floating.map((item) => (
          <span
            key={item.key}
            className={`danmaku-atmosphere-line danmaku-motion-${item.motion}`}
            onAnimationEnd={() =>
              setFloating((current) => current.filter((entry) => entry.key !== item.key))
            }
            style={
              {
                top: `${item.top}%`,
                fontSize: fontSizeValue(settings.fontSize),
                animationDuration: `${item.duration}s`,
                animationDelay: `${item.delay}s`,
                animationName: motionAnimationName(item.motion, item.direction, "danmaku"),
                animationPlayState: isPlaying ? "running" : "paused",
                "--danmaku-glow": intensityGlow(settings.emotionalIntensity),
              } as CSSProperties
            }
          >
            <span
              className={`danmaku-entrance danmaku-entrance-${settings.entranceStyle}`}
              style={{
                animationDelay: `${item.delay}s`,
                animationPlayState: isPlaying ? "running" : "paused",
              }}
            >
              {item.text}
            </span>
          </span>
        ))}
      </div>
      {(showDebug ||
        ((import.meta as ImportMeta & { env?: { DEV?: boolean } }).env?.DEV &&
          Boolean(debug?.error))) && (
        <div className="absolute right-3 top-3 z-30 w-[min(310px,48%)] rounded-[12px] bg-black/62 p-3 font-mono text-[10px] leading-4 text-white/80 backdrop-blur-md">
          <p>bvid: {debug?.bvid || "-"}</p>
          <p>aid: {debug?.aid || "-"}</p>
          <p>cid: {debug?.cid || "-"}</p>
          <p className="truncate" title={debug?.danmakuRequestUrl}>
            request: {debug?.danmakuRequestUrl || "-"}
          </p>
          <p>
            raw: {String(Boolean(debug?.rawDanmakuLoaded))} / {debug?.rawDanmakuLength ?? 0} bytes
          </p>
          <p>
            parsed: {debug?.parsedDanmakuCount ?? items.length} / first:{" "}
            {debug?.firstDanmakuTime?.toFixed(2) ?? "-"}s
          </p>
          <p>
            current: {currentTime.toFixed(2)}s / active: {floating.length} / rendered:{" "}
            {floating.length}
          </p>
          <p>
            enabled: {String(settings.enabled)} / visible: {String(layerVisible)} / z: 20 / opacity:{" "}
            {(hasLyrics
              ? Math.min(0.42, settings.opacity * 0.82)
              : Math.max(0.56, settings.opacity)
            ).toFixed(2)}
          </p>
          {debug?.error && <p className="mt-1 break-words text-[#ffc5b7]">error: {debug.error}</p>}
        </div>
      )}
    </div>
  );
}

function resolveMotionStyle(
  style: DanmakuMotionStyle,
  seed: number,
): Exclude<DanmakuMotionStyle, "mixed"> {
  if (style !== "mixed") return style;
  return (["arc", "classic", "drift", "float", "pulse", "meteor"] as const)[seed % 6];
}

function motionAnimationName(
  motion: Exclude<DanmakuMotionStyle, "mixed">,
  direction: "rtl" | "ltr",
  prefix: "danmaku" | "ambient",
): string {
  if (motion === "float" || motion === "pulse") return `${prefix}-${motion}`;
  return `${prefix}-${motion}-${direction}`;
}

function motionDurationFactor(motion: Exclude<DanmakuMotionStyle, "mixed">): number {
  if (motion === "meteor") return 0.78;
  if (motion === "float") return 1.25;
  if (motion === "pulse") return 0.72;
  if (motion === "drift") return 1.12;
  if (motion === "arc") return 1.18;
  return 1;
}

function fontSizeValue(size: DanmakuSettings["fontSize"]): string {
  // Bumped one tier up so danmaku reads as atmosphere, not fine print.
  // Small ≈20.8px / Medium ≈26px / Large 32px. Medium is the default and
  // must be comfortably legible at full screen.
  return size === "small" ? "1.3rem" : size === "large" ? "2rem" : "1.625rem";
}

function intensityOpacity(intensity: DanmakuSettings["emotionalIntensity"]): number {
  return intensity === "quiet" ? 0.76 : intensity === "expressive" ? 1 : 0.9;
}

function intensityGlow(intensity: DanmakuSettings["emotionalIntensity"]): string {
  return intensity === "quiet" ? "0.14" : intensity === "expressive" ? "0.48" : "0.28";
}

function stableNumber(value: string): number {
  let hash = 0;
  for (let index = 0; index < value.length; index += 1) {
    hash = (hash * 31 + value.charCodeAt(index)) >>> 0;
  }
  return hash;
}
