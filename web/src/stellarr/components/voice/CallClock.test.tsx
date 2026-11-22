import { act, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { CallClock } from "./CallClock";

afterEach(() => {
  vi.useRealTimers();
  vi.restoreAllMocks();
});

describe("CallClock", () => {
  it("rests out of sight and shows the right time the moment the page is back", () => {
    vi.useFakeTimers();
    const visibility = vi.spyOn(document, "visibilityState", "get").mockReturnValue("visible");
    render(<CallClock since={Date.now() - 5_000} />);
    expect(screen.getByText("00:05")).toBeTruthy();

    visibility.mockReturnValue("hidden");
    act(() => {
      document.dispatchEvent(new Event("visibilitychange"));
    });
    act(() => {
      vi.advanceTimersByTime(65_000);
    });
    expect(vi.getTimerCount()).toBe(0);

    visibility.mockReturnValue("visible");
    act(() => {
      document.dispatchEvent(new Event("visibilitychange"));
    });
    expect(screen.getByText("01:10")).toBeTruthy();
    act(() => {
      vi.advanceTimersByTime(1_000);
    });
    expect(screen.getByText("01:11")).toBeTruthy();
  });
});
