import { fireEvent, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";

import { answerConfirm, renderWithConfirm } from "@/test/confirm";

import { MessageEntry, type MessageEntryProps } from "./MessageEntry";

const desktopWidth = window.innerWidth;

afterEach(() => {
  window.innerWidth = desktopWidth;
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

    expect(screen.queryByRole("button", { name: /^Zareaguj / })).toBeNull();
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
