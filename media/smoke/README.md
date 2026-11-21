# Stellarr Media smoke test

End-to-end browser test suite that runs the `stellarr-media` release binary against headless browser instances to verify WebRTC negotiation, codec selection, RTCP feedback, downlink congestion allocation, end-to-end encrypted media transforms, and RTP forwarding as specified in [`docs/stellarr-media.md`](../../docs/stellarr-media.md).

## Running

Requires Node.js 20+ and a local Chromium or Chrome binary.

```bash
cd media/smoke
npm ci
npm run smoke          # build release binary and run all default scenarios
node smoke.mjs a e     # run specific scenarios against an existing binary
```

| Variable | Default | Description |
| --- | --- | --- |
| `STELLARR_SMOKE_BROWSER` | `/usr/bin/chromium` or `/usr/bin/google-chrome-stable` | Path to Chromium/Chrome executable |
| `STELLARR_SMOKE_OUT` | new directory under system temp | Output directory for logs, SDP dumps, and `results.json` |
| `STELLARR_SMOKE_ADVERTISE` | `127.0.0.1` | Candidate IP advertised by `stellarr-media` |
| `STELLARR_MEDIA_BINARY` | `../target/release/stellarr-media` | Path to `stellarr-media` binary |
| `STELLARR_MEDIA_LOG` | `info,stellarr_media=debug` | `tracing` filter for the media server |
| `STELLARR_SMOKE_ENGINE` | `chromium` | Browser engine (`chromium`, `firefox`, or `webkit`). `firefox` and `webkit` launch Playwright's browser builds and synthesize participant audio tones and video frames via Web Audio / canvas inside the page instead of Chromium fake-device flags |
| `STELLARR_SMOKE_E2EE` | unset | `1` runs all compatible scenarios with SFrame end-to-end media encryption enabled (`i` is skipped because H.264 and AV1 are disallowed in E2EE calls); `streams` forces Chromium's legacy `createEncodedStreams` path |

Each scenario prints `PASS` or `FAIL` with verified metrics and writes `stellarr-media.log`, `results.json`, and per-participant SDP logs (`<scenario>-<participant>-descriptions.json`) to `STELLARR_SMOKE_OUT`.

## Architecture

- `smoke.mjs` acts as the signaling hub: it spawns `stellarr-media` on ephemeral loopback ports, creates and deletes sessions via the HTTP control API, relays browser offers, answers, ICE candidates, and track subscriptions, and long-polls `/v1/events` (optionally delaying events to stage glare collisions).
- `page/client.js` implements the browser side of the SFU protocol: a single `RTCPeerConnection` (`max-bundle`), `sendonly` transceivers for published tracks, impolite glare handling, track attribution via the server directory, and explicit receiver subscriptions (`POST /v1/rooms/{roomId}/sessions/{sessionId}/subscriptions`) ordered with all audio mids before video mids (scenario `p` exercises selective subscriptions directly).
- Each simulated participant runs in an isolated browser context with a distinct synthetic audio tone frequency and video frame color (`lib/fixtures.mjs`). Receivers verify `RTCPeerConnection.getStats()` counters (`packetsReceived`, `framesDecoded`, `packetsSent`, keyframes, NACKs, RTX, selected candidate pairs) as well as decoded audio frequencies and video frame colors against the track directory.
- Chromium's flat-color fake camera encodes at ~15 kbit/s. Scenarios that test bandwidth allocation and bitrate-heavy video streams render animated per-frame noise blocks around the participant's signature color onto a canvas track capped by the sender's `maxBitrate`; spatial averaging recovers the participant's base color at the receiver.
- `lib/nat.mjs` is a programmable UDP relay placed in front of a second `stellarr-media` instance (scenarios `n`, `q`, and `r`). It supports deterministic `1-in-N` payload-type packet drops, downlink traffic shaping (token-bucket rate cap including IP/UDP header overhead, bounded millisecond queue depth with tail drop, propagation delay, in-order jitter, and seeded two-state Markov burst loss), and per-packet transit timestamping used by scenario `r` to measure SFU forwarding latency.

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
| `p` | Selective receiver subscription: subscribing only to microphone mids forwards zero screen-video or screen-audio packets until subscribed, stops forwarding immediately on unsubscription, and resumes with a fresh PLI keyframe and contiguous RTP sequence numbers without SDP renegotiation |
| `q` | Shaped downlink (`300 ms` queue, `40 ms` delay, `15 ms` jitter, `1%` burst loss) stepped through `512`, `256`, and `128 kbit/s` and then restored: audio remains within loss and latency bounds throughout; at `512 kbit/s` only the higher-priority video mid is forwarded while the second is marked `heldBack`; at `256` and `128 kbit/s` both video mids are held back with zero forwarded video bytes and no state flapping; both video streams resume within 75 s once unthrottled |
| `r` | Clean-link baseline: measures time-to-first-decoded-frame for five sequential late joiners subscribing to an active `1.5 Mbit/s` video stream, and measures audio packet transit time through `stellarr-media` via UDP relay timestamps |

### Encrypted call scenarios

Encrypted call scenarios run when explicitly named on the command line (`node smoke.mjs xa xb`). Each participant loads the web client's production E2EE media worker (`web/src/stellarr/lib/voice/e2ee/`) on every sender and receiver transceiver, keyed with shared epoch keys, and restricts video codecs to VP8 and VP9 unless the scenario tests codec rejection.

| ID | Verified behavior |
| --- | --- |
| `xa` | Encrypted codec matrix: Opus audio, VP8, VP9, and VP9 temporal layers are encrypted, forwarded, decrypted, and decoded with matching frequency and color; H.264 and AV1 are blocked from being published |
| `xb` | Key mismatch: `stellarr-media` forwards the encrypted packets unmodified, while the receiver with an invalid key rejects all frames, emits decryption failure diagnostics, and requests PLI keyframes without playing corrupted media |
| `xc` | In-call epoch rotation: pre-distributed epoch keys rotate with zero dropped frames; when a sender rotates before the receiver receives the new epoch key, the receiver drops frames for the unknown epoch, throttles missing-epoch notifications to at most once per second, and resumes decoding as soon as the key arrives |
| `xd` | Fail-closed capability guard: a browser context lacking WebRTC Encoded Transform support refuses to publish any plaintext media tracks in an encrypted call |
| `xe` | Encryption CPU overhead benchmark (Chromium only): compares browser CPU usage and per-frame transform cost across four modes (unencrypted baseline, no-op worker pass-through, worker buffer copy, and full AES-256-GCM SFrame encryption) for bidirectional audio plus `1.2 Mbit/s` video |
| `xf` | Encrypted Opus frame size distribution and bitrate verification with and without Opus constant bitrate (`cbr=1`) enabled |
