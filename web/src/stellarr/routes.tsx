import { defineRoutes } from "@stellarr/router";

import ConfirmEmailPage from "./pages/ConfirmEmailPage";
import ForgotPasswordPage from "./pages/ForgotPasswordPage";
import HomePage from "./pages/HomePage";
import InvitePage from "./pages/InvitePage";
import LinkAccountPage from "./pages/LinkAccountPage";
import LoginPage from "./pages/LoginPage";
import NotFoundPage from "./pages/NotFoundPage";
import RegisterPage from "./pages/RegisterPage";
import ResetPasswordPage from "./pages/ResetPasswordPage";
import SetupPage from "./pages/SetupPage";

/**
 * The web client's addresses. Everything under /cockpit belongs to Cockpit,
 * a document of its own, so no route here claims it and a link there loads
 * that document.
 *
 * The pages come with the entry: together they weigh less than a round trip
 * costs, and the entry already holds the ports and the session every page
 * needs. The signed-in app, most of the client's code, is the one part loaded
 * on demand (`pages/loadApp.ts`), so no sign-in page waits for it: the home
 * page starts it, and the sign-in, registration and invite pages fetch it
 * while idle.
 *
 * infra/nginx.conf repeats these addresses, in `map $uri $stellarr_address`:
 * the container answers them with this document and 200, and any other with
 * the same document and 404. web/documents.test.ts fails when the two differ.
 */
export const routes = defineRoutes({
  routes: [
    { path: "/", page: HomePage },
    { path: "/login", page: LoginPage },
    { path: "/register", page: RegisterPage },
    { path: "/setup", page: SetupPage },
    { path: "/forgot-password", page: ForgotPasswordPage },
    { path: "/reset-password", page: ResetPasswordPage },
    { path: "/confirm-email", page: ConfirmEmailPage },
    { path: "/link-account", page: LinkAccountPage },
    { path: "/invite/:code", page: InvitePage },
  ],
  notFound: NotFoundPage,
});
