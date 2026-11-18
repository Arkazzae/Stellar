import { fireEvent, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import type { DragHandleProps } from "@/lib/drag/useRailDrag";

import { ChannelButton } from "./ChannelButton";

describe("ChannelButton interaction structure", () => {
  it("renders selection, thread disclosure and settings as sibling buttons", async () => {
    const user = userEvent.setup();
    const onSelect = vi.fn();
    const onToggleThreads = vi.fn();
    const onOpenSettings = vi.fn();
    const { container } = render(
      <ChannelButton
        kind="text"
        name="ogólny"
        active
        threadCount={2}
        threadsId="threads-ogolny"
        onToggleThreads={onToggleThreads}
        onOpenSettings={onOpenSettings}
        onSelect={onSelect}
      />,
    );

    const row = container.querySelector<HTMLElement>(".channel");
    expect(row).not.toBeNull();
    expect(row?.tagName).toBe("DIV");
    expect(row?.querySelector("button button")).toBeNull();

    const main = within(row!).getByRole("button", { name: "ogólny" });
    const threads = within(row!).getByRole("button", {
      name: "Pokaż wątki kanału ogólny",
    });
    const settings = within(row!).getByRole("button", {
      name: "Ustawienia kanału: ogólny",
    });

    expect(main).toHaveAttribute("aria-current", "page");
    expect(threads).toHaveAttribute("aria-controls", "threads-ogolny");

    await user.click(threads);
    expect(onToggleThreads).toHaveBeenCalledOnce();
    expect(onSelect).not.toHaveBeenCalled();

    await user.click(settings);
    expect(onOpenSettings).toHaveBeenCalledOnce();
    expect(onSelect).not.toHaveBeenCalled();

    await user.click(main);
    expect(onSelect).toHaveBeenCalledOnce();
  });

  it("hands every key to the drag handle and still follows the channel on Enter", async () => {
    const user = userEvent.setup();
    const onSelect = vi.fn();
    const onOpenSettings = vi.fn();
    const handle: DragHandleProps = {
      ref: vi.fn(),
      onPointerDown: vi.fn(),
      onKeyDown: vi.fn(),
      onKeyUp: vi.fn(),
      onBlur: vi.fn(),
      "aria-describedby": "how-to-sort",
      "aria-roledescription": "kanał do przestawienia",
    };

    const { container } = render(
      <ChannelButton
        kind="text"
        name="ogólny"
        onOpenSettings={onOpenSettings}
        onSelect={onSelect}
        drag={handle}
      />,
    );

    const row = container.querySelector<HTMLElement>(".channel");
    const main = screen.getByRole("button", { name: "ogólny" });
    const settings = screen.getByRole("button", {
      name: "Ustawienia kanału: ogólny",
    });

    expect(row).not.toHaveAttribute("role");
    expect(main).toHaveAttribute("aria-describedby", "how-to-sort");

    fireEvent.keyDown(main, { key: "Enter" });
    expect(onSelect).toHaveBeenCalledOnce();

    fireEvent.keyDown(main, { key: " " });
    expect(handle.onKeyDown).toHaveBeenCalledTimes(2);
    expect(onSelect).toHaveBeenCalledOnce();

    await user.click(settings);
    expect(onOpenSettings).toHaveBeenCalledOnce();
    expect(handle.onPointerDown).not.toHaveBeenCalled();
    expect(onSelect).toHaveBeenCalledOnce();
  });
});
