import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { formatDate } from "@/lib/format/format";
import type { User } from "@/ports";
import { answerConfirm, renderWithConfirm } from "@/test/confirm";
import { resetStubs, setStubs, stubs } from "@/test/hookStubs";

import { DmProfileContainer } from "./DmProfileContainer";

vi.mock("@/state/hooks", async () => (await import("@/test/hookStubs")).hooksModule);

const ADA: User = {
  id: "u-ada",
  username: "ada",
  displayName: "Ada",
  presence: "online",
  customStatus: "  w trasie  ",
};
const SINCE = "2026-03-12T12:00:00Z";

beforeEach(resetStubs);

describe("DmProfileContainer", () => {
  it("describes the person in the open conversation from the friends graph, and acts on them", async () => {
    setStubs({
      ui: { profilePanelOpen: true },
      friends: {
        active: { id: "dm-ada", peer: ADA, unread: 0, updatedAt: SINCE },
        friends: [{ user: ADA, since: SINCE }],
      },
    });
    const user = userEvent.setup();
    renderWithConfirm(<DmProfileContainer />);

    expect(screen.getByText("w trasie")).toBeInTheDocument();
    expect(screen.getByText(formatDate(SINCE, "pl"))).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Usuń ze znajomych" }));
    await answerConfirm(user, "Usuń");
    expect(stubs.friends.remove).toHaveBeenCalledWith("u-ada");

    await user.click(screen.getByRole("button", { name: "Zamknij profil" }));
    expect(stubs.ui.setProfilePanelOpen).toHaveBeenCalledWith(false);
  });
});
