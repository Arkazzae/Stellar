//! Sealed values: what the server hands to a client to keep, or writes to the
//! database, and must be able to trust when it comes back.
//!
//! One master key lives in the `keys` folder of the data root. It is made on
//! the first start, readable by the server's user alone, and read on every
//! later start; a backup of the data root carries it, so a restore keeps
//! people signed in.
//!
//! A value is sealed for a named purpose. The key it is sealed under is
//! derived from the master key and the purpose (HKDF-SHA256), so a value
//! sealed for one purpose cannot be opened as another, and the caller may add
//! associated data, such as the id of the account the value is for, which must
//! be the same when the value is opened.
//!
//! The sealed text is Base64 for URLs of
//! `version (1 byte) | nonce (12 bytes) | AES-256-GCM(expiry (8 bytes) | payload)`.
//! The version byte is part of what the tag covers, so the format can change
//! later without an old value being read as a new one. The expiry is inside
//! the seal, in seconds since 1970, or zero for none.

use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use aws_lc_rs::aead::{AES_256_GCM, Aad, LessSafeKey, NONCE_LEN, Nonce, UnboundKey};
use aws_lc_rs::hkdf::{HKDF_SHA256, Salt};
use aws_lc_rs::rand;

use crate::encoding::{base64_url, from_base64_url};
use crate::values::Timestamp;

const VERSION: u8 = 1;
const KEY_LEN: usize = 32;
const TAG_LEN: usize = 16;
const EXPIRY_LEN: usize = 8;
/// Mixed into every derivation, so that a key made here is a key for nothing else.
const SALT: &[u8] = b"stellarr sealed values v1";
const KEY_FILE: &str = "master.key";

/// Why a value could not be opened. The reason is for the log and for tests;
/// a caller answers every one of them the same way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SealError {
    /// Not text this module wrote: wrong characters, a version it does not
    /// know, too short.
    Malformed,
    /// The tag did not match: the value was altered, or sealed for another
    /// purpose, under another key or with other associated data.
    Refused,
    /// Authentic, and its time has passed.
    Expired,
}

impl fmt::Display for SealError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            SealError::Malformed => "the sealed value is malformed",
            SealError::Refused => "the sealed value was refused",
            SealError::Expired => "the sealed value has expired",
        })
    }
}

impl std::error::Error for SealError {}

/// Why the master key could not be made or read.
#[derive(Debug)]
pub struct KeyError(String);

impl fmt::Display for KeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for KeyError {}

/// Seals and opens values. Cloning it is cheap enough to hand around.
#[derive(Clone)]
pub struct Sealer {
    master: [u8; KEY_LEN],
}

/// Nothing of the key is shown.
impl fmt::Debug for Sealer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Sealer(..)")
    }
}

