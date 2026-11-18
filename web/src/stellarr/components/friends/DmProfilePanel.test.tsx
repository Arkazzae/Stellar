import { screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { ConfirmProvider } from "@/components/primitives/ConfirmDialog";
import { answerConfirm, renderWithConfirm } from "@/test/confirm";

import { DmProfilePanel, type DmProfilePanelProps } from "./DmProfilePanel";

function setup(overrides: Partial<DmProfilePanelProps> = {}) {
  const props: DmProfilePanelProps = {
    open: true,
    person: { id: "u-maja", displayName: "Maja Nowak", initials: "MN", presence: "online" },
    username: "maja",
    customStatus: "w trasie do Gdańska",
    friendsSince: "12 marca 2026",
    relation: "friend",
    onClose: vi.fn(),
    onOpenProfile: vi.fn(),
    onRemoveFriend: vi.fn(),
    onBlock: vi.fn(),
    onUnblock: vi.fn(),
    ...overrides,
  };
  const view = renderWithConfirm(<DmProfilePanel {...props} />);
  return { props, user: userEvent.setup(), ...view };
}

describe("DmProfilePanel", () => {
  it("describes a friend, opens their full profile, and leaves the tab order once shut", async () => {
    const { user, props, rerender } = setup();

    const panel = screen.getByRole("complementary", { name: "Profil" });
    expect(panel).not.toHaveAttribute("inert");
    expect(within(panel).getByRole("heading", { level: 4, name: "Maja Nowak" })).toBeTruthy();
    expect(within(panel).getByText("@maja")).toBeTruthy();
    expect(within(panel).getByText("w trasie do Gdańska")).toBeTruthy();
    expect(within(panel).getByText("Znajomi od")).toBeTruthy();
    expect(within(panel).getByText("12 marca 2026")).toBeTruthy();

    const full = within(panel).getByRole("button", { name: "Pełny profil" });
    await user.click(full);
    expect(props.onOpenProfile).toHaveBeenCalledWith(full);

    await user.click(within(panel).getByRole("button", { name: "Zamknij profil" }));
    expect(props.onClose).toHaveBeenCalledOnce();

    rerender(
      <ConfirmProvider>
        <DmProfilePanel {...props} open={false} />
      </ConfirmProvider>,
    );
    expect(screen.getByRole("complementary", { hidden: true })).toHaveAttribute("inert");
  });

  it("removes the friend and blocks only once each question is answered yes", async () => {
    const { user, props } = setup();

    await user.click(screen.getByRole("button", { name: "Usuń ze znajomych" }));
    expect(props.onRemoveFriend).not.toHaveBeenCalled();
    await answerConfirm(user, "Usuń");
    expect(props.onRemoveFriend).toHaveBeenCalledOnce();

    await user.click(screen.getByRole("button", { name: "Zablokuj" }));
    await answerConfirm(user, "Zablokuj");
    expect(props.onBlock).toHaveBeenCalledOnce();
  });

  it("offers nothing about a friendship that is not there, and unblocks the blocked", async () => {
    const { user, props } = setup({ relation: "blocked", friendsSince: undefined, customStatus: undefined });

    expect(screen.queryByText("Znajomi od")).toBeNull();
    expect(screen.queryByRole("button", { name: "Usuń ze znajomych" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Zablokuj" })).toBeNull();

    await user.click(screen.getByRole("button", { name: "Odblokuj" }));
    expect(props.onUnblock).toHaveBeenCalledOnce();
  });
});
