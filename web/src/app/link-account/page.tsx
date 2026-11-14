"use client";

import { useSearchParams } from "next/navigation";
import { Suspense } from "react";

import { singleQueryValue } from "@/lib/query";

import { LinkAccountResult, type LinkOutcome } from "./LinkAccountResult";

const OUTCOMES: readonly LinkOutcome[] = ["linked", "taken", "other", "cancelled", "failed"];

function LinkAccountFromRedirect() {
  const result = singleQueryValue(useSearchParams(), "result");
  // Anything the server did not send reads as a failure, never as a success.
  const outcome = OUTCOMES.find((candidate) => candidate === result) ?? "failed";
  return <LinkAccountResult outcome={outcome} />;
}

/**
 * Where the API sends the browser back after linking an identity. The result
 * is read from the address in the browser; the boundary is where a static
 * render stops before the query.
 */
export default function LinkAccountPage() {
  return (
    <Suspense>
      <LinkAccountFromRedirect />
    </Suspense>
  );
}
