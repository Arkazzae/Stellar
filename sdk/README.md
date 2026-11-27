# `@stellarr/sdk`

Build multiplayer games, watch parties, polls, and solo mini-apps that run directly inside Stellarr.

An Activity is just a **static web app** (`stellarr.json` + `index.html` + assets)—no game servers to host, no auth flows to wire up, and no WebRTC plumbing to maintain. Stellarr serves your bundle from an isolated origin, identifies everyone in the room, and gives you a single `connect()` client for shared state, host-authoritative commands, secret player hands, binary/JSON events, cross-device saves, and consented mic/camera inputs.

> [!TIP]
> **Browsing the full documentation?** Read the multi-page SDK guides in [`docs/sdk/`](../docs/sdk/getting-started.md):
> [Getting started](../docs/sdk/getting-started.md) · [Rooms, state & saves](../docs/sdk/rooms-and-state.md) · [Media, manifest & publishing](../docs/sdk/media-and-publishing.md) · [API & CLI reference](../docs/sdk/api-reference.md) · [Platform contract](CONTRACT.md)

---

## Build a playable activity in 60 seconds

No bundler is required to try the SDK. Create an empty folder with two files:

**1. `stellarr.json`**

```json
{
  "protocol": 1,
  "id": "quick-buzz",
  "name": "Quick Buzz",
  "description": "First player to hit the buzzer claims the round.",
  "version": "0.1.0",
  "launchModes": ["voice", "personal"]
}
```

**2. `index.html`**

```html
<!doctype html>
<html lang="en">
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Quick Buzz</title>
<main>
  <p id="status">Waiting for someone to buzz…</p>
  <button id="buzz" disabled>Buzz!</button>
  <button id="reset" disabled>Next round</button>
</main>
<script type="module">
  import { connect } from "/_stellarr/sdk/v1.js";

  const session = await connect({ initialState: { winnerId: null } });
  const statusEl = document.getElementById("status");
  const buzzBtn = document.getElementById("buzz");
  const resetBtn = document.getElementById("reset");

  function render(state = session.state.get(), roster = session.participants.get()) {
    const winner = roster.find((p) => p.id === state.winnerId);
    statusEl.textContent = winner ? `${winner.name} buzzed first!` : "Waiting for someone to buzz…";
    buzzBtn.disabled = state.winnerId !== null;
    resetBtn.disabled = state.winnerId === null;
  }

  session.state.subscribe(() => render());
  session.participants.subscribe(() => render());

  buzzBtn.onclick = () =>
    session.state.set((cur) => (cur.winnerId ? undefined : { winnerId: session.context.me.id }));
  resetBtn.onclick = () => session.state.set({ winnerId: null });
</script>
</html>
```

**3. Run the local multi-player simulator** (Node.js `22+`):

```sh
npx @stellarr/sdk dev --dir .
```

Open the printed `http://127.0.0.1:4174/` URL and click **New player** in the top bar to join the same room from a second tab. Notice how `session.state.set((cur) => ...)` runs an optimistic transaction—if two tabs click **Buzz!** at the exact same millisecond, only the first wins and the second cancels cleanly by returning `undefined`.

---

## Using a bundler (Vite / TypeScript)

Install `@stellarr/sdk` in your project for full TypeScript autocomplete and module bundling:

```sh
npm install --save-dev @stellarr/sdk
```

```ts
import { connect } from "@stellarr/sdk";

interface GameState {
  round: number;
  scores: Record<string, number>;
}

const session = await connect<GameState>({
  initialState: { round: 1, scores: {} },
});
```

Point the CLI at your build output directory (`stellarr.json` + `index.html` at its root):

```sh
npx stellarr validate --dir dist
npx stellarr dev --dir dist
```

---

## Choosing the right data channel

`@stellarr/sdk` gives you four distinct primitives so you never have to abuse one mechanism for another:

