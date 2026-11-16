import { describe, expect, it, vi } from "vitest";
import { DEFAULT_SETTINGS, VoiceSettingsStore, storageKeyForUser } from "./settings";

function memoryStorage(seed: Record<string, string> = {}) {
  const map = new Map(Object.entries(seed));
  return {
    getItem: vi.fn((key: string) => map.get(key) ?? null),
    setItem: vi.fn((key: string, value: string) => {
      map.set(key, value);
    }),
    removeItem: vi.fn((key: string) => {
      map.delete(key);
    }),
  } as unknown as Storage;
}

describe("VoiceSettingsStore.reset", () => {
  it("saves the defaults and notifies even when nothing differs", () => {
    const storage = memoryStorage();
    const store = new VoiceSettingsStore(storage);
    store.setOwner("fresh");
    const listener = vi.fn();
    store.subscribe(listener);

    expect(store.reset()).toEqual(DEFAULT_SETTINGS);

    expect(listener).toHaveBeenCalledTimes(1);
    expect(JSON.parse(storage.getItem(storageKeyForUser("fresh")) ?? "null")).toEqual(
      DEFAULT_SETTINGS,
    );
  });

  it("still leaves update() silent when a patch changes nothing", () => {
    const store = new VoiceSettingsStore(memoryStorage());
    const listener = vi.fn();
    store.subscribe(listener);

    store.update({ inputGate: DEFAULT_SETTINGS.inputGate });

    expect(listener).not.toHaveBeenCalled();
  });
});

describe("VoiceSettingsStore.setOwner", () => {
  it("starts from defaults when the active account's own entry is corrupt", () => {
    const storage = memoryStorage({ [storageKeyForUser("owner")]: "][" });
    const store = new VoiceSettingsStore(storage);

    expect(store.setOwner("owner")).toEqual(DEFAULT_SETTINGS);
  });
});
