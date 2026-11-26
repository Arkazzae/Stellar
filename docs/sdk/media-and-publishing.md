# Media, manifest, and publishing

Activities run inside a sandboxed origin with direct `getUserMedia` and `getDisplayMedia` disabled for privacy. Instead, Stellarr provides a built-in **Game audio** mixer plus two consented media capabilities—microphone voice levels and host-rendered camera tiles—alongside deterministic release packaging for **Cockpit → Activities**.

---

## 1. Respecting the Game audio slider (`session.media.volume`)

Every time an activity opens inside a call, Stellarr shows a **Game audio** slider (`0–100%`) right next to the user's call volume so each listener can turn down background music or sound effects locally without muting voice chat or affecting anyone else.

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

## 3. Complete `stellarr.json` manifest guide

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
  "launchModes": ["voice", "personal"],
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

## 4. Packaging and publishing releases

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