impl Sealer {
    /// Reads the master key from `keys_folder`, making it first if there is
    /// none. The file is created for its owner alone; a file that group or
    /// others can read is made private again.
    pub fn load_or_create(keys_folder: &Path) -> Result<Sealer, KeyError> {
        let path = keys_folder.join(KEY_FILE);
        let fail = |what: &str, error: &dyn fmt::Display| {
            KeyError(format!("{what} {}: {error}", path.display()))
        };
        fs::create_dir_all(keys_folder).map_err(|e| fail("cannot create the folder of", &e))?;
        // The key is written whole to a file of its own and then linked to its
        // name, so that the name is never seen without the whole key, whatever
        // stops the process, and of two processes that start at once on an
        // empty folder one makes the key and the other reads it.
        if !path.exists() {
            let mut key = [0u8; KEY_LEN];
            rand::fill(&mut key)
                .map_err(|_| KeyError("the system random number generator failed".into()))?;
            let draft = keys_folder.join(format!("{KEY_FILE}.{}", crate::values::Id::random()));
            let written = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&draft)
                .and_then(|mut file| file.write_all(&key).and_then(|()| file.sync_all()));
            let linked = written.and_then(|()| fs::hard_link(&draft, &path));
            let _ = fs::remove_file(&draft);
            match linked {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(fail("cannot create", &error)),
            }
        }
        Sealer::read(&path).map_err(|e| fail("cannot read", &e))
    }

    fn read(path: &PathBuf) -> Result<Sealer, String> {
        let permissions = fs::metadata(path).map_err(|e| e.to_string())?.permissions();
        if permissions.mode() & 0o077 != 0 {
            fs::set_permissions(path, fs::Permissions::from_mode(0o600))
                .map_err(|e| e.to_string())?;
        }
        let mut bytes = Vec::new();
        fs::File::open(path)
            .and_then(|mut file| file.read_to_end(&mut bytes))
            .map_err(|e| e.to_string())?;
        let master: [u8; KEY_LEN] = bytes
            .as_slice()
            .try_into()
            .map_err(|_| format!("the key is {} bytes long, not {KEY_LEN}", bytes.len()))?;
        Ok(Sealer { master })
    }

    /// A sealer on a key of the caller's, for tests.
    pub fn from_key(master: [u8; KEY_LEN]) -> Sealer {
        Sealer { master }
    }

    /// The 32-byte key for `purpose`, for a part that seals on its own, such
    /// as the single sign-on library.
    pub fn derive_key(&self, purpose: &str) -> [u8; KEY_LEN] {
        let mut key = [0u8; KEY_LEN];
        let info = [purpose.as_bytes()];
        Salt::new(HKDF_SHA256, SALT)
            .extract(&self.master)
            .expand(&info, KeyLength)
            .and_then(|okm| okm.fill(&mut key))
            .expect("32 bytes are within what HKDF-SHA256 can give");
        key
    }

    /// Seals `payload` for `purpose` with `aad` as associated data, to be
    /// opened before `expires` if there is one.
    pub fn seal(
        &self,
        purpose: &str,
        aad: &[u8],
        expires: Option<Timestamp>,
        payload: &[u8],
    ) -> String {
        let key = self.key_for(purpose);
        let mut nonce = [0u8; NONCE_LEN];
        rand::fill(&mut nonce).expect("the system random number generator failed");
        let mut body = Vec::with_capacity(EXPIRY_LEN + payload.len() + TAG_LEN);
        body.extend_from_slice(&expiry_seconds(expires).to_be_bytes());
        body.extend_from_slice(payload);
        key.seal_in_place_append_tag(
            Nonce::assume_unique_for_key(nonce),
            Aad::from(associated(aad)),
            &mut body,
        )
        .expect("AES-GCM seals what is this small");
        let mut sealed = Vec::with_capacity(1 + NONCE_LEN + body.len());
        sealed.push(VERSION);
        sealed.extend_from_slice(&nonce);
        sealed.extend_from_slice(&body);
        base64_url(&sealed)
    }

    /// Opens a value sealed for `purpose` with the same `aad`, as of `now`.
    pub fn open(
        &self,
        purpose: &str,
        aad: &[u8],
        sealed: &str,
        now: Timestamp,
    ) -> Result<Vec<u8>, SealError> {
        let bytes = from_base64_url(sealed).ok_or(SealError::Malformed)?;
        if bytes.len() < 1 + NONCE_LEN + EXPIRY_LEN + TAG_LEN || bytes[0] != VERSION {
            return Err(SealError::Malformed);
        }
        let nonce: [u8; NONCE_LEN] = bytes[1..1 + NONCE_LEN]
            .try_into()
            .map_err(|_| SealError::Malformed)?;
        let mut body = bytes[1 + NONCE_LEN..].to_vec();
        let opened = self
            .key_for(purpose)
            .open_in_place(
                Nonce::assume_unique_for_key(nonce),
                Aad::from(associated(aad)),
                &mut body,
            )
            .map_err(|_| SealError::Refused)?;
        let (expiry, payload) = opened.split_at(EXPIRY_LEN);
        let expiry = i64::from_be_bytes(expiry.try_into().map_err(|_| SealError::Malformed)?);
        if expiry != 0 && now.unix_seconds() >= expiry {
            return Err(SealError::Expired);
        }
        Ok(payload.to_vec())
    }

    fn key_for(&self, purpose: &str) -> LessSafeKey {
        let key = self.derive_key(purpose);
        LessSafeKey::new(UnboundKey::new(&AES_256_GCM, &key).expect("a 32-byte key"))
    }
}

