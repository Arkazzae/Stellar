import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { ConfirmProvider } from "@/components/primitives/ConfirmDialog";
import { EventRefusedError, type ServerEvent } from "@/ports";
import { BOARD_CHANNELS, serverEvent } from "@/test/events";

import { EventCard, type EventCardActions } from "./EventCard";

/** Thursday 26 November 2026, 13:30 in Warsaw. */
const BEFORE = new Date("2026-11-26T12:30:00Z");
const DURING = new Date("2026-11-28T19:10:00Z");

function setup(item: ServerEvent, { now = BEFORE, overrides = {}, manage = false } = {}) {
  const user = userEvent.setup();
  const actions = {
    signUp: vi.fn<NonNullable<EventCardActions["signUp"]>>(async () => {}),
    signOut: vi.fn<NonNullable<EventCardActions["signOut"]>>(async () => {}),
    join: vi.fn(),
    ...overrides,
  };
  const management = { edit: vi.fn(), cancel: vi.fn(async () => {}) };
  render(
    <EventCard
      event={item}
      now={now}
      zone="Europe/Warsaw"
      channel={BOARD_CHANNELS["vc-gaming"]}
      actions={actions}
      {...(manage ? { manage: management } : {})}
    />,
    { wrapper: ConfirmProvider },
  );
  return { user, actions, management, card: screen.getByRole("article", { name: item.name }) };
}

describe("an event's card", () => {
  it("signs up for the date it shows", async () => {
    const first = setup(serverEvent());
    expect(within(first.card).getByText("sobota, 28 listopada · 20:00–22:00")).toBeInTheDocument();
    await first.user.click(within(first.card).getByRole("button", { name: "Zapisz się" }));
    expect(first.actions.signUp).toHaveBeenCalledExactlyOnceWith(expect.objectContaining({ id: "ev-1", occurrenceId: "occ-1" }));
  });

  it("signs out from Zapisano", async () => {
    const { user, actions, card } = setup(serverEvent({ mine: "signed", signedUp: 8 }));
    const signed = within(card).getByRole("button", { name: "Zapisano" });
    expect(signed).toHaveAttribute("aria-pressed", "true");
    await user.click(signed);
    expect(actions.signOut).toHaveBeenCalledOnce();
  });

  it("puts the reader on the waitlist when it is full, and takes them off it", async () => {
    const full = setup(serverEvent({ signedUp: 10, waiting: 2 }));
    await full.user.click(within(full.card).getByRole("button", { name: "Na listę rezerwową" }));
    expect(full.actions.signUp).toHaveBeenCalledOnce();
  });

  it("takes the reader off the waitlist", async () => {
    const { user, actions, card } = setup(serverEvent({ signedUp: 10, waiting: 3, mine: "waiting", myPosition: 3 }));
    expect(within(card).getByText("Jesteś 3. w kolejce")).toBeInTheDocument();
    await user.click(within(card).getByRole("button", { name: "Na liście" }));
    expect(actions.signOut).toHaveBeenCalledOnce();
  });

  it("joins its voice channel while it runs", async () => {
    const { user, actions, card } = setup(serverEvent(), { now: DURING });
    expect(within(card).getByText("Trwa")).toBeInTheDocument();
    await user.click(within(card).getByRole("button", { name: "Dołącz do Gaming" }));
    expect(actions.join).toHaveBeenCalledExactlyOnceWith("vc-gaming");
  });

  it("says why a sign-up did not happen", async () => {
    const { user, card } = setup(serverEvent(), {
      overrides: { signUp: vi.fn(async () => Promise.reject(new EventRefusedError("gone"))) },
    });
    await user.click(within(card).getByRole("button", { name: "Zapisz się" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Tego wydarzenia już nie ma.");
  });

  it("offers its creator and managers a change or a cancellation, which asks first", async () => {
    const { user, management, card } = setup(serverEvent(), { manage: true });

    await user.click(within(card).getByRole("button", { name: "Zarządzaj wydarzeniem" }));
    await user.click(screen.getByRole("menuitem", { name: "Odwołaj wydarzenie" }));
    const question = screen.getByRole("dialog", { name: "Odwołać wydarzenie?" });
    expect(management.cancel).not.toHaveBeenCalled();
    await user.click(within(question).getByRole("button", { name: "Odwołaj wydarzenie" }));

    expect(management.cancel).toHaveBeenCalledOnce();
  });

  it("says when it was cancelled, with nothing to press", () => {
    const { card } = setup(serverEvent({ state: "cancelled" }), { manage: true });
    expect(within(card).getByText("Odwołane")).toBeInTheDocument();
    expect(within(card).queryByRole("button")).toBeNull();
  });

  it("offers nothing to manage to anyone else, and nothing to press once it is over", () => {
    const { card } = setup(serverEvent(), { now: new Date("2026-11-28T22:00:00Z") });
    expect(within(card).getByText("Zakończone")).toBeInTheDocument();
    expect(within(card).queryByRole("button")).toBeNull();
  });

  it("links a place somewhere else only when it is a web address", () => {
    setup(serverEvent({ name: "Spotkanie", place: { kind: "elsewhere", text: "https://meet.example.org/abc" } }));
    expect(screen.getByRole("link", { name: "https://meet.example.org/abc" })).toHaveAttribute("rel", "noopener noreferrer");
  });
});
