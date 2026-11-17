# Activity notifications

Stellarr can notify users about **channel mentions** and **direct messages** over **email** and **Web Push** when they opt in under **Settings → Notifications**.

Preferences apply at the moment a message is posted. Turning notifications on later does not send alerts for older messages.

## Privacy design

Notifications avoid leaking private conversation data to external email relays or push providers:

- **No conversation details**: Emails and push payloads exclude sender names, server names, channel names, and message text.
- **Generic localized text**: Copy uses the instance language (`STELLARR_LANGUAGE`, `en` or `pl`) and links only to the instance root (`STELLARR_PUBLIC_URL` or `STELLARR_DOMAIN`), since channels and DMs have no public permalinks:
  - Mention: *"Someone mentioned you in a channel. Open Stellarr to read it."*
  - Direct message: *"You have a new direct message. Open Stellarr to read it."*
- **No PII in logs**: Push endpoints, encryption keys, recipients, and message content are never logged.

## Email notifications

Email notifications require:

- **Instance setup**: Working mail transport (SMTP in **Cockpit → Settings**, or Resend under **Integrations**).
- **User setup**: Email notifications enabled in **Settings → Notifications**, plus a confirmed email second factor on their current address.

> [!NOTE]
> Activity notification emails use the instance's regular mail configuration whenever users enable them. They are independent of the security-alert email switch (`mail.alerts.enabled`). Server invitations continue using in-app notifications only.

## Web Push

Web Push lets Stellarr wake a user's browser even when the Stellarr tab is closed. It stays disabled until you supply a persistent VAPID key pair and users enable Push notifications in **Settings → Notifications**.

### 1. Generate a persistent VAPID key pair

Generate **one** P-256 VAPID key pair per installation and store it with your instance secrets.

> [!WARNING]
> Never regenerate VAPID keys on container restarts. Browsers bind subscriptions to the application server public key, so changing or losing the key breaks all existing subscriptions until users re-subscribe.

Using OpenSSL:

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

On macOS BSD `base64`, omit `-w0`, or generate keys with `npx web-push generate-vapid-keys`.

### 2. Configure environment variables

Add the key pair to `.env` and apply with `docker compose up -d`:

```dotenv
STELLARR_PUSH_ENABLED=true
STELLARR_PUSH_SUBJECT=mailto:admin@example.com
STELLARR_PUSH_PUBLIC_KEY=YOUR_PUBLIC_KEY
STELLARR_PUSH_PRIVATE_KEY=YOUR_PRIVATE_KEY
```

| Compose `.env` | ASP.NET config | Description |
| --- | --- | --- |
| `STELLARR_PUSH_ENABLED` | `Push:Enabled` | Turns Web Push on when valid keys and subject are present. |
| `STELLARR_PUSH_SUBJECT` | `Push:Subject` | `mailto:` address or `https://` URL identifying the operator. |
| `STELLARR_PUSH_PUBLIC_KEY` | `Push:PublicKey` | Base64url uncompressed P-256 public key (65 bytes decoded). |
| `STELLARR_PUSH_PRIVATE_KEY` | `Push:PrivateKey` | Base64url P-256 private key (32 bytes decoded). Keep secret. |

If keys are missing, malformed, or mismatched, the container still boots normally and reports Web Push as unavailable in **Settings → Notifications**.

> [!NOTE]
> Browsers require HTTPS (or `localhost` during local development) for Service Workers and Web Push. Set `STELLARR_DOMAIN` or `STELLARR_PUBLIC_URL` so notification links target your public instance URL.

### Subscription limits and security

| Property | Limit / Rule |
| --- | --- |
| Subscriptions per account | Maximum 10 active browser subscriptions (`PUT`/`DELETE /api/me/push-subscriptions`, status via `GET /api/me/notification-delivery`). |
| Ownership and session binding | Bound to both account and active sign-in session. Conflicts return `409`; once an older session ends, the same account can re-register that endpoint from a new session. |
| Endpoint validation | Must be an absolute `https://` URL on port `443` with no credentials or fragments. Private and reserved IP ranges are blocked before connecting, redirects are rejected, and proxies/cookies are not used. |
| Payload and privacy | Encrypted camelCase JSON (`id`, `title`, `body`, `url`, `tag`), capped at `3072` UTF-8 bytes before encryption, `300 s` TTL. Endpoints, keys, and `Authorization` headers are never logged. |

### Push service status handling

| Response | Action |
| --- | --- |
| `2xx` | Delivered. |
| `404`, `410` | Subscription removed (`Gone`); not retried. |
| `408`, `425`, `429`, `5xx`, timeout, network error | Retried with backoff (up to 5 attempts). |
| `401`, `403` | Retried; indicates a VAPID configuration problem rather than an expired browser subscription. |
| Other `4xx` (e.g. `400`, `413`), `3xx` redirect | Retried up to the attempt cap without deleting the subscription. |

## Delivery, cancellation, and retries

Pending deliveries are stored in SQLite (`NotificationDeliveries`) and sent with **at-least-once** delivery: one item for the user's verified email address and one per active Web Push subscription.

### Pre-send checks

Before calling an email or push provider, the dispatcher re-validates that the alert is still relevant and cancels it silently if:

- The user disabled notifications or the account was disabled
- The message was deleted, or an edit removed the mention
- Either user blocked the other, the channel was deleted, or the user lost channel access
- The user muted the channel or already read past the message
- For direct messages: the user is no longer a participant or no longer allows DMs from that sender
- For email: the verified email address changed or its email second factor was revoked
- For Web Push: the subscription's session expired, was revoked, or its security version changed

### Retry limits

| Rule | Value |
| --- | --- |
| Retry attempts | Up to 5 attempts with exponential backoff from 2 minutes to 1 hour |
| Missing mail transport | Email rows defer up to 1 hour without calling providers |
| Expiration and pruning | Undelivered items expire after 7 days; completed rows prune after 30 days |
| Expired push endpoints | `404` or `410` deletes the push subscription and marks delivery complete |
| Deduplication | Emails send a stable idempotency key (`activity-notification/<id>`). Web Push lacks a provider-side key, so rare crash windows can repeat a push |

## Upgrades

The `PushSubscriptions` and `NotificationDeliveries` tables and indexes are created automatically on startup. Because push subscriptions bind to the current sign-in session, users reopen **Settings → Notifications** once after upgrading to re-save their browser subscription (browser keys stay valid while `STELLARR_PUSH_PUBLIC_KEY` stays unchanged).