| Primitive | Persistence & visibility | Ideal for | Limit |
| --- | --- | --- | --- |
| **`session.state`** | Shared across room; survives reloads (`emptySeconds: 120 s`) | Lobby state, scores, queues, board positions (`authority: "shared"`) | `256 KiB` JSON, `32` writes/s |
| **`session.commands` + `assignments`** | Host-decided state + per-seat secret state (`authority: "host"`) | Turn validation, trivia answers, hidden poker hands, charades prompts | `64 KiB` cmd/result, `256 KiB` assignments |
| **`session.send` / `session.on`** | Ephemeral, best-effort (broadcast or unicast `to`, JSON or `Uint8Array`) | Pointers, drawing strokes, 60 Hz physics inputs (`latest: true`) | `64 KiB` / msg, `96` msgs/s |
| **`session.progress`** | Private to each signed-in account across sessions, updates & devices | High scores, unlocked levels, personal keybinds/preferences | `64 KiB` JSON |

---

## Common patterns

### 1. Match the player's Stellarr theme, locale, and device

Every Observable (`theme`, `locale`, `device`, `participants`, `seats`, `host`, `assignments`, `connection`) exposes `.get()` and `.subscribe(fn)`—invoking `fn` immediately with the current value and again whenever it changes during play:

```ts
session.theme.subscribe(({ mode, colors }) => {
  const root = document.documentElement;
  root.dataset.theme = mode; // "dark" | "light"
  for (const [key, hex] of Object.entries(colors)) {
    root.style.setProperty(`--stellarr-${key}`, hex);
    // --stellarr-background, -surface, -text, -muted, -accent, -border
  }
});

session.locale.subscribe((locale) => {
  document.documentElement.lang = locale; // e.g. "en" or "pl"
});

session.device.subscribe((device) => {
  document.documentElement.dataset.device = device; // "desktop" | "tablet" | "phone"
});
```

### 2. Host-authoritative games, spectators, and secret hands

Set `"authority": "host"` (and optionally `"seats": 4`) in `stellarr.json`. Stellarr automatically elects a seated connected player as host and fails over to the next player if the host disconnects past `graceSeconds` (`10 s`). Register the **same** `commands.handle` logic on every client so any player can take over seamlessly:

```ts
// Register on EVERY client so host failover works automatically:
session.commands.handle("deal", (_data, _meta, state) => {
  const seated = session.seats.get();
  const deck = shuffleDeck();
  const assignments = Object.fromEntries(seated.map((id) => [id, deck.splice(0, 5)]));
  return {
    patch: { round: state.round + 1, turn: seated[0] },
    assignments, // delivered privately: each player only sees their own hand!
    result: "dealt",
  };
});

// Any seated player submits a move (pass a stable `id` for safe network retries):
await session.commands.submit("deal", null, { id: `deal-${nextRound}` });

// Each seated player receives ONLY their own secret assignment:
session.assignments.subscribe((myHand) => {
  renderHand(myHand); // null for spectators or when cleared with `assignments: { [id]: null }`
});
```

Extra participants beyond `"seats"` join as spectators (`!session.seats.get().includes(session.context.me.id)`) and can claim an open seat with `await session.seats.claim()` or step down with `await session.seats.release()`.

### 3. Shared countdown timers with `session.now()`

Local device clocks drift. Store deadlines in server time using `session.now()` so every tab renders the exact same remaining time:

```ts
// Starting a 30-second round:
await session.state.set({ roundEndsAt: session.now() + 30_000 });

// Rendering the countdown locally:
const remainingSeconds = Math.max(0, Math.ceil((state.roundEndsAt - session.now()) / 1000));
```

### 4. High-frequency cursors or binary packets (`send` / `on`)

Pass `{ latest: true }` so slow connections coalesce older cursor/position updates instead of building up lag:

```ts
window.onpointermove = (e) => {
  session.send("cursor", { x: e.clientX / innerWidth, y: e.clientY / innerHeight }, { latest: true });
};

session.on("cursor", (pos, { from }) => {
  moveRemoteCursor(from, pos as { x: number; y: number });
});
```

`session.send` also accepts `Uint8Array` or `ArrayBuffer` (delivered to `session.on` listeners as `Uint8Array`).

### 5. Persistent per-player saves (`session.progress`)

Saves belong to the signed-in account (`≤ 64 KiB` JSON) and persist across voice rooms, personal launches, server restarts, and activity updates:

```ts
async function recordHighScore(score: number) {
  for (;;) {
    const { data, revision } = await session.progress.load();
    if (((data?.bestScore as number) ?? 0) >= score) return;
    try {
      await session.progress.save({ ...data, bestScore: score }, revision);
      return;
    } catch (err) {
      if ((err as { status?: number }).status !== 409) throw err;
    }
  }
}
```

