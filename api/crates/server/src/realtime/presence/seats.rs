//! Who sits in which voice room, and the media session each seat holds.
//!
//! A voice seat belongs to one concrete connection, not to the account: a
//! newer tab takes the seat of an older one, because the signalling protocol
//! names peers by account and cannot hold two of them.
//!
//! A seat also holds the session the media server keeps for it, and the
//! session lives exactly as long as the seat stays in its room. That rule is
//! kept here and not by the hub methods and routes that move people around:
//! every change of a seat goes through [`set_seat`], which hands the session it
//! ends to the queue of closes. A new way of ending a seat therefore cannot
//! forget to stop the media.
//!
//! The seats live in the same state as the connections, under the same lock,
//! so that ending a connection frees its seat in one step: nothing can take a
//! seat for a connection that is going away.

use std::collections::HashSet;
use std::collections::VecDeque;

use super::{PresenceTracker, State};
use crate::values::Id;

/// More closes than this only pile up while the control API refuses every
/// request, and then the oldest are the least likely to still matter.
pub const MAX_PENDING_CLOSES: usize = 4096;

/// A session of the media server: its opaque id and the run of the media
/// server that created it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaSession {
    pub session_id: String,
    pub epoch: String,
}

/// A media session and the seat that holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeatedMediaSession {
    pub connection_id: String,
    pub channel: Id,
    pub session: MediaSession,
}

/// A seat that ended: whose it was and in which room.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VoiceDeparture {
    pub user: Id,
    pub channel: Id,
}

/// One person in a room, as the room's occupancy lists them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VoiceParticipant {
    pub user: Id,
    pub muted: bool,
    /// They share a screen, by their own state.
    pub sharing: bool,
    /// Their camera is on, by their own state.
    pub camera: bool,
}

/// What a connection's own state says about it. A member left out says
/// nothing, and the seat keeps what it had.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct VoiceSeatState {
    pub muted: Option<bool>,
    pub sharing: Option<bool>,
    pub camera: Option<bool>,
}

/// One ordered occupancy frame of a room. The lookups and sends that follow
/// run outside the lock and may finish out of order, so the sequence lets a
/// client drop a stale frame instead of tearing down live peers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoiceSnapshot {
    pub sequence: i64,
    pub participants: Vec<VoiceParticipant>,
}

/// A connection that lost its seat to a newer one of the same account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoiceDisplacement {
    pub connection_id: String,
    pub channel: Id,
}

/// How a join went. `admitted` is false for a newcomer the room may not take,
/// `accepted` for one a full room turns away.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoiceJoin {
    pub previous_channel: Option<Id>,
    pub displaced: Vec<VoiceDisplacement>,
    pub accepted: bool,
    pub admitted: bool,
}

/// What a move of somebody's seats between rooms did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoiceMove {
    pub connection_ids: Vec<String>,
    pub moved: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Seat {
    user: Id,
    channel: Id,
    muted: bool,
    sharing: bool,
    camera: bool,
    media: Option<MediaSession>,
}

/// The one way a seat is written, `None` meaning that it ends. A media
/// session the seat no longer holds afterwards, because the seat ended, moved
/// to another room or took a newer session, is queued to be closed.
fn set_seat(state: &mut State, connection: &str, next: Option<Seat>) {
    let current = state.seats.get(connection).cloned();
    let mut next = next;
    if let Some(next) = next.as_mut()
        && Some(next.channel) != current.as_ref().map(|seat| seat.channel)
    {
        // A session belongs to one room; the client opens a new one in the next.
        next.media = None;
    }

    if let Some(media) = current.and_then(|seat| seat.media)
        && next.as_ref().and_then(|seat| seat.media.as_ref()) != Some(&media)
    {
        queue_close(state, media);
    }

    match next {
        Some(seat) => {
            state.seats.insert(connection.to_string(), seat);
        }
        None => {
            state.seats.remove(connection);
        }
    }
}

