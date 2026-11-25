import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { translateMessage, type MessageKey } from "@/i18n";
import { ApiError } from "@/lib/api/errors";
import { harness, resetHarness } from "@/test/modalsHarness";

import { ModalsContainer } from "./ModalsContainer";
import { CREATED_SERVER_STORAGE_KEY } from "./ServerDirectoryDialogs";

vi.mock("@/state/hooks", async () => (await import("@/test/modalsHarness")).hooksModule());

const t = (key: MessageKey, values?: Record<string, string | number>) =>
  translateMessage("pl", key, values);

const PUBLIC = { id: "srv-open", name: "Open House", description: "Come in" };

beforeEach(() => {
  resetHarness();
  sessionStorage.clear();
});
afterEach(() => vi.restoreAllMocks());

function expectServerActivated(serverId: string) {
  expect(harness.directory.selectServer).toHaveBeenCalledExactlyOnceWith(serverId);
  expect(harness.ui.openServer).toHaveBeenCalledOnce();
  expect(harness.session.reload).toHaveBeenCalledOnce();
}

describe("ModalsContainer: creating a server", () => {
  const INVITE = {
    id: "inv-1",
    url: "https://chat.example/invite/K7fQ2m",
    createdByName: "Ania",
    uses: 0,
    maxUses: null,
    expiresAt: null,
  };

  async function submit() {
    const user = userEvent.setup();
    harness.ui.modal = "server-create";
    const view = render(<ModalsContainer />);
    await user.type(screen.getByRole("textbox", { name: new RegExp(`^${t("shell.serverName")}`) }), "Garage");
    await user.click(screen.getByRole("button", { name: t("shell.createServerNow") }));
    return view;
  }

  /** Switching servers reloads the session, and the app mounts again on the new one. */
  function remountOn(server: { id: string; name: string }) {
    harness.directory.server = server;
    harness.ui.modal = null;
    render(<ModalsContainer />);
  }

  it("creates the server, switches to it, and makes its invite link once the app runs on it", async () => {
    harness.directory.createServer.mockResolvedValue({ id: "srv-new", name: "Garage" });
    const createInvite = vi.mocked(harness.ports.admin.createInvite).mockResolvedValue(INVITE);

    const first = await submit();

    await waitFor(() => expectServerActivated("srv-new"));
    expect(harness.directory.createServer).toHaveBeenCalledExactlyOnceWith({
      name: "Garage",
      description: "",
      isPublic: false,
      textChannelName: "ogólny",
      voiceChannelName: "rozmowa",
      mentionsOnly: true,
      voiceJoinSound: true,
    });
    // Asked before the switch, the link would be made on the server left behind.
    expect(createInvite).not.toHaveBeenCalled();

    first.unmount();
    remountOn({ id: "srv-new", name: "Garage" });

    expect(await screen.findByRole("status", { name: t("auth.invite.linkLabel") })).toHaveTextContent(INVITE.url);
    expect(createInvite).toHaveBeenCalledOnce();
    expect(harness.directory.selectServer.mock.invocationCallOrder[0]).toBeLessThan(
      createInvite.mock.invocationCallOrder[0]!,
    );

    await userEvent.setup().click(screen.getByRole("button", { name: t("common.done") }));
    expect(harness.ui.closeModal).toHaveBeenCalledOnce();
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(sessionStorage.getItem(CREATED_SERVER_STORAGE_KEY)).toBeNull();
  });

  it("opens the Discord wizard and creates a server from the reviewed scope", async () => {
    const installUrl = "https://discord.com/oauth2/authorize?client_id=5&scope=bot&permissions=66560";
    const server = { id: "5", name: "Stary serwer" };
    let listed = false;
    let previewed = false;
    const start = vi.fn((body: unknown) => {
      expect(body).toMatchObject({
        botToken: "example-bot-token",
        options: {
          sourceId: "5",
          name: "Przeniesiony serwer",
          channelIds: ["10"],
          history: { type: "since", since: new Date(2024, 0, 5).toISOString() },
          includeThreads: false,
        },
      });
      return Response.json({
        id: "job-1",
        state: "running",
        name: "Przeniesiony serwer",
        serverId: null,
        totalSources: null,
        completedSources: 0,
        channelsImported: 0,
        sectionsImported: 0,
        historyRequested: true,
      });
    });
    vi.spyOn(globalThis, "fetch").mockImplementation(async (input, init) => {
      switch (String(input)) {
        case "/api/servers/discord/imports/latest":
          return Response.json(null);
        case "/api/servers/discord/servers": {
          const servers = listed ? [server] : [];
          listed = true;
          return Response.json({ servers, installUrl });
        }
        case "/api/servers/discord/preview": {
          const messageContent = previewed;
          previewed = true;
          return Response.json({
            name: server.name,
            messageContent,
            restrictedChannels: 1,
            sections: [],
            channels: [
              { id: "10", name: "ogólny", kind: "text", canCopyHistory: true, hasOwnMessages: true },
              { id: "11", name: "pomoc", kind: "text", canCopyHistory: true, hasOwnMessages: true },
            ],
          });
        }
        case "/api/servers/discord/imports":
          return start(JSON.parse(String(init?.body)));
        case "/api/servers/discord/imports/job-1":
          return Response.json({
            id: "job-1",
            state: "completed",
            name: "Przeniesiony serwer",
            serverId: "srv-imported",
            totalSources: 1,
            completedSources: 1,
            channelsImported: 1,
            sectionsImported: 0,
            historyRequested: true,
            report: { messages: 25, skippedMessages: 2, messagesMissingAttachments: 0 },
          });
        default: throw new Error(String(input));
      }
    });

    const user = userEvent.setup();
    harness.ui.modal = "server-create";
    const view = render(<ModalsContainer />);
    await user.click(screen.getByRole("button", { name: t("discordImport.title") }));
    expect(harness.ui.openModal).toHaveBeenCalledWith("discord-import");
    harness.ui.modal = "discord-import";
    view.rerender(<ModalsContainer />);

    await user.type(await screen.findByLabelText(t("discordImport.token")), "example-bot-token");
    await user.click(screen.getByRole("button", { name: t("discordImport.connectButton") }));
    expect(await screen.findByRole("link", { name: t("discordImport.addBot") })).toHaveAttribute("href", installUrl);
    await user.click(screen.getByRole("button", { name: t("discordImport.refreshServers") }));
    expect(await screen.findByRole("combobox")).toHaveValue("5");
    await user.click(screen.getByRole("button", { name: t("shell.createServerNext") }));

    const serverName = await screen.findByRole("textbox", { name: t("shell.serverName") });
    await user.clear(serverName);
    await user.type(serverName, "Przeniesiony serwer");
    await user.click(screen.getByRole("checkbox", { name: "ogólny" }));
    expect(screen.getByRole("button", { name: t("shell.createServerNext") })).toBeDisabled();
    expect(screen.getByRole("radio", { name: t("discordImport.sinceDate") })).toBeDisabled();
    await user.click(screen.getByRole("button", { name: t("discordImport.refreshHistoryAccess") }));
    await waitFor(() => expect(screen.getByRole("radio", { name: t("discordImport.sinceDate") })).toBeEnabled());
    expect(screen.getByRole("checkbox", { name: "ogólny" })).toBeChecked();
    expect(screen.getByRole("button", { name: t("shell.createServerNext") })).toBeDisabled();
    await user.click(screen.getByRole("radio", { name: t("discordImport.sinceDate") }));
    fireEvent.change(screen.getByLabelText(t("discordImport.startDate")), { target: { value: "2024-01-05" } });
    await user.click(screen.getByRole("checkbox", { name: t("discordImport.threads") }));
    await user.click(screen.getByRole("button", { name: t("shell.createServerNext") }));

    expect(screen.getByRole("heading", { name: t("discordImport.review") })).toBeVisible();
    expect(screen.getByText(t("discordImport.reviewAttachments"))).toBeVisible();
    expect(start).not.toHaveBeenCalled();
    await user.click(screen.getByRole("button", { name: t("discordImport.start") }));
    await user.click(await screen.findByRole("button", { name: t("discordImport.openServer") }));
    expect(start).toHaveBeenCalledOnce();
    expectServerActivated("srv-imported");
    expect(harness.directory.createServer).not.toHaveBeenCalled();
    expect(Object.values(sessionStorage)).not.toContain("example-bot-token");
  });

  it("repairs attachments of the latest import without creating another server", async () => {
    const completed = {
      id: "job-1", state: "completed", name: "Arkaland", serverId: "srv-imported",
      totalSources: 1, completedSources: 1, channelsImported: 1, sectionsImported: 0,
      historyRequested: true, pendingAttachments: 2,
      report: { messages: 25, skippedMessages: 2, messagesMissingAttachments: 2 },
    };
    const resumed = vi.fn();
    vi.spyOn(globalThis, "fetch").mockImplementation(async (input, init) => {
      switch (String(input)) {
        case "/api/servers/discord/imports/latest":
          return Response.json(completed);
        case "/api/servers/discord/imports/job-1/resume":
          resumed(JSON.parse(String(init?.body)));
          return Response.json({ ...completed, state: "running" });
        case "/api/servers/discord/imports/job-1":
          return Response.json({
            ...completed, pendingAttachments: 0,
            report: { messages: 27, skippedMessages: 0, messagesMissingAttachments: 0 },
          });
        default: throw new Error(String(input));
      }
    });
    harness.ui.modal = "discord-import";
    const user = userEvent.setup();
    render(<ModalsContainer />);

    expect(await screen.findByRole("heading", { name: t("discordImport.serverReady") })).toBeVisible();
    expect(screen.getByRole("region", { name: t("discordImport.attachmentsPending") })).toHaveTextContent(
      t("discordImport.attachmentOnlyHelp"),
    );
    await user.click(screen.getByRole("button", { name: t("discordImport.restoreAttachments") }));
    await user.type(screen.getByLabelText(t("discordImport.token")), "example-bot-token");
    await user.click(screen.getByRole("button", { name: t("discordImport.startAttachments") }));
    await screen.findByRole("heading", { name: t("discordImport.completed") });
    await user.click(screen.getByRole("button", { name: t("discordImport.openServer") }));

    expect(resumed).toHaveBeenCalledExactlyOnceWith({ botToken: "example-bot-token" });
    expectServerActivated("srv-imported");
    expect(harness.directory.createServer).not.toHaveBeenCalled();
  });

  it("shows the structure imported for a completed job that excluded history", async () => {
    sessionStorage.setItem("stellarr.discord-import", "structure-job");
    harness.ui.modal = "discord-import";
    vi.spyOn(globalThis, "fetch").mockResolvedValue(Response.json({
      id: "structure-job",
      state: "completed",
      serverId: "srv-imported",
      name: "Structure",
      totalSources: 0,
      completedSources: 0,
      channelsImported: 19,
      sectionsImported: 2,
      historyRequested: false,
      report: { messages: 0, skippedMessages: 0, messagesMissingAttachments: 0 },
    }));

    render(<ModalsContainer />);

    expect(await screen.findByText(t("discordImport.structureResult"))).toBeVisible();
    expect(screen.getByText(t("discordImport.channelsImported")).nextElementSibling).toHaveTextContent("19");
    expect(screen.getByText(t("discordImport.sectionsImported")).nextElementSibling).toHaveTextContent("2");
    expect(screen.queryByText(t("discordImport.messagesCopied"))).toBeNull();
    await userEvent.setup().click(screen.getByRole("button", { name: t("discordImport.newImport") }));
    expect(screen.getByLabelText(t("discordImport.token"))).toBeVisible();
    expect(sessionStorage.getItem("stellarr.discord-import")).toBeNull();
  });

  it("shows the same link again when the page is loaded again, without making another", async () => {
    harness.directory.createServer.mockResolvedValue({ id: "srv-new", name: "Garage" });
    const createInvite = vi.mocked(harness.ports.admin.createInvite).mockResolvedValue(INVITE);

    (await submit()).unmount();
    remountOn({ id: "srv-new", name: "Garage" });
    await screen.findByText(INVITE.url);
    cleanup();

    remountOn({ id: "srv-new", name: "Garage" });

    expect(screen.getByRole("status", { name: t("auth.invite.linkLabel") })).toHaveTextContent(INVITE.url);
    expect(createInvite).toHaveBeenCalledOnce();
  });

  it("says so when the link could not be made", async () => {
    harness.directory.createServer.mockResolvedValue({ id: "srv-new", name: "Garage" });
    vi.mocked(harness.ports.admin.createInvite).mockRejectedValue(new ApiError("forbidden", "no", 403));

    (await submit()).unmount();
    remountOn({ id: "srv-new", name: "Garage" });

    // Read whole: the sentence keeps a one-letter word on the next one with a no-break space.
    expect((await screen.findByRole("alert")).textContent).toBe(t("auth.setup.inviteFailed"));
  });

  it("does not bring the invite back on a server the switch did not land on", () => {
    sessionStorage.setItem(CREATED_SERVER_STORAGE_KEY, JSON.stringify({ id: "srv-elsewhere" }));

    render(<ModalsContainer />);

    expect(screen.queryByRole("dialog")).toBeNull();
    expect(harness.ports.admin.createInvite).not.toHaveBeenCalled();
    expect(sessionStorage.getItem(CREATED_SERVER_STORAGE_KEY)).toBeNull();
  });

  it("stays on the current server and words the failure when creation is refused", async () => {
    harness.directory.createServer.mockRejectedValue(new ApiError("rate-limited", "slow", 429));

    await submit();

    expect(await screen.findByText(t("session.rateLimited"))).toBeVisible();
    expect(harness.directory.selectServer).not.toHaveBeenCalled();
  });
});

