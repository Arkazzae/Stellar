import type { AccountSecurity, IdentityProvider } from "@/lib/api/types";

/**
 * Where an account proves it is its owner with a fresh sign-in at a
 * provider: one without a password and without a second step. The providers
 * are the configured ones it is linked to, the same the server counts as its
 * ways in and accepts that sign-in from, so none is offered that would fail.
 * Null for any other account, which has a password or a second step to give.
 * The list is empty when none of its providers is configured any more.
 */
export function providerProof(security: AccountSecurity): IdentityProvider[] | null {
  if (security.hasPassword || security.methods.some((method) => !method.pending)) return null;
  return security.identityProviders.filter((provider) =>
    security.identities.some((identity) => identity.providerId === provider.id),
  );
}
