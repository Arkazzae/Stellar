//! What a file may be: the checks every upload goes through before a byte of
//! it reaches the store, and the name it is downloaded as.

use stellarr_storage::UploadOptions;
use stellarr_storage::validation::{
    UploadKind, UploadSpec, matches_signature, max_bytes, spec_for,
};
use stellarr_unicode::utf16;

use super::form::Staged;

/// The longest stem a file name keeps, in UTF-16 units.
const MAX_STEM_UNITS: usize = 200;

/// Why a file is refused, in the order the checks are made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Failure {
    Empty,
    UnsupportedType,
    TooLarge,
    /// Its first bytes are not those of the type it says it is.
    SignatureMismatch,
}

/// The refusal, and what the file claimed to be when that is known.
pub struct Refused {
    pub failure: Failure,
    pub spec: Option<UploadSpec>,
}

/// An attachment of any allowed type, within the limit of its kind.
pub fn attachment(file: &Staged, options: &UploadOptions) -> Result<UploadSpec, Refused> {
    inspect(file, None, |spec| max_bytes(spec, options))
}

/// An image, within `maximum` bytes.
pub fn image(file: &Staged, maximum: i64) -> Result<UploadSpec, Refused> {
    inspect(file, Some(UploadKind::Image), |_| maximum)
}

fn inspect(
    file: &Staged,
    required: Option<UploadKind>,
    maximum: impl Fn(&UploadSpec) -> i64,
) -> Result<UploadSpec, Refused> {
    let refuse = |failure, spec| Err(Refused { failure, spec });
    if file.length == 0 {
        return refuse(Failure::Empty, None);
    }
    let spec = match spec_for(&file.content_type) {
        Some(spec) if required.is_none_or(|kind| spec.kind == kind) => spec,
        _ => return refuse(Failure::UnsupportedType, None),
    };
    if i64::try_from(file.length).map_or(true, |length| length > maximum(&spec)) {
        return refuse(Failure::TooLarge, Some(spec));
    }
    if !matches_signature(&spec, &file.header) {
        return refuse(Failure::SignatureMismatch, Some(spec));
    }
    Ok(spec)
}

/// How much of a file with this `Content-Type` is worth keeping while it is
/// read: what the checks above would let through, and none for a type they
/// refuse.
pub fn keep_attachment(content_type: &str, options: &UploadOptions) -> Option<u64> {
    let spec = spec_for(content_type)?;
    Some(max_bytes(&spec, options).max(0) as u64)
}

/// As [`keep_attachment`], for a place that takes images only.
pub fn keep_image(content_type: &str, maximum: i64) -> Option<u64> {
    let spec = spec_for(content_type)?;
    (spec.kind == UploadKind::Image).then_some(maximum.max(0) as u64)
}

/// Whether the text is standard padded base64 of one to 32 bytes. Only the
/// shape is checked; the bytes mean something to clients alone.
pub fn is_placeholder(text: &str) -> bool {
    let bytes = text.as_bytes();
    if bytes.is_empty() || !bytes.len().is_multiple_of(4) {
        return false;
    }
    let padding = bytes.iter().rev().take_while(|byte| **byte == b'=').count();
    if padding > 2 {
        return false;
    }
    let body = &bytes[..bytes.len() - padding];
    if !body
        .iter()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/'))
    {
        return false;
    }
    let decoded = bytes.len() / 4 * 3 - padding;
    decoded > 0 && decoded <= 32
}

/// The name a recipient downloads the file as. The extension comes from the
/// verified type, never from the submitted name, so that a PDF cannot be
/// handed out as `invoice.exe`.
pub fn safe_file_name(submitted: &str, spec: &UploadSpec) -> String {
    let name = file_name_of(submitted);
    let stem = match name.rfind('.') {
        Some(at) => &name[..at],
        None => name,
    };
    let kept: Vec<u16> = utf16::units(stem)
        .into_iter()
        .filter(|unit| {
            utf16::unit_is_letter_or_digit(*unit) || matches!(*unit, 0x20 | 0x2D | 0x5F | 0x2E)
        })
        .take(MAX_STEM_UNITS)
        .collect();
    let safe = utf16::text(&kept);
    let safe = safe.trim_matches([' ', '.']);
    format!(
        "{}{}",
        if safe.is_empty() { "attachment" } else { safe },
        spec.extension
    )
}

