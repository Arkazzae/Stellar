//! Which kind of picture came back.
//!
//! A preview picture comes from a stranger and is about to be served from
//! this origin, so its bytes have to be what its header claims, as for a
//! file a member uploads. Four kinds are accepted.

/// The picture formats kept as previews.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageKind {
    Jpeg,
    Png,
    Gif,
    Webp,
}

impl ImageKind {
    /// The kind a `Content-Type` media type names, in any case.
    pub fn from_media_type(media_type: &str) -> Option<ImageKind> {
        match media_type.to_ascii_lowercase().as_str() {
            "image/jpeg" => Some(ImageKind::Jpeg),
            "image/png" => Some(ImageKind::Png),
            "image/gif" => Some(ImageKind::Gif),
            "image/webp" => Some(ImageKind::Webp),
            _ => None,
        }
    }

    /// The kind the first bytes of a file say it is.
    pub fn sniff(bytes: &[u8]) -> Option<ImageKind> {
        if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
            Some(ImageKind::Jpeg)
        } else if bytes.starts_with(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]) {
            Some(ImageKind::Png)
        } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
            Some(ImageKind::Gif)
        } else if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
            Some(ImageKind::Webp)
        } else {
            None
        }
    }

    pub fn content_type(self) -> &'static str {
        match self {
            ImageKind::Jpeg => "image/jpeg",
            ImageKind::Png => "image/png",
            ImageKind::Gif => "image/gif",
            ImageKind::Webp => "image/webp",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            ImageKind::Jpeg => ".jpg",
            ImageKind::Png => ".png",
            ImageKind::Gif => ".gif",
            ImageKind::Webp => ".webp",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_signatures_agree_for_each_kind() {
        for (kind, bytes) in [
            (ImageKind::Jpeg, &b"\xff\xd8\xff\xe0rest"[..]),
            (ImageKind::Png, b"\x89PNG\r\n\x1a\n\0\0"),
            (ImageKind::Gif, b"GIF87a..."),
            (ImageKind::Gif, b"GIF89a..."),
            (ImageKind::Webp, b"RIFF\x10\0\0\0WEBPVP8 "),
        ] {
            assert_eq!(ImageKind::sniff(bytes), Some(kind));
            assert_eq!(ImageKind::from_media_type(kind.content_type()), Some(kind));
        }
    }

    #[test]
    fn rejects_what_is_not_one_of_the_four() {
        for bytes in [
            &b""[..],
            b"<script>alert(1)</script>",
            b"<svg xmlns='http://www.w3.org/2000/svg'/>",
            b"RIFF\0\0\0\0WAVEfmt ",
            b"RIFF\0\0\0\0WEB",
            b"GIF90a",
            b"\xff\xd8",
            b"BM\0\0",
        ] {
            assert_eq!(ImageKind::sniff(bytes), None, "{bytes:?}");
        }
        for media_type in [
            "image/svg+xml",
            "image/jpg",
            "image/avif",
            "text/html",
            "image/",
            "",
        ] {
            assert_eq!(ImageKind::from_media_type(media_type), None, "{media_type}");
        }
        assert_eq!(
            ImageKind::from_media_type("IMAGE/PNG"),
            Some(ImageKind::Png)
        );
    }
}
