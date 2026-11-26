import "fake-indexeddb/auto";
import { expect, it } from "vitest";
import { requestResult, transactionDone } from "@/lib/chatTranslation/idb";
import { setShareOwner, sharedFiles } from "./sharedFiles";

async function stage(owner: string | null, expires = Date.now() + 10000) {
  const id = crypto.randomUUID();
  const db = await requestResult(indexedDB.open("stellarr-incoming-shares"));
  const tx = db.transaction("shares", "readwrite");
  tx.objectStore("shares").put({ owner, expires, files: [new File(["a"], "a.txt")] }, id);
  await transactionDone(tx);
  db.close();
  return id;
}

it("preserves an anonymous handoff across login and lets only one window consume it", async () => {
  await setShareOwner(null);
  const id = await stage(null);
  await setShareOwner("ada");
  expect(await sharedFiles(id, "ada")).toHaveLength(1);
  expect(await sharedFiles(id, "ben", true)).toHaveLength(0);
  const taken = await Promise.all([sharedFiles(id, "ada", true), sharedFiles(id, "ada", true)]);
  expect(taken.flat()).toHaveLength(1);
});

it("discards expired batches and the old account's pending files on sign-out", async () => {
  await setShareOwner("ada");
  const expired = await stage("ada", Date.now() - 1);
  const id = await stage("ada");
  expect(await sharedFiles(expired, "ada")).toEqual([]);
  await setShareOwner(null);
  expect(await sharedFiles(id, "ada")).toEqual([]);
});
