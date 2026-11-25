import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { MediaTab } from "./MediaTab";
import type { MediaItemVM } from "./channelPanelModel";

const status = { loading: false, loadingMore: false, hasMore: false, error: null };

describe("MediaTab", () => {
  it("opens an encrypted file through the download callback", () => {
    const part = { kind: "file" as const, name: "notes.txt", ext: "txt", size: "12 B", url: "e2ee:file-1" };
    const onDownloadFile = vi.fn();
    render(<MediaTab
      media={[{ id: "file-1", part, authorName: "Ada", timestamp: "2026-08-13T10:00:00Z" }]}
      status={status}
      callbacks={{ onDownloadFile }}
    />);

    expect(screen.queryByRole("link")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: /notes\.txt/ }));
    expect(onDownloadFile).toHaveBeenCalledExactlyOnceWith(part);
  });

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
