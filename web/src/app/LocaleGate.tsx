"use client";

import { usePathname } from "next/navigation";
import type { ReactNode } from "react";

import { useHydrated } from "@/hooks/useHydrated";
import { isInvitePath } from "@/lib/invites";

import { AppSkeleton } from "./AppSkeleton";

/**
 * Whether an address opens into the signed-in shell, whose skeleton is then
 * the right first paint: the home page and an invite, which joins and goes
 * there.
 */
export function opensIntoShell(pathname: string | null): boolean {
  return pathname === "/" || isInvitePath(pathname);
}

/**
 * Holds every page back until the browser has taken over.
 *
 * The client is rendered ahead of time, once for every person and every
 * instance, so its HTML cannot know the language: the person's choice and the
 * instance default exist only in the browser, where the locale script puts
 * them on `<html lang>` before the first paint. Any words in the HTML would be
 * in a language fixed at build time and would not match the first client
 * render. So the HTML and the render that hydrates it hold a placeholder
 * without words, and the page itself renders right after, already in the
 * language `<html lang>` names.
 *
 * The placeholder is the shell's skeleton where the shell is what loads, and
 * the bare canvas elsewhere. It depends on the address only through
 * `opensIntoShell`, which gives the same answer for the address a page was
 * rendered for and every address it is served at.
 */
export function LocaleGate({ children }: { children: ReactNode }) {
  const hydrated = useHydrated();
  const pathname = usePathname();

  if (hydrated) return children;
  return opensIntoShell(pathname) ? <AppSkeleton label={null} /> : null;
}
