//! What may be uploaded: the allow-list of types, where each is kept, how
//! large it may be, and the signature its first bytes must have.
//!
//! The content type of an upload is only a claim. A file reaches the store
//! only when its first bytes fit the container it declares, which keeps
//! executables and arbitrary data from being served from this origin as
//! media. Whether a browser can decode the codec inside is left to the
//! reader.
//!
//! Type names are compared without regard to ASCII case. Nothing outside
//! ASCII folds, so a name that only matches after Unicode case mapping is
//! refused.

use crate::settings::UploadOptions;

/// How many leading bytes of a file the signature checks may need to see.
/// Callers read this many (or all, for a shorter file) before asking.
pub const HEADER_LENGTH: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UploadKind {
    Image,
    Audio,
    Video,
    File,
}

/// What the instance stores a type as: its canonical media type, extension
/// and folder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UploadSpec {
    pub kind: UploadKind,
    pub content_type: &'static str,
    pub extension: &'static str,
    pub folder: &'static str,
}

struct Rule {
    kind: UploadKind,
    content_type: &'static str,
    extension: &'static str,
    aliases: &'static [&'static str],
    matches: fn(&[u8]) -> bool,
}

impl Rule {
    fn spec(&self) -> UploadSpec {
        UploadSpec {
            kind: self.kind,
            content_type: self.content_type,
            extension: self.extension,
            folder: match self.kind {
                UploadKind::Image => "images",
                UploadKind::Audio => "audio",
                UploadKind::Video => "videos",
                UploadKind::File => "files",
            },
        }
    }

    fn answers_to(&self, name: &str) -> bool {
        self.content_type.eq_ignore_ascii_case(name)
            || self.aliases.iter().any(|a| a.eq_ignore_ascii_case(name))
    }
}

const DOCX: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.document";
const XLSX: &str = "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet";
const PPTX: &str = "application/vnd.openxmlformats-officedocument.presentationml.presentation";

macro_rules! rule {
    ($kind:ident, $ty:expr, $ext:expr, $check:expr) => {
        rule!($kind, $ty, $ext, $check, [])
    };
    ($kind:ident, $ty:expr, $ext:expr, $check:expr, [$($alias:expr),*]) => {
        Rule {
            kind: UploadKind::$kind,
            content_type: $ty,
            extension: $ext,
            aliases: &[$($alias),*],
            matches: $check,
        }
    };
}

static RULES: &[Rule] = &[
    rule!(Image, "image/jpeg", ".jpg", is_jpeg),
    rule!(Image, "image/png", ".png", is_png),
    rule!(Image, "image/gif", ".gif", is_gif),
    rule!(Image, "image/webp", ".webp", is_webp),
    rule!(Audio, "audio/mpeg", ".mp3", is_mp3),
    rule!(Audio, "audio/ogg", ".ogg", is_ogg),
    rule!(Audio, "audio/wav", ".wav", is_wave, ["audio/x-wav"]),
    rule!(Audio, "audio/webm", ".webm", is_webm),
    rule!(Audio, "audio/mp4", ".m4a", is_iso_media, ["audio/x-m4a"]),
    rule!(Audio, "audio/aac", ".aac", is_aac),
    rule!(Video, "video/mp4", ".mp4", is_iso_media, ["video/x-m4v"]),
    rule!(Video, "video/webm", ".webm", is_webm),
    rule!(Video, "video/ogg", ".ogv", is_ogg),
    rule!(Video, "video/quicktime", ".mov", is_iso_media),
    rule!(File, "application/pdf", ".pdf", is_pdf),
    rule!(
        File,
        "application/zip",
        ".zip",
        is_zip,
        ["application/x-zip-compressed"]
    ),
    rule!(File, "text/plain", ".txt", is_plain_text),
    rule!(File, "application/json", ".json", is_json),
    rule!(File, DOCX, ".docx", is_zip),
    rule!(File, XLSX, ".xlsx", is_zip),
    rule!(File, PPTX, ".pptx", is_zip),
];

fn rule_named(name: &str) -> Option<&'static Rule> {
    RULES.iter().find(|rule| rule.answers_to(name))
}

/// Finds the spec for a declared media type. Parameters such as
/// `; charset=utf-8` are ignored and an alias resolves to its canonical type.
pub fn spec_for(declared_content_type: &str) -> Option<UploadSpec> {
    let media_type = declared_content_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim();
    rule_named(media_type).map(Rule::spec)
}

