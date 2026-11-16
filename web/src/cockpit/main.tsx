// The stylesheet first, ahead of any style the modules below bring with them.
import "./index.css";

import { Outlet, Router } from "@stellarr/router";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import { CockpitApp } from "./app";
import { routes } from "./routes";

/**
 * Cockpit's entry, which `web/cockpit/index.html` loads. The panel shares
 * nothing with the web client beside it: its own document, stylesheet,
 * language and session. Going from one to the other is a page load.
 */
const root = document.getElementById("root");
if (root === null) throw new Error("The document has no #root to render into.");

createRoot(root).render(
  <StrictMode>
    <Router routes={routes}>
      <CockpitApp>
        <Outlet />
      </CockpitApp>
    </Router>
  </StrictMode>,
);
