import { afterEach, describe, expect, it, vi } from "vitest";

import { jsonResponse, mockFetch, noContent, sentRequest } from "@/test/http";

import { ACTIVE_SERVER_STORAGE_KEY, rememberActiveServerId, request } from "../transport";
import { sessionCalls } from "./session";

const fetchMock = mockFetch();
const calls = sessionCalls();

const USER = { id: "u-1", username: "patryk", displayName: "Patryk", presence: "online" };
const BOOTSTRAP = {
  user: USER,
  server: { id: "s-2", name: "Orbit" },
  channels: [],
  sections: [],
  members: [USER],
};

afterEach(() => window.history.replaceState({}, "", "/"));

describe("signing in and out", () => {
  it("checks whether the instance still waits for its first account", async () => {
    fetchMock.mockResolvedValue(jsonResponse({ setupRequired: true }));
    await expect(calls.setupRequired()).resolves.toBe(true);
    expect(sentRequest(fetchMock).url).toBe("/api/auth/registration");
    fetchMock.mockResolvedValue(jsonResponse({ needsServerSetup: false }));
    await expect(calls.setupRequired()).rejects.toMatchObject({ kind: "malformed" });
  });

  it("reads the instance's name from the discovery document", async () => {
    fetchMock.mockResolvedValue(jsonResponse({ product: "stellarr", instanceName: "Stellarr Dev", apiProtocol: 1 }));
    await expect(calls.instanceName()).resolves.toBe("Stellarr Dev");
    expect(sentRequest(fetchMock).url).toBe("/.well-known/stellarr/client");
  });

  it("previews an invite by its code, and treats a refused one as not found", async () => {
    fetchMock.mockResolvedValueOnce(
      jsonResponse({
        server: { id: "s-1", name: "Stellarr Dev", description: "", iconUrl: null },
        channel: null,
        inviter: { displayName: "Ada", avatarColor: "oklch(0.58 0.14 24)", avatarUrl: null },
        expiresAt: null,
      }),
    );

    await expect(calls.invitePreview("k7/qm")).resolves.toEqual({
      server: { id: "s-1", name: "Stellarr Dev", description: "", iconUrl: null },
      channel: null,
      inviter: { displayName: "Ada", avatarColor: "oklch(0.58 0.14 24)", avatarUrl: null },
      expiresAt: null,
    });
    expect(sentRequest(fetchMock).url).toBe("/api/invites/k7%2Fqm");

    fetchMock.mockResolvedValueOnce(jsonResponse({ title: "Not Found" }, 404));
    await expect(calls.invitePreview("gone")).rejects.toMatchObject({ kind: "not-found" });
  });

  it("reads the identity waiting for this browser as none once it is gone, and forgets it", async () => {
    fetchMock.mockResolvedValueOnce(jsonResponse({ title: "Not Found" }, 404));
    await expect(calls.pendingIdentity()).resolves.toBeNull();
    expect(sentRequest(fetchMock).url).toBe("/api/auth/oidc/pending");

    fetchMock.mockResolvedValueOnce(noContent());
    await calls.dropPendingIdentity();
    expect(sentRequest(fetchMock, 1)).toMatchObject({ url: "/api/auth/oidc/pending", method: "DELETE" });
  });

  it("names the fields a refused registration got wrong", async () => {
    fetchMock.mockResolvedValue(
      jsonResponse({ title: "One or more validation errors occurred.", errors: { setupCode: ["Kod jest nieprawidłowy."] } }, 400),
    );

    await expect(
      calls.register({ username: "owner", email: "o@example.org", password: "pw", displayName: "O", setupCode: "K7QM" }),
    ).rejects.toMatchObject({ kind: "invalid", fields: { setupCode: "Kod jest nieprawidłowy." } });
    expect(sentRequest(fetchMock).body).toMatchObject({ setupCode: "K7QM" });
  });

  it("posts the credentials and reads the user from an envelope", async () => {
    fetchMock.mockResolvedValue(jsonResponse({ user: USER }));

    await expect(calls.register({ username: "patryk", email: "p@example.org", password: "pw", displayName: "P" }))
      .resolves.toMatchObject({ kind: "signed-in", user: { id: "u-1" } });

    const sent = sentRequest(fetchMock);
    expect([sent.method, sent.url]).toEqual(["POST", "/api/auth/register"]);
    expect(sent.body).toEqual({
      username: "patryk",
      email: "p@example.org",
      password: "pw",
      displayName: "P",
    });
  });

  it("asks for a password reset and completes it", async () => {
    fetchMock.mockResolvedValue(noContent());

    await calls.requestPasswordReset("a@b.c");
    await calls.resetPassword("token", "new-password");

    expect(sentRequest(fetchMock, 0)).toMatchObject({
      url: "/api/auth/password/forgot",
      body: { email: "a@b.c" },
    });
    expect(sentRequest(fetchMock, 1)).toMatchObject({
      url: "/api/auth/password/reset",
      body: { token: "token", newPassword: "new-password" },
    });
  });

  it("treats a logout the server rejects as signed out", async () => {
    fetchMock.mockResolvedValue(new Response(null, { status: 401 }));
    await expect(calls.logout()).resolves.toBeUndefined();
    expect(sentRequest(fetchMock).method).toBe("POST");
  });

  it("still fails when the server could not log out", async () => {
    fetchMock.mockResolvedValue(new Response(null, { status: 500 }));
    await expect(calls.logout()).rejects.toMatchObject({ kind: "server" });
  });
});

