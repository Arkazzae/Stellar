import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ReactNode } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { ConfirmProvider } from "@/components/primitives/ConfirmDialog";
import type { AdminMember, Invite, Role } from "@/ports";

import { ChannelsPane, type ChannelsPaneProps } from "./ChannelsPane";
import { InvitesPane } from "./InvitesPane";
import { MembersPane, type MembersPaneProps } from "./MembersPane";
import { RolesPane, type RolesPaneProps } from "./RolesPane";

const withConfirm = (node: ReactNode) => <ConfirmProvider>{node}</ConfirmProvider>;

function role(overrides: Partial<Role> = {}): Role {
  return {
    id: "role-crew",
    name: "Ekipa",
    color: "#8b7cf6",
    permissions: ["manageChannels"],
    memberCount: 2,
    isSystem: false,
    isOwner: false,
    ...overrides,
  };
}

const OWNER = role({ id: "role-owner", name: "Właściciel", permissions: [], memberCount: 1, isSystem: true, isOwner: true });
const MEMBER = role({ id: "role-member", name: "Członek", permissions: [], memberCount: 3, isSystem: true });

const member = (id: string, name: string, roleId: string): AdminMember => ({
  user: { id, username: name.toLowerCase(), displayName: name, presence: "online" },
  roleId,
  joinedAt: new Date(2026, 0, 15, 12).toISOString(),
});

describe("ChannelsPane", () => {
  function pane(overrides: Partial<ChannelsPaneProps> = {}) {
    const props: ChannelsPaneProps = {
      supported: true,
      channels: [
        { id: "general", name: "ogolny", kind: "text" },
        { id: "lobby", name: "lobby", kind: "voice" },
        { id: "news", name: "nowosci", kind: "text", sectionId: "info" },
      ],
      onAdd: vi.fn(),
      onEdit: vi.fn(),
      sectionsSupported: true,
      sections: [
        { id: "info", name: "Informacje" },
        { id: "empty", name: "Pusta" },
      ],
      onAddSection: vi.fn(),
      onEditSection: vi.fn(),
      onMoveChannel: vi.fn().mockResolvedValue(undefined),
      onMoveSection: vi.fn().mockResolvedValue(undefined),
      ...overrides,
    };
    render(<ChannelsPane {...props} />);
    return props;
  }

  it("lists the channels without a section first, then each section with its own", () => {
    pane();

    const list = screen.getByRole("region", { name: "Kanały i sekcje" });
    const groups = list.querySelectorAll(".ss-group");
    expect(Array.from(groups, (group) => Array.from(group.querySelectorAll(".sec-copy b"), (b) => b.textContent))).toEqual([
      ["ogolny", "lobby"],
      ["Informacje", "nowosci"],
      ["Pusta"],
    ]);
    expect(within(groups[1] as HTMLElement).getByText("1 kanał")).toBeVisible();
    expect(within(groups[2] as HTMLElement).getByText("W tej sekcji nie ma jeszcze kanałów.")).toBeVisible();
  });

  it("gives every row a handle to drag it by, announced as the rail's are", () => {
    pane();

    const channel = screen.getByRole("button", { name: "Przenieś #ogolny" });
    const section = screen.getByRole("button", { name: "Przenieś sekcję Informacje" });
    expect(channel).toHaveAttribute("aria-roledescription", "kanał do przestawienia");
    expect(section).toHaveAttribute("aria-roledescription", "sekcja do przestawienia");
  });

  it("offers no handles and no edits without permission to manage channels", () => {
    pane({ supported: false, sectionsSupported: false });

    expect(screen.queryByRole("button", { name: /^Przenieś/ })).toBeNull();
    expect(screen.getByRole("button", { name: "Edytuj kanał ogolny" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Nowa sekcja" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Nowy kanał" })).toBeDisabled();
  });

  it("opens the editors and the dialogs that add", async () => {
    const user = userEvent.setup();
    const props = pane();

    await user.click(screen.getByRole("button", { name: "Edytuj kanał nowosci" }));
    await user.click(screen.getByRole("button", { name: "Edytuj sekcję Informacje" }));
    await user.click(screen.getByRole("button", { name: "Nowy kanał" }));
    await user.click(screen.getByRole("button", { name: "Nowa sekcja" }));

    expect(props.onEdit).toHaveBeenCalledWith("news");
    expect(props.onEditSection).toHaveBeenCalledWith("info");
    expect(props.onAdd).toHaveBeenCalledOnce();
    expect(props.onAddSection).toHaveBeenCalledOnce();
  });
});

