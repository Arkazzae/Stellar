# Media, manifest, and publishing

Activities run inside a sandboxed origin with direct `getUserMedia` and `getDisplayMedia` disabled for privacy. Instead, Stellarr provides a built-in **Game audio** mixer plus two consented media capabilities—microphone voice levels and host-rendered camera tiles—and GIF search through the instance's GIPHY integration, alongside deterministic release packaging for **Cockpit → Activities**.

The sandbox does allow ordinary browser downloads. To save a file such as a replay GIF, turn it into a `Blob`, give it an object URL, and click an `<a download>` link. No SDK call or manifest permission is involved:

```ts
const link = document.createElement("a");
link.href = URL.createObjectURL(gifBlob);
link.download = "replay.gif";
link.click();
```

---

## 1. Respecting the Game audio slider (`session.media.volume`)

Every running activity has a **Game audio** control: a mute button beside the activity's name in the call bar, and a slider (`0–100%`) in its settings next to the call volume, so each listener can turn down background music or sound effects locally without muting voice chat or affecting anyone else.

No manifest permission is needed—wire `session.media.volume` (`0..1` number) to your Web Audio master `GainNode` or HTML `<audio>` / `<video>` elements at startup:

```ts
const audioCtx = new AudioContext();
const masterGain = audioCtx.createGain();
masterGain.connect(audioCtx.destination);

// Apply initial slider position and react to live changes:
masterGain.gain.value = session.media.volume.get();
session.media.volume.subscribe((volume) => {
  masterGain.gain.setTargetAtTime(volume, audioCtx.currentTime, 0.015);
});
```

---

## 2. Consented microphone and camera inputs

Declare requested device capabilities in `stellarr.json` under `"permissions"`:

```json
{
  "permissions": ["media.microphoneLevel", "media.cameraTile"]
}
```

Both capabilities follow a **two-tier consent model**:

1. **Instance owner review:** Cockpit displays requested permissions before installing or updating the activity.
2. **Per-session player consent:** Calling `.open()` prompts that specific player (even if their mic or camera is already on in the voice call) and adds a **Stop** button beside their call controls.

### Voice-reactive games (`media.microphoneLevel`)

Streams a normalized `0..1` speech level from the consenting player's microphone without exposing raw audio samples to game scripts:

```ts
try {
  const mic = await session.media.microphoneLevel.open();

  const unsubscribe = mic.subscribe((level) => {
    // level is 0..1 (automatically 0 while the player mutes their call mic)
    meterBar.style.transform = `scaleX(${level})`;
    if (level > 0.65) triggerJump();
  });

  mic.onEnded((reason) => {
    unsubscribe();
    // Player clicked Stop, muted/unplugged device, or closed session:
    enableSpacebarFallback(reason);
  });
} catch {
  // Player declined the consent prompt — fall back to keyboard/touch controls:
  enableSpacebarFallback("declined");
}
```

### Performer camera overlays (`media.cameraTile`)

Calling `open()` asks the player for permission to share their camera with this activity, including when their call camera is already on. In installed activities, Stellarr sends video over WebRTC through a dedicated media room. Only players who have joined the activity can receive it. A player can stop sharing from the activity bar or by leaving the game. The game positions the tile and Stellarr plays the video above the iframe. Camera tracks and pixels stay in the parent page.

```ts
// 1. On the active performer's client, ask permission to share their camera tile:
const camHandle = await session.media.cameraTile.open();
camHandle.onEnded(() => notifyTurnCameraStopped());

// 2. On EVERY viewer's client, mount the performer's tile over a placeholder element:
const stageSlot = document.getElementById("performer-slot")!;
const unmountTile = session.media.cameraTile.mount(currentPerformerId, stageSlot);

// Hide the tile while the viewer's pause menu is open.
unmountTile.setVisible(false);
// Restore it when the menu closes.
unmountTile.setVisible(true);
// Call unmountTile() when the round ends or the performer changes.
```

`mount()` tracks `stageSlot.getBoundingClientRect()` while the tile is visible and reads the current bounds when it is shown again.

Hiding the tile affects this viewer only. The performer can stop sharing at any time using the activity bar.

### External players and embeds (`session.media.embed`)

Declare the provider's hostname in `stellarr.json` under `connects`, then pass its HTTPS **embed URL** and a sized placeholder to `mount()`. The provider must support framing on your instance. Use the provider's embed URL format and include any parameters it requires. A normal share link may lead to a page that refuses framing.