fn queue_close(state: &mut State, session: MediaSession) {
    if state.closes.len() >= MAX_PENDING_CLOSES
        && let Some(dropped) = state.closes.pop_front()
    {
        tracing::warn!(
            session = %dropped.session_id,
            "too many media sessions wait to be closed; the oldest is no longer tried"
        );
    }
    state.closes.push_back(session);
    state.closes_added.notify_one();
}

/// Frees the seat of a connection and says whose it was.
pub(super) fn take_seat(state: &mut State, connection: &str) -> Option<VoiceDeparture> {
    let seat = state.seats.get(connection)?.clone();
    set_seat(state, connection, None);
    Some(VoiceDeparture {
        user: seat.user,
        channel: seat.channel,
    })
}

fn seat_connections(state: &State, wanted: impl Fn(&Seat) -> bool) -> Vec<String> {
    state
        .seats
        .iter()
        .filter(|(_, seat)| wanted(seat))
        .map(|(connection, _)| connection.clone())
        .collect()
}

fn occupants(state: &State, channel: Id) -> Vec<VoiceParticipant> {
    let mut found: Vec<VoiceParticipant> = state
        .seats
        .values()
        .filter(|seat| seat.channel == channel)
        .map(|seat| VoiceParticipant {
            user: seat.user,
            muted: seat.muted,
            sharing: seat.sharing,
            camera: seat.camera,
        })
        .collect();
    found.sort_by_key(|participant| participant.user);
    found
}

impl PresenceTracker {
    /// Seats this connection in a voice room, displacing the account's other
    /// connections wherever they sat. A room at its `user_limit` (0 means
    /// unlimited) refuses newcomers, but not an account already in it. Without
    /// `may_take_seat` only a connection that already sits in the room, as one
    /// a moderator moved there does, keeps its seat; the decision and the seat
    /// are taken under one lock, so a seat lost in between cannot be taken
    /// again without it. `None` for a connection that is not connected.
    pub fn join_voice(
        &self,
        connection: &str,
        user: Id,
        channel: Id,
        user_limit: i64,
        may_take_seat: bool,
    ) -> Option<VoiceJoin> {
        let mut state = self.state();
        if state.user_by_connection.get(connection) != Some(&user) {
            return None;
        }

        let current = state.seats.get(connection).cloned();
        if !may_take_seat && current.as_ref().map(|seat| seat.channel) != Some(channel) {
            return Some(VoiceJoin {
                previous_channel: None,
                displaced: Vec::new(),
                accepted: false,
                admitted: false,
            });
        }

        let people: HashSet<Id> = state
            .seats
            .values()
            .filter(|seat| seat.channel == channel)
            .map(|seat| seat.user)
            .collect();
        if user_limit > 0
            && i64::try_from(people.len()).unwrap_or(i64::MAX) >= user_limit
            && !people.contains(&user)
        {
            return Some(VoiceJoin {
                previous_channel: None,
                displaced: Vec::new(),
                accepted: false,
                admitted: true,
            });
        }

        let displaced: Vec<VoiceDisplacement> = state
            .seats
            .iter()
            .filter(|(other, seat)| other.as_str() != connection && seat.user == user)
            .map(|(other, seat)| VoiceDisplacement {
                connection_id: other.clone(),
                channel: seat.channel,
            })
            .collect();
        for seat in &displaced {
            set_seat(&mut state, &seat.connection_id, None);
        }

        // Joining the room the connection already sits in keeps the seat as it
        // is, its mute state and its media session included.
        if current.as_ref().is_none_or(|seat| seat.channel != channel) {
            let seat = Seat {
                user,
                channel,
                muted: false,
                sharing: false,
                camera: false,
                media: None,
            };
            set_seat(&mut state, connection, Some(seat));
        }
        Some(VoiceJoin {
            previous_channel: current
                .map(|seat| seat.channel)
                .filter(|previous| *previous != channel),
            displaced,
            accepted: true,
            admitted: true,
        })
    }

    pub fn leave_voice(&self, connection: &str) -> Option<VoiceDeparture> {
        take_seat(&mut self.state(), connection)
    }

