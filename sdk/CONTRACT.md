# Activity contract

> [!NOTE]
> Draft of 2026-11-25. Nothing in this document is implemented. It records what an activity and Stellarr promise each other, so the SDK, the web client and the instance can be built against one text.

An activity is something the people in a voice channel or a call open together: a watch party, a board game, a whiteboard. This document is for whoever writes an activity and for whoever builds the parts of Stellarr that run one.

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
| Activity | A web page that people open together inside Stellarr. |
| Author | Whoever writes an activity. |
| Session | One running activity and the people in it. They share one state. |
| Participant | A person in a session. |
| Library | The activities an instance owner has added to the instance. |

## 1. What an activity is

*Decided.*

- An activity is a static web page: HTML, scripts and assets. It has no server code of its own.
- Stellarr on the instance is the activity's backend. Whatever an activity needs from a server, it gets from this contract.
- The instance serves the activity. People's browsers fetch it from the instance and connect to nothing else.
- The instance owner adds activities to the instance. What gets added is the owner's responsibility.
- SyncTube ships with Stellarr and is built on this same contract.
- The SDK is an installable package. The owner's activities already import it as `@stellarr/sdk`.

## 2. Who talks to whom

*Proposed.*

The activity's page talks only to the Stellarr page around it. That page talks to the instance over Stellarr's own connection. Which connection carries session traffic is open (section 12).

```text
activity page  <->  Stellarr page around it  <->  instance
```

- The activity's page holds no session, no token and no connection of its own to the instance.
- Stellarr's page knows who the person is and which session the frame belongs to, so an activity cannot claim to be someone else.
- The instance's own API is out of reach from the activity's page (section 4).

## 3. What Stellarr gives an activity

*Proposed, including every name below.*

| Thing | In one sentence |
| --- | --- |
| Context | Who I am, where this runs, which language, which colours. |
| Participants | Who is in the session right now. |
| Shared state | One object that everyone in the session reads and writes, kept by the instance. |
| Messages | Quick notes to the others that nobody stores. |
| Clock | One time that is the same for everyone. |

### Operations

| Operation | What it does | What is promised |
| --- | --- | --- |
| `connect({ initialState })` | Joins the session. | Finishes only when the current state is loaded, so a late joiner starts in step. `initialState` is used only when the session has no state yet. |
| `context` | Session id, the person (`me`), the place, the locale, a `standalone` flag. | Does not change while the page lives. |
| `theme` | Light or dark, and six colours of the person's palette. | Changes are announced. |
| `participants` | The list of people, with a way to hear changes. | Same order for everyone: by time of joining. |
| `state.get()` | The shared state as this page last received it. | |
| `state.set(patch)` | Replaces the given top-level keys. | The instance puts all writes in one order and sends each to everyone, the writer included. |
| `state.set(change)` | Computes the patch from the latest state. | Runs again if someone else wrote first. Returning nothing gives up; throwing refuses and writes nothing. |
| `state.subscribe(listener)` | Hears every change of the state. | |
| `send(kind, data, options)` | Sends a message to the others, or to one of them. | Never stored. Returns false when it was dropped at once. |
| `on(kind, listener)` | Hears messages of one kind. | The sender is named by the instance, not by the message. |
| `now()` | The instance's time in milliseconds. | The same moment gives every participant the same number, within the error of the estimate. That error is not measured yet. |
| `limits` | The sizes and rates the instance enforces (section 11). | |

### Participants

- A participant is a person, not a tab. The id stays the same after a reload and on another device.
- An id is an opaque text. An activity may compare ids and must not read anything into their shape.
- A participant has an id, a name, an avatar address served by the instance, and a `connected` flag.
- A person has one frame per session. Opening the activity somewhere else closes it where it was open before.
- Someone whose page reloads or whose connection drops keeps their place for `graceSeconds`, shown as not connected.
- There is no host in the contract. An activity that wants one treats the first participant as host; when that person leaves, the next one is first.

### Shared state

- The state is one JSON object per session.
- A write replaces whole top-level keys. Of two writes to the same key, the later one stays.
- A page sees its own write when the instance sends it back, never earlier.
- The instance keeps the state while anyone is in the session and for `emptySeconds` after the last person leaves. A reload loses nothing.
- After a lost connection the page receives the whole state again.
- A write is refused when the state would exceed `stateBytes`, when writes come faster than `writesPerSecond`, or when the page is offline.
- Any participant may write any key, and every participant can read all of it. The instance does not look at the content.

