//! The allow-list of upload types, the signature of each, and the limits.

use stellarr_storage::UploadOptions;
use stellarr_storage::validation::{
    HEADER_LENGTH, UploadKind, UploadSpec, image, matches_signature, max_bytes, spec_for,
};

const PNG: &[u8] = &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0, 0];
const JPEG: &[u8] = &[0xFF, 0xD8, 0xFF, 0xE0, 0, 0];
const GIF: &[u8] = b"GIF89a\0\0";
const WEBP: &[u8] = b"RIFF\0\0\0\0WEBPVP8 ";
const WAVE: &[u8] = b"RIFF\0\0\0\0WAVEfmt ";
const ISO_MEDIA: &[u8] = &[0, 0, 0, 16, b'f', b't', b'y', b'p', 1, 2, 3, 4];
const WEBM: &[u8] = &[0x1A, 0x45, 0xDF, 0xA3, 0, 0];
const ZIP: &[u8] = &[b'P', b'K', 3, 4, 0, 0];
const OGG: &[u8] = b"OggS\0\0";

fn spec(content_type: &str) -> UploadSpec {
    spec_for(content_type).unwrap_or_else(|| panic!("{content_type} is not an accepted type"))
}

const DOCX: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.document";
const XLSX: &str = "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet";
const PPTX: &str = "application/vnd.openxmlformats-officedocument.presentationml.presentation";

#[test]
fn declared_types_map_to_their_kind_folder_and_extension() {
    use UploadKind::*;
    let known = [
        ("image/jpeg", Image, "images", ".jpg"),
        ("image/png", Image, "images", ".png"),
        ("image/gif", Image, "images", ".gif"),
        ("image/webp", Image, "images", ".webp"),
        ("audio/mpeg", Audio, "audio", ".mp3"),
        ("audio/ogg", Audio, "audio", ".ogg"),
        ("audio/wav", Audio, "audio", ".wav"),
        ("audio/webm", Audio, "audio", ".webm"),
        ("audio/mp4", Audio, "audio", ".m4a"),
        ("audio/aac", Audio, "audio", ".aac"),
        ("video/mp4", Video, "videos", ".mp4"),
        ("video/webm", Video, "videos", ".webm"),
        ("video/ogg", Video, "videos", ".ogv"),
        ("video/quicktime", Video, "videos", ".mov"),
        ("application/pdf", File, "files", ".pdf"),
        ("application/zip", File, "files", ".zip"),
        ("text/plain", File, "files", ".txt"),
        ("application/json", File, "files", ".json"),
        (DOCX, File, "files", ".docx"),
        (XLSX, File, "files", ".xlsx"),
        (PPTX, File, "files", ".pptx"),
    ];
    for (content_type, kind, folder, extension) in known {
        let found = spec(content_type);
        assert_eq!(found.content_type, content_type);
        assert_eq!(found.kind, kind, "{content_type}");
        assert_eq!(found.folder, folder, "{content_type}");
        assert_eq!(found.extension, extension, "{content_type}");
    }
}

#[test]
fn aliases_case_and_parameters_resolve_to_the_canonical_type() {
    for (declared, canonical) in [
        ("audio/x-wav", "audio/wav"),
        ("audio/x-m4a", "audio/mp4"),
        ("video/x-m4v", "video/mp4"),
        ("application/x-zip-compressed", "application/zip"),
        ("IMAGE/PNG", "image/png"),
        ("text/plain; charset=utf-8", "text/plain"),
        ("  application/json ;charset=utf-8", "application/json"),
        ("image/png;", "image/png"),
        ("image/png ; a=b; c=d", "image/png"),
        ("Video/X-M4V", "video/mp4"),
    ] {
        assert_eq!(spec(declared).content_type, canonical, "{declared}");
    }
}

#[test]
fn unknown_types_are_refused() {
    for declared in [
        "",
        " ",
        ";",
        ";image/png",
        "image/svg+xml",
        "text/html",
        "application/x-msdownload",
        "application/octet-stream",
        "image",
        "image/",
        "/png",
        "image/png\0",
        "image/p\0ng",
        "image/png x",
        "image /png",
        // Letters that only match after Unicode case mapping.
        "\u{131}mage/png",
        "image/png\u{212A}",
        "application/pd\u{17F}",
        "audio/mp\u{FF34}",
    ] {
        assert!(spec_for(declared).is_none(), "{declared:?}");
    }
}

