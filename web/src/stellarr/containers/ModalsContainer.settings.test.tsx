import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { createServerAdminAdapter } from "@/adapters/admin";
import { ConfirmProvider } from "@/components/primitives/ConfirmDialog";
import { translateMessage, type MessageKey } from "@/i18n";
import type { AdminApiClient } from "@/lib/api/client";
import { FRESH_SECOND_STEP_REQUIRED } from "@/containers/freshSecondStep";
import { ApiError } from "@/lib/api/errors";
import type { AdminMember, Invite, Role } from "@/ports";
import { fakeAdminPort } from "@/test/adminPort";
import { harness, resetHarness } from "@/test/modalsHarness";

import { ModalsContainer } from "./ModalsContainer";

vi.mock("@/state/hooks", async () => (await import("@/test/modalsHarness")).hooksModule());

const t = (key: MessageKey, values?: Record<string, string | number>) =>
  translateMessage("pl", key, values);

const MEMBER_ROLE: Role = {
  id: "role-member",
  name: "Member",
  color: "#8b7cf6",
  permissions: [],
  memberCount: 2,
  isSystem: false,
  isOwner: false,
};

const OWNER_ROLE: Role = {
  ...MEMBER_ROLE,
  id: "role-owner",
  name: "Owner",
  isSystem: true,
  isOwner: true,
};

const MAJA: AdminMember = {
  user: { id: "u-maja", username: "maja", displayName: "Maja Kowalska", presence: "online" },
  roleId: OWNER_ROLE.id,
  joinedAt: "2026-01-01T00:00:00.000Z",
};

const OLA: AdminMember = {
  user: { id: "u-ola", username: "ola", displayName: "Ola", presence: "idle" },
  roleId: MEMBER_ROLE.id,
  joinedAt: "2026-01-02T00:00:00.000Z",
};

const created: Invite = {
  id: "inv-1",
  url: "https://chat.example/invite/fresh",
  createdByName: "Maja",
  uses: 0,
  maxUses: null,
  expiresAt: null,
};

function openSettings(pane?: MessageKey) {
  harness.ui.modal = "settings";
  render(
    <ConfirmProvider>
      <ModalsContainer />
    </ConfirmProvider>,
  );
  return pane === undefined
    ? Promise.resolve()
    : userEvent.setup().click(screen.getByRole("button", { name: t(pane) }));
}

beforeEach(resetHarness);
afterEach(() => vi.restoreAllMocks());

