import { renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

// The launch instruction is consumed once per page load, which is module state.
async function freshModule() {
  vi.resetModules();
  return import("./callPopout");
}

function openedAt(search: string) {
  window.history.replaceState({}, "", `/${search}`);
}

const channels = [{ id: "ch-salon", kind: "voice" }] as const;
const server = "server-1";

beforeEach(() => {
  openedAt("");
  sessionStorage.clear();
  Object.defineProperty(window, "opener", { configurable: true, writable: true, value: null });
});

async function authorizedPopout(join: (channelId: string) => void) {
  const { popOutCall, useJoinFromPopoutLink } = await freshModule();
  const open = vi.spyOn(window, "open").mockReturnValue(null);
  popOutCall("ch-salon", server);
  window.opener = { sessionStorage };
  openedAt(new URL(String(open.mock.calls[0]![0])).search);
  return () => useJoinFromPopoutLink(join, server, channels);
}
afterEach(() => vi.restoreAllMocks());

describe("useJoinFromPopoutLink", () => {
  it("joins the authorized call once and consumes the handoff", async () => {
    const join = vi.fn();
    const usePopout = await authorizedPopout(join);
    const address = window.location.search;
    renderHook(usePopout).unmount();
    expect(join).toHaveBeenCalledExactlyOnceWith("ch-salon");
    expect(window.location.search).not.toContain("callHandoff");

    openedAt(address);
    const { useJoinFromPopoutLink } = await freshModule();
    renderHook(() => useJoinFromPopoutLink(join, server, channels));
    expect(join).toHaveBeenCalledOnce();
  });

  it("ignores a voice link without an opener's handoff", async () => {
    const { useJoinFromPopoutLink } = await freshModule();
    const join = vi.fn();
    window.opener = { sessionStorage };
    openedAt("?popout=1&serverId=server-1&voiceChannel=ch-salon&callHandoff=untrusted");

    renderHook(() => useJoinFromPopoutLink(join, server, channels));
    expect(join).not.toHaveBeenCalled();
  });

  it("does not dial a conversation outside the voice channel list", async () => {
    const join = vi.fn();
    const usePopout = await authorizedPopout(join);
    const params = new URLSearchParams(window.location.search);
    params.set("voiceChannel", "private-call");
    openedAt(`?${params}`);
    renderHook(usePopout);
    expect(join).not.toHaveBeenCalled();
  });

  it("ignores an address that names a call but is not a popout", async () => {
    const { useJoinFromPopoutLink } = await freshModule();
    const join = vi.fn();
    openedAt("?voiceChannel=ch-salon");

    renderHook(() => useJoinFromPopoutLink(join, server, channels));

    expect(join).not.toHaveBeenCalled();
  });

  it("ignores a popout address that names no call", async () => {
    const { useJoinFromPopoutLink } = await freshModule();
    const join = vi.fn();
    openedAt("?popout=1");

    renderHook(() => useJoinFromPopoutLink(join, server, channels));

    expect(join).not.toHaveBeenCalled();
  });

  it("does not wait for a later address once the first room has mounted", async () => {
    const { useJoinFromPopoutLink } = await freshModule();
    const join = vi.fn();

    renderHook(() => useJoinFromPopoutLink(join, server, channels));
    openedAt("?popout=1&voiceChannel=ch-salon");
    renderHook(() => useJoinFromPopoutLink(join, server, channels));

    expect(join).not.toHaveBeenCalled();
  });
});

describe("popOutCall", () => {
  it("opens this page in a small window that dials the call", async () => {
    const { popOutCall } = await freshModule();
    const open = vi.spyOn(window, "open").mockReturnValue(null);
    openedAt("?keep=this");

    popOutCall("ch-salon", "server-1");

    const [url, name, features] = open.mock.calls[0]!;
    const opened = new URL(String(url));
    expect(opened.searchParams.get("popout")).toBe("1");
    expect(opened.searchParams.get("voiceChannel")).toBe("ch-salon");
    expect(opened.searchParams.get("serverId")).toBe("server-1");
    expect(opened.searchParams.get("keep")).toBe("this");
    expect(name).toBe("stellarr-call");
    expect(features).toMatch(/popup/);
  });

  it("carries the new server when reusing the named call window", async () => {
    const { popOutCall } = await freshModule();
    const open = vi.spyOn(window, "open").mockReturnValue(null);
    popOutCall("voice-1", "server-1");
    popOutCall("voice-2", "server-2");

    expect(open.mock.calls.map(([, name]) => name)).toEqual(["stellarr-call", "stellarr-call"]);
    const second = new URL(String(open.mock.calls[1]![0]));
    expect(second.searchParams.get("serverId")).toBe("server-2");
    expect(second.searchParams.get("voiceChannel")).toBe("voice-2");
  });

  it("does not launch a call before a server has been selected", async () => {
    const { popOutCall } = await freshModule();
    const open = vi.spyOn(window, "open").mockReturnValue(null);

    popOutCall("voice-1", "");

    expect(open).not.toHaveBeenCalled();
  });
});
