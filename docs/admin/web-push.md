# Web Push

Web Push lets the instance wake a browser whose Stellarr tab is closed, for a
mention or a direct message the person opted into. It is optional and disabled
until the operator supplies a persistent VAPID key pair. E-mail uses its own
configuration and is reported separately by the account settings.

The server never sends a live push during development, tests or a backup
restore. A delivery happens only when the durable activity outbox has a due row
for an account with a live session and a saved browser subscription.

## Configuration

Set these in `.env` for the production Compose stack (or as environment
variables for a direct .NET deployment):

```dotenv
STELLARR_PUSH_ENABLED=true
STELLARR_PUSH_SUBJECT=mailto:admin@example.com
STELLARR_PUSH_PUBLIC_KEY=YOUR_PUBLIC_KEY
STELLARR_PUSH_PRIVATE_KEY=YOUR_PRIVATE_KEY
```

| Compose / `.env` | ASP.NET configuration | Meaning |
| --- | --- | --- |
| `STELLARR_PUSH_ENABLED` | `Push:Enabled` | Turns Web Push on. It is also off when the key pair is missing or invalid. |
| `STELLARR_PUSH_SUBJECT` | `Push:Subject` | `mailto:` address or `https://` URL identifying the operator to push services. |
| `STELLARR_PUSH_PUBLIC_KEY` | `Push:PublicKey` | Base64url uncompressed P-256 public key, 65 bytes decoded. |
| `STELLARR_PUSH_PRIVATE_KEY` | `Push:PrivateKey` | Base64url P-256 private key, 32 bytes decoded. Treat it as a secret. |

Without `STELLARR_DOMAIN` or `STELLARR_PUBLIC_URL`, the instance has no public
address and activity e-mail stays off. Push can still be configured, but
clients should be given a public origin before relying on its links.

A missing, malformed or mismatched pair makes `pushAvailable` false and
`applicationServerKey` null. It does not stop the API from starting, and the
private key is never returned to a client or written to a log.

## Generate and keep the keys

Generate one pair per installation and keep it with the other instance
secrets. Do not regenerate it on every start: browsers bind their subscription
to the application server key, and a different key makes every existing
subscription unusable.

With OpenSSL:

```sh
openssl ecparam -name prime256v1 -genkey -noout -out vapid-private.pem
openssl ec -in vapid-private.pem -pubout -outform DER -out vapid-public.der

# Raw public key: the last 65 bytes of SubjectPublicKeyInfo.
tail -c 65 vapid-public.der | base64 -w0 | tr '+/' '-_' | tr -d '='

# Raw private scalar from the text form (hex), then base64url.
priv_hex="$(openssl ec -in vapid-private.pem -text -noout 2>/dev/null \
  | awk '/priv:/{flag=1;next}/pub:/{flag=0}flag' | tr -d ' :\n')"
printf '%s' "$priv_hex" | xxd -r -p | base64 -w0 | tr '+/' '-_' | tr -d '='
```

Alternatively, Node's `web-push` package can print a pair with
`npx web-push generate-vapid-keys`; the values are the same. Store the private
PEM or the encoded private key in the deployment's secret store, not in the
repository. Losing the private key means clients must resubscribe; rotating it
has the same effect.

## What the API exposes

All three routes require an authenticated account session and deliberately do
not depend on the selected server, because a mention or direct message belongs
to the person:

- `GET /api/me/notification-delivery` returns
  `{ "emailAvailable": bool, "emailVerified": bool, "pushAvailable": bool, "applicationServerKey": string | null }`.
  `emailAvailable` means the instance has a usable mail transport.
  `emailVerified` means this account has a confirmed e-mail second step at its
  current address.
- `PUT /api/me/push-subscriptions` takes
  `{ "endpoint": "...", "keys": { "p256dh": "...", "auth": "..." } }` and
  returns `{ "id": "..." }`. Repeating it for the same account, session and
  endpoint is idempotent and returns the same id.
- `DELETE /api/me/push-subscriptions` takes `{ "endpoint": "..." }` and returns
  `204`. It removes only a subscription bound to the current account and the
  current sign-in. Another device's or account's endpoint is left untouched.

An endpoint is unique across the instance. An attempt to bind an endpoint that
already belongs to a different account is refused with `409`; it is never
reassigned. A different, still-active sign-in of the same account is refused
the same way. Once that older sign-in has ended, the same account may bind the
endpoint again from a new sign-in, so a browser does not lose push after a
server-side session revocation. An account may keep at most ten subscriptions.

The endpoint must be an absolute `https` URL on port 443, with no user
information, fragment or local/reserved address. Subscription keys are bounded
and checked before storage: `p256dh` must decode to an uncompressed P-256 point
and `auth` to exactly 16 bytes, both in base64url. Private and reserved
addresses are also refused when the socket is opened, after DNS resolution, so
a public name cannot be rebound to an internal address between registration and
delivery. Deliveries follow no redirects, send no cookies and use no proxy.

## Delivery and retries

The durable activity outbox owns retries. One send attempt maps the push
service's answer as follows:

| Result | Meaning |
| --- | --- |
| `2xx` | `Delivered`; the provider accepted the notification. |
| `404`, `410` | `Gone`; the browser subscription no longer exists and the row is removed. |
| `408`, `425`, `429`, `5xx`, timeout or network failure | `Retry`; the outbox applies its own backoff and attempt cap. |
| `401`, `403` | `Retry`; this is an operator VAPID configuration problem, not a dead subscription. |
| Other `4xx` (for example `400`, `413`) | `Gone`; this payload or subscription cannot succeed by repeating it. |
| Redirect (`3xx`) | `Gone`; the transport never follows it. |

The JSON body is camelCase (`id`, `title`, `body`, `url`, `tag`), encrypted by
the maintained `WebPush` library. It is capped at 3072 UTF-8 bytes before
encryption and sent with a 300-second TTL. Invalid subscription keys are
retired rather than retried forever. Endpoint URLs, subscription keys, the
VAPID private key and the signed `Authorization` header are never logged.

## Upgrade and client notes

Existing installations get the `PushSubscriptions` table through the normal
startup schema step; no separate migration command is needed. After an upgrade,
each browser must save its subscription once more, because a subscription is
bound to both the browser and the current sign-in. The browser's local
subscription remains valid while the same VAPID public key is kept.
