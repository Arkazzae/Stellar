import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { describe, expect, it, vi } from "vitest";

import { Modal } from "./Modal";

function Harness({
  exitMs,
  onClose,
  className = "act-modal",
}: {
  exitMs?: number;
  onClose?: () => void;
  className?: string;
}) {
  const [open, setOpen] = useState(false);
  return (
    <>
      <button type="button" onClick={() => setOpen(true)}>
        Open
      </button>
      <Modal
        open={open}
        onClose={() => {
          onClose?.();
          setOpen(false);
        }}
        labelledBy="title"
        className={className}
        exitMs={exitMs}
      >
        <h2 id="title">Settings</h2>
        <button type="button">First</button>
        <button type="button">Second</button>
        <button type="button" disabled>
          Unavailable
        </button>
        <button type="button">Last</button>
      </Modal>
    </>
  );
}

async function openDialog() {
  const user = userEvent.setup();
  await user.click(screen.getByRole("button", { name: "Open" }));
  return { user, dialog: screen.getByRole("dialog", { name: "Settings" }) };
}

describe("Modal", () => {
  it("is absent until opened, then named by its title", async () => {
    render(<Harness />);
    expect(screen.queryByRole("dialog")).toBeNull();

    const { dialog } = await openDialog();

    expect(dialog).toHaveAttribute("aria-modal", "true");
  });

  it("moves focus in on opening and gives it back on closing", async () => {
    render(<Harness />);
    const { user } = await openDialog();
    expect(screen.getByRole("button", { name: "First" })).toHaveFocus();

    await user.keyboard("{Escape}");

    expect(screen.queryByRole("dialog")).toBeNull();
    expect(screen.getByRole("button", { name: "Open" })).toHaveFocus();
  });

  it("keeps Tab and Shift+Tab inside, skipping disabled controls", async () => {
    render(<Harness />);
    const { user } = await openDialog();

    await user.tab();
    expect(screen.getByRole("button", { name: "Second" })).toHaveFocus();
    await user.tab();
    expect(screen.getByRole("button", { name: "Last" })).toHaveFocus();
    await user.tab();
    expect(screen.getByRole("button", { name: "First" })).toHaveFocus();
    await user.tab({ shift: true });
    expect(screen.getByRole("button", { name: "Last" })).toHaveFocus();
  });

  it("closes on Escape without letting a window listener act on it", async () => {
    const onClose = vi.fn();
    const onWindowKey = vi.fn();
    window.addEventListener("keydown", onWindowKey);
    try {
      render(<Harness onClose={onClose} />);
      const { user } = await openDialog();

      await user.keyboard("{Escape}");

      expect(onClose).toHaveBeenCalledOnce();
      expect(onWindowKey).not.toHaveBeenCalled();
    } finally {
      window.removeEventListener("keydown", onWindowKey);
    }
  });

  it("closes on a click on the backdrop but not inside the dialog", async () => {
    const onClose = vi.fn();
    render(<Harness onClose={onClose} />);
    const { user, dialog } = await openDialog();

    await user.click(screen.getByRole("button", { name: "Second" }));
    expect(onClose).not.toHaveBeenCalled();

    await user.click(dialog);
    expect(onClose).toHaveBeenCalledOnce();
  });

  it("does not close on a click on the backdrop that no press began", async () => {
    const onClose = vi.fn();
    render(<Harness onClose={onClose} />);
    const { dialog } = await openDialog();

    fireEvent.click(dialog);

    expect(onClose).not.toHaveBeenCalled();
    expect(dialog).toBeInTheDocument();
  });

  it("does not close when a press began inside the dialog and its click landed on the backdrop", async () => {
    const onClose = vi.fn();
    render(<Harness onClose={onClose} />);
    const { dialog } = await openDialog();

    fireEvent.pointerDown(screen.getByRole("button", { name: "Second" }));
    fireEvent.click(dialog);

    expect(onClose).not.toHaveBeenCalled();
    expect(dialog).toBeInTheDocument();
  });

  it("locks page scrolling while mounted and restores what was there", async () => {
    document.body.style.overflow = "scroll";
    try {
      render(<Harness />);
      const { user } = await openDialog();
      expect(document.body.style.overflow).toBe("hidden");

      await user.keyboard("{Escape}");

      expect(document.body.style.overflow).toBe("scroll");
    } finally {
      document.body.style.overflow = "";
    }
  });

  // The settings dialogs hide their wrapper until it has its open class. A
  // browser gives no focus to an element that is not rendered; jsdom does, so
  // the test teaches it that rule and the wrapper's stylesheet.
  it("takes the focus and holds Tab when its wrapper is hidden until it opens", async () => {
    const style = document.createElement("style");
    style.textContent = ".sv-wrap { display: none } .sv-wrap.open { display: flex }";
    document.head.append(style);
    const focus = HTMLElement.prototype.focus;
    const rendered = (element: Element | null): boolean =>
      element === null || (getComputedStyle(element).display !== "none" && rendered(element.parentElement));
    const focusIfRendered = vi
      .spyOn(HTMLElement.prototype, "focus")
      .mockImplementation(function (this: HTMLElement, options?: FocusOptions) {
        if (rendered(this)) focus.call(this, options);
      });
    try {
      render(<Harness className="sv-wrap" />);
      const user = userEvent.setup();
      // Not through openDialog: until it opens, the hidden dialog has no role to find.
      await user.click(screen.getByRole("button", { name: "Open" }));

      await waitFor(() => expect(screen.getByRole("button", { name: "First" })).toHaveFocus());
      await user.tab();
      await user.tab();
      await user.tab();
      expect(screen.getByRole("button", { name: "First" })).toHaveFocus();
      expect(screen.getByRole("button", { name: "Open" })).not.toHaveFocus();
    } finally {
      focusIfRendered.mockRestore();
      style.remove();
    }
  });

  it("leaves the focus with a layer raised from the dialog before its wrapper showed", async () => {
    const frames: FrameRequestCallback[] = [];
    const requestFrame = vi
      .spyOn(window, "requestAnimationFrame")
      .mockImplementation((callback) => frames.push(callback));
    render(
      <>
        <Harness />
        <button type="button">Menu item</button>
      </>,
    );
    await openDialog();
    // A menu raised from the dialog draws outside its element and takes the focus.
    const menuItem = screen.getByRole("button", { name: "Menu item" });
    act(() => menuItem.focus());

    act(() => frames.splice(0).forEach((callback) => callback(performance.now())));
    requestFrame.mockRestore();

    expect(menuItem).toHaveFocus();
  });

  it("applies the open class after mount and keeps the dialog for its exit", async () => {
    // Hold the entry frame, so that the closed state can be seen before it runs
    // whatever the speed of the machine.
    const frames: FrameRequestCallback[] = [];
    const requestFrame = vi
      .spyOn(window, "requestAnimationFrame")
      .mockImplementation((callback) => frames.push(callback));
    render(<Harness exitMs={40} />);
    const { user, dialog } = await openDialog();
    expect(dialog).not.toHaveClass("open");
    act(() => frames.splice(0).forEach((callback) => callback(performance.now())));
    expect(dialog).toHaveClass("open");
    requestFrame.mockRestore();

    await user.keyboard("{Escape}");

    // The exit transition needs the closed state while the dialog is still there.
    const leaving = screen.getByRole("dialog", { name: "Settings" });
    expect(leaving).not.toHaveClass("open");
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });
});
