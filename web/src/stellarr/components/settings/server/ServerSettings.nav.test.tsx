import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";

import { ConfirmProvider } from "@/components/primitives/ConfirmDialog";

import { ServerSettings, type ServerSettingsProps } from "./ServerSettings";

function props(overrides: Partial<ServerSettingsProps> = {}): ServerSettingsProps {
  return {
    open: true,
    onClose: vi.fn(),
    name: "Orbit",
    mark: "OR",
    overview: {
      supported: true,
      iconSupported: true,
      name: "Orbit",
      mark: "OR",
      memberCount: 2,
      channelCount: 1,
      description: "",
      isPublic: false,
      onSave: vi.fn().mockResolvedValue(undefined),
      onPickIcon: vi.fn().mockResolvedValue(undefined),
      onRemoveIcon: vi.fn().mockResolvedValue(undefined),
    },
    channels: {
      supported: true,
      channels: [],
      onAdd: vi.fn(),
      onEdit: vi.fn(),
      sectionsSupported: true,
      sections: [],
      onAddSection: vi.fn(),
      onEditSection: vi.fn(),
      onMoveChannel: vi.fn().mockResolvedValue(undefined),
      onMoveSection: vi.fn().mockResolvedValue(undefined),
    },
    roles: {
      supported: true,
      roles: [],
      onCreate: vi.fn().mockResolvedValue(undefined),
      onUpdate: vi.fn().mockResolvedValue(undefined),
      onDelete: vi.fn().mockResolvedValue(undefined),
    },
    members: { supported: true, members: [], roles: [], onAssignRole: vi.fn().mockResolvedValue(undefined) },
    invites: {
      supported: true,
      invites: [],
      onCreate: vi.fn().mockResolvedValue(undefined),
      onRevoke: vi.fn().mockResolvedValue(undefined),
    },
    emoji: {
      supported: true,
      emoji: [],
      onUpload: vi.fn().mockResolvedValue(undefined),
      onDelete: vi.fn().mockResolvedValue(undefined),
    },
    soundboard: {
      supported: true,
      sounds: [],
      everyoneCanPlay: true,
      onEveryoneCanPlayChange: vi.fn(),
      rateLimited: true,
      onRateLimitedChange: vi.fn(),
      onCreate: vi.fn(),
      onUpdate: vi.fn(),
      onDelete: vi.fn().mockResolvedValue(undefined),
      playback: { soundboardVolume: 100 },
      onReorder: vi.fn(),
    },
    audit: { supported: true, entries: [] },
    bans: { supported: true, bans: [], onUnban: vi.fn().mockResolvedValue(undefined) },
    ...overrides,
  };
}

function dialog(p: ServerSettingsProps) {
  return (
    <ConfirmProvider>
      <ServerSettings {...p} />
    </ConfirmProvider>
  );
}

/** What each pane shows once it is the one on screen, besides the bar. */
const PANES = [
  { item: "Przegląd", detail: "Jak serwer wygląda w Odkrywaniu i w zaproszeniach", marker: "Profil" },
  { item: "Kanały", detail: "Sekcje i ich kanały", marker: "Kanały i sekcje" },
  { item: "Emoji", detail: "Własne emoji dla wszystkich na serwerze", marker: "Emoji" },
  { item: "Soundboard", detail: "Klipy do puszczania na kanałach głosowych", marker: "Dźwięki" },
  { item: "Członkowie", detail: "Kto jest na serwerze i z jaką rolą", marker: "Członkowie" },
  { item: "Role", detail: "Na co pozwala każda rola", marker: "Role" },
  { item: "Zaproszenia", detail: "Linki, przez które można dołączyć", marker: "Linki z zaproszeniem" },
  { item: "Bany", detail: "Osoby, które nie mogą wrócić", marker: "Zbanowani" },
  { item: "Dziennik zdarzeń", detail: "Kto co zmienił na serwerze, od najnowszych", marker: "Dziennik zdarzeń" },
];

afterEach(() => vi.restoreAllMocks());

