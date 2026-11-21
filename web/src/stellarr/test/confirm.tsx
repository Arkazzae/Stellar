import { render, screen, within, type RenderResult } from "@testing-library/react";
import type { UserEvent } from "@testing-library/user-event";
import type { ReactElement } from "react";

import { ConfirmProvider } from "@/components/primitives/ConfirmDialog";

/**
 * Renders under the provider that owns the app's confirm dialog.
 *
 * Anything that asks a question — a bin, a "Zablokuj", a delete row — calls
 * `useConfirm`, and that hook needs the provider the app mounts at its root.
 */
export function renderWithConfirm(ui: ReactElement): RenderResult {
  return render(<ConfirmProvider>{ui}</ConfirmProvider>);
}

/**
 * The same provider as a wrapper, so a `rerender` with the bare element keeps
 * it: for a dialog that is rendered again with new props.
 */
export function renderInConfirm(ui: ReactElement): RenderResult {
  return render(ui, { wrapper: ConfirmProvider });
}

/** Answers the dialog by pressing one of its two buttons. */
export async function answerConfirm(user: UserEvent, label: string): Promise<void> {
  const dialog = await screen.findByRole("dialog");
  await user.click(within(dialog).getByRole("button", { name: label }));
}
