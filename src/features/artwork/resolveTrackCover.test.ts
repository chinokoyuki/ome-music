import { describe, expect, it } from "vitest";
import { resolveTrackCover } from "./resolveTrackCover";
import type { Track } from "../../types/music";

function track(overrides: Partial<Track>): Track {
  return {
    id: "t1",
    title: "Song",
    artist: "Artist",
    album: "Album",
    durationSeconds: 200,
    filePath: "/music/song.mp3",
    source: "local",
    sourceId: null,
    unavailableReason: null,
    coverUrl: "",
    genres: [],
    moods: [],
    language: "unknown",
    year: undefined,
    playCount: 0,
    skipCount: 0,
    liked: false,
    importedAt: new Date().toISOString(),
    ...overrides,
  };
}

describe("resolveTrackCover", () => {
  it("passes through stable remote URLs unchanged", () => {
    const result = resolveTrackCover(
      track({ source: "netease", coverUrl: "https://p1.music.126.net/abc.jpg" }),
    );
    expect(result.src).toBe("https://p1.music.126.net/abc.jpg");
    expect(result.isTransient).toBe(false);
    expect(result.source).toBe("netease");
  });

  it("marks transient ome-media proxy URLs for the current session", () => {
    const result = resolveTrackCover(
      track({ source: "bilibili", coverUrl: "http://ome-media.localhost/abc123" }),
    );
    expect(result.isTransient).toBe(true);
  });

  it("marks transient blob URLs", () => {
    const result = resolveTrackCover(track({ source: "local", coverUrl: "blob:abc" }));
    expect(result.isTransient).toBe(true);
  });

  it("returns empty src for a null track without panicking", () => {
    const result = resolveTrackCover(null);
    expect(result.src).toBe("");
    expect(result.source).toBe("unknown");
    expect(result.fallbackLabel).toBe("Ome");
  });

  it("handles empty cover URLs as non-transient", () => {
    const result = resolveTrackCover(track({ coverUrl: "" }));
    expect(result.src).toBe("");
    expect(result.isTransient).toBe(false);
  });
});
