//! What the core keeps for one session: its connection, what it publishes,
//! what it receives and where its negotiation stands.

use std::collections::{HashMap, HashSet, VecDeque};
use std::time::Instant;

use str0m::change::SdpPendingOffer;
use str0m::media::{Direction, KeyframeRequestKind, MediaKind, Mid};
use str0m::rtp::Ssrc;
use str0m::{IceConnectionState, Rtc};

use super::camera::{Layer, Selection};
use super::downlink::{Downlink, RateMeter};
use super::limits::{Limits, TrackLimit};
use super::{
    CONNECT_TIMEOUT, DISCONNECT_TIMEOUT, Endpoint, KEYFRAME_INTERVAL, MAX_SOURCE_ADDRS,
    OFFER_TIMEOUT,
};
use crate::sdp_text::{self, FormatOrders, OpusParams, Restreams};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct SessionKey(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct RoomKey(pub u64);

/// A track is named by the session that publishes it and the mid of the
/// publisher's section that carries it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct TrackKey {
    pub session: SessionKey,
    pub mid: Mid,
}

/// The transport milestones the 30 second rules are measured against.
#[derive(Debug)]
pub(crate) struct Transport {
    /// ICE, DTLS and SRTP have been up at least once.
    pub ever_connected: bool,
    pub ice: IceConnectionState,
    /// Since when a connection that was up has been down.
    pub disconnected_since: Option<Instant>,
}

pub(crate) struct PendingOffer {
    pub id: u64,
    pub pending: SdpPendingOffer,
    pub sent_at: Instant,
    /// Sections this offer makes inactive. The browser has heard of their
    /// withdrawal once it answers.
    pub inactive: Vec<Mid>,
}

#[derive(Default)]
pub(crate) struct Negotiation {
    /// The browser's first offer has been answered. The media server never
    /// offers before that, so the browser always offers first.
    pub remote_described: bool,
    pub pending: Option<PendingOffer>,
    /// The `negotiationId` of the last offer the media server made.
    pub last_id: u64,
    /// Offers in a row that timed out without an answer.
    pub unanswered: u32,
}

/// Keyframe requests to a publisher are rate limited per track. A request
/// that comes too soon is held and sent when the window ends, so a second
/// receiver that starts shortly after the first still gets its keyframe.
#[derive(Debug, Default)]
pub(crate) struct KeyframeGate {
    pub last: Option<Instant>,
    pub pending: Option<KeyframeRequestKind>,
}

impl KeyframeGate {
    pub fn hold(&mut self, kind: KeyframeRequestKind) {
        // FIR asks for more than PLI, so it is the one that is kept.
        self.pending = match (self.pending, kind) {
            (Some(KeyframeRequestKind::Fir), _) | (_, KeyframeRequestKind::Fir) => {
                Some(KeyframeRequestKind::Fir)
            }
            _ => Some(KeyframeRequestKind::Pli),
        };
    }

    pub fn can_send(&self, now: Instant) -> bool {
        self.last.is_none_or(|last| now >= last + KEYFRAME_INTERVAL)
    }

    /// When a held request may go out. A request is only ever held after
    /// one was sent, so there always is a last one to count from.
    pub fn due(&self) -> Option<Instant> {
        self.pending?;
        Some(self.last? + KEYFRAME_INTERVAL)
    }
}

