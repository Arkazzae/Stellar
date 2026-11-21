import { useCallback, useEffect, useRef, useState } from "react";

import { ClientDownloadModal } from "@/components/settings/server/ClientDownloadModal";
import {
  ServerCreateModal,
  ServerDiscoverModal,
  type CreatedServerInvite,
  type ServerCreateInput,
} from "@/components/settings/server/ServerDirectoryModals";
import { getClientReleases } from "@/lib/platform/desktopRelease";
import { ApiError, describeError } from "@/lib/api/errors";
import { useDirectory, useLanguage, usePorts, useSession, useUi } from "@/state/hooks";

export const CREATED_SERVER_STORAGE_KEY = "stellarr.created-server";

/**
 * The server this tab has just created. Switching to it reloads the session,
 * which mounts the app again from nothing, dialogs included; the id carries
 * the create dialog over that, so it comes back on the new server with its
 * invite screen. It lives in `sessionStorage`, beside the active server, and
 * is forgotten when the invite screen is closed.
 */
function rememberCreatedServer(serverId: string): void {
  try {
    window.sessionStorage.setItem(CREATED_SERVER_STORAGE_KEY, serverId);
  } catch {
    // Without storage the server is still made and opened, without the invite screen.
  }
}

function createdServer(): string | null {
  try {
    return window.sessionStorage.getItem(CREATED_SERVER_STORAGE_KEY);
  } catch {
    return null;
  }
}

function forgetCreatedServer(): void {
  try {
    window.sessionStorage.removeItem(CREATED_SERVER_STORAGE_KEY);
  } catch {
    // Nothing to forget if storage cannot be written.
  }
}

/** The code at the end of an invite link, or the code itself when that is all that was pasted. */
function inviteCodeOf(value: string): string {
  const trimmed = value.trim();
  try {
    return new URL(trimmed).pathname.split("/").filter(Boolean).at(-1) ?? trimmed;
  } catch {
    const path = trimmed.split(/[?#]/, 1)[0] ?? trimmed;
    return path.split("/").filter(Boolean).at(-1) ?? trimmed;
  }
}

/** Creating a server, finding or joining one, and picking a client to install. */
export function ServerDirectoryDialogs() {
  const directory = useDirectory();
  const session = useSession();
  const ui = useUi();
  const { admin } = usePorts();
  const { t } = useLanguage();

  // Only the server the session came back on counts: a remembered id it did
  // not land on is from a switch that went elsewhere.
  const [createdId, setCreatedId] = useState(() => {
    const remembered = createdServer();
    return remembered === directory.server.id ? remembered : null;
  });
  useEffect(() => {
    if (createdId === null) forgetCreatedServer();
  }, [createdId]);
  const created = createdId !== null && createdId === directory.server.id ? directory.server : null;

  // The fresh app knows of no open dialog; the invite screen is one, so the
  // shortcuts and drawers underneath stay out of its way.
  const { openModal } = ui;
  const createdKey = created?.id ?? null;
  useEffect(() => {
    if (createdKey !== null) openModal("server-create");
  }, [createdKey, openModal]);

  // One link per new server, asked for once the app runs on it, so the
  // request is addressed to the new server and not the one left behind.
  const [invite, setInvite] = useState<CreatedServerInvite>({ state: "making" });
  const invited = useRef<string | null>(null);
  useEffect(() => {
    if (createdKey === null || invited.current === createdKey) return;
    invited.current = createdKey;
    admin.createInvite().then(
      (made) => setInvite({ state: "ready", url: made.url }),
      () => setInvite({ state: "failed" }),
    );
  }, [admin, createdKey]);

  // Joining a server reloads the session, so the person lands in it with its
  // channels and members rather than the one they left.
  const activateServer = useCallback(
    (serverId: string) => {
      directory.selectServer(serverId);
      ui.openServer();
      session.reload();
    },
    [directory, session, ui],
  );

  const createServer = useCallback(
    async (input: ServerCreateInput) => {
      let serverId: string;
      try {
        serverId = (await directory.createServer(input)).id;
      } catch (cause) {
        throw new Error(describeError(cause, t("shell.serverCreateFailed"), t));
      }
      rememberCreatedServer(serverId);
      setCreatedId(serverId);
      activateServer(serverId);
    },
    [activateServer, directory, t],
  );

  const closeCreate = () => {
    setCreatedId(null);
    ui.closeModal();
  };

  const loadPublicServers = useCallback(async () => {
    try {
      return await directory.discoverServers();
    } catch (cause) {
      throw new Error(describeError(cause, t("shell.serverDiscoveryFailed"), t));
    }
  }, [directory, t]);

  const joinPublicServer = useCallback(
    async (serverId: string) => {
      try {
        activateServer((await directory.joinServer(serverId)).id);
      } catch (cause) {
        throw new Error(describeError(cause, t("shell.serverJoinFailed"), t));
      }
    },
    [activateServer, directory, t],
  );

  const joinInvitedServer = useCallback(
    async (invite: string) => {
      try {
        activateServer((await directory.joinInvite(inviteCodeOf(invite))).id);
      } catch (cause) {
        throw new Error(
          cause instanceof ApiError && cause.status === 404
            ? t("shell.inviteInvalidDetail")
            : describeError(cause, t("shell.serverJoinFailed"), t),
        );
      }
    },
    [activateServer, directory, t],
  );

  return (
    <>
      <ClientDownloadModal
        open={ui.modal === "client"}
        onClose={ui.closeModal}
        loadReleases={getClientReleases}
      />
      <ServerCreateModal
        open={ui.modal === "server-create" || created !== null}
        onClose={closeCreate}
        onCreate={createServer}
        servers={directory.servers}
        created={created}
        invite={invite}
        onCopyInvite={(url) => navigator.clipboard.writeText(url)}
      />
      <ServerDiscoverModal
        open={ui.modal === "server-discover"}
        onClose={ui.closeModal}
        loadServers={loadPublicServers}
        onJoinServer={joinPublicServer}
        onJoinInvite={joinInvitedServer}
      />
    </>
  );
}
