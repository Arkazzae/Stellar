import { fireEvent, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import type { Poll } from "@/ports";
import { renderWithConfirm } from "@/test/confirm";

import { MessageGroup } from "./MessageGroup";

const poll: Poll = {
  question: "Kiedy gramy w ten weekend?",
  answers: [
    { id: 1, text: "Piątek 21:00", votes: 0 },
    { id: 2, text: "Sobota 20:00", votes: 0 },
  ],
  multiple: false,
  endsAt: "2999-01-01T00:00:00Z",
  voters: 0,
  mine: [],
};

describe("a message with a poll", () => {
  it("draws the poll, and its author may delete it but not rewrite it", () => {
    renderWithConfirm(
      <MessageGroup
        id="m-1"
        authorId="u-1"
        authorName="Kenji"
        timestamp="2026-11-26T13:02:00Z"
        bodies={[{ id: "m-1", parts: [], poll }]}
        own
        onAction={vi.fn()}
        onReact={vi.fn()}
      />,
    );

    expect(screen.getByRole("heading", { name: "Kiedy gramy w ten weekend?" })).toBeInTheDocument();
    fireEvent.mouseOver(screen.getByText("Kiedy gramy w ten weekend?"));
    expect(screen.getByRole("button", { name: "Usuń" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Edytuj" })).toBeNull();
  });
});
