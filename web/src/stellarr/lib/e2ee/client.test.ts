import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { ApiError } from "@/lib/api/errors";
import type { Message } from "@/lib/api/types";
import { FakeEncryptionServer } from "@/test/e2eeServer";

import { fromBase64, toBase64, utf8 } from "./bytes";
import { createE2eeClient, E2eeError, LAZY_ATTACHMENT_PREFIX, type E2eeEvent } from "./client";
import { exportPrivateKey, generateKeyPair } from "./keys";
import { encryptPayload } from "./message";
import { importEpochKey } from "./epoch";
import { sealBackup } from "./pin";
import { createMemoryKeyStore } from "./store";

const PIN_ADA = "135790";
const PIN_OLA = "246802";
const ITERATIONS = 1_000;

const problem = (status: number, code: string, details: Record<string, unknown> = {}) =>
  new ApiError("server", code, status, { code, details: { code, ...details } });

function browser(
  server: FakeEncryptionServer,
  userId: string,
  session: string,
  store = createMemoryKeyStore(),
  extra: Partial<Parameters<typeof createE2eeClient>[0]> = {},
) {
  const client = createE2eeClient({
    api: server.api(userId, session),
    store,
    probe: async () => "supported",
    deviceLabel: () => "Test, Linux",
    pinIterations: ITERATIONS,
    ...extra,
  });
  return { client, store, userId, session };
}

async function pair() {
  const server = new FakeEncryptionServer("Ada", "Ola");
  const [adaId, olaId] = server.userIds();
  const ada = browser(server, adaId, "ada-1");
  const ola = browser(server, olaId, "ola-1");
  await ada.client.start(adaId);
  await ola.client.start(olaId);
  await ada.client.setUp(PIN_ADA);
  await ola.client.setUp(PIN_OLA);
  server.turnOn();
  const send = (who: { client: typeof ada.client }, text: string, clientMessageId: string = crypto.randomUUID()) =>
    who.client.send(server.conversationId, { text, uploads: [], clientMessageId });
  return { server, ada, ola, adaId, olaId, send };
}

const sendCalls = (server: FakeEncryptionServer) =>
  server.calls.filter((entry) => entry.method === "sendEncryptedDirectMessage");
const epochCalls = (server: FakeEncryptionServer) =>
  server.calls.filter((entry) => entry.method === "createConversationEpoch");
const stored = (server: FakeEncryptionServer, index = -1): Message => {
  const message = server.messages.at(index);
  if (message === undefined) throw new Error("no message");
  return message;
};

