import { invoke } from "@tauri-apps/api/core";
import { isTauriRuntime } from "../library/libraryApi";
import type { Track } from "../../types/music";

export interface ResolvedLyrics {
  cacheKey: string;
  source: string;
  lyrics: string;
  translatedLyrics: string;
  confidence: number;
  warning?: string | null;
  offsetMs: number;
}

export interface LyricLine {
  id: string;
  startTime: number;
  text: string;
}

const timeTagPattern = /\[(\d{1,2}):(\d{2})(?:[.:](\d{1,3}))?\]/g;

export async function resolveLyrics(track: Track): Promise<ResolvedLyrics> {
  if (!isTauriRuntime()) {
    return {
      cacheKey: `${track.source || "local"}:${track.sourceId || track.id}:lrc`,
      source: track.source || "local",
      lyrics: "",
      translatedLyrics: "",
      confidence: 0,
      warning: "No matched lyrics for this version.",
      offsetMs: 0,
    };
  }

  return invoke<ResolvedLyrics>("resolve_track_lyrics", { payload: { track } });
}

export async function importLyricsFile(track: Track): Promise<ResolvedLyrics> {
  if (!isTauriRuntime()) {
    return {
      cacheKey: `${track.source || "local"}:${track.sourceId || track.id}:lrc`,
      source: track.source || "local",
      lyrics: "",
      translatedLyrics: "",
      confidence: 0,
      warning: "No matched lyrics for this version.",
      offsetMs: 0,
    };
  }

  return invoke<ResolvedLyrics>("import_track_lyrics", { payload: { track } });
}

export async function saveLyricOffset(cacheKey: string, offsetMs: number): Promise<void> {
  if (!isTauriRuntime()) return;
  await invoke("save_lyric_offset", { payload: { cacheKey, offsetMs } });
}

export function parseLrc(rawLyrics: string, track?: Pick<Track, "title" | "artist">): LyricLine[] {
  const lines: LyricLine[] = [];

  rawLyrics.split(/\r?\n/).forEach((rawLine, rawIndex) => {
    const timestamps = [...rawLine.matchAll(timeTagPattern)];
    const text = rawLine.replace(timeTagPattern, "").trim();
    if (!timestamps.length || !text) return;

    timestamps.forEach((match, tagIndex) => {
      const minutes = Number(match[1] || 0);
      const seconds = Number(match[2] || 0);
      const fraction = match[3] ?? "0";
      const millis = Number(fraction.padEnd(3, "0").slice(0, 3));
      const startTime = minutes * 60 + seconds + millis / 1000;
      if (isLyricMetadataLine(text, startTime, track)) return;
      lines.push({
        id: `${rawIndex}-${tagIndex}-${minutes}-${seconds}-${millis}`,
        startTime,
        text,
      });
    });
  });

  return lines.sort((a, b) => a.startTime - b.startTime);
}

export function getCurrentLyricIndex(
  lines: LyricLine[],
  currentTimeSeconds: number,
  offsetMs: number,
): number {
  if (!lines.length) return -1;
  const adjustedTime = currentTimeSeconds + offsetMs / 1000;
  let low = 0;
  let high = lines.length - 1;
  let result = -1;

  while (low <= high) {
    const mid = Math.floor((low + high) / 2);
    if (lines[mid].startTime <= adjustedTime) {
      result = mid;
      low = mid + 1;
    } else {
      high = mid - 1;
    }
  }

  return result;
}

const lyricCreditPattern =
  /^(?:作词|填词|词|作曲|曲|编曲|制作人|监制|混音|母带|和声|演唱|歌手|录音|吉他|贝斯|鼓|弦乐|出品|发行|op|sp|lyrics?|composer|arranger|producer|vocal|mixed by|mastered by)\s*[:：]/i;

function isLyricMetadataLine(
  text: string,
  startTime: number,
  track?: Pick<Track, "title" | "artist">,
): boolean {
  if (startTime > 20) return false;
  if (lyricCreditPattern.test(text.trim())) return true;
  if (!track?.title || !track.artist) return false;

  const normalizedText = normalizeLyricIdentity(text);
  const normalizedTitle = normalizeLyricIdentity(track.title);
  const normalizedArtist = normalizeLyricIdentity(track.artist);
  const hasTitleArtistSeparator = /[-—–·|｜()（）]/.test(text);
  return (
    hasTitleArtistSeparator &&
    normalizedTitle.length >= 2 &&
    normalizedArtist.length >= 2 &&
    normalizedText.includes(normalizedTitle) &&
    normalizedText.includes(normalizedArtist)
  );
}

function normalizeLyricIdentity(value: string): string {
  return value.toLocaleLowerCase().replace(/[\s\-—–·|｜()（）【】\[\]《》]/g, "");
}

export function lyricCacheKeyForTrack(track: Track): string {
  return `${track.source || "local"}:${track.sourceId || track.id}:lrc`;
}