    /// The seat a connection that has just been disconnected held, once. The
    /// seat itself was freed with the connection, under the same lock.
    pub fn take_departure(&self, connection: &str) -> Option<VoiceDeparture> {
        self.state().departures.remove(connection)
    }

    /// Frees every seat the account holds in the given server channels. Direct
    /// calls are not touched: being removed from a server ends no friendship.
    pub fn remove_user_from_voice_channels(
        &self,
        user: Id,
        channels: &HashSet<Id>,
    ) -> Vec<VoiceDeparture> {
        let mut state = self.state();
        let connections = seat_connections(&state, |seat| {
            seat.user == user && channels.contains(&seat.channel)
        });
        connections
            .iter()
            .filter_map(|connection| take_seat(&mut state, connection))
            .collect()
    }

    /// Moves every seat the account holds in one room to another. Connection
    /// and mute state stay as they are; the client is told to rebuild its
    /// peers.
    pub fn move_user_between_voice_channels(&self, user: Id, source: Id, target: Id) -> VoiceMove {
        let mut state = self.state();
        let connections =
            seat_connections(&state, |seat| seat.user == user && seat.channel == source);
        for connection in &connections {
            if let Some(seat) = state.seats.get(connection).cloned() {
                set_seat(
                    &mut state,
                    connection,
                    Some(Seat {
                        channel: target,
                        ..seat
                    }),
                );
            }
        }
        VoiceMove {
            moved: !connections.is_empty(),
            connection_ids: connections,
        }
    }

    pub fn is_connection_in_voice(&self, connection: &str, channel: Id) -> bool {
        self.state()
            .seats
            .get(connection)
            .is_some_and(|seat| seat.channel == channel)
    }

    pub fn set_voice_muted(&self, connection: &str, muted: bool) {
        self.set_voice_state(
            connection,
            VoiceSeatState {
                muted: Some(muted),
                ..VoiceSeatState::default()
            },
        );
    }

    /// Takes what the connection's own state says about its seat; whether
    /// anything of the seat changed.
    pub fn set_voice_state(&self, connection: &str, said: VoiceSeatState) -> bool {
        let mut state = self.state();
        let Some(seat) = state.seats.get(connection).cloned() else {
            return false;
        };
        let next = Seat {
            muted: said.muted.unwrap_or(seat.muted),
            sharing: said.sharing.unwrap_or(seat.sharing),
            camera: said.camera.unwrap_or(seat.camera),
            ..seat.clone()
        };
        if next == seat {
            return false;
        }
        set_seat(&mut state, connection, Some(next));
        true
    }

    /// The account's connections that actually sit in this room.
    pub fn voice_connections_of(&self, user: Id, channel: Id) -> Vec<String> {
        seat_connections(&self.state(), |seat| {
            seat.user == user && seat.channel == channel
        })
    }

    /// Every connection that sits in this room.
    pub fn voice_connections(&self, channel: Id) -> Vec<String> {
        seat_connections(&self.state(), |seat| seat.channel == channel)
    }

    /// Every room with at least one seat taken.
    pub fn active_voice_rooms(&self) -> Vec<Id> {
        let state = self.state();
        let mut rooms: Vec<Id> = Vec::new();
        for seat in state.seats.values() {
            if !rooms.contains(&seat.channel) {
                rooms.push(seat.channel);
            }
        }
        rooms
    }

    /// Empties a room, as when a two-person call is declined; returns the
    /// connections that sat in it.
    pub fn end_voice_room(&self, channel: Id) -> Vec<String> {
        let mut state = self.state();
        let connections = seat_connections(&state, |seat| seat.channel == channel);
        for connection in &connections {
            set_seat(&mut state, connection, None);
        }
        connections
    }

    /// Gives the connection's seat in `channel` its media session, closing one
    /// it held before. False when the connection no longer sits there, which
    /// happens when the seat ended while the media server was creating the
    /// session; the caller then closes the session.
    pub fn attach_media_session(
        &self,
        connection: &str,
        channel: Id,
        session: MediaSession,
    ) -> bool {
        let mut state = self.state();
        let Some(seat) = state.seats.get(connection).cloned() else {
            return false;
        };
        if seat.channel != channel {
            return false;
        }
        set_seat(
            &mut state,
            connection,
            Some(Seat {
                media: Some(session),
                ..seat
            }),
        );
        true
    }

