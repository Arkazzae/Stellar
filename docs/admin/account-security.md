# Account security

Stellarr supports second-factor authentication (TOTP apps, email codes, and passkeys or hardware security keys), OIDC identity linking, trusted devices, and operator-assisted recovery.

## Second-factor methods

Users manage their own sign-in methods under **Settings → Login and security**:

| Method | How it works | Notes |
| --- | --- | --- |
| **Authenticator app (TOTP)** | 6-digit, 30-second codes via QR code or text key (secret shown only during setup) | Multiple named authenticators per account |
| **Email code** | 6-digit one-time code sent to the account's email address | Requires [Mail configuration](../install/configuration.md#mail); subject to a 5-request / 15-minute send limit |
| **Passkey or security key** | Device passkey (biometrics/PIN) or USB/NFC hardware key | Passkeys also allow passwordless sign-in (see [Passkeys](#passkeys)) |
| **Recovery codes** | Ten single-use backup codes generated when the first method is added | Shown once; regenerating invalidates previous codes |

The same settings pane shows **trusted devices**, **active sessions** (allowing users to revoke individual sessions or sign out everywhere else), and the account's **security history**. Revoking a session immediately closes its live SignalR/voice connections and returns that browser to the sign-in screen.

### Proof for sensitive account changes

Adding or removing methods, regenerating recovery codes, linking or unlinking an OIDC provider, and changing passwords require fresh proof: either a second factor completed within the **last 5 minutes**, or the current password if the account has no second factor yet. Security-change endpoints allow up to 30 requests per IP every 15 minutes.

## Passkeys

Passkeys require an HTTPS origin with a real hostname: `STELLARR_PUBLIC_URL`, or `STELLARR_DOMAIN` set to a domain name rather than an IP address (except `http://localhost` during local development). Without a hostname origin, the web client hides passkey setup.

> [!WARNING]
> Passkeys are bound to the hostname they were created on. **Changing `STELLARR_DOMAIN` or `STELLARR_PUBLIC_URL` invalidates all existing passkeys**; users must sign in with their password and another method (or a recovery code) and register a new passkey.

- **Passwordless sign-in**: A discoverable passkey enables **Continue with a passkey** on the sign-in screen. Because it verifies both user presence and user verification, it satisfies both password and second-factor requirements (including Cockpit access).
- **Second-factor-only keys**: Non-discoverable security keys work as a second factor after password sign-in, not standalone.

## Linking an identity provider

When OIDC is enabled (see [Single sign-on](../install/configuration.md#single-sign-on-oidc)), users can link external accounts under **Settings → Login and security → Sign-in providers**:

- Linking and unlinking require fresh security proof.
- Links are bound per `(issuer, subject)`. One provider identity cannot be linked to two Stellarr accounts, and an account cannot link two identities from the same provider.
- By default, OIDC sign-in still requires Stellarr's own second factor unless `TrustProviderMfa` is enabled for that provider.
- Operators can inspect links with `stellarr security show` and remove them via Cockpit or `stellarr security unlink`. Operator MFA recovery preserves OIDC links.

## Changing an account's email address

Because the email address receives email 2FA codes, security alerts, and password-reset links, changing it under **Settings → Login and security** requires the current password plus a fresh second factor (if enabled), as well as configured mail delivery:

1. Stellarr emails a confirmation link (valid for 60 minutes) to the **new** address.
2. The address updates only after the confirmation link is opened and submitted.
3. The **old** address receives a security alert, and pending codes or reset links sent to the old address stop working.

## Trusted devices

During second-factor sign-in, users can check **Don't ask for a code on this device for N days** (controlled by `mfa.trusted-device-days`, `14` by default).

- A trusted device skips the second-factor prompt during normal sign-in only.
- It **never** satisfies the fresh second-factor check required for Cockpit or sensitive account changes.

## Session lifetimes

- **Default (`Keep me signed in` checked)**: Sessions stay valid for 30 days.
- **Unchecked**: Cookie expires when the browser closes, with a hard 12-hour server-side limit (even if the browser restores tabs/cookies).
- **Desktop handoff**: Sessions issued through desktop browser sign-in are always persistent.

Changing second-factor methods revokes other active sessions. Signing out, remote session revocation, and operator recovery terminate active WebSocket/voice connections immediately.

## Instance MFA policy (operators)

The instance owner configures who must use a second factor in **Cockpit → Settings → Account security** or via the [operator CLI](cli.md):

```bash
docker compose exec stellarr stellarr config set mfa.requirement everyone --yes
```

| Value | Effect |
| --- | --- |
| `optional` | Default. Users decide for themselves; accounts with a configured method are prompted on sign-in. |
| `admins` | Required for the instance owner and any role holding **Manage server**. |
| `everyone` | Required for every account on the instance. |
| `cockpit` | Normal client sign-in follows each account's setup; accessing Cockpit requires a second step. |

With `admins` or `everyone`, users without a second-factor method are guided through enrolling one on their next sign-in before accessing the app.

> [!IMPORTANT]
> **Set up a second-factor method on the owner account before changing `mfa.requirement` away from `optional`.** With any stricter policy, Cockpit rejects sessions that only used a trusted device and redirects you to confirm a real second step in the web client.

Even when `mfa.requirement` is `optional`, once the owner account has a second factor enabled, high-risk actions in Cockpit (and equivalent role/ownership changes on the instance's primary server) require a second factor completed within the last 5 minutes:

- Creating accounts, modifying account status, or changing server roles and permissions
- Transferring or deleting servers
- Editing secrets, desktop releases, account security settings, or mail transport settings
- Downloading backups

Timing and rate-limit policy keys (such as `mfa.trusted-device-days`, `mfa.challenge-minutes`, and `mfa.cli-recovery-minutes`) can be adjusted in Cockpit or listed via `stellarr config list`; see [CLI Configuration](cli.md#configuration).

## Account recovery

### User self-recovery

If a user loses their authenticator device or passkey, they can click **Use a recovery code** on the second-factor prompt and enter one of their ten single-use codes.

### Operator recovery

If a user has no recovery codes left, an administrator can reset their second-factor enrollment either in **Cockpit → Users → (account) → Account security → Recover access**, or via the CLI:

```bash
docker compose exec stellarr stellarr security show maja
docker compose exec stellarr stellarr security recover maja --reason "lost phone"
```

What operator recovery does:

1. Revokes all MFA methods, trusted devices, active sessions, approved desktop handoffs, and unused recovery codes on the account (while keeping OIDC links intact).
2. Prints a **one-time CLI recovery credential** valid for `mfa.cli-recovery-minutes` (`10` minutes by default). Deliver it to the user over a trusted channel.
3. Locks normal session issuance until the user signs in with their password, redeems the one-time credential, and enrolls a replacement second factor (which issues fresh recovery codes).

> [!NOTE]
> The **instance owner's** own account can only be recovered via the CLI (`stellarr security recover`), because recovery revokes all active sessions including the current Cockpit session.

## Backups and encryption keys

TOTP secrets are stored encrypted with the instance's data-protection keys (`<data-root>/keys/`). Always use full backups (via Cockpit or volume archives in [Upgrading](../install/upgrading.md)) so `keys/` stays paired with the database. Restoring a database without its matching `keys/` renders existing TOTP authenticators unusable and requires recovery codes or operator recovery.
