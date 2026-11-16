import { act, fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import {
  fetchGiphyGif,
  readGiphyConfig,
  resolveGiphyApiKey,
  trackGiphyAction,
  type GiphyGif,
} from "@/lib/giphy/giphy";
import type * as GiphyModule from "@/lib/giphy/giphy";
import { giphyChoice } from "@/lib/giphy/giphyChoice";

import type { GiphyPart } from "@/lib/messages/parts";

import { GiphyAttachment } from "./GiphyAttachment";

vi.mock("@/lib/giphy/giphy", async (importOriginal) => ({
  ...(await importOriginal<typeof GiphyModule>()),
  readGiphyConfig: vi.fn(),
  answeredGiphyConfig: vi.fn(() => undefined),
  resolveGiphyApiKey: vi.fn(),
  fetchGiphyGif: vi.fn(),
  trackGiphyAction: vi.fn(),
}));

const part: GiphyPart = { kind: "giphy", id: "abc123", pageUrl: "https://giphy.com/gifs/abc123" };

const gif: GiphyGif = {
  id: "abc123",
  pageUrl: "https://giphy.com/gifs/cat-abc123",
  alt: "A dancing cat",
  username: "catlover",
  previewUrl: "https://media.giphy.com/preview.gif",
  mediaUrl: "https://media.giphy.com/media.gif",
  width: 320,
  height: 240,
  analytics: { onload: "https://giphy-analytics.giphy.com/view" },
};

beforeEach(() => {
  vi.mocked(fetchGiphyGif).mockReset();
  vi.mocked(trackGiphyAction).mockReset();
  vi.mocked(resolveGiphyApiKey).mockReset().mockResolvedValue("key");
  vi.mocked(readGiphyConfig).mockReset().mockResolvedValue({ enabled: true, apiKey: "key" });
  window.localStorage.clear();
  giphyChoice.set(true);
  giphyChoice.accountRead();
});

describe("GiphyAttachment", () => {
  it("shows a loading state, then the GIF with its attribution", async () => {
    vi.mocked(fetchGiphyGif).mockResolvedValue(gif);
    render(<GiphyAttachment part={part} />);

    expect(screen.getByRole("status")).toBeInTheDocument();
    const image = await screen.findByRole("img", { name: "A dancing cat" });
    expect(image).toHaveAttribute("src", gif.mediaUrl);
    // GIPHY is not told which page the GIF is shown on.
    expect(image).toHaveAttribute("referrerpolicy", "no-referrer");
    expect(screen.getByRole("link", { name: "@catlover w GIPHY" })).toHaveAttribute(
      "href",
      gif.pageUrl,
    );
    expect(fetchGiphyGif).toHaveBeenCalledWith("key", "abc123", expect.any(AbortSignal));
  });

  it("names a GIF GIPHY gave no title in the page's language", async () => {
    const { alt: _alt, ...untitled } = gif;
    vi.mocked(fetchGiphyGif).mockResolvedValue(untitled);
    render(<GiphyAttachment part={part} />);
    expect(await screen.findByRole("img", { name: "GIF z GIPHY" })).toBeInTheDocument();
  });

  it("holds the attribution's line while GIPHY is asked, out of reach of assistive tech", () => {
    vi.mocked(fetchGiphyGif).mockReturnValue(new Promise(() => undefined));
    render(<GiphyAttachment part={part} />);

    const caption = screen.getByRole("status").querySelector(".att-giphy-caption");
    expect(caption).not.toBeNull();
    expect(caption).toHaveAttribute("aria-hidden", "true");
    expect(screen.getByRole("status")).toHaveTextContent("Ładowanie GIF-a");
  });

  it("falls back to a link to GIPHY when the GIF cannot be fetched", async () => {
    vi.mocked(fetchGiphyGif).mockRejectedValue(new Error("gone"));
    render(<GiphyAttachment part={part} />);

    expect(await screen.findByText("GIF nie jest dostępny w podglądzie.")).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Otwórz w GIPHY" })).toHaveAttribute(
      "href",
      part.pageUrl,
    );
  });

  it("falls back the same way when the image itself fails to load", async () => {
    vi.mocked(fetchGiphyGif).mockResolvedValue(gif);
    render(<GiphyAttachment part={part} />);

    fireEvent.error(await screen.findByRole("img", { name: "A dancing cat" }));
    expect(screen.getByRole("link", { name: "Otwórz w GIPHY" })).toBeInTheDocument();
  });

  it("reports a view to GIPHY once, however often the image fires load", async () => {
    vi.mocked(fetchGiphyGif).mockResolvedValue(gif);
    render(<GiphyAttachment part={part} />);

    const image = await screen.findByRole("img", { name: "A dancing cat" });
    fireEvent.load(image);
    fireEvent.load(image);
    expect(trackGiphyAction).toHaveBeenCalledTimes(1);
    expect(trackGiphyAction).toHaveBeenCalledWith(gif.analytics.onload);
  });

  it("zooms the GIF through the lightbox callback when the host provides one", async () => {
    const user = userEvent.setup();
    const onOpen = vi.fn();
    vi.mocked(fetchGiphyGif).mockResolvedValue(gif);
    render(<GiphyAttachment part={part} onOpen={onOpen} />);

    await user.click(await screen.findByRole("button", { name: "Powiększ GIF: A dancing cat" }));
    expect(onOpen).toHaveBeenCalledWith([{ src: gif.mediaUrl, alt: gif.alt }], 0);
  });
});