#[test]
fn a_body_that_starts_like_its_declared_type_is_accepted() {
    let matching: &[(&str, &[u8])] = &[
        ("image/jpeg", JPEG),
        ("image/png", PNG),
        ("image/gif", GIF),
        ("image/gif", b"GIF87a\0\0"),
        ("image/webp", WEBP),
        ("audio/mpeg", b"ID3\x04\0"),
        ("audio/mpeg", &[0xFF, 0xFB, 0x90, 0x00]),
        ("audio/ogg", OGG),
        ("audio/wav", WAVE),
        ("audio/webm", WEBM),
        ("audio/mp4", ISO_MEDIA),
        ("audio/aac", &[0xFF, 0xF1, 0x50, 0x80]),
        ("audio/aac", &[0xFF, 0xF9, 0x50, 0x80]),
        ("video/mp4", ISO_MEDIA),
        ("video/webm", WEBM),
        ("video/ogg", OGG),
        ("video/quicktime", ISO_MEDIA),
        ("application/pdf", b"%PDF-1.7\n"),
        ("application/zip", ZIP),
        ("application/zip", &[b'P', b'K', 5, 6]),
        ("application/zip", &[b'P', b'K', 7, 8]),
        ("text/plain", b"hello"),
        ("application/json", b"  \r\n\t{\"a\":1}"),
        ("application/json", b"[1]"),
        (DOCX, ZIP),
        (XLSX, ZIP),
        (PPTX, ZIP),
    ];
    for (content_type, header) in matching {
        assert!(
            matches_signature(&spec(content_type), header),
            "{content_type}"
        );
    }
}

#[test]
fn a_body_that_does_not_start_like_its_declared_type_is_refused() {
    let foreign: &[(&str, &[u8])] = &[
        ("image/png", JPEG),
        ("image/jpeg", PNG),
        ("image/gif", b"GIF90a"),
        ("image/webp", WAVE),
        ("audio/mpeg", b"not really an mp3"),
        ("audio/mpeg", &[0xFF, 0xE0]),
        ("audio/ogg", b"OGGS"),
        ("audio/wav", WEBP),
        ("audio/webm", ZIP),
        ("audio/mp4", &[0, 0, 0, 16, b'f', b't', b'y']),
        ("audio/aac", &[0xFF, 0xFB]),
        ("video/mp4", WEBM),
        ("application/pdf", b"PDF-1.7 and more"),
        ("application/zip", &[b'P', b'K', 1, 2]),
        ("application/zip", b"PK"),
        ("text/plain", &[0x68, 0x00, 0x69]),
        ("text/plain", b""),
        ("application/json", b"plain words"),
        ("application/json", b"   \n"),
        ("application/json", b"{\0}"),
        (DOCX, b"%PDF-"),
    ];
    for (content_type, header) in foreign {
        assert!(
            !matches_signature(&spec(content_type), header),
            "{content_type}"
        );
    }
}

#[test]
fn an_alias_is_checked_against_the_signature_of_its_canonical_type() {
    let wave = spec("audio/x-wav");
    assert!(matches_signature(&wave, WAVE));
    assert!(!matches_signature(&wave, PNG));
}

#[test]
fn size_limits_come_from_the_upload_options_of_each_kind() {
    let options = UploadOptions {
        max_image_bytes: 1,
        max_audio_bytes: 2,
        max_video_bytes: 3,
        max_file_bytes: 4,
        ..UploadOptions::default()
    };
    let limit = |content_type: &str| max_bytes(&spec(content_type), &options);
    assert_eq!(limit("image/png"), 1);
    assert_eq!(limit("audio/ogg"), 2);
    assert_eq!(limit("video/webm"), 3);
    assert_eq!(limit("application/pdf"), 4);
    assert_eq!(limit("application/json"), 4);
}