/// Whether `header`, the first bytes of a file, belong to the format `spec`
/// names. Pass up to [`HEADER_LENGTH`] bytes.
pub fn matches_signature(spec: &UploadSpec, header: &[u8]) -> bool {
    RULES
        .iter()
        .find(|rule| rule.content_type.eq_ignore_ascii_case(spec.content_type))
        .is_some_and(|rule| (rule.matches)(header))
}

/// The size limit of the kind `spec` belongs to.
pub fn max_bytes(spec: &UploadSpec, options: &UploadOptions) -> i64 {
    match spec.kind {
        UploadKind::Image => options.max_image_bytes,
        UploadKind::Audio => options.max_audio_bytes,
        UploadKind::Video => options.max_video_bytes,
        UploadKind::File => options.max_file_bytes,
    }
}

/// The image formats alone, as avatars, banners and the like accept them.
pub mod image {
    use super::{RULES, UploadKind, is_gif, is_jpeg, is_png, is_webp, rule_named};

    /// Inspect supported image types in tests.
    #[doc(hidden)]
    pub fn is_allowed_content_type(content_type: &str) -> bool {
        rule_named(content_type).is_some_and(|rule| rule.kind == UploadKind::Image)
    }

    /// Whether `header` starts like an image of `content_type`. A type that
    /// is not an image type never matches.
    pub fn matches_signature(content_type: &str, header: &[u8]) -> bool {
        match content_type.to_ascii_lowercase().as_str() {
            "image/jpeg" => is_jpeg(header),
            "image/png" => is_png(header),
            "image/gif" => is_gif(header),
            "image/webp" => is_webp(header),
            _ => false,
        }
    }

    /// The extension of an image type, with its dot.
    /// Inspect image extensions in tests.
    #[doc(hidden)]
    pub fn extension_for(content_type: &str) -> Option<&'static str> {
        RULES
            .iter()
            .find(|rule| rule.kind == UploadKind::Image && rule.answers_to(content_type))
            .map(|rule| rule.extension)
    }
}

fn is_jpeg(header: &[u8]) -> bool {
    header.starts_with(&[0xFF, 0xD8, 0xFF])
}

fn is_png(header: &[u8]) -> bool {
    header.starts_with(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A])
}

fn is_gif(header: &[u8]) -> bool {
    header.starts_with(b"GIF87a") || header.starts_with(b"GIF89a")
}

fn riff_form(header: &[u8], form: &[u8; 4]) -> bool {
    header.len() >= 12 && header.starts_with(b"RIFF") && &header[8..12] == form
}

fn is_webp(header: &[u8]) -> bool {
    riff_form(header, b"WEBP")
}

fn is_wave(header: &[u8]) -> bool {
    riff_form(header, b"WAVE")
}

/// An ID3 tag, or an MPEG audio frame: eleven set sync bits, then a layer
/// that is not the reserved one.
fn is_mp3(header: &[u8]) -> bool {
    header.starts_with(b"ID3")
        || (header.len() >= 2 && header[0] == 0xFF && (header[1] & 0xE6) == 0xE2)
}

fn is_ogg(header: &[u8]) -> bool {
    header.starts_with(b"OggS")
}

fn is_webm(header: &[u8]) -> bool {
    header.starts_with(&[0x1A, 0x45, 0xDF, 0xA3])
}

/// MP4, M4A and QuickTime share the ISO base media layout: a box size, then
/// `ftyp`.
fn is_iso_media(header: &[u8]) -> bool {
    header.len() >= 12 && &header[4..8] == b"ftyp"
}

/// An ADTS frame header: twelve set sync bits, then layer bits that must be
/// zero.
fn is_aac(header: &[u8]) -> bool {
    header.len() >= 2 && header[0] == 0xFF && (header[1] & 0xF6) == 0xF0
}

fn is_pdf(header: &[u8]) -> bool {
    header.starts_with(b"%PDF-")
}

/// A local file header, an empty archive or a spanned archive marker.
fn is_zip(header: &[u8]) -> bool {
    header.len() >= 4
        && header[0] == b'P'
        && header[1] == b'K'
        && matches!((header[2], header[3]), (3, 4) | (5, 6) | (7, 8))
}

fn is_plain_text(header: &[u8]) -> bool {
    !header.is_empty() && !header.contains(&0)
}

fn is_json(header: &[u8]) -> bool {
    if !is_plain_text(header) {
        return false;
    }
    header
        .iter()
        .find(|byte| !matches!(byte, b' ' | b'\t' | b'\r' | b'\n'))
        .is_some_and(|byte| matches!(byte, b'{' | b'['))
}
