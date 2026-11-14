"use client";

import { useSearchParams } from "next/navigation";
import { Suspense } from "react";

import { singleQueryValue } from "@/lib/query";

import { ResetPasswordClient } from "./ResetPasswordClient";

function ResetPasswordFromLink() {
  // A repeated ?token= reads as none, since no mailed link produces it.
  const token = singleQueryValue(useSearchParams(), "token") ?? "";
  return <ResetPasswordClient token={token} />;
}

/**
 * Where the mailed reset link leads. The token is read from the address in
 * the browser, so the page is the same file for every link; the boundary is
 * where a static render stops before the query.
 */
export default function ResetPasswordPage() {
  return (
    <Suspense>
      <ResetPasswordFromLink />
    </Suspense>
  );
}
