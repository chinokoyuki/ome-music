export type TrackMood =
  | "calm"
  | "focused"
  | "romantic"
  | "melancholy"
  | "energetic"
  | "dreamy"
  | "happy"
  | "tired"
  | "anxious"
  | "sad"
  | "excited"
  | "unknown";

export type JournalMood = "开心" | "平静" | "疲惫" | "焦虑" | "伤感" | "兴奋";

export type DesiredMusicVibe =
  "安静氛围" | "轻快明亮" | "专注背景" | "情绪陪伴" | "能量提升" | "探索新歌";

// LoopMode drives the "what happens when this track ends" decision.
//   off      — stop after the current track
//   one      — repeat the same track
//   all      — list loop (advance to next track, wrap at end)
//   curator  — Curator mode: advance like list loop for now; the taste-based
//              "recommend next" wiring is a later phase. Distinct from "all"
//              so the UI can show a different icon and so the ended handler
//              can later swap in a recommendation step without touching the
//              mode cycle.
export type LoopMode = "off" | "one" | "all" | "curator";

// Unified playback mode shown by the single mode button in the player bar.
// Maps onto (LoopMode, shuffle) so the existing playAdjacentTrack / ended
// logic keeps working unchanged.
//   curator    → loopMode="curator", shuffle=false
//   loop       → loopMode="all",     shuffle=false
//   repeat-one → loopMode="one",     shuffle=false
//   shuffle    → loopMode="all",     shuffle=true
export type PlaybackMode = "curator" | "loop" | "repeat-one" | "shuffle";

export type MusicSource = "local" | "netease" | "bilibili" | "qqmusic";

export type MusicLanguage = "zh" | "en" | "jp" | "kr" | "instrumental" | "unknown";

export interface Artist {
  id: string;
  name: string;
  aliases?: string[];
  genres: string[];
}

export interface Album {
  id: string;
  title: string;
  artistId: string;
  year?: number;
  coverUrl?: string;
}

export interface Track {
  id: string;
  title: string;
  artist: string;
  album: string;
  durationSeconds: number;
  filePath: string;
  source: MusicSource;
  sourceId?: string | null;
  unavailableReason?: string | null;
  coverUrl: string;
  genres: string[];
  moods: TrackMood[];
  language: MusicLanguage;
  year?: number;
  playCount: number;
  skipCount: number;
  liked: boolean;
  importedAt: string;
}

export interface Playlist {
  id: string;
  name: string;
  description?: string;
  trackIds: string[];
  source: "local" | "imported" | "ai";
  createdAt: string;
  updatedAt: string;
}

export interface PlaybackState {
  queue: Track[];
  currentTrackId: string | null;
  isPlaying: boolean;
  progressSeconds: number;
  volume: number;
  loopMode: LoopMode;
  shuffle: boolean;
}

export interface ListeningStats {
  hourOfDay: number;
  plays: number;
  completions: number;
  skips: number;
}

export interface ProfileRank {
  label: string;
  weight: number;
  confidence: number;
}

export interface HourPreference {
  hour: number;
  weight: number;
  confidence: number;
}

export interface ScoreConfidence {
  score: number;
  confidence: number;
}

export interface UserMusicProfile {
  favoriteArtists: ProfileRank[];
  favoriteAlbums: ProfileRank[];
  favoriteGenres: ProfileRank[];
  favoriteMoods: ProfileRank[];
  preferredListeningHours: HourPreference[];
  nightListeningPreference: ScoreConfidence;
  skipPatterns: ProfileRank[];
  repeatPatterns: ProfileRank[];
  likedSongPatterns: ProfileRank[];
  explorationScore: ScoreConfidence;
  calmMusicPreference: ScoreConfidence;
  energeticMusicPreference: ScoreConfidence;
  eventCount: number;
  confidence: number;
  isLearning: boolean;
  updatedAt: string;
}

export interface MoodEntry {
  id: string;
  date: string;
  mood: JournalMood;
  moodSignal: TrackMood;
  note: string;
  desiredVibe?: DesiredMusicVibe;
  privateTags: string[];
  recommendedTrackIds: string[];
  createdAt: string;
}
