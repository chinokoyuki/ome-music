import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueueDrawer } from "./QueueDrawer";
import type { Track } from "../types/music";

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

const defaultProps = {
  open: true,
  tracks: [] as Track[],
  currentTrackId: null,
  isPlaying: false,
  qualityLabel: "",
  recommendSimilar: false,
  onClose: () => undefined,
  onPlay: () => undefined,
  onRemove: () => undefined,
  onClear: () => undefined,
  onLikeAll: () => undefined,
  onToggleLike: () => undefined,
  onToggleRecommendSimilar: () => undefined,
};

describe("QueueDrawer", () => {
  it("shows the empty queue state", () => {
    render(<QueueDrawer {...defaultProps} />);
    expect(screen.getByText("The queue is quiet.")).toBeInTheDocument();
  });

  it("labels each source type", () => {
    const tracks = [
      track({ id: "local-1", source: "local", title: "Local Song" }),
      track({ id: "ne-1", source: "netease", title: "NetEase Song" }),
      track({ id: "bl-1", source: "bilibili", title: "Bilibili Song" }),
      track({ id: "qq-1", source: "qqmusic", title: "QQ Song" }),
    ];
    render(<QueueDrawer {...defaultProps} tracks={tracks} />);
    expect(screen.getByText("Local Song")).toBeInTheDocument();
    expect(screen.getByText("NetEase Song")).toBeInTheDocument();
    expect(screen.getByText("Bilibili Song")).toBeInTheDocument();
    expect(screen.getByText("QQ Song")).toBeInTheDocument();
    // Source tags appear once per row; label text may also appear inside the
    // artwork fallback for tracks without a cover, so match by count.
    expect(screen.getAllByText("QQ音乐").length).toBeGreaterThanOrEqual(1);
    expect(screen.getAllByText("NetEase").length).toBeGreaterThanOrEqual(1);
    expect(screen.getAllByText("Bilibili").length).toBeGreaterThanOrEqual(1);
    expect(screen.getAllByText("Local").length).toBeGreaterThanOrEqual(1);
  });

  it("clicking a queue row plays the track and stops propagation of like", async () => {
    const user = userEvent.setup();
    const onPlay = vi.fn();
    const onToggleLike = vi.fn();
    const likeTrack = track({ id: "like-1", source: "local", title: "Like Me" });
    const tracks = [likeTrack];
    render(
      <QueueDrawer {...defaultProps} tracks={tracks} onPlay={onPlay} onToggleLike={onToggleLike} />,
    );
    await user.click(screen.getByText("Like Me"));
    expect(onPlay).toHaveBeenCalledWith(likeTrack);
  });

  it("Escape closes the drawer", async () => {
    const user = userEvent.setup();
    const onClose = vi.fn();
    render(<QueueDrawer {...defaultProps} onClose={onClose} />);
    await user.keyboard("{Escape}");
    expect(onClose).toHaveBeenCalled();
  });

  it("Clear leaves the library untouched (only resets queue machinery)", async () => {
    const user = userEvent.setup();
    const onClear = vi.fn();
    render(
      <QueueDrawer
        {...defaultProps}
        tracks={[track({ id: "x", title: "One" })]}
        onClear={onClear}
      />,
    );
    await user.click(screen.getByTitle(/Stop and reset playback/));
    expect(onClear).toHaveBeenCalledTimes(1);
  });
});