describe("encryption client", () => {
  describe("messages", () => {
    it("sends ciphertext only, and the other browser reads the plaintext", async () => {
      const { server, ada, ola, send } = await pair();

      const sent = await send(ada, "login: klub.kajakowy");

      const wire = stored(server);
      expect(wire.content).toBe("");
      expect(new TextDecoder().decode(fromBase64(wire.encrypted?.ciphertext ?? ""))).not.toContain("klub");
      expect(sent.content).toBe("login: klub.kajakowy");
      const read = await ola.client.decode(wire);
      expect(read.content).toBe("login: klub.kajakowy");
      expect(read.encrypted).toBeUndefined();
      expect(read.sealed).toBeUndefined();
      // Marked, so that nothing derived from it (a translation) is written to disk.
      expect(read.decrypted).toBe(true);
    });

    it("seals a changed ciphertext and a message naming files it does not carry", async () => {
      const { server, ada, ola, send } = await pair();
      await send(ada, "one");
      const wire = stored(server);
      const tampered = server.replaceCiphertext(wire.id, (() => {
        const bytes = fromBase64(wire.encrypted?.ciphertext ?? "");
        bytes[0] = (bytes[0] ?? 0) ^ 1;
        return bytes;
      })());
      expect((await ola.client.decode(tampered)).sealed).toEqual({ reason: "lost", count: 1 });

      // A valid plaintext that points at an upload the message does not list.
      const keys = await server.api(ola.userId, ola.session).getConversationKeys(server.conversationId);
      const epochId = keys.encryption.currentEpochId ?? "";
      const forged = await (async () => {
        const raw = crypto.getRandomValues(new Uint8Array(32));
        const sealed = await encryptPayload(await importEpochKey(raw), { conversationId: server.conversationId, epochId, authorId: ada.userId }, {
          v: 1,
          text: "x",
          attachments: [{ id: "someone-elses", key: toBase64(raw), iv: toBase64(raw.slice(0, 12)), name: "a", type: "image/png", size: 1 }],
        });
        return { ...wire, id: "forged", encrypted: { epochId, iv: toBase64(sealed.iv), ciphertext: toBase64(sealed.ciphertext) } };
      })();
      expect((await ola.client.decode(forged)).sealed?.reason).toBe("lost");
    });

    it("passes a plain message and a timeline line through untouched", async () => {
      const { ola, server } = await pair();
      const plain: Message = { id: "p", channelId: server.conversationId, author: { id: "x", displayName: "X" }, content: "hi", attachments: [], embeds: [], createdAt: "2026-11-18T10:00:00Z" };
      const line: Message = { ...plain, id: "l", content: "", line: { kind: "device_added", actorId: "x" } };
      expect(await ola.client.decode(plain)).toBe(plain);
      expect(await ola.client.decode(line)).toBe(line);
    });
  });

  describe("epochs", () => {
    it("rotates when the other person adds a browser, which reads new messages at once and the history after the PIN", async () => {
      const { server, ada, olaId, send } = await pair();
      await send(ada, "before");
      const before = stored(server);

      const ola2 = browser(server, olaId, "ola-2");
      await ola2.client.start(olaId);
      expect((await ola2.client.decode(before)).sealed).toEqual({ reason: "pin", count: 1 });

      // Ada's cached epoch no longer covers Ola's new device: the server says so
      // and the same message goes out again in a new epoch.
      const clientMessageId = crypto.randomUUID();
      await send(ada, "after", clientMessageId);
      expect(server.epochs).toHaveLength(2);
      const deviceId = ola2.client.getStatus().deviceId;
      expect(server.epochs[1]?.epoch.recipients.map((key) => key.keyId)).toContain(deviceId);
      const sends = sendCalls(server).slice(-2).map((entry) => (entry.input as { clientMessageId: string }).clientMessageId);
      expect(sends).toEqual([clientMessageId, clientMessageId]);
      expect((await ola2.client.decode(stored(server))).content).toBe("after");

      expect(await ola2.client.unlock("000000")).toEqual({ kind: "wrong-pin", remainingAttempts: 9 });
      const events: E2eeEvent[] = [];
      ola2.client.subscribe((event) => events.push(event));
      expect(await ola2.client.unlock(PIN_OLA)).toEqual({ kind: "unlocked" });
      expect(events).toContainEqual({ type: "keys" });
      expect((await ola2.client.decode(before)).content).toBe("before");
    });

    it("leaves a removed browser out of the next epoch", async () => {
      const { server, ada, olaId, send } = await pair();
      const ola2 = browser(server, olaId, "ola-2");
      await ola2.client.start(olaId);
      await send(ada, "while both");
      const removed = ola2.client.getStatus().deviceId ?? "";
      server.revokeDevice(removed);

      await send(ada, "after removal");

      const last = server.epochs.at(-1)?.epoch;
      expect(last?.recipients.map((key) => key.keyId)).not.toContain(removed);
      expect((await ola2.client.decode(stored(server))).sealed).toBeDefined();
    });

    it("adopts the epoch someone else created first instead of making another (epoch-stale)", async () => {
      const { server, ada, ola, send } = await pair();
      // Ada reads the keys while there is no epoch yet; Ola then creates one.
      await ada.client.safetyCode(server.conversationId);
      await send(ola, "first");

      await send(ada, "second");

      expect(server.epochs).toHaveLength(1);
      expect(epochCalls(server).filter((entry) => entry.userId === ada.userId)).toHaveLength(1);
      expect((await ola.client.decode(stored(server))).content).toBe("second");
    });

    it("wraps again for today's keys when the coverage was refused (epoch-coverage)", async () => {
      const { server, ada, olaId, send } = await pair();
      await ada.client.safetyCode(server.conversationId);
      const ola2 = browser(server, olaId, "ola-2");
      await ola2.client.start(olaId);

      await send(ada, "hello");

      const fromAda = epochCalls(server).filter((entry) => entry.userId === ada.userId);
      expect(fromAda).toHaveLength(2);
      expect((await ola2.client.decode(stored(server))).content).toBe("hello");
    });

    it("picks another epoch id when the first is taken (epoch-exists)", async () => {
      const { server, ada, send } = await pair();
      server.failNext("createConversationEpoch", problem(409, "epoch-exists"));

      await send(ada, "hello");

      const ids = epochCalls(server).map((entry) => (entry.input as { epochId: string }).epochId);
      expect(ids).toHaveLength(2);
      expect(ids[0]).not.toBe(ids[1]);
      expect(server.messages).toHaveLength(1);
    });

    it("sends again under the same id when the epoch is not current (epoch-not-current)", async () => {
      const { server, ada, send } = await pair();
      await send(ada, "first");
      server.failNext("sendEncryptedDirectMessage", problem(409, "epoch-not-current", { currentEpochId: null }));

      const clientMessageId = crypto.randomUUID();
      await send(ada, "second", clientMessageId);

      const ids = sendCalls(server).slice(-2).map((entry) => (entry.input as { clientMessageId: string }).clientMessageId);
      expect(ids).toEqual([clientMessageId, clientMessageId]);
      expect(server.messages).toHaveLength(2);
    });

    it("never sends once the conversation is off (encryption-off)", async () => {
      const { server, ada, send } = await pair();
      server.encryption = { ...server.encryption, state: "off", enabledAt: null, currentEpochId: null };

      await expect(send(ada, "secret")).rejects.toEqual(new E2eeError("encryption-off"));
      expect(sendCalls(server)).toHaveLength(0);

      server.turnOn();
      await send(ada, "warm-up");
      server.failNext("sendEncryptedDirectMessage", problem(409, "encryption-off"));
      await expect(send(ada, "secret")).rejects.toBeInstanceOf(E2eeError);
    });

    it("still lists older epochs after a newer one arrived from the hub", async () => {
      const { server, ada, ola, olaId, send } = await pair();
      await send(ada, "in the first epoch");
      const first = stored(server);
      const ola2 = browser(server, olaId, "ola-2");
      await ola2.client.start(olaId);
      await send(ada, "in the second epoch");

      // A fresh start of Ola's browser hears of epoch 2 before it ever lists.
      const again = browser(server, olaId, "ola-1", ola.store);
      await again.client.start(olaId);
      const [second] = (await server.api(olaId, "ola-1").listConversationEpochs(server.conversationId, 1)).epochs;
      again.client.epochCreated(server.conversationId, second!);

      expect((await again.client.decode(first)).content).toBe("in the first epoch");
    });

    it("registers one device when two tabs of a browser start at once", async () => {
      const { server, ada, adaId } = await pair();
      let queue: Promise<unknown> = Promise.resolve();
      const withLock = <T,>(_name: string, task: () => Promise<T>): Promise<T> => {
        const run = queue.then(task);
        queue = run.then(() => undefined, () => undefined);
        return run;
      };
      const store = createMemoryKeyStore();
      const tabs = [browser(server, adaId, "ada-9", store, { withLock }), browser(server, adaId, "ada-9", store, { withLock })];
      await Promise.all(tabs.map((tab) => tab.client.start(adaId)));

      const devices = server.person(adaId).devices.filter((device) => device.session === "ada-9");
      expect(devices).toHaveLength(1);
      expect(tabs.map((tab) => tab.client.getStatus().deviceId)).toEqual([devices[0]?.id, devices[0]?.id]);
      expect(ada.client.getStatus().deviceId).not.toBe(devices[0]?.id);
    });

    it("gives up after a few rounds rather than loop", async () => {
      const { server, ada, send } = await pair();
      for (let round = 0; round < 6; round += 1) {
        server.failNext("createConversationEpoch", problem(409, "epoch-stale", { currentEpochId: null }));
      }
      await expect(send(ada, "x")).rejects.toEqual(new E2eeError("gave-up"));
    });
  });

  describe("a browser that cannot encrypt", () => {
    it("shows encrypted messages sealed with the reason, and never sends", async () => {
      const { server, ada, olaId, send } = await pair();
      await send(ada, "secret");
      const old = browser(server, olaId, "ola-old", createMemoryKeyStore(), { probe: async () => "unsupported" });
      await old.client.start(olaId);

      expect(old.client.getStatus().support).toBe("unsupported");
      expect((await old.client.decode(stored(server))).sealed).toEqual({ reason: "unsupported", count: 1 });
      await expect(send(old, "x")).rejects.toEqual(new E2eeError("unsupported"));
      expect(server.person(olaId).devices.some((device) => device.session === "ola-old")).toBe(false);
    });
  });

  describe("the switch", () => {
    it("adopts the state the server reports when it moved first (encryption-state)", async () => {
      const { server, ada } = await pair();
      const encryption = await ada.client.switchConversation(server.conversationId, "request");
      expect(encryption.state).toBe("on");
    });
  });

  describe("the account key and the PIN", () => {
    it("reads the account set up in another tab instead of failing (account-key-exists)", async () => {
      const { server, adaId } = await pair();
      const otherTab = browser(server, adaId, "ada-1");
      await otherTab.client.start(adaId);
      await otherTab.client.setUp(PIN_ADA);
      expect(otherTab.client.getStatus()).toMatchObject({ accountKey: { keyId: expect.any(String) }, unlocked: false });
    });

    it("counts wrong PINs down and reports the deleted backup after the tenth", async () => {
      const { server, olaId } = await pair();
      const ola2 = browser(server, olaId, "ola-2");
      await ola2.client.start(olaId);
      for (let attempt = 1; attempt <= 9; attempt += 1) {
        expect(await ola2.client.unlock("000000")).toEqual({ kind: "wrong-pin", remainingAttempts: 10 - attempt });
      }
      expect(await ola2.client.unlock("000000")).toEqual({ kind: "backup-deleted" });
      expect(ola2.client.getStatus().backup).toBeNull();
      expect(await ola2.client.unlock(PIN_OLA)).toEqual({ kind: "backup-missing" });
    });

    it("does not count an attempt the server refused for the rate", async () => {
      const { server, olaId } = await pair();
      const ola2 = browser(server, olaId, "ola-2");
      await ola2.client.start(olaId);
      server.failNext("unlockBackup", new ApiError("rate-limited", "429", 429));
      expect(await ola2.client.unlock(PIN_OLA)).toEqual({ kind: "rate-limited" });
      expect(await ola2.client.unlock(PIN_OLA)).toEqual({ kind: "unlocked" });
    });

    it("refuses a backup that opens with the PIN but holds another key", async () => {
      const { server, olaId } = await pair();
      const other = await generateKeyPair(true);
      const sealed = await sealBackup(await exportPrivateKey(other.privateKey), PIN_OLA, ITERATIONS);
      server.person(olaId).backup = {
        salt: toBase64(sealed.salt),
        iterations: sealed.iterations,
        verifier: toBase64(sealed.verifier),
        iv: toBase64(sealed.iv),
        ciphertext: toBase64(sealed.ciphertext),
        failed: 0,
      };
      const ola2 = browser(server, olaId, "ola-2");
      await ola2.client.start(olaId);

      expect(await ola2.client.unlock(PIN_OLA)).toEqual({ kind: "backup-unusable" });
      expect(ola2.client.getStatus().unlocked).toBe(false);
      expect(await ola2.store.getAccountKey(olaId)).toBeNull();
    });

    it("writes a new PIN that a new browser then opens", async () => {
      const { server, ola, olaId } = await pair();
      await ola.client.changePin(await ola.client.prepareNewPin("111111"), "password");

      const ola2 = browser(server, olaId, "ola-2");
      await ola2.client.start(olaId);
      expect(await ola2.client.unlock(PIN_OLA)).toEqual({ kind: "wrong-pin", remainingAttempts: 9 });
      expect(await ola2.client.unlock("111111")).toEqual({ kind: "unlocked" });
    });

    it("registers this browser and tries again when the server has no device for it (device-missing)", async () => {
      const { server, ola } = await pair();
      server.revokeDevice(ola.client.getStatus().deviceId ?? "");
      await ola.client.changePin(await ola.client.prepareNewPin("111111"), "password");
      expect(server.calls.filter((entry) => entry.method === "replaceBackup")).toHaveLength(2);
    });

    it("drops a key a reset elsewhere replaced (account-key-changed)", async () => {
      const { server, ola, olaId } = await pair();
      const prepared = await ola.client.prepareNewPin("111111");
      server.person(olaId).accountKey = { keyId: "replaced", algorithm: "x25519", publicKey: toBase64(new Uint8Array(32)), createdAt: "x" };

      await expect(ola.client.changePin(prepared, "password")).rejects.toEqual(new E2eeError("key-changed"));
      expect(await ola.store.getAccountKey(olaId)).toBeNull();
      expect(ola.client.getStatus().unlocked).toBe(false);
    });

    it("hands the server's refusal of the password or the second step back as it came", async () => {
      const { server, ola } = await pair();
      server.failNext("replaceBackup", problem(403, "fresh-mfa-required"));
      await expect(ola.client.changePin(await ola.client.prepareNewPin("111111"), "password")).rejects.toMatchObject({
        code: "fresh-mfa-required",
      });
    });

    it("after a reset, the other side's code changes and new messages reach the new keys", async () => {
      const { server, ada, ola, olaId, send } = await pair();
      await send(ola, "old");
      const old = stored(server);
      const codeBefore = await ada.client.safetyCode(server.conversationId);

      const ola2 = browser(server, olaId, "ola-2");
      await ola2.client.start(olaId);
      await ola2.client.reset(await ola2.client.prepareReset("222222"), "password");
      ada.client.keysChanged(server.conversationId);

      const codeAfter = await ada.client.safetyCode(server.conversationId);
      expect(codeAfter?.groups).not.toEqual(codeBefore?.groups);
      await send(ada, "new");
      expect((await ola2.client.decode(stored(server))).content).toBe("new");
      expect((await ola2.client.decode(old)).sealed?.reason).toBe("lost");
      expect(ola.client.getStatus().unlocked).toBe(true);
    });
  });

  describe("files", () => {
    const urls = new Map<string, Blob>();
    beforeEach(() => {
      let next = 0;
      vi.stubGlobal("URL", Object.assign(URL, {
        createObjectURL: vi.fn((blob: Blob) => {
          next += 1;
          const url = `blob:test/${next}`;
          urls.set(url, blob);
          return url;
        }),
        revokeObjectURL: vi.fn(),
      }));
    });
    afterEach(() => {
      urls.clear();
      vi.unstubAllGlobals();
    });

    it("uploads each file sealed, shows a picture once decrypted and decrypts a document when opened", async () => {
      const { server, ada, ola } = await pair();
      const picture = await ada.client.uploadFile(new File([new Uint8Array([137, 80, 78, 71])], "tile.png", { type: "image/png" }), { width: 2, height: 2 });
      const document = await ada.client.uploadFile(new File(["tajny raport"], "raport.txt", { type: "text/plain" }), {});
      expect(new Uint8Array(server.uploads.get(document.payload.id) ?? new ArrayBuffer(0))).not.toEqual(utf8("tajny raport"));

      await ada.client.send(server.conversationId, { text: "", uploads: [picture, document], clientMessageId: crypto.randomUUID() });
      const wire = stored(server);
      expect(wire.attachments.map((attachment) => attachment.name)).toEqual(["encrypted.bin", "encrypted.bin"]);

      const ready = new Promise<void>((resolve) => ola.client.subscribe((event) => event.type === "attachments" && resolve()));
      const first = await ola.client.decode(wire);
      expect(first.attachments.map((attachment) => [attachment.name, attachment.url])).toEqual([
        ["tile.png", ""],
        ["raport.txt", `${LAZY_ATTACHMENT_PREFIX}${document.payload.id}`],
      ]);
      await ready;
      const shown = await ola.client.decode(wire);
      expect(shown.attachments[0]).toMatchObject({ contentType: "image/png", url: expect.stringMatching(/^blob:test\//), width: 2 });

      const video = await ada.client.uploadFile(new File([new Uint8Array([0, 0, 0, 24])], "clip.mp4", { type: "video/mp4" }), {
        durationSeconds: 3,
        poster: new Blob([new Uint8Array([255, 216, 255])], { type: "image/jpeg" }),
      });
      expect(video.attachmentIds).toHaveLength(2);
      await ada.client.send(server.conversationId, { text: "", uploads: [video], clientMessageId: crypto.randomUUID() });
      const clip = stored(server);
      const posterReady = new Promise<void>((resolve) =>
        ola.client.subscribe((event) => event.type === "attachments" && event.messageId === clip.id && resolve()),
      );
      expect((await ola.client.decode(clip)).attachments[0]).toMatchObject({
        contentType: "video/mp4",
        url: `${LAZY_ATTACHMENT_PREFIX}${video.payload.id}`,
        durationSeconds: 3,
      });
      await posterReady;
      const withPoster = (await ola.client.decode(clip)).attachments[0];
      expect(new Uint8Array(await urls.get(withPoster?.posterUrl ?? "")!.arrayBuffer())).toEqual(new Uint8Array([255, 216, 255]));

      const photo = await ada.client.uploadFile(new File([new Uint8Array([137, 80, 78, 71])], "big.png", { type: "image/png" }), {
        width: 4000,
        height: 3000,
        preview: new Blob([new Uint8Array([82, 73, 70, 70])], { type: "image/webp" }),
        placeholder: "AAAA",
      });
      expect(photo.payload.placeholder).toBe("AAAA");
      expect(photo.attachmentIds).toHaveLength(2);
      expect(photo.payload.preview).toMatchObject({ type: "image/webp", size: 4 });
      await ada.client.send(server.conversationId, { text: "", uploads: [photo], clientMessageId: crypto.randomUUID() });
      const wirePhoto = stored(server);
      expect(wirePhoto.attachments).toHaveLength(2);
      await ola.client.decode(wirePhoto);
      await vi.waitFor(async () => {
        const view = (await ola.client.decode(wirePhoto)).attachments[0];
        expect(view?.url).toMatch(/^blob:test/);
        expect(view?.placeholder).toBe("AAAA");
        expect(new Uint8Array(await urls.get(view?.previewUrl ?? "")!.arrayBuffer())).toEqual(new Uint8Array([82, 73, 70, 70]));
      });

      const opened = await ola.client.openAttachment(document.payload.id);
      expect(opened?.type).toBe("application/octet-stream");
      expect(await urls.get(opened?.url ?? "")?.text()).toBe("tajny raport");
    });
  });

  describe("signing out", () => {
    it("deletes this browser's device key and account key copy", async () => {
      const { ada, adaId } = await pair();
      expect(await ada.store.getDevice(adaId)).not.toBeNull();
      expect(await ada.store.getAccountKey(adaId)).not.toBeNull();

      await ada.client.forget();

      expect(await ada.store.getDevice(adaId)).toBeNull();
      expect(await ada.store.getAccountKey(adaId)).toBeNull();
      expect(ada.client.getStatus()).toMatchObject({ accountKey: null, unlocked: false, deviceId: null });
    });
  });
});
