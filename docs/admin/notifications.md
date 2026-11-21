# Activity notifications

Stellarr can notify users about **channel mentions** (`@username`) and **direct messages** over **email** and **Web Push** when they opt in under **Settings → Notifications**.

Notification preferences are evaluated at the moment a message is posted. Enabling notifications later does not queue alerts for historical messages.

## Privacy design

Notifications are designed so external email relays and browser push providers never see private conversation metadata or message content:

- **Zero conversation content**: Emails and Web Push payloads omit sender names, server names, channel names, and message text (including for both unencrypted and end-to-end encrypted direct messages).
- **Generic localized copy**: Notification text uses the instance language (`STELLARR_LANGUAGE`, `en` or `pl`) and links only to the instance root (`STELLARR_PUBLIC_URL` or `STELLARR_DOMAIN`):
  - Mention: *"Someone mentioned you in a channel. Open Stellarr to read it."*
  - Direct message: *"You have a new direct message. Open Stellarr to read it."*
- **Self-contained email rendering**: Outgoing HTML emails use Stellarr's dark *Midnight* template (`MailLayout.cs`) with inline CID PNG attachments (`lockup.png`, `mention.png`, `direct-message.png`), making zero external image or font requests when opened in a mail client.
- **No PII in logs**: Push endpoints, P-256DH/auth keys, recipient email addresses, and message contents are never written to application logs.

## Email notifications

Email notifications require:

- **Instance setup**: Working mail transport (SMTP configured in **Cockpit → Settings**, or Resend enabled under **Cockpit → Integrations**).
- **User setup**: Email notifications toggled on in **Settings → Notifications**, plus a confirmed email second-factor method on the user's current email address.

> [!NOTE]
> Activity notification emails use the instance's mail transport whenever users opt in, independently of the security-alert email switch (`mail.alerts.enabled`). Server invitations continue using in-app notifications.

## Web Push

Web Push delivers notifications through the browser's push service (`web/public/stellarr-worker.js`) even when all Stellarr tabs are closed. When a Stellarr window is currently visible and focused, the Service Worker suppresses the duplicate OS notification because the live app already displays the in-app alert.

Web Push remains disabled until you configure a persistent VAPID key pair in `.env` and users opt in under **Settings → Notifications**.

### 1. Generate a persistent VAPID key pair

Generate **one** P-256 VAPID key pair per installation and store it with your instance secrets.

> [!WARNING]
> Never regenerate VAPID keys across container restarts. Browsers bind push subscriptions to the application server's public key; changing the key invalidates all existing browser subscriptions until users re-subscribe.

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

*(On macOS BSD `base64`, omit `-w0`, or generate a key pair with `npx web-push generate-vapid-keys`.)*

### 2. Configure environment variables

Add the VAPID configuration to `.env` (see [Configuration — Web Push](../install/configuration.md#web-push)) and recreate the container with `docker compose up -d`:

```dotenv
STELLARR_PUSH_ENABLED=true
STELLARR_PUSH_SUBJECT=mailto:admin@example.com
STELLARR_PUSH_PUBLIC_KEY=YOUR_PUBLIC_KEY
STELLARR_PUSH_PRIVATE_KEY=YOUR_PRIVATE_KEY
```

If keys are missing, malformed, or mismatched, the container boots normally and reports Web Push as unavailable in **Settings → Notifications**.

> [!NOTE]
> Browsers require a secure context (`https://` or `http://localhost`) for Service Workers and Web Push. Configure `STELLARR_DOMAIN` or `STELLARR_PUBLIC_URL` so notification links target your public instance origin.

### Subscription limits and security

| Property | Limit / Rule |
| --- | --- |
| Subscriptions per account | Maximum 10 active browser subscriptions (`PUT`/`DELETE /api/me/push-subscriptions`; status via `GET /api/me/notification-delivery`). |
| Ownership and session binding | Bound to both the user account and the active sign-in session. Endpoint conflicts across accounts or active sessions return `409`; once an older session ends, the same account can re-register that endpoint from a new session. |
| Endpoint validation | Must be an absolute `https://` URL on port `443` with no credentials or fragments. Loopback, link-local, and private IP ranges are blocked before connecting, HTTP redirects are rejected, and outbound proxies/cookies are disabled. |
| Payload format | RFC 8291 encrypted camelCase JSON (`id`, `title`, `body`, `url`, `tag`), capped at `3072` UTF-8 bytes before encryption, `300 s` TTL. |

### Push service status handling

| Response | Action |
| --- | --- |
| `2xx` | Marked delivered. |
| `404`, `410` | Subscription deleted (`Gone`); delivery not retried. |
| `408`, `425`, `429`, `5xx`, timeout, network error | Retried with exponential backoff (up to 5 attempts). |
| `401`, `403` | Retried up to the attempt cap; indicates a VAPID key/subject mismatch rather than an expired browser subscription. |
| Other `4xx` (e.g. `400`, `413`), `3xx` redirect | Retried up to the attempt cap without deleting the subscription. |

## Delivery, cancellation, and retries

Pending deliveries are persisted in SQLite (`NotificationDeliveries`) and processed asynchronously by `NotificationDeliveryWorker` with **at-least-once** semantics: one row for the user's verified email address and one row per active Web Push subscription.

### Pre-send checks

Immediately before calling an email or push provider, `NotificationDeliveryDispatcher` re-checks live database state and cancels the delivery silently if:

- The user disabled notifications or the account was disabled
- The triggering message was deleted, or an edit removed the `@mention`
- Either user blocked the other, the channel was deleted, or the recipient lost channel view permissions
- The recipient muted the channel or already read past the message
- For direct messages: the recipient is no longer a participant or no longer accepts DMs from the sender
- For email: the recipient's email address changed or their email second factor was revoked
- For Web Push: the subscription's sign-in session expired, was revoked, or its `SecurityVersion` changed

### Retry limits

| Rule | Value |
| --- | --- |
| Retry attempts | Up to 5 attempts with exponential backoff from 2 minutes to 1 hour |
| Missing mail transport | Email deliveries defer for up to 1 hour without consuming provider attempts |
| Expiration and pruning | Undelivered items expire after 7 days; completed/cancelled rows are pruned after 30 days |
| Deduplication | Emails pass a stable idempotency key (`activity-notification/<id>`). Web Push payloads carry a matching `tag` so the browser replaces any existing notification for the same event |