/// A section of the browser's description that sends towards the media
/// server.
#[derive(Debug)]
pub(crate) struct Published {
    pub mid: Mid,
    pub kind: MediaKind,
    /// The stream id the publisher currently gives the track; empty when it
    /// names none.
    pub stream_id: String,
    pub keyframe: KeyframeGate,
    /// What forwarding the track costs a receiver.
    pub meter: RateMeter,
    pub layered: bool,
    pub thumbnail_meter: RateMeter,
    /// What the publisher may send on the track.
    pub limit: TrackLimit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OutState {
    /// To be added by the next offer.
    Want,
    /// Added by the pending offer.
    Offered(Mid),
    /// Added by the pending offer, but the track was withdrawn meanwhile.
    OfferedWithdrawn(Mid),
    /// Negotiated: packets are forwarded.
    Live(Mid),
    /// Withdrawn: inactive locally, the browser has not been told yet.
    Ending(Mid),
}

/// The payload type a receiver uses for the codec of one incoming payload
/// type, valid until either side negotiates again.
#[derive(Debug, Clone, Copy)]
pub(crate) struct PtCache {
    pub pt_in: str0m::media::Pt,
    pub generation_in: u64,
    pub generation_out: u64,
    pub pt_out: Option<str0m::media::Pt>,
}

/// A send-only section that forwards one track of another session.
#[derive(Debug)]
pub(crate) struct OutSection {
    pub source: TrackKey,
    pub kind: MediaKind,
    pub state: OutState,
    /// Whether a packet has been forwarded yet. The first one asks the
    /// publisher for a keyframe.
    pub started: bool,
    pub rewrite: Rewrite,
    pub camera: Selection,
    pub pt_cache: Option<PtCache>,
    /// The section was taken from the idle ones for this track. If its offer
    /// is rolled back it goes back to them: the browser still has it inactive.
    pub recycled: bool,
}

impl OutSection {
    pub fn new(source: TrackKey, kind: MediaKind) -> OutSection {
        OutSection {
            source,
            kind,
            state: OutState::Want,
            started: false,
            rewrite: Rewrite::default(),
            camera: Selection::default(),
            pt_cache: None,
            recycled: false,
        }
    }

    /// The mid of the section in this browser's directory, if it has one.
    pub fn directory_mid(&self) -> Option<Mid> {
        match self.state {
            OutState::Offered(mid) | OutState::Live(mid) => Some(mid),
            _ => None,
        }
    }

    /// Whether the section still stands for its track. One whose track was
    /// withdrawn only waits until the browser has heard of that.
    pub fn is_current(&self) -> bool {
        matches!(
            self.state,
            OutState::Want | OutState::Offered(_) | OutState::Live(_)
        )
    }
}

/// Maps the sequence numbers and timestamps of a publisher onto one
/// receiving section.
///
/// A section starts with the wire sequence number of the first packet it
/// forwards, so it never begins past a rollover the receiver cannot know
/// about. The offset stays constant while packets are forwarded, so a gap the
/// publisher had is a gap the receiver sees and can ask to be repaired.
///
/// A section the receiver stops asking for skips what arrives meanwhile, and
/// the first packet after it continues the numbering where it stopped: the
/// receiver must not ask for what was never meant for it. The timestamps keep
/// the publisher's clock, so the track stays in step with the ones that were
/// never paused, as after any silence. When the publisher changes its SSRC
/// the section continues where it was instead of jumping; a new SSRC brings a
/// clock of its own, so its timestamps continue from the last one sent.
#[derive(Debug, Default)]
pub(crate) struct Rewrite {
    state: Option<RewriteState>,
    /// The section was paused: the next packet begins a new run of numbers.
    resume: bool,
    /// The highest source sequence number of one SSRC that was forwarded
    /// before a pause or skipped during it. Nothing of that SSRC up to it is
    /// forwarded again, so a late packet cannot become the first of the next
    /// run, nor follow it under a new SSRC.
    resume_after: Option<(Ssrc, u64)>,
}

#[derive(Debug)]
struct RewriteState {
    ssrc: Ssrc,
    seq_offset: i128,
    ts_offset: u32,
    last_seq: u64,
    last_ts: u32,
    /// The first source sequence number of the current run, when a pause
    /// moved the offset. Older packets were numbered under the old offset and
    /// would land on numbers already given out.
    resume_floor: Option<u64>,
}

impl Rewrite {
    /// The receiver stopped asking for the section.
    pub fn pause(&mut self) {
        self.resume = true;
        if let Some(state) = &self.state {
            let last_source = u64::try_from(state.last_seq as i128 + state.seq_offset)
                .expect("a source sequence previously mapped from u64");
            self.resume_after = Some(match self.resume_after {
                Some((ssrc, previous)) if ssrc == state.ssrc => (ssrc, previous.max(last_source)),
                _ => (state.ssrc, last_source),
            });
        }
    }