describe("GiphyAttachment with the person's GIPHY switch off", () => {
  beforeEach(() => {
    giphyChoice.set(false);
  });

  it("asks GIPHY nothing, and offers the GIF's page and that one GIF", async () => {
    render(<GiphyAttachment part={part} />);

    expect(screen.getByText("GIF z GIPHY. Wyświetlenie go łączy tę przeglądarkę z GIPHY.")).toBeInTheDocument();
    const link = screen.getByRole("link", { name: "Otwórz w GIPHY" });
    expect(link).toHaveAttribute("href", part.pageUrl);
    expect(link.getAttribute("rel")).toContain("noreferrer");
    expect(await screen.findByRole("button", { name: "Pokaż GIF" })).toBeInTheDocument();
    expect(resolveGiphyApiKey).not.toHaveBeenCalled();
    expect(fetchGiphyGif).not.toHaveBeenCalled();
    expect(screen.queryByRole("img")).toBeNull();
  });

  it("loads the one GIF it is asked to, and no other", async () => {
    const user = userEvent.setup();
    vi.mocked(fetchGiphyGif).mockResolvedValue(gif);
    const other: GiphyPart = { kind: "giphy", id: "zzz999", pageUrl: "https://giphy.com/gifs/zzz999" };
    render(
      <>
        <GiphyAttachment part={part} />
        <GiphyAttachment part={other} />
      </>,
    );

    const [first] = await screen.findAllByRole("button", { name: "Pokaż GIF" });
    await user.click(first!);

    expect(await screen.findByRole("img", { name: "A dancing cat" })).toBeInTheDocument();
    expect(fetchGiphyGif).toHaveBeenCalledOnce();
    expect(fetchGiphyGif).toHaveBeenCalledWith("key", "abc123", expect.any(AbortSignal));
    expect(screen.getAllByRole("button", { name: "Pokaż GIF" })).toHaveLength(1);
  });

  it("offers no button while the instance offers no GIFs", async () => {
    vi.mocked(readGiphyConfig).mockResolvedValue({ enabled: false, apiKey: "" });
    render(<GiphyAttachment part={part} />);
    await act(async () => {
      await Promise.resolve();
    });

    expect(screen.queryByRole("button", { name: "Pokaż GIF" })).toBeNull();
    expect(screen.getByRole("link", { name: "Otwórz w GIPHY" })).toBeInTheDocument();
  });

  it("stops showing a GIF once the switch is turned off", async () => {
    giphyChoice.set(true);
    vi.mocked(fetchGiphyGif).mockResolvedValue(gif);
    render(<GiphyAttachment part={part} />);
    await screen.findByRole("img", { name: "A dancing cat" });

    act(() => giphyChoice.set(false));

    expect(screen.queryByRole("img")).toBeNull();
    expect(screen.getByRole("link", { name: "Otwórz w GIPHY" })).toBeInTheDocument();
  });
});

describe("GiphyAttachment before the account's copy of the switch has come", () => {
  it("waits without asking GIPHY, then loads once it is on", async () => {
    giphyChoice.awaitAccount();
    vi.mocked(fetchGiphyGif).mockResolvedValue(gif);
    render(<GiphyAttachment part={part} />);

    expect(screen.getByRole("status")).toBeInTheDocument();
    expect(resolveGiphyApiKey).not.toHaveBeenCalled();
    expect(fetchGiphyGif).not.toHaveBeenCalled();

    act(() => giphyChoice.accountRead());

    expect(await screen.findByRole("img", { name: "A dancing cat" })).toBeInTheDocument();
  });
});
