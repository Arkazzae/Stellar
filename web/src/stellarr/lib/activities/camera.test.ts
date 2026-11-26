import { expect, it, vi } from "vitest";

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