    /// A packet arrived while the receiver does not ask for the section.
    pub fn skip(&mut self, ssrc: Ssrc, seq: u64) {
        self.resume = true;
        self.resume_after = Some(match self.resume_after {
            Some((previous_ssrc, previous)) if previous_ssrc == ssrc => (ssrc, previous.max(seq)),
            _ => (ssrc, seq),
        });
    }

    /// The outgoing sequence number and timestamp of an incoming packet, or
    /// `None` for a packet that would land before the start of the section.
    pub fn map(&mut self, ssrc: Ssrc, seq: u64, ts: u32) -> Option<(u64, u32)> {
        if let Some((skipped, last)) = self.resume_after
            && skipped == ssrc
        {
            if seq <= last {
                return None;
            }
            self.resume_after = None;
        }
        let resume = std::mem::take(&mut self.resume);
        let state = match &mut self.state {
            Some(state) if state.ssrc == ssrc => {
                let offset = seq as i128 - (state.last_seq as i128 + 1);
                // A pause that skipped nothing leaves the numbering as it
                // was, and a late repair from before it still fits.
                if resume && offset != state.seq_offset {
                    state.seq_offset = offset;
                    state.resume_floor = Some(seq);
                }
                state
            }
            Some(state) => {
                state.ssrc = ssrc;
                state.seq_offset = seq as i128 - (state.last_seq as i128 + 1);
                state.ts_offset = state.last_ts.wrapping_add(1).wrapping_sub(ts);
                // Older packets of this source would overlap the previous SSRC's numbers.
                state.resume_floor = Some(seq);
                state
            }
            None => self.state.insert(RewriteState {
                ssrc,
                seq_offset: (seq - u64::from(seq as u16)) as i128,
                ts_offset: 0,
                last_seq: 0,
                last_ts: ts,
                resume_floor: resume.then_some(seq),
            }),
        };

        if state.resume_floor.is_some_and(|floor| seq < floor) {
            return None;
        }
        let out_seq = u64::try_from(seq as i128 - state.seq_offset).ok()?;
        let out_ts = ts.wrapping_add(state.ts_offset);
        if out_seq >= state.last_seq {
            state.last_seq = out_seq;
            state.last_ts = out_ts;
        }
        Some((out_seq, out_ts))
    }
}

/// Packet and byte counts for the stats.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct Counter {
    pub packets: u64,
    pub bytes: u64,
}

impl Counter {
    pub fn count(&mut self, bytes: usize) {
        self.packets += 1;
        self.bytes += bytes as u64;
    }
}

/// How many remote addresses one session keeps routed to it: every address
/// its checks may come from. A browser with many interfaces keeps checking its
/// backup pairs from all of them, about every 25 seconds, while its media
/// leaves through one. With fewer routes than addresses, those checks would
/// push out the route of the address that carries the media, whose packets
/// would then be dropped until its own next check, up to 2.5 seconds later.
const MAX_ADDRS: usize = MAX_SOURCE_ADDRS;