/// The version and the caller's data, the two things the tag covers besides
/// the text itself.
fn associated(aad: &[u8]) -> Vec<u8> {
    let mut all = Vec::with_capacity(1 + aad.len());
    all.push(VERSION);
    all.extend_from_slice(aad);
    all
}

fn expiry_seconds(expires: Option<Timestamp>) -> i64 {
    // The moment 1970-01-01T00:00:00Z is "none"; nobody seals for it.
    expires.map_or(0, |moment| moment.unix_seconds().max(1))
}

/// How long a key `Okm` is asked for.
struct KeyLength;

impl aws_lc_rs::hkdf::KeyType for KeyLength {
    fn len(&self) -> usize {
        KEY_LEN
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::testing::TempDir;

    fn sealer() -> Sealer {
        Sealer::from_key([7u8; KEY_LEN])
    }

    fn now() -> Timestamp {
        Timestamp::parse("2026-11-24 12:00:00").unwrap()
    }

    #[test]
    fn a_value_opens_for_its_purpose_and_data() {
        let sealed = sealer().seal("session", b"user-1", None, b"hello");

        assert_eq!(
            sealer().open("session", b"user-1", &sealed, now()).unwrap(),
            b"hello"
        );
    }

    #[test]
    fn nothing_is_readable_in_the_text() {
        let sealed = sealer().seal("session", b"", None, b"a password nobody must read");

        assert!(!sealed.contains("password"));
        assert_ne!(
            sealed,
            sealer().seal("session", b"", None, b"a password nobody must read")
        );
    }

    #[test]
    fn another_purpose_cannot_open_it() {
        let sealed = sealer().seal("session", b"", None, b"x");

        assert_eq!(
            sealer().open("totp", b"", &sealed, now()),
            Err(SealError::Refused)
        );
    }

    #[test]
    fn other_associated_data_cannot_open_it() {
        let sealed = sealer().seal("session", b"user-1", None, b"x");

        assert_eq!(
            sealer().open("session", b"user-2", &sealed, now()),
            Err(SealError::Refused)
        );
        assert_eq!(
            sealer().open("session", b"", &sealed, now()),
            Err(SealError::Refused)
        );
    }

    #[test]
    fn another_key_cannot_open_it() {
        let sealed = sealer().seal("session", b"", None, b"x");

        assert_eq!(
            Sealer::from_key([8u8; KEY_LEN]).open("session", b"", &sealed, now()),
            Err(SealError::Refused)
        );
    }

    #[test]
    fn it_expires_at_its_moment_and_not_before() {
        let sealed = sealer().seal(
            "session",
            b"",
            Some(now().plus(Duration::from_secs(60))),
            b"x",
        );

        assert!(sealer().open("session", b"", &sealed, now()).is_ok());
        assert!(
            sealer()
                .open("session", b"", &sealed, now().plus(Duration::from_secs(59)))
                .is_ok()
        );
        assert_eq!(
            sealer().open("session", b"", &sealed, now().plus(Duration::from_secs(60))),
            Err(SealError::Expired)
        );
    }

    #[test]
    fn an_empty_payload_is_sealed_too() {
        let sealed = sealer().seal("session", b"", None, b"");

        assert_eq!(sealer().open("session", b"", &sealed, now()).unwrap(), b"");
    }

    #[test]
    fn a_truncated_value_is_refused_at_every_length() {
        let sealed = sealer().seal("session", b"", None, b"payload");
        let bytes = from_base64_url(&sealed).unwrap();

        for length in 0..bytes.len() {
            let cut = base64_url(&bytes[..length]);
            let outcome = sealer().open("session", b"", &cut, now());
            assert!(outcome.is_err(), "{length}");
        }
        assert_eq!(
            sealer().open("session", b"", "", now()),
            Err(SealError::Malformed)
        );
        assert_eq!(
            sealer().open("session", b"", "not base64!", now()),
            Err(SealError::Malformed)
        );
    }

    #[test]
    fn a_value_with_any_bit_flipped_is_refused() {
        let sealed = sealer().seal("session", b"data", None, b"payload");
        let bytes = from_base64_url(&sealed).unwrap();

        for index in 0..bytes.len() {
            for bit in 0..8 {
                let mut changed = bytes.clone();
                changed[index] ^= 1 << bit;
                let outcome = sealer().open("session", b"data", &base64_url(&changed), now());
                assert!(outcome.is_err(), "byte {index} bit {bit}");
            }
        }
    }

    #[test]
    fn a_version_the_module_does_not_know_is_malformed() {
        let sealed = sealer().seal("session", b"", None, b"x");
        let mut bytes = from_base64_url(&sealed).unwrap();
        bytes[0] = 2;

        assert_eq!(
            sealer().open("session", b"", &base64_url(&bytes), now()),
            Err(SealError::Malformed)
        );
    }

    #[test]
    fn a_text_with_a_different_spelling_of_the_same_bytes_is_refused() {
        let sealed = sealer().seal("session", b"", None, b"x");

        assert!(
            sealer()
                .open("session", b"", &format!("{sealed}="), now())
                .is_err()
        );
        assert!(
            sealer()
                .open("session", b"", &format!(" {sealed}"), now())
                .is_err()
        );
    }

    #[test]
    fn derived_keys_differ_by_purpose_and_by_master_key() {
        let (a, b) = (sealer(), Sealer::from_key([8u8; KEY_LEN]));

        assert_ne!(a.derive_key("oidc"), a.derive_key("session"));
        assert_ne!(a.derive_key("oidc"), b.derive_key("oidc"));
        assert_eq!(a.derive_key("oidc"), a.derive_key("oidc"));
        assert_ne!(a.derive_key("oidc"), [7u8; KEY_LEN]);
    }

    #[test]
    fn nothing_secret_is_in_debug_or_in_an_error() {
        let shown = format!("{:?}", sealer());

        assert_eq!(shown, "Sealer(..)");
        assert!(!format!("{:?} {}", SealError::Refused, SealError::Refused).contains('7'));
    }

    #[test]
    fn the_key_is_made_once_private_and_read_afterwards() {
        let folder = TempDir::new();
        let keys = folder.path().join("keys");

        let first = Sealer::load_or_create(&keys).unwrap();
        let mode = fs::metadata(keys.join(KEY_FILE))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        let second = Sealer::load_or_create(&keys).unwrap();

        assert_eq!(mode, 0o600);
        assert_eq!(first.derive_key("x"), second.derive_key("x"));
        assert_eq!(fs::metadata(keys.join(KEY_FILE)).unwrap().len(), 32);
    }

    #[test]
    fn a_key_file_others_can_read_is_made_private_and_a_broken_one_is_refused() {
        let folder = TempDir::new();
        let keys = folder.path().join("keys");
        Sealer::load_or_create(&keys).unwrap();
        let path = keys.join(KEY_FILE);
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();

        Sealer::load_or_create(&keys).unwrap();
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );

        fs::write(&path, b"short").unwrap();
        let refusal = Sealer::load_or_create(&keys).unwrap_err().to_string();
        assert!(refusal.contains("5 bytes"), "{refusal}");
    }
}