    /// Closes the media session the connection's seat holds, if it holds one;
    /// the seat stays.
    pub fn close_media_session(&self, connection: &str) {
        let mut state = self.state();
        if let Some(seat) = state.seats.get(connection).cloned()
            && seat.media.is_some()
        {
            set_seat(
                &mut state,
                connection,
                Some(Seat {
                    media: None,
                    ..seat
                }),
            );
        }
    }

    /// The media session of the connection's seat in this room; none without a
    /// seat there or without a session.
    pub fn media_session_of(&self, connection: &str, channel: Id) -> Option<MediaSession> {
        self.state()
            .seats
            .get(connection)
            .filter(|seat| seat.channel == channel)
            .and_then(|seat| seat.media.clone())
    }

    /// The seat that holds a media session, which is where its signals go.
    pub fn find_media_session(&self, session_id: &str) -> Option<SeatedMediaSession> {
        let state = self.state();
        state.seats.iter().find_map(|(connection, seat)| {
            seat.media
                .as_ref()
                .filter(|media| media.session_id == session_id)
                .map(|media| SeatedMediaSession {
                    connection_id: connection.clone(),
                    channel: seat.channel,
                    session: media.clone(),
                })
        })
    }

    /// Takes a media session off its seat without closing it, because the
    /// media plane already ended it. None when no seat holds it any more: of
    /// two paths that learn about the same end, only the first one gets it and
    /// tells the browser.
    pub fn forget_media_session(&self, session_id: &str) -> Option<SeatedMediaSession> {
        self.forget_media_sessions(|media| media.session_id == session_id)
            .into_iter()
            .next()
    }

    /// Takes every media session `ended` picks off its seat without closing it.
    pub fn forget_media_sessions(
        &self,
        ended: impl Fn(&MediaSession) -> bool,
    ) -> Vec<SeatedMediaSession> {
        let mut state = self.state();
        let forgotten: Vec<SeatedMediaSession> = state
            .seats
            .iter()
            .filter_map(|(connection, seat)| {
                seat.media
                    .as_ref()
                    .filter(|media| ended(media))
                    .map(|media| SeatedMediaSession {
                        connection_id: connection.clone(),
                        channel: seat.channel,
                        session: media.clone(),
                    })
            })
            .collect();
        // Written directly rather than through `set_seat`: the session is gone
        // from the media server, and closing it there again would only be one
        // more request to a process that may be down.
        for session in &forgotten {
            if let Some(seat) = state.seats.get_mut(&session.connection_id) {
                seat.media = None;
            }
        }
        forgotten
    }

    /// One row per account, which a join keeps true even when two tabs race.
    pub fn voice_users(&self, channel: Id) -> Vec<VoiceParticipant> {
        occupants(&self.state(), channel)
    }

    /// One ordered occupancy frame of a room.
    pub fn voice_snapshot(&self, channel: Id) -> VoiceSnapshot {
        let mut state = self.state();
        state.snapshot_sequence += 1;
        VoiceSnapshot {
            sequence: state.snapshot_sequence,
            participants: occupants(&state, channel),
        }
    }

    /// Has the media server told to close a session no seat holds, as one
    /// that was created while its seat ended.
    pub fn close_session(&self, session: MediaSession) {
        queue_close(&mut self.state(), session);
    }

    /// The oldest media session that waits to be closed, left in the queue
    /// until [`finish_close`](Self::finish_close) says it is done.
    pub fn next_close(&self) -> Option<MediaSession> {
        self.state().closes.front().cloned()
    }

    /// Takes a session off the queue once the media server has closed it. A
    /// queue that dropped it meanwhile, to make room, is left alone.
    pub fn finish_close(&self, session: &MediaSession) {
        let mut state = self.state();
        if state.closes.front() == Some(session) {
            state.closes.pop_front();
        }
    }

