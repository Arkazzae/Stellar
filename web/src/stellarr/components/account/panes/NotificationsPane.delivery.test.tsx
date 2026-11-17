import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import type { UserPreferences } from "@/ports";

import {
  UserNotificationsPane,
  type NotificationDeliveryProps,
} from "./NotificationsPane";

const preferences: UserPreferences = {
  desktopNotifications: true,
  mentionPopups: true,
  callRingtone: true,
  emailNotifications: false,
  pushNotifications: false,
  mentionsOnly: true,
  messageSound: true,
  joinLeaveSound: true,
  allowDirectMessages: true,
  allowFriendRequests: true,
  showPresence: true,
  developerMode: false,
  reducedMotion: false,
  mutedChannelIds: [],
  vadSensitivity: 60,
  joinSoundId: null,
};

function delivery(overrides: Partial<NotificationDeliveryProps> = {}): NotificationDeliveryProps {
  return {
    loading: false,
    loadFailed: false,
    emailAvailable: true,
    emailVerified: true,
    pushAvailable: true,
    pushKeyAvailable: true,
    emailEnabled: false,
    pushEnabled: false,
    savingEmail: false,
    savingPush: false,
    emailError: null,
    pushError: null,
    device: {
      supported: true,
      permission: "granted",
      status: "idle",
      pending: false,
      error: null,
    },
    onEmailChange: vi.fn(),
    onPushChange: vi.fn(),
    onEnableDevice: vi.fn(),
    onDisableDevice: vi.fn(),
    onReload: vi.fn(),
    ...overrides,
  };
}

function renderPane(props: NotificationDeliveryProps) {
  return render(
    <UserNotificationsPane
      preferences={preferences}
      onChange={vi.fn()}
      delivery={props}
    />,
  );
}

describe("notification delivery settings", () => {
  it("does not offer delivery controls when the container wires none", () => {
    render(<UserNotificationsPane preferences={preferences} onChange={vi.fn()} />);

    expect(screen.queryByRole("switch", { name: "Powiadomienia e-mail" })).toBeNull();
    expect(screen.queryByRole("switch", { name: "Powiadomienia push" })).toBeNull();
  });

  it("says e-mail is unavailable instead of pretending the switch would work", () => {
    renderPane(delivery({ emailAvailable: false, emailVerified: false }));

    expect(screen.getByRole("switch", { name: "Powiadomienia e-mail" })).toBeDisabled();
    expect(screen.getByText(/nie ma skonfigurowanej wysyłki e-mail/)).toBeInTheDocument();
  });

  it("requires a verified address and names how the existing second step verifies it", () => {
    renderPane(delivery({ emailAvailable: true, emailVerified: false }));

    expect(screen.getByRole("switch", { name: "Powiadomienia e-mail" })).toBeDisabled();
    expect(screen.getByText(/wymagają potwierdzonego adresu/)).toBeInTheDocument();
    expect(screen.getByText(/„kod mailem” jako drugi krok/)).toBeInTheDocument();
  });

  it("saves the global e-mail and push switches separately", async () => {
    const user = userEvent.setup();
    const onEmailChange = vi.fn();
    const onPushChange = vi.fn();
    renderPane(delivery({ onEmailChange, onPushChange }));

    await user.click(screen.getByRole("switch", { name: "Powiadomienia e-mail" }));
    await user.click(screen.getByRole("switch", { name: "Powiadomienia push" }));

    expect(onEmailChange).toHaveBeenCalledExactlyOnceWith(true);
    expect(onPushChange).toHaveBeenCalledExactlyOnceWith(true);
  });

  it("disables push when the server has no Web Push configuration or key", () => {
    const { rerender } = renderPane(delivery({ pushAvailable: false }));

    expect(screen.getByRole("switch", { name: "Powiadomienia push" })).toBeDisabled();
    expect(screen.getByText(/nie ma skonfigurowanego Web Push/)).toBeInTheDocument();

    rerender(
      <UserNotificationsPane
        preferences={preferences}
        onChange={vi.fn()}
        delivery={delivery({ pushAvailable: true, pushKeyAvailable: false })}
      />,
    );
    expect(screen.getByRole("switch", { name: "Powiadomienia push" })).toBeDisabled();
    expect(screen.getAllByText(/Web Push nie jest skonfigurowany/).length).toBeGreaterThan(1);
  });

  it("enrolls this device explicitly and explains that it is separate", async () => {
    const user = userEvent.setup();
    const onEnableDevice = vi.fn();
    renderPane(delivery({ onEnableDevice }));

    expect(screen.getByText(/Każde urządzenie zapisujesz do push osobno/)).toBeInTheDocument();
    expect(screen.getByText(/Przeglądarka zapyta o zgodę dopiero po kliknięciu/)).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Zapisz to urządzenie" }));

    expect(onEnableDevice).toHaveBeenCalledOnce();
  });

  it("shows an enrolled device with the way to turn it off", async () => {
    const user = userEvent.setup();
    const onDisableDevice = vi.fn();
    renderPane(
      delivery({
        device: {
          supported: true,
          permission: "granted",
          status: "enrolled",
          pending: false,
          error: null,
        },
        onDisableDevice,
      }),
    );

    expect(screen.getByText(/To urządzenie jest zapisane/)).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Wyłącz to urządzenie" }));

    expect(onDisableDevice).toHaveBeenCalledOnce();
  });

  it("shows blocked, unsupported and pending device states", () => {
    const { rerender } = renderPane(
      delivery({
        device: {
          supported: true,
          permission: "denied",
          status: "idle",
          pending: false,
          error: null,
        },
      }),
    );
    expect(screen.getAllByText(/zablokowane w przeglądarce lub systemie/).length)
      .toBeGreaterThan(0);

    rerender(
      <UserNotificationsPane
        preferences={preferences}
        onChange={vi.fn()}
        delivery={delivery({
          device: {
            supported: false,
            permission: "unsupported",
            status: "idle",
            pending: false,
            error: null,
          },
        })}
      />,
    );
    expect(screen.getAllByText(/nie obsługuje Web Push/).length).toBeGreaterThan(0);

    rerender(
      <UserNotificationsPane
        preferences={preferences}
        onChange={vi.fn()}
        delivery={delivery({
          device: {
            supported: true,
            permission: "granted",
            status: "idle",
            pending: true,
            error: null,
          },
        })}
      />,
    );
    expect(screen.getAllByText("Zapisywanie tego urządzenia…").length).toBeGreaterThan(0);
    expect(screen.getByRole("button", { name: "Zapisywanie tego urządzenia…" })).toBeDisabled();
  });

  it("shows a useful device failure without changing the global switches", () => {
    renderPane(
      delivery({
        device: {
          supported: true,
          permission: "granted",
          status: "idle",
          pending: false,
          error: "preference.pushDeviceServerFailed",
        },
      }),
    );

    expect(screen.getByRole("alert")).toHaveTextContent(/Serwer nie przyjął subskrypcji/);
  });

  it("offers a retry when capabilities could not be read", async () => {
    const user = userEvent.setup();
    const onReload = vi.fn();
    renderPane(delivery({ loadFailed: true, onReload }));

    expect(screen.getByRole("alert")).toHaveTextContent(/Nie udało się sprawdzić dostawy/);
    await user.click(screen.getByRole("button", { name: "Sprawdź ponownie" }));

    expect(onReload).toHaveBeenCalledOnce();
  });
});
