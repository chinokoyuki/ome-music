import { describe, expect, it } from "vitest";
import { getCurrentLyricIndex, parseLrc } from "./lyricsResolver";

describe("parseLrc", () => {
  it("parses timed lines", () => {
    const lines = parseLrc("[00:01.50]First line\n[00:05.00]Second line", {
      title: "Song",
      artist: "Artist",
    });
    expect(lines).toHaveLength(2);
    expect(lines[0].startTime).toBeCloseTo(1.5);
    expect(lines[1].text).toBe("Second line");
  });

  it("ignores non-timed text lines", () => {
    const lines = parseLrc("plain text line\n[00:01.00]Timed", { title: "Song", artist: "Artist" });
    expect(lines).toHaveLength(1);
    expect(lines[0].text).toBe("Timed");
  });

  it("filters title/artist credit headers inside the first 20 seconds", () => {
    const lrc = [
      "[00:01.00]가사/歌词 - Song title - Artist name",
      "[00:02.00]Actual first line",
    ].join("\n");
    const lines = parseLrc(lrc, { title: "Song title", artist: "Artist name" });
    expect(lines.map((line) => line.text)).toEqual(["Actual first line"]);
  });

  it("filters 作词/作曲 credit lines early in the track", () => {
    const lrc = ["[00:00.50]作词：某某", "[00:01.00]作曲：某某", "[00:03.00]Real lyric"].join("\n");
    const lines = parseLrc(lrc, { title: "T", artist: "A" });
    expect(lines.map((line) => line.text)).toEqual(["Real lyric"]);
  });

  it("keeps a legitimate early lyric that looks like a title separator", () => {
    // A real first line containing the title is below the 20s threshold and
    // matches the separator pattern, but the whole-text match must win for
    // the honest case: when the text does not actually contain title+artist
    // tokens, it is kept.
    const lrc = "[00:10.00]We are the champions".split("\n")[0];
    const lines = parseLrc(lrc, { title: "Song title", artist: "Artist name" });
    expect(lines[0].text).toBe("We are the champions");
  });
});

describe("getCurrentLyricIndex", () => {
  const lyrics = [
    { id: "0", startTime: 1, text: "a" },
    { id: "1", startTime: 5, text: "b" },
    { id: "2", startTime: 9, text: "c" },
  ];

  it("returns -1 before the first real line", () => {
    expect(getCurrentLyricIndex(lyrics, 0, 0)).toBe(-1);
  });

  it("finds the current line by binary search", () => {
    expect(getCurrentLyricIndex(lyrics, 6, 0)).toBe(1);
    expect(getCurrentLyricIndex(lyrics, 9, 0)).toBe(2);
  });

  it("applies the lyric offset", () => {
    // adjustedTime = 5.5 - 1.0 = 4.5 → still before line 1 (start 5) → line 0
    expect(getCurrentLyricIndex(lyrics, 5.5, -1000)).toBe(0);
    // adjustedTime = 5.5 + 1.0 = 6.5 → line 1
    expect(getCurrentLyricIndex(lyrics, 5.5, 1000)).toBe(1);
  });
});
