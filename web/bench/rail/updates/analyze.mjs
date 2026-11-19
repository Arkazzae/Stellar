// Reads what probe.mjs wrote and prints, per page and per window: commits,
// components rendered in the rail and in the app, what changed on the occupant
// rows, and, for the speaking ring, whether each flip of the ring follows a
// flip of the detector or comes from somewhere else.
//
//   node bench/rail/updates/analyze.mjs result.json [--pages a,c] [--snaps]
import { readFileSync } from "node:fs";

const [file, ...rest] = process.argv.slice(2);
const pagesWanted = (rest[rest.indexOf("--pages") + 1] ?? "a,b,c").split(",");
const showSnaps = rest.includes("--snaps");
const result = JSON.parse(readFileSync(file, "utf8"));

const RAIL = ["RailContainer", "Rail", "RailVoiceList", "VoiceOccupantRow", "ChannelButton", "RailTextList", "Avatar", "VoiceDock", "AccountRow"];
const BLIP_MS = 300;

function windows(page) {
  const marks = page.marks;
  return marks.map(([t, label], index) => ({ label, from: t, to: marks[index + 1]?.[0] ?? Infinity }));
}

const within = (rows, w) => rows.filter((row) => row[0] >= w.from && row[0] < w.to);

function ringTimeline(page) {
  // Per row (channel/who): the times the ring went on and off.
  const rows = new Map();
  for (const [t, kind, channel, who, oldValue, value] of page.dom) {
    if (kind !== "cls") continue;
    const before = (oldValue ?? "").split(" ").includes("speaking");
    const after = (value ?? "").split(" ").includes("speaking");
    if (before === after) continue;
    const key = `${channel}/${who}`;
    if (!rows.has(key)) rows.set(key, []);
    rows.get(key).push([t, after ? 1 : 0]);
  }
  return rows;
}

function flips(page) {
  const byNode = new Map();
  for (const [t, id, speaking] of page.vad) {
    const list = byNode.get(id) ?? [];
    if (list.length === 0 || list.at(-1)[1] !== speaking) list.push([t, speaking]);
    byNode.set(id, list);
  }
  return byNode;
}

function blips(events) {
  // Short "off" gaps between two "on" stretches, and short "on" stretches.
  let shortOff = 0;
  let shortOn = 0;
  const gaps = [];
  for (let i = 1; i < events.length; i += 1) {
    const [t0, v0] = events[i - 1];
    const [t1] = events[i];
    const length = t1 - t0;
    if (length < BLIP_MS) {
      if (v0 === 0) {
        shortOff += 1;
        gaps.push(length);
      } else shortOn += 1;
    }
  }
  return { shortOff, shortOn, gaps };
}

function explain(ring, nodeFlips, toleranceMs = 120) {
  // A ring flip is the detector's when one of the candidate nodes flipped the
  // same way within the tolerance before it.
  let explained = 0;
  const unexplained = [];
  for (const [t, value] of ring) {
    const match = nodeFlips.some((list) => list.some(([ft, fv]) => fv === value && ft <= t + 5 && t - ft <= toleranceMs));
    if (match) explained += 1;
    else unexplained.push([t, value]);
  }
  return { explained, unexplained };
}

function fmt(n) {
  return Number.isFinite(n) ? n.toFixed(1) : String(n);
}

