# Activity notifications (e-mail and Web Push)

Mentions and direct messages can leave the instance as an e-mail or a browser
push notification, but only for a person who has turned that channel on in
their notification preferences. The preference is read when the message is
created: turning it on later does not send notifications for messages that
were already posted.

## What is sent

An e-mail or push notification names no sender, server, channel, conversation
or message text. It is a fixed sentence in the instance's language (`Stellarr`
in `Stellarr:Language`, English or Polish) with a link to the public root of
the instance:

- "Someone mentioned you in a channel. Open Stellarr to read it."
- "You have a new direct message. Open Stellarr to read it."

The app has no stable channel or direct-message permalink route, so the link
is the public entry point (`Operations:PublicUrl`) rather than a guessed deep
link. The mail HTML escapes the fixed text and the URL; the plain-text part
keeps the URL readable.

An e-mail is addressed only to the account's current verified address: the
address must have a confirmed, non-revoked e-mail second factor, and it must
still match at delivery time. Changing the address, revoking the factor or
turning the preference off before a pending delivery is sent cancels that
delivery instead of sending it to the old destination.

## What is not sent

- Server invitations keep their existing in-app behaviour; they are not part of
  this outbox.
- Activity mail is not gated by the security-alert mail switch
  (`AlertMailEnabled`). It uses the instance's normal mail setup
  (`AccountMail.FindAsync`, SMTP or Resend) whenever activity notifications are
  enabled by the recipients themselves.
- No push endpoint, key, secret, message body or destination is written to a
  log or stored in an error column.

## Delivery guarantees

The outbox provides **at least once** delivery. Each of these is a separate
row:

- one e-mail row for the verified address;
- one push row for every live subscription of the recipient.

A retry for one row never resends a row that already succeeded. A dispatcher
claim uses a short lease, so two dispatcher instances cannot work on the same
row at once, and a row whose lease expired after a crash is retried.

E-mail rows carry a stable `activity-notification/{row-id}` idempotency key.
SMTP itself has no exactly-once promise, so the key lets the mail provider
recognise a repeat after a crash between the provider accepting a message and
the row being marked sent. Web Push has no equivalent provider-side key; a
repeat is possible in the same narrow window.

Retries are bounded to five attempts, with exponential backoff from two
minutes up to one hour. A row expires after seven days; an expired row is
completed as cancelled rather than delivered late. Completed rows older than
30 days are pruned. If the instance has no usable mail setup, e-mail rows wait
for up to an hour and no provider is called.

Before an external delivery, the dispatcher checks again that:

- the preference is still on and the recipient account is enabled;
- the source message (or direct message) still exists;
- neither person blocks the other;
- the recipient still belongs to the server and the channel still exists
  (mentions), using the normal `ServerAccess` authorization semantics;
- the mentioned name is still present after any edit;
- the recipient has not muted that channel;
- the recipient has not already read past the message;
- for a direct message, the recipient is still a participant and still accepts
  messages from that peer under `AllowDirectMessages`;
- for e-mail, the verified current address is the same one the row was created
  for;
- for push, the subscription still belongs to the recipient and its sign-in is
  live: not revoked, not expired, and carrying the account's current session
  version.

A subscription the push service reports as gone is deleted, and its delivery is
completed without further retries.

## Storage and upgrades

The `NotificationDeliveries` table and its indexes are created by an
idempotent schema step on every start. `EnsureCreated` alone does not add a
table to an existing installation, so an upgraded database gets the table from
`NotificationDeliverySchema`; repeated preparation is safe. The push
subscription table is created first, because the outbox refers to its rows by
id.