describe("ModalsContainer: finding a server", () => {
  async function openDiscover() {
    harness.ui.modal = "server-discover";
    harness.directory.discoverServers.mockResolvedValue([PUBLIC]);
    render(<ModalsContainer />);
    await screen.findByText(PUBLIC.name);
  }

  const joinByInvite = async (text: string) => {
    const user = userEvent.setup();
    await user.type(screen.getByRole("textbox", { name: t("shell.inviteCode") }), text);
    await user.click(screen.getByRole("button", { name: t("shell.joinInvite") }));
  };

  it("lists the public servers and joins the one picked", async () => {
    harness.directory.joinServer.mockResolvedValue({ id: PUBLIC.id, name: PUBLIC.name });
    await openDiscover();

    await userEvent.setup().click(screen.getByRole("button", { name: t("shell.joinServer") }));

    await waitFor(() => expectServerActivated(PUBLIC.id));
    expect(harness.directory.joinServer).toHaveBeenCalledExactlyOnceWith(PUBLIC.id);
  });

  it.each([
    ["a pasted link", "https://chat.example/invite/abc123", "abc123"],
    ["a link with a query", " https://chat.example/invite/abc123?ref=mail#top ", "abc123"],
    ["a path", "/invite/abc123/", "abc123"],
    ["a bare code", "abc123", "abc123"],
  ])("joins from %s by its code", async (_name, pasted, code) => {
    harness.directory.joinInvite.mockResolvedValue({ id: "srv-invited", name: "Invited" });
    await openDiscover();

    await joinByInvite(pasted);

    await waitFor(() => expectServerActivated("srv-invited"));
    expect(harness.directory.joinInvite).toHaveBeenCalledExactlyOnceWith(code);
  });

  it("says the invite is not valid when the server does not know it", async () => {
    harness.directory.joinInvite.mockRejectedValue(new ApiError("not-found", "no such invite", 404));
    await openDiscover();

    await joinByInvite("gone");

    expect(await screen.findByRole("alert")).toHaveTextContent(t("shell.inviteInvalidDetail"));
    expect(harness.directory.selectServer).not.toHaveBeenCalled();
  });

  it("words other join failures by their kind, or with its own sentence", async () => {
    harness.directory.joinInvite
      .mockRejectedValueOnce(new ApiError("forbidden", "banned", 403))
      .mockRejectedValueOnce(new TypeError("fetch failed"));
    await openDiscover();

    await joinByInvite("abc");
    expect(await screen.findByRole("alert")).toHaveTextContent(t("session.forbidden"));

    await userEvent.setup().click(screen.getByRole("button", { name: t("shell.joinInvite") }));
    await waitFor(() =>
      expect(screen.getByRole("alert")).toHaveTextContent(t("shell.serverJoinFailed")),
    );
  });

  it("words a failure to load the directory", async () => {
    harness.ui.modal = "server-discover";
    harness.directory.discoverServers.mockRejectedValue(new ApiError("network", "offline"));

    render(<ModalsContainer />);

    expect(await screen.findByRole("alert")).toHaveTextContent(t("session.network"));
  });
});

describe("ModalsContainer: client download", () => {
  it("lists the clients this instance publishes", async () => {
    const fetchSpy = vi.spyOn(globalThis, "fetch").mockResolvedValue(
      new Response(
        JSON.stringify([
          {
            version: "1.4.0",
            publishedAt: "2026-03-01T00:00:00.000Z",
            notes: "",
            platform: "android",
            architecture: "arm64-v8a",
            fileName: "stellarr-1.4.0.apk",
            sizeBytes: 1_048_576,
            sha256: "ab".repeat(32),
            downloadUrl: "/downloads/stellarr-1.4.0.apk",
          },
        ]),
        { status: 200 },
      ),
    );
    harness.ui.modal = "client";

    render(<ModalsContainer />);

    expect(await screen.findByText("Android")).toBeVisible();
    expect(fetchSpy).toHaveBeenCalledWith("/api/client/releases", expect.anything());
  });
});
