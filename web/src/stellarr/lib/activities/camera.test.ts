import { expect, it, vi } from "vitest";

import type { MediaSignal } from "@/lib/realtime/types";
import {
  encodeMediaServerOffer, RTC_PROTOCOL_VERSION, type BrowserMediaSignalKind, type MediaServerSignalKind,
  type MediaTrack,
} from "@/lib/rtc/protocol";
import { FakeRealtimeClient } from "@/test/fakes";
import { ActivityCameraTransport } from "./camera";

it("releases a late SFU grant if the activity closes while the viewer is joining", async () => {
  let grant!: (value: Record<string, unknown>) => void;
  const opening = new Promise<Record<string, unknown>>(resolve => { grant = resolve; });
  const activityCamera = vi.fn().mockReturnValueOnce(opening).mockResolvedValue({});
  const hub = Object.assign(new FakeRealtimeClient(), { activityCamera });
  await hub.start();
  const lease = { install: "game", access: "lease" };
  const camera = new ActivityCameraTransport(hub, "viewer", "place", lease, async () => []);
  const show = vi.fn();
  camera.watch("player", show);
  await vi.waitFor(() => expect(activityCamera).toHaveBeenCalledWith("place", { ...lease, action: { op: "open" } }));

  camera.dispose();
  grant({ channelId: "activity", sessionId: "retired", grant: "old" });
  await vi.waitFor(() => expect(activityCamera).toHaveBeenCalledWith("place", { ...lease, action: { op: "close", grant: "old" } }));
  expect(show.mock.calls.every(([stream]) => stream === null)).toBe(true);
});

/** Just enough of a connection to negotiate; no media ever flows. */
class FakePeerConnection {
  signalingState: RTCSignalingState = "stable";
  connectionState: RTCPeerConnectionState = "new";
  ontrack = null;
  onicecandidate = null;
  onconnectionstatechange = null;
  addTransceiver() {
    return { sender: { track: null, replaceTrack: async () => {} }, direction: "inactive" };
  }
  async createOffer() {
    return { type: "offer", sdp: "v=0\r\n" };
  }
  async createAnswer() {
    return { type: "answer", sdp: "v=0\r\n" };
  }
  async setLocalDescription({ type }: RTCSessionDescriptionInit) {
    this.signalingState = type === "offer" ? "have-local-offer" : "stable";
  }
  async setRemoteDescription({ type }: RTCSessionDescriptionInit) {
    this.signalingState = type === "offer" ? "have-remote-offer" : "stable";
  }
  close() {}
}

it("asks again for the same list when the media server offers a section to another person's camera", async () => {
  vi.stubGlobal("RTCPeerConnection", FakePeerConnection);
  vi.stubGlobal("MediaStream", class {});
  const activityCamera = vi.fn().mockResolvedValue({ channelId: "activity", sessionId: "s1", grant: "g1" });
  const hub = Object.assign(new FakeRealtimeClient(), { activityCamera });
  const sent: MediaSignal<BrowserMediaSignalKind>[] = [];
  vi.spyOn(hub, "sendMediaSignal").mockImplementation(async (signal) => { sent.push(signal); });
  await hub.start();
  const camera = new ActivityCameraTransport(hub, "viewer", "place", { install: "game", access: "lease" }, async () => []);
  camera.watch("u-b", () => {});
  camera.watch("u-c", () => {});
  const kinds = () => sent.map(signal => signal.kind);
  const lists = () => sent.filter(signal => signal.kind === "subscriptions").map(signal => JSON.parse(signal.payload).mids);
  const server = (kind: MediaServerSignalKind, payload: string) => hub.push("MediaSignal", {
    protocolVersion: RTC_PROTOCOL_VERSION, channelId: "activity", sessionId: "s1", kind, payload,
  } satisfies MediaSignal<MediaServerSignalKind>);
  const offer = async (negotiationId: number, tracks: MediaTrack[]) => {
    const answers = kinds().filter(kind => kind === "answer").length;
    server("offer", encodeMediaServerOffer({ sdp: "v=0\r\n", negotiationId, tracks }));
    await vi.waitFor(() => expect(kinds().filter(kind => kind === "answer")).toHaveLength(answers + 1));
  };
  await vi.waitFor(() => expect(kinds()).toEqual(["offer"]));
  server("answer", JSON.stringify({ type: "answer", sdp: "v=0\r\n" }));

  await offer(1, [{ mid: "v1", userId: "u-b", kind: "video", streamId: "cam-b", heldBack: false }]);
  expect(lists()).toEqual([["v1"]]);
  // Past the pace, so a list can go at once.
  await new Promise(resolve => setTimeout(resolve, 300));

  // B's camera went and C's took its section, without this side ever seeing
  // a directory between the two: the list reads as the one it sent.
  await offer(2, [{ mid: "v1", userId: "u-c", kind: "video", streamId: "cam-c", heldBack: false }]);

  expect(lists()).toEqual([["v1"], ["v1"]]);
  // It goes ahead of the answer that makes the section receive.
  expect(kinds().lastIndexOf("subscriptions")).toBeLessThan(kinds().lastIndexOf("answer"));
  camera.dispose();
  vi.unstubAllGlobals();
});
