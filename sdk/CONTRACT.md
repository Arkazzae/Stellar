# Activity contract

> [!NOTE]
> Activity contract, 2026-11-26. The [SDK guide](README.md) describes the current development tools. This document also records proposed features and open decisions.

People open activities such as watch parties, board games and whiteboards in Stellarr. Authors can offer a shared voice session, a personal session or both. This contract describes what an activity can expect from the SDK and the instance.

Every part carries one of three marks:

| Mark | Meaning |
| --- | --- |
| **Decided** | The owner of the project said so. |
| **Proposed** | Suggested during the design talks and not yet confirmed. |
| **Open** | Asked and not answered, or never discussed. |

## Terms

| Term | Meaning |
| --- | --- |
| Instance | One installation of Stellarr: one container, one data volume. |
| Instance owner | The person who runs the instance and uses Cockpit. |
| Server | A community inside an instance, with its channels and members. |
| Activity | A web app that a person opens privately or shares in a voice channel. |
| Author | Whoever writes an activity. |
| Session | One running activity, its participants and the state they share. |
| Participant | A person in a session. |
| Host | An optional role held by one participant who decides the outcome of commands for a session. |
| Checkpoint | A saved revision from which a session can resume. It can include data reserved for the current host. |
| Library | The activities an instance owner has added to the instance. |

## 1. What an activity is

*Decided.*

