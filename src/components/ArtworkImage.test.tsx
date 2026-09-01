import { describe, expect, it } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import { ArtworkImage } from "./ArtworkImage";

describe("ArtworkImage", () => {
  it("renders the image when a src is provided", () => {
    render(<ArtworkImage src="https://cdn.example/cover.jpg" alt="Track art" />);
    const img = screen.getByRole("img", { name: "Track art" });
    expect(img.tagName).toBe("IMG");
    expect(img).toHaveAttribute("src", "https://cdn.example/cover.jpg");
  });

  it("falls back to artwork placeholder when src is empty", () => {
    render(<ArtworkImage src="" alt="Track art" />);
    // Fallback is a div with role=img and an "artwork unavailable" label.
    const fallback = screen.queryByRole("img", { name: "Track art artwork unavailable" });
    expect(fallback).not.toBeNull();
  });

  it("resets the failed state when src changes", () => {
    const { rerender } = render(<ArtworkImage src="https://cdn.example/broken.jpg" alt="Cover" />);
    // Simulate a load error on the img element (wrapped in act by fireEvent).
    fireEvent.error(screen.getByRole("img", { name: "Cover" }));

    // After error, the fallback is rendered.
    expect(screen.queryByRole("img", { name: "Cover artwork unavailable" })).not.toBeNull();

    // New src → fallback state resets (failedSrc cleared) → image renders again.
    rerender(<ArtworkImage src="https://cdn.example/new.jpg" alt="Cover" />);
    expect(screen.getByRole("img", { name: "Cover" }).tagName).toBe("IMG");
  });

  it("renders the fallback again if the same src errors again", () => {
    const { rerender } = render(<ArtworkImage src="https://cdn.example/broken.jpg" alt="Cover" />);
    fireEvent.error(screen.getByRole("img", { name: "Cover" }));
    rerender(<ArtworkImage src="https://cdn.example/broken.jpg" alt="Cover" />);
    // Same src: failed state persists, fallback still shown.
    expect(screen.queryByRole("img", { name: "Cover artwork unavailable" })).not.toBeNull();
  });
});
