import { describe, expect, it } from "vitest";

import { itSendsCalls, jsonResponse, mockFetch } from "@/test/http";

import { notificationDeliveryCalls } from "./notificationDelivery";

const fetchMock = mockFetch();
const calls = notificationDeliveryCalls();

const CAPABILITIES = {
  emailAvailable: true,
  emailVerified: false,
  pushAvailable: true,
  applicationServerKey: "BEl6c3RhbGxhcnItdGVzdA",
};

const SUBSCRIPTION = {
  endpoint: "https://push.example/sub-1",
  keys: { p256dh: "p256dh-key", auth: "auth-secret" },
};

describe("notification delivery calls", () => {
  itSendsCalls(fetchMock, [
    {
      name: "get notification delivery",
      run: () => calls.getNotificationDelivery(),
      method: "GET",
      url: "/api/me/notification-delivery",
      answer: CAPABILITIES,
    },
    {
      name: "save a push subscription",
      run: () => calls.savePushSubscription(SUBSCRIPTION),
      method: "PUT",
      url: "/api/me/push-subscriptions",
      body: SUBSCRIPTION,
      answer: { id: "push-row-1" },
    },
    {
      name: "delete a push subscription",
      run: () => calls.deletePushSubscription(SUBSCRIPTION.endpoint),
      method: "DELETE",
      url: "/api/me/push-subscriptions",
      body: { endpoint: SUBSCRIPTION.endpoint },
    },
  ]);

  it("reads the capabilities and the saved row id", async () => {
    fetchMock
      .mockResolvedValueOnce(jsonResponse(CAPABILITIES))
      .mockResolvedValueOnce(jsonResponse({ id: "push-row-1" }));

    await expect(calls.getNotificationDelivery()).resolves.toEqual(CAPABILITIES);
    await expect(calls.savePushSubscription(SUBSCRIPTION)).resolves.toBe("push-row-1");
  });

  it("treats an empty application server key as no push configuration", async () => {
    fetchMock.mockResolvedValue(jsonResponse({ ...CAPABILITIES, applicationServerKey: "" }));

    await expect(calls.getNotificationDelivery()).resolves.toMatchObject({
      applicationServerKey: null,
    });
  });

  it("rejects an answer missing a capability flag", async () => {
    fetchMock.mockResolvedValue(jsonResponse({ ...CAPABILITIES, emailAvailable: "yes" }));

    await expect(calls.getNotificationDelivery()).rejects.toMatchObject({ kind: "malformed" });
  });
});
