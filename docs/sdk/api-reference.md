# API and CLI reference

Complete technical reference for `@stellarr/sdk` (`0.4.0`), `/_stellarr/sdk/v1.js`, `stellarr.json`, and the `stellarr` CLI.

---

## Entry points

```ts
// With a bundler (Vite, Rollup, esbuild):
import { connect, detectDevice } from "@stellarr/sdk";

// Without a bundler (served automatically on the activity origin):
import { connect, detectDevice } from "/_stellarr/sdk/v1.js";
```

| Function | Signature | Description |
| --- | --- | --- |
| `connect<S>` | `connect<S extends ActivityState>(options?: { initialState?: S }): Promise<DevSession<S>>` | Connects the activity iframe to the surrounding Stellarr host (or `stellarr dev` shell). Initializes room state to `initialState` (`{}` default) once if the room is new. Throws if opened outside Stellarr / `stellarr dev` or if the handshake times out (`12 s`). |
| `detectDevice` | `detectDevice(): "desktop" \| "tablet" \| "phone"` | Classifies the physical device from primary pointer precision and `window.screen` dimensions (`desktop` = fine pointer; `tablet` = coarse pointer with `min(width, height) ≥ 600`; `phone` = smaller coarse touchscreen). |

---

## `DevSession<S>` interface

### Properties and lifecycle

| Member | Type | Description |
| --- | --- | --- |
| `context` | `ActivityContext` | `{ standalone, launchMode, sessionId, me, place, locale, device }`. `place` is `{ id: string }` in `"voice"` mode and `null` in `"personal"` mode; `standalone` is `true` in `stellarr dev` and `false` on a Stellarr instance. |
| `limits` | `ActivityLimits` | Active room limits (`stateBytes`, `messageBytes`, `writesPerSecond`, `messagesPerSecond`, `graceSeconds`, `emptySeconds`, `maxParticipants`). |
| `capabilities` | `string[]` | Supported host features (e.g. `"context"`, `"state"`, `"theme"`, `"participants"`, `"messages"`, `"clock"`, `"seats"`, `"progress"`, plus `"host"`, `"commands"`, `"assignments"` when `authority === "host"`, `"shell.dock"` where the page draws a dock, and granted `permissions`). |
| `now()` | `() => number` | Estimated server timestamp in milliseconds (continuously RTT-compensated). |
| `close()` | `() => Promise<void>` | Leaves the session immediately, disposes active media handles/mounts, and sets `connection` to `"closed"`. |

### Observables (`Observable<T>`)

Every `Observable<T>` provides `get(): T` (returns a `structuredClone` snapshot) and `subscribe(listener: (value: T) => void): () => void` (invokes `listener` synchronously on registration with the current value, then whenever the value changes).

