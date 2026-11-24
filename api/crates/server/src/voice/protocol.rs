//! The contract of voice signalling that the bootstrap and the hub share.
//! [`VERSION`] only changes with an incompatible change.

/// The version of the contract; every signal names it.
pub const VERSION: i64 = 1;

/// The largest signal payload, in bytes, that the hub relays.
pub const MAX_SIGNAL_BYTES: usize = 128 * 1024;

/// What a browser may send the others in its room with `SendVoiceSignal`: only
/// its state. Media and its negotiation go through the media server, so
/// browsers never connect to each other and never learn each other's
/// addresses.
pub const SIGNAL_KINDS: [&str; 1] = ["state"];

/// What a browser may send the media server with `SendMediaSignal`.
pub const MEDIA_SIGNAL_KINDS: [&str; 4] = ["offer", "answer", "candidate", "subscriptions"];

/// What the media server may send a browser as `MediaSignal`.
pub const MEDIA_SERVER_SIGNAL_KINDS: [&str; 3] = ["offer", "answer", "tracks"];

/// The most receiving lines one subscription list may name.
pub const MAX_SUBSCRIPTION_MIDS: usize = 4096;

/// The longest id of a receiving line in a subscription list.
pub const MAX_SUBSCRIPTION_MID_LENGTH: usize = 16;
