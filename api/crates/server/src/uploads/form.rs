//! A form with files in it, read into what a route needs: the files it asked
//! for, staged on disk, and the values it asked for, in memory.
//!
//! A file is written to the staging folder as it arrives and never held whole.
//! Each slot says how much of a file it will keep, once the file's own
//! `Content-Type` is known: a file over that is not written, only counted, and
//! one the slot does not take is not written at all. The rest of the body is
//! read to its end either way, so that the answer a route gives does not
//! depend on where in the body it found out.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use stellarr_unicode::to_upper;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;

use crate::http::ApiError;
use crate::http::multipart::{Disposition, FormError, Kind, Multipart, PartHead};
use crate::values::Id;

/// How many leading bytes of a file are kept for checking its signature.
pub const HEADER_BYTES: usize = stellarr_storage::validation::HEADER_LENGTH;

/// The most a value of the form may be before it is no value of any field a
/// route knows. A longer one is kept as a mark that it was too long.
const VALUE_LIMIT: usize = 64 * 1024;

/// A file the form carried, on its way into the store.
#[derive(Debug)]
pub struct Staged {
    /// The name the client gave it.
    pub file_name: String,
    /// The `Content-Type` of its part, empty when it had none.
    pub content_type: String,
    /// Its size in bytes, counted whether or not it was kept.
    pub length: u64,
    /// Its first bytes.
    pub header: Vec<u8>,
    /// Where it is, when it was kept: removed when this is dropped.
    path: Option<PathBuf>,
}

impl Staged {
    /// The file opened for reading, from its first byte.
    pub async fn open(&self) -> std::io::Result<File> {
        match &self.path {
            Some(path) => File::open(path).await,
            None => Err(std::io::Error::other("the file was not kept")),
        }
    }

    /// For tests: a staged file that is not on disk.
    #[cfg(test)]
    pub fn unkept(content_type: &str, header: &[u8], length: u64) -> Staged {
        Staged {
            file_name: String::new(),
            content_type: content_type.to_string(),
            length,
            header: header.to_vec(),
            path: None,
        }
    }
}