### Messages

- A message goes to everyone else in the session, or to one participant named in `options.to`. A message for one participant is not shown to the others.
- The data is anything JSON can hold, or bytes.
- With `options.latest`, a newer message of the same kind replaces an older one that is still waiting on a slow connection.
- A message can be lost: when the sender is offline or over a limit, when the receiver is not connected, or when a newer one replaced it.
- **Open:** whether messages arrive in the order they were sent, and whether the sender can learn about congestion. Both depend on a transport that does not exist yet.

## 4. What an activity cannot do

| Limit | Why | Mark |
| --- | --- | --- |
| Act as the signed-in person | The page has no session and cannot reach the instance's API. | Proposed; holds only once the instance refuses requests from other pages (section 7) |
| Read another activity's saved data | Each activity is served under its own name, and a browser keeps storage apart by name. | Decided (a name per activity) |
| Connect to anything outside the instance | Privacy of the people on the instance. | Decided |
| Keep a secret in the shared state | Every participant can read all of it. | Proposed |
| Enforce a rule against a modified page | The instance does not check what is written. | Proposed, accepted as the cost of having no server code |
| Run code on the instance | See section 1. | Decided |

### The rule against outside connections

*Proposed mechanism for a decided goal.*

- Every file served under an activity's name tells the browser to load from and connect to that name only.
- An activity that needs an outside service names each host in its description file. The instance owner sees the list before adding the activity. SyncTube names YouTube. Which hosts an activity may name at all is open (section 15).
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

The frame is meant to refuse new windows, navigating the Stellarr page, and the camera, microphone and screen.

Only scripts, own storage and unprompted playback were tested (section 14). `minecraft-wasm` also asks for cross-origin isolation of the whole page, which this contract does not cover.

## 6. Description file

*Proposed.*

An activity carries one small file, `stellarr.json`, beside its page.

| Field | Meaning |
| --- | --- |
| `protocol` | The version of this contract the activity speaks. `1` today. |
| `id` | Lower-case letters, digits and dashes. Becomes the activity's name on the instance. |
| `name`, `description` | What people see in the list of activities. |
| `version` | The author's version. |
| `icon`, `cover` | Paths beside the file. |
| `connects` | Outside hosts the page talks to, each with a reason. Empty for most activities. Rests on an open question (section 15). |

```json
{
  "protocol": 1,
  "id": "synctube",
  "name": "SyncTube",
  "description": "Watch YouTube together.",
  "version": "1.0.3",
  "icon": "artwork/icon.png",
  "cover": "artwork/cover.webp",
  "connects": [{ "host": "www.youtube.com", "why": "the video player" }]
}
```

The file does not describe the state or the messages. The instance checks sizes and rates, never content, so an activity declares its data once, in its own code.

## 7. Getting an activity onto an instance

### Where it comes from

- **Decided:** an activity lives in a repository, such as one on GitHub. The instance owner pastes the link in Cockpit.
- **Decided:** the instance never builds an activity. It downloads files that are ready to serve.
- **Decided as the working split:** an activity that needs no build is taken straight from its branch. An activity that needs one is built on GitHub's side on every push, by a recipe that comes with the SDK.
- **Proposed:** every import pins one exact commit and checks what it downloaded against a checksum. What the owner approved cannot change afterwards.
- **Proposed:** one repository may describe several activities, so a whole collection is one link.

### What the instance does

*Proposed steps for a decided flow.*

1. Reads the description file and shows the owner the name, the version, the size and the outside hosts.
2. On approval, downloads the files once. People's browsers never contact GitHub.
3. Stores them on the data volume, under that version. Nothing is written into the image, which every upgrade replaces.
4. Gives the activity its own name.
5. Lists it for launch on every server of the instance.

An update happens only when the owner asks for one. Removing an activity deletes its files.

### Names and certificates

