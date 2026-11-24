//! What a backup holds.

/// Which parts of an instance a backup covers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Parts(u8);

impl Parts {
    pub const NONE: Parts = Parts(0);
    /// A snapshot of the SQLite database.
    pub const DATABASE: Parts = Parts(1);
    /// Every object in the object store: uploads, avatars, emoji, sounds.
    pub const OBJECTS: Parts = Parts(2);
    /// The keys folder, without which the secrets set in the panel and the
    /// sessions cannot be read.
    pub const KEYS: Parts = Parts(4);
    /// The instance's own files on the data volume: the secrets the container
    /// generated, the TLS certificates and the desktop releases.
    pub const INSTANCE: Parts = Parts(8);
    pub const ALL: Parts = Parts(15);

    pub const fn with(self, other: Parts) -> Parts {
        Parts(self.0 | other.0)
    }

    pub const fn has(self, other: Parts) -> bool {
        self.0 & other.0 == other.0
    }

    pub const fn is_none(self) -> bool {
        self.0 == 0
    }

    /// The names the manifest and the panel use, in their fixed order.
    pub fn names(self) -> Vec<&'static str> {
        [
            (Parts::DATABASE, DATABASE),
            (Parts::OBJECTS, OBJECTS),
            (Parts::KEYS, KEYS),
            (Parts::INSTANCE, INSTANCE),
        ]
        .into_iter()
        .filter(|(part, _)| self.has(*part))
        .map(|(_, name)| name)
        .collect()
    }
}

pub const DATABASE: &str = "database";
pub const OBJECTS: &str = "objects";
pub const KEYS: &str = "keys";
pub const INSTANCE: &str = "instance";
