import { useEffect, useRef, useState, type CSSProperties } from "react";
import type { DanmakuItem } from "../features/musicSources/provider";
import {
  getDanmakuSettings,
  type DanmakuMotionStyle,
  type DanmakuSettings,
} from "../features/danmaku/danmakuSettings";

interface GlobalDanmakuAtmosphereLayerProps {
  items: DanmakuItem[];
  currentTime: number;
  isPlaying: boolean;
  trackId: string | null;
}

interface AmbientLine {
  key: string;
  text: string;
  top: number;
  left: number;
  width: number;
  duration: number;
  delay: number;
  direction: "rtl" | "ltr";
  depth: number;
  motion: Exclude<DanmakuMotionStyle, "mixed">;
}

interface SafeCorridor {
  top: number;
  left: number;
  width: number;
}

export function GlobalDanmakuAtmosphereLayer({
  items,
  currentTime,
  isPlaying,
  trackId,
}: GlobalDanmakuAtmosphereLayerProps) {
  const [settings, setSettings] = useState<DanmakuSettings>(() => getDanmakuSettings());
  const [lines, setLines] = useState<AmbientLine[]>([]);
  const linesRef = useRef<AmbientLine[]>([]);
  useEffect(() => {
    linesRef.current = lines;
  }, [lines]);
  const lastTimeRef = useRef(0);
  const lastSpawnTimeRef = useRef(-10);
  const isAmbient = settings.enabled && settings.displayMode === "ambient";

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
    setLines([]);
    lastTimeRef.current = 0;
    lastSpawnTimeRef.current = -10;
  }, [items, trackId]);

  useEffect(() => {
    if (!isAmbient) setLines([]);
  }, [isAmbient]);

  // Pause = silence: freeze-frame leftover danmaku lines would linger over
  // the artwork while the music is stopped, violating the quiet-idle
  // principle. Clear the stage the moment playback pauses; it refills from
  // the item window when playback resumes.
  useEffect(() => {
    if (!isPlaying) setLines([]);
  }, [isPlaying]);

  useEffect(() => {
    if (!isAmbient || lines.length === 0) return;
    const frame = window.requestAnimationFrame(() => {
      const safeRects = visibleSafeRects(settings.avoidLyricsArea);
      const unsafeKeys = new Set(
        Array.from(document.querySelectorAll<HTMLElement>("[data-ambient-danmaku-key]"))
          .filter((element) =>
            safeRects.some((rect) => rectanglesOverlap(element.getBoundingClientRect(), rect)),
          )
          .map((element) => element.dataset.ambientDanmakuKey)
          .filter((key): key is string => Boolean(key)),
      );
      if (unsafeKeys.size > 0)
        setLines((current) => current.filter((line) => !unsafeKeys.has(line.key)));
    });
    return () => window.cancelAnimationFrame(frame);
  }, [currentTime, isAmbient, lines.length, settings.avoidLyricsArea]);

  useEffect(() => {
    if (!isAmbient || !isPlaying || items.length === 0) return;

    const previousTime = lastTimeRef.current;
    const didSeek = currentTime < previousTime - 0.45 || currentTime > previousTime + 1.8;
    const fromTime = didSeek ? Math.max(0, currentTime - 1.15) : previousTime;
    if (didSeek) {
      setLines([]);
      lastSpawnTimeRef.current = currentTime - 10;
    }
    lastTimeRef.current = currentTime;

    const spawnInterval =
      settings.density === "high" ? 0.75 : settings.density === "medium" ? 1.25 : 2;
    if (!didSeek && currentTime - lastSpawnTimeRef.current < spawnInterval) return;

    const visibleText = new Set(linesRef.current.map((line) => normalizeText(line.text)));
    const maxNewItems = settings.density === "high" ? 2 : 1;
    const candidates = items
      .filter((item) => item.time > fromTime && item.time <= currentTime + 0.6)
      .filter((item) => !settings.hideLongComments || item.text.length <= 26)
      .filter((item, index, all) => {
        if (!settings.filterRepeated) return true;
        const normalized = normalizeText(item.text);
        return (
          !visibleText.has(normalized) &&
          all.findIndex((entry) => normalizeText(entry.text) === normalized) === index
        );
      })
      .slice(0, maxNewItems);
    if (!candidates.length) return;

    const corridors = measureSafeCorridors(settings.avoidLyricsArea);
    if (!corridors.length) return;
    lastSpawnTimeRef.current = currentTime;

    const additions = candidates.map((item, index): AmbientLine => {
      const seed = stableNumber(`${trackId ?? "track"}:${item.id}`);
      const corridor = corridors[(seed + index * 7) % corridors.length];
      const direction =
        settings.direction === "mixed" ? (seed % 2 === 0 ? "rtl" : "ltr") : settings.direction;
      const motion = resolveMotionStyle(settings.motionStyle, seed);
      const durationBase =
        settings.speed === "fast" ? 10.5 : settings.speed === "normal" ? 14 : 18.5;
      return {
        key: `${item.id}:${currentTime.toFixed(2)}`,
        text: item.text,
        top: corridor.top,
        left: corridor.left,
        width: corridor.width,
        duration: durationBase * motionDurationFactor(motion) + (seed % 4),
        delay: (seed % 35) / 100,
        direction,
        depth: seed % 3,
        motion,
      };
    });

    setLines((current) => {
      const limit = settings.density === "high" ? 16 : settings.density === "medium" ? 12 : 8;
      return [...current, ...additions].slice(-limit);
    });
  }, [currentTime, isAmbient, isPlaying, items, settings, trackId]);

  if (!isAmbient) return null;

  return (
    <div
      className="ambient-danmaku-layer pointer-events-none fixed inset-0 z-[22] overflow-hidden"
      aria-hidden="true"
    >
      {lines.map((line) => {
        const depthOpacity = line.depth === 0 ? 1 : line.depth === 1 ? 0.82 : 0.68;
        const style = {
          top: `${line.top}px`,
          left: `${line.left}px`,
          width: `${line.width}px`,
          opacity: settings.opacity * depthOpacity * intensityOpacity(settings.emotionalIntensity),
          "--ambient-travel": `${line.width}px`,
          "--ambient-mid": `${line.width * 0.42}px`,
          "--ambient-float-start": `${line.width * 0.72}px`,
          "--ambient-float-mid": `${line.width * 0.34}px`,
          "--ambient-float-end": `${line.width * 0.18}px`,
          "--ambient-pulse-start": `${line.width * 0.5}px`,
          "--ambient-pulse-mid": `${line.width * 0.48}px`,
          "--ambient-pulse-end": `${line.width * 0.46}px`,
          "--danmaku-glow": intensityGlow(settings.emotionalIntensity),
        } as CSSProperties;
        return (
          <div
            key={line.key}
            data-ambient-danmaku-key={line.key}
            className="ambient-danmaku-corridor fixed overflow-visible"
            style={style}
          >
            <span
              className={`ambient-danmaku-line danmaku-motion-${line.motion}`}
              onAnimationEnd={() =>
                setLines((current) => current.filter((entry) => entry.key !== line.key))
              }
              style={{
                animationDuration: `${line.duration}s`,
                animationDelay: `${line.delay}s`,
                animationName: motionAnimationName(line.motion, line.direction),
                animationPlayState: isPlaying ? "running" : "paused",
                fontSize: fontSizeValue(settings.fontSize),
              }}
            >
              <span
                className={`danmaku-entrance danmaku-entrance-${settings.entranceStyle}`}
                style={{
                  animationDelay: `${line.delay}s`,
                  animationPlayState: isPlaying ? "running" : "paused",
                }}
              >
                {line.text}
              </span>
            </span>
          </div>
        );
      })}
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
): string {
  if (motion === "float" || motion === "pulse") return `ambient-${motion}`;
  return `ambient-${motion}-${direction}`;
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
  // Small ≈20.8px / Medium ≈26px / Large 32px. Mirrors DanmakuAtmosphereLayer
  // so the same size system applies to both the in-video danmaku and the
  // ambient atmosphere layer.
  return size === "small" ? "1.3rem" : size === "large" ? "2rem" : "1.625rem";
}

