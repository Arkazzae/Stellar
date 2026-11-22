import { fireEvent, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";

import { answerConfirm, renderWithConfirm } from "@/test/confirm";

import { MessageEntry, type MessageEntryProps } from "./MessageEntry";

const picker = () => screen.queryByRole("dialog", { name: "Wybierz reakcję" });
const pickerTrigger = () => screen.getByRole("button", { name: "Wybierz reakcję" });
const sheet = () => screen.queryByRole("dialog", { name: "Akcje wiadomości" });
const desktopWidth = window.innerWidth;

afterEach(() => {
  vi.restoreAllMocks();
  document.body.style.overflow = "";
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

/** The sheet is the phone's way to a message's actions. */
function onPhone() {
  window.innerWidth = 390;
}

describe("MessageToolbar reaction picker", () => {
  it("reacts with the shortcode when the pick is a server emoji", async () => {
    const user = userEvent.setup();
    const onReact = vi.fn();
    renderMessage({
      customEmoji: [{ id: "e-1", shortcode: ":mushris:", glyph: "https://cdn.example/m.png" }],
      onReact,
      onAction: vi.fn(),
    });
    pointAtMessage();

    await user.click(pickerTrigger());
    await user.type(screen.getByRole("textbox", { name: "Szukaj emoji" }), "mushr");
    await user.click(screen.getByRole("option", { name: ":mushris:" }));

    expect(onReact).toHaveBeenCalledExactlyOnceWith(":mushris:");
  });

  it("closes on Escape and hands focus back to the button that opened it", async () => {
    const user = userEvent.setup();
    renderMessage({ onReact: vi.fn(), onAction: vi.fn() });
    pointAtMessage();

    await user.click(pickerTrigger());
    await waitFor(() =>
      expect(screen.getByRole("textbox", { name: "Szukaj emoji" })).toHaveFocus(),
    );
    await user.keyboard("{Escape}");

    expect(picker()).toBeNull();
    expect(pickerTrigger()).toHaveFocus();
  });

  it("closes on Escape pressed outside the search field too", async () => {
    const user = userEvent.setup();
    renderMessage({ onReact: vi.fn(), onAction: vi.fn() });
    pointAtMessage();

    await user.click(pickerTrigger());
    screen.getByRole("textbox", { name: "Szukaj emoji" }).blur();
    await user.keyboard("{Escape}");

    expect(picker()).toBeNull();
    expect(pickerTrigger()).toHaveFocus();
  });

  it("closes when the same button is pressed again", async () => {
    const user = userEvent.setup();
    renderMessage({ onReact: vi.fn(), onAction: vi.fn() });
    pointAtMessage();

    await user.click(pickerTrigger());
    expect(pickerTrigger()).toHaveAttribute("aria-expanded", "true");
    await user.click(pickerTrigger());

    expect(picker()).toBeNull();
    expect(pickerTrigger()).toHaveAttribute("aria-expanded", "false");
  });

  it("closes when something outside it is pressed", async () => {
    const user = userEvent.setup();
    renderWithConfirm(
      <>
        <p>elsewhere</p>
        <MessageEntry actions={{ onReact: vi.fn(), onAction: vi.fn() }}>
          <p>wiadomość</p>
        </MessageEntry>
      </>,
    );
    pointAtMessage();

    await user.click(pickerTrigger());
    await user.click(screen.getByText("elsewhere"));

    expect(picker()).toBeNull();
  });

  it("stays open while its own panel is pressed", async () => {
    const user = userEvent.setup();
    renderMessage({ onReact: vi.fn(), onAction: vi.fn() });
    pointAtMessage();

    await user.click(pickerTrigger());
    await user.click(screen.getByRole("textbox", { name: "Szukaj emoji" }));

    expect(picker()).not.toBeNull();
  });

  it("keeps its message's toolbar while the pointer is on the picker instead", async () => {
    const user = userEvent.setup();
    const onReact = vi.fn();
    renderMessage({ onReact, onAction: vi.fn() });
    pointAtMessage();

    await user.click(pickerTrigger());
    // The picker is portaled outside the message, so the pointer leaves it.
    await user.hover(screen.getByRole("textbox", { name: "Szukaj emoji" }));
    await new Promise((resolve) => setTimeout(resolve, 500));

    expect(screen.getByRole("toolbar", { name: "Akcje wiadomości" })).toBeInTheDocument();
    expect(pickerTrigger()).toHaveAttribute("aria-expanded", "true");
    await user.click(screen.getAllByRole("option")[0] as HTMLElement);
    expect(onReact).toHaveBeenCalledOnce();
  });

  it("opens above the message when there is little room below it", async () => {
    const user = userEvent.setup();
    renderMessage({ onReact: vi.fn(), onAction: vi.fn() });
    pointAtMessage();
    vi.spyOn(screen.getByRole("toolbar"), "getBoundingClientRect").mockReturnValue(
      new DOMRect(900, window.innerHeight - 100, 100, 30),
    );

    await user.click(pickerTrigger());

    expect(picker()).toHaveClass("up");
    expect(picker()).toHaveStyle({ bottom: `${100 + 6}px` });
  });

  it("opens below the message when there is room", async () => {
    const user = userEvent.setup();
    renderMessage({ onReact: vi.fn(), onAction: vi.fn() });
    pointAtMessage();
    vi.spyOn(screen.getByRole("toolbar"), "getBoundingClientRect").mockReturnValue(
      new DOMRect(900, 20, 100, 30),
    );

    await user.click(pickerTrigger());

    expect(picker()).not.toHaveClass("up");
    expect(picker()).toHaveStyle({ top: "56px" });
  });
});

describe("MessageToolbar actions", () => {
  it("offers the thread to anyone", async () => {
    const user = userEvent.setup();
    const onAction = vi.fn();
    renderMessage({ onReact: vi.fn(), onAction });
    pointAtMessage();

    await user.click(screen.getByRole("button", { name: "Otwórz wątek" }));

    expect(onAction).toHaveBeenCalledExactlyOnceWith("thread");
  });

  it("does nothing on a press when no handler is given", async () => {
    const user = userEvent.setup();
    renderMessage({ own: true });
    pointAtMessage();

    await user.click(screen.getByRole("button", { name: "Usuń" }));

    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("marks the pin as pressed only when the message is pinned", () => {
    const { unmount } = renderMessage({ canModerate: true, onAction: vi.fn() });
    pointAtMessage();
    expect(screen.getByRole("button", { name: "Przypnij" })).toHaveAttribute(
      "aria-pressed",
      "false",
    );
    unmount();

    renderMessage({ canModerate: true, pinned: true, onAction: vi.fn() });
    pointAtMessage();
    expect(screen.getByRole("button", { name: "Odepnij" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
  });

  it("has a toolbar in the document only while its message is pointed at", async () => {
    renderMessage({ own: true, onReact: vi.fn(), onAction: vi.fn() });

    expect(screen.queryByRole("toolbar")).toBeNull();
    expect(screen.queryByRole("button", { name: "Odpowiedz" })).toBeNull();
    // What stands for the actions meanwhile, for the keyboard and for a
    // screen reader reading the transcript.
    expect(screen.getByRole("button", { name: "Akcje wiadomości" })).toBeInTheDocument();

    pointAtMessage();
    expect(screen.getByRole("toolbar", { name: "Akcje wiadomości" })).toBeInTheDocument();
    fireEvent.mouseOut(screen.getByText("wiadomość"), { relatedTarget: document.body });
    await waitFor(() => expect(screen.queryByRole("toolbar")).toBeNull());
  });
});

describe("MessageToolbar keyboard", () => {
  function renderTwo() {
    return renderWithConfirm(
      <>
        <button type="button">przed</button>
        <MessageEntry actions={{ own: true, onReact: vi.fn(), onAction: vi.fn() }}>
          <p>pierwsza</p>
        </MessageEntry>
        <MessageEntry actions={{ onReact: vi.fn(), onAction: vi.fn() }}>
          <p>druga</p>
        </MessageEntry>
        <button type="button">po</button>
      </>,
    );
  }

  const focusedStop = () => {
    const element = document.activeElement as HTMLElement;
    const entry = element.closest(".msg-entry");
    const where = entry === null ? "" : `${entry.textContent?.includes("pierwsza") ? 1 : 2}:`;
    return `${where}${element.getAttribute("aria-label") ?? element.textContent}`;
  };

  it("meets every action of each message in order, and the same stops backwards", async () => {
    const user = userEvent.setup();
    const { unmount } = renderTwo();
    screen.getByRole("button", { name: "przed" }).focus();

    const forward = [focusedStop()];
    for (let stop = 0; stop < 9; stop += 1) {
      await user.tab();
      forward.push(focusedStop());
    }
    expect(forward).toEqual([
      "przed",
      "1:Wybierz reakcję",
      "1:Odpowiedz",
      "1:Otwórz wątek",
      "1:Edytuj",
      "1:Usuń",
      "2:Wybierz reakcję",
      "2:Odpowiedz",
      "2:Otwórz wątek",
      "po",
    ]);
    unmount();

    // Afresh, so that no toolbar is left from the way down: Shift+Tab has to
    // find each message's last action from below.
    renderTwo();
    screen.getByRole("button", { name: "po" }).focus();
    const backward = [focusedStop()];
    for (let stop = 0; stop < 9; stop += 1) {
      await user.tab({ shift: true });
      backward.push(focusedStop());
    }
    expect(backward).toEqual([...forward].reverse());
  });
});

describe("MessageToolbar action sheet", () => {
  it("asks before deleting from the sheet, as the toolbar does", async () => {
    onPhone();
    const user = userEvent.setup();
    const onAction = vi.fn();
    renderMessage({ own: true, onAction });

    await user.click(screen.getByRole("button", { name: "Akcje wiadomości" }));
    await user.click(within(sheet() as HTMLElement).getByRole("button", { name: "Usuń" }));
    expect(onAction).not.toHaveBeenCalled();
    await answerConfirm(user, "Usuń");

    expect(onAction).toHaveBeenCalledExactlyOnceWith("delete");
  });

  it("names the pin by its direction and reports its state", async () => {
    onPhone();
    const user = userEvent.setup();
    renderMessage({ canModerate: true, pinned: true, onAction: vi.fn() });

    await user.click(screen.getByRole("button", { name: "Akcje wiadomości" }));

    expect(
      within(sheet() as HTMLElement).getByRole("button", { name: "Odepnij" }),
    ).toHaveAttribute("aria-pressed", "true");
  });

  it("locks page scroll while open and gives it back when it closes", async () => {
    onPhone();
    const user = userEvent.setup();
    document.body.style.overflow = "auto";
    renderMessage({ onAction: vi.fn() });

    await user.click(screen.getByRole("button", { name: "Akcje wiadomości" }));
    expect(document.body.style.overflow).toBe("hidden");
    await user.click(within(sheet() as HTMLElement).getByRole("button", { name: "Zamknij" }));

    expect(sheet()).toBeNull();
    expect(document.body.style.overflow).toBe("auto");
  });

  it("closes on Escape and returns focus to the button that opened it", async () => {
    onPhone();
    const user = userEvent.setup();
    renderMessage({ onAction: vi.fn() });
    const more = screen.getByRole("button", { name: "Akcje wiadomości" });

    await user.click(more);
    await user.keyboard("{Escape}");

    expect(sheet()).toBeNull();
    expect(more).toHaveFocus();
  });

  it("closes when the layer behind the sheet is pressed", async () => {
    onPhone();
    const user = userEvent.setup();
    renderMessage({ onAction: vi.fn() });

    await user.click(screen.getByRole("button", { name: "Akcje wiadomości" }));
    const layer = (sheet() as HTMLElement).parentElement as HTMLElement;
    fireEvent.pointerDown(within(layer).getByRole("heading"));
    expect(sheet()).not.toBeNull();
    fireEvent.pointerDown(layer);

    expect(sheet()).toBeNull();
  });

  it("keeps Tab inside the sheet", async () => {
    onPhone();
    const user = userEvent.setup();
    renderMessage({ own: true, onAction: vi.fn() });

    await user.click(screen.getByRole("button", { name: "Akcje wiadomości" }));
    const buttons = within(sheet() as HTMLElement).getAllByRole("button");
    // The sheet takes focus a frame after it opens; wait for that before moving it.
    await waitFor(() => expect(buttons[0]).toHaveFocus());
    (buttons.at(-1) as HTMLElement).focus();
    await user.tab();

    expect(buttons[0]).toHaveFocus();
  });
});