```ts
const player = session.media.embed.mount(embedUrl, playerSlot, {
  title: "Watch together",
  onMessage(data) { handlePlayerEvent(data); },
});

function showPauseMenu(open: boolean) {
  player.setVisible(!open);
  pauseMenu.hidden = !open;
}
// Call player.destroy() when the viewer leaves this screen.
```

Showing the player again keeps its iframe and playback position. Playback and audio can continue while it is hidden. To control playback, use the provider's own message format with `player.postMessage(data)`. The optional `onLoad()` callback signals that the iframe loaded. Providers with a separate ready event deliver it through `onMessage(data)`.

On the default instance hostname, Stellarr draws the embed above the activity and limits the frame to hosts approved for that installation. In local previews and activities with a dedicated origin, the iframe lives inside the placeholder.

### Testing media without physical devices

The local simulator and private development previews use an image relay to simulate camera tiles. Install the activity on a test instance to exercise WebRTC and call coexistence.

In `npx stellarr dev`, declaring `media.microphoneLevel` or `media.cameraTile` adds simulated device controls to the simulator header:

1. Select any local audio (`.wav`, `.mp3`, `.ogg`) or video (`.mp4`, `.webm`) file in Tab 1.
2. Trigger `.open()` inside your game and click **Allow** in the Stellarr prompt.
3. Open **New player** in Tab 2 to watch the live camera tile or resulting gameplay, and test **Deny**, **Mute**, and **Stop** transitions deterministically.

---

## 3. GIF search (`media.gifs`)

A party game can let each player search GIFs for a prompt and show everyone's picks. Declare the permission in `stellarr.json`:

```json
{
  "permissions": ["media.gifs"]
}
```

The activity's frame never contacts GIPHY. The Stellarr page around it searches GIPHY and fetches each GIF itself, under the rules chat follows: the instance's GIPHY switch and key in **Cockpit → Integrations**, and each player's own GIPHY setting under **Settings → Privacy**. The frame receives GIF ids, descriptions, sizes and image bytes, never a GIPHY address, the API key or GIPHY's analytics. Nothing passes through or stays on the Stellarr server. Searches use the player's interface language and GIPHY's `pg-13` rating; an empty query lists trending GIFs.

Put GIF **ids** in shared state and let every client load each id itself:

```ts
import type { GifError } from "@stellarr/sdk";

// The player searches, sees the results through showPick(item.id, …), and chooses one.
const { items } = await session.media.gifs.search(prompt.value, { limit: 12 });
async function choose(id: string) {
  await session.state.set({ ["pick-" + session.context.me.id]: id });
}

// Every client shows every pick from the shared state.
async function showPick(id: string, slot: HTMLElement) {
  try {
    const gif = await session.media.gifs.load(id); // "preview" by default, or "full"
    const image = new Image(gif.width, gif.height);
    image.src = gif.url; // a blob: URL made inside the frame
    image.alt = gif.alt ?? "GIF";
    slot.replaceChildren(image);
    return gif; // call gif.release() when the round ends
  } catch (error) {
    slot.textContent = (error as GifError).code === "declined"
      ? "GIFs are off in your privacy settings."
      : "This GIF could not be shown.";
  }
}
```

`load()` resolves to `{ url, type, width, height, alt?, release() }`. The `url` works in an `<img>` at once. `release()` revokes it, and closing the session revokes the rest. `"preview"` is GIPHY's small rendition, about 200 pixels wide; `"full"` is up to about 480 pixels and larger. Keep the image you loaded while you show it instead of loading the id again.

Every rejection carries `error.code`, so the game can choose what each player sees:

| `error.code` | Meaning | What to show |
| --- | --- | --- |
| `permission` | The manifest lacks `media.gifs`. | Nothing: fix the manifest. |
| `unavailable` | The instance offers no GIFs, or the activity runs outside Stellarr. | Hide the GIF parts, for example let the player type a caption. |
| `declined` | This player turned GIPHY off. Other players' picks stay hidden for them too. | Say that GIFs are off in their privacy settings, and show other picks as placeholders. |
| `rateLimited` | This frame, or the instance's shared GIPHY key, asked too often. | Wait, then try again. |
| `notFound` | GIPHY has no such GIF any more. | Skip the pick. |
| `failed` | GIPHY could not be reached, sent an unusable image, or did not answer in time. | Offer to try again. |