describe("RolesPane", () => {
  function pane(overrides: Partial<RolesPaneProps> = {}) {
    const props: RolesPaneProps = {
      supported: true,
      roles: [OWNER, role(), MEMBER],
      members: [member("u-1", "Maja", OWNER.id), member("u-2", "Jan", "role-crew"), member("u-3", "Ola", "role-crew")],
      onCreate: vi.fn().mockResolvedValue(undefined),
      onUpdate: vi.fn().mockResolvedValue(undefined),
      onDelete: vi.fn().mockResolvedValue(undefined),
      ...overrides,
    };
    render(withConfirm(<RolesPane {...props} />));
    return props;
  }

  it("puts the default role on its own and the others in a table with how many hold each", () => {
    pane();

    const defaults = screen.getByRole("region", { name: "Rola domyślna" });
    expect(within(defaults).getByRole("button", { name: "Edytuj rolę Członek" })).toHaveTextContent("3");
    const table = screen.getByRole("region", { name: "Role" });
    const rows = within(table).getAllByRole("button", { name: /^Edytuj rolę/ });
    expect(rows.map((row) => row.getAttribute("aria-label"))).toEqual(["Edytuj rolę Właściciel", "Edytuj rolę Ekipa"]);
    expect(rows[1]).toHaveTextContent("2");
  });

  it("marks a role with a shield in its colour, and the owner's with a crown", () => {
    pane();

    const crew = screen.getByRole("button", { name: "Edytuj rolę Ekipa" }).querySelector(".ss-role-ico") as HTMLElement;
    expect(crew.style.getPropertyValue("--rc")).toBe("#8b7cf6");
  });

  it("opens a role in place of the list and saves the draft in one go", async () => {
    const user = userEvent.setup();
    const props = pane();

    await user.click(screen.getByRole("button", { name: "Edytuj rolę Ekipa" }));
    expect(screen.queryByRole("region", { name: "Rola domyślna" })).toBeNull();
    expect(screen.getByText("2 osoby z tą rolą.")).toBeVisible();

    await user.click(screen.getByRole("switch", { name: "Zarządzanie banami" }));
    await user.click(screen.getByRole("radio", { name: "Pomarańcz" }));
    expect(props.onUpdate).not.toHaveBeenCalled();
    await user.click(screen.getByRole("button", { name: "Zapisz zmiany" }));

    await waitFor(() =>
      expect(props.onUpdate).toHaveBeenCalledWith("role-crew", {
        name: "Ekipa",
        color: "#f76b15",
        permissions: ["manageChannels", "manageBans"],
      }),
    );
  });

  it("groups the permissions every member uses and saves them with the rest", async () => {
    const user = userEvent.setup();
    const everyday = ["sendMessages", "attachFiles", "addReactions", "connect"] as const;
    const props = pane({ roles: [OWNER, role(), { ...MEMBER, permissions: [...everyday] }] });

    await user.click(screen.getByRole("button", { name: "Edytuj rolę Członek" }));
    const text = screen.getByRole("group", { name: "Kanały tekstowe" });
    expect(within(text).getAllByRole("switch").map((item) => item.getAttribute("aria-label"))).toEqual([
      "Wysyłanie wiadomości",
      "Załączanie plików",
      "Dodawanie reakcji",
      "Oznaczanie wszystkich",
    ]);
    // Plain members do not call on the whole channel unless given it.
    expect(within(text).getByRole("switch", { name: "Oznaczanie wszystkich" })).not.toBeChecked();
    expect(within(screen.getByRole("group", { name: "Kanały głosowe" })).getByRole("switch")).toBeChecked();
    expect(
      within(screen.getByRole("group", { name: "Ludzie" })).getByRole("switch", { name: "Tworzenie zaproszeń" }),
    ).not.toBeChecked();
    expect(
      within(screen.getByRole("group", { name: "Zaawansowane" })).getByRole("switch", { name: "Administrator" }),
    ).not.toBeChecked();

    await user.click(screen.getByRole("switch", { name: "Załączanie plików" }));
    await user.click(screen.getByRole("switch", { name: "Tworzenie zaproszeń" }));
    await user.click(screen.getByRole("button", { name: "Zapisz zmiany" }));

    await waitFor(() =>
      expect(props.onUpdate).toHaveBeenCalledWith("role-member", {
        name: "Członek",
        color: "#8b7cf6",
        permissions: ["sendMessages", "addReactions", "connect", "createInvite"],
      }),
    );
  });

  it("gives calling on the whole channel with the message permissions, explained, and saves it", async () => {
    const user = userEvent.setup();
    const props = pane();

    await user.click(screen.getByRole("button", { name: "Edytuj rolę Ekipa" }));
    const text = screen.getByRole("group", { name: "Kanały tekstowe" });
    const mention = within(text).getByRole("switch", { name: "Oznaczanie wszystkich" });
    expect(mention).not.toBeChecked();
    expect(text).toHaveTextContent("przez @wszyscy");
    await user.click(mention);
    await user.click(screen.getByRole("button", { name: "Zapisz zmiany" }));

    await waitFor(() =>
      expect(props.onUpdate).toHaveBeenCalledWith("role-crew", {
        name: "Ekipa",
        color: "#8b7cf6",
        permissions: ["manageChannels", "mentionEveryone"],
      }),
    );
  });

  it("lists viewing channels in the Server group, after its other rows, and keeps it on save", async () => {
    const user = userEvent.setup();
    const everyday = ["viewChannel", "sendMessages", "attachFiles", "addReactions", "connect"] as const;
    const props = pane({ roles: [OWNER, role(), { ...MEMBER, permissions: [...everyday] }] });

    await user.click(screen.getByRole("button", { name: "Edytuj rolę Członek" }));
    const server = screen.getByRole("group", { name: "Serwer" });
    expect(within(server).getAllByRole("switch").map((item) => item.getAttribute("aria-label"))).toEqual([
      "Zarządzanie serwerem",
      "Zarządzanie kanałami",
      "Wyświetlanie kanałów",
    ]);
    expect(within(server).getByRole("switch", { name: "Wyświetlanie kanałów" })).toBeChecked();

    await user.click(screen.getByRole("switch", { name: "Załączanie plików" }));
    await user.click(screen.getByRole("button", { name: "Zapisz zmiany" }));

    await waitFor(() =>
      expect(props.onUpdate).toHaveBeenCalledWith("role-member", {
        name: "Członek",
        color: "#8b7cf6",
        permissions: ["viewChannel", "sendMessages", "addReactions", "connect"],
      }),
    );
  });

  it("puts the draft back on reset, and goes back to the list", async () => {
    const user = userEvent.setup();
    pane();

    await user.click(screen.getByRole("button", { name: "Edytuj rolę Ekipa" }));
    await user.click(screen.getByRole("switch", { name: "Zarządzanie kanałami" }));
    expect(screen.getByRole("switch", { name: "Zarządzanie kanałami" })).not.toBeChecked();
    await user.click(screen.getByRole("button", { name: "Przywróć" }));
    expect(screen.getByRole("switch", { name: "Zarządzanie kanałami" })).toBeChecked();

    await user.click(screen.getByRole("button", { name: "Wszystkie role" }));
    expect(screen.getByRole("region", { name: "Rola domyślna" })).toBeVisible();
  });

  it("deletes a role after asking, and says who falls back to the default role", async () => {
    const user = userEvent.setup();
    const props = pane();

    await user.click(screen.getByRole("button", { name: "Edytuj rolę Ekipa" }));
    await user.click(screen.getByRole("button", { name: "Usuń rolę" }));
    const question = screen.getByRole("dialog", { name: "Usunąć tę rolę?" });
    expect(question).toHaveTextContent("Osoby z tą rolą (2 osoby) dostaną rolę domyślną.");
    await user.click(within(question).getByRole("button", { name: "Usuń rolę" }));

    await waitFor(() => expect(props.onDelete).toHaveBeenCalledWith("role-crew"));
    await waitFor(() => expect(screen.getByRole("region", { name: "Rola domyślna" })).toBeVisible());
  });

  it("locks the owner role and keeps the default role from being deleted", async () => {
    const user = userEvent.setup();
    pane();

    await user.click(screen.getByRole("button", { name: "Edytuj rolę Właściciel" }));
    expect(screen.getByText(/Właściciel może wszystko/)).toBeVisible();
    expect(screen.getByRole("switch", { name: "Zarządzanie banami" })).toBeChecked();
    expect(screen.getByRole("switch", { name: "Zarządzanie banami" })).toBeDisabled();
    expect(screen.queryByRole("button", { name: "Usuń rolę" })).toBeNull();

    await user.click(screen.getByRole("button", { name: "Wszystkie role" }));
    await user.click(screen.getByRole("button", { name: "Edytuj rolę Członek" }));
    expect(screen.getByRole("switch", { name: "Zarządzanie banami" })).toBeEnabled();
    expect(screen.queryByRole("button", { name: "Usuń rolę" })).toBeNull();
  });

  it("opens a new role as soon as it is made", async () => {
    const user = userEvent.setup();
    const made = role({ id: "role-new", name: "Nowa rola", permissions: [], memberCount: 0 });
    const onCreate = vi.fn().mockResolvedValue(made);
    const { rerender } = render(
      withConfirm(
        <RolesPane supported roles={[OWNER, role(), MEMBER]} onCreate={onCreate} onUpdate={vi.fn()} onDelete={vi.fn()} />,
      ),
    );

    await user.click(screen.getByRole("button", { name: "Nowa rola" }));
    rerender(
      withConfirm(
        <RolesPane supported roles={[OWNER, role(), made, MEMBER]} onCreate={onCreate} onUpdate={vi.fn()} onDelete={vi.fn()} />,
      ),
    );

    await waitFor(() => expect(screen.getByRole("textbox", { name: /^Nazwa/ })).toHaveValue("Nowa rola"));
  });
});

