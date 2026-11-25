import { render, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { DmCallView, type DmCallViewProps } from "./DmCallView";

const ada = { id: "u-ada", displayName: "Ada", initials: "AD", presence: "online" } as const;
const maja = { id: "u-maja", displayName: "Maja", initials: "MA", presence: "online" } as const;

function openCall(overrides: Partial<DmCallViewProps> = {}) {
  const props: DmCallViewProps = {
    peer: ada,
    self: maja,
    ringing: false,
    since: Date.now(),
    shares: [],
    onWatchShare: vi.fn(),
    onStopWatchingShare: vi.fn(),
    localPersonId: maja.id,
    bar: <div role="toolbar" aria-label="Pasek" />,
    ...overrides,
  };
  const view = render(<DmCallView {...props} />);
  return { props, user: userEvent.setup(), ...view };
}

const corner = (container: HTMLElement) => container.querySelector<HTMLElement>(".dm-call-corner")!;
const middle = (container: HTMLElement) => container.querySelector<HTMLElement>(".dm-call-stage")!;

describe("DmCallView", () => {
  it("puts the other person in the middle with the call's duration, and you in the corner", () => {
    const { container } = openCall();

    expect(within(middle(container)).getByText("Ada")).toBeTruthy();
    expect(within(middle(container)).getByText("00:00")).toBeTruthy();
    expect(within(corner(container)).getByText("Ty")).toBeTruthy();
    expect(within(corner(container)).queryByText("Ada")).toBeNull();
  });

  it("says it is calling, or what holds the call up, in place of the duration", () => {
    const { container, rerender, props } = openCall({ ringing: true });
    expect(within(middle(container)).getByText("Dzwonię…")).toBeTruthy();
    expect(container.querySelector("time")).toBeNull();
    // Only a call this side really encrypts says it is encrypted.
    expect(within(middle(container)).queryByText("Szyfrowane end-to-end")).toBeNull();

    rerender(<DmCallView {...props} ringing={false} status="Łączenie" encrypted />);
    expect(within(middle(container)).getByText("Łączenie")).toBeTruthy();
    expect(within(middle(container)).getByText("Szyfrowane end-to-end")).toBeTruthy();
    expect(container.querySelector("time")).toBeNull();
  });

  it("says whose sound or picture does not decrypt, and shows no picture of theirs", () => {
    const camera = {} as MediaStream;
    const { container, rerender, props } = openCall({
      peer: { ...ada, undecryptable: { audio: true, video: false } },
    });
    expect(within(middle(container)).getByText("Nie można odszyfrować dźwięku od Ada")).toBeTruthy();

    rerender(
      <DmCallView
        {...props}
        peer={{ ...ada, undecryptable: { audio: true, video: true } }}
        cameraStreams={new Map([[ada.id, camera]])}
      />,
    );
    expect(within(middle(container)).getByText("Nie można odszyfrować dźwięku ani obrazu od Ada")).toBeTruthy();
    expect(middle(container).querySelector("video")).toBeNull();
  });

  it("puts the other person's camera in place of their face, and yours in your tile", () => {
    const camera = {} as MediaStream;
    const { container } = openCall({
      cameraStreams: new Map([
        [ada.id, camera],
        [maja.id, camera],
      ]),
    });

    expect(within(middle(container)).getByLabelText("Kamera: Ada")).toBeTruthy();
    expect(within(middle(container)).queryByText("00:00")).toBeNull();
    expect(within(corner(container)).getByLabelText("Kamera: Maja")).toBeTruthy();
  });

  it("gives a shared screen the middle and moves the other person above you", async () => {
    const { container, user, props } = openCall({
      shares: [
        {
          key: "remote:u-ada:s1",
          ownerId: ada.id,
          ownerName: "Ada",
          local: false,
          hasAudio: false,
          stream: null,
          watching: false,
        },
      ],
    });

    await user.click(within(middle(container)).getByRole("button", { name: "Oglądaj: Ada (ekran)" }));
    expect(props.onWatchShare).toHaveBeenCalledWith("remote:u-ada:s1");

    const tiles = within(corner(container));
    expect(tiles.getByText("Ada")).toBeTruthy();
    expect(tiles.getByText("Ty")).toBeTruthy();
  });
});
