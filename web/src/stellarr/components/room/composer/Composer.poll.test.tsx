import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { Composer, type ComposerProps } from "./Composer";

function setup(props: Partial<ComposerProps> = {}) {
  const user = userEvent.setup();
  const onPoll = vi.fn();
  render(
    <Composer
      channelName="general"
      chips={[]}
      onSubmit={vi.fn()}
      onRemoveChip={vi.fn()}
      onRetryChip={vi.fn()}
      onReorderChips={vi.fn()}
      onPoll={onPoll}
      {...props}
    />,
  );
  const field = screen.getByRole("combobox", { name: "Napisz na #general" });
  return { user, field, onPoll };
}

describe("Composer's poll command", () => {
  it("opens a new poll with the question typed after /ankieta", async () => {
    const { user, field, onPoll } = setup();

    await user.type(field, "/ankieta ");
    await user.type(screen.getByRole("textbox", { name: "pytanie" }), "Kiedy gramy w weekend?{Enter}");

    expect(onPoll).toHaveBeenCalledExactlyOnceWith("Kiedy gramy w weekend?");
    expect(field).toHaveValue("");
  });

  it("opens an empty one from the English name too", async () => {
    const { user, field, onPoll } = setup();

    await user.type(field, "/poll{Enter}");
    await user.keyboard("{Enter}");

    expect(onPoll).toHaveBeenCalledExactlyOnceWith("");
  });

  it("is not offered where no poll can be sent, as in a private conversation", async () => {
    const { user, field } = setup({ onPoll: undefined });

    await user.type(field, "/ank");

    expect(screen.queryByRole("option", { name: /^\/ankieta/ })).toBeNull();
  });
});
