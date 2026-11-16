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

/**
 * The web client's addresses. Everything under /cockpit belongs to Cockpit,
 * a document of its own, so no route here claims it and a link there loads
 * that document.
 *
 * The pages come with the entry: together they weigh less than a round trip
 * costs, and the entry already holds the ports and the session every page
 * needs. The signed-in app, most of the client's code, is the one part loaded
 * on demand, by the home page (`pages/HomePage.tsx`), so no sign-in page
 * waits for it.
 */
export const routes = defineRoutes({
  routes: [
    { path: "/", page: HomePage },
    { path: "/login", page: LoginPage },
    { path: "/register", page: RegisterPage },
    { path: "/forgot-password", page: ForgotPasswordPage },
    { path: "/reset-password", page: ResetPasswordPage },
    { path: "/confirm-email", page: ConfirmEmailPage },
    { path: "/link-account", page: LinkAccountPage },
    { path: "/invite/:code", page: InvitePage },
  ],
  notFound: NotFoundPage,
});
