# Stellarr Media smoke test

End-to-end browser test suite that runs the `stellarr-media` release binary against headless Chromium instances to verify WebRTC negotiation, codec selection, RTCP feedback, and media forwarding as described in [`docs/stellarr-media.md`](../../docs/stellarr-media.md).

## Running

Requires Node.js 20+ and a local Chromium or Chrome binary.

```bash
cd media/smoke
npm ci
npm run smoke          # build release binary and run all scenarios
node smoke.mjs a e     # run specific scenarios against an existing binary
```

| Variable | Default | Description |
| --- | --- | --- |
| `STELLARR_SMOKE_BROWSER` | `/usr/bin/chromium` or `/usr/bin/google-chrome-stable` | Path to Chromium/Chrome executable |
| `STELLARR_SMOKE_OUT` | new directory under system temp | Output directory for logs, SDP dumps, and `results.json` |
| `STELLARR_SMOKE_ADVERTISE` | `127.0.0.1` | Candidate IP advertised by `stellarr-media` |
| `STELLARR_MEDIA_BINARY` | `../target/release/stellarr-media` | Path to `stellarr-media` binary |
| `STELLARR_MEDIA_LOG` | `info,stellarr_media=debug` | `tracing` filter for the media server |
| `STELLARR_SMOKE_ENGINE` | `chromium` | `firefox` or `webkit` runs Playwright's own build of that engine, which makes each person's tone and colour in the page instead of from fake devices |
| `STELLARR_SMOKE_E2EE` | unset | `1` runs every scenario as an end-to-end encrypted call (scenario `i` is left out: H.264 and AV1 cannot be sent encrypted); `streams` does so through Chromium's older `createEncodedStreams` |

Each scenario prints `PASS` or `FAIL` with the verified metrics and writes `stellarr-media.log`, `results.json`, and per-participant SDP logs (`<scenario>-<participant>-descriptions.json`) to `STELLARR_SMOKE_OUT`.

## Architecture

- `smoke.mjs` acts as the signaling hub: it spawns `stellarr-media` on ephemeral loopback ports, creates/deletes sessions via the HTTP control API, relays browser offers/answers/candidates/subscriptions, and long-polls `/v1/events` (optionally delaying events to stage glare collisions).
- `page/client.js` implements the browser side of the SFU protocol: a single `RTCPeerConnection` (`max-bundle`), `sendonly` transceivers for published tracks, impolite glare handling, track attribution via the server directory, and receive subscriptions that ask for every section of each directory, the voices first and then the pictures, since the order is the order of importance, before answering its offer (scenario `p` chooses its own).
- Each simulated participant runs in an isolated browser instance with a distinct synthetic audio frequency and video color (`lib/fixtures.mjs`; Chromium plays them from fake devices, Firefox and WebKit draw and play them in the page). Receivers verify both `RTCPeerConnection.getStats()` counters (`packetsReceived`, `framesDecoded`, `packetsSent`, keyframes, NACKs, RTX, candidate pairs) and the decoded audio frequencies and video frame colors against the track directory.
- The fake camera is a flat color that encodes to about 15 kbit/s. Scenarios that need a camera with a real cost draw one on a canvas instead: blocks of noise around the person's color, new every frame, capped by the encoder's `maxBitrate`. The noise averages out, so the measured color is still the person's.
- `lib/nat.mjs` is a relay in front of a second media server (scenarios `n`, `q` and `r`). Besides losing every n-th packet of one payload type, it can put everything the media server sends one browser through a shaped link: a rate cap counted in whole datagrams with UDP and IP headers, a queue of a bounded number of milliseconds at that rate that drops what does not fit, delay, jitter that never reorders, and losses in bursts from a two-state chain with a fixed seed. It also time-stamps every datagram it passes, which is how scenario `r` measures how long a voice spends in the media server.

## Scenarios