#[test]
fn default_limits_leave_room_for_video_and_keep_images_small() {
    let options = UploadOptions::default();
    const MIB: i64 = 1024 * 1024;
    assert_eq!(options.max_image_bytes, 10 * MIB);
    assert_eq!(options.max_audio_bytes, 25 * MIB);
    assert_eq!(options.max_video_bytes, 100 * MIB);
    assert_eq!(options.max_file_bytes, 25 * MIB);
    assert_eq!(options.max_stored_bytes_per_user, 2 * 1024 * MIB);
    assert_eq!(options.max_stored_bytes, 20 * 1024 * MIB);
    assert_eq!(options.minimum_free_data_root_bytes, 1024 * MIB);
    assert_eq!(options.max_concurrent_uploads, 4);
}

#[test]
fn concurrent_uploads_are_kept_between_one_and_thirty_two() {
    for (set, used) in [
        (i32::MIN, 1),
        (-5, 1),
        (0, 1),
        (1, 1),
        (4, 4),
        (32, 32),
        (33, 32),
        (i32::MAX, 32),
    ] {
        let options = UploadOptions {
            max_concurrent_uploads: set,
            ..UploadOptions::default()
        };
        assert_eq!(options.concurrent_uploads(), used, "{set}");
    }
}

#[test]
fn images_are_recognised_by_type_and_get_their_extension() {
    for (content_type, extension, allowed) in [
        ("image/png", Some(".png"), true),
        ("IMAGE/JPEG", Some(".jpg"), true),
        ("image/gif", Some(".gif"), true),
        ("image/webp", Some(".webp"), true),
        ("image/svg+xml", None, false),
        ("audio/mpeg", None, false),
        ("", None, false),
    ] {
        assert_eq!(
            image::is_allowed_content_type(content_type),
            allowed,
            "{content_type}"
        );
        assert_eq!(
            image::extension_for(content_type),
            extension,
            "{content_type}"
        );
    }
}

#[test]
fn image_signatures_are_checked_per_type() {
    assert!(image::matches_signature("image/png", PNG));
    assert!(image::matches_signature("IMAGE/PNG", PNG));
    assert!(!image::matches_signature("image/png", JPEG));
    assert!(!image::matches_signature("image/png", &PNG[..7]));
    assert!(!image::matches_signature("image/svg+xml", b"<svg"));
    assert!(
        !image::matches_signature("audio/wav", WAVE),
        "only image types match here"
    );
}

/// One valid header of every type, and how many of its first bytes the check
/// needs: no shorter prefix may pass, and every longer one must.
fn minimal_headers() -> Vec<(&'static str, Vec<u8>, usize)> {
    vec![
        ("image/jpeg", JPEG.to_vec(), 3),
        ("image/png", PNG.to_vec(), 8),
        ("image/gif", GIF.to_vec(), 6),
        ("image/webp", WEBP.to_vec(), 12),
        ("audio/mpeg", b"ID3\x04\0".to_vec(), 3),
        ("audio/ogg", OGG.to_vec(), 4),
        ("audio/wav", WAVE.to_vec(), 12),
        ("audio/webm", WEBM.to_vec(), 4),
        ("audio/mp4", ISO_MEDIA.to_vec(), 12),
        ("audio/aac", vec![0xFF, 0xF1, 0x50, 0x80], 2),
        ("video/mp4", ISO_MEDIA.to_vec(), 12),
        ("video/webm", WEBM.to_vec(), 4),
        ("video/ogg", OGG.to_vec(), 4),
        ("video/quicktime", ISO_MEDIA.to_vec(), 12),
        ("application/pdf", b"%PDF-1.7\n".to_vec(), 5),
        ("application/zip", ZIP.to_vec(), 4),
        ("text/plain", b"hello".to_vec(), 1),
        ("application/json", b"  \r\n\t{\"a\":1}".to_vec(), 6),
        (DOCX, ZIP.to_vec(), 4),
        (XLSX, ZIP.to_vec(), 4),
        (PPTX, ZIP.to_vec(), 4),
    ]
}

#[test]
fn a_truncated_file_is_accepted_from_exactly_the_bytes_its_check_needs() {
    for (content_type, header, needed) in minimal_headers() {
        let spec = spec(content_type);
        for length in 0..=header.len() {
            assert_eq!(
                matches_signature(&spec, &header[..length]),
                length >= needed,
                "{content_type} cut to {length} bytes"
            );
        }
    }
    // An mp3 frame header needs two bytes.
    let mp3 = spec("audio/mpeg");
    assert!(!matches_signature(&mp3, &[0xFF]));
    assert!(matches_signature(&mp3, &[0xFF, 0xFB]));
}