pub(crate) struct Session {
    pub key: SessionKey,
    pub id: String,
    pub user_id: String,
    pub room: RoomKey,
    pub rtc: Rtc,
    pub ufrag: String,
    pub created_at: Instant,
    pub transport: Transport,
    pub negotiation: Negotiation,
    pub published: Vec<Published>,
    pub outs: Vec<OutSection>,
    pub waiting_for_space: bool,
    /// The mids of this receiver's sections it asked to be sent. A section
    /// that is not listed is negotiated and named in the directory, but
    /// nothing is forwarded on it.
    pub subscriptions: HashSet<String>,
    pub thumbnail_mids: HashSet<String>,
    /// The same mids in the order the receiver listed them, most important
    /// first.
    pub subscription_order: Vec<String>,
    /// What this receiver's downlink carries, and which of the pictures it
    /// asks for are held back to stay inside it.
    pub downlink: Downlink,
    /// An allocation of the downlink is queued.
    pub allocation_queued: bool,
    /// The current section of every track this session receives. A section
    /// whose track was withdrawn is not in it any more, so the same track
    /// sent again on the publisher's same section gets a new one while the
    /// old one still waits for its withdrawal to be answered.
    pub out_by_source: HashMap<TrackKey, usize>,
    /// Every mid the media server created in this connection, once the
    /// browser has it. Never forgotten, because a section keeps its mid for
    /// the life of the connection, however many tracks it carries.
    pub server_mids: HashSet<Mid>,
    /// Withdrawn sections the browser has confirmed inactive, free for the
    /// next track of their kind. A track that is wanted takes one of these
    /// before the description grows.
    pub idle: Vec<(Mid, MediaKind)>,
    /// The stream id a reused section names in its `a=msid`, which str0m
    /// keeps as it was when the section was created.
    pub restreams: Restreams,
    pub ssrc_mids: Vec<(Ssrc, (Mid, Option<Layer>))>,
    /// The codec order of every section in the browser's last offer.
    pub format_orders: FormatOrders,
    /// The Opus parameters the browser gave each section it publishes, in
    /// its last offer.
    pub opus_params: OpusParams,
    pub addrs: VecDeque<Endpoint>,
    /// Every address a valid check of this browser has come from.
    pub sources: HashSet<Endpoint>,
    /// Trickled candidates handed to the `Rtc` so far.
    pub remote_candidates: usize,
    pub ingress: Counter,
    pub egress: Counter,
    /// What the session may send, and what was turned away.
    pub limits: Limits,
    /// Grows with every completed negotiation, which may move payload types.
    pub generation: u64,
    pub rtc_timeout: Instant,
    /// The earliest time this session is in the timer heap for.
    pub scheduled: Option<Instant>,
}

impl Session {
    pub fn new(
        key: SessionKey,
        id: String,
        user_id: String,
        room: RoomKey,
        rtc: Rtc,
        ufrag: String,
        now: Instant,
    ) -> Session {
        Session {
            key,
            id,
            user_id,
            room,
            rtc,
            ufrag,
            created_at: now,
            transport: Transport {
                ever_connected: false,
                ice: IceConnectionState::New,
                disconnected_since: None,
            },
            negotiation: Negotiation::default(),
            published: Vec::new(),
            outs: Vec::new(),
            waiting_for_space: false,
            subscriptions: HashSet::new(),
            thumbnail_mids: HashSet::new(),
            subscription_order: Vec::new(),
            downlink: Downlink::default(),
            allocation_queued: false,
            out_by_source: HashMap::new(),
            server_mids: HashSet::new(),
            idle: Vec::new(),
            restreams: Vec::new(),
            ssrc_mids: Vec::new(),
            format_orders: Vec::new(),
            opus_params: Vec::new(),
            addrs: VecDeque::new(),
            sources: HashSet::new(),
            remote_candidates: 0,
            ingress: Counter::default(),
            egress: Counter::default(),
            limits: Limits::default(),
            generation: 0,
            rtc_timeout: now,
            scheduled: None,
        }
    }

    /// What the media server sends the browser for a description str0m
    /// wrote: every section in the browser's own codec order, and every
    /// section the browser publishes with the browser's own Opus parameters,
    /// which are what its encoder follows. A section that carries another
    /// track than the one it was made for names that track's stream.
    pub fn describe(&self, sdp: &str) -> String {
        let ordered = sdp_text::order_formats(sdp, &self.format_orders);
        let opus = sdp_text::apply_opus_params(&ordered, &self.opus_params);
        sdp_text::apply_restreams(&opus, &self.restreams)
    }