/// Returns the final segment of a slash-separated path.
pub fn file_name_of(path: &str) -> &str {
    match path.rfind('/') {
        Some(at) => &path[at + 1..],
        None => path,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PNG: &[u8] = &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 1, 2, 3, 4];

    fn failure(file: &Staged, options: &UploadOptions) -> Option<Failure> {
        attachment(file, options).err().map(|refused| refused.failure)
    }

    #[test]
    fn upload_checks_report_the_first_error_in_validation_order() {
        let options = UploadOptions {
            max_image_bytes: 12,
            ..UploadOptions::default()
        };
        let png = Staged::unkept("image/png", PNG, 12);
        assert_eq!(failure(&png, &options), None);
        // Empty beats everything, a type nobody serves beats the size, the
        // size beats the bytes.
        assert_eq!(failure(&Staged::unkept("text/html", b"", 0), &options), Some(Failure::Empty));
        assert_eq!(failure(&Staged::unkept("text/html", b"x", 99), &options), Some(Failure::UnsupportedType));
        assert_eq!(failure(&Staged::unkept("image/png", b"x", 13), &options), Some(Failure::TooLarge));
        assert_eq!(failure(&Staged::unkept("image/png", b"x", 12), &options), Some(Failure::SignatureMismatch));
        assert_eq!(failure(&Staged::unkept("IMAGE/PNG; charset=x", PNG, 12), &options), None);
    }

    #[test]
    fn a_picture_place_takes_images_only() {
        let pdf = Staged::unkept("application/pdf", b"%PDF-1", 6);

        assert_eq!(image(&pdf, 100).err().map(|r| r.failure), Some(Failure::UnsupportedType));
        assert!(image(&Staged::unkept("image/png", PNG, 12), 12).is_ok());
        assert_eq!(keep_image("application/pdf", 10), None);
        assert_eq!(keep_image("image/gif", 10), Some(10));
        assert_eq!(keep_image("image/gif", -1), Some(0));
    }

    #[test]
    fn a_placeholder_is_padded_base64_of_one_to_thirty_two_bytes() {
        for good in ["AAAA", "AA==", "AAA=", &"A".repeat(40), &format!("{}A=", "A".repeat(42)), &format!("{}==", "A".repeat(42))] {
            assert!(is_placeholder(good), "{good}");
        }
        for bad in ["", "AAA", "A===", "AAAA AAAA", &"A".repeat(44), &"A".repeat(48), "AA=A", "????", "AAAA\n", "====", "AAAA=="] {
            assert!(!is_placeholder(bad), "{bad:?}");
        }
    }

    #[test]
    fn a_file_name_keeps_its_stem_and_takes_its_extension_from_the_type() {
        let pdf = spec_for("application/pdf").unwrap();
        for (submitted, expected) in [
            ("release notes.exe", "release notes.pdf"),
            ("my:file*name?.png", "myfilename.pdf"),
            ("../../etc/passwd.png", "passwd.pdf"),
            ("////", "attachment.pdf"),
            ("   ...   .png", "attachment.pdf"),
            (".png", "attachment.pdf"),
            ("a.b.c.png", "a.b.c.pdf"),
            ("zażółć gęślą.png", "zażółć gęślą.pdf"),
            ("emoji😀.png", "emoji.pdf"),
            ("x\0y.png", "xy.pdf"),
            ("C:\\dir\\a b.png", "Cdira b.pdf"),
        ] {
            assert_eq!(safe_file_name(submitted, &pdf), expected, "{submitted:?}");
        }
        let long = format!("{}.png", "a".repeat(400));
        assert_eq!(safe_file_name(&long, &pdf), format!("{}.pdf", "a".repeat(200)));
    }
}
