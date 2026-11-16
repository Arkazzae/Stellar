// The stylesheet first, so the CSS modules of the components come after it,
// as the pages' styles did after the layout's.
import "./styles/globals.css";

import { Outlet, Router } from "@stellarr/router";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import { StellarrApp } from "./app";
import { CatalogUnavailable } from "./components/shell/CatalogUnavailable";
import { loadCatalog } from "./i18n";
import { documentLocale } from "./i18n/defaultLocale";
import { routes } from "./routes";

/**
 * The web client's entry, which `web/index.html` loads. By the time it runs,
 * the document's pre-paint scripts have put the language and the theme on
 * `<html>`, the page stands on the theme's canvas, and the document has
 * started fetching that language's catalog.
 *
 * The app renders once the catalog is here, so everything inside it can
 * translate without waiting. A catalog that cannot be fetched gets a notice
 * that needs none, with a way to load the page again.
 */
const container = document.getElementById("root");
if (container === null) throw new Error("The document has no #root to render into.");
const root = createRoot(container);
const locale = documentLocale();

loadCatalog(locale).then(
  () => {
    root.render(
      <StrictMode>
        <Router routes={routes}>
          <StellarrApp>
            <Outlet />
          </StellarrApp>
        </Router>
      </StrictMode>,
    );
  },
  () => {
    root.render(<CatalogUnavailable locale={locale} onReload={() => window.location.reload()} />);
  },
);