    /// How many sessions wait to be closed.
    pub fn pending_closes(&self) -> usize {
        self.state().closes.len()
    }

    /// Completes when something has been queued to close since the last time it
    /// completed. A close queued before the wait began is not missed.
    pub async fn closes_queued(&self) {
        let added = self.state().closes_added.clone();
        added.notified().await;
    }
}

/// What the state holds of seats, for [`State`]'s declaration.
pub(super) type Closes = VecDeque<MediaSession>;

#[cfg(test)]
mod tests {
    use super::*;

    fn maja() -> Id {
        Id::parse("30000000-0000-0000-0000-000000000001").unwrap()
    }

    fn ada() -> Id {
        Id::parse("30000000-0000-0000-0000-000000000002").unwrap()
    }

    fn salon() -> Id {
        Id::parse("20000000-0000-0000-0000-000000000002").unwrap()
    }

    fn night_shift() -> Id {
        Id::parse("20000000-0000-0000-0000-000000000005").unwrap()
    }

    fn media(session: &str) -> MediaSession {
        MediaSession {
            session_id: session.to_string(),
            epoch: "epoch-1".to_string(),
        }
    }

    fn closed(tracker: &PresenceTracker) -> Vec<String> {
        let mut found = Vec::new();
        while let Some(session) = tracker.next_close() {
            tracker.finish_close(&session);
            found.push(session.session_id);
        }
        found
    }

    fn join(tracker: &PresenceTracker, connection: &str, user: Id, room: Id) -> VoiceJoin {
        tracker.join_voice(connection, user, room, 0, true).unwrap()
    }

    #[test]
    fn a_voice_seat_belongs_to_the_connection_that_joined_it() {
        let tracker = PresenceTracker::default();
        tracker.connect("maja-voice", maja(), true);
        tracker.connect("maja-chat", maja(), true);
        tracker.connect("ada-voice", ada(), true);

        join(&tracker, "maja-voice", maja(), salon());
        join(&tracker, "ada-voice", ada(), salon());
        tracker.set_voice_muted("maja-voice", true);

        assert_eq!(tracker.voice_users(salon()).len(), 2);
        assert_eq!(tracker.voice_connections_of(maja(), salon()).len(), 1);
        assert!(!tracker.is_connection_in_voice("maja-chat", salon()));

        // The seat goes with the connection, and the account stays online
        // through its other one.
        let offline = tracker.disconnect("maja-voice");

        assert!(!offline);
        assert_eq!(
            tracker.take_departure("maja-voice"),
            Some(VoiceDeparture {
                user: maja(),
                channel: salon()
            })
        );
        assert_eq!(tracker.take_departure("maja-voice"), None);
        assert!(
            tracker
                .voice_users(salon())
                .iter()
                .all(|p| p.user != maja())
        );
        assert_eq!(tracker.presence_of(maja()), "online");
    }

    #[test]
    fn a_connection_that_never_sat_leaves_no_departure() {
        let tracker = PresenceTracker::default();
        tracker.connect("maja-chat", maja(), true);

        tracker.disconnect("maja-chat");

        assert_eq!(tracker.take_departure("maja-chat"), None);
    }

    #[test]
    fn only_a_connected_account_can_take_a_seat() {
        let tracker = PresenceTracker::default();
        tracker.connect("maja", maja(), true);

        assert!(
            tracker
                .join_voice("nobody", maja(), salon(), 0, true)
                .is_none()
        );
        assert!(
            tracker
                .join_voice("maja", ada(), salon(), 0, true)
                .is_none()
        );
        assert!(tracker.voice_users(salon()).is_empty());
    }

