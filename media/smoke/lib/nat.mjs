// A NAT in front of the media server that can lose packets and shape the
// downlink of one browser.
//
// Browsers reach the media server only through this relay. Each browser
// address gets an upstream socket of its own, so the media server sees one
// distinct address per browser, as behind a real NAT, and the address it
// advertises is not the one it is bound to. Losing every n-th packet of one
// payload type, on the way up or down, makes NACK and RTX do their work on
// each leg of the media server separately. A shaped link puts what the media
// server sends one browser through a poor downlink: a rate cap with a bounded
// queue, delay, jitter and losses in bursts.

import { createSocket } from "node:dgram";

/** UDP and IPv4 headers, which a shaped link carries as well. */
const DATAGRAM_OVERHEAD = 28;

/** Whether a datagram is RTP (not STUN, DTLS or RTCP) of payload type `pt`. */
export function isRtp(data, pt) {
  if (data.length < 12 || data[0] >> 6 !== 2) return false;
  const second = data[1];
  // RTCP packet types 192 to 223 fill the whole second byte.
  if (second >= 192 && second <= 223) return false;
  return (second & 0x7f) === pt;
}

/** A generator that gives the same numbers on every run. */
function generator(seed) {
  let state = seed >>> 0 || 1;
  return () => {
    // mulberry32
    state = (state + 0x6d2b79f5) >>> 0;
    let t = state;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

/**
 * A poor downlink. Datagrams queue at a bottleneck that sends them one after
 * another at `kbps`, and one that would make the queue longer than `queueMs`
 * of that rate is dropped. What leaves it is delayed by `delayMs` plus up to
 * `jitterMs`, keeps its order, and is lost in bursts: a mean of `lossMean`
 * of the datagrams in bursts of `lossBurst` on average.
 */
export class ShapedLink {
  constructor(shape, deliver) {
    this.deliver = deliver;
    this.random = generator(shape.seed ?? 7);
    this.bad = false;
    this.freeAt = 0;
    this.lastArrival = 0;
    this.inFlight = [];
    this.timer = null;
    this.stats = { offered: 0, delivered: 0, deliveredBytes: 0, queueDrops: 0, burstDrops: 0 };
    this.maxQueueingMs = 0;
    /** How long each Opus packet waited at the bottleneck, in ms, in order. */
    this.voiceQueueingMs = [];
    this.reshape(shape);
  }

  reshape({ kbps, queueMs = 300, delayMs = 40, jitterMs = 15, lossMean = 0, lossBurst = 1 }) {
    this.bytesPerMs = (kbps * 1000) / 8 / 1000;
    this.queueBytes = this.bytesPerMs * queueMs;
    this.delayMs = delayMs;
    this.jitterMs = jitterMs;
    this.leave = 1 / lossBurst;
    this.enter = lossMean === 0 ? 0 : (lossMean * this.leave) / (1 - lossMean);
    this.kbps = kbps;
  }

  /** A datagram for `to` reaches the bottleneck now. */
  push(data, to) {
    const now = performance.now();
    this.stats.offered += 1;
    const size = data.length + DATAGRAM_OVERHEAD;
    const start = Math.max(now, this.freeAt);
    const backlog = (start - now) * this.bytesPerMs;
    if (backlog + size > this.queueBytes) {
      this.stats.queueDrops += 1;
      return;
    }
    this.freeAt = start + size / this.bytesPerMs;
    this.maxQueueingMs = Math.max(this.maxQueueingMs, start - now);
    if (isRtp(data, 111)) this.voiceQueueingMs.push(start - now);

    this.bad = this.bad ? this.random() >= this.leave : this.random() < this.enter;
    if (this.bad) {
      this.stats.burstDrops += 1;
      return;
    }
    const arrival = Math.max(
      this.lastArrival,
      this.freeAt + this.delayMs + this.random() * this.jitterMs,
    );
    this.lastArrival = arrival;
    this.stats.delivered += 1;
    this.stats.deliveredBytes += size;
    this.inFlight.push({ arrival, data, to });
    this.schedule();
  }

  schedule() {
    if (this.timer !== null || this.inFlight.length === 0) return;
    const wait = Math.max(0, this.inFlight[0].arrival - performance.now());
    this.timer = setTimeout(() => {
      this.timer = null;
      const now = performance.now();
      while (this.inFlight.length > 0 && this.inFlight[0].arrival <= now + 0.5) {
        const { data, to } = this.inFlight.shift();
        this.deliver(data, to);
      }
      this.schedule();
    }, wait);
  }

  close() {
    if (this.timer !== null) clearTimeout(this.timer);
    this.timer = null;
    this.inFlight = [];
  }
}

export class LossyNat {
  static listen() {
    return new Promise((resolve, reject) => {
      const socket = createSocket("udp4");
      socket.once("error", reject);
      socket.bind(0, "127.0.0.1", () => resolve(new LossyNat(socket)));
    });
  }

  constructor(socket) {
    this.socket = socket;
    this.port = socket.address().port;
    this.target = null;
    this.upstreams = new Map();
    this.rule = null;
    this.seen = { up: 0, down: 0 };
    this.lost = { up: 0, down: 0 };
    /** Shaped downlinks, by browser address. */
    this.links = new Map();
    /** Called with (direction, browser address, datagram, time) for every datagram. */
    this.taps = [];
    socket.on("message", (data, from) => this.fromBrowser(data, from));
  }

  /** Loses every `every`-th RTP packet of payload type `pt` in both directions. */
  lose({ pt, every }) {
    this.rule = { pt, every };
  }

  /** Every browser address this relay has heard from so far. */
  addresses() {
    return [...this.upstreams.keys()];
  }

  /**
   * Puts everything the media server sends to `addresses` through one shaped
   * link, or changes the shape of that link. Every address of one browser
   * shares it, as its one access link would.
   */
  shape(addresses, shape) {
    const existing = addresses.map((a) => this.links.get(a)).find(Boolean);
    if (existing) {
      existing.reshape(shape);
      return existing;
    }
    const link = new ShapedLink(shape, (data, to) => this.socket.send(data, to.port, to.address));
    for (const address of addresses) this.links.set(address, link);
    return link;
  }

  /** Lets what goes to `addresses` through untouched again. */
  unshape(addresses) {
    for (const address of addresses) {
      const link = this.links.get(address);
      if (!link) continue;
      this.links.delete(address);
      if (![...this.links.values()].includes(link)) link.close();
    }
  }

  dropped(direction, data) {
    if (!this.rule || !isRtp(data, this.rule.pt)) return false;
    this.seen[direction] += 1;
    if (this.seen[direction] % this.rule.every !== 0) return false;
    this.lost[direction] += 1;
    return true;
  }

  fromBrowser(data, from) {
    if (!this.target) return;
    const key = `${from.address}:${from.port}`;
    let upstream = this.upstreams.get(key);
    if (!upstream) {
      upstream = createSocket("udp4");
      upstream.bind(0, "127.0.0.1");
      upstream.on("message", (reply) => {
        for (const tap of this.taps) tap("down", key, reply, performance.now());
        if (this.dropped("down", reply)) return;
        const link = this.links.get(key);
        if (link) link.push(reply, from);
        else this.socket.send(reply, from.port, from.address);
      });
      this.upstreams.set(key, upstream);
    }
    for (const tap of this.taps) tap("up", key, data, performance.now());
    if (!this.dropped("up", data)) upstream.send(data, this.target, "127.0.0.1");
  }

  close() {
    for (const link of new Set(this.links.values())) link.close();
    for (const upstream of this.upstreams.values()) upstream.close();
    this.socket.close();
  }
}