| Observable | Value type | Description |
| --- | --- | --- |
| `connection` | `"connected" \| "reconnecting" \| "closed"` | Transport state (`401`/`403`/`404`/`410` transitions permanently to `"closed"`). |
| `theme` | `ActivityTheme` | `{ mode: "light" \| "dark", colors: { background, surface, text, muted, accent, border } }`. |
| `locale` | `string` | Active Stellarr UI locale (`"en"`, `"pl"`, or browser BCP 47 tag in local preview). |
| `device` | `DeviceClass` | `"desktop" \| "tablet" \| "phone"`. |
| `participants` | `Participant[]` | Ordered roster: `{ id, name, username?, avatar: string \| null, connected }`. |
| `seats` | `Observable<string[]> & { claim(): Promise<void>; release(): Promise<void> }` | IDs of seated players (`claim()` takes a free seat or rejects `409`; `release()` steps down to spectator and clears the caller's secret assignment). |
| `host` | `Observable<string \| null> & { transfer(id: string): Promise<void> }` | Current host ID (`null` under `"shared"` authority). `transfer(id)` hands host control to another connected seated player. |
| `assignments` | `Observable<JsonValue> & { checkpoint(): Promise<{ revision: number; state: S; assignments: Record<string, JsonValue> }> }` | The player sees their own assignment (`null` if none or spectator). The current host can read every assignment through `checkpoint()`, including after a host transfer. |

### State, commands, messages, and progress

| API | Signature | Behavior |
| --- | --- | --- |
| `state.get` | `() => S` | Deep-cloned latest confirmed room state. |
| `state.subscribe` | `(listener: (state: S, revision: number) => void) => Unsubscribe` | Fires immediately, then in strictly increasing `revision` order. |
| `state.set` *(patch)* | `(patch: Partial<S>) => Promise<number>` | Replaces specified top-level keys and resolves to the new `revision` (requires a seat, and host role if `authority === "host"`). |
| `state.set` *(fn)* | `(change: (current: S) => Partial<S> \| void) => Promise<number \| undefined>` | Pure optimistic transaction: retries automatically on revision conflict; returning `undefined` cancels without writing. |
| `commands.handle` | `(kind: string, handler: (data, meta: { id, from }, state: S, priv: { assignments }) => CommandDecision<S> \| void \| Promise<...>) => Unsubscribe` | Registers a host command processor returning `{ patch?, result?, assignments? }` (`assignments[id] = null` deletes a secret). Throwing rejects the caller's `submit()` with `error.message` (`≤ 512` chars). |
| `commands.submit` | `(kind: string, data: JsonValue, options?: { id?: string }) => Promise<JsonValue \| undefined>` | Submits a command from a seated player (`authority: "host"`). Syncs local `state` to the decided revision before resolving. Passing `options.id` (`1–128` chars) makes retries idempotent. |
| `send` | `(kind: string, data: JsonValue \| ArrayBuffer \| ArrayBufferView, options?: { to?: string; latest?: boolean }) => boolean` | Queues a transient message (`kind` `1–128` chars; `32`-slot outbox). Returns `false` if disconnected, full, or `> messageBytes`. |
| `on` | `(kind: string, listener: (data: JsonValue \| Uint8Array, meta: { from: string }) => void) => Unsubscribe` | Subscribes to transient messages of `kind` (binary payloads arrive as `Uint8Array`). |
| `progress.load` | `() => Promise<{ revision: number; data: ActivityState \| null }>` | Loads the signed-in account's private save (`{ revision: 0, data: null }` before first save). |
| `progress.save` | `(data: ActivityState, expectedRevision: number) => Promise<number>` | Atomically replaces the account's save (`≤ 64 KiB` JSON object) when `expectedRevision` matches, returning the new revision (`409` on conflict). |

### Media (`session.media`)

| API | Signature | Requires permission / host |
| --- | --- | --- |
| `media.volume` | `{ get(): number; subscribe(fn: (v: number) => void): () => void }` | None (`0..1` float) |
| `media.embed.mount` | `(url: string, element: Element, options?: EmbedOptions) => EmbedHandle` | HTTPS embed URL on a host in `connects`. |
| `media.microphoneLevel.open` | `() => Promise<DeviceHandle & { subscribe(fn: (level: number) => void): () => void }>` | `"media.microphoneLevel"` (`60 s` prompt timeout) |
| `media.cameraTile.open` | `() => Promise<DeviceHandle>` | `"media.cameraTile"` (`60 s` prompt timeout) |
| `media.cameraTile.mount` | `(participantId: string, element: Element) => CameraTileMount` | `"media.cameraTile"` |
| `media.gifs.search` | `(query?: string, options?: { offset?: number; limit?: number }) => Promise<GifPage>` | `"media.gifs"` (`30 s` timeout; `limit` `1–24`, default `12`) |
| `media.gifs.load` | `(id: string, size?: "preview" \| "full") => Promise<GifImage>` | `"media.gifs"` (`60 s` timeout; `"preview"` default) |

`GifPage` is `{ items: GifItem[]; offset: number; total: number }`, with `GifItem` = `{ id, alt?, width, height }`; an empty query lists trending GIFs. `GifImage` is `{ url, type, width, height, alt?, release() }`: `url` is a `blob:` URL the SDK makes inside the frame from bytes the host sends, `type` is `image/gif` or `image/webp` (`image/svg+xml` in `stellarr dev`), and `release()` revokes the URL (closing the session revokes the rest). The frame never contacts GIPHY. Rejections are `GifError`s whose `code` is `"permission"`, `"unavailable"` (no GIFs on the instance), `"declined"` (the player turned GIPHY off), `"rateLimited"`, `"notFound"` or `"failed"`. The `GifError`, `GifErrorCode`, `GifImage`, `GifItem` and `GifPage` types are exported from `@stellarr/sdk`. The GIF search section of [Media, manifest, and publishing](media-and-publishing.md) has the limits and an example.

`EmbedOptions` accepts `title?: string`, `onLoad?(): void`, and `onMessage?(data: unknown): void`. `EmbedHandle` provides `setVisible(visible: boolean)`, `postMessage(data: unknown)`, and `destroy()`. `onMessage` receives messages from the mounted provider window and origin. `CameraTileMount` is callable to unmount the tile and has a `setVisible(visible: boolean)` method. Both visibility controls apply to the local viewer.

`DeviceHandle` exposes `stop(): void` and `onEnded(listener: (reason: string) => void): () => void`.

### The page around the activity (`session.shell`)

In full screen Stellarr keeps a small dock at the bottom edge: it shows for a moment on the way in, comes up when the pointer reaches the edge, and drops when the pointer leaves it. A game that uses that edge itself (edge scrolling, a bottom HUD) can hold it.

| API | Signature | Behavior |
| --- | --- | --- |
| `shell.dock` | `Observable<DockState>` | `{ fullscreen, visible, held, height }`: whether the activity has the screen, whether the dock is showing, whether a hold is in force, and the CSS pixels the visible dock takes from the bottom of the frame (`0` while hidden). |
| `shell.dock.hold` | `() => { release(): void }` | While full screen, the edge no longer brings the dock up and it does not show on the way in. Holds count; the last `release()` ends it, and it also ends when the frame reloads or reconnects. The player keeps a small handle at the bottom edge, the keyboard and Esc, and may refuse holds altogether in the activity's settings. |
| `shell.dock.peek` | `() => void` | Shows the dock for about 2.5 s, for example after a round. At most once every 10 s. |

Where the page draws no dock (`stellarr dev`, older instances) `capabilities` lacks `"shell.dock"`, the dock stays `{ fullscreen: false, visible: false, held: false, height: 0 }` and `hold()` and `peek()` do nothing.

```ts
const round = session.shell.dock.hold();
session.shell.dock.subscribe(({ visible, height }) => {
  hud.style.bottom = visible ? `${height}px` : "0";
});
// When the round is over:
round.release();
session.shell.dock.peek();
```

---

## CLI reference (`stellarr`)

```sh
# Local multi-tab simulator (default port 4174, --port 0 for ephemeral port)
stellarr dev [--dir <folder>] [--port <port>] [--mode voice|personal]

# Live instance voice-channel preview (Cockpit pairing code)
stellarr dev [--dir <folder>] --instance <origin> [--pair | --stop <tester-invite>]

# Validate published manifest, artwork magic bytes, and asset limits
stellarr validate [--dir <folder>]

# Build deterministic release ZIP(s) + stellarr-release.json (default --out .stellarr-release)
stellarr pack [--dir <folder> ...] [--out <release-folder>]
```

- `stellarr pack` accepts `1` to `64` `--dir` flags when packaging multiple activities into one release index (requires `zip` on Linux/macOS or PowerShell on Windows).
- Automation can pipe a single-line pairing code into `stellarr dev --instance <origin> --pair`.

---

## Allowed static files, errors, and limits

### Allowed packaged file types (`sdk/dev/files.js`)

| Category | Extensions / filenames | Notes |
| --- | --- | --- |
| **Documents & code** | `.html`, `.css`, `.js`, `.mjs`, `.json`, `.txt`, `.wasm`, `LICENSE` | UTF-8 `stellarr.json` + `index.html` required at root |
| **Images** | `.svg`, `.png`, `.jpg`, `.jpeg`, `.webp`, `.gif`, `.ico`, `.avif` | `icon` (`≤ 512 KiB`) & `cover` (`≤ 2 MiB`) must be PNG/JPEG/WebP |
| **Audio & video** | `.mp3`, `.opus`, `.ogg`, `.wav`, `.mp4`, `.webm` | Served with `Accept-Ranges: bytes` (`206 Partial Content`) |
| **Fonts & 3D/binary** | `.woff`, `.woff2`, `.ttf`, `.otf`, `.gltf`, `.glb`, `.bin`, `.data` | Paths `≤ 512` bytes; dotfiles, `node_modules`, `_stellarr`, and `package*.json` skipped |

### Error status codes (`error.status`)

| Status | Meaning |
| --- | --- |
| `400` | Invalid JSON object, malformed `id`/`kind`, non-seated assignment target, or writing state directly under `"host"` authority. |
| `403` | Retired frame (`This frame is no longer active.`), spectator attempting a player write (`A playing seat is required.`), or revoked room access. |
| `404` | Session or command not found. |
| `409` | `progress.save` revision conflict, all seats taken on `seats.claim()`, stale host epoch, or duplicate command `id` with mismatched payload. |
| `410` | Session ended (`connection` transitions to `"closed"`). |
| `413` | Payload exceeds `stateBytes` (`256 KiB`), `messageBytes` (`64 KiB`), `progress` (`64 KiB`), preview (`16 MiB`), or release (`256 MiB`). |
| `429` | Rate limit reached (`32` writes/s or `96` msgs/s), room at `maxParticipants` (`128`), or `128` pending undecided host commands. |

*(Transient `5xx` and network errors are retried automatically with `200 ms`–`2,000 ms` exponential backoff by `connect()` operations.)*
