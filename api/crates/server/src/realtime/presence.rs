//! Who is connected, and what each account tells others about itself.
//!
//! Presence belongs to an account, not to a connection: the status an account
//! chose is shared by all its tabs, the account is online while it has any
//! connection and offline when the last one is gone, and what it chose
//! outlives its connections, so that a reconnect announces the status it had.
//! The state lives in this process only.
//!
//! The voice seats of the connections are kept here too, under the same lock,
//! so that a connection that ends frees its seat in the same step; see
//! [`seats`].

mod seats;

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use tokio::sync::Notify;

use self::seats::{Closes, Seat};
pub use self::seats::{
    MAX_PENDING_CLOSES, MediaSession, SeatedMediaSession, VoiceDeparture, VoiceDisplacement,
    VoiceJoin, VoiceMove, VoiceParticipant, VoiceSeatState, VoiceSnapshot,
};

use crate::values::Id;

pub const ONLINE: &str = "online";
pub const IDLE: &str = "idle";
pub const DO_NOT_DISTURB: &str = "dnd";
pub const OFFLINE: &str = "offline";

/// Whether `value` is a status an account may choose. Offline is one: an
/// account may say so while connected.
pub fn is_supported(value: &str) -> bool {
    [ONLINE, IDLE, DO_NOT_DISTURB, OFFLINE].contains(&value)
}

#[derive(Default)]
struct State {
    user_by_connection: HashMap<String, Id>,
    connections_by_user: HashMap<Id, HashSet<String>>,
    chosen: HashMap<Id, String>,
    visible: HashMap<Id, bool>,
    /// What a development seed says of accounts that have no connection.
    seeded: HashMap<Id, String>,
    /// The voice seat each connection holds.
    seats: HashMap<String, Seat>,
    cameras: HashMap<(String, Id), seats::CameraSeat>,
    /// The seats of connections that ended, until the hub's hook has told the
    /// rooms they left.
    departures: HashMap<String, VoiceDeparture>,
    snapshot_sequence: i64,
    /// Media sessions whose seat ended and that the media server has yet to
    /// be told to close.
    closes: Closes,
    closes_added: Arc<Notify>,
}

impl State {
    fn chosen(&self, user: Id) -> &str {
        self.chosen.get(&user).map_or(ONLINE, String::as_str)
    }

    fn presence(&self, user: Id) -> String {
        if self.connections_by_user.contains_key(&user) {
            self.chosen(user).to_string()
        } else {
            self.seeded
                .get(&user)
                .cloned()
                .unwrap_or_else(|| OFFLINE.to_string())
        }
    }
}

#[derive(Default)]
pub struct PresenceTracker {
    state: Mutex<State>,
}

impl PresenceTracker {
    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Open connections of every account, on every server.
    pub fn connection_count(&self) -> usize {
        self.state().user_by_connection.len()
    }

    /// Registers a connection; true means the account just came online.
    pub fn connect(&self, connection_id: &str, user: Id, show_presence: bool) -> bool {
        let mut state = self.state();
        if let Some(owner) = state.user_by_connection.get(connection_id) {
            // A connection never changes hands, so there is nothing to
            // announce if it is registered again.
            debug_assert_eq!(*owner, user);
            return false;
        }
        state
            .user_by_connection
            .insert(connection_id.to_string(), user);
        let connections = state.connections_by_user.entry(user).or_default();
        connections.insert(connection_id.to_string());
        let first = connections.len() == 1;
        state.seeded.remove(&user);
        state
            .chosen
            .entry(user)
            .or_insert_with(|| ONLINE.to_string());
        state.visible.insert(user, show_presence);
        first
    }

    /// Gives accounts that have no live connection a status of their own, as
    /// a development seed does so that a seeded instance does not read as a
    /// room of people who all signed off. Connecting promotes the account to a
    /// real session and drops the seeded status.
    pub fn seed(&self, presences: impl IntoIterator<Item = (Id, String)>) {
        let mut state = self.state();
        for (user, presence) in presences {
            if is_supported(&presence)
                && presence != OFFLINE
                && !state.connections_by_user.contains_key(&user)
            {
                state.seeded.insert(user, presence);
            }
        }
    }

    /// Forgets a connection. True when it was the account's last one.
    pub fn disconnect(&self, connection_id: &str) -> bool {
        let mut state = self.state();
        let Some(user) = state.user_by_connection.remove(connection_id) else {
            return false;
        };
        seats::close_cameras(&mut state, connection_id, None);
        if let Some(departure) = seats::take_seat(&mut state, connection_id) {
            state
                .departures
                .insert(connection_id.to_string(), departure);
        }
        let Some(connections) = state.connections_by_user.get_mut(&user) else {
            return false;
        };
        connections.remove(connection_id);
        let offline = connections.is_empty();
        if offline {
            state.connections_by_user.remove(&user);
        }
        offline
    }

    /// Sets the status a connected account advertises on all its connections.
    /// True means observers need to hear about it: the account is connected
    /// and the status is not the one it already had.
    pub fn set_presence(&self, user: Id, value: &str) -> bool {
        let mut state = self.state();
        if !state.connections_by_user.contains_key(&user) {
            return false;
        }
        let changed = state.chosen(user) != value;
        state.chosen.insert(user, value.to_string());
        changed
    }

    /// Changes whether other accounts may see the real status; true when it
    /// matters now.
    pub fn set_visibility(&self, user: Id, visible: bool) -> bool {
        let mut state = self.state();
        let changed = state.visible.get(&user).copied().unwrap_or(true) != visible;
        state.visible.insert(user, visible);
        changed && state.connections_by_user.contains_key(&user)
    }

