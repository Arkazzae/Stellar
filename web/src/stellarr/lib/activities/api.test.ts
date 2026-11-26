import { afterEach, expect, it, vi } from "vitest";

import { translateMessage, type MessageKey, type MessageValues } from "@/i18n";
import { request } from "@/lib/api/transport";
import type * as Transport from "@/lib/api/transport";

import { activitiesApi, ActivityRequestError } from "./api";
import { refusalText } from "./reasons";

vi.mock("@/lib/api/transport", async (importOriginal) => ({
  ...(await importOriginal<typeof Transport>()),
  request: vi.fn(),
}));

const t = (key: MessageKey, values?: MessageValues) => translateMessage("pl", key, values);

afterEach(() => vi.clearAllMocks());

it("reads what a place may run, skipping entries it cannot use", async () => {
  vi.mocked(request).mockResolvedValueOnce(
    Response.json({
      activities: [
        {
          install: "charades",
          name: "Charades",
          version: "1.2.0",
          description: "Act it out.",
          artwork: { icon: "/icon.png", cover: null },
          running: false,
          supported: false,
          devices: ["desktop"],
          seats: 2,
          permissions: ["media.cameraTile"],
        },
        { name: "No install" },
      ],
      live: null,
      owner: { id: "u-owner", name: "Owner", avatar: null },
      viewerIsOwner: true,
    }),
  );

  const library = await activitiesApi.list("voice-1", { device: "phone" });

  expect(vi.mocked(request).mock.calls[0]?.[0]).toBe("/api/activities/voice-1/list");
  expect(library).toEqual({
    activities: [
      {
        install: "charades",
        name: "Charades",
        version: "1.2.0",
        description: "Act it out.",
        icon: "/icon.png",
        cover: null,
        running: false,
        supported: false,
        devices: ["desktop"],
        seats: 2,
        permissions: ["media.cameraTile"],
      },
    ],
    live: null,
    owner: { id: "u-owner", name: "Owner", avatar: null },
    viewerIsOwner: true,
    voiceOnly: [],
  });
});

it("keeps the server's reason for a refusal, by the code in its problem body", async () => {
  const refuse = (status: number, body: object) =>
    vi.mocked(request).mockResolvedValueOnce(Response.json(body, { status }));

  refuse(409, { title: "Conflict", status: 409, code: "full" });
  const full = await activitiesApi.launch("voice-1", { device: "desktop" }).catch((cause: unknown) => cause);
  expect(full).toBeInstanceOf(ActivityRequestError);
  expect(full).toMatchObject({ status: 409, code: "full" });
  expect(refusalText(t, full, "desktop")).toBe("Komplet. Spróbuj, gdy ktoś wyjdzie.");

  refuse(422, { code: "unsupported-device" });
  const device = await activitiesApi.launch("voice-1", { install: "jump", device: "phone" }).catch((c: unknown) => c);
  expect(refusalText(t, device, "phone")).toBe("Nie działa na telefonie");

  refuse(500, { code: "something-new" });
  const unknown = await activitiesApi.launch("voice-1", { device: "desktop" }).catch((c: unknown) => c);
  expect(unknown).toMatchObject({ status: 500, code: null });
  expect(refusalText(t, unknown, "desktop")).toBe(t("activities.error.generic"));
});