    #[test]
    fn snapshots_carry_a_process_wide_increasing_sequence() {
        let tracker = PresenceTracker::default();
        tracker.connect("maja-voice", maja(), true);
        tracker.connect("ada-voice", ada(), true);
        join(&tracker, "maja-voice", maja(), salon());

        let first = tracker.voice_snapshot(salon());
        join(&tracker, "ada-voice", ada(), salon());
        let second = tracker.voice_snapshot(salon());
        let other = tracker.voice_snapshot(night_shift());

        assert!(first.sequence < second.sequence);
        assert!(second.sequence < other.sequence);
        assert_eq!(first.participants.len(), 1);
        assert_eq!(second.participants.len(), 2);
        assert!(other.participants.is_empty());
    }

    #[test]
    fn participants_are_listed_in_the_order_of_their_ids() {
        let tracker = PresenceTracker::default();
        tracker.connect("ada", ada(), true);
        tracker.connect("maja", maja(), true);
        join(&tracker, "maja", maja(), salon());
        join(&tracker, "ada", ada(), salon());

        let users: Vec<Id> = tracker
            .voice_snapshot(salon())
            .participants
            .iter()
            .map(|p| p.user)
            .collect();

        assert_eq!(users, [maja(), ada()]);
    }

    #[test]
    fn a_newer_connection_displaces_the_older_one_of_the_same_account() {
        let tracker = PresenceTracker::default();
        tracker.connect("maja-one", maja(), true);
        tracker.connect("maja-two", maja(), true);
        join(&tracker, "maja-one", maja(), salon());
        tracker.set_voice_muted("maja-one", true);

        let joined = join(&tracker, "maja-two", maja(), salon());

        let users = tracker.voice_users(salon());
        assert_eq!(users.len(), 1);
        assert!(!users[0].muted);
        assert_eq!(
            joined.displaced,
            [VoiceDisplacement {
                connection_id: "maja-one".to_string(),
                channel: salon()
            }]
        );
        assert!(!tracker.is_connection_in_voice("maja-one", salon()));
        assert!(tracker.is_connection_in_voice("maja-two", salon()));
    }

    #[test]
    fn the_user_limit_is_checked_in_the_same_step_as_the_seat() {
        let tracker = PresenceTracker::default();
        tracker.connect("maja-voice", maja(), true);
        tracker.connect("ada-voice", ada(), true);

        assert!(
            tracker
                .join_voice("maja-voice", maja(), salon(), 1, true)
                .unwrap()
                .accepted
        );
        let refused = tracker
            .join_voice("ada-voice", ada(), salon(), 1, true)
            .unwrap();
        assert!(refused.admitted && !refused.accepted);
        // Somebody already in the room is not a newcomer.
        assert!(
            tracker
                .join_voice("maja-voice", maja(), salon(), 1, true)
                .unwrap()
                .accepted
        );
        let users = tracker.voice_users(salon());
        assert_eq!(users.len(), 1);
        assert_eq!(users[0].user, maja());
    }

    #[test]
    fn without_the_right_to_take_a_seat_only_a_seat_already_in_the_room_is_kept() {
        let tracker = PresenceTracker::default();
        tracker.connect("ada", ada(), true);

        let refused = tracker.join_voice("ada", ada(), salon(), 0, false).unwrap();
        assert!(!refused.admitted);
        assert!(tracker.voice_users(salon()).is_empty());

        join(&tracker, "ada", ada(), night_shift());
        tracker.move_user_between_voice_channels(ada(), night_shift(), salon());
        let kept = tracker.join_voice("ada", ada(), salon(), 0, false).unwrap();
        assert!(kept.admitted && kept.accepted);
        assert_eq!(kept.previous_channel, None);

        // A seat elsewhere is not a way into this room.
        let elsewhere = tracker
            .join_voice("ada", ada(), night_shift(), 0, false)
            .unwrap();
        assert!(!elsewhere.admitted);
        assert!(tracker.is_connection_in_voice("ada", salon()));
    }

    #[test]
    fn joining_another_room_names_the_one_that_was_left() {
        let tracker = PresenceTracker::default();
        tracker.connect("maja", maja(), true);
        join(&tracker, "maja", maja(), salon());

        let moved = join(&tracker, "maja", maja(), night_shift());
        let again = join(&tracker, "maja", maja(), night_shift());

        assert_eq!(moved.previous_channel, Some(salon()));
        assert_eq!(again.previous_channel, None);
        assert!(tracker.voice_users(salon()).is_empty());
    }