    /// The account's own view of its status; being disconnected wins over any
    /// choice it made.
    pub fn presence_of(&self, user: Id) -> String {
        self.state().presence(user)
    }

    /// The status `viewer` may see: an account that hides its status reads as
    /// offline to everybody but itself.
    pub fn public_presence_of(&self, user: Id, viewer: Option<Id>) -> String {
        let state = self.state();
        let hidden = viewer != Some(user) && !state.visible.get(&user).copied().unwrap_or(true);
        if hidden {
            OFFLINE.to_string()
        } else {
            state.presence(user)
        }
    }

    pub fn connection_ids(&self, user: Id) -> Vec<String> {
        self.state()
            .connections_by_user
            .get(&user)
            .map(|connections| connections.iter().cloned().collect())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn maja() -> Id {
        Id::parse("30000000-0000-0000-0000-000000000001").unwrap()
    }

    fn ada() -> Id {
        Id::parse("30000000-0000-0000-0000-000000000002").unwrap()
    }

    #[test]
    fn a_chosen_status_is_shared_by_all_connections_and_offline_wins_after_the_last_leaves() {
        let tracker = PresenceTracker::default();

        assert!(tracker.connect("maja-one", maja(), true));
        assert!(!tracker.connect("maja-two", maja(), true));
        assert_eq!(tracker.presence_of(maja()), "online");

        assert!(tracker.set_presence(maja(), "dnd"));
        assert!(!tracker.set_presence(maja(), "dnd"));
        assert_eq!(tracker.presence_of(maja()), "dnd");

        assert!(!tracker.disconnect("maja-one"));
        assert_eq!(tracker.presence_of(maja()), "dnd");

        assert!(tracker.disconnect("maja-two"));
        assert_eq!(tracker.presence_of(maja()), "offline");

        assert!(tracker.connect("maja-reconnected", maja(), true));
        assert_eq!(tracker.presence_of(maja()), "dnd");
    }

    #[test]
    fn a_hidden_status_is_offline_to_observers_and_visible_to_its_owner() {
        let tracker = PresenceTracker::default();
        tracker.connect("maja-hidden", maja(), false);
        tracker.set_presence(maja(), "dnd");

        assert_eq!(tracker.presence_of(maja()), "dnd");
        assert_eq!(tracker.public_presence_of(maja(), Some(maja())), "dnd");
        assert_eq!(tracker.public_presence_of(maja(), Some(ada())), "offline");
        assert_eq!(tracker.public_presence_of(maja(), None), "offline");
        assert!(tracker.set_visibility(maja(), true));
        assert_eq!(tracker.public_presence_of(maja(), Some(ada())), "dnd");
    }

    #[test]
    fn a_seeded_status_shows_while_offline_and_connecting_promotes_the_account() {
        let tracker = PresenceTracker::default();
        let nobody = Id::random();
        tracker.seed([
            (maja(), "idle".to_string()),
            (ada(), "dnd".to_string()),
            (nobody, "offline".to_string()),
            (Id::random(), "invisible".to_string()),
        ]);

        assert_eq!(tracker.presence_of(maja()), "idle");
        assert_eq!(tracker.public_presence_of(maja(), Some(ada())), "idle");
        assert_eq!(tracker.presence_of(ada()), "dnd");
        assert_eq!(tracker.presence_of(nobody), "offline");

        tracker.connect("maja-one", maja(), true);
        assert_eq!(tracker.presence_of(maja()), "online");
        tracker.set_presence(maja(), "dnd");
        assert_eq!(tracker.presence_of(maja()), "dnd");

        tracker.disconnect("maja-one");
        assert_eq!(tracker.presence_of(maja()), "offline");
    }

    #[test]
    fn a_seeded_status_obeys_the_accounts_visibility_choice() {
        let tracker = PresenceTracker::default();
        tracker.seed([(maja(), "idle".to_string())]);

        // Not connected, so what observers see does not change now.
        assert!(!tracker.set_visibility(maja(), false));

        assert_eq!(tracker.presence_of(maja()), "idle");
        assert_eq!(tracker.public_presence_of(maja(), Some(ada())), "offline");
    }

    #[test]
    fn a_seed_does_not_overrule_a_live_connection() {
        let tracker = PresenceTracker::default();
        tracker.connect("a", maja(), true);

        tracker.seed([(maja(), "idle".to_string())]);

        assert_eq!(tracker.presence_of(maja()), "online");
    }

    #[test]
    fn only_the_four_known_words_are_statuses() {
        for known in ["online", "idle", "dnd", "offline"] {
            assert!(is_supported(known));
        }
        for unknown in ["", "Online", "invisible", "dnd ", "away"] {
            assert!(!is_supported(unknown), "{unknown:?}");
        }
    }

    #[test]
    fn the_status_a_choice_set_while_connected_is_remembered_after_going_offline() {
        let tracker = PresenceTracker::default();
        tracker.connect("a", maja(), true);
        tracker.set_presence(maja(), "idle");
        tracker.disconnect("a");

        assert!(!tracker.set_presence(maja(), "online"));
        assert!(tracker.connect("b", maja(), true));
        assert_eq!(tracker.presence_of(maja()), "idle");
        assert_eq!(tracker.connection_ids(maja()), ["b"]);
        assert_eq!(tracker.connection_count(), 1);
    }
}
