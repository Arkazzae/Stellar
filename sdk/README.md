# Stellarr activity SDK

An activity is a static web app. Stellarr hosts the page, identifies the people in the room and carries state and commands between them. The browser client, TypeScript declarations and development CLI live in this directory of the Stellarr monorepo. The SDK requires Node.js 22 or later for development. A deployed activity runs in the browser.

## Try an activity locally

From the Stellarr checkout, point the CLI at your activity’s published folder. It must contain `stellarr.json` and `index.html` at its root.

```sh
node sdk/cli.js dev --dir /path/to/activity
```

Open the printed address. In voice mode, choose **New player** to join the same room from another tab. Each tab gets its own identity, which survives a reload. To test an activity that supports both launch modes, add `--mode personal` for a private room per tab or `--mode voice` for a shared room. The CLI defaults to voice when the manifest supports it, otherwise to personal. The game runs in its own origin inside a Stellarr frame. Changes to game files appear on refresh. Restart the command after editing the manifest. Use `--port 0` for an available port.

Install the SDK in your activity project, then validate and preview its published directory.

```sh
npm install --save-dev @stellarr/sdk
npx stellarr validate --dir dist
npx stellarr dev --dir dist
```

For plain HTML, import `connect` from `/_stellarr/sdk/v1.js`. Bundlers can import it from `@stellarr/sdk`. The [two-file poll in the contract](CONTRACT.md#minimal-activity-without-a-build-step) is a complete starting point.

## Publish from one repository

For each release, provide `id`, `protocol`, `version`, `name`, `description` and `icon` in `stellarr.json`. Point `icon` to a PNG, JPEG or WebP in the activity folder, up to 512 KiB. Optionally add `cover` pointing to an image up to 2 MiB. Without a cover, Cockpit places the icon on the current theme’s background. You can omit both while developing locally or testing privately in a voice channel.

Give each activity its own manifest version, changelog and GitHub release. For example, when SyncTube is ready for its first release, set its manifest version to 1.0.0 and add a `## 1.0.0` entry to its `CHANGELOG.md`. Validate and package just its published directory.

```sh
npx stellarr validate --dir activities/synctube/dist
npx stellarr pack --dir activities/synctube/dist --out release/synctube
```

Use the tag `synctube/v1.0.0`, the release title **SyncTube 1.0.0** and that version’s changelog entry. Attach the generated `stellarr-release.json` and the ZIP it names. Each sibling activity keeps its own tags and assets. The public example automates these steps when the author pushes a scoped tag. Packaging uses the system’s `zip` program on macOS/Linux and PowerShell on Windows. [Public release workflow](https://github.com/Arkazzae/StellarrActivities/blob/main/.github/workflows/release.yml)

The instance owner pastes the repository URL in **Cockpit → Activities**. Cockpit finds a stable release for each published activity and opens a picker when several are available. A direct release URL selects that particular release. Older releases with a shared multi-activity index remain supported. Selecting one shows that activity’s metadata, download size, device permissions and external hosts. **Install and enable** adds just that activity. The owner can use the same URL again for another activity. Updates, enable/disable and uninstall apply to the selected library entry. Installed packages run inside the container using the existing Stellarr HTTPS address. Use package-relative asset URLs and import the SDK from `/_stellarr/sdk/v1.js`. A dedicated wildcard domain is optional. [Instance configuration](../docs/install/configuration.md)

## Choose where people can launch

The optional `launchModes` field accepts `["voice"]`, `["personal"]` or `["voice", "personal"]`. Omitting it enables voice only. For example, enable both for a video queue and choose only voice for a guessing game that needs other people.

A signed-in user opens **Friends → Activities** and chooses **Launch for myself**. Personal state, commands, media grants and the queue belong to that user’s session. A voice call is optional. Read `session.context.launchMode` to adapt the UI. A personal session on the instance has `place: null` and `standalone: false`. Local previews set `standalone: true`.

## Use the room

```js
import { connect } from "/_stellarr/sdk/v1.js";

const session = await connect({ initialState: { count: 0 } });
session.state.subscribe(state => { score.textContent = String(state.count); });
add.onclick = () => session.state.set(state => ({ count: state.count + 1 }));
```

`context.me.id` identifies the current participant. `participants` and `seats` track membership and who can play. By default, participants can patch shared state. Set `"authority": "host"` in the manifest for a game that accepts moves through a host. Each eligible client registers the same command handlers so a successor can take over. Return a patch and optional result from the handler. A stable command ID lets a participant retry a move after reconnecting.

```js
session.commands.handle("score", (_data, { from }, state) => ({
  patch: { lastScorer: from, count: state.count + 1 },
  result: "accepted",
}));
await session.commands.submit("score", null, { id: turnId });
```

Set a seat count, such as `"seats": 2`, to allow spectators. A host command can atomically return `assignments: { [playerId]: secret }`. Each seated participant reads their own assignment. The current host can read the private checkpoint to resume a round. Keep secret prompts out of public state and messages.

Use `session.now()` for a shared playback clock. Use `send(kind, data, { to, latest })` for a transient event. With `latest: true`, queued events of the same kind can replace earlier ones. Check the return value to detect a locally rejected send. Subscribe to `connection` to display reconnecting or closed states. [Browser API declarations](client.d.ts)

## Adapt the game to its players

Declare `"devices": ["phone", "tablet", "desktop"]` in `stellarr.json`, or omit it when the game supports all three. The launcher and game use the same classification. A coarse primary pointer and a shorter screen dimension of at least 600 CSS pixels identify a tablet. Smaller touchscreens are phones. A fine primary pointer identifies a desktop. Use responsive CSS for the available frame size and provide touch and keyboard controls as appropriate.

Read `session.context.locale` for the active Stellarr interface language. A local preview uses the browser language, which may include a region such as `pl-PL`. Subscriptions run immediately and when the setting changes, including during a round. The game supplies its own translations.

```js
session.locale.subscribe(locale => {
  document.documentElement.lang = locale;
  renderLabels(locale);
});
session.device.subscribe(device => {
  document.documentElement.dataset.device = device;
});
```

The Stellarr frame displays the participants. Inside a scoreboard, subscribe to `session.participants` and match scores to `player.id`. Each entry includes the display name, optional account username, current avatar URL or `null`, and connection status. The roster and `session.context.me` reflect profile edits during play. Show the name or initials when an image is unavailable.

## Save a player's progress

`session.progress.load()` reads the signed-in player's private save on this instance. A first visit returns `{ revision: 0, data: null }`. Call `save(data, expectedRevision)` at a checkpoint with a complete JSON object of up to 64 KiB. Its result is the committed revision. This save follows the account across devices, voice and personal sessions, server restarts, updates and reinstalls from the same source. Removing the account deletes its saves.

```js
async function finishLevel(level) {
  for (;;) {
    const { data, revision } = await session.progress.load();
    if ((data?.highestLevel ?? 0) >= level) return;
    try {
      await session.progress.save({ ...data, highestLevel: level }, revision);
      return;
    } catch (error) {
      if (error.status !== 409) throw error;
    }
  }
}
```

A conflicting change returns HTTP 409. Read the new revision and merge according to the game's rules. Retrying an acknowledged revision with the same data succeeds while that revision is current. The SDK retries interrupted requests with the original revision, so an old save cannot overwrite newer progress. Bundled games supporting older hosts can check `session.capabilities.includes("progress")`.

The CLI stores test saves under `~/.stellarr-preview/progress` for the project, port and simulated player. Reload the same tab to resume. A paired development preview keeps separate tester saves across uploads for the lifetime of the pairing.

## Test camera and microphone input

Declare `media.microphoneLevel` or `media.cameraTile` in `permissions`. A game calls `session.media.microphoneLevel.open()` or `session.media.cameraTile.open()`. Stellarr asks that player for consent. Handle a rejected request and use the handle’s `onEnded` to restore touch or keyboard controls.

The local toolbar accepts an audio or video file for the current tab. Select a file, request the input from the game, then allow it in the Stellarr dialog. Audio loops and produces levels from 0 to 1. For camera input, mount a tile with `session.media.cameraTile.mount(playerId, element)`. The parent renders the image over that element. Open a second tab to see it. **Stop**, microphone mute and a denied request can all be tested without a physical device. Preview camera tiles use low-resolution JPEG frames. [Media API declarations](media.d.ts)

Apply `session.media.volume.get()` to each player or audio graph at startup and subscribe for subsequent changes. Each participant’s **Game audio** slider controls their own playback.

For a YouTube activity, declare `www.youtube-nocookie.com` in the manifest’s `connects`. On the instance hostname, `session.media.youtube.mount(videoId, element, events)` displays one player inside the activity viewport. Call `prepare(positionInSeconds, playing)` from the ready handler. The handle provides play, pause, seek, volume (0–1), fullscreen and destroy. Dedicated-host packages and local previews can use the YouTube IFrame API directly when this handle is unavailable. [API declarations](media.d.ts)


## Test with someone in a voice channel

On a test instance, the operator enables `STELLARR_ACTIVITY_DEV_ENABLED=true` for the Vite dev web server and starts the Development API. In Cockpit, the owner opens **Integrations → Connect project for testing** and gives the author the instance address and pairing code. The code expires after five minutes or one successful redemption.

```sh
npx stellarr dev --dir dist --instance http://localhost:3000
```

Enter the pairing code at the prompt. The CLI stores the author credential in a private file under `~/.stellarr-preview` for that instance and project directory. The pairing expires within 24 hours or when its draft stops or expires. Run the same command to upload the next build of the activity, or add `--pair` to reconnect using a fresh code. With `--pair`, automation can supply the code on one line of standard input.

Share the separate tester invitation from the CLI. Both testers sign in, join the same voice channel, open **Activity preview** and enter that invitation. The first successful launch binds the preview to the channel. The instance checks the current voice seat on each room request. Each upload replaces the previous snapshot, creates a new tester invitation and lasts up to two hours. Stopping a draft ends the preview for everyone and revokes the author credential. To stop it early, run from the same project directory:

```sh
npx stellarr dev --dir dist --instance http://localhost:3000 --stop <tester-invite>
```

A preview accepts up to 16 MiB and 1024 browser files. Pass a directory containing only assets intended for participants. For testers on different computers, configure `STELLARR_ACTIVITY_DEV_ORIGIN` as the test instance’s HTTPS origin and `STELLARR_ACTIVITY_DEV_DOMAIN` as a dedicated wildcard hostname for activity frames. Route both to the dev web server and include the instance hostname in `STELLARR_DEV_ALLOWED_HOSTS`. The local defaults are `localhost:3000` and `*.activity.localhost:3000`.

Run the SDK regression suite from the repository root with `npm run test:sdk`. [Activity contract](CONTRACT.md)
