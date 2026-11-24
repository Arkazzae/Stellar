//! What the Discord ids in one batch of messages stand for, from the import's
//! bookkeeping. Nobody is ever mentioned: a placeholder belongs to no server
//! and must not be notified, so a mention of a Discord user is a label with
//! the name they showed. Emoji are not imported yet, so a custom emoji keeps
//! its name as text.
//!
//! The rewriting asks as it reads, and the answers have to be at hand by
//! then. So a batch is rewritten twice: first with a resolver that only notes
//! which users and channels it was asked about ([`ImportResolver::recording`]),
//! then, once those are loaded, for real.

use std::collections::{BTreeSet, HashMap};
use std::sync::Mutex;

use crate::content::{MentionText, ReferenceResolver};

pub struct ImportResolver {
    user_names: HashMap<String, String>,
    channel_labels: HashMap<String, String>,
    role_names: HashMap<String, String>,
    asked_users: Mutex<BTreeSet<String>>,
    asked_channels: Mutex<BTreeSet<String>>,
}

impl ImportResolver {
    /// A resolver that knows these names. Users by id, to the name they
    /// showed; channels by id, to how a link to them reads (`#general`);
    /// roles by id, to how a mention of them reads (`@Moderators`).
    pub fn new(
        user_names: HashMap<String, String>,
        channel_labels: HashMap<String, String>,
        role_names: HashMap<String, String>,
    ) -> ImportResolver {
        ImportResolver {
            user_names,
            channel_labels,
            role_names,
            asked_users: Mutex::default(),
            asked_channels: Mutex::default(),
        }
    }

    /// A resolver that knows the roles and nothing else, for the first pass.
    pub fn recording(role_names: HashMap<String, String>) -> ImportResolver {
        ImportResolver::new(HashMap::new(), HashMap::new(), role_names)
    }

    /// The users it was asked about, known or not.
    pub fn asked_users(&self) -> BTreeSet<String> {
        lock(&self.asked_users).clone()
    }

    /// The channels it was asked about, known or not.
    pub fn asked_channels(&self) -> BTreeSet<String> {
        lock(&self.asked_channels).clone()
    }
}

fn lock(set: &Mutex<BTreeSet<String>>) -> std::sync::MutexGuard<'_, BTreeSet<String>> {
    set.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl ReferenceResolver for ImportResolver {
    fn user(&self, user_id: &str) -> Option<MentionText> {
        lock(&self.asked_users).insert(user_id.to_owned());
        self.user_names
            .get(user_id)
            .map(|name| MentionText::label(format!("@{name}")))
    }

    fn role(&self, role_id: &str) -> Option<String> {
        self.role_names.get(role_id).cloned()
    }

    fn channel(&self, channel_id: &str) -> Option<String> {
        lock(&self.asked_channels).insert(channel_id.to_owned());
        self.channel_labels.get(channel_id).cloned()
    }

    fn emoji(&self, _emoji_id: &str) -> Option<String> {
        None
    }
}
