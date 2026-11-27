import { fireEvent, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";

import { forgetQuickReactionOrder } from "@/lib/emoji/quickReactions";
import { answerConfirm, renderWithConfirm } from "@/test/confirm";

import { MessageEntry, type MessageEntryProps } from "./MessageEntry";

const desktopWidth = window.innerWidth;

afterEach(() => {
  window.innerWidth = desktopWidth;
  // A pick in the picker is a recent, and recents fill the quick row.
  localStorage.clear();
  forgetQuickReactionOrder();
});

/** One message with its actions, as a block draws it. */
function renderMessage(actions: MessageEntryProps["actions"] = {}) {
  return renderWithConfirm(
    <MessageEntry actions={actions}>
      <p>wiadomość</p>
    </MessageEntry>,
  );
}

/** The toolbar is there while the pointer is on its message. */
const pointAtMessage = () => fireEvent.mouseOver(screen.getByText("wiadomość"));

describe("MessageToolbar", () => {
  it("offers no edit and no bin on somebody else's message", () => {
    renderMessage({ onReact: vi.fn(), onAction: vi.fn() });
    pointAtMessage();

    expect(screen.getByRole("button", { name: "Odpowiedz" })).toBeInTheDocument();
    // Absent rather than disabled: you are never going to be allowed to.
    expect(screen.queryByRole("button", { name: "Edytuj" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Usuń" })).toBeNull();
  });

  it("gives the author both", () => {
    renderMessage({ own: true, onReact: vi.fn(), onAction: vi.fn() });
    pointAtMessage();

    expect(screen.getByRole("button", { name: "Edytuj" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Usuń" })).toBeInTheDocument();
  });

  it("gives a moderator the bin but never the pencil", () => {
    renderMessage({ canModerate: true, onReact: vi.fn(), onAction: vi.fn() });
    pointAtMessage();

    expect(screen.getByRole("button", { name: "Usuń" })).toBeInTheDocument();
    // Moderating is removing a message, not rewriting it under its author's
    // name — the one thing no permission should buy.
    expect(screen.queryByRole("button", { name: "Edytuj" })).toBeNull();
  });

  it("offers the pin to a moderator and to nobody else", () => {
    const { unmount } = renderMessage({ own: true, onReact: vi.fn(), onAction: vi.fn() });
    pointAtMessage();
    // Your own message is not yours to pin: a pin puts it above the channel
    // for everybody, which is moderation rather than a bookmark.
    expect(screen.queryByRole("button", { name: "Przypnij" })).toBeNull();
    unmount();

    renderMessage({ canModerate: true, onReact: vi.fn(), onAction: vi.fn() });
    pointAtMessage();
    expect(screen.getByRole("button", { name: "Przypnij" })).toBeInTheDocument();
  });

  it("turns the same button into the one that unpins", async () => {
    const user = userEvent.setup();
    const onAction = vi.fn();
    renderMessage({ canModerate: true, pinned: true, onReact: vi.fn(), onAction });
    await user.hover(screen.getByText("wiadomość"));

    expect(screen.queryByRole("button", { name: "Przypnij" })).toBeNull();
    const unpin = screen.getByRole("button", { name: "Odepnij" });
    expect(unpin).toHaveAttribute("aria-pressed", "true");

    // One action either way: what it means is read off the message, so the
    // handler never has to be told which direction was intended.
    await user.click(unpin);
    expect(onAction).toHaveBeenCalledExactlyOnceWith("pin");
  });

  it("deletes nothing until the question is answered", async () => {
    const user = userEvent.setup();
    const onAction = vi.fn();
    renderMessage({ own: true, onReact: vi.fn(), onAction });
    await user.hover(screen.getByText("wiadomość"));

    // Scoped to the toolbar: the dialog's own button is called "Usuń" too, and
    // it is still on its way out while the second press happens.
    const bin = () =>
      within(screen.getByRole("toolbar")).getByRole("button", { name: "Usuń" });

    await user.click(bin());
    expect(onAction).not.toHaveBeenCalled();
    expect(screen.getByRole("dialog")).toHaveTextContent("Usunąć tę wiadomość?");

    await answerConfirm(user, "Anuluj");
    expect(onAction).not.toHaveBeenCalled();

    await user.hover(screen.getByText("wiadomość"));
    await user.click(bin());
    await answerConfirm(user, "Usuń");
    expect(onAction).toHaveBeenCalledWith("delete");
  });

  it("asks nothing before the reversible actions", async () => {
    const user = userEvent.setup();
    const onAction = vi.fn();
    renderMessage({ own: true, onReact: vi.fn(), onAction });
    await user.hover(screen.getByText("wiadomość"));

    await user.click(screen.getByRole("button", { name: "Odpowiedz" }));

    expect(onAction).toHaveBeenCalledWith("reply");
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("opens the labelled action sheet on a phone and runs an action", async () => {
    window.innerWidth = 390;
    const user = userEvent.setup();
    const onAction = vi.fn();
    renderMessage({ own: true, onReact: vi.fn(), onAction });

    await user.click(screen.getByRole("button", { name: "Akcje wiadomości" }));
    const sheet = screen.getByRole("dialog", { name: "Akcje wiadomości" });
    expect(within(sheet).getByRole("button", { name: "Wybierz reakcję" })).toBeEnabled();
    expect(within(sheet).getByRole("button", { name: "Edytuj" })).toBeEnabled();

    await user.click(within(sheet).getByRole("button", { name: "Odpowiedz" }));

    expect(onAction).toHaveBeenCalledExactlyOnceWith("reply");
    expect(screen.queryByRole("dialog", { name: "Akcje wiadomości" })).toBeNull();
  });

  it("opens the reaction picker from the phone's action sheet", async () => {
    window.innerWidth = 390;
    const user = userEvent.setup();
    renderMessage({ onReact: vi.fn(), onAction: vi.fn() });

    await user.click(screen.getByRole("button", { name: "Akcje wiadomości" }));
    const sheet = screen.getByRole("dialog", { name: "Akcje wiadomości" });
    await user.click(within(sheet).getByRole("button", { name: "Wybierz reakcję" }));

    expect(
      screen.getByRole("dialog", { name: "Wybierz reakcję" }),
    ).toBeInTheDocument();
    expect(screen.queryByRole("dialog", { name: "Akcje wiadomości" })).toBeNull();
  });

  it("reacts with anything through the picker", async () => {
    const user = userEvent.setup();
    const onReact = vi.fn();
    renderMessage({ onReact, onAction: vi.fn() });
    await user.hover(screen.getByText("wiadomość"));

    await user.click(screen.getByRole("button", { name: "Wybierz reakcję" }));
    await user.type(screen.getByRole("textbox", { name: "Szukaj emoji" }), "zaba");
    await user.click(screen.getByRole("option", { name: ":zaba:" }));

    expect(onReact).toHaveBeenCalledWith("🐸");
    // One reaction is one pick, so the panel closes behind it.
    expect(screen.queryByRole("dialog", { name: "Wybierz reakcję" })).toBeNull();
  });

  it("opens touch reactions without raising search and dismisses the clear layer", () => {
    renderMessage({ onReact: vi.fn(), onAction: vi.fn() });
    pointAtMessage();

    const trigger = screen.getByRole("button", { name: "Wybierz reakcję" });
    fireEvent.pointerDown(trigger, { pointerType: "touch" });
    fireEvent.click(trigger, { detail: 1 });

    expect(screen.getByRole("textbox", { name: "Szukaj emoji" })).not.toHaveFocus();
    const dialog = screen.getByRole("dialog", { name: "Wybierz reakcję" });
    fireEvent.pointerDown(dialog);

    expect(screen.queryByRole("dialog", { name: "Wybierz reakcję" })).toBeNull();
  });
});

describe("MessageToolbar quick reactions", () => {
  const quickButtons = () =>
    within(screen.getByRole("toolbar")).queryAllByRole("button", { name: /^Zareaguj: / });

  it("offers the most recent emoji before the picker, filled up with the usual three", () => {
    localStorage.setItem("stellarr.emoji.recent", JSON.stringify(["zaba", "kciuk_w_gore"]));
    renderMessage({ onReact: vi.fn(), onAction: vi.fn() });
    pointAtMessage();

    const buttons = within(screen.getByRole("toolbar")).getAllByRole("button");
    expect(buttons.slice(0, 4).map((button) => button.getAttribute("aria-label"))).toEqual([
      "Zareaguj: 🐸",
      "Zareaguj: 👍",
      "Zareaguj: ❤️",
      "Wybierz reakcję",
    ]);
    // Named on hover as the other buttons are, and plain buttons for the keyboard.
    expect(buttons[0]).toHaveAttribute("title", "Zareaguj: 🐸");
    expect(buttons[0]).toHaveAttribute("type", "button");
    expect(buttons[0]).toHaveClass("tb", "emo");
  });

  it("reacts in one press, as the chip does, and marks the ones already yours", async () => {
    const user = userEvent.setup();
    const onReact = vi.fn();
    renderMessage({
      onReact,
      onAction: vi.fn(),
      reactions: [
        { emoji: "👍", count: 2, mine: true },
        { emoji: "❤️", count: 1, mine: false },
      ],
    });
    await user.hover(screen.getByText("wiadomość"));

    const thumbs = screen.getByRole("button", { name: "Zareaguj: 👍" });
    expect(thumbs).toHaveAttribute("aria-pressed", "true");
    expect(thumbs).toHaveClass("on");
    expect(screen.getByRole("button", { name: "Zareaguj: ❤️" })).toHaveAttribute(
      "aria-pressed",
      "false",
    );

    // The same toggle either way; the handler reads off the message which.
    await user.click(thumbs);
    await user.click(screen.getByRole("button", { name: "Zareaguj: 😂" }));
    expect(onReact.mock.calls).toEqual([["👍"], ["😂"]]);
  });

  it("offers a recent server emoji while the server still has it", async () => {
    localStorage.setItem("stellarr.emoji.recent", JSON.stringify(["mushris"]));
    const user = userEvent.setup();
    const onReact = vi.fn();
    const { unmount } = renderMessage({
      customEmoji: [{ id: "e-1", shortcode: ":mushris:", glyph: "https://cdn.example/m.png" }],
      onReact,
      onAction: vi.fn(),
    });
    await user.hover(screen.getByText("wiadomość"));

    const mushris = screen.getByRole("button", { name: "Zareaguj: :mushris:" });
    expect(mushris.querySelector("img")).toHaveAttribute("src", "https://cdn.example/m.png");
    await user.click(mushris);
    expect(onReact).toHaveBeenCalledExactlyOnceWith(":mushris:");
    unmount();

    // Deleted from the server since: the usual three stand in.
    renderMessage({ onReact: vi.fn(), onAction: vi.fn() });
    pointAtMessage();
    expect(quickButtons().map((button) => button.getAttribute("aria-label"))).toEqual([
      "Zareaguj: 👍",
      "Zareaguj: ❤️",
      "Zareaguj: 😂",
    ]);
  });

  it("lets the toolbar go when the pointer leaves after a press", async () => {
    const user = userEvent.setup();
    renderMessage({ onReact: vi.fn(), onAction: vi.fn() });
    await user.hover(screen.getByText("wiadomość"));

    const thumbs = screen.getByRole("button", { name: "Zareaguj: 👍" });
    await user.click(thumbs);
    fireEvent.mouseOut(thumbs, { relatedTarget: document.body });

    await waitFor(() => expect(screen.queryByRole("toolbar")).toBeNull());
  });

  it("leaves them out where you cannot react, keeping the picker as it was", () => {
    const { unmount } = renderMessage({ onAction: vi.fn() });
    pointAtMessage();
    expect(quickButtons()).toEqual([]);
    expect(screen.getByRole("button", { name: "Wybierz reakcję" })).toBeInTheDocument();
    unmount();

    renderMessage({ canAddReactions: false, onReact: vi.fn(), onAction: vi.fn() });
    pointAtMessage();
    expect(quickButtons()).toEqual([]);
    expect(screen.getByRole("button", { name: "Wybierz reakcję" })).toBeInTheDocument();
  });
});
