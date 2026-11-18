import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import type { PersonVM } from "@/components/view-models";

import { DmRoom, type DmRoomProps } from "./DmRoom";

function person(id: string, displayName: string, presence: PersonVM["presence"] = "online") {
  return { id, displayName, initials: displayName.slice(0, 2).toUpperCase(), presence };
}

const ada = person("u-ada", "Ada");

function openRoom(overrides: Partial<DmRoomProps> = {}) {
  const props: DmRoomProps = {
    peer: ada,
    railOpen: false,
    onToggleRail: vi.fn(),
    profilePanelOpen: false,
    onToggleProfilePanel: vi.fn(),
    callActive: false,
    inCall: false,
    onCall: vi.fn(),
    videoOn: false,
    onVideoCall: vi.fn(),
    chatOpen: false,
    onToggleChat: vi.fn(),
    children: <p>Zawartość</p>,
    ...overrides,
  };
  const view = render(<DmRoom {...props} />);
  return { props, user: userEvent.setup(), ...view };
}

describe("DmRoom header", () => {
  it("titles the room with the person and says how reachable they are", () => {
    openRoom({ peer: person("u-ada", "Ada", "dnd"), status: "w trasie do Gdańska" });

    expect(screen.getByRole("heading", { level: 2, name: "Ada" })).toBeTruthy();
    expect(screen.getByText("Nie przeszkadzać · w trasie do Gdańska")).toBeTruthy();
  });

  it("shows and hides the profile from its button and from the person's name", async () => {
    const { user, props } = openRoom({ profilePanelOpen: true });

    const toggle = screen.getByRole("button", { name: "Profil" });
    const name = screen.getByRole("button", { name: "Ada" });
    expect(toggle).toHaveAttribute("aria-pressed", "true");
    expect(name).toHaveAttribute("aria-expanded", "true");

    await user.click(toggle);
    await user.click(name);
    expect(props.onToggleProfilePanel).toHaveBeenCalledTimes(2);
  });

  it("exposes the state of the conversation drawer from the burger", async () => {
    const { user, props } = openRoom({ railOpen: true });

    const burger = screen.getByRole("button", { name: "Pokaż rozmowy" });
    expect(burger).toHaveAttribute("aria-expanded", "true");
    await user.click(burger);
    expect(props.onToggleRail).toHaveBeenCalledOnce();
  });

  it("offers the notification button inside the conversation toolbar", () => {
    openRoom({ notificationButton: <button type="button">Powiadomienia</button> });

    const toolbar = screen.getByRole("toolbar", { name: "Rozmowa" });
    expect(toolbar).toContainElement(screen.getByRole("button", { name: "Powiadomienia" }));
  });

  it("keeps what it is given in the room's second grid track", () => {
    const { container } = openRoom();

    const view = container.querySelector(".room > .room-body > .view.active");
    expect(view).toHaveTextContent("Zawartość");
  });
});

describe("DmRoom call", () => {
  it("offers to start a call when nobody is in one", async () => {
    const { user, props } = openRoom();

    const call = screen.getByRole("button", { name: "Zadzwoń do Ada" });
    expect(call).toHaveAttribute("title", "Rozpocznij rozmowę głosową");
    expect(call).toHaveAttribute("aria-pressed", "false");
    await user.click(call);

    expect(props.onCall).toHaveBeenCalledOnce();
  });

  it("offers to join when the other person is already in the call", () => {
    openRoom({ callActive: true });

    const join = screen.getByRole("button", { name: "Dołącz do rozmowy z Ada" });
    expect(join).toHaveAttribute("title", "Dołącz do rozmowy");
    expect(join).toHaveAttribute("aria-pressed", "false");
  });

  it("shows you are connected", () => {
    openRoom({ callActive: true, inCall: true });

    const call = screen.getByRole("button", { name: "Rozmowa z Ada jest połączona" });
    expect(call).toHaveAttribute("title", "Jesteś w tej rozmowie");
    expect(call).toHaveAttribute("aria-pressed", "true");
  });

  it("gives the profile's button to the chat while you are in the call", async () => {
    const { user, props } = openRoom({ inCall: true, chatOpen: true });

    expect(screen.queryByRole("button", { name: "Profil" })).toBeNull();
    await user.click(screen.getByRole("button", { name: "Czat" }));
    // The face and the name still open the profile.
    await user.click(screen.getByRole("button", { name: "Ada" }));

    expect(props.onToggleChat).toHaveBeenCalledOnce();
    expect(props.onToggleProfilePanel).toHaveBeenCalledOnce();
  });

  it("counts on the chat's button the messages that arrived while it was shut", () => {
    openRoom({ inCall: true, chatUnread: 3 });

    const chat = screen.getByRole("button", { name: "Czat, nieprzeczytane: 3" });
    expect(chat).toHaveTextContent("3");
  });

  it("turns the camera on and off from the header", async () => {
    const { user, props } = openRoom();

    const camera = screen.getByRole("button", { name: "Włącz kamerę" });
    expect(camera).toHaveAttribute("aria-pressed", "false");
    await user.click(camera);

    expect(props.onVideoCall).toHaveBeenCalledOnce();
  });

  it("makes no call where this browser cannot encrypt one, and says why when pressed", async () => {
    const reason = "Ta przeglądarka nie potrafi szyfrować połączeń.";
    const { user, props } = openRoom({ callBlocked: reason });

    const call = screen.getByRole("button", { name: "Zadzwoń do Ada" });
    const camera = screen.getByRole("button", { name: "Włącz kamerę" });
    expect(screen.queryByRole("alert")).toBeNull();
    await user.click(call);

    // The reason is shown where a failed call is announced, not only in a title.
    expect(screen.getByRole("alert")).toHaveTextContent(reason);
    await user.click(screen.getByRole("button", { name: "Zamknij" }));
    expect(screen.queryByRole("alert")).toBeNull();
    await user.click(camera);
    expect(screen.getByRole("alert")).toHaveTextContent(reason);

    expect(props.onCall).not.toHaveBeenCalled();
    expect(props.onVideoCall).not.toHaveBeenCalled();
    for (const button of [call, camera]) {
      expect(button).toHaveAttribute("aria-disabled", "true");
      expect(button).toHaveAttribute("aria-description", reason);
    }
  });
});
