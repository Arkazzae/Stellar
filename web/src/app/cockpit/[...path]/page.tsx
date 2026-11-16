"use client";

import { NotFoundPage } from "stellarr-cockpit/pages/not-found-page";

/**
 * Any address under /cockpit that names no page of the panel gets the panel's
 * own not-found page, inside its shell. The panel renders in the browser
 * only, so the page is not a `notFound()` boundary, whose status the server
 * would never see; the address answers 200, as the panel's addresses always
 * have.
 */
export default NotFoundPage;