for (const key of pagesWanted) {
  const page = result.pages[key];
  if (!page || page.error) {
    console.log(`\n== page ${key}: ${page?.error ?? "missing"}`);
    continue;
  }
  console.log(`\n== page ${key}  (${page.commits.length} commits, ${page.vad.length} detector posts, nodes ${JSON.stringify(page.vadNodes)})`);
  for (const w of windows(page)) {
    const commits = within(page.commits, w);
    const end = Number.isFinite(w.to) ? w.to : (page.commits.at(-1)?.[0] ?? w.from);
    const seconds = Math.max(0.001, (end - w.from) / 1000);
    const rail = Object.fromEntries(RAIL.map((name) => [name, 0]));
    let total = 0;
    let railTotal = 0;
    for (const [, , count, inRail] of commits) {
      total += count;
      for (const [name, n] of Object.entries(inRail)) {
        railTotal += n;
        if (name in rail) rail[name] += n;
      }
    }
    const dom = within(page.dom, w);
    const cls = dom.filter((row) => row[1] === "cls");
    const changed = (word) =>
      cls.filter(([, , , , o, v]) => (o ?? "").split(" ").includes(word) !== (v ?? "").split(" ").includes(word)).length;
    const adds = dom.filter((row) => row[1] === "add").map((row) => `+${row[3]}@${row[2]}`);
    const removes = dom.filter((row) => row[1] === "remove").map((row) => `-${row[3]}@${row[2]}`);
    const anims = within(page.anim, w).map((row) => `${row[1]}:${row[3]}@${row[2]}`);
    const vad = within(page.vad, w);
    const top = Object.entries(page.all[w.label] ?? {})
      .sort((x, y) => y[1] - x[1])
      .slice(0, 8)
      .map(([name, n]) => `${name} ${n}`)
      .join(", ");
    console.log(
      `\n-- ${w.label}  ${fmt(seconds)} s: ${commits.length} commits (${fmt(commits.length / seconds)}/s), ${total} components, ${railTotal} in rail; detector posts ${vad.length}`,
    );
    console.log(`   rail: ${RAIL.map((name) => `${name} ${rail[name]}`).join(", ")}`);
    // Split by cause: a commit within 150 ms after a detector post is the
    // detector's; the rest come from the hub, the call's timers or the page.
    const postTimes = page.vad.map((row) => row[0]);
    const byDetector = commits.filter(([t]) => postTimes.some((p) => t - p >= 0 && t - p <= 150));
    const other = commits.filter((commit) => !byDetector.includes(commit));
    const sum = (list) => list.reduce((acc, [, , count, inRail]) => [acc[0] + count, acc[1] + (inRail.RailContainer ?? 0), acc[2] + (inRail.VoiceOccupantRow ?? 0)], [0, 0, 0]);
    const [detTotal, detRail, detRows] = sum(byDetector);
    const [otherTotal, otherRail, otherRows] = sum(other);
    console.log(
      `   by detector: ${byDetector.length} commits, ${detTotal} components, rail container ${detRail}x, rows ${detRows}; other: ${other.length} commits, ${otherTotal} components, rail container ${otherRail}x, rows ${otherRows}`,
    );
    console.log(`   rows: speaking flips ${changed("speaking")}, muted flips ${changed("muted")}, me ${changed("me")}, moving ${changed("is-moving")}, draggable ${changed("is-draggable")}; ${adds.join(" ")} ${removes.join(" ")}`);
    if (anims.length > 0) console.log(`   animations: ${anims.join(" ")}`);
    console.log(`   app top: ${top}`);
    if (showSnaps) {
      for (const [t, snap] of within(page.snaps, w)) console.log(`   snap +${t - w.from} ms: ${snap}`);
    }
  }

  // Commits while no detector posted anything for a whole second: the call's own timers.
  const quiet = [];
  const posts = page.vad.map((row) => row[0]);
  const inCallFrom = page.marks.find(([, label]) => label === "a-join-speaking")?.[0] ?? 0;
  for (const [t, label, count] of page.commits) {
    if (t < inCallFrom || label === "restart" || label === "after-restart" || label === "end") continue;
    const near = posts.some((p) => Math.abs(p - t) < 1000);
    if (!near) quiet.push([t, count]);
  }
  console.log(`\n   commits with no detector post within 1 s (in-call part): ${quiet.length}`);

  // The ring against the detector.
  const ring = ringTimeline(page);
  const nodes = flips(page);
  const nodeInfo = new Map(page.vadNodes.map((node) => [node.id, node]));
  console.log(`\n   detector nodes: ${[...nodes].map(([id, list]) => `#${id}${nodeInfo.get(id)?.gate ? "(gate)" : nodeInfo.get(id)?.silentLevels ? "(own mic)" : "(remote)"} ${list.length} flips, blips ${JSON.stringify(blips(list).shortOff)} short-off ${blips(list).shortOn} short-on, gaps ${blips(list).gaps.join(",")}`).join("; ")}`);
  for (const [row, events] of ring) {
    const b = blips(events);
    const own = row.endsWith(`/${({ a: "Ala", b: "Bob", c: "Cyd" })[key]}`);
    const candidates = [...nodes]
      .filter(([id]) => {
        const info = nodeInfo.get(id);
        return own ? info?.silentLevels && !info.gate : !info?.silentLevels && !info?.gate;
      })
      .map(([, list]) => list);
    const { explained, unexplained } = explain(events, candidates);
    console.log(
      `   ring ${row}: ${events.length} flips, ${b.shortOff} off-gaps < ${BLIP_MS} ms (${b.gaps.join(",")}), ${b.shortOn} on-blips; follows a detector flip: ${explained}, does not: ${unexplained.length}`,
    );
  }
}

console.log(`\ntimeline: ${result.timeline.map(([, text]) => text).join(" | ")}`);
if (result.error) console.log(`error: ${result.error}`);
