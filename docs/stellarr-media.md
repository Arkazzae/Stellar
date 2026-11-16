# Stellarr Media

Stellarr Media (`stellarr-media`, in `media/`) is Stellarr's Rust Selective Forwarding Unit (SFU). Every voice channel and direct call goes through it: each participant opens a single `RTCPeerConnection` to the media server, which forwards microphone, camera, and screen-share RTP packets to everyone else in the room without transcoding or mixing. Browsers never send media directly to one another.

This page covers:

1. **Hub leg**: SignalR methods and events between browser clients and `AppHub`.
2. **Control API**: Loopback HTTP protocol between `AppHub` and `stellarr-media`.
3. **Media plane**: ICE-lite transport, SDP negotiation, RTP forwarding, and resource limits.

See [Voice and RTC protocol](./rtc-protocol.md) for bootstrap configuration, `VoiceSignal` state messages, `VoiceState`, and audio/video profiles.

## Overview

| Aspect | Behavior |
| --- | --- |
| WebRTC engine | `str0m` (sans-IO ICE, DTLS, SRTP, RTP/RTCP) |
| Transport | ICE-lite on a single UDP socket (`0.0.0.0:49000`) |
| Forwarded media | Microphone audio, camera video, screen video, and screen audio |
| Media processing | Packet-level RTP forwarding; no decoding, mixing, or transcoding |
| Authorization | `AppHub` authorizes seats; `stellarr-media` only accepts commands on its loopback control API |
| Signaling path | Browser ↔ `AppHub` (SignalR) ↔ `stellarr-media` (`127.0.0.1:5200`) |
| Connections per client | Single `RTCPeerConnection` (`bundlePolicy: "max-bundle"`) |
| Offer direction | Browser offers published tracks (`sendonly`); `stellarr-media` offers forwarded tracks (`sendonly` to the receiver) |
| Glare resolution | Browser is the impolite peer; `stellarr-media` is the polite peer |
| Track attribution | Server-issued directory mapping SDP `mid` → `{ userId, kind, streamId, heldBack }` |
| Downlink adaptation | Per receiver: its downlink is estimated from its own transport-wide feedback; every sound it asks for is sent, its pictures in its order of importance while they fit, and the rest held back for it alone |
| Direct calls | Routed through the media server just like server voice channels |

## Topology

```mermaid
flowchart LR
    BrowserA["Participant A (Browser)"]
    BrowserB["Participant B (Browser)"]
    Turn["coturn (:3478 / 49160-49200/udp)"]
    Hub["AppHub SignalR (/hubs/app)"]
    Media["stellarr-media (:49000/udp)"]

    BrowserA <-->|"OpenMediaSession / SendMediaSignal"| Hub
    BrowserB <-->|"OpenMediaSession / SendMediaSignal"| Hub
    Hub <-->|"HTTP Control API (127.0.0.1:5200)"| Media

    BrowserA <-->|"Direct SRTP (:49000/udp)"| Media
    BrowserB <-->|"TURN relay (when direct UDP is blocked)"| Turn
    Turn <-->|"Local UDP"| Media
```