- **Decided:** each activity gets its own name. A wildcard domain is acceptable for that.
- **Proposed:** the base of those names is a setting of its own, not necessarily a subdomain of the instance's name.
- The owner supplies a DNS record that covers those names and a certificate that covers them. Stellarr does not obtain certificates: by default it makes a self-signed one for a single name.
- A page on a subdomain of the instance has the instance's session cookie attached to its requests (section 14). The instance must therefore refuse any request that does not come from its own page. As of this draft the hub does; the same was not found for the HTTP API as a whole.
- **Open:** an instance reached by a bare IP address, or with the default self-signed certificate, cannot give activities their own names. Proposed: such an instance runs only the activities that ship with Stellarr.

### Large assets

- **Decided:** a small fixed cap on size, such as 20 MB, is too low.
- **Proposed:** large assets travel as separate packs, each named by the hash of its bytes. An unchanged pack is never downloaded twice, by the instance or by a browser.
- **Proposed:** the instance fetches packs itself and skips the ones it already holds.
- **Proposed:** in the browser, a ready part of the SDK keeps packs under the activity's own name and answers ordinary file addresses from them. The activity's code needs no patches and no list of packs.
- **Proposed:** the owner sees the size before approving, and assets stay out of backups because they can be downloaded again.
- **Open:** whether packs belong to the first version. Only `cheetah-strike` needs them: 16 packs, the largest 506 MB, about 1.9 GB and 7,799 files together.

The earlier Stellarr had this mechanism working. Its reader of the pack format, its packer and its browser store are candidates to carry over.

## 8. Sessions

What this contract fixes, *proposed*:

- A session belongs to a place: a voice channel, a call, or no place at all for someone who starts alone.
- The people in a session share one state and nothing else.
- The state outlives the people by `emptySeconds`.

Everything else is **open** and was never discussed: who may start an activity, whether one place can run several at once, what leaving the voice channel does, and whether a session's state is still there the next day.

## 9. Outside Stellarr

*Proposed.*

A page opened on its own, outside any frame, works the same way. `connect` then joins a room shared by the tabs of that page's address, so two tabs of a development server are two participants. No instance is needed to write or test an activity. `context.standalone` is true in that case.

## 10. Compatibility

*Proposed.*

- An activity says which version of the contract it speaks. Within a version, things are only added, and each side ignores what it does not know.
- An instance keeps serving older versions. An activity written for version 1 keeps working after the instance is upgraded.
- The SDK's version never has to match the instance's version.
- For activities without a build step, the instance serves the SDK file itself. This is untested.

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
| people in a session | | SyncTube allowed 25, `cheetah-strike` 10 |

## 12. Known gaps

With no server code of its own, an activity can only do what Stellarr offers. These needs have no answer in this contract yet.

| Gap | Who needs it |
| --- | --- |
| Data saved per person | `cheetah-strike`: inventory, statistics, settings |
| Data shared by everyone on the instance | `cheetah-strike`: its market |
| Large shared data | `excalidraw`: a document of up to 16 MiB with images |
| State only one participant may read, that survives that person leaving | `cheetah-strike`: the secret role and the host's checkpoint |
| A connection quick enough for games | `sporeio`, `cheetah-strike` |
| Signals from Stellarr such as volume and full screen | `cheetah-strike` |
| Reaching an outside service without the person's browser doing it | `kings-reigns` and `resonance` name such services in their code; whether they call them was not checked |

## 13. Outside this document

| Subject | Standing |
| --- | --- |
| Bots, and compatibility with Discord's bots and activities | Wanted by the owner. Proposed: bots run as their own processes and connect to the instance. They need a document of their own. |
| An activity hosted on someone else's server, reached through the instance | Asked for on 2026-11-22 and chosen on 2026-11-24, then overtaken by import from GitHub and by section 1. Never explicitly dropped. |
| Server code of an activity running on the instance | Dropped by section 1. |
| Building an activity on the instance | Dropped. |

## 14. What was tested

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

## 15. Open questions

1. Is an activity hosted on someone else's server still wanted, or is import enough?
2. What does an instance without a domain get?
3. Do asset packs belong to the first version?
4. How does a private repository get in: an access token kept in Cockpit, public releases, or a folder copied onto the data volume?
5. Who may fetch an activity's files without signing in?
6. Which outside hosts may an activity name: only YouTube for SyncTube, or others too, such as an AI service or a music source?
7. Who may start an activity, can one place run several, and what does leaving the voice channel do?
8. Is a session's state still there the next day?
9. Which bots have to work, and is a music bot one of them?
