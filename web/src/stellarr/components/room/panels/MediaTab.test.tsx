import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { MediaTab } from "./MediaTab";
import type { MediaItemVM } from "./channelPanelModel";

const status = { loading: false, loadingMore: false, hasMore: false, error: null };

describe("MediaTab", () => {
  it("draws the preview in a tile and opens the viewer with the original", () => {
    const media: MediaItemVM[] = [
      {
        id: "g0:0",
        part: { kind: "image", src: "/assets/a", preview: "/assets/a/preview", alt: "Kot" },
        authorName: "Ada",
        timestamp: "2026-08-13T10:00:00Z",
      },
    ];
    const onOpenImage = vi.fn();
    render(<MediaTab media={media} status={status} callbacks={{ onOpenImage }} />);

    const tile = screen.getByRole("button", { name: /Kot/ });
    expect(tile.querySelector("img")).toHaveAttribute("src", "/assets/a/preview");

    fireEvent.click(tile);
    expect(onOpenImage).toHaveBeenCalledWith(
      [{ src: "/assets/a", alt: "Kot", preview: "/assets/a/preview" }],
      0,
    );
  });
});
