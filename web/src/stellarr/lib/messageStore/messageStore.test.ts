import "fake-indexeddb/auto";

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { Message } from "@/ports";
import { message } from "@/test/messages";

import {
  CHANNEL_LIMIT,
  contiguousRun,
  createMessageStore,
  isStorable,
  WINDOW_LIMIT,
  wipeAll,
  wipeServer,
} from "./messageStore";

const NONE: ReadonlySet<string> = new Set();

function openStore(userId = "u-ada", serverId = "srv", channels = ["ch-general"]) {
  const store = createMessageStore({ userId, serverId });
  return { store, ready: store.keepOnly(channels) };
}

async function storeFor(userId = "u-ada", serverId = "srv", channels = ["ch-general"]) {
  const opened = openStore(userId, serverId, channels);
  await opened.ready;
  return opened.store;
}

function ids(messages: readonly Message[] | undefined): string[] {
  return (messages ?? []).map((item) => item.id);
}

describe("message store", () => {
  beforeEach(async () => {
    await wipeAll();
  });
  afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it("returns the messages and the cursor that were written together", async () => {
    const store = await storeFor();
    await store.write("ch-general", [message("a", 1), message("b", 2)], "cursor-7", NONE);

    const stored = await (await storeFor()).read("ch-general");
    expect(ids(stored?.messages)).toEqual(["a", "b"]);
    expect(stored?.cursor).toBe("cursor-7");
  });

  it("replaces the window of a channel on every write", async () => {
    const store = await storeFor();
    await store.write("ch-general", [message("a", 1), message("b", 2)], "c1", NONE);
    await store.write("ch-general", [message("z", 9)], "c2", NONE);

    const stored = await store.read("ch-general");
    expect(ids(stored?.messages)).toEqual(["z"]);
    expect(stored?.cursor).toBe("c2");
  });

  it("keeps only the newest messages of a channel", async () => {
    const store = await storeFor();
    const many = Array.from({ length: WINDOW_LIMIT + 20 }, (_, index) =>
      message(`m${String(index).padStart(3, "0")}`, index));
    await store.write("ch-general", many, "c", NONE);

    const stored = await store.read("ch-general");
    expect(stored?.messages).toHaveLength(WINDOW_LIMIT);
    expect(stored?.messages[0]?.id).toBe("m020");
    expect(stored?.messages.at(-1)?.id).toBe("m119");
  });

  it("keeps only the channels written most recently", async () => {
    const channels = Array.from({ length: CHANNEL_LIMIT + 2 }, (_, index) => `ch-${index}`);
    const store = await storeFor("u-ada", "srv", channels);
    // Only the clock is replaced: fake timers would stall the database itself.
    const now = vi.spyOn(Date, "now");
    for (const [index, channelId] of channels.entries()) {
      now.mockReturnValue(1_000 + index);
      await store.write(channelId, [message(`m${index}`, 1, { channelId })], "c", NONE);
    }

    expect(await store.read("ch-0")).toBeUndefined();
    expect(await store.read("ch-1")).toBeUndefined();
    expect(await store.read("ch-2")).toBeDefined();
    expect(await store.read(`ch-${CHANNEL_LIMIT + 1}`)).toBeDefined();
  });

  it("never writes a message the server has not confirmed", async () => {
    const store = await storeFor();
    await store.write(
      "ch-general",
      [message("a", 1), message("draft", 2)],
      "c",
      new Set(["draft"]),
    );

    expect(ids((await store.read("ch-general"))?.messages)).toEqual(["a"]);
  });

  describe("what belongs to a private conversation", () => {
    // Typed against the real message, so renaming a marker stops this compiling.
    const markers: Record<string, Partial<Message>> = {
      encrypted: { encrypted: { epochId: "e", iv: "iv", ciphertext: "ct" } },
      decrypted: { decrypted: true },
      sealed: { sealed: { reason: "pin", count: 1 } },
      line: { line: { kind: "encryption_enabled", actorId: "u-ada" } },
    };

    it.each(Object.entries(markers))("is never written when it carries `%s`", async (_name, extra) => {
      const store = await storeFor();
      await store.write("ch-general", [message("a", 1)], "c", NONE);
      await store.write("ch-general", [message("a", 1), message("b", 2, extra)], "c", NONE);

      expect(await store.read("ch-general")).toBeUndefined();
      expect(isStorable(message("b", 2, extra))).toBe(false);
    });

    it("is not written for a channel that is not a text channel of the server", async () => {
      const store = await storeFor("u-ada", "srv", ["ch-general"]);
      await store.write("dm-1", [message("a", 1, { channelId: "dm-1" })], "c", NONE);

      expect(await store.read("dm-1")).toBeUndefined();
    });
  });

  it("deletes the windows of channels that are gone, in the open server only", async () => {
    const first = await storeFor("u-ada", "srv", ["ch-general", "ch-old"]);
    await first.write("ch-general", [message("a", 1)], "c", NONE);
    await first.write("ch-old", [message("b", 1)], "c", NONE);
    const other = await storeFor("u-ada", "srv-2", ["ch-other"]);
    await other.write("ch-other", [message("c", 1)], "c", NONE);

    const reopened = await storeFor("u-ada", "srv", ["ch-general"]);

    expect(await reopened.read("ch-old")).toBeUndefined();
    expect(await reopened.read("ch-general")).toBeDefined();
    expect(await other.read("ch-other")).toBeDefined();
  });

  it("keeps the same channel id on two servers apart", async () => {
    const one = await storeFor("u-ada", "srv-1");
    const two = await storeFor("u-ada", "srv-2");
    await one.write("ch-general", [message("a", 1)], "c1", NONE);
    await two.write("ch-general", [message("b", 1)], "c2", NONE);

    expect(ids((await one.read("ch-general"))?.messages)).toEqual(["a"]);
    expect(ids((await two.read("ch-general"))?.messages)).toEqual(["b"]);
  });

  it("is empty for a different user, and stays theirs from then on", async () => {
    const ada = await storeFor("u-ada");
    await ada.write("ch-general", [message("a", 1)], "c", NONE);

    const grace = await storeFor("u-grace");
    expect(await grace.read("ch-general")).toBeUndefined();
    expect(await (await storeFor("u-ada")).read("ch-general")).toBeUndefined();
  });

  it("forgets one server and leaves the others", async () => {
    const one = await storeFor("u-ada", "srv-1");
    const two = await storeFor("u-ada", "srv-2");
    await one.write("ch-general", [message("a", 1)], "c", NONE);
    await two.write("ch-general", [message("b", 1)], "c", NONE);

    await wipeServer("srv-1");

    expect(await one.read("ch-general")).toBeUndefined();
    expect(await two.read("ch-general")).toBeDefined();
  });

  it("forgets everything, and a store from before cannot write it back", async () => {
    const store = await storeFor();
    await store.write("ch-general", [message("a", 1)], "c", NONE);

    await wipeAll();
    await store.write("ch-general", [message("b", 2)], "c", NONE);

    expect(await (await storeFor()).read("ch-general")).toBeUndefined();
  });

  it("answers as if there were no store when the database cannot be opened", async () => {
    const store = await storeFor();
    await store.write("ch-general", [message("a", 1)], "c", NONE);
    vi.stubGlobal("indexedDB", {
      open: () => {
        throw new Error("blocked in a private window");
      },
    });

    const broken = createMessageStore({ userId: "u-ada", serverId: "srv" });
    await expect(broken.keepOnly(["ch-general"])).resolves.toBeUndefined();
    await expect(broken.write("ch-general", [message("b", 2)], "c", NONE)).resolves.toBeUndefined();
    await expect(broken.read("ch-general")).resolves.toBeUndefined();
    await expect(wipeServer("srv")).resolves.toBeUndefined();
  });

  it("answers as if there were no store when there is no IndexedDB", async () => {
    vi.stubGlobal("indexedDB", undefined);

    const store = createMessageStore({ userId: "u-ada", serverId: "srv" });
    await expect(store.keepOnly(["ch-general"])).resolves.toBeUndefined();
    await expect(store.write("ch-general", [message("a", 1)], "c", NONE)).resolves.toBeUndefined();
    await expect(store.read("ch-general")).resolves.toBeUndefined();
    await expect(wipeAll()).resolves.toBeUndefined();
  });
});

describe("contiguousRun", () => {
  const bucket = [message("old", 1), message("a", 10), message("b", 11), message("c", 12)];

  it("leaves out what lies below the floor", () => {
    expect(ids(contiguousRun(bucket, message("a", 10)))).toEqual(["a", "b", "c"]);
  });

  it("takes the whole bucket when the run reaches the start of the channel", () => {
    expect(ids(contiguousRun(bucket, null))).toEqual(["old", "a", "b", "c"]);
  });
});
