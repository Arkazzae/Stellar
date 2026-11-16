// The stylesheet first, so the CSS modules of the components come after it,
// as the pages' styles did after the layout's.
import "./styles/globals.css";

import { Outlet, Router } from "@stellarr/router";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import { StellarrApp } from "./app";
import { routes } from "./routes";

/**
 * The web client's entry, which `web/index.html` loads. By the time it runs,
 * the document's pre-paint scripts have put the language and the theme on
 * `<html>`, and the page stands on the theme's canvas.
 */
const root = document.getElementById("root");
if (root === null) throw new Error("The document has no #root to render into.");

createRoot(root).render(
  <StrictMode>
    <Router routes={routes}>
      <StellarrApp>
        <Outlet />
      </StellarrApp>
    </Router>
  </StrictMode>,
);
