import { screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import type { Role } from "@/ports";
import { renderInConfirm } from "@/test/confirm";

import { SectionSettings, type SectionSettingsProps } from "./SectionSettings";

const SECTION = { id: "sec-1", name: "Gry", channels: [] };

const MODERATOR: Role = {
  id: "mod",
  name: "Moderator",
  color: "#5865f2",
  permissions: [],
  memberCount: 3,
  isSystem: false,
  isOwner: false,
};

function props(overrides: Partial<SectionSettingsProps> = {}): SectionSettingsProps {
  return {
    open: true,
    onClose: vi.fn(),
    section: SECTION,
    supported: true,
    permissionsSupported: true,
    roles: [MODERATOR],
    permissionOverrides: [],
    onSave: vi.fn(() => Promise.resolve()),
    onSavePermissionOverride: vi.fn(() => Promise.resolve()),
    onDelete: vi.fn(async () => "deleted" as const),
    onOpenRoles: vi.fn(),
    ...overrides,
  };
}

const nameField = () => screen.getByRole("textbox", { name: /Nazwa sekcji/ });
const saveButton = () => screen.queryByRole("button", { name: /Zapisz zmiany|Zapisywanie…/ });
const goTo = (user: ReturnType<typeof userEvent.setup>, pane: string) =>
  user.click(
    within(screen.getByRole("navigation", { name: "Sekcje ustawień sekcji" })).getByRole(
      "button",
      { name: pane },
    ),
  );

describe("SectionSettings", () => {
  it("opens on the overview of the section it is named after", () => {
    renderInConfirm(<SectionSettings {...props()} />);

    expect(screen.getByRole("dialog", { name: "Edytuj sekcję Gry" })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Przegląd" })).toBeInTheDocument();
    expect(nameField()).toHaveValue("Gry");
  });

  describe("rename", () => {
    it("offers a save only for a new, non-empty name", async () => {
      const user = userEvent.setup();
      renderInConfirm(<SectionSettings {...props()} />);
      expect(saveButton()).toBeNull();

      await user.clear(nameField());
      expect(saveButton()).toBeDisabled();

      await user.type(nameField(), "  Gry  ");
      expect(saveButton()).toBeNull();

      await user.type(nameField(), "!");
      expect(saveButton()).toBeEnabled();
    });

    it("saves the trimmed name", async () => {
      const user = userEvent.setup();
      const onSave = vi.fn(() => Promise.resolve());
      renderInConfirm(<SectionSettings {...props({ onSave })} />);

      await user.clear(nameField());
      await user.type(nameField(), "  Ogłoszenia ");
      await user.click(saveButton()!);

      expect(onSave).toHaveBeenCalledExactlyOnceWith({ name: "Ogłoszenia" });
    });

    it("reports a failed save and keeps what was typed", async () => {
      const user = userEvent.setup();
      const onSave = vi.fn().mockRejectedValue(new Error("boom"));
      renderInConfirm(<SectionSettings {...props({ onSave })} />);

      await user.type(nameField(), "!");
      await user.click(saveButton()!);

      expect(await screen.findByRole("alert")).toHaveTextContent(
        "Nie udało się zapisać zmian sekcji.",
      );
      expect(nameField()).toHaveValue("Gry!");
    });

    it("takes the new name from the section once the rail has it", async () => {
      const user = userEvent.setup();
      const { rerender } = renderInConfirm(<SectionSettings {...props()} />);
      await user.type(nameField(), "!");

      rerender(<SectionSettings {...props({ section: { ...SECTION, name: "Gry!" } })} />);

      expect(nameField()).toHaveValue("Gry!");
      expect(saveButton()).toBeNull();
    });
  });

  describe("permissions", () => {
    it("edits a role's override through the shared editor", async () => {
      const user = userEvent.setup();
      const onSavePermissionOverride = vi.fn(() => Promise.resolve());
      renderInConfirm(<SectionSettings {...props({ onSavePermissionOverride })} />);
      await goTo(user, "Uprawnienia");

      await user.click(screen.getByRole("button", { name: "Edytuj uprawnienia roli Moderator" }));
      await user.click(
        within(screen.getByRole("radiogroup", { name: "Dziennik zdarzeń" })).getByRole("radio", {
          name: "Odmów",
        }),
      );
      await user.click(screen.getByRole("button", { name: "Zapisz" }));

      expect(onSavePermissionOverride).toHaveBeenCalledExactlyOnceWith("mod", {
        allow: [],
        deny: ["viewAuditLog"],
      });
    });

    it("keeps the editor read-only without permission to manage roles", async () => {
      const user = userEvent.setup();
      renderInConfirm(<SectionSettings {...props({ permissionsSupported: false })} />);
      await goTo(user, "Uprawnienia");

      expect(screen.getByText(/więc wyjątki są tylko do odczytu/)).toBeInTheDocument();
      expect(
        screen.getByRole("button", { name: "Edytuj uprawnienia roli Moderator" }),
      ).toBeDisabled();
    });

    it("says so when the server has no roles, and still links to them", async () => {
      const user = userEvent.setup();
      const onOpenRoles = vi.fn();
      renderInConfirm(<SectionSettings {...props({ roles: [], onOpenRoles })} />);
      await goTo(user, "Uprawnienia");

      expect(screen.getByText("Ten serwer nie ma jeszcze ról.")).toBeInTheDocument();
      await user.click(screen.getByRole("button", { name: "Otwórz role serwera" }));
      expect(onOpenRoles).toHaveBeenCalledOnce();
    });
  });

  it("is read-only and says why without permission to manage channels", () => {
    renderInConfirm(<SectionSettings {...props({ supported: false })} />);

    expect(screen.getByText(/Nie masz uprawnienia do zarządzania kanałami/)).toBeInTheDocument();
    expect(nameField()).toBeDisabled();
    expect(screen.getByRole("button", { name: "Usuń sekcję" })).toBeDisabled();
  });

  it("returns to the overview each time it opens", async () => {
    const user = userEvent.setup();
    const { rerender } = renderInConfirm(<SectionSettings {...props()} />);
    await goTo(user, "Uprawnienia");
    expect(screen.getByRole("heading", { name: "Uprawnienia" })).toBeInTheDocument();

    rerender(<SectionSettings {...props({ open: false })} />);
    rerender(<SectionSettings {...props()} />);

    expect(screen.getByRole("heading", { name: "Przegląd" })).toBeInTheDocument();
  });
});
