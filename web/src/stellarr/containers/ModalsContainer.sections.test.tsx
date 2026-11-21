import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { translateMessage, type MessageKey } from "@/i18n";
import { ApiError } from "@/lib/api/errors";
import { fakeAdminPort } from "@/test/adminPort";
import { renderWithConfirm } from "@/test/confirm";
import { harness, resetHarness } from "@/test/modalsHarness";

import { ModalsContainer } from "./ModalsContainer";

vi.mock("@/state/hooks", async () => (await import("@/test/modalsHarness")).hooksModule());

const t = (key: MessageKey, values?: Record<string, string | number>) =>
  translateMessage("pl", key, values);

beforeEach(() => {
  resetHarness();
  harness.directory.sections = [{ id: "s-a", name: "Alpha" }];
});

function openSectionSettings() {
  harness.ui.modal = "section";
  harness.ui.sectionSettingsId = "s-a";
  return renderWithConfirm(<ModalsContainer />);
}

/** Presses delete at the foot of the dialog's navigation; the card it raises is left unanswered. */
async function pressDelete() {
  const user = userEvent.setup();
  await user.click(screen.getByRole("button", { name: t("section.delete") }));
  return user;
}

async function askToDelete() {
  const user = await pressDelete();
  const question = screen.getByRole("dialog", { name: t("confirm.deleteSectionTitle") });
  await user.click(within(question).getByRole("button", { name: t("section.delete") }));
  return question;
}

describe("ModalsContainer: section settings", () => {
  it("renames the section", async () => {
    const user = userEvent.setup();
    openSectionSettings();

    const name = screen.getByRole("textbox", { name: new RegExp(`^${t("section.name")}`) });
    await user.clear(name);
    await user.type(name, "Beta");
    await user.click(screen.getByRole("button", { name: t("common.saveChanges") }));

    expect(harness.ports.admin.updateSection).toHaveBeenCalledExactlyOnceWith("s-a", { name: "Beta" });
  });

  it("deletes the section and closes the dialog", async () => {
    openSectionSettings();

    await askToDelete();

    await waitFor(() => expect(harness.ui.closeModal).toHaveBeenCalledOnce());
    expect(harness.ports.admin.deleteSection).toHaveBeenCalledExactlyOnceWith("s-a");
  });

  it("says to move the channels first, without asking the server, when the directory has some under it", async () => {
    harness.directory.channels = [{ ...harness.directory.channels[0]!, sectionId: "s-a" }];
    openSectionSettings();

    await pressDelete();

    const blocked = screen.getByRole("dialog", { name: t("confirm.sectionBlockedTitle") });
    expect(blocked).toHaveTextContent(harness.directory.channels[0]!.name);
    expect(within(blocked).queryByRole("button", { name: t("section.delete") })).toBeNull();
    expect(harness.ports.admin.deleteSection).not.toHaveBeenCalled();
  });

  it("leads the server's refusal over channels to the same card", async () => {
    harness.ports.admin = fakeAdminPort({
      deleteSection: vi.fn(async () => {
        throw new ApiError("invalid", "has channels", 409);
      }),
    });
    openSectionSettings();

    await askToDelete();

    expect(await screen.findByRole("dialog", { name: t("confirm.sectionBlockedTitle") })).toBeVisible();
    expect(harness.ui.closeModal).not.toHaveBeenCalled();
  });

  it("keeps the card open and says so when the deletion fails for another cause", async () => {
    harness.ports.admin = fakeAdminPort({
      deleteSection: vi.fn(async () => {
        throw new ApiError("server", "boom", 500);
      }),
    });
    openSectionSettings();

    const question = await askToDelete();

    expect(await within(question).findByRole("alert")).toHaveTextContent(t("section.deleteFailed"));
    expect(screen.queryByRole("dialog", { name: t("confirm.sectionBlockedTitle") })).toBeNull();
  });
});

describe("ModalsContainer: new section", () => {
  async function createSection(name: string) {
    const user = userEvent.setup();
    harness.ui.modal = "section-new";
    render(<ModalsContainer />);
    await user.type(screen.getByRole("textbox", { name: new RegExp(`^${t("section.name")}`) }), name);
    await user.click(screen.getByRole("button", { name: t("section.new") }));
  }

  it("creates the section and closes", async () => {
    await createSection("  Games ");

    await waitFor(() => expect(harness.ui.closeModal).toHaveBeenCalledOnce());
    expect(harness.ports.admin.createSection).toHaveBeenCalledExactlyOnceWith({ name: "Games" });
  });

  it("words an API failure by its kind", async () => {
    harness.ports.admin = fakeAdminPort({
      createSection: vi.fn(async () => {
        throw new ApiError("rate-limited", "slow down", 429);
      }),
    });

    await createSection("Games");

    expect(await screen.findByText(t("session.rateLimited"))).toBeVisible();
    expect(harness.ui.closeModal).not.toHaveBeenCalled();
  });
});