describe("ModalsContainer: server settings", () => {
  it("stays shut for somebody who may not manage anything on the server", () => {
    harness.directory.canAccessServerSettings = false;

    openSettings();

    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(harness.ports.admin.listRoles).not.toHaveBeenCalled();
  });

  it("tells a member who may manage only roles why the overview is read-only", () => {
    const noop = vi.fn();
    const { supported } = createServerAdminAdapter(
      {} as AdminApiClient,
      () => ["manageRoles"],
      noop,
      noop,
      noop,
      noop,
      noop,
      noop,
      noop,
      noop,
    );
    harness.ports.admin = fakeAdminPort({ supported });

    openSettings();

    expect(
      screen.getByText("Nie masz uprawnienia do zarządzania serwerem. Ustawienia są tylko do odczytu."),
    ).toBeVisible();
    expect(screen.queryByText(/backend/i)).not.toBeInTheDocument();
  });

  it("shows the roles it fetched and refetches them after a role is edited", async () => {
    const user = userEvent.setup();
    const listRoles = vi
      .fn<() => Promise<Role[]>>()
      .mockResolvedValueOnce([MEMBER_ROLE])
      .mockResolvedValueOnce([{ ...MEMBER_ROLE, name: "Regular" }]);
    harness.ports.admin = fakeAdminPort({ listRoles, updateRole: vi.fn(async () => MEMBER_ROLE) });
    await openSettings("srv.nav.roles");

    await user.click(await screen.findByRole("button", { name: t("srv.roles.edit", { name: "Member" }) }));
    const name = screen.getByRole("textbox", { name: /^Nazwa/ });
    await user.clear(name);
    await user.type(name, "Regular");
    await user.click(screen.getByRole("button", { name: t("common.saveChanges") }));

    await waitFor(() =>
      expect(harness.ports.admin.updateRole).toHaveBeenCalledExactlyOnceWith("role-member", {
        name: "Regular",
        color: "#8b7cf6",
        permissions: [],
      }),
    );
    await waitFor(() => expect(listRoles).toHaveBeenCalledTimes(2));
    expect(await screen.findByRole("textbox", { name: /^Nazwa/ })).toHaveValue("Regular");
  });

  it("neither refetches nor changes the list when a role edit is refused", async () => {
    const user = userEvent.setup();
    const listRoles = vi.fn(async () => [MEMBER_ROLE]);
    harness.ports.admin = fakeAdminPort({
      listRoles,
      updateRole: vi.fn(async () => {
        throw new ApiError("forbidden", "no", 403);
      }),
    });
    await openSettings("srv.nav.roles");

    await user.click(await screen.findByRole("button", { name: t("srv.roles.edit", { name: "Member" }) }));
    await user.type(screen.getByRole("textbox", { name: /^Nazwa/ }), " 2");
    await user.click(screen.getByRole("button", { name: t("common.saveChanges") }));

    expect(await screen.findByRole("alert")).toHaveTextContent(t("settings.roleActionFailed"));
    expect(listRoles).toHaveBeenCalledTimes(1);
    expect(screen.getByRole("textbox", { name: /^Nazwa/ })).toHaveValue("Member 2");
  });

  it("keeps a role draft unsaved when its fresh second step is declined", async () => {
    const user = userEvent.setup();
    vi.spyOn(window, "confirm").mockReturnValue(false);
    harness.ports.admin = fakeAdminPort({
      listRoles: vi.fn(async () => [MEMBER_ROLE]),
      updateRole: vi.fn(async () => {
        throw new ApiError("forbidden", "fresh", 403, { code: FRESH_SECOND_STEP_REQUIRED });
      }),
    });
    await openSettings("srv.nav.roles");

    await user.click(await screen.findByRole("button", { name: t("srv.roles.edit", { name: "Member" }) }));
    await user.type(screen.getByRole("textbox", { name: /^Nazwa/ }), " 2");
    await user.click(screen.getByRole("button", { name: t("common.saveChanges") }));

    expect(await screen.findByRole("alert")).toHaveTextContent(t("settings.roleActionFailed"));
    expect(screen.getByRole("region", { name: t("settings.unsaved.region") })).not.toHaveTextContent(t("common.saved"));
    expect(screen.getByRole("button", { name: t("common.saveChanges") })).toBeVisible();
  });

  it("frees the pane as soon as the mutation lands, not when the lists are back", async () => {
    const user = userEvent.setup();
    const listInvites = vi
      .fn<() => Promise<Invite[]>>()
      .mockResolvedValueOnce([])
      .mockReturnValueOnce(new Promise(() => {}));
    harness.ports.admin = fakeAdminPort({
      listInvites,
      createInvite: vi.fn(async () => created),
    });
    await openSettings("srv.nav.invites");

    const create = await screen.findByRole("button", { name: t("settings.newInvite") });
    await user.click(create);

    await waitFor(() => expect(listInvites).toHaveBeenCalledTimes(2));
    expect(harness.ports.admin.createInvite).toHaveBeenCalledOnce();
    await waitFor(() => expect(create).toBeEnabled());
  });

  it("shows a created invite once the list has been refetched", async () => {
    const user = userEvent.setup();
    let invites: Invite[] = [];
    harness.ports.admin = fakeAdminPort({
      listInvites: vi.fn(async () => invites),
      createInvite: vi.fn(async () => {
        invites = [created];
        return created;
      }),
    });
    await openSettings("srv.nav.invites");

    await user.click(await screen.findByRole("button", { name: t("settings.newInvite") }));

    expect(await screen.findByText("invite/fresh")).toBeVisible();
  });

  it("lists the sections with handles to drag them by", async () => {
    harness.directory.sections = [
      { id: "s-a", name: "Alpha" },
      { id: "s-b", name: "Beta" },
    ];
    await openSettings("srv.nav.channels");

    expect(screen.getByRole("button", { name: t("srv.channels.moveSection", { name: "Alpha" }) })).toBeVisible();
    expect(screen.getByRole("button", { name: t("srv.channels.moveSection", { name: "Beta" }) })).toBeVisible();
  });

  describe("transferring ownership", () => {
    beforeEach(() => {
      harness.ports.admin = fakeAdminPort({
        listMembers: vi.fn(async () => [MAJA, OLA]),
        listRoles: vi.fn(async () => [OWNER_ROLE, MEMBER_ROLE]),
      });
      (harness.session as unknown as { user: { id: string } }).user = { id: MAJA.user.id };
    });

    it("hands the server to the member picked, and reloads", async () => {
      const user = userEvent.setup();
      await openSettings();

      await user.click(await screen.findByRole("button", { name: t("srv.transfer.button") }));
      const card = screen.getByRole("dialog", { name: t("srv.transfer.title") });
      await user.click(within(card).getByRole("combobox", { name: t("srv.transfer.pick") }));
      await user.click(screen.getByRole("option", { name: /Ola/ }));
      await user.click(within(card).getByRole("button", { name: t("srv.transfer.confirm") }));

      await waitFor(() => expect(harness.ports.admin.transferOwnership).toHaveBeenCalledExactlyOnceWith("u-ola"));
      await waitFor(() => expect(harness.ports.admin.listMembers).toHaveBeenCalledTimes(2));
    });

    it("is not offered to someone who is not the owner", async () => {
      (harness.session as unknown as { user: { id: string } }).user = { id: OLA.user.id };
      await openSettings();

      await screen.findByText(t("srv.ownerIs", { name: MAJA.user.displayName }));
      expect(screen.queryByRole("button", { name: t("srv.transfer.button") })).toBeNull();
      expect(screen.queryByRole("button", { name: t("srv.delete.button") })).toBeNull();
    });

    it("deletes the server only when its exact name is typed", async () => {
      const user = userEvent.setup();
      await openSettings();

      await user.click(await screen.findByRole("button", { name: t("srv.delete.button") }));
      const question = screen.getByRole("dialog", { name: t("srv.delete.title", { name: "Orbit" }) });
      await user.type(within(question).getByRole("textbox"), "orbit");

      expect(within(question).getByRole("button", { name: t("srv.delete.button") })).toBeDisabled();
      expect(harness.ports.admin.deleteServer).not.toHaveBeenCalled();
    });
  });
});
