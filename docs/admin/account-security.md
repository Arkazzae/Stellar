# Account security

Stellarr supports second-factor authentication (TOTP apps, email codes, and passkeys or hardware security keys), OIDC identity linking, passwordless accounts created through an OIDC provider, trusted devices, and operator-assisted recovery.

## Second-factor methods

Users manage their authentication methods under **Settings → Login and security**:

| Method | How it works | Notes |
| --- | --- | --- |
| **Authenticator app (TOTP)** | 6-digit, 30-second codes scanned via QR code or entered as a text key (seed shown only during enrollment) | Multiple named authenticators supported per account |
| **Email code** | 6-digit one-time code sent to the account's email address | Requires [Mail configuration](../install/configuration.md#mail); rate-limited (`mfa.email-cooldown-seconds`, `mfa.email-sends-per-hour`, `mfa.email-attempts`) |
| **Passkey or security key** | Device passkey (biometrics/PIN) or USB/NFC Fido2 hardware key | Discoverable passkeys also support passwordless sign-in (see [Passkeys](#passkeys)) |
| **Recovery codes** | Ten single-use backup codes generated when the first second-factor method is enrolled | Displayed once; regenerating invalidates all previous codes |

The same settings pane displays **trusted devices**, **active sessions** (allowing users to revoke individual sessions or sign out everywhere else), and the account's **security history**. Revoking a session immediately terminates its active SignalR WebSocket and voice/media connections and returns that browser to `/login`.

### Proof for sensitive account changes

Adding or removing second-factor methods, regenerating recovery codes, linking or unlinking an OIDC provider, changing or setting a password, changing the email address, or replacing/resetting end-to-end encryption PIN backups and account keys require fresh authentication proof (`FreshSecondStepFilter`):

- **Accounts with a second factor**: A second factor completed within the **last 5 minutes** in the current session (trusted devices do not qualify).
- **Accounts without a second factor**: Either the current password or a primary sign-in completed within the **last 5 minutes**.
- **Accounts without a password and without a second factor**: When the current session's sign-in is more than 5 minutes old, the settings UI redirects the user to re-authenticate at one of their linked OIDC providers (`/api/auth/oidc/{id}/reauthenticate`). Stellarr sends `prompt=login` and `max_age=300` to the provider, verifies that the returned identity belongs to the account and carries an `auth_time` claim within the last 5 minutes, and updates the session's authentication timestamp to that `auth_time` value.

Security-change endpoints are rate-limited to 30 requests per IP per 15 minutes (`RateLimitPolicies.SecurityWrite`).

## Passkeys

Passkeys require an HTTPS origin with a domain name: `STELLARR_PUBLIC_URL` or `STELLARR_DOMAIN` set to a hostname rather than an IP address (except `http://localhost` during local development). When no hostname origin is configured, the web client hides passkey enrollment.

> [!WARNING]
> Passkeys are cryptographically bound to the relying-party hostname (`Operations:PublicUrl`). **Changing `STELLARR_DOMAIN` or `STELLARR_PUBLIC_URL` invalidates all existing passkeys**; users must sign in with their password and another second factor (or a recovery code) and register a new passkey.

- **Passwordless sign-in**: A discoverable passkey enables **Continue with a passkey** on `/login`. Because it verifies both user presence and user verification (`uv`), it satisfies both primary authentication and second-factor requirements (including Cockpit access).
- **Second-factor-only keys**: Non-discoverable hardware security keys work as a second factor after password or OIDC sign-in.

## Linking an identity provider

When OIDC is configured (see [Single sign-on](../install/configuration.md#single-sign-on-oidc)), users can link external identities under **Settings → Login and security → Sign-in providers**:

- Linking and unlinking require fresh security proof.
- Identity links are bound strictly by `(issuer, subject)`. A provider identity can be linked to at most one Stellarr account, and an account can link at most one identity per configured provider.
- Identities are never matched or auto-linked by email address.
- By default, OIDC sign-in still requires Stellarr's own second factor unless `TrustProviderMfa` is enabled for that provider and the ID token contains a matching `amr` claim and a recent `auth_time`.
- Operators can inspect linked identities with `stellarr security show` and remove them in Cockpit or via `stellarr security unlink`. Operator MFA recovery preserves OIDC links.

### Signing in with an unlinked OIDC identity

When a user authenticates at an OIDC provider with an identity that is not yet linked to any Stellarr account, Stellarr stores the pending identity in the database for **10 minutes**, binds it to the browser via an HttpOnly cookie, and redirects to `/link-account`. To prevent account enumeration, `/link-account` displays the provider identity (display name, email, provider name) without revealing whether that email address is registered on the instance, and offers:

- **I have an account on {instance}**: The user signs in with their existing Stellarr username/email and password. Once authentication completes (including a real second factor if the account has one enabled; trusted devices are not accepted for linking), the OIDC identity is linked to the account and recorded in the security history. If the identity was linked to another account in the meantime, or the account already has a link for that provider, sign-in still succeeds and displays why the link was skipped.
- **I have an invite link**: Shown only when the instance owner has enabled account creation for that provider (see below).
- **Back to login**: Clears the pending identity cookie and returns to `/login`.

### Creating an account through a provider

In **Cockpit → Integrations → Sign-in providers**, the instance owner can allow each configured OIDC provider to create new accounts for invited users (`IdentityProviderSettings.SignUpAllowed`). This toggle defaults to **off** for every provider; changing it requires a fresh second factor and writes an operator audit entry.

When enabled for a provider, a user holding a valid server invite link can register via that provider—either from `/link-account` or by clicking **Create with {provider}, no password** on the invite registration page:

- The registration form pre-fills the display name, suggested username, and email address from the provider's claims, which the user can edit before submitting.
- The resulting account is created **without a local password**, linked to the provider's `(issuer, subject)`, and joined to the invite's server.
- The account's email address is marked verified only if the provider reported `email_verified: true` for that exact address.
- Subsequent sign-ins follow standard OIDC MFA rules (`TrustProviderMfa` with recent `auth_time`, or the instance's second-factor policy).

The instance owner account can never be created through an OIDC provider; an empty instance must be initialized at `/setup` with a password and the setup code.

## Accounts without a password

An account created through an OIDC provider has no local password (`PasswordHash == null`) until the user sets one under **Settings → Login and security** after completing fresh security proof:

- Password sign-in attempts against a passwordless account fail identically to an incorrect password.
- Changing the account's email address, or replacing/resetting end-to-end encryption PIN backups and account keys, uses the fresh security proof described above in place of a current password.
- **Last sign-in method protection (`WaysIn`)**: An account's last remaining way to sign in independently (a password, a confirmed passkey, or an identity at an actively configured OIDC provider) cannot be unlinked or deleted in user settings or Cockpit (`409 last-way-in`). The user must add another sign-in method first. In the operator CLI, `stellarr security unlink` warns before unlinking an account's last configured way in.
- **Forgot password** (when mail is configured) and **Operator recovery** both work on passwordless accounts.

## Changing an account's email address

Because the account email receives email 2FA codes, security alerts, and password-reset links, changing it under **Settings → Login and security** requires the current password (or fresh proof for a passwordless account), a fresh second factor (if enabled), and configured mail delivery:

1. Stellarr sends a confirmation link (valid for 60 minutes and bound to the requesting session) to the **new** email address.
2. The account's email address updates only after the confirmation link is opened and submitted.
3. The **old** email address receives a security alert, and any pending codes or reset links sent to the old address are invalidated.

## Trusted devices

During second-factor sign-in, users can check **Don't ask for a code on this device for N days** (controlled by `mfa.trusted-device-days`, `14` days by default):

- A trusted device skips the second-factor prompt during standard sign-in only.
- It **never** satisfies the fresh 5-minute second-factor requirement for Cockpit, OIDC identity linking, or sensitive account/server changes.

## Session lifetimes

- **Default (`Keep me signed in` checked)**: Sessions remain valid for 30 days.
- **Unchecked**: Issues a browser-session cookie with a hard 12-hour server-side expiration limit (even if the browser restores closed tabs).
- **Desktop browser handoff**: Sessions issued via `/api/auth/desktop/browser/*` are always persistent (30 days) because the desktop client maintains its own cookie store across restarts.

Modifying second-factor methods revokes all other active sessions on the account. Signing out, revoking a session remotely, or running operator recovery terminates active SignalR and voice/media connections within 1 second (`SessionRevocationWatcher`) and deactivates any end-to-end encryption device bound to that session.

## Instance MFA policy (operators)

The instance owner configures the instance-wide second-factor policy in **Cockpit → Settings → Account security** or via the [operator CLI](cli.md):

```bash
docker compose exec stellarr stellarr config set mfa.requirement everyone --yes
```

| Value | Effect |
| --- | --- |
| `optional` | Default. Users choose whether to enable 2FA; accounts with a confirmed method are prompted on sign-in. |
| `admins` | Required for the instance owner and any member holding a role with **Manage server**. |
| `everyone` | Required for every account on the instance. |
| `cockpit` | Standard client sign-in follows each account's methods; accessing Cockpit requires a second factor. |

Under `admins` or `everyone`, users without an enrolled second factor are guided through setting one up on their next sign-in before accessing the application.

> [!IMPORTANT]
> **Enroll a second-factor method on the owner account before changing `mfa.requirement` away from `optional`.** Under any stricter policy, Cockpit rejects sessions that relied on a trusted-device cookie and redirects to the web client to complete a real second step.

Even when `mfa.requirement` is `optional`, once the owner account has a second factor enabled, high-risk actions in Cockpit (and role/override/ownership mutations on the primary instance server) require a second factor completed within the last 5 minutes:

- Creating accounts, changing account status, or modifying server roles and permission overrides
- Enabling or disabling OIDC provider account creation
- Transferring server ownership or deleting servers
- Updating encrypted secrets, desktop client minimum/recommended versions, account security policy, or mail transport settings
- Updating backup schedules or Cloudflare R2 settings, or downloading backup archives

Timing and rate-limit policy settings (`mfa.trusted-device-days`, `mfa.challenge-minutes`, `mfa.email-code-minutes`, `mfa.email-cooldown-seconds`, `mfa.email-attempts`, `mfa.email-sends-per-hour`, and `mfa.cli-recovery-minutes`) can be edited in Cockpit or via `stellarr config set`; see [Operator CLI](cli.md#configuration).

## Account recovery

### User self-recovery

If a user loses access to their authenticator app or passkey, they can select **Use a recovery code** on the second-factor prompt and enter one of their ten single-use recovery codes.

### Operator recovery

If a user has no recovery codes remaining, the instance owner can reset their second-factor enrollment in **Cockpit → Users → (account) → Account security → Recover access**, or via the operator CLI:

```bash
docker compose exec stellarr stellarr security show maja
docker compose exec stellarr stellarr security recover maja --reason "lost phone"
```

Operator recovery performs the following steps atomically:

1. Revokes all MFA methods, trusted devices, active sessions, approved desktop handoffs, and unused recovery codes on the account (while keeping OIDC identity links intact).
2. Issues a **one-time CLI recovery credential** valid for `mfa.cli-recovery-minutes` (`10` minutes by default). Deliver this credential to the user over a verified out-of-band channel.
3. Locks normal session issuance (`SecurityRecoveryRequired`) until the user signs in (with their password or linked OIDC provider), redeems the recovery credential (`POST /api/auth/recovery/redeem`), and enrolls a replacement second factor (which generates ten new recovery codes).

> [!NOTE]
> The **instance owner's** own account can only be recovered via the CLI (`stellarr security recover`), because recovery revokes all active sessions including the current Cockpit session.

## Backups and data-protection keys

TOTP seeds and Cockpit secrets are encrypted using ASP.NET Core data-protection keys stored in `/data/stellarr/keys/`. Always take full backups (via Cockpit or volume archives in [Upgrading](../install/upgrading.md)) so `keys/` remains paired with `db/stellarr.db`. Restoring a database without its matching `keys/` directory renders encrypted TOTP seeds unreadable and requires recovery codes or operator recovery.