Because all media goes through the container, any browser outside the Docker host needs the media server's public candidate: set `PUBLIC_IP` and forward UDP `STELLARR_MEDIA_PORT` (default `49000`) to the host on the same external port. Without `PUBLIC_IP`, the media server advertises only the container's internal address, which off-host browsers can reach only through TURN (`coturn`). See [Requirements](install/requirements.md#behind-nat).

`GET /api/bootstrap` returns:

```json
{
  "protocolVersion": 1,
  "mediaServer": "udp://chat.example.com:49000"
}
```

- `mediaServer` is `udp://{Rtc:PublicHost}:{Rtc:MediaPort}` (falling back to `localhost` when `Rtc:PublicHost` is unset). It is used for diagnostics; actual connection addresses come from SDP ICE candidates.

## Hub leg

`AppHub` exposes two SignalR methods and two events for the media server:

| Type | Name | Description |
| --- | --- | --- |
| Method | `OpenMediaSession` | Allocate a media session for the caller's active voice seat |
| Method | `SendMediaSignal` | Send an SDP offer/answer, ICE candidate, or receive subscriptions to `stellarr-media` |
| Event | `MediaSignal` | Deliver an SDP offer/answer or track directory from `stellarr-media` |
| Event | `MediaSessionClosed` | Notify the client that the media session was closed by the media server |

### `OpenMediaSession(channelId) → { sessionId }`

- Requires the caller to hold an active seat in `channelId` (via `JoinVoice`).
- If the connection already has a media session, the previous session is detached and closed asynchronously in the background. Because `stellarr-media` enforces at most one session per `userId`, out-of-order close requests cannot affect the new session.
- Returns an opaque `sessionId` that must accompany all subsequent `SendMediaSignal` calls. Clients ignore any `MediaSignal` or `MediaSessionClosed` event carrying a different `sessionId` or `channelId`.
- Error responses:
  - `HubException("Invalid channelId.")` when `channelId` is not a valid UUID.
  - `HubException("Not seated in this voice room.")` when the caller has no seat in `channelId` (including if the seat ended while the session was being created).
  - `HubException("Media server is unavailable.")` when `stellarr-media` is unreachable, times out, or is undergoing a startup/event-loss reset.

Clients call `OpenMediaSession` after every `JoinVoice` (initial join, moderator move, hub reconnect, or session recovery) and create a fresh `RTCPeerConnection` for each session.

### `MediaSignal` envelope

```json
{
  "protocolVersion": 1,
  "channelId": "uuid",
  "sessionId": "opaque",
  "kind": "offer | answer | candidate | subscriptions | tracks",
  "payload": "JSON string"
}
```

| Kind | Direction | Payload schema |
| --- | --- | --- |
| `offer` | Browser → Media | `{ "type": "offer", "sdp": "…" }` |
| `answer` | Media → Browser | `{ "type": "answer", "sdp": "…" }` |
| `offer` | Media → Browser | `{ "type": "offer", "sdp": "…", "negotiationId": 7, "tracks": [Track] }` |
| `answer` | Browser → Media | `{ "type": "answer", "sdp": "…", "negotiationId": 7 }` |
| `candidate` | Browser → Media | `RTCIceCandidateInit` object, or `null` (end-of-candidates) |
| `subscriptions` | Browser → Media | `{ "mids": ["receiving-mid", "…"] }`, every receiving section to forward |
| `tracks` | Media → Browser | `{ "tracks": [Track] }` |

Each `Track` entry in the directory has the shape:

```json
{ "mid": "fAk", "userId": "uuid", "kind": "audio | video", "streamId": "…", "heldBack": false }
```

`heldBack` is `true` for a picture this receiver asks for that its downlink has no room for now: nothing of it is sent until it is `false` again (see [Downlink adaptation](#downlink-adaptation)). It is always present, and `false` for every sound and for every section the receiver does not ask for. A change of it sends a new `tracks` directory.

- **Validation**: `AppHub` verifies `protocolVersion == 1`, `kind` (`offer`, `answer`, `candidate`, or `subscriptions` from browser), seat ownership, `sessionId` ownership, payload UTF-8 byte length ≤ 128 KiB, and payload schema. Invalid signals throw `HubException("Invalid media signal.")` (or `HubException("Invalid ChannelId.")`).
- **Control API forwarding**:
  - `409 stale-negotiation` on a stale answer returns normally (`stellarr-media` discards stale answers by design).
  - `422` or `413` throws `HubException("Invalid media signal.")`.
  - `404 unknown-session` returns normally and triggers `MediaSessionClosed` with `reason: "media-restarted"`.
  - Other failures throw `HubException("Media server is unavailable.")`.
- **Server-to-browser events**: `MediaSignal` carries only `offer`, `answer`, and `tracks`. `stellarr-media` includes all ICE host candidates directly in its SDP and never trickles candidates.
- Every `tracks` list contains the complete track directory for the session, never a partial diff.

### Receive subscriptions

`subscriptions` replaces the complete list of the receiving session's SDP mids that `stellarr-media` forwards. A new session is forwarded nothing until it asks, and an empty `mids` array stops everything. The order of the list is the receiver's order of importance, most important first: when its downlink cannot carry every picture it asks for, the order decides which are sent (see [Downlink adaptation](#downlink-adaptation)). The same mids in another order are a new list. A section left out stays negotiated and named in the directory but carries no packets, and so does every section negotiated after the list was sent until a later list names it. A mid the session does not have, or does not have yet, is kept and harmless: it never reaches another session or room, and a list may name a section before the answer that negotiates it.

- A list holds at most 4,096 unique mids, each 1–16 printable ASCII characters without spaces. An invalid list is rejected whole and the previous one stays in force.
- Subscriptions change forwarding only: the directory, the transceivers and the SDP mids stay as they are, and a new list never renegotiates SDP.
- A section that is asked for again continues its RTP sequence numbers where it stopped, so the receiver never asks for what was not meant for it, while losses during forwarding stay repairable. Timestamps keep the publisher's clock. Video that resumes asks the publisher for a keyframe through the rate-limited PLI gate, and a receiver's keyframe requests for a section it does not ask for are ignored.
- The browser asks for every microphone first, then for the video and then the sound of the one screen share it is watching, then for the cameras, each group in the order of the directory. It sends a new list whenever that changes: with every directory, when the occupancy or a publisher's `state` settles whose track is what, and when watching starts or stops. What is not asked for stops playing locally at once.
- The list for the sections an offer brings goes before the answer to that offer, without waiting for the offer to be applied, so the sections are asked for by the time they go live. The answer follows it in the same ordered outbox, which can hold the answer back by up to one `SendMediaSignal` round trip. Lists go at most every 250 ms, so the others' `state` frames cannot use up the hub's allowance for media signals, and the last one sent is always the latest wanted.
- Until a publisher's `state` names a stream, a stream with video counts as a share (see [Track directory](#track-directory)), so a camera and the voice in its stream are asked for once that `state` arrives. A newcomer has the others' `state` before the media server's first offer: the hub sends it the occupancy before anyone can answer it, and the offer comes only after a full negotiation. What a `state` announced stands until the publisher's next one, whichever media session is replaced meanwhile, and the stream of an announced camera stays the voice after the camera stops.
- `VoiceClient.setReceivePreferences({ screen, cameras })` selects one remote `{ userId, streamId }` or no screen, and whether cameras are received. Microphones are always asked for. The UI keeps cameras on and selects the screen through its watch controls.

Subscriptions choose whole tracks, and the media server then holds back for the receiver whatever pictures of them its downlink cannot carry. A track that is sent carries the publisher's encoded quality: no layer or bitrate is chosen per receiver.

### Negotiation rules

```mermaid
sequenceDiagram
    participant B as Browser
    participant H as AppHub (SignalR)
    participant M as stellarr-media (:5200)

    B->>H: JoinVoice(channelId)
    B->>H: OpenMediaSession(channelId)
    H->>M: POST /v1/sessions { roomId, userId }
    M-->>H: 201 { sessionId, epoch }
    H-->>B: { sessionId }
    B->>H: SendMediaSignal(offer)
    H->>M: POST /v1/sessions/{id}/offer
    M-->>H: Event: signal (answer)
    H-->>B: MediaSignal(answer)
    M-->>H: Event: signal (offer + tracks, negotiationId)
    H-->>B: MediaSignal(offer + tracks)
    B->>H: SendMediaSignal(subscriptions)
    H->>M: POST /v1/sessions/{id}/subscriptions
    B->>H: SendMediaSignal(answer, negotiationId)
    H->>M: POST /v1/sessions/{id}/answer
```

- **Browser offers (publishing)**: The browser sends an SDP offer whenever its published tracks change (initial join, camera toggle, screen-share toggle). The initial offer after `OpenMediaSession` must contain at least one m-line; a receive-only client offers a `recvonly` audio transceiver (answered `inactive`).
- **Server offers (forwarding)**: `stellarr-media` sends an offer whenever forwarded tracks for the session change, but only after answering the browser's first offer.
- **Ordering**: Signals from `stellarr-media` are delivered in sequence order. The browser serializes its outgoing `SendMediaSignal` calls, awaiting each hub response before sending the next.
- **Glare**: The browser is impolite and drops any incoming media-server offer while its own offer is in flight. `stellarr-media` is polite: if a browser offer arrives while a server offer is pending, it rolls back the server offer, answers the browser's offer, and issues a new server offer with an incremented `negotiationId`.
- **Stale answers and timeouts**: Each server offer carries a monotonically increasing `negotiationId` (starting at `1`), which the browser echoes in its `answer`. Answers with a non-matching `negotiationId` are rejected with `409 stale-negotiation`. If a server offer receives no answer or competing browser offer within 10 s, it is rolled back and re-offered; after 3 consecutive unanswered offers, the session is closed with `error`.
- **Immediate client recovery**: If the browser fails to apply a remote SDP description or `SendMediaSignal` is rejected by the hub, the client immediately replaces the media session rather than waiting for server timeouts.

> [!IMPORTANT]
> Published tracks must be added on dedicated transceivers via `addTransceiver(track, { direction: "sendonly", streams: [stream] })`. Do not use `addTrack`: browsers may attach `addTrack` to an existing server-created transceiver, which `stellarr-media` never accepts incoming media on (answering `sendonly` or `inactive`).

### Track directory

Because all remote media arrives over a single `RTCPeerConnection`, the client attributes incoming tracks using the server-provided directory:

- `mid`: Matches the receiving transceiver's SDP `a=mid` value, assigned by `stellarr-media`.
- `userId`: Lowercase UUID of the publishing user, bound from the authenticated seat.
- `streamId`: Stream identifier declared by the publisher (`""` if none), matched against `screenStreamId` and `cameraStreamId` in the publisher's `VoiceSignal` (`kind: "state"`) to distinguish microphone, camera, and screen tracks.
- **Shared microphone and camera stream**: A publisher places its microphone and camera tracks into a single `MediaStream` (one `msid`) per call and advertises that stream ID as `cameraStreamId` while the camera is active. `stellarr-media` preserves the publisher's `msid` in forwarded offers so receiving browsers keep voice and camera synchronized. Screen shares use a separate `MediaStream`.
- **Distinguishing microphone vs. screen audio**: On the receiving side, an incoming audio track is treated as the publisher's microphone if its stream is or was announced as `cameraStreamId` or if the audio track was registered on that stream before any video track. It is treated as screen-share audio if its stream matches `screenStreamId` or if a video track was registered on that stream first. Because the track directory lists a screen share's video track before its audio track, receivers classify screen audio accurately even before the publisher's `state` signal arrives.
- **Holding unattributed tracks**: Tracks whose `mid` is not yet in the directory, or whose `userId` is not in the current `VoiceParticipantChanged` occupancy snapshot, remain muted/unplayed until both include them.
- **In-place track swaps**: When a publisher switches capture devices and updates its stream ID without adding a new transceiver, `stellarr-media` emits a `tracks` signal without renegotiating SDP.

### `MediaSessionClosed`

```json
{ "channelId": "uuid", "sessionId": "opaque", "reason": "media-restarted | transport-timeout | error" }
```

Sent only when the media server closes a session while the user's voice seat is still active:

| Reason | Cause |
| --- | --- |
| `media-restarted` | `stellarr-media` restarted, the API reset it after an event-log gap, or a signal returned `404 unknown-session` |
| `transport-timeout` | ICE + DTLS + SRTP failed to connect within 30 s, or remained disconnected for 30 s |
| `error` | Protocol or resource limit exceeded (e.g. 3 unanswered offers or SDP size limit) |

Sessions closed because the user left the seat or opened a replacement session do not emit `MediaSessionClosed`.

## Seat and session lifecycle

A media session is bound to an active voice seat in `PresenceTracker`. `AppHub` and moderation endpoints close the media session whenever a seat changes:

| Trigger | Source |
| --- | --- |
| User leaves voice | `LeaveVoice` |
| SignalR connection disconnects | `OnDisconnectedAsync` |
| Newer browser tab takes the seat | `JoinVoice` (`VoiceSessionReplaced`) |
| User switches voice rooms | `JoinVoice` (from a previous room) |
| Direct call declined or user blocked | `DeclineDirectCall`, `SocialEndpoints` |
| Moderator kicks or moves user in voice | `VoiceModerationEndpoints` |
| Moderator kicks or bans user from server | `ModerationEndpoints` |

`PresenceTracker` detaches the session immediately in memory and enqueues a `DELETE /v1/sessions/{sessionId}` job in `MediaSessionCloser` (which retries every 2 s up to a bounded queue of 4,096 pending deletes). Calling `JoinVoice` again for the room the connection already occupies is a no-op and preserves the existing session.

## Client state and recovery

### `VoiceState` mapping

| State / Field | Condition |
| --- | --- |
| `peerCount` | Number of other participants seated in the room |
| `connectedPeerCount` | Number of seated participants whose microphone `inbound-rtp` packet counter is advancing (or who have an active transceiver while muted) |
| `transport.media = connecting` | Peer connection is not yet `connected`, or outgoing audio RTP packets have not yet advanced |
| `transport.media = ready` | Peer connection is `connected`, outgoing audio RTP is verified, and `peerCount === 0` |
| `transport.media = connected` | Peer connection is `connected`, and every other participant's microphone either arrives or is still within its arrival grace, with at least one arriving |
| `transport.media = degraded` | A microphone is missing past its arrival grace, connection is `disconnected`/`failed`, outgoing audio is unverified after 3 stats samples (`voiceClient.mediaServerStalled`), or session replacement is in progress |
| `transport.media = failed` | Automatic session replacement attempts exhausted (`voiceClient.mediaServerLost`) |

### Session recovery

- If the peer connection stays `disconnected` for more than 10 s, transitions to `failed`/`closed`, receives `MediaSessionClosed`, fails SDP application, or fails `OpenMediaSession`/`SendMediaSignal`, the client closes the `RTCPeerConnection` and opens a replacement media session while keeping local microphone, camera, and screen tracks alive.
- The client performs up to 3 automatic replacement attempts with backoff delays of `2 s`, `4 s`, and `8 s`. Once outgoing audio RTP is verified (or on a fresh `JoinVoice`), the retry counter resets. If all 3 attempts fail, `transport.media` enters `failed` (`voiceClient.mediaServerLost`).
- Clicking **Reconnect** in the call banner repairs the session in place: it refreshes ICE credentials, calls `JoinVoice` for the current room (keeping the seat without broadcasting a leave/join to other participants), and opens a new media session with a reset retry budget.

### Proving media quickly

- Transport health is verified from cumulative RTP counters (`getStats`), sampled on a 3 s interval. When the connection first reaches `connected` or a new participant joins, the client runs rapid initial checks at `250 ms`, `500 ms`, `1 s`, and `2 s` until RTP flow is confirmed, then resumes the normal 3 s interval.
- A newly joined participant's microphone is treated as pending (not missing) for a 3 s grace window starting from when the connection came up or when the participant joined, whichever is later.
- Each remote microphone transceiver is re-evaluated at most once every 500 ms; samples taken less than 500 ms apart keep the previous status so DTX silence packets (sent every 400 ms) are not mistaken for a stalled stream.

## Control API (`127.0.0.1:5200`)

`AppHub` is the only client of the HTTP/1.1 JSON control API:

- Every endpoint except `GET /v1/health` requires `Authorization: Bearer <secret>` (unless bound to loopback with an empty secret). Invalid or missing credentials return `401`.
- Error responses have the shape `{ "error": "<code>", "detail": "<text>" }`.
- `sessionId` and `epoch` are 32-character lowercase hex strings; `roomId` and `userId` are lowercase UUIDs.

| Endpoint | Success response | Error statuses |
| --- | --- | --- |
| `GET /v1/health` | `200 { "status": "ok", "version": "…", "epoch": "…", "rooms": 0, "sessions": 0 }` | None |
| `POST /v1/reset` | `200 { "epoch": "…", "seq": 0 }` | None |
| `POST /v1/sessions` (`{ "roomId", "userId" }`) | `201 { "sessionId": "…", "epoch": "…" }` | `422 invalid-request` |
| `DELETE /v1/sessions/{sessionId}` | `204` (idempotent for unknown sessions) | None |
| `POST /v1/sessions/{sessionId}/offer` (`{ "sdp" }`) | `204` | `404 unknown-session`, `422 invalid-sdp`, `422 limit-exceeded` |
| `POST /v1/sessions/{sessionId}/answer` (`{ "sdp", "negotiationId" }`) | `204` | `404 unknown-session`, `409 stale-negotiation`, `422 invalid-sdp` |
| `POST /v1/sessions/{sessionId}/candidate` (`{ "candidate" }`) | `204` | `404 unknown-session`, `422 invalid-candidate` |
| `POST /v1/sessions/{sessionId}/subscriptions` (`{ "mids": [...] }`) | `204` | `404 unknown-session`, `422 invalid-request` |
| `GET /v1/events?epoch=…&after=…&waitMs=…` | `200 { "epoch": "…", "reset": false, "events": [Event] }` | `422 invalid-request` |
| `GET /v1/stats` | `200 { "epoch": "…", "rooms": [...] }` | None |

Additional status codes: `413 payload-too-large` (request body > 256 KiB), `404 not-found`, `405 method-not-allowed`, `503 unavailable`.

### Event long-polling (`GET /v1/events`)

Returns immediately when events with `seq > after` exist, or waits up to `waitMs` (max `30000`; the API polls with `waitMs=25000` and a 30 s HTTP timeout). Returns up to 256 events per response:

```json
{ "seq": 41, "type": "signal", "sessionId": "…", "kind": "answer | offer | tracks", "payload": { } }
{ "seq": 42, "type": "closed", "sessionId": "…", "reason": "transport-timeout | error" }
```

- `epoch` identifies the `stellarr-media` process instance. If the requested `epoch` does not match the running epoch, or if `after` falls outside the 4,096-event ring buffer, `stellarr-media` responds immediately with `{ "epoch": "<current>", "reset": true, "events": [] }`.
- On `reset: true`, the API closes all tracked sessions from the stale epoch (emitting `MediaSessionClosed` with `media-restarted`), calls `POST /v1/reset` if the epoch was unchanged but the ring buffer overflowed, and resumes polling from `after=0`.

### Diagnostics (`GET /v1/stats`)

```json
{
  "epoch": "…",
  "rooms": [
    {
      "roomId": "uuid",
      "sessions": [
        {
          "sessionId": "…",
          "userId": "uuid",
          "state": "new | connecting | connected | disconnected",
          "ageMs": 0,
          "published": 2,
          "forwarded": 6,
          "ingress": { "packets": 0, "bytes": 0 },
          "egress": { "packets": 0, "bytes": 0 },
          "downlink": {
            "estimateBps": 431000,
            "rawEstimateBps": 512000,
            "smoothedEstimateBps": 431000,
            "measured": true,
            "forwardedBps": 281000,
            "desiredBps": 531000,
            "paddingBps": 0,
            "heldBack": [{ "mid": "pmQ", "userId": "uuid", "costBps": 250000 }],
            "holds": 2,
            "resumes": 1
          }
        }
      ]
    }
  ]
}
```

`ingress` and `egress` count media payload packets and bytes (excluding RTP headers, RTCP, and RTX retransmissions).

`downlink` is the session's [downlink adaptation](#downlink-adaptation), in bits per second of RTP payload: `estimateBps` is the estimate the allocation uses (the lower of str0m's latest raw estimate and its three-second smoothing, or the initial guess while `measured` is `false`), `forwardedBps` what is sent, `desiredBps` what the estimator probes toward, `paddingBps` the rate padding keeps up toward the first held picture while a trial runs, `heldBack` every picture held back with its publisher and its current cost, and `holds` and `resumes` how often a picture was held back and came back since the session began. Every change of what is held back is also logged at `debug` with the estimate, what is forwarded and what is desired.

## Media plane behavior

### Transport

- Single UDP socket (`0.0.0.0:49000`); incoming packets are demultiplexed by remote socket address (and by STUN username fragment on the initial connectivity check).
- Each session accepts connectivity checks from at most 64 distinct remote addresses over its lifetime.
- ICE-lite: `stellarr-media` responds to STUN checks and acts as DTLS server.
- Advertised host candidates (IPv4 only): `STELLARR_MEDIA_PUBLIC_IP:STELLARR_MEDIA_PUBLIC_PORT` (when set) followed by the primary local IPv4 address on the bound port (used for internal `coturn` relay).

### Forwarding

- Every session in a `roomId` negotiates the tracks published by the other sessions in that room, never its own, and is forwarded those its [receive subscriptions](#receive-subscriptions) ask for.
- Per connection, `stellarr-media` rewrites SSRC, payload type, RTP header extension IDs, and sequence numbers/timestamps while leaving media payloads untouched after SRTP decryption and re-encryption.
- Supported codecs: Opus, VP9, VP8, H.264, AV1. Publisher codec order is preserved, and publisher Opus `a=fmtp` parameters are echoed back so the browser configures its encoder according to the selected profile.
- Audio level RTP header extensions are forwarded.
- NACK, RTX, transport-wide congestion control (TWCC), and keyframe requests (PLI/FIR, rate-limited to once per second per track) are handled per leg. The TWCC feedback of each receiving browser drives the media server's estimate of that receiver's downlink.

### Downlink adaptation

Each receiving session estimates its own downlink and keeps what it sends inside it, so a receiver on a slow link keeps its voices while the pictures that do not fit are held back for it alone. Rates are bits per second of RTP payload, as str0m's estimate is; a link of a given rate on the wire carries less, all the less for small packets such as voices.

- **Estimate.** Every connection runs str0m's send-side estimator, a port of WebRTC's GoogCC, started at 1 Mbit/s and fed by the browser's transport-wide feedback. The allocation takes the lower of the latest raw estimate, read from the connection's statistics every 200 ms, and str0m's three-second smoothing of it, so a drop counts at once and a rise only once it has lasted. Until feedback has moved the estimate off its start and it has settled for a second, nothing is held back and video is not paced: a receiver is sent everything it asks for as it comes, as before there was an estimator. The initial value is a guess, and str0m's first probes report within that second; pacing against it would only hold a first keyframe back (at 1 Mbit/s, a quarter of a second for a 1.5 Mbit/s camera). Then what was sent so far is judged as a picture with room is, below, but against the lower of the two estimates, which the initial guess does not inflate: a link with no room for it sheds it within 2 s, or within 300 ms on a deep drop, while on a good link the estimate has those 2 s to grow past what is being sent.
- **Cost.** Every published track measures its own payload bitrate as it arrives, over the last two seconds (or, for a new track, over at least half a second), which is what forwarding it costs any receiver.
- **Allocation.** Every sound a receiver asks for is sent, whatever the estimate. Its pictures follow in the order of its list: each that fits in what the sounds and the pictures before it leave is taken, and one that does not is held back. A held picture still within reach of the estimate (needing at most 1.5 times it) keeps the pictures after it from being taken, so a later one never takes the room an earlier one waits for; one far out of reach does not, so a receiver that cannot have the share it watches still gets cameras that fit. The allocation runs again whenever the estimate, the list or the directory changes, and every 500 ms while the receiver asks for a picture, when a track's cost has moved by a fifth (and at least 8 kbit/s) or a clock below is running.
- **Holding back.** A held picture is paused exactly as a section left out of the list: its sequence numbers carry on where they stopped, its return asks the publisher for a keyframe through the one-second rate limit, and the receiver's own keyframe requests for it are ignored. Its directory entry says `heldBack: true`. Holding back is for that receiver alone; the publisher and every other receiver are untouched.
- **Hysteresis.** A sent picture is held back once what it needs has been above 1.1 times the smoothed estimate for 2 s (str0m paces at 1.1 times the estimate, and the smoothed estimate rides out the dip every congestion signal brings), or at once, after 300 ms, when the raw estimate falls below two thirds of what it needs. It comes back once what it needs has been within 85% of the estimate for 4 s; a picture held back again within 30 s of coming back waits twice as long each time, up to 64 s, and one that stays 30 s is steady again. A picture newly asked for is taken if it fits at once. One held back that never had room has no evidence the link carries it, and str0m's estimate can stand well above the link while little is sent: it comes back only during a padding trial toward it, once the estimate has held what it needs for 1 s while the padding sends that much. On a steady link nothing changes.
- **Probing.** The estimator is asked to look for what is sent plus the first held picture, and no more: str0m probes toward that (at most twice it), and with nothing held back toward what is sent. A picture within reach is also given padding: for six-second trials, padding keeps the rate it needs up, capped by the estimate and suppressed by str0m while the link shows congestion, with rests of 15, 30 and then 60 s after trials that do not bring it back; a rest ends early once the estimate has risen by a quarter since the trial before it. During a trial and for 5 s after, only the raw estimate's deep drop holds a sent picture back, never a dip of the media server's own making. A picture out of reach gets no padding, since a link that cannot carry it is not filled in front of the voices. Whenever a picture is held and no trial runs, and the estimate has stood still for 5 s, the estimator is started again at its current value, which ends str0m's loss-limited state (in which it never probes, and in which a lossy start can leave it on a link that has room) and sends its two first probes. Such looks then wait 10 s, twice as long each time, up to two minutes.
- **When not even the sounds fit.** They are all still sent, since there is nothing left to hold back, and the link drops what it cannot carry of them. Nothing else is added: no picture and no padding toward one, only str0m's own floor of 50 kbit/s of padding while less than that is sent.
- **Pacing.** Once the estimate counts, str0m paces video at 1.1 times it; audio is never paced and goes ahead of any video waiting, so voices gain no delay from it.
- **Bounds.** Everything a receiver sends (its list, its feedback) affects only what is sent to it. Directory updates for what is held back come at most as often as its lists, which the hub limits, or as the hysteresis allows, and the keyframe requests that returns bring go through the per-track rate limit. A withdrawn section's send stream is removed once the browser knows it is inactive, so no padding ever goes out on it.

### Resource limits

| Limit | Value | Behavior when exceeded |
| --- | ---: | --- |
| SDP length | 128 KiB | `422 invalid-sdp` on browser SDP; session closed with `error` if server SDP exceeds limit |
| Published tracks per session | 8 | `422 limit-exceeded` |
| Control API request body | 256 KiB | `413 payload-too-large` |
| Usable trickled ICE candidates per session | 64 | Subsequent candidates return `204` and are ignored |
| Remote peer addresses per session | 64 | Checks from additional addresses are dropped |
| Initial connection timeout (ICE + DTLS + SRTP) | 30 s | Closed with `transport-timeout` |
| Disconnected timeout | 30 s | Closed with `transport-timeout` |
| Consecutive unanswered server offers | 3 | Closed with `error` |
| Mids in one receive subscription list | 4,096 | `422 invalid-request`; the previous list stays in force |
| Return of a held-back picture | after 4 s of headroom, doubling to 64 s after each return that lasts under 30 s; one that never had room, after 1 s of padding toward it that the estimate holds | The picture stays held back meanwhile |
| Padding toward a held-back picture | 6 s trials, rests of 15, 30, then 60 s | Only while the picture needs at most 1.5 times the estimate |
| Fresh looks of a still estimate | after 5 s, then every 10 s doubling to 2 min | Only while a picture is held back and no padding trial runs |

### Current limitations

`stellarr-media` does not currently implement:

- **Layer and simulcast selection per receiver**: A receiver is sent a picture whole, at the publisher's quality, or not at all; nothing drops VP9 temporal layers or picks a simulcast encoding for a slow downlink, and sounds are always sent at their publisher's rate.
- **SDP m-line recycling**: Inactive m-lines are not reused within a session. A long session with many track toggles eventually reaches the 128 KiB SDP limit and automatically reconnects with a fresh session.
- **Active-speaker filtering**: Receive subscriptions and the downlink allocation select whole tracks in the order the browser gives; nothing follows who is speaking.
- **Session persistence across hub reconnects or ICE restarts**: The client opens a new session instead.
- **IPv6 or direct ICE-TCP**: The media socket uses IPv4 UDP only; clients restricted to TCP connect through `coturn`.

> [!WARNING]
> Call media is encrypted in transit between each browser and `stellarr-media` (DTLS-SRTP), not end-to-end encrypted between browsers. Because `stellarr-media` terminates SRTP on each leg to forward packets, audio and video are decrypted in process memory on the host running the container.

## Process configuration

### `stellarr-media` environment variables

| Variable | Default | Description |
| --- | --- | --- |
| `STELLARR_MEDIA_BIND` | `0.0.0.0:49000` | UDP media bind address |
| `STELLARR_MEDIA_PUBLIC_IP` | empty | Public IPv4 address advertised in ICE host candidates |
| `STELLARR_MEDIA_PUBLIC_PORT` | port of `STELLARR_MEDIA_BIND` | Public UDP port advertised with `STELLARR_MEDIA_PUBLIC_IP` |
| `STELLARR_MEDIA_ADVERTISE` | empty | Comma-separated `ip:port` list overriding derived host candidates |
| `STELLARR_MEDIA_CONTROL_BIND` | `127.0.0.1:5200` | HTTP control API bind address |
| `STELLARR_MEDIA_CONTROL_SECRET` | empty | Bearer token required by the control API (required when not bound to loopback) |
| `STELLARR_MEDIA_LOG` | `info` | `tracing` log filter (e.g. `info,stellarr_media=debug`) |

### ASP.NET Core API settings

| Setting | Default | Description |
| --- | --- | --- |
| `Rtc:MediaControlUrl` | `http://127.0.0.1:5200` | Base URL of the `stellarr-media` control API |
| `Rtc:MediaControlSecret` | empty | Shared bearer secret |
| `Rtc:MediaPort` | `49000` | Public UDP port advertised in `GET /api/bootstrap` (`1–65535`) |