describe("bootstrap", () => {
  const POPOUT_SERVER = "84ef8596-b119-42a4-94ee-a34c8be86ae3";

  it("retargets a reused popout before bootstrap and allows later manual switching", async () => {
    rememberActiveServerId("previous-window-server");
    window.history.replaceState({}, "", `/?popout=1&voiceChannel=voice-2&serverId=${POPOUT_SERVER}`);
    fetchMock.mockResolvedValueOnce(jsonResponse({
      ...BOOTSTRAP,
      server: { ...BOOTSTRAP.server, id: POPOUT_SERVER },
    }));

    await calls.bootstrap();

    expect(sentRequest(fetchMock).headers.get("X-Stellarr-Server")).toBe(POPOUT_SERVER);
    expect(window.sessionStorage.getItem(ACTIVE_SERVER_STORAGE_KEY)).toBe(POPOUT_SERVER);
    expect(new URLSearchParams(window.location.search).has("serverId")).toBe(false);
    expect(new URLSearchParams(window.location.search).get("voiceChannel")).toBe("voice-2");

    rememberActiveServerId("s-2");
    fetchMock.mockResolvedValueOnce(jsonResponse(BOOTSTRAP));
    await calls.bootstrap();
    expect(sentRequest(fetchMock, 1).headers.get("X-Stellarr-Server")).toBe("s-2");
  });

  it("does not fall back when the explicitly launched popout server is forbidden", async () => {
    rememberActiveServerId("previous-window-server");
    window.history.replaceState({}, "", `/?popout=1&serverId=${POPOUT_SERVER}`);
    fetchMock.mockResolvedValue(new Response(null, { status: 403 }));

    await expect(calls.bootstrap()).rejects.toMatchObject({ kind: "forbidden" });

    expect(fetchMock).toHaveBeenCalledOnce();
    expect(sentRequest(fetchMock).headers.get("X-Stellarr-Server")).toBe(POPOUT_SERVER);
    expect(new URLSearchParams(window.location.search).get("serverId")).toBe(POPOUT_SERVER);
  });

  it.each(["", "not-a-server"])("rejects the invalid popout server %j without requesting a default", async (serverId) => {
    window.history.replaceState({}, "", `/?popout=1&serverId=${serverId}`);

    await expect(calls.bootstrap()).rejects.toMatchObject({ kind: "invalid" });

    expect(fetchMock).not.toHaveBeenCalled();
  });

  it("refuses a response from a different server than the popout requested", async () => {
    window.history.replaceState({}, "", `/?popout=1&serverId=${POPOUT_SERVER}`);
    fetchMock.mockResolvedValue(jsonResponse(BOOTSTRAP));

    await expect(calls.bootstrap()).rejects.toMatchObject({ kind: "malformed" });

    expect(window.sessionStorage.getItem(ACTIVE_SERVER_STORAGE_KEY)).toBeNull();
  });

  it("remembers the server it was answered for", async () => {
    fetchMock.mockResolvedValue(jsonResponse(BOOTSTRAP));

    await calls.bootstrap();

    expect(window.sessionStorage.getItem(ACTIVE_SERVER_STORAGE_KEY)).toBe("s-2");
  });

  it("sends the remembered server and asks again without it once it is refused", async () => {
    rememberActiveServerId("s-gone");
    fetchMock
      .mockResolvedValueOnce(new Response(null, { status: 403 }))
      .mockResolvedValueOnce(jsonResponse(BOOTSTRAP));

    await expect(calls.bootstrap()).resolves.toMatchObject({ server: { id: "s-2" } });

    expect(fetchMock).toHaveBeenCalledTimes(2);
    expect(sentRequest(fetchMock, 0).headers.get("X-Stellarr-Server")).toBe("s-gone");
    expect(sentRequest(fetchMock, 1).headers.has("X-Stellarr-Server")).toBe(false);
    expect(window.sessionStorage.getItem(ACTIVE_SERVER_STORAGE_KEY)).toBe("s-2");
  });

  it("asks only once more, and reports a second refusal", async () => {
    rememberActiveServerId("s-gone");
    fetchMock.mockImplementation(() => Promise.resolve(new Response(null, { status: 403 })));

    await expect(calls.bootstrap()).rejects.toMatchObject({ kind: "forbidden" });

    expect(fetchMock).toHaveBeenCalledTimes(2);
  });

  it("does not retry a refusal when no server was remembered", async () => {
    fetchMock.mockResolvedValue(new Response(null, { status: 403 }));

    await expect(calls.bootstrap()).rejects.toMatchObject({ kind: "forbidden" });

    expect(fetchMock).toHaveBeenCalledTimes(1);
  });

  it("does not retry other failures", async () => {
    rememberActiveServerId("s-1");
    fetchMock.mockResolvedValue(new Response(null, { status: 500 }));

    await expect(calls.bootstrap()).rejects.toMatchObject({ kind: "server" });

    expect(fetchMock).toHaveBeenCalledTimes(1);
  });

  it("still answers when the browser will not store the server", async () => {
    vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => {
      throw new DOMException("blocked", "QuotaExceededError");
    });
    fetchMock.mockResolvedValue(jsonResponse(BOOTSTRAP));

    await expect(calls.bootstrap()).resolves.toMatchObject({ server: { id: "s-2" } });
    vi.spyOn(Storage.prototype, "getItem").mockImplementation(() => {
      throw new DOMException("blocked", "SecurityError");
    });
    await request("/api/admin/channels", { method: "POST" });
    expect(sentRequest(fetchMock, 1).headers.get("X-Stellarr-Server")).toBe("s-2");
  });
});
