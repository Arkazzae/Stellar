import { ServerRail } from "@/components/shell/rail/ServerRail";
import { useMediaQuery } from "@/hooks/useMediaQuery";
import { initials } from "@/lib/format/format";
import {
  useActivities,
  useDirectory,
  useFriends,
  usePorts,
  useSession,
  useUi,
} from "@/state/hooks";

export function ServerRailContainer() {
  const { openPicker } = useActivities();
  const directory = useDirectory();
  const { server } = directory;
  const friends = useFriends();
  const ui = useUi();
  const session = useSession();
  const ports = usePorts();

  const activateServer = (serverId: string) => {
    directory.selectServer(serverId);
    ui.openServer();
    session.reload();
  };

  // Below 768 the column rides inside the rail drawer instead of being its
  // own grid track. On wider screens it stays open.
  const railIsDrawer = useMediaQuery("(max-width: 767px)", false);
  const open = !railIsDrawer || ui.railOpen;

  return (
    <ServerRail
      open={open}
      onClose={railIsDrawer ? () => ui.setRailOpen(false) : undefined}
      name={server.name}
      mark={initials(server.name)}
      activeServerId={server.id}
      servers={directory.servers.map((item) => ({
        id: item.id,
        name: item.name,
        mark: initials(item.name),
        ...(item.iconUrl === undefined ? {} : { iconUrl: item.iconUrl }),
      }))}
      {...(server.iconUrl === undefined ? {} : { iconUrl: server.iconUrl })}
      dmSpace={ui.space === "dm"}
      activityOpen={ui.activityOpen}
      onOpenActivities={() => {
        if (railIsDrawer) ui.setRailOpen(false);
        openPicker({ scope: "personal" });
      }}
      onOpenMessages={ui.openFriends}
      onOpenServer={ui.openServer}
      onSelectServer={activateServer}
      unread={friends.unread}
      onOpenClient={ports.desktop ? undefined : () => ui.openModal("client")}
      onCreateServer={() => ui.openModal("server-create")}
      onDiscoverServers={() => ui.openModal("server-discover")}
    />
  );
}