    #[test]
    fn a_moderators_move_keeps_the_connection_and_the_mute_state() {
        let tracker = PresenceTracker::default();
        tracker.connect("ada-voice", ada(), true);
        join(&tracker, "ada-voice", ada(), salon());
        tracker.set_voice_muted("ada-voice", true);

        let result = tracker.move_user_between_voice_channels(ada(), salon(), night_shift());

        assert!(result.moved);
        assert_eq!(result.connection_ids, ["ada-voice"]);
        assert!(tracker.voice_users(salon()).is_empty());
        let users = tracker.voice_users(night_shift());
        assert_eq!(users.len(), 1);
        assert!(users[0].muted);
        let nothing = tracker.move_user_between_voice_channels(ada(), salon(), night_shift());
        assert!(!nothing.moved && nothing.connection_ids.is_empty());
    }

    #[test]
    fn a_media_session_needs_the_seat_of_its_room() {
        let tracker = PresenceTracker::default();
        tracker.connect("maja-voice", maja(), true);

        assert!(!tracker.attach_media_session("maja-voice", salon(), media("unseated")));
        join(&tracker, "maja-voice", maja(), salon());
        assert!(!tracker.attach_media_session("maja-voice", night_shift(), media("elsewhere")));
        assert!(tracker.attach_media_session("maja-voice", salon(), media("first")));

        assert_eq!(
            tracker.media_session_of("maja-voice", salon()),
            Some(media("first"))
        );
        assert_eq!(tracker.media_session_of("maja-voice", night_shift()), None);
        assert_eq!(
            tracker.find_media_session("first"),
            Some(SeatedMediaSession {
                connection_id: "maja-voice".to_string(),
                channel: salon(),
                session: media("first")
            })
        );
        assert!(closed(&tracker).is_empty());

        // A newer session of the same seat replaces the older one, which closes.
        assert!(tracker.attach_media_session("maja-voice", salon(), media("second")));
        assert_eq!(closed(&tracker), ["first"]);
        assert_eq!(tracker.find_media_session("first"), None);
    }

    #[test]
    fn every_change_of_the_seat_hands_its_media_session_to_the_closer() {
        type Change = fn(&PresenceTracker);
        let changes: [(&str, Change); 8] = [
            ("leave", |t| {
                t.leave_voice("maja-voice");
            }),
            ("disconnect", |t| {
                t.disconnect("maja-voice");
            }),
            ("another room", |t| {
                join(t, "maja-voice", maja(), night_shift());
            }),
            ("newer tab", |t| {
                join(t, "maja-other", maja(), salon());
            }),
            ("removed from the channel", |t| {
                t.remove_user_from_voice_channels(maja(), &HashSet::from([salon()]));
            }),
            ("moved", |t| {
                t.move_user_between_voice_channels(maja(), salon(), night_shift());
            }),
            ("room ended", |t| {
                t.end_voice_room(salon());
            }),
            ("session closed", |t| t.close_media_session("maja-voice")),
        ];
        for (name, change) in changes {
            let tracker = PresenceTracker::default();
            tracker.connect("maja-voice", maja(), true);
            tracker.connect("maja-other", maja(), true);
            join(&tracker, "maja-voice", maja(), salon());
            tracker.attach_media_session("maja-voice", salon(), media("seated"));

            change(&tracker);

            assert_eq!(closed(&tracker), ["seated"], "{name}");
            assert_eq!(tracker.find_media_session("seated"), None, "{name}");
            assert_eq!(
                tracker.media_session_of("maja-voice", night_shift()),
                None,
                "{name}"
            );
        }
    }