describe("MembersPane", () => {
  function pane(overrides: Partial<MembersPaneProps> = {}) {
    const props: MembersPaneProps = {
      supported: true,
      roles: [OWNER, role(), MEMBER],
      members: [member("u-1", "Maja", OWNER.id), member("u-2", "Jan", "role-crew"), member("u-3", "Ola", MEMBER.id)],
      onAssignRole: vi.fn().mockResolvedValue(undefined),
      viewerId: "u-1",
      onKick: vi.fn().mockResolvedValue(undefined),
      onBan: vi.fn().mockResolvedValue(undefined),
      ...overrides,
    };
    render(withConfirm(<MembersPane {...props} />));
    return props;
  }

  it("shows the reader's own row and the owner's role as fixed pills", () => {
    pane();

    const maja = screen.getByText("Maja").closest(".ss-member") as HTMLElement;
    expect(within(maja).getByText("Ty")).toBeVisible();
    expect(within(maja).queryByRole("button", { name: /Rola Maja/ })).toBeNull();
    expect(within(maja).getByText("Właściciel")).toBeVisible();
    expect(within(maja).queryByRole("button", { name: /Więcej dla/ })).toBeNull();
  });

  it("finds people by name", async () => {
    const user = userEvent.setup();
    pane();

    await user.type(screen.getByRole("searchbox", { name: "Szukaj po nazwie" }), "ol");
    expect(screen.queryByText("Jan")).toBeNull();
    expect(screen.getByText("Ola")).toBeVisible();

    await user.type(screen.getByRole("searchbox", { name: "Szukaj po nazwie" }), "x");
    expect(screen.getByText("Nikt tu nie pasuje do tej nazwy.")).toBeVisible();
  });

  it("gives someone another role from the pill's menu", async () => {
    const user = userEvent.setup();
    const props = pane();

    await user.click(screen.getByRole("button", { name: "Rola Jan: Ekipa" }));
    const menu = screen.getByRole("menu");
    expect(within(menu).queryByRole("menuitem", { name: "Właściciel" })).toBeNull();
    await user.click(within(menu).getByRole("menuitem", { name: "Członek" }));

    await waitFor(() => expect(props.onAssignRole).toHaveBeenCalledWith("u-2", "role-member"));
  });

  it("kicks after the member menu's question", async () => {
    const user = userEvent.setup();
    const props = pane();

    await user.click(screen.getByRole("button", { name: "Więcej dla Jan" }));
    await user.click(screen.getByRole("menuitem", { name: "Wyrzuć z serwera" }));
    await user.click(within(screen.getByRole("dialog", { name: "Wyrzucić tę osobę?" })).getByRole("button", { name: "Wyrzuć" }));

    await waitFor(() => expect(props.onKick).toHaveBeenCalledWith("u-2"));
  });

  it("bans with the reason given, or none", async () => {
    const user = userEvent.setup();
    const props = pane();

    await user.click(screen.getByRole("button", { name: "Więcej dla Ola" }));
    await user.click(screen.getByRole("menuitem", { name: "Zbanuj" }));
    const question = screen.getByRole("dialog", { name: "Zbanować tę osobę?" });
    await user.type(within(question).getByRole("textbox", { name: /^Powód/ }), "Spam");
    await user.click(within(question).getByRole("button", { name: "Zbanuj" }));

    await waitFor(() => expect(props.onBan).toHaveBeenCalledWith("u-3", "Spam"));
  });

  it("offers no menu to someone who may not kick or ban, and read-only pills without the right to assign", () => {
    pane({ supported: false, onKick: undefined, onBan: undefined });

    expect(screen.queryByRole("button", { name: /Więcej dla/ })).toBeNull();
    expect(screen.queryByRole("button", { name: /^Rola / })).toBeNull();
    expect(screen.getByText("Ekipa")).toBeVisible();
  });

  it("says so when a change fails", async () => {
    const user = userEvent.setup();
    pane({ onAssignRole: vi.fn().mockRejectedValue(new Error("offline")) });

    await user.click(screen.getByRole("button", { name: "Rola Jan: Ekipa" }));
    await user.click(screen.getByRole("menuitem", { name: "Członek" }));

    expect(await screen.findByRole("alert")).toHaveTextContent("Nie udało się zmienić roli użytkownika.");
  });
});