    pub fn is_server_mid(&self, mid: &str) -> bool {
        self.server_mids.contains(&Mid::from(mid))
    }

    pub fn state_name(&self) -> &'static str {
        if !self.negotiation.remote_described {
            "new"
        } else if !self.transport.ever_connected {
            "connecting"
        } else if self.transport.disconnected_since.is_some() {
            "disconnected"
        } else {
            "connected"
        }
    }

    /// The earliest moment this session needs attention, if any.
    pub fn deadline(&self) -> Option<Instant> {
        let candidates = [
            self.rtc.is_alive().then_some(self.rtc_timeout),
            (!self.transport.ever_connected).then(|| self.created_at + CONNECT_TIMEOUT),
            self.transport
                .disconnected_since
                .map(|since| since + DISCONNECT_TIMEOUT),
            self.negotiation
                .pending
                .as_ref()
                .map(|p| p.sent_at + OFFER_TIMEOUT),
            self.downlink.next_check,
        ];

        let keyframes = self.published.iter().filter_map(|t| t.keyframe.due());
        candidates.into_iter().flatten().chain(keyframes).min()
    }

    /// Whether this receiver's description could take tracks of `kinds` on
    /// top of what it has. It holds every section it was ever given: those
    /// that carry a track or are offered, those whose track is ending, and
    /// the idle ones. A track still waiting for a section, and each of
    /// `kinds`, takes an idle section of its kind first and a new one only
    /// when there is none.
    pub fn fits(&self, kinds: impl IntoIterator<Item = MediaKind>) -> bool {
        let held = self
            .outs
            .iter()
            .filter(|o| o.state != OutState::Want)
            .count()
            + self.idle.len();
        let mut free: Vec<MediaKind> = self.idle.iter().map(|(_, kind)| *kind).collect();
        let waiting = self
            .outs
            .iter()
            .filter(|o| o.state == OutState::Want)
            .map(|o| o.kind);
        let mut new = 0;
        for kind in waiting.chain(kinds) {
            match free.iter().position(|k| *k == kind) {
                Some(at) => {
                    free.swap_remove(at);
                }
                None => new += 1,
            }
        }
        held + new <= super::MAX_OUT_SECTIONS
    }

    pub fn add_out(&mut self, source: TrackKey, kind: MediaKind) -> bool {
        if self.out_by_source.contains_key(&source) {
            return false;
        }
        self.out_by_source.insert(source, self.outs.len());
        self.outs.push(OutSection::new(source, kind));
        true
    }

    pub fn is_subscribed(&self, mid: Mid) -> bool {
        self.subscriptions.contains(&*mid)
    }

    /// Whether packets are forwarded on the receiving section `mid`: the
    /// receiver asks for it, and its downlink has room for it.
    pub fn forwards(&self, mid: Mid) -> bool {
        self.is_subscribed(mid) && !self.downlink.held.contains(&mid)
    }

    /// Every section of the directory that is forwarded now.
    pub fn forwarded_mids(&self) -> HashSet<Mid> {
        self.outs
            .iter()
            .filter_map(OutSection::directory_mid)
            .filter(|mid| self.forwards(*mid))
            .collect()
    }

    /// Withdraws a track from this receiver. Returns whether the browser has
    /// to be told with an offer.
    pub fn withdraw_out(&mut self, source: TrackKey) -> bool {
        let Some(&idx) = self.out_by_source.get(&source) else {
            return false;
        };
        let out = &mut self.outs[idx];
        match out.state {
            OutState::Want => {
                self.remove_outs(|o| o.source == source && o.state == OutState::Want);
                false
            }
            OutState::Offered(mid) => {
                out.state = OutState::OfferedWithdrawn(mid);
                self.out_by_source.remove(&source);
                false
            }
            OutState::Live(mid) => {
                out.state = OutState::Ending(mid);
                self.out_by_source.remove(&source);
                true
            }
            OutState::OfferedWithdrawn(_) | OutState::Ending(_) => false,
        }
    }

    /// Gives the idle section `mid` to the track of `outs[idx]`, whose stream
    /// it names from now on.
    ///
    /// Everything the receiver and its downlink knew of the section belonged
    /// to the track it carried before: a receiver that asked for that track
    /// has not asked for this one, so the section starts out unasked for, and
    /// the clocks of the old track do not run for the new one.
    ///
    /// This is also what keeps the old owner's sinks dark. Nothing is
    /// forwarded on the section until a list that names it arrives after
    /// this offer, and a client sends that list only after it has taken the
    /// offer's directory, which files the section under its new owner. So no
    /// packet of the new track reaches a client that still files the section
    /// under the old one.
    pub fn take_idle(&mut self, mid: Mid, idx: usize, stream: &str) {
        self.idle.retain(|(idle, _)| *idle != mid);
        self.outs[idx].recycled = true;
        self.subscriptions.remove(&*mid);
        self.thumbnail_mids.remove(&*mid);
        self.subscription_order.retain(|m| **m != *mid);
        self.downlink.retain(|held| held != mid);
        self.downlink.dirty = true;
        let stream = stream.to_string();
        match self.restreams.iter_mut().find(|(m, _)| **m == *mid) {
            Some((_, current)) => *current = stream,
            None => self.restreams.push((mid.to_string(), stream)),
        }
    }

    /// Forgets the sections of an offer that will never be answered. A
    /// section it took from the idle ones is idle again: the browser never
    /// heard of its new track. That changes str0m's session, which is a
    /// mutation to drain.
    pub fn roll_back_offer(&mut self) {
        self.negotiation.pending = None;
        let mut parked = Vec::new();
        for out in &mut self.outs {
            match out.state {
                OutState::Offered(mid) => {
                    if std::mem::take(&mut out.recycled) {
                        parked.push((mid, out.kind));
                    }
                    out.state = OutState::Want;
                }
                OutState::OfferedWithdrawn(mid) if out.recycled => parked.push((mid, out.kind)),
                _ => {}
            }
        }
        self.remove_outs(|o| matches!(o.state, OutState::OfferedWithdrawn(_)));
        for (mid, kind) in parked {
            // str0m took the section's new stream and direction when the offer
            // was made. It must read as inactive again, as the browser has it,
            // and must not keep a stream that nothing sends on. The changes of
            // the direction are dropped unmade, so this offers nothing.
            {
                let mut api = self.rtc.direct_api();
                if let Some(ssrc) = api.stream_tx_by_mid(mid, None).map(|t| t.ssrc()) {
                    api.remove_stream_tx(ssrc);
                }
            }
            self.rtc.sdp_api().set_direction(mid, Direction::Inactive);
            self.idle.push((mid, kind));
        }
    }

    /// Frees the withdrawn sections `settled` names, which the browser now
    /// knows are inactive, for the next track of their kind, and forgets
    /// their send streams: str0m would otherwise keep padding on the stream
    /// of one that is inactive. The section gets a stream of its own again
    /// when a track takes it. Returns whether a stream had to go, which is a
    /// mutation to drain.
    pub fn retire_ended(&mut self, settled: impl Fn(Mid) -> bool) -> bool {
        let ended: Vec<(Mid, MediaKind)> = self
            .outs
            .iter()
            .filter_map(|o| match o.state {
                OutState::Ending(mid) if settled(mid) => Some((mid, o.kind)),
                _ => None,
            })
            .collect();
        self.remove_outs(
            |o| matches!(o.state, OutState::Ending(mid) if ended.iter().any(|(m, _)| *m == mid)),
        );
        let mut removed = false;
        for (mid, kind) in ended {
            {
                let mut api = self.rtc.direct_api();
                if let Some(ssrc) = api.stream_tx_by_mid(mid, None).map(|t| t.ssrc()) {
                    removed |= api.remove_stream_tx(ssrc);
                }
            }
            if self.rtc.media(mid).is_some() {
                self.idle.push((mid, kind));
            }
        }
        removed
    }

    pub fn remove_outs(&mut self, remove: impl Fn(&OutSection) -> bool) {
        let before = self.outs.len();
        self.outs.retain(|o| !remove(o));
        if self.outs.len() != before {
            self.out_by_source = self
                .outs
                .iter()
                .enumerate()
                .filter(|(_, o)| o.is_current())
                .map(|(i, o)| (o.source, i))
                .collect();
        }
    }

    pub fn needs_offer(&self) -> bool {
        self.outs
            .iter()
            .any(|o| matches!(o.state, OutState::Want | OutState::Ending(_)))
    }

    /// Routes a remote address to this session. Returns an address that no
    /// longer is, when the session already had as many as it may.
    pub fn remember_addr(&mut self, addr: Endpoint) -> Option<Endpoint> {
        if self.addrs.contains(&addr) {
            return None;
        }
        self.addrs.push_back(addr);
        if self.addrs.len() > MAX_ADDRS {
            self.addrs.pop_front()
        } else {
            None
        }
    }

    pub fn published_mut(&mut self, mid: Mid) -> Option<&mut Published> {
        self.published.iter_mut().find(|t| t.mid == mid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrite_starts_at_the_wire_sequence_number() {
        let mut r = Rewrite::default();
        let ssrc = Ssrc::from(7);

        // A publisher already past several rollovers.
        let first = 5 * 65_536 + 40_000;
        assert_eq!(r.map(ssrc, first, 1000), Some((40_000, 1000)));
        assert_eq!(r.map(ssrc, first + 1, 1960), Some((40_001, 1960)));

        // A gap stays a gap, so the receiver can ask for the packet.
        assert_eq!(r.map(ssrc, first + 5, 5800), Some((40_005, 5800)));
        // The repaired packet lands where it belongs.
        assert_eq!(r.map(ssrc, first + 3, 3880), Some((40_003, 3880)));
    }

    #[test]
    fn rewrite_crosses_a_rollover_without_jumping() {
        let mut r = Rewrite::default();
        let ssrc = Ssrc::from(7);

        assert_eq!(r.map(ssrc, 65_535, 0), Some((65_535, 0)));
        assert_eq!(r.map(ssrc, 65_536, 960), Some((65_536, 960)));
    }

    #[test]
    fn rewrite_drops_what_came_before_the_start() {
        let mut r = Rewrite::default();
        let ssrc = Ssrc::from(7);

        assert_eq!(r.map(ssrc, 65_536 + 2, 0), Some((2, 0)));
        assert_eq!(r.map(ssrc, 65_536 + 1, 0), Some((1, 0)));
        assert_eq!(r.map(ssrc, 65_535, 0), None);
    }

    #[test]
    fn rewrite_continues_across_a_new_ssrc() {
        let mut r = Rewrite::default();

        assert_eq!(r.map(Ssrc::from(1), 100, 9000), Some((100, 9000)));
        assert_eq!(r.map(Ssrc::from(1), 101, 12000), Some((101, 12000)));

        // The new source starts anywhere; the section does not jump.
        assert_eq!(r.map(Ssrc::from(2), 7_000_000, 5), Some((102, 12001)));
        assert_eq!(r.map(Ssrc::from(2), 6_999_999, 0), None);
        assert_eq!(r.map(Ssrc::from(2), 7_000_001, 3005), Some((103, 15001)));
    }

    #[test]
    fn rewrite_compacts_a_pause_but_preserves_loss_and_rtp_time_after_resume() {
        let mut r = Rewrite::default();
        let ssrc = Ssrc::from(1);
        assert_eq!(r.map(ssrc, 100, 9000), Some((100, 9000)));
        r.pause();
        assert_eq!(r.map(ssrc, 99, 6000), None);
        r.skip(ssrc, 150);
        assert_eq!(r.map(ssrc, 149, 156000), None);

        // Fifty source packets were intentionally omitted. Time still advances
        // with the source to keep this track synchronized with unpaused audio.
        assert_eq!(r.map(ssrc, 151, 162000), Some((101, 162000)));
        assert_eq!(r.map(ssrc, 154, 171000), Some((104, 171000)));
        assert_eq!(r.map(ssrc, 153, 168000), Some((103, 168000)));

        // Late packets of the discarded interval must not reuse new numbers.
        assert_eq!(r.map(ssrc, 149, 156000), None);
        r.pause();
        r.pause();
        r.skip(ssrc, 299);
        assert_eq!(r.map(ssrc, 300, 609000), Some((105, 609000)));
    }

    #[test]
    fn rewrite_can_start_paused_or_resume_with_another_ssrc() {
        let mut r = Rewrite::default();
        r.pause();
        assert_eq!(r.map(Ssrc::from(1), 42, 9000), Some((42, 9000)));
        assert_eq!(r.map(Ssrc::from(1), 41, 6000), None);
        r.pause();
        assert_eq!(r.map(Ssrc::from(2), 700, 100), Some((43, 9001)));
        assert_eq!(r.map(Ssrc::from(2), 701, 3100), Some((44, 12001)));
    }

    #[test]
    fn rewrite_drops_what_was_skipped_of_an_ssrc_that_was_replaced_meanwhile() {
        let mut r = Rewrite::default();
        let (old, new) = (Ssrc::from(1), Ssrc::from(2));
        assert_eq!(r.map(old, 100, 9000), Some((100, 9000)));
        r.pause();
        r.skip(old, 150);

        // The publisher moved to a new SSRC while the section was paused.
        assert_eq!(r.map(new, 700, 100), Some((101, 9001)));
        // A late packet of the old one was skipped, and stays skipped.
        assert_eq!(r.map(old, 149, 156_000), None);
        assert_eq!(r.map(new, 701, 3100), Some((102, 12001)));
    }

    #[test]
    fn rewrite_keeps_its_numbering_across_a_pause_that_skipped_nothing() {
        let mut r = Rewrite::default();
        let ssrc = Ssrc::from(1);
        assert_eq!(r.map(ssrc, 100, 9000), Some((100, 9000)));
        assert_eq!(r.map(ssrc, 102, 15000), Some((102, 15000)));
        r.pause();
        assert_eq!(r.map(ssrc, 103, 18000), Some((103, 18000)));
        // The gap from before the pause can still be repaired.
        assert_eq!(r.map(ssrc, 101, 12000), Some((101, 12000)));
    }

    #[test]
    fn keyframe_gate_holds_requests_inside_the_window() {
        let now = Instant::now();
        let mut gate = KeyframeGate::default();

        assert!(gate.can_send(now));
        assert_eq!(gate.due(), None);
        gate.hold(KeyframeRequestKind::Pli);
        assert_eq!(gate.due(), None);
        gate.pending = None;

        gate.last = Some(now);
        assert!(!gate.can_send(now + KEYFRAME_INTERVAL / 2));
        gate.hold(KeyframeRequestKind::Pli);
        gate.hold(KeyframeRequestKind::Fir);
        gate.hold(KeyframeRequestKind::Pli);
        assert_eq!(gate.pending, Some(KeyframeRequestKind::Fir));
        assert_eq!(gate.due(), Some(now + KEYFRAME_INTERVAL));
        assert!(gate.can_send(now + KEYFRAME_INTERVAL));
    }
}
