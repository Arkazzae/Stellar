import { act, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { AppSkeleton } from "./AppSkeleton";
import styles from "./AppSkeleton.module.css";

describe("AppSkeleton", () => {
  afterEach(() => vi.useRealTimers());

  it("shows a tip only when a verified session is still loading", () => {
    vi.useFakeTimers();
    const tip = "Czy wiesz, że…?";
    const view = render(<AppSkeleton />);
    act(() => vi.advanceTimersByTime(800));
    expect(screen.queryByText(tip)).not.toBeInTheDocument();

    view.rerender(<AppSkeleton verifiedAt={performance.now()} />);
    act(() => vi.advanceTimersByTime(390));
    expect(screen.queryByText(tip)).not.toBeInTheDocument();
    act(() => vi.advanceTimersByTime(20));
    expect(screen.getByText(tip)).toBeInTheDocument();

    view.rerender(<AppSkeleton />);
    expect(screen.queryByText(tip)).not.toBeInTheDocument();
  });

  it("cancels a short session's tip when the skeleton unmounts", () => {
    vi.useFakeTimers();
    const view = render(<AppSkeleton verifiedAt={performance.now()} />);
    view.unmount();
    expect(vi.getTimerCount()).toBe(0);
  });

  it("announces that the application is busy", () => {
    render(<AppSkeleton />);

    const skeleton = screen.getByRole("region", {
      name: "Wczytywanie serwera…",
    });
    expect(skeleton).toHaveAttribute("aria-busy", "true");
    expect(screen.getByRole("status")).toHaveTextContent("Wczytywanie serwera…");
  });

  it("uses the provided loading label consistently", () => {
    render(<AppSkeleton label="Przygotowywanie rozmowy…" />);

    expect(
      screen.getByRole("region", { name: "Przygotowywanie rozmowy…" }),
    ).toBeInTheDocument();
    expect(screen.getByRole("status")).toHaveTextContent(
      "Przygotowywanie rozmowy…",
    );
  });

  it("keeps the visual placeholders out of the accessibility tree", () => {
    render(<AppSkeleton />);

    expect(screen.queryByRole("navigation")).not.toBeInTheDocument();
    expect(screen.queryByRole("main")).not.toBeInTheDocument();
    expect(screen.queryByRole("complementary")).not.toBeInTheDocument();
  });

  it("reserves every standard room-header action before the shell loads", () => {
    const { container } = render(<AppSkeleton />);
    const tools = container.querySelector(`.${styles.headerTools}`);

    expect(tools).not.toBeNull();
    expect(tools?.children).toHaveLength(3);
  });
});
