import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { BOARD_CHANNELS, serverEvent } from "@/test/events";

import { EventsRail } from "./EventsRail";

const BEFORE = new Date("2026-11-26T12:30:00Z");
const DURING = new Date("2026-11-28T19:10:00Z");

const coming = [
  serverEvent({ mine: "signed", signedUp: 8 }),
  serverEvent({
    id: "ev-2",
    name: "Przegląd sprintu",
    startsAt: "2026-11-30T09:00:00Z",
    endsAt: undefined,
    repeat: "weekly",
    seats: undefined,
    signedUp: 4,
    place: { kind: "voice", channelId: "vc-meet" },
  }),
];

describe("the events row in the rail", () => {
  it("counts what is coming, and opens the list soonest first", async () => {
    const user = userEvent.setup();
    const onOpen = vi.fn();
    render(<EventsRail events={coming} now={BEFORE} zone="Europe/Warsaw" channels={BOARD_CHANNELS} onOpen={onOpen} />);

    const row = screen.getByRole("button", { name: /Wydarzenia/ });
    expect(within(row).getByLabelText("Nadchodzące: 2")).toBeInTheDocument();
    await user.click(row);

    const list = screen.getByRole("dialog", { name: "Wydarzenia" });
    const items = within(list).getAllByRole("button").map((item) => item.textContent);
    expect(items).toEqual([
      expect.stringContaining("Wieczór planszówek20:00 · Gaming · 8 z 10 miejscZapisano"),
      expect.stringContaining("Przegląd sprintuco tydzień · 10:00 · Spotkania · 4 osoby"),
    ]);
    // Only whoever may create events is offered a new one.
    expect(within(list).queryByRole("button", { name: "Nowe" })).toBeNull();

    await user.click(within(list).getByRole("button", { name: /Przegląd sprintu/ }));
    expect(onOpen).toHaveBeenCalledWith(expect.objectContaining({ id: "ev-2" }));
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("says when one runs, and leaves cancelled ones out of the list", async () => {
    const user = userEvent.setup();
    const onCreate = vi.fn();
    render(
      <EventsRail
        events={[...coming, serverEvent({ id: "old", name: "Stare", state: "cancelled", startsAt: "2026-11-30T19:00:00Z" })]}
        channels={BOARD_CHANNELS}
        now={DURING}
        zone="Europe/Warsaw"
        onOpen={vi.fn()}
        onCreate={onCreate}
      />,
    );

    const row = screen.getByRole("button", { name: /Wydarzenia/ });
    expect(within(row).getByText("trwa")).toBeInTheDocument();
    await user.click(row);
    const list = screen.getByRole("dialog", { name: "Wydarzenia" });
    expect(within(list).queryByText("Stare")).toBeNull();
    await user.click(within(list).getByRole("button", { name: "Nowe" }));
    expect(onCreate).toHaveBeenCalledOnce();
  });

  it("closes the list with Escape", async () => {
    const user = userEvent.setup();
    render(<EventsRail events={coming} now={BEFORE} zone="Europe/Warsaw" channels={BOARD_CHANNELS} onOpen={vi.fn()} />);
    await user.click(screen.getByRole("button", { name: /Wydarzenia/ }));
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("opens the list beside the rail, level with the row, and inside the window", async () => {
    const user = userEvent.setup();
    const box = (left: number, top: number, width: number, height: number) =>
      ({ left, top, right: left + width, bottom: top + height, width, height, x: left, y: top }) as DOMRect;
    const rendered = () =>
      render(
        <nav className="rail">
          <EventsRail events={coming} now={BEFORE} zone="Europe/Warsaw" channels={BOARD_CHANNELS} onOpen={vi.fn()} />
        </nav>,
      );
    vi.spyOn(window, "innerHeight", "get").mockReturnValue(800);
    vi.spyOn(window, "innerWidth", "get").mockReturnValue(1400);

    const high = rendered();
    vi.spyOn(high.container.querySelector(".rail")!, "getBoundingClientRect").mockReturnValue(box(72, 0, 240, 800));
    const row = screen.getByRole("button", { name: /Wydarzenia/ });
    vi.spyOn(row, "getBoundingClientRect").mockReturnValue(box(80, 120, 224, 34));
    await user.click(row);

    const list = screen.getByRole("dialog", { name: "Wydarzenia" });
    expect(list.style.left).toBe("320px");
    expect(list.style.top).toBe("120px");
    expect(list.style.getPropertyValue("--list-max-height")).toBe("520px");
    high.unmount();

    // Too low for the list to fit under its top: it moves up.
    const low = rendered();
    vi.spyOn(low.container.querySelector(".rail")!, "getBoundingClientRect").mockReturnValue(box(72, 0, 240, 800));
    const lowRow = screen.getByRole("button", { name: /Wydarzenia/ });
    vi.spyOn(lowRow, "getBoundingClientRect").mockReturnValue(box(80, 700, 224, 34));
    await user.click(lowRow);

    const raised = screen.getByRole("dialog", { name: "Wydarzenia" });
    expect(raised.style.top).toBe("508px");
    expect(raised.style.getPropertyValue("--list-max-height")).toBe("280px");
    vi.restoreAllMocks();
  });
});