describe("ServerSettings navigation", () => {
  it("lists two captioned groups in order, as user settings do", () => {
    render(dialog(props()));

    const nav = screen.getByRole("navigation", { name: "Sekcje ustawień" });
    expect(Array.from(nav.children, (child) => child.textContent)).toEqual([
      "Serwer",
      "Przegląd",
      "Kanały",
      "Emoji",
      "Soundboard",
      "Ludzie",
      "Członkowie",
      "Role",
      "Zaproszenia",
      "Bany",
      "Dziennik zdarzeń",
    ]);
  });

  it("is named after the server and opens on the overview", () => {
    render(dialog(props()));

    expect(screen.getByRole("dialog", { name: "Orbit" })).toBeVisible();
    expect(screen.getByText("Ustawienia serwera")).toBeVisible();
    expect(screen.getByRole("heading", { name: "Przegląd" })).toBeVisible();
    expect(screen.getByRole("button", { name: "Przegląd" })).toHaveAttribute("aria-current", "page");
  });

  it("shows the server's mark beside its name, its icon when it has one", () => {
    const { rerender } = render(dialog(props()));
    const brand = document.querySelector(".sv-brand")!;
    expect(brand.querySelector(".ss-mark")).toHaveTextContent("OR");

    rerender(dialog(props({ iconUrl: "/assets/icon" })));

    expect(document.querySelector(".sv-brand .ss-mark img")).toHaveAttribute("src", "/assets/icon");
  });

  it.each(PANES)("shows the $item pane with its title and detail line", async ({ item, detail, marker }) => {
    const user = userEvent.setup();
    render(dialog(props()));

    await user.click(screen.getByRole("button", { name: item }));

    expect(screen.getByRole("heading", { level: 2, name: item })).toBeVisible();
    expect(screen.getByText(detail)).toBeVisible();
    expect(screen.getByRole("region", { name: marker })).toBeVisible();
    expect(screen.getByRole("button", { name: item })).toHaveAttribute("aria-current", "page");
    expect(screen.getAllByRole("button", { current: "page" })).toHaveLength(1);
  });

  it("opens on the pane it is asked for", () => {
    render(dialog(props({ initialPane: "bany" })));

    expect(screen.getByRole("heading", { level: 2, name: "Bany" })).toBeVisible();
  });

  it("mounts only the chosen pane", async () => {
    const user = userEvent.setup();
    render(dialog(props()));
    expect(screen.getByRole("region", { name: "Profil" })).toBeVisible();

    await user.click(screen.getByRole("button", { name: "Bany" }));

    expect(screen.queryByRole("region", { name: "Profil" })).toBeNull();
  });

  it("starts the next pane at the top of the scroll area", async () => {
    const user = userEvent.setup();
    render(dialog(props()));
    const panes = document.querySelector('section[data-pane="prz"]')!.parentElement!;
    panes.scrollTop = 240;

    await user.click(screen.getByRole("button", { name: "Role" }));

    expect(panes.scrollTop).toBe(0);
  });

  it("brings the chosen item into view in the horizontal navigation", async () => {
    const user = userEvent.setup();
    const reveal = vi.spyOn(Element.prototype, "scrollIntoView");
    render(dialog(props()));

    const target = screen.getByRole("button", { name: "Zaproszenia" });
    await user.click(target);

    await waitFor(() => expect(reveal.mock.contexts.at(-1)).toBe(target));
  });
});

describe("deleting the server", () => {
  it("is offered alone at the bottom of the navigation, to the owner only", () => {
    const { rerender } = render(dialog(props()));
    expect(screen.queryByRole("button", { name: "Usuń serwer" })).toBeNull();

    rerender(dialog(props({ onDeleteServer: vi.fn().mockResolvedValue(undefined) })));

    const nav = screen.getByRole("navigation", { name: "Sekcje ustawień" });
    const button = screen.getByRole("button", { name: "Usuń serwer" });
    expect(within(nav).queryByRole("button", { name: "Usuń serwer" })).toBeNull();
    expect(button.closest(".us-out")).not.toBeNull();
  });

  it("deletes only once the server's exact name is typed", async () => {
    const user = userEvent.setup();
    const onDeleteServer = vi.fn().mockResolvedValue(undefined);
    render(dialog(props({ onDeleteServer })));

    await user.click(screen.getByRole("button", { name: "Usuń serwer" }));
    const question = screen.getByRole("dialog", { name: "Usunąć Orbit?" });
    const field = within(question).getByRole("textbox", { name: "WPISZ Orbit, ABY POTWIERDZIĆ" });
    const confirm = within(question).getByRole("button", { name: "Usuń serwer" });

    await user.type(field, "Orb");
    expect(confirm).toBeDisabled();
    await user.type(field, "it");
    expect(confirm).toBeEnabled();
    await user.click(confirm);

    await waitFor(() => expect(onDeleteServer).toHaveBeenCalledOnce());
  });

  it("does nothing when the question is cancelled", async () => {
    const user = userEvent.setup();
    const onDeleteServer = vi.fn().mockResolvedValue(undefined);
    render(dialog(props({ onDeleteServer })));

    await user.click(screen.getByRole("button", { name: "Usuń serwer" }));
    await user.click(within(screen.getByRole("dialog", { name: "Usunąć Orbit?" })).getByRole("button", { name: "Anuluj" }));

    expect(onDeleteServer).not.toHaveBeenCalled();
  });
});

describe("ServerSettings dialog", () => {
  it("renders nothing while closed", () => {
    render(dialog(props({ open: false })));

    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("closes from the close button and from Escape", async () => {
    const user = userEvent.setup();
    const p = props();
    render(dialog(p));

    await user.click(screen.getByRole("button", { name: "Zamknij ustawienia serwera" }));
    await user.keyboard("{Escape}");

    expect(p.onClose).toHaveBeenCalledTimes(2);
  });

  it("starts on the overview again when it is reopened", async () => {
    const user = userEvent.setup();
    const p = props();
    const { rerender } = render(dialog(p));
    await user.click(screen.getByRole("button", { name: "Bany" }));

    rerender(dialog({ ...p, open: false }));
    rerender(dialog({ ...p, open: true }));

    expect(screen.getByRole("heading", { name: "Przegląd" })).toBeVisible();
    expect(screen.getByRole("button", { name: "Przegląd" })).toHaveAttribute("aria-current", "page");
  });
});
