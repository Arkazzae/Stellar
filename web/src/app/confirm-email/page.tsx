"use client";

import { useSearchParams } from "next/navigation";
import { Suspense } from "react";

import { singleQueryValue } from "@/lib/query";

import { ConfirmEmailClient } from "./ConfirmEmailClient";

function ConfirmEmailFromLink() {
  // A repeated ?token= reads as none, since no mailed link produces it.
  const token = singleQueryValue(useSearchParams(), "token") ?? "";
  return <ConfirmEmailClient token={token} />;
}

/**
 * Where the link mailed to a new address leads. The token is read from the
 * address in the browser, so the page is the same file for every link; the
 * boundary is where a static render stops before the query.
 */
export default function ConfirmEmailPage() {
  return (
    <Suspense>
      <ConfirmEmailFromLink />
    </Suspense>
  );
}