- An activity is a static web page: HTML, scripts and assets. It has no server code of its own.
- Stellarr on the instance is the activity's backend. Whatever an activity needs from a server, it gets from this contract.
- The instance serves the activity and its packaged assets. Keeping participants' network traffic on the instance is the privacy goal. Section 4 describes its limits and the proposed exception for external services.
- The instance owner installs and manages an activity for the whole instance in Cockpit. An eligible participant opens an enabled activity directly from its entry. That single action starts or joins the session with the approved permissions.
- SyncTube is developed in a separate public repository and uses the same contract as other activities. [Stellarr Activities](https://github.com/Arkazzae/StellarrActivities)
- The SDK is an installable package. The owner's activities already import it as `@stellarr/sdk`.
- Stellarr builds the SDK from scratch, using standard platform APIs and its own code wherever practical to avoid third-party dependencies. Existing activities and their tests serve as references for the behavior it needs to support.

## 2. Who talks to whom

*Proposed.*

```text
activity page  <->  Stellarr page around it  <->  instance
```

The surrounding Stellarr page binds the activity frame to the signed-in participant and session. It keeps the credentials and opens the connections to the instance. The activity receives a session identifier and uses the SDK.

An activity chooses between a confirmed change and a short-lived message. Stellarr chooses the transport. For activities that stream live updates, a large checkpoint must not sit ahead of a fresh update in the same transport queue. Both can still compete for bandwidth. Section 14 describes the proposed measurement.

## 3. What Stellarr gives an activity

*Proposed, including every name below.*

The proposed v1 core is a session with participants, shared state and messages. A session can optionally assign a host. Section 12 records additional needs for particular activities.

### Core operations

| Operation | What an author can rely on |
| --- | --- |
| `connect({ initialState })` | Resolves to the session API (including `context` and `state`) after loading the current state and revision. Concurrent first joins initialize the state once. |
| `context`, `theme`, `participants` | Context identifies the session, person, place, locale and standalone mode for the life of the page. Theme exposes light or dark mode and six palette colours. Theme and participant changes can be observed. Everyone sees participants in the same join order. |
| `state.get()`, `state.subscribe(listener)` | `get()` returns the last confirmed state object. A subscription calls `listener(state, revision)` immediately and then in revision order. After a gap the listener receives a fresh snapshot before later changes. |
| `state.set(patch)` | Replaces the specified top-level keys atomically and resolves with the committed revision. |
| `state.set(change)` | Computes the patch from the current revision and retries on conflict. The function must be free of side effects. Returning nothing cancels the change. Throwing leaves state unchanged. |
| `send(kind, data, options)`, `on(kind, listener)` | Send and receive a short-lived message. The instance identifies the sender. Sending returns false on immediate refusal. Success means local acceptance. |
| `now()`, `limits` | Read the estimated instance time in milliseconds and the limits the instance enforces. Clock accuracy remains to be measured. |

### Participants

- A participant is a person, not a tab. The id stays the same after a reload and on another device.
- An id is an opaque text. An activity may compare ids and must not read anything into their shape.
- A participant has an id, a name, an avatar address served by the instance, and a `connected` flag.
- A person has one active frame per session. Opening the activity elsewhere retires the previous frame. Its pending traffic can no longer act for that participant.
- Someone whose page reloads or whose connection drops keeps their place for `graceSeconds`, shown as not connected.

### Optional host and commands

By default every participant can write. Games with spectators can restrict play to seated players as described in section 12. An activity that needs one decision maker can ask the instance to assign a host. In that mode only the current host can write. Other participants submit commands with a stable ID. The host uses the same revisioned write to record a command's outcome and its state change together. The caller receives the committed revision or a refusal. The retry rule below also applies across host transfer.

While a disconnected host holds its place, host operations pause. On transfer, the instance announces the successor and rejects further writes or live updates from the old host. Revoked access takes effect immediately. The host interprets the activity's rules.

### Shared state

- All participants can read the session's JSON object. For unconditional writes to the same key, the later committed value wins. Readers see their own writes after confirmation.
- The SDK reports a refusal when a write exceeds a limit or cannot be submitted. Losing a reply after submission leaves the outcome unknown. A retry of the same operation uses its original ID and can apply the change at most once. An expired session rejects the retry.

### Messages

- A message goes to everyone else in the session, or to one participant named in `options.to`. A message for one participant is not shown to the others.
- The data is anything JSON can hold, or bytes.
- With `options.latest`, Stellarr keeps at most one waiting value per session, sender, kind and recipient. Broadcast delivery applies this rule independently for each recipient. Data already handed to the transport may still arrive. An activity must make each replacement sufficient for its purpose.
- A message can be lost: when the sender is offline or over a limit, when the receiver is not connected, or when a newer one replaced it.
- A sender can reduce its rate when its local queue is full. Ordering across a disconnect is unspecified.
- A cursor or selection uses an ordinary latest-value message. Recipients clear it when its sender disconnects.

## 4. What an activity cannot do

| Limit | Why | Mark |
| --- | --- | --- |
| Act as the signed-in person | The instance protects operations that use the person's credentials (section 7). | Proposed |
| Read another activity's saved data | Each activity is served under its own name, and a browser keeps storage apart by name. | Decided (a name per activity) |
| Load undeclared external resources | The default resource policy restricts the page to its own origin. Its scope is described below. | Proposed mechanism for the decided privacy goal |
| Keep a secret in the shared state | Every participant can read all of it. | Proposed |
| Guarantee that a participant running activity code follows its rules | The instance enforces the session's write permissions. A participant entitled to write can still submit invalid activity data. | Proposed |
| Run code on the instance | See section 1. | Decided |

### The rule against outside connections

*Proposed mechanism for a decided goal.*

- By default, every file served under an activity's name restricts resource loads and connections to that origin.
- The activity declares each external host it needs and why. The owner sees those hosts alongside the requested permissions in Cockpit. That approval covers the activity's use of the declared services, including players and previews, for everyone who launches it.
- The rule stops accidents: fonts, analytics and adverts loaded from elsewhere.
- The rule does not stop intent. A page can still send its own frame to an outside address, and it can still open a WebRTC connection (section 14). An imported activity is therefore trusted code as far as people's network addresses go, which is why adding one is the owner's decision.

## 5. The frame

*Proposed.*

The activity runs in a frame that has its own name on the instance. A frame without a name of its own was ruled out: a YouTube player did not start in one (section 14), and such a frame has no browser storage to keep anything in, which was not tested here.

| The frame is meant to allow | Activities in the owner's registry that need it |
| --- | --- |
| Scripts | all 27 |
| Its own browser storage | 25 |
| Sound and video that start without a click inside the frame | SyncTube |
| Pointer lock | 5 |
| Full screen | 5 |
| Clipboard | 8 |
| Gamepad | 2 |

The frame is meant to refuse new windows, navigation of the surrounding Stellarr page, and direct capture of the camera, microphone or screen. Stellarr provides the game inputs below through its own controls.

Only scripts, own storage and unprompted playback were tested (section 14). `minecraft-wasm` also asks for cross-origin isolation of the whole page, which this contract does not cover.

### Microphone and camera in games

**Decided for the first release.** A game can request two independent capabilities through its manifest. The permission names below are proposed.

| Permission | What the activity receives | Example |
| --- | --- | --- |
| `media.microphoneLevel` | A local level from 0 to 1 for the consenting player's microphone. The game chooses which gameplay event to send. | A loud sound makes the character jump. Other players receive the jump. |
| `media.cameraTile` | A place for Stellarr to display a consenting participant's camera to people in the activity. Stellarr renders the tile outside the activity's readable content. The game receives the participant ID and the tile's availability. | In charades, players and spectators see the performer. Only Stellarr handles the video stream. |

Cockpit approval lets an installed game ask. Before the first reading or tile appears, Stellarr asks the participant whether this game may use that input for this session. For a camera tile, the prompt identifies who can watch. This choice applies even when the participant is already using the device in the call. A private development preview follows the same consent rules.

Stellarr shows which game is using the device and offers **Stop** alongside the call controls. Muting the call microphone suspends level readings, and switching the camera off clears its tile for all viewers. Refusal or device loss tells the game that input is unavailable, so it can offer another control or let the next player take the turn. **Stop**, closing the activity, or losing session access releases the game's use of the device. An ongoing voice call keeps its own device settings.

*Proposed for later releases.* Pitch detection for karaoke, access to raw microphone samples, and access to camera frames for motion tracking are separate capabilities. A raw-input prompt must say that the game's code can read and copy the audio or images it receives. Recording a performance and sharing that recording each require a separate participant choice.

### Game audio volume

**Decided.** Stellarr shows a **Game audio** slider (**Dźwięk gry** in Polish) beside the call volume control, in the same place for every activity. Each listener can adjust the activity's music, effects and embedded media from 0% to 100%. At 0% the activity is silent for that listener while shared playback continues. Call volume, microphone input and other participants' listening levels are independent.

The SDK supplies the listener's current level when the activity opens and reports changes during play. The activity applies that level to its audio output, including sounds started after the slider moves. Its own music and effects settings retain their relative levels. The public SDK names are open.

## 6. Description file

*Proposed.*

**Decided.** The default entry point is `index.html` beside `stellarr.json` at the root of the published activity. The instance serves the browser SDK from the activity's own origin. A plain HTML activity can use it directly.

| Field | Meaning |
| --- | --- |
| `protocol` | The version of this contract the activity speaks. `1` today. |
| `id` | Lower-case letters, digits and dashes. Identifies the activity within its source. The instance assigns its serving origin at installation. |
| `name`, `description` | What people see in the list of activities. |
| `version` | The author's release version in SemVer 2.0.0 format, independent of `protocol`. |
| `icon` | Required for publishing, installing or updating a release. Relative path to a packaged PNG, JPEG or WebP image, up to 512 KiB. A local or private prototype can omit it. |
| `cover` | Optional background illustration in the same formats, up to 2 MiB. If omitted, Cockpit displays the activity icon on the current theme’s background. |
| `devices` | Optional list of supported device classes. Proposed values are `desktop`, `tablet` and `phone`. Omitting the field declares support for all three. |
| `launchModes` | Optional list containing `voice`, `personal`, or both. Defaults to `["voice"]`. The instance only offers modes declared by the installed release. |
| `authority` | Optional. `shared` allows shared-state patches. `host` gives the current host authority to commit game decisions. |
| `seats` | Optional limit on players, from 1 to 128. Additional participants can spectate. |
| `permissions` | Additional Stellarr permissions the activity requests. Cockpit describes each one in plain language. Device requests also require the participant's consent under section 5. Defaults to an empty list. |
| `connects` | External hosts the page uses, each with a reason the owner can review. Defaults to an empty list. |

### Minimal activity without a build step

These two files form a playable local poll. Each participant has one vote and can change it. The first file is `stellarr.json`.

```json
{
  "protocol": 1,
  "id": "shared-poll",
  "name": "Shared Poll",
  "description": "Vote together in a session.",
  "version": "1.0.0"
}
```

The second file is `index.html`. The proposed module address `/_stellarr/sdk/v1.js` is reserved for the instance. It exports the same `connect` as `@stellarr/sdk`. The activity's script policy permits the inline module from the approved page.

```html
<!doctype html>
<html lang="en">
<meta charset="utf-8">
<title>Shared Poll</title>
<form>
  <fieldset disabled>
    <legend>Play another round?</legend>
    <button value="yes">Yes</button> <button value="no">No</button>
  </fieldset>
</form>
<p role="status">Connecting…</p>
<script type="module">
  import { connect } from "/_stellarr/sdk/v1.js";

  const form = document.querySelector("form");
  const status = document.querySelector('[role="status"]');
  try {
    const { context, state } = await connect({ initialState: {} });
    state.subscribe(votes => {
      const choices = Object.values(votes);
      const count = choice => choices.filter(vote => vote === choice).length;
      status.textContent = `Yes: ${count("yes")} · No: ${count("no")}`;
    });
    form.onsubmit = async event => {
      event.preventDefault();
      try {
        await state.set({ ["vote-" + context.me.id]: event.submitter.value });
      } catch {
        status.textContent = "Could not confirm your vote. Try again.";
      }
    };
    form.querySelector("fieldset").disabled = false;
  } catch {
    status.textContent = "Could not join the poll.";
  }
</script>
</html>
```

For an activity that uses an outside service, each entry in `connects` has a `host` and a `why` field explaining the connection. The activity defines its state and messages in its own code.

## 7. Getting an activity onto an instance

### Publish a release

**Decided.** The author creates a repository, tests the activity and publishes a release containing the ready-to-serve files from section 6. The two-file poll runs in preview as written. To publish it, add an icon file and its relative path to the manifest. For an activity with a build step, the author publishes the build output. The instance downloads those files. An author who runs their own instance can then install the release in their own Cockpit. On another instance, its owner uses the same link.

**Decided. One repository can publish several activities independently.** Each activity has a directory, manifest ID, SemVer and changelog. A manual tag of the form `<activity-id>/v<manifest-version>` releases that activity. For example, `synctube/v1.0.0` publishes **SyncTube 1.0.0** using only SyncTube’s build and changelog entry. Other activities keep their published assets. Branch pushes can run checks. The public example follows this format. [Stellarr Activities](https://github.com/Arkazzae/StellarrActivities)

A scoped release attaches one ZIP and one `stellarr-release.json`. The ZIP contains `index.html` and `stellarr.json` at its root. The index has `protocol: 1` and an `activities` array containing the entry matching the tag. Each entry gives `id`, `name`, `version`, ZIP asset filename (`asset`), archive size in bytes (`bytes`) and lowercase SHA-256 of the ZIP (`sha256`). A repository may also have older releases with unscoped tags and one or more activity entries. Existing release links and installed source references keep working.

### Install from a link

**Decided.** A repository URL is enough to install an activity. Cockpit finds the release and its ready-to-serve files and presents one approval.

1. In the activity library, choose **Add activity** and paste the repository URL. Cockpit looks across published stable releases and lists each activity ID once at the highest stable version with a supported release index. Cockpit validates the selected manifest and its permissions during review. The picker retains each entry’s exact release link. Drafts and prereleases are excluded from automatic discovery. The owner selects an activity to review, or pastes a direct release URL for a particular version.
2. The installer shows the selected activity’s name, description, icon, cover or theme background, version, download size and source together. It lists requested permissions and external hosts with the author's reasons before approval. When installation cannot proceed, the message identifies the problem and the next action. For example, “Release 1.2.0 contains only project sources. Attach the built activity to this release.”
3. Choose **Install and enable**. The instance downloads and verifies the selected files, then marks the activity **Enabled**. Eligible participants can launch it immediately.

### Manage the activity in Cockpit

**Decided.** Approval grants access to the selected activity only. To install another activity from the same repository, the owner opens **Add activity** with the same URL and chooses another ID. The library identifies each installation by its source repository and activity ID. Its grants, enabled status, updates and removal are independent of sibling activities.

The library entry shows the installed version and whether the activity is enabled. The owner can **Disable**, **Enable**, **Update** or **Uninstall** it there. Cockpit checks the saved repository for a published release of the same activity with higher SemVer precedence. It shows both versions and highlights changes to permissions and external hosts before the owner chooses **Update**. A push to a branch or a rebuild at the same version does not offer an update. [SemVer precedence](https://semver.org/spec/v2.0.0.html#spec-item-11)

*Proposed effects on sessions and stored releases.*

| Action | Effect on this instance |
| --- | --- |
| **Disable** | End active sessions and refuse new launches, joins and reconnects. Keep the installed release, source and approved permissions in the library. |
| **Enable** | Make the installed release available again. Its next launch starts a fresh session. |
| **Update** | Verify and store the approved release before switching versions. An enabled activity uses it for new sessions. Existing sessions, including late joins and reconnects, keep their original release and grants. A disabled activity stays disabled after an update. |
| **Uninstall** | End any active sessions, remove the library entry and delete the installed releases and their grants. The owner can later start a fresh installation from the link. |

The proposed installer pins the selected release to its exact source revision and a checksum of the files approved by the owner. If the download or verification fails, Cockpit reports the failure. A first install creates a library entry only after success. A failed update preserves the installed version and its enabled or disabled state.

Each installation has a dedicated origin. A different source claiming the same `id` gets a distinct origin, including after uninstall, so it cannot inherit another activity's browser storage.

**Decided.** Before offering to start or join, Stellarr checks device support for the release the participant would actually open. On an unsupported device a running activity can still appear with a short inline label such as “Available on desktop”. Its page and assets are loaded only on a supported device after launch.

### Names and certificates

- **Decided:** each activity gets its own name. A wildcard domain is acceptable for that.
- **Proposed:** the base of those names is a setting of its own, not necessarily a subdomain of the instance's name.
- The owner supplies a DNS record that covers those names and a certificate that covers them. Stellarr does not obtain certificates: by default it makes a self-signed one for a single name.
- A request from a page on a sibling origin to the instance can carry the instance's session cookie (section 14). Operations acting on that cookie must reject unauthorized callers. Protocol endpoints intended for external callers, such as identity-provider callbacks, retain their own validation.
- **Open:** an instance reached by a bare IP address, or with the default self-signed certificate, cannot give activities their own names. Proposed: such an instance runs only the activities that ship with Stellarr.

### Large assets

- **Decided:** a small fixed cap on size, such as 20 MB, is too low.
- **Proposed:** large assets travel as separate packs, each named by the hash of its bytes. Clients can fetch packs on demand and reuse a verified local copy while it remains cached. An evicted pack can be fetched again.
- **Proposed:** the instance fetches packs itself and skips the ones it already holds.
- **Proposed:** in the browser, a ready part of the SDK keeps packs under the activity's own name and answers ordinary file addresses from them. The activity's code needs no patches and no list of packs.
- **Proposed:** the owner sees the size before approving, and assets stay out of backups because they can be downloaded again.
- **Open:** whether packs belong to the first version. A locally packaged release of `cheetah-strike` (0.7.2) contains 14 packs totaling 1.93 GB. Pack counts and sizes depend on the release.

The earlier Stellarr had this mechanism working. Its pack format and cache behavior provide reference cases for the new implementation.

## 8. Sessions

**Decided.** The author chooses the supported launch modes per activity. In a shared voice session the current channel controls membership. An activity offering `personal` also appears under **Friends → Activities → Launch for myself**. A personal session belongs to the signed-in account and can start outside a call. Another account opening the same activity gets separate state, participants and host privileges. A personal launch and a shared launch by the same person are independent. The private session still runs on the instance, with `context.standalone === false`, `context.launchMode === "personal"` and `context.place === null`. The instance enforces the release’s declared modes on every launch. A participant can play a voice-only game from its voice channel.


**Decided.** A voice channel or call has at most one unexpired session per installed activity. Launch joins that session if it exists, even while it is empty and retained. Otherwise the instance creates it once. Concurrent first launches by eligible participants resolve to the same session ID. A full session reports that it is full. Different activities can run alongside one another in the same place.

*Proposed.*

- A session belongs to a place: a voice channel, a call, or no place at all for someone who starts alone.
- The instance checks membership and access to the place at join and resume, before returning state or replaying a live stream. Expulsion or loss of the underlying permission ends access on every connection, including during the reconnect grace period.
- The last explicit departure or expiry of the last participant's grace period starts `emptySeconds`. A permitted join before expiry resumes the session. Once it expires, a new session gets a fresh identity and state. Session-scoped documents and checkpoints follow the same lifetime.

**Open.** Who may start an activity and whether voluntarily leaving the channel or call also leaves its activity. Documents intended to survive their session need a separate retention policy.

## 9. Developing and testing an activity

### Test locally

**Decided.** Run `npx stellarr dev` in the activity folder and open the printed address in two tabs. In voice mode, each tab represents a distinct test participant in the same room. In personal mode, each tab gets its own room. The local CLI accepts `--mode voice` or `--mode personal` when declared by the activity and defaults to voice when supported. Reloading a tab keeps its participant ID. The SDK reports which features the local room supports and sets `context.standalone` to true.

**Decided.** The development tools let the author select a local audio or video file as a simulated device for one test participant. Another tab can watch the camera tile or observe the resulting move. The author can replay the same input, deny the device request, mute the microphone or disconnect a device during a round. Automated tests can drive these cases without physical devices.

### Pair a project and invite a tester

**Decided.** An author can try the same files with another person on a test instance before publishing a release.

1. The instance owner opens **Connect project for testing** in Cockpit. Cockpit shows the instance address and a one-time pairing code with its expiry time. If someone else wrote the activity, the owner gives these to the author.
2. The author starts the SDK's instance preview command from the project folder and enters the code at the CLI prompt. The instance consumes it once. The CLI can now upload the project's ready-to-serve files for this preview session. The owner keeps the instance credentials.
3. The author opens the draft in a voice channel and shares a separate tester invitation. The tester signs in, joins that channel and opens the invitation. It grants participation in this draft only. The activity runs in its own frame with `context.standalone` set to false and uses the same `connect()` call as in the local test.
4. Leaving the voice channel ends that person's access. Ending the preview session closes the draft for everyone and expires both the CLI's upload access and the tester invitation. Another test session starts with a fresh pairing code. Publishing and installing a release follow section 7.

## 10. Compatibility

*Proposed.*

- **Decided.** Cockpit checks the declared `protocol` and every requested Stellarr permission before offering **Install and enable** or **Update**. An unsupported protocol or unknown permission makes that release unavailable. The owner sees the specific version or permission in the installer or library entry. Existing installations and sessions continue on their approved release.
- Within a protocol version, changes are additive. Readers may ignore unfamiliar descriptive metadata.
- An instance keeps serving older versions. An activity written for version 1 keeps working after the instance is upgraded.
- The SDK's version never has to match the instance's version.
- The no-build module in section 6 is versioned by protocol. A standalone development server can serve a local copy at the same address.

## 11. Limits

*Names proposed. No value is set; each is to be set after measuring.*

| Limit | Meaning | What the owner's activities needed on the earlier platform |
| --- | --- | --- |
| `stateBytes` | Largest shared state | SyncTube capped its state at 256 KiB, `cheetah-strike` at 512 KiB |
| `writesPerSecond` | State writes per participant | `cheetah-strike` wrote every 2 s and after each command |
| `messageBytes` | Largest message | `cheetah-strike` sent whole-match snapshots of up to 64 KiB |
| `messagesPerSecond` | Messages per participant | `cheetah-strike`: 64 inputs a second from each guest, 15 to 30 snapshots a second from the host |
| `emptySeconds` | How long an empty session keeps its state | not known |
| `graceSeconds` | How long a disconnected person keeps their place | 10 s before the host role moved |
| `maxParticipants` | People admitted to a session | SyncTube allowed 25, `cheetah-strike` 10 |

## 12. Requirements beyond the core

*Proposed scope.* These are requirements for activities that need them. Their public APIs and release dates are open.

| Need | Observable behavior |
| --- | --- |
| Collaborative documents | Edits to different items survive concurrent writes and reconnects. A late joiner gets the document and its embedded content. The earlier Excalidraw integration allowed 16 MiB including images. The merge rule for edits to the same item and retention beyond a session are open. |
| Fast live updates | A late joiner can obtain a current snapshot. A slow recipient can skip obsolete values without holding up other recipients. Section 14 proposes a workload to evaluate the core message API under these conditions. |
| Private session data | A participant can recover a private assignment after reconnecting. A new host can recover the last confirmed public state and matching private checkpoint together. Only the current host may fetch the private checkpoint. |
| Saved progress | A person can resume on another device. Concurrent writes and a retry after a lost reply must preserve progress. |
| Instance-wide data | A shared listing can be claimed once when two people act at the same time. The earlier marketplace was optional. |
| Other requests | Activities have also asked for full-screen notifications and access to outside services via the instance. Their interfaces are open. Section 5 covers activity playback volume. |

### Players, spectators and private assignments

**Decided.** A game can reserve a playing seat for a participant and admit late arrivals as spectators while the session has capacity. When four people are playing cards, a fifth can watch. Spectators see the public round and score. The instance checks who holds a seat before accepting a player move or a score-changing write. A returning participant who still holds a seat resumes it after a reload or reconnect.

A game can give a seated participant a private assignment, such as a hand of cards or the word to act out. The instance checks the current account and seat before delivering or restoring the assignment. Public state, spectator snapshots and broadcasts omit it. If someone else takes the seat, the prior occupant's assignment stays private. The public SDK signatures for seats and assignments are open.

*Proposed round handling.* A game can keep a round ID and an end time in shared state. Clients display the countdown using the instance clock. A game checks the round ID when accepting a move. A guess that arrives after a rematch starts cannot score in the new round.

## 13. Outside this document

| Subject | Standing |
| --- | --- |
| Bots, and compatibility with Discord's bots and activities | Wanted by the owner. Proposed: bots run as their own processes and connect to the instance. They need a document of their own. |
| An activity hosted on someone else's server, reached through the instance | Asked for on 2026-11-22 and chosen on 2026-11-24, then overtaken by import from GitHub and by section 1. Never explicitly dropped. |
| Server code of an activity running on the instance | Dropped by section 1. |
| Building an activity on the instance | Dropped. |

## 14. Evidence and acceptance checks

### Earlier browser experiments

Each line is one run in Chrome 148. Firefox and Safari were not run. The tests used small stand-in pages on local names, not Stellarr: "the instance" was a test server with a cookie scoped like Stellarr's, and "outside" was another local name or port.

| Claim | Result |
| --- | --- |
| A YouTube player in a frame without a name of its own | Never became ready |
| A YouTube player in a frame with its own name | Played with sound when started by script |
| Names such as `app.chat.localhost` | Resolved with no setup |
| A page on a subdomain calling the instance | The instance's session cookie was attached |
| The rule against outside connections | Blocked requests, images and WebSockets to other hosts and to the instance |
| The same rule, the frame sending itself elsewhere | The request left; the response could be blocked |
| The same rule, WebRTC | Packets reached an outside port |
| An activity keeping 50 MB of assets in the browser | The second visit fetched only the page; requests from scripts, images and style sheets were all answered from the copy |

Not tested: more than 1 GB of assets, a browser restart, a self-signed certificate, and pointer lock, full screen, gamepad and clipboard inside the frame.

### Proposed acceptance checks

- **Install from a repository URL.** Give a new owner only the repository URL for the published poll with its icon. Cockpit must find the published release, show its version, size and grants, and make it available in a voice channel after one **Install and enable** action. Check that a direct release URL selects the named version. Try a repository with no release and a built activity whose release lacks its build output. Each failure must identify what the author should publish.
- **Multi-activity repository.** Publish two activities with different IDs, versions and grants under separate scoped tags. Paste the repository URL, select the second activity and check that only its permissions are requested and only its entry is installed. Publish a higher version of the first activity. The picker must offer both, with the second activity still pointing at its original release and ZIP. Repeat with a historical multi-entry release and an explicit scoped release URL. Disable, update and uninstall one while the other stays available.
- **Personal launch.** Open the same opted-in activity from two signed-in accounts outside voice. Confirm each account has its own session ID, queue and device grants. Start it from voice as well and confirm that the shared queue is independent. A voice-only release must decline personal launches, including direct API requests.
- **Private preview.** Pair the poll from the CLI, invite a second account in the same voice channel and cast one vote from each account. Both must see the same result. A second redemption of the pairing code, redemption of an expired code, and an upload using the tester invitation must all be refused. End the preview and verify that both frames close, further uploads fail and the invitation can no longer open the draft.
- **Voice-controlled jump.** Replay the same audio fixture twice from the same starting state and check that the same jump occurs each time. Deny the request, mute the microphone and disconnect the simulated device in separate runs. Sound must trigger no jumps while input is unavailable. On a paired instance, stop game access during a call and verify that voice still works.
- **Game audio during a call.** Two people talk while game music plays. One sets **Game audio** to 20%, then 0%, and triggers a new sound effect. At 0% that listener hears only the call. The other person still hears the music, effect and speaker at their chosen levels. Raise the slider again and verify that the music resumes audibly at its current position.
- **Charades, the second SDK example.** Give one player a private word and invite another to guess. With the performer's camera already on in the call, check that the game shows their tile only after consent. A third person joining as a spectator sees the tile and public score. Verify that their snapshot omits the word and their attempted move is refused. Reload the performer's frame and recover the same seat and word. Switch the camera off mid-turn and check that the tile clears for everyone and play can pass to the next participant.
- **Rematch (proposed).** Delay a correct guess from round one until round two has started in the same session. Both players must still see the initial score for round two. A returning player sees the same round deadline as the player who stayed.
- **Disable, enable and uninstall.** Disable the poll while two people have it open. Both participants' frames must close and a reconnect must be refused. Enable it again and verify that the same version starts a fresh poll. Uninstall it and verify that its library entry and launch option disappear.
- **Update.** Push a change without publishing a release, then publish a rebuild at the installed version. Neither offers an update. Publish a higher version with a supported additional permission. Cockpit must show the change before approval. Verify that an existing session retains its version, that a disabled installation stays disabled after updating, and that a failed verification preserves the previous installation. Repeat with an unknown permission or unsupported protocol and check that Cockpit names the reason.
- **Concurrent launch and device support.** Two eligible people click the same activity in an unused channel at the same time. Both see the same session ID and each other. Launch a different activity there and verify it has its own session. Open the channel on a phone with a desktop-only activity running. Its inline label appears before any activity assets are fetched.
- **Shared state.** Two participants add different items. Drop the reply to one accepted write, then retry it. Both participants and a late joiner must see each item once. After access is revoked, a reconnect must disclose no further state.
- **Hosted session.** Transfer the host role after a command commits but before its caller receives the reply. Retrying returns the recorded result. A delayed write from the previous host is refused.
- **Document.** Two people edit different items across a disconnect, one with an image. Both must converge after reconnecting. A third person must load both items and the image without either editor remaining online while the session is retained.
- **Live traffic.** Compare a shared connection against a separate binary WebSocket for live updates, keeping the workload and network conditions identical. Use ten clients, 64 inputs/s per guest, 15–30 snapshots/s from the host (up to 64 KiB each), and a checkpoint every two seconds. Repeat with a limited host uplink, a slow recipient and packet loss. Measure snapshot age at receipt and the longest visible freeze at each recipient, including during checkpoints. If transport retransmissions still dominate freezes, evaluate datagrams using the same SDK operations. These are candidate implementations and planned measurements.

## 15. Open questions

1. Is an activity hosted on someone else's server still wanted, or is import enough?
2. What does an instance without a domain get?
3. Do asset packs belong to the first version?
4. How does a private repository get in: an access token kept in Cockpit, public releases, or a folder copied onto the data volume?
5. Who may fetch an activity's files without signing in?
6. Who may start an activity, and what does voluntarily leaving the channel or call do?
7. Which documents, if any, should remain available after the session expires, and who may reopen them?
8. Which bots have to work, and is a music bot one of them?
