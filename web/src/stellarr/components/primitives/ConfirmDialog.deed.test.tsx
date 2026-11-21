import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { ConfirmProvider, useConfirm, usePrompt, type Confirm, type Prompt } from "./ConfirmDialog";

/** Hands the provider's functions to the test so it can ask without a button. */
function captureAsking() {
  const asking: { confirm?: Confirm; prompt?: Prompt } = {};
  function Capture() {
    asking.confirm = useConfirm();
    asking.prompt = usePrompt();
    return null;
  }
  render(
    <ConfirmProvider>
      <Capture />
    </ConfirmProvider>,
  );
  return { confirm: asking.confirm!, prompt: asking.prompt! };
}

function deferred() {
  let resolve!: () => void;
  let reject!: (cause: unknown) => void;
  const promise = new Promise<void>((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
}

const request = {
  title: "Usunąć ten kanał?",
  confirmLabel: "Usuń kanał",
  busyLabel: "Usuwanie…",
  failure: "Nie udało się usunąć kanału. Spróbuj jeszcze raz.",
  danger: true,
};

describe("ConfirmProvider with the deed on the card", () => {
  it("stays open while the deed runs, cannot be dismissed, and closes once it succeeds", async () => {
    const user = userEvent.setup();
    const { confirm } = captureAsking();
    const deed = deferred();
    const run = vi.fn(() => deed.promise);
    const answer = vi.fn();

    void confirm({ ...request, run }).then(answer);
    const dialog = await screen.findByRole("dialog", { name: "Usunąć ten kanał?" });
    await user.click(within(dialog).getByRole("button", { name: "Usuń kanał" }));

    expect(run).toHaveBeenCalledOnce();
    expect(within(dialog).getByRole("button", { name: "Usuwanie…" })).toHaveAttribute("aria-busy", "true");
    expect(within(dialog).getByRole("button", { name: "Anuluj" })).toBeDisabled();
    await user.keyboard("{Escape}");
    expect(screen.getByRole("dialog")).toBeInTheDocument();
    expect(answer).not.toHaveBeenCalled();

    deed.resolve();
    await waitFor(() => expect(answer).toHaveBeenCalledWith(true));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });

  it("keeps the card open with the failure when the deed rejects, and lets it be tried again", async () => {
    const user = userEvent.setup();
    const { confirm } = captureAsking();
    const run = vi.fn().mockRejectedValueOnce(new Error("refused")).mockResolvedValueOnce(undefined);
    const answer = vi.fn();

    void confirm({ ...request, run }).then(answer);
    const dialog = await screen.findByRole("dialog");
    await user.click(within(dialog).getByRole("button", { name: "Usuń kanał" }));

    expect(await within(dialog).findByRole("alert")).toHaveTextContent(
      "Nie udało się usunąć kanału. Spróbuj jeszcze raz.",
    );
    expect(within(dialog).getByRole("button", { name: "Anuluj" })).toBeEnabled();
    expect(answer).not.toHaveBeenCalled();

    await user.click(within(dialog).getByRole("button", { name: "Usuń kanał" }));
    await waitFor(() => expect(answer).toHaveBeenCalledWith(true));
    expect(run).toHaveBeenCalledTimes(2);
  });

  it("passes the typed answer to the deed, and enables it only for the exact name", async () => {
    const user = userEvent.setup();
    const { prompt } = captureAsking();
    const run = vi.fn(async () => undefined);

    void prompt({
      ...request,
      title: "Usunąć ten serwer?",
      confirmLabel: "Usuń serwer",
      field: { label: "Wpisz nazwę serwera, aby potwierdzić", match: "Stellarr Dev" },
      run,
    });
    const dialog = await screen.findByRole("dialog");
    const field = within(dialog).getByRole("textbox", { name: "Wpisz nazwę serwera, aby potwierdzić" });
    const deed = within(dialog).getByRole("button", { name: "Usuń serwer" });

    await user.type(field, "Stellarr dev");
    expect(deed).toBeDisabled();
    await user.keyboard("{Enter}");
    expect(run).not.toHaveBeenCalled();

    await user.clear(field);
    await user.type(field, "Stellarr Dev");
    expect(deed).toBeEnabled();
    await user.click(deed);
    expect(run).toHaveBeenCalledWith("Stellarr Dev");
  });
});
