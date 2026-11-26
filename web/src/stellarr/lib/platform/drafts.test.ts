import "fake-indexeddb/auto";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { requestResult } from "@/lib/chatTranslation/idb";
import { clearDrafts, composerDraft, draftScope, readDraftFiles, writeDraftFiles } from "./drafts";

const dm = draftScope("ada", "dm", "one");
const thread = draftScope("ada", "server", "general", "root");

async function openEntry(scope: string, factory = composerDraft) {
  const entry = factory(scope);
  const stop = entry.subscribe(() => undefined);
  await vi.waitFor(() => expect(entry.get().loading).toBe(false));
  stop();
  return entry;
}

beforeEach(clearDrafts);

describe("local composer drafts", () => {
  it("restores encrypted text and code settings in the original conversation after a restart", async () => {
    const entry = await openEntry(dm);
    entry.change(draft => ({ ...draft, text: "private note", codeLanguage: "rust", manualCode: true }));
    await readDraftFiles(dm); // after the preceding write committed
    vi.resetModules();
    const fresh = await import("./drafts");
    expect((await openEntry(dm, fresh.composerDraft)).get().draft).toEqual(entry.get().draft);
    expect((await openEntry(thread)).get().draft.text).toBe("");
    expect((await openEntry(draftScope("ben", "dm", "one"))).get().draft.text).toBe("");
    const db = await requestResult(indexedDB.open("stellarr-composer-drafts"));
    const saved = await requestResult(db.transaction("drafts").objectStore("drafts").get(dm));
    expect(Object.keys(saved).sort()).toEqual(["body", "iv"]);
    expect(new TextDecoder().decode(saved.body)).not.toContain("private note");
    const key = await requestResult(db.transaction("keys").objectStore("keys").get("ada"));
    expect(key.extractable).toBe(false);
    db.close();
  });

  it("keeps the file names, order and exact bytes, then deletes sent or removed originals", async () => {
    const a = { id: "first", file: new File(["hello"], "załącznik.txt", { type: "text/plain", lastModified: 123 }) };
    const b = { id: "second", file: new File([new Uint8Array([0, 10, 255])], "photo.png", { type: "image/png" }) };
    await writeDraftFiles(dm, [], [a, b]);
    await writeDraftFiles(dm, [a, b], [b, a]);
    expect(await readDraftFiles(thread)).toEqual([]);
    const restored = await readDraftFiles(dm);
    expect(restored.map(item => item.file.name)).toEqual(["photo.png", "załącznik.txt"]);
    expect(new Uint8Array(await restored[0]!.file.arrayBuffer())).toEqual(new Uint8Array([0, 10, 255]));
    expect(restored[1]!.file.lastModified).toBe(123);
    await writeDraftFiles(dm, [b, a], [a]);
    expect((await readDraftFiles(dm)).map(item => item.id)).toEqual(["first"]);
    await clearDrafts();
    expect(await readDraftFiles(dm)).toEqual([]);
  });

  it("removes text, keys to file records and file bytes of a revoked server only", async () => {
    const { forgetServerDrafts } = await import("./drafts");
    const channel = draftScope("ada", "server", "general", "");
    const other = draftScope("ada", "another", "general", "");
    const item = { id: "f", file: new File(["bytes"], "note.txt") };
    for (const scope of [channel, thread, other, dm]) {
      await writeDraftFiles(scope, [], [item]);
    }
    await forgetServerDrafts("server");
    expect(await readDraftFiles(channel)).toEqual([]);
    expect(await readDraftFiles(thread)).toEqual([]);
    expect(await readDraftFiles(other)).toHaveLength(1);
    expect(await readDraftFiles(dm)).toHaveLength(1);
  });

  it("does not replay a delayed edit after logout", async () => {
    const entry = await openEntry(dm);
    entry.change(draft => ({ ...draft, text: "queued" }));
    await clearDrafts();
    entry.change(draft => ({ ...draft, text: "late" }));
    expect((await openEntry(dm)).get().draft.text).toBe("");
  });
});