Each frame may search `30` times and load `120` GIFs per minute, with `4` loads in flight, because the instance's one GIPHY key also serves chat. Wait for a pause in typing before searching. The [contract](../../sdk/CONTRACT.md#4-packaging-cockpit-lifecycle-and-limits) lists every limit.

Once the activity has searched or shown a GIF, Stellarr puts GIPHY's mark in the activity bar. GIPHY requires this attribution, so the activity does not need to draw its own. The activity picker tells players before launch that the activity searches GIFs on GIPHY.

In `npx stellarr dev`, `media.gifs` answers from an offline set of generated SVG images labelled with the query, without a network or a key (`type` is `image/svg+xml` there; GIPHY sends `image/gif` or `image/webp`). Picks load in every **New player** tab. The **GIFs** menu in the header switches the tab between **On**, **Off for this player** (`declined`) and **Off on the instance** (`unavailable`).

---

## 4. Complete `stellarr.json` manifest guide

Place `stellarr.json` beside `index.html` at the root of your published folder (`public/stellarr.json` in Vite):

```json
{
  "protocol": 1,
  "id": "synctube",
  "name": "SyncTube",
  "description": "Watch videos together in frame-accurate sync.",
  "version": "1.2.0",
  "icon": "icon.webp",
  "cover": "cover.webp",
  "devices": ["desktop", "tablet", "phone"],
  "launchModes": ["voice"],
  "authority": "host",
  "seats": 25,
  "permissions": [],
  "connects": [
    {
      "host": "www.youtube-nocookie.com",
      "why": "Embeds the privacy-enhanced YouTube video player."
    }
  ]
}
```

### Declaring external origins (`connects`)

By default, Stellarr serves every activity with a strict `Content-Security-Policy` blocking third-party scripts, styles, fonts, analytics, and `fetch`/`WebSocket` calls (`default-src 'self'`).

- Bundle your fonts, icons, WASM, and libraries inside your static build whenever possible so participants leak zero IP traffic outside their Stellarr server.
- When an activity genuinely requires an external service (such as an embedded player or public API), list each lowercase hostname in `connects` (up to `64` entries) with a clear `why` explanation (`≤ 1024` bytes). Cockpit shows every host and reason to the instance owner during installation and whitelists `https://<host>` upon approval.

---

## 5. Packaging and publishing releases

### Validate and build deterministic archives

Before cutting a release, verify your manifest, artwork magic bytes (`icon ≤ 512 KiB`, optional `cover ≤ 2 MiB`), and file limits (`≤ 256 MiB`, `≤ 8,192` files):

```sh
npx stellarr validate --dir dist
npx stellarr pack --dir dist --out release
```

`stellarr pack` normalizes timestamps and file modes so identical builds always produce the exact same SHA-256 hash, writing two artifacts into `release/`:

- `synctube-<16-char-sha256>.zip` — deterministic archive with `index.html` and `stellarr.json` at its root.
- `stellarr-release.json` — release index read by Cockpit (`protocol`, `id`, `name`, `version`, `asset`, `sha256`, `bytes`).

*(Dotfiles, `node_modules`, `_stellarr`, `package.json`, and `package-lock.json` are excluded automatically.)*

### Publish on GitHub Releases (single or monorepo)

One GitHub repository can host either a single activity or dozens of independent activities side by side:

1. Tag the release as `<id>/v<version>` matching `id` and `version` in `stellarr.json` (for example, `synctube/v1.2.0`).
2. Attach the generated `.zip` file and `stellarr-release.json` to that GitHub Release (see the [reference GitHub Actions workflow](https://github.com/Arkazzae/StellarrActivities/blob/main/.github/workflows/release.yml)).

### Install and update in Stellarr Cockpit

1. Open **Cockpit → Activities → Add activity** and paste your GitHub repository URL (or a direct release URL). Installed packages run on the instance's existing HTTPS address out of the box (always use package-relative asset paths and `/_stellarr/sdk/v1.js`); configure `STELLARR_ACTIVITY_DOMAIN` only if an activity requires origin-bound browser storage ([Configuration: Address and TLS](../install/configuration.md#address-and-tls)).
2. Select the activity from the picker, review its size, permissions, and `connects` hosts, and click **Install and enable**.

When you publish a higher SemVer tag later, Cockpit offers **Update**, highlights any changed permissions or external hosts before approval, and lets any in-progress sessions finish undisturbed on their original release origin.