function intensityOpacity(intensity: DanmakuSettings["emotionalIntensity"]): number {
  return intensity === "quiet" ? 0.72 : intensity === "expressive" ? 1 : 0.88;
}

function intensityGlow(intensity: DanmakuSettings["emotionalIntensity"]): string {
  return intensity === "quiet" ? "0.12" : intensity === "expressive" ? "0.44" : "0.26";
}

function measureSafeCorridors(avoidLyricsArea: boolean): SafeCorridor[] {
  const viewportWidth = window.innerWidth;
  const viewportHeight = window.innerHeight;
  const edge = Math.max(18, viewportWidth * 0.018);
  const top = Math.max(104, viewportHeight * 0.105);
  const bottom = viewportHeight - Math.max(148, viewportHeight * 0.16);
  const minWidth = Math.min(430, viewportWidth * 0.3);
  const safeRects = visibleSafeRects(avoidLyricsArea);

  const corridors: SafeCorridor[] = [];
  for (let laneTop = top; laneTop <= bottom - 64; laneTop += 72) {
    const laneBottom = laneTop + 64;
    const blocked = safeRects
      .filter((rect) => rect.top - 14 < laneBottom && rect.bottom + 14 > laneTop)
      .map(
        (rect) =>
          [
            Math.max(edge, rect.left - 18),
            Math.min(viewportWidth - edge, rect.right + 18),
          ] as const,
      )
      .sort((a, b) => a[0] - b[0]);

    let cursor = edge;
    for (const [start, end] of blocked) {
      if (start - cursor >= minWidth)
        corridors.push({ top: laneTop, left: cursor, width: start - cursor });
      cursor = Math.max(cursor, end);
    }
    if (viewportWidth - edge - cursor >= minWidth) {
      corridors.push({ top: laneTop, left: cursor, width: viewportWidth - edge - cursor });
    }
  }
  return corridors;
}

function visibleSafeRects(avoidLyricsArea: boolean): DOMRect[] {
  return Array.from(document.querySelectorAll<HTMLElement>("[data-danmaku-safe-zone]"))
    .filter((element) => avoidLyricsArea || element.dataset.danmakuSafeZone !== "lyrics")
    .filter((element) => !element.closest('[aria-hidden="true"]'))
    .filter((element) => {
      const style = window.getComputedStyle(element);
      return style.display !== "none" && style.visibility !== "hidden";
    })
    .map((element) => element.getBoundingClientRect())
    .filter((rect) => rect.width > 0 && rect.height > 0);
}

function rectanglesOverlap(a: DOMRect, b: DOMRect): boolean {
  return (
    a.left < b.right + 14 && a.right > b.left - 14 && a.top < b.bottom + 10 && a.bottom > b.top - 10
  );
}

function normalizeText(value: string): string {
  return value.replace(/\s+/g, "").toLowerCase();
}

function stableNumber(value: string): number {
  let hash = 0;
  for (let index = 0; index < value.length; index += 1) {
    hash = (hash * 31 + value.charCodeAt(index)) >>> 0;
  }
  return hash;
}
