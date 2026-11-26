# Activity notifications

Users can opt into **email** and **Web Push** notifications for **channel mentions** (`@username`) and **direct messages** under **Settings → Notifications**. Preferences are evaluated when a message is posted; opting in later never backfills alerts for older messages.

## Zero-metadata privacy model

External mail relays and browser push services never see conversation metadata or message contents:

- **Opaque payloads:** Both email and Web Push omit sender names, server/channel names, and message text (for standard and E2EE conversations alike), using fixed localized copy (`STELLARR_LANGUAGE`):
  - Mention: *"Someone mentioned you in a channel. Open Stellarr to read it."*
  - Direct message: *"You have a new direct message. Open Stellarr to read it."*
- **Self-contained HTML mail:** Uses Stellarr's *Midnight* template (`api/crates/mail/src/layout.rs`) with inline CID PNG icons—zero external image or font fetches when opened in a mail client.
- **Redacted logs:** Recipient emails, push endpoints, P-256DH/auth keys, and message text are never logged.

## Email notifications

Requires two conditions:

1. **Instance mail transport:** Configured via SMTP (**Cockpit → Settings**) or Resend (**Cockpit → Integrations**), independent of `mail.alerts.enabled`.
2. **Verified user email:** Email notifications enabled in **Settings → Notifications** plus a confirmed email 2FA method on the account's current email address.

---

## Web Push

Web Push wakes `web/public/stellarr-worker.js` to display system notifications even when Stellarr tabs are closed, updates the installed PWA's unread badge (counting unread DMs + unread inbox `@mentions` across accessible channels), and deep-links straight to the target message after checking current server access.

Check VAPID readiness in **Cockpit → Integrations → Web Push**. On iPhone and iPad, install the site via **Share → Add to Home Screen** first, open it from the Home Screen icon, and enroll the device under **Settings → Notifications**.

### 1. Generate a persistent VAPID key pair

Generate **one** P-256 key pair per installation (`npx web-push generate-vapid-keys`, or with OpenSSL below):

```bash
openssl ecparam -name prime256v1 -genkey -noout -out vapid-private.pem
openssl ec -in vapid-private.pem -pubout -outform DER -out vapid-public.der

# Public key (base64url, 65 bytes decoded):
tail -c 65 vapid-public.der | base64 -w0 | tr '+/' '-_' | tr -d '='

# Private key (base64url, 32 bytes decoded):
priv_hex="$(openssl ec -in vapid-private.pem -text -noout 2>/dev/null \
  | awk '/priv:/{flag=1;next}/pub:/{flag=0}flag' | tr -d ' :\n')"
printf '%s' "$priv_hex" | xxd -r -p | base64 -w0 | tr '+/' '-_' | tr -d '='
```

*(On macOS BSD `base64`, omit `-w0`.)*

> [!WARNING]
> Never rotate VAPID keys across restarts unless intentionally invalidating subscriptions—browsers bind push endpoints to the server's public key and stop receiving pushes until users re-subscribe.

### 2. Enable in `.env`

Add the keys to `.env` ([Configuration: Web Push](../install/configuration.md#web-push)) and run `docker compose up -d`:

```dotenv
STELLARR_PUSH_ENABLED=true
STELLARR_PUSH_SUBJECT=mailto:admin@example.com
STELLARR_PUSH_PUBLIC_KEY=YOUR_PUBLIC_KEY
STELLARR_PUSH_PRIVATE_KEY=YOUR_PRIVATE_KEY
```

Missing or mismatched VAPID keys never block container startup; Web Push simply reports as unavailable in **Settings → Notifications**.

### Subscription security and status codes

| Rule / Upstream HTTP status | Behavior |
| --- | --- |
| **Per-account limit & binding** | Max `10` active browser subscriptions per account, bound to both the account and active sign-in session (`409` on cross-account/session conflict). |
| **SSRF protection** | Endpoints must use `https://` on port `443` without credentials/fragments; private/loopback/link-local IPs and HTTP redirects are rejected. |
| **Payload envelope** | RFC 8291 encrypted JSON (`id`, `title`, `body`, `url`, `tag`), `≤ 3072` UTF-8 bytes plaintext, `300 s` TTL. |
| `2xx` | Marked delivered. |
| `404`, `410` | Subscription deleted immediately (`Gone`); not retried. |
| `408`, `425`, `429`, `5xx`, network timeout | Retried with exponential backoff (`2 min` to `1 h`, max `5` attempts). |
| `401`, `403`, other `4xx`, `3xx` | Retried up to `5` attempts without deleting the subscription (`401`/`403` signals a VAPID config mismatch). |

---

## Dispatcher, pre-send checks, and retries

The `notification-delivery` worker (`api/crates/server/src/delivery/dispatcher.rs`) polls `NotificationDeliveries` every `30 s` in batches of `25` (`10 s` push timeout, `15 s` mail timeout, at-least-once delivery with idempotency keys and matching notification `tag` deduplication).

Immediately before dispatching each row, the worker re-evaluates live SQLite state and cancels silently when any condition holds:

- Recipient disabled notifications, muted the channel, already read past the message, or had their account disabled.
- Message was deleted, edited to remove `@username`, or resides in a deleted/inaccessible channel (`viewChannel` lost).
- Either user blocked the other, or the DM conversation no longer admits messages between them.
- **Email only:** Recipient's email changed or email 2FA method was removed (missing mail transport defers up to `1 h` without burning retry attempts).
- **Web Push only:** Bound sign-in session expired, was revoked, or changed `SecurityVersion`.

Undelivered rows expire after `7` days; completed and cancelled rows are pruned after `30` days.