### 6. Game audio volume, voice level, and camera tiles

- **Game audio (`0..1`):** Always scale your audio graph or `<audio>` elements by `session.media.volume` so each listener's **Game audio** slider in Stellarr works out of the box:

  ```ts
  gainNode.gain.value = session.media.volume.get();
  session.media.volume.subscribe((v) => { gainNode.gain.value = v; });
  ```

- **Microphone level (`"permissions": ["media.microphoneLevel"]`):** Prompts the player for consent and streams normalized `0..1` voice amplitude—ideal for voice-reactive games without exposing raw audio samples:

  ```ts
  const mic = await session.media.microphoneLevel.open();
  mic.subscribe((level) => { if (level > 0.6) jumpCharacter(); });
  mic.onEnded((reason) => showKeyboardFallback(reason));
  ```

- **Camera tile (`"permissions": ["media.cameraTile"]`):** Prompts the performer for consent while any participant calls `session.media.cameraTile.mount(performerId, placeholderEl)`. Stellarr renders the video overlay outside the iframe's readable DOM and tracks `placeholderEl` automatically. Call `setVisible(false)` on the returned unmount function to hide the tile locally.
- **External embeds (`session.media.embed`)**. Declare the service hostname in `connects`, then call `session.media.embed.mount(embedUrl, placeholderEl, { title, onMessage })`. The handle supports `setVisible(boolean)`, `postMessage(data)`, and `destroy()`. Use the service’s own embed URL and player protocol.
- **GIF search (`"permissions": ["media.gifs"]`):** `session.media.gifs.search(query)` returns GIF ids, descriptions and sizes, and `session.media.gifs.load(id)` returns a `blob:` URL ready for an `<img>`. The frame never contacts GIPHY: Stellarr's page asks it under the instance's GIPHY switch and each player's own GIPHY setting. Put ids in shared state and let every client load them. Rejections carry `error.code`, such as `declined` when the player turned GIPHY off. Stellarr shows GIPHY's mark in the activity bar.

  ```ts
  const { items } = await session.media.gifs.search("high five");
  const gif = await session.media.gifs.load(items[0].id);
  image.src = gif.url; // gif.release() when it is no longer shown
  ```

In `stellarr dev`, pick any local audio or video file in the simulator header bar to test consented mic levels, camera tiles, mutes, and denials without physical hardware. `media.gifs` answers there from an offline set of generated images, and the header's **GIFs** menu turns GIFs off for the player or the instance.

---

## CLI workflow: preview and publish

```sh
# 1. Local simulator (--mode voice | personal, --port 0 for a random free port)
npx stellarr dev --dir dist

# 2. Pair with a live development Stellarr instance (Cockpit -> Integrations -> Connect project)
npx stellarr dev --dir dist --instance http://localhost:3000
npx stellarr dev --dir dist --instance http://localhost:3000 --stop <tester-invite>

# 3. Validate artwork/manifest and build deterministic release archives
npx stellarr validate --dir dist
npx stellarr pack --dir dist --out release
```

`stellarr pack` verifies `icon` (`≤ 512 KiB` PNG/JPEG/WebP), bundles static assets (`≤ 256 MiB`, `≤ 8,192` files), and writes `<id>-<sha256>.zip` plus `stellarr-release.json` into `release/`. Attach both files to a GitHub Release tagged `<id>/v<version>` (for example, `quick-buzz/v0.1.0`), then paste your repository URL into **Cockpit → Activities**. Installed activities run on the instance's existing HTTPS address out of the box (using package-relative asset URLs); `STELLARR_ACTIVITY_DOMAIN` is optional.

---

## Further reading

- [Getting started tutorial](../docs/sdk/getting-started.md) — project setup, local simulator toolbar, and live voice-channel previews.
- [Rooms, state & saves](../docs/sdk/rooms-and-state.md) — deep dive into concurrency, host failover, seats, secret assignments, and saves.
- [Media, manifest & publishing](../docs/sdk/media-and-publishing.md) — `stellarr.json` schema, CSP `connects`, media permissions, and multi-activity repos.
- [API & CLI reference](../docs/sdk/api-reference.md) — complete TypeScript types, error status codes, allowed MIME extensions, and runtime limits.
- [Activity contract](CONTRACT.md) — security model, isolation guarantees, and open platform design items.
