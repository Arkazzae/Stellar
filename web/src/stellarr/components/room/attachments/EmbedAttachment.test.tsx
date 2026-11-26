import { render } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";

import type { EmbedPart } from "@/lib/messages/parts";
import { EmbedAttachment } from "./EmbedAttachment";

const part: EmbedPart = {
  kind: "embed", domain: "example.org", title: "A clip", description: "",
  url: "https://example.org/clip", animationUrl: "/assets/clip", showFullImage: true,
};

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

it.each([false, true])("loads a visible preview and respects reduced motion (%s)", (reduce) => {
  vi.stubGlobal("matchMedia", () => ({
    matches: reduce, addEventListener() {}, removeEventListener() {},
  }));
  let notify!: (entries: Pick<IntersectionObserverEntry, "target" | "isIntersecting">[]) => void;
  const unobserve = vi.fn();
  vi.stubGlobal("IntersectionObserver", class {
    constructor(callback: typeof notify) { notify = callback; }
    observe() {}
    unobserve = unobserve;
    disconnect() {}
  });
  const play = vi.spyOn(HTMLMediaElement.prototype, "play").mockResolvedValue(undefined);
  const pause = vi.spyOn(HTMLMediaElement.prototype, "pause").mockImplementation(() => {});

  const { container, unmount } = render(<EmbedAttachment part={part} />);
  const video = container.querySelector("video")!;
  const visible = (isIntersecting: boolean) => notify([{ target: video, isIntersecting }]);
  visible(false);
  expect(video).not.toHaveAttribute("src");
  visible(true);
  expect(video).toHaveAttribute("src", "/assets/clip");
  expect(play).toHaveBeenCalledTimes(reduce ? 0 : 1);
  pause.mockClear();
  visible(false);
  expect(pause).toHaveBeenCalledOnce();
  visible(true);
  expect(play).toHaveBeenCalledTimes(reduce ? 0 : 2);
  unmount();
  expect(unobserve).toHaveBeenCalledWith(video);
});
