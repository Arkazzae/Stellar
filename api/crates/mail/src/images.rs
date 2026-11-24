//! The pictures the messages carry. They are rendered from the brand files:
//! the lockup at three times the 36 px it is shown at, the stickers at twice
//! their 112 px.

use stellarr_smtp::InlineImage;

pub(crate) fn lockup() -> InlineImage {
    image(
        "lockup@stellarr",
        "lockup.png",
        include_bytes!("../assets/lockup.png"),
    )
}

pub(crate) fn mention() -> InlineImage {
    image(
        "mention@stellarr",
        "mention.png",
        include_bytes!("../assets/mention.png"),
    )
}

pub(crate) fn direct_message() -> InlineImage {
    image(
        "direct-message@stellarr",
        "direct-message.png",
        include_bytes!("../assets/direct-message.png"),
    )
}

fn image(content_id: &str, file_name: &str, content: &[u8]) -> InlineImage {
    InlineImage {
        content_id: content_id.to_owned(),
        file_name: file_name.to_owned(),
        content_type: "image/png".to_owned(),
        content: content.to_vec(),
    }
}