| ID | Verified behavior |
| --- | --- |
| `a` | Two-way audio forwarding without self-echo; audio-level RTP header extension delivery |
| `b` | Immediate PLI keyframe request when a new participant joins an already-running camera stream |
| `c` | Late third participant receives all four existing tracks attributed to the right `userId`, with directory delivered before `ontrack` |
| `d` | Leaving participant's forwarded m-lines transition to `inactive` and are removed from the directory |
| `e` | Offer glare collision: browser ignores pending server offer, server rolls back and re-offers after answering browser |
| `f` | Track swap (`replaceTrack` + `setStreams`) emits a `tracks` directory update without SDP renegotiation |
| `g` | Selected ICE candidate pairs terminate at `stellarr-media` |
| `h` | Demonstrates why `addTrack` is disallowed (`addTrack` attaches to a server-created m-line and is answered `recvonly`/`inactive`) |
| `i` | VP9, H.264, AV1, and VP9 temporal layers negotiate and decode across mismatched payload-type numbering |
| `j` | Publisher Opus `a=fmtp` parameters (128 kb/s mono mic, 160 kb/s stereo screen audio) are echoed and applied by the encoder |
| `k` | Toggling camera off/on (`removeTrack` or `transceiver.stop()`) allocates a fresh forwarded m-line |
| `l` | 50-second continuous call remains connected past all internal inactivity timers |
| `m` | Receive-only participant (`recvonly` initial offer) receives all published tracks |
| `n` | Simulated NAT with 1-in-15 packet loss in both directions recovers via per-leg NACK and RTX |
| `o` | Unanswered server offers time out after 10 s and close the unresponsive session with `error` after 3 strikes without affecting other peers |
| `p` | A receiver that asks only for the microphones gets nothing of a new screen video and its sound until it asks for them, downloads nothing of them while it stops asking, and resumes with a decoded keyframe, contiguous sequence numbers and no renegotiation; the microphones and another viewer carry on |
| `q` | Throttled downlink (300 ms queue, 40 ms delay, 15 ms jitter, 1% burst loss) stepped through 512, 256, and 128 kbit/s then restored: voice stays inside loss/latency bounds, at 512 kbit/s only the prioritized camera decodes while the second holds back, below 512 both hold back with zero traffic, held states stay stable, and both resume within 75 s once unthrottled |
| `r` | On a clean link: how soon five newcomers in turn decode a 1.5 Mbit/s camera that is already running, and how long a voice spends in the media server while it flows, measured at the relay from the RTP timestamps the media server keeps |

### Encrypted calls

These run only when named (`node smoke.mjs xa xb`). Every person runs the web client's own frame worker (`web/src/stellarr/lib/voice/e2ee/`, served with its types stripped) on every sender and receiver, keyed from an epoch the script hands out, as an end-to-end encrypted private call does, and offers only Opus, VP8 and VP9 unless a scenario asks for a codec.

| ID | Verified behavior |
| --- | --- |
| `xa` | Codec matrix: the voice, VP8, VP9 and VP9 with temporal layers are sealed, forwarded, opened and decoded in the publisher's tone and colour; H.264 and AV1 never leave the publisher |
| `xb` | A wrong key: the media server forwards everything, the receiver opens and plays nothing, says the sound and picture are failing and asks for keyframes |
| `xc` | Rotation during a call: with the new epoch handed to both sides before either sends under it nothing is dropped; when the sender moves first, the receiver drops that epoch's frames, asks for it at most once a second and decodes again once it has it |
| `xd` | A browser without encoded transforms publishes nothing in an encrypted call, and nothing of it reaches anybody |
| `xe` | Cost (Chromium only): the whole browser's CPU for a voice and a 1.2 Mbit/s camera each way, without a transform, through a worker that only passes frames on, one that copies them, and encrypted, interleaved twice; and the frame format's own cost by frame size |
| `xf` | A speech-like voice sent encrypted with and without a constant bitrate: the sizes of the frames the sender sealed, and the bitrate |
