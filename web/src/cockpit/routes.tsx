import { defineRoutes } from "@stellarr/router";

import { cockpitHref } from "./lib/paths";
import { BackupsPage } from "./pages/backups-page";
import { DashboardPage } from "./pages/dashboard-page";
import { IntegrationsPage } from "./pages/integrations-page";
import { LogsPage } from "./pages/logs-page";
import { NotFoundPage } from "./pages/not-found-page";
import { RuntimePage } from "./pages/runtime-page";
import { ServersPage } from "./pages/servers-page";
import { SettingsPage } from "./pages/settings-page";
import { UsersPage } from "./pages/users-page";

/**
 * Cockpit's addresses, all under /cockpit. Any other address under /cockpit
 * gets the panel's not-found page inside its shell; the web client's
 * addresses are claimed by none, so a link there loads the web client's
 * document.
 */
export const routes = defineRoutes({
  routes: [
    { path: cockpitHref("/"), page: DashboardPage },
    { path: cockpitHref("/users"), page: UsersPage },
    { path: cockpitHref("/servers"), page: ServersPage },
    { path: cockpitHref("/runtime"), page: RuntimePage },
    { path: cockpitHref("/logs"), page: LogsPage },
    { path: cockpitHref("/backups"), page: BackupsPage },
    { path: cockpitHref("/integrations"), page: IntegrationsPage },
    { path: cockpitHref("/settings"), page: SettingsPage },
  ],
  notFound: NotFoundPage,
});
