import "fake-indexeddb/auto";

import { describe, expect, it } from "vitest";

import { exportPrivateKey, exportPublicKey, generateKeyPair, openLocally, sealLocally } from "./keys";
import { createIndexedDbKeyStore } from "./store";

describe("the browser's key store", () => {
  it("keeps a non-extractable device key and the account key sealed by a key of its own, per account", async () => {
    const store = createIndexedDbKeyStore(`test-${crypto.randomUUID()}`);
    const device = await generateKeyPair(false);
    const account = await generateKeyPair(true);
    const pkcs8 = await exportPrivateKey(account.privateKey);

    await store.putDevice({ userId: "u-1", privateKey: device.privateKey, publicKey: await exportPublicKey(device.publicKey) });
    await store.putAccountKey({ userId: "u-1", keyId: "k-1", publicKey: await exportPublicKey(account.publicKey), ...(await sealLocally(pkcs8)) });

    const kept = await store.getDevice("u-1");
    expect(kept?.privateKey.extractable).toBe(false);
    const sealed = await store.getAccountKey("u-1");
    expect(sealed?.wrappingKey.extractable).toBe(false);
    expect(sealed?.sealed).not.toEqual(pkcs8);
    expect(await openLocally(sealed!)).toEqual(pkcs8);
    expect(await store.getDevice("u-2")).toBeNull();
  });

  it("binds each conversation to one peer across tabs and keeps old comparisons for history", async () => {
    const name = "test-" + crypto.randomUUID();
    const a = createIndexedDbKeyStore(name);
    const b = createIndexedDbKeyStore(name);
    const results = await Promise.all([a.bindPeer("self", "dm", "peer"), b.bindPeer("self", "dm", "other")]);
    expect(results.filter(Boolean)).toHaveLength(1);
    const peerId = (await a.getPeer("self", "dm"))!;
    await a.putVerification({ userId: "self", peerId, code: "first", at: "then" });
    await b.setPlaintextAllowed("self", peerId, true);
    await a.putVerification({ userId: "self", peerId, code: "next", at: "now" });
    const confirmed = await b.getVerification("self", peerId);
    expect(confirmed?.previousCodes).toEqual(["first"]);
    expect(confirmed?.plaintextAllowed).toBeFalsy();
  });

  it("forgets the keys of one account on sign-out and keeps what was verified", async () => {
    const store = createIndexedDbKeyStore(`test-${crypto.randomUUID()}`);
    const device = await generateKeyPair(false);
    await store.putDevice({ userId: "u-1", privateKey: device.privateKey, publicKey: await exportPublicKey(device.publicKey) });
    await store.putDevice({ userId: "u-2", privateKey: device.privateKey, publicKey: await exportPublicKey(device.publicKey) });
    await store.putVerification({ userId: "u-1", peerId: "p", code: "00000", at: "2026-11-18T00:00:00Z" });

    await store.forget("u-1");

    expect(await store.getDevice("u-1")).toBeNull();
    expect(await store.getAccountKey("u-1")).toBeNull();
    expect(await store.getDevice("u-2")).not.toBeNull();
    expect(await store.getVerification("u-1", "p")).toMatchObject({ code: "00000" });
  });
});
