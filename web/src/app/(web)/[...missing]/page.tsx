import { notFound } from "next/navigation";

/**
 * Every address that no other route claims. The web client and Cockpit each
 * have a root layout of their own and nothing sits above them, so a root
 * not-found page would have no layout to render in. This route takes such an
 * address into the web client's layout, where `not-found.tsx` answers it.
 */
export default function UnknownAddress(): never {
  notFound();
}