describe("InvitesPane", () => {
  beforeEach(() => vi.useFakeTimers({ toFake: ["Date"] }));
  afterEach(() => vi.useRealTimers());

  function invite(id: string, expiresAt: string | null, overrides: Partial<Invite> = {}): Invite {
    return { id, url: `https://stellarr.test/invite/${id}`, createdByName: "Maja", uses: 1, maxUses: 5, expiresAt, ...overrides };
  }

  it("shows each link without its host, who made it, its uses and when it expires", () => {
    vi.setSystemTime(new Date(2026, 7, 6, 12));
    render(<InvitesPane supported invites={[invite("abc", new Date(2026, 7, 9, 12).toISOString())]} onCreate={vi.fn()} onRevoke={vi.fn()} />);

    expect(screen.getByText("invite/abc")).toHaveAttribute("title", "https://stellarr.test/invite/abc");
    expect(screen.getByText(/Utworzył\(a\) Maja · 1 z 5 użyć/)).toBeVisible();
  });

  it("says how long a new link lasts, and makes one", async () => {
    vi.useRealTimers();
    const user = userEvent.setup();
    const onCreate = vi.fn().mockResolvedValue(undefined);
    render(<InvitesPane supported invites={[]} onCreate={onCreate} onRevoke={vi.fn()} />);

    expect(screen.getByText(/Nowy link działa 7 dni albo 25 użyć/)).toBeVisible();
    await user.click(screen.getByRole("button", { name: "Nowy link" }));

    expect(onCreate).toHaveBeenCalledOnce();
  });

  it("copies and revokes with buttons of their own", async () => {
    vi.useRealTimers();
    const user = userEvent.setup();
    Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText: vi.fn().mockResolvedValue(undefined) } });
    const onRevoke = vi.fn().mockResolvedValue(undefined);
    render(<InvitesPane supported invites={[invite("a", null)]} onCreate={vi.fn()} onRevoke={onRevoke} />);

    await user.click(screen.getByRole("button", { name: "Kopiuj zaproszenie https://stellarr.test/invite/a" }));
    expect(screen.getByRole("button", { name: "Kopiuj zaproszenie https://stellarr.test/invite/a" })).toHaveTextContent("Skopiowano");
    await user.click(screen.getByRole("button", { name: "Unieważnij zaproszenie https://stellarr.test/invite/a" }));

    expect(onRevoke).toHaveBeenCalledWith("a");
  });

  it("says so when a change fails", async () => {
    vi.useRealTimers();
    const user = userEvent.setup();
    render(<InvitesPane supported invites={[]} onCreate={vi.fn().mockRejectedValue(new Error("offline"))} onRevoke={vi.fn()} />);

    await user.click(screen.getByRole("button", { name: "Nowy link" }));

    expect(await screen.findByRole("alert")).toBeVisible();
  });
});