/// Bytes an attacker may send: no check panics, reads past the end or
/// accepts a type it was not declared as, for any length up to the header
/// that is read.
#[test]
fn hostile_headers_are_refused_without_a_panic() {
    let specs: Vec<(&str, UploadSpec)> = minimal_headers()
        .iter()
        .map(|(t, _, _)| (*t, spec(t)))
        .collect();

    let mut hostile: Vec<Vec<u8>> = vec![
        vec![],
        vec![0],
        vec![0xFF; HEADER_LENGTH],
        vec![0; HEADER_LENGTH],
        vec![0xFF; 3],
        b"RIFF".to_vec(),
        b"RIFF\0\0\0\0".to_vec(),
        b"RIFF\0\0\0\0WEB".to_vec(),
        b"RIFF\0\0\0\0WEBQ".to_vec(),
        b"ftyp".to_vec(),
        b"\0\0\0\x10ftyq1234".to_vec(),
        b"\0\0\0\x10ftyp".to_vec(),
        b"\0\0\0ftyp\0\0\0\0".to_vec(),
        vec![b'P', b'K', 0, 0],
        vec![b'P', b'K', 3, 5],
        vec![b'P', b'K', 4, 3],
        vec![0xEF, 0xBB, 0xBF, b'{', b'}'],
        b"\xFF\xFE{\0}".to_vec(),
        b"{\0".to_vec(),
        b"\0{".to_vec(),
        b"   \t\r\n   ".to_vec(),
        b"  x{".to_vec(),
        b"%PDF".to_vec(),
        b"%pdf-".to_vec(),
        b"id3".to_vec(),
        b"GIF89".to_vec(),
        b"gif89a".to_vec(),
        vec![0xFF, 0xE1],
        vec![0xFF, 0xE3],
        vec![0xFF, 0xF2],
        vec![0xFF, 0xF6],
    ];
    // Every valid header with one byte changed, at every place.
    for (_, header, _) in minimal_headers() {
        for at in 0..header.len() {
            let mut changed = header.clone();
            changed[at] ^= 0x80;
            hostile.push(changed);
            let mut zeroed = header.clone();
            zeroed[at] = 0;
            hostile.push(zeroed);
        }
        // And a valid header buried after other bytes.
        let mut shifted = vec![0x20u8; 7];
        shifted.extend_from_slice(&header);
        hostile.push(shifted);
        hostile.push(
            header
                .iter()
                .cycle()
                .take(HEADER_LENGTH * 4)
                .copied()
                .collect(),
        );
    }
    for header in &hostile {
        for length in 0..=header.len() {
            for (_, spec) in &specs {
                // The only requirement is that nothing panics or reads out of bounds.
                let _ = matches_signature(spec, &header[..length]);
            }
        }
    }

    // Binary data is not text, however it starts.
    let text = spec("text/plain");
    let json = spec("application/json");
    for position in 1..HEADER_LENGTH {
        let mut data = vec![b' '; HEADER_LENGTH];
        data[0] = b'{';
        data[position] = 0;
        assert!(!matches_signature(&text, &data), "NUL at {position}");
        assert!(!matches_signature(&json, &data), "NUL at {position}");
    }
    // Signatures of one type are not the signatures of every other.
    for (declared, header, _) in minimal_headers() {
        for (other, spec) in &specs {
            let same_family = declared == *other
                || (header == ZIP && matches!(*other, "application/zip" | DOCX | XLSX | PPTX))
                || (header == ISO_MEDIA && matches!(*other, "audio/mp4" | "video/mp4" | "video/quicktime"))
                || (header == WEBM && matches!(*other, "audio/webm" | "video/webm"))
                || (header == OGG && matches!(*other, "audio/ogg" | "video/ogg"))
                // Plain text is whatever has no NUL.
                || *other == "text/plain"
                || (*other == "application/json" && header.iter().find(|b| !b" \r\n\t".contains(b)).is_some_and(|b| matches!(b, b'{' | b'[')));
            if !same_family {
                assert!(
                    !matches_signature(spec, &header),
                    "{declared} header passes as {other}"
                );
            }
        }
    }
}