impl Drop for Staged {
    fn drop(&mut self) {
        if let Some(path) = self.path.take() {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// One file a route asks for.
pub struct Slot<'a> {
    /// The name of the form field, in any letter case.
    pub name: &'a str,
    /// How many bytes of a file of this `Content-Type` are kept, or none when
    /// the file is not wanted at all.
    pub keeps: &'a (dyn Fn(&str) -> Option<u64> + Sync),
}

/// What the text fields of a form hold.
#[derive(Debug, Default)]
pub struct Fields {
    values: HashMap<String, Value>,
}

#[derive(Debug)]
enum Value {
    Text(String),
    TooLong,
}

impl Fields {
    /// The value of a field, several of the same name joined by commas, as
    /// the first server joined them. `Err` for one that was too long to be
    /// anything.
    pub fn get(&self, name: &str) -> Option<Result<&str, TooLong>> {
        self.values.get(&to_upper(name)).map(|value| match value {
            Value::Text(text) => Ok(text.as_str()),
            Value::TooLong => Err(TooLong),
        })
    }

    fn add(&mut self, name: &str, text: String) {
        match self.values.entry(to_upper(name)) {
            std::collections::hash_map::Entry::Vacant(slot) => {
                slot.insert(if text.len() > VALUE_LIMIT {
                    Value::TooLong
                } else {
                    Value::Text(text)
                });
            }
            std::collections::hash_map::Entry::Occupied(mut slot) => {
                let joined = match slot.get() {
                    Value::Text(have) if have.len() + 1 + text.len() <= VALUE_LIMIT => {
                        Some(format!("{have},{text}"))
                    }
                    _ => None,
                };
                slot.insert(joined.map_or(Value::TooLong, Value::Text));
            }
        }
    }
}

/// A value too long to be read as anything.
#[derive(Debug, Clone, Copy)]
pub struct TooLong;

/// What the form held.
#[derive(Debug, Default)]
pub struct Received {
    /// In the order of the slots: the first file of each name, if any.
    pub files: Vec<Option<Staged>>,
    pub fields: Fields,
}

/// Reads the whole form. `staging` is where files are written, `slots` the
/// files wanted and `fields` the names of the text values wanted: the rest of
/// the form is read and let go.
pub async fn read(
    reader: &mut Multipart,
    staging: &Path,
    slots: &[Slot<'_>],
    fields: &[&str],
) -> Result<Received, ApiError> {
    let mut received = Received {
        files: slots.iter().map(|_| None).collect(),
        fields: Fields::default(),
    };
    let mut taken = vec![false; slots.len()];
    while let Some(head) = reader.next_part().await? {
        let disposition = head.disposition()?;
        match disposition.kind {
            Kind::File => {
                let wanted = slots
                    .iter()
                    .position(|slot| same_name(&disposition.name, slot.name))
                    .filter(|at| !taken[*at]);
                match wanted {
                    Some(at) => {
                        taken[at] = true;
                        received.files[at] =
                            Some(stage(reader, staging, &head, &disposition, &slots[at]).await?);
                    }
                    None => skip(reader).await?,
                }
            }
            // Only the values a route reads are kept, so that what a form holds
            // in memory is a few names long however many parts it has.
            Kind::Field if fields.iter().any(|wanted| same_name(&disposition.name, wanted)) => {
                let text = value(reader).await?;
                received.fields.add(&disposition.name, text);
            }
            Kind::Field => skip(reader).await?,
            Kind::Other => skip(reader).await?,
        }
    }
    Ok(received)
}

fn same_name(sent: &str, wanted: &str) -> bool {
    to_upper(sent) == to_upper(wanted)
}

async fn skip(reader: &mut Multipart) -> Result<(), FormError> {
    while reader.chunk().await?.is_some() {}
    Ok(())
}

/// The text of a field. What is past the limit is passed over, and the text
/// is one byte longer than the limit so that it reads as too long.
async fn value(reader: &mut Multipart) -> Result<String, FormError> {
    let mut bytes: Vec<u8> = Vec::new();
    while let Some(chunk) = reader.chunk().await? {
        let room = (VALUE_LIMIT + 1).saturating_sub(bytes.len());
        bytes.extend_from_slice(&chunk[..chunk.len().min(room)]);
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

async fn stage(
    reader: &mut Multipart,
    staging: &Path,
    head: &PartHead,
    disposition: &Disposition,
    slot: &Slot<'_>,
) -> Result<Staged, ApiError> {
    let content_type = head.header("content-type").unwrap_or_default();
    let mut keeps = (slot.keeps)(&content_type);
    let mut staged = Staged {
        file_name: disposition.file_name.clone().unwrap_or_default(),
        content_type,
        length: 0,
        header: Vec::with_capacity(HEADER_BYTES),
        path: None,
    };
    let mut open: Option<File> = None;
    while let Some(chunk) = reader.chunk().await? {
        let room = HEADER_BYTES.saturating_sub(staged.header.len());
        staged.header.extend_from_slice(&chunk[..chunk.len().min(room)]);
        staged.length += chunk.len() as u64;
        let Some(limit) = keeps else { continue };
        if staged.length > limit {
            // Over what is kept: what was written is not wanted any more.
            keeps = None;
            open = None;
            if let Some(path) = staged.path.take() {
                let _ = tokio::fs::remove_file(path).await;
            }
            continue;
        }
        if open.is_none() {
            let path = staging.join(format!("{}.part", Id::random()));
            let file = File::create(&path).await.map_err(ApiError::internal)?;
            // From here on the file goes when the staged file does, whatever
            // happens to the request.
            staged.path = Some(path);
            open = Some(file);
        }
        if let Some(file) = open.as_mut() {
            file.write_all(&chunk).await.map_err(ApiError::internal)?;
        }
    }
    if let Some(mut file) = open {
        file.flush().await.map_err(ApiError::internal)?;
    }
    Ok(staged)
}