    #[test]
    fn changes_that_keep_the_seat_keep_its_media_session() {
        let tracker = PresenceTracker::default();
        tracker.connect("maja-voice", maja(), true);
        tracker.connect("ada-voice", ada(), true);
        join(&tracker, "maja-voice", maja(), salon());
        tracker.attach_media_session("maja-voice", salon(), media("seated"));

        tracker.set_voice_muted("maja-voice", true);
        join(&tracker, "maja-voice", maja(), salon());
        join(&tracker, "ada-voice", ada(), salon());
        tracker.move_user_between_voice_channels(ada(), salon(), night_shift());
        tracker.remove_user_from_voice_channels(maja(), &HashSet::from([night_shift()]));

        assert!(closed(&tracker).is_empty());
        assert_eq!(
            tracker
                .media_session_of("maja-voice", salon())
                .map(|session| session.session_id),
            Some("seated".to_string())
        );
        let users = tracker.voice_users(salon());
        assert_eq!(users.len(), 1);
        assert!(users[0].muted);
    }

    #[test]
    fn a_session_the_media_plane_ended_is_forgotten_once_and_not_closed_again() {
        let tracker = PresenceTracker::default();
        tracker.connect("maja-voice", maja(), true);
        tracker.connect("ada-voice", ada(), true);
        join(&tracker, "maja-voice", maja(), salon());
        join(&tracker, "ada-voice", ada(), salon());
        let old = MediaSession {
            session_id: "old-run".to_string(),
            epoch: "epoch-1".to_string(),
        };
        let new = MediaSession {
            session_id: "new-run".to_string(),
            epoch: "epoch-2".to_string(),
        };
        tracker.attach_media_session("maja-voice", salon(), old.clone());
        tracker.attach_media_session("ada-voice", salon(), new);

        let lost = tracker.forget_media_session("old-run");
        let again = tracker.forget_media_session("old-run");
        let restarted = tracker.forget_media_sessions(|media| media.epoch != "epoch-2");

        assert_eq!(
            lost,
            Some(SeatedMediaSession {
                connection_id: "maja-voice".to_string(),
                channel: salon(),
                session: old
            })
        );
        assert_eq!(again, None);
        assert!(restarted.is_empty());
        assert!(tracker.is_connection_in_voice("maja-voice", salon()));
        assert_eq!(
            tracker
                .media_session_of("ada-voice", salon())
                .map(|session| session.session_id),
            Some("new-run".to_string())
        );

        // The seat that lost its session ends later without a second close.
        tracker.leave_voice("maja-voice");
        assert!(closed(&tracker).is_empty());
    }

    #[test]
    fn the_queue_of_closes_drops_the_oldest_when_it_is_full() {
        let tracker = PresenceTracker::default();
        tracker.connect("maja", maja(), true);
        for number in 0..=MAX_PENDING_CLOSES {
            join(&tracker, "maja", maja(), salon());
            tracker.attach_media_session("maja", salon(), media(&format!("session-{number}")));
            tracker.close_media_session("maja");
        }

        assert_eq!(tracker.pending_closes(), MAX_PENDING_CLOSES);
        assert_eq!(tracker.next_close(), Some(media("session-1")));
    }

    #[test]
    fn a_session_that_is_finished_leaves_the_queue_only_when_it_is_the_oldest() {
        let tracker = PresenceTracker::default();
        tracker.connect("maja", maja(), true);
        join(&tracker, "maja", maja(), salon());
        tracker.attach_media_session("maja", salon(), media("a"));
        tracker.close_media_session("maja");
        tracker.attach_media_session("maja", salon(), media("b"));
        tracker.close_media_session("maja");

        tracker.finish_close(&media("b"));
        assert_eq!(tracker.pending_closes(), 2);
        tracker.finish_close(&media("a"));
        assert_eq!(tracker.next_close(), Some(media("b")));
    }

    #[tokio::test]
    async fn a_close_queued_before_the_wait_is_not_missed() {
        let tracker = PresenceTracker::default();
        tracker.connect("maja", maja(), true);
        join(&tracker, "maja", maja(), salon());
        tracker.attach_media_session("maja", salon(), media("a"));
        tracker.leave_voice("maja");

        tokio::time::timeout(std::time::Duration::from_secs(5), tracker.closes_queued())
            .await
            .expect("the close was announced");
    }
}
