import { act, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { ConfirmProvider } from "@/components/primitives/ConfirmDialog";
import { PollRefusedError, type Poll } from "@/ports";

import { PollCard, type PollCallbacks } from "./PollCard";

/** Thursday 26 November 2026, 14:02. */
const NOW = new Date("2026-11-26T13:02:00Z");

const single = (extra: Partial<Poll> = {}): Poll => ({
  question: "Kiedy gramy w ten weekend?",
  answers: [
    { id: 1, text: "Piątek 21:00", emoji: "🎲", votes: 3 },
    { id: 2, text: "Sobota 20:00", emoji: ":dice:", votes: 5 },
    { id: 3, text: "Niedziela po południu", votes: 1 },
  ],
  multiple: false,
  endsAt: "2026-11-27T12:00:00Z",
  voters: 9,
  mine: [],
  ...extra,
});

function setup(poll: Poll, { canEnd = false, clock = () => NOW, actions: overrides = {} } = {}) {
  const user = userEvent.setup();
  const actions = {
    vote: vi.fn<PollCallbacks["vote"]>(async () => {}),
    end: vi.fn<PollCallbacks["end"]>(async () => {}),
    voters: vi.fn<PollCallbacks["voters"]>(async () => ({
      voters: [
        { id: "u1", displayName: "Kenji" },
        { id: "u2", displayName: "Marta" },
        { id: "u3", displayName: "Julia" },
      ],
      nextCursor: null,
    })),
    ...overrides,
  };
  const props = {
    messageId: "m1",
    canEnd,
    actions,
    clock,
    customEmoji: [{ id: "e1", shortcode: ":dice:", glyph: "/emoji/dice.png" }],
  };
  const view = render(<PollCard {...props} poll={poll} />, { wrapper: ConfirmProvider });
  const rerender = (next: Poll) => view.rerender(<PollCard {...props} poll={next} />);
  const answer = (name: RegExp | string) => screen.getByRole("button", { name });
  return { user, actions, answer, rerender };
}

describe("voting in a poll", () => {
  it("waits for a pick, takes one answer of a single-answer poll, then sends it with Zagłosuj", async () => {
    const { user, actions, answer } = setup(single());
    const vote = screen.getByRole("button", { name: "Zagłosuj" });
    expect(vote).toBeDisabled();
    // Nothing is counted on screen before a vote.
    expect(screen.queryByText("56%")).toBeNull();

    await user.click(answer(/Piątek/));
    await user.click(answer(/Sobota/));
    expect(answer(/Sobota/)).toHaveAttribute("aria-pressed", "true");
    expect(answer(/Piątek/)).toHaveAttribute("aria-pressed", "false");

    await user.click(vote);
    expect(actions.vote).toHaveBeenCalledExactlyOnceWith("m1", [2]);
  });

  it("lets several answers be picked and unpicked where the poll allows it", async () => {
    const { user, actions, answer } = setup(single({ multiple: true }));
    expect(screen.getByText("Ankieta · kilka odpowiedzi")).toBeInTheDocument();

    await user.click(answer(/Niedziela/));
    await user.click(answer(/Piątek/));
    await user.click(answer(/Niedziela/));
    await user.click(answer(/Sobota/));
    await user.click(screen.getByRole("button", { name: "Zagłosuj" }));

    expect(actions.vote).toHaveBeenCalledExactlyOnceWith("m1", [1, 2]);
  });

  it("shows the results without a vote, and goes back to the answers", async () => {
    const { user, actions } = setup(single());

    await user.click(screen.getByRole("button", { name: "Pokaż wyniki" }));
    expect(screen.getByText("56%")).toBeInTheDocument();
    expect(actions.vote).not.toHaveBeenCalled();

    await user.click(screen.getByRole("button", { name: "Zagłosuj" }));
    expect(screen.getByRole("button", { name: /Sobota/ })).toHaveAttribute("aria-pressed", "false");
  });

  it("shows the reader's own vote among the results, and changes it from there", async () => {
    const { user, actions, answer } = setup(single({ mine: [2] }));
    expect(within(answer(/Sobota/)).getByText("Twój wybór")).toBeInTheDocument();
    expect(screen.getByText("9 głosów")).toBeInTheDocument();
    expect(screen.getByText("zostały 22 godz.")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Zmień głos" }));
    // The vote already given is picked.
    expect(answer(/Sobota/)).toHaveAttribute("aria-pressed", "true");
    await user.click(answer(/Piątek/));
    await user.click(screen.getByRole("button", { name: "Zagłosuj" }));

    expect(actions.vote).toHaveBeenCalledExactlyOnceWith("m1", [1]);
  });

  it("says why a vote was refused, and keeps the pick", async () => {
    const { user, answer } = setup(single(), {
      actions: { vote: vi.fn(async () => Promise.reject(new PollRefusedError("ended"))) },
    });

    await user.click(answer(/Piątek/));
    await user.click(screen.getByRole("button", { name: "Zagłosuj" }));

    expect(await screen.findByRole("alert")).toHaveTextContent("Ta ankieta już się zakończyła.");
    expect(answer(/Piątek/)).toHaveAttribute("aria-pressed", "true");
  });

  it("names who chose an answer when it is pointed at", async () => {
    const { user, actions, answer } = setup(single({ mine: [2] }));

    await user.hover(answer(/Sobota/));

    expect(await screen.findByRole("tooltip")).toHaveTextContent("Kenji, Marta, Julia i 2 inne osoby");
    expect(actions.voters).toHaveBeenCalledWith("m1", 2);
  });
});

describe("a poll as it changes", () => {
  it("keeps a pick in progress when new counts arrive", async () => {
    const { user, answer, rerender } = setup(single());
    await user.click(answer(/Niedziela/));

    rerender(single({ answers: single().answers.map((a) => ({ ...a, votes: a.votes + 1 })), voters: 12 }));

    expect(answer(/Niedziela/)).toHaveAttribute("aria-pressed", "true");
  });

  it("closes on screen when its time runs out, with the leading answer bold and nothing to press", () => {
    let now = NOW;
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    try {
      setup(single({ endsAt: "2026-11-26T13:02:30Z" }), { clock: () => now });
      expect(screen.getByText("Ankieta · jedna odpowiedź")).toBeInTheDocument();

      now = new Date("2026-11-26T13:03:00Z");
      act(() => vi.advanceTimersByTime(60_000));

      expect(screen.getByText("Ankieta · zakończona")).toBeInTheDocument();
      expect(screen.getByText(/^zakończona dziś o /)).toBeInTheDocument();
      expect(screen.queryByRole("button", { name: "Zagłosuj" })).toBeNull();
      expect(screen.queryByRole("button", { name: "Zakończ" })).toBeNull();
    } finally {
      vi.useRealTimers();
    }
  });

  it("ends early for whoever may, after asking", async () => {
    const { user, actions } = setup(single({ mine: [1] }), { canEnd: true });

    await user.click(screen.getByRole("button", { name: "Zakończ" }));
    const card = screen.getByRole("dialog", { name: "Zakończyć ankietę?" });
    expect(actions.end).not.toHaveBeenCalled();
    await user.click(within(card).getByRole("button", { name: "Zakończ" }));

    expect(actions.end).toHaveBeenCalledExactlyOnceWith("m1");
  });

  it("offers no end to someone who neither wrote it nor may manage messages", () => {
    setup(single({ mine: [1] }));
    expect(screen.queryByRole("button", { name: "Zakończ" })).toBeNull();
  });
});
