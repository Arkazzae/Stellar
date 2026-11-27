//! One spatial encoding per receiver. Each encoding has its own RTP sequence
//! space; the receiver keeps its current picture until the next keyframe of
//! the encoding it requested.
use std::time::Instant;

use str0m::format::{self, Codec};
use str0m::media::Rid;

use super::KEYFRAME_INTERVAL;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum Layer {
    Thumbnail,
    #[default]
    Main,
}

impl Layer {
    pub fn from_rid(rid: Rid) -> Option<Self> {
        match &*rid {
            "thumb" => Some(Self::Thumbnail),
            "main" => Some(Self::Main),
            _ => None,
        }
    }
}

#[derive(Debug, Default)]
pub(super) struct Selection {
    pub wanted: Layer,
    current: Option<Layer>,
    requested_at: Option<Instant>,
}

impl Selection {
    pub fn pause(&mut self) {
        self.current = None;
        self.requested_at = None;
    }

    /// Returns (forward this packet, request a keyframe).
    pub fn accepts(&mut self, layer: Layer, keyframe: bool, now: Instant) -> (bool, bool) {
        if layer == self.wanted && self.current != Some(layer) {
            if keyframe {
                self.current = Some(layer);
                self.requested_at = None;
            } else if self
                .requested_at
                .is_none_or(|last| now - last >= KEYFRAME_INTERVAL)
            {
                self.requested_at = Some(now);
                return (false, true);
            }
        }
        (self.current == Some(layer), false)
    }
}

pub(super) fn starts_keyframe(codec: Codec, payload: &[u8]) -> bool {
    match codec {
        Codec::Vp8 => format::detect_vp8_keyframe(payload),
        Codec::Vp9 => format::detect_vp9_keyframe(payload),
        Codec::H264 => format::detect_h264_keyframe(payload),
        Codec::Av1 => format::detect_av1_keyframe(payload),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_switch_waits_for_a_keyframe_and_keeps_the_previous_encoding_until_then() {
        let now = Instant::now();
        let mut camera = Selection::default();
        assert_eq!(camera.accepts(Layer::Main, false, now), (false, true));
        assert_eq!(camera.accepts(Layer::Main, false, now), (false, false));
        assert_eq!(camera.accepts(Layer::Main, true, now), (true, false));

        camera.wanted = Layer::Thumbnail;
        assert_eq!(camera.accepts(Layer::Thumbnail, false, now), (false, true));
        assert_eq!(camera.accepts(Layer::Main, false, now), (true, false));
        assert_eq!(
            camera.accepts(Layer::Thumbnail, false, now + KEYFRAME_INTERVAL),
            (false, true)
        );
        assert_eq!(
            camera.accepts(Layer::Thumbnail, true, now + KEYFRAME_INTERVAL),
            (true, false)
        );
        assert_eq!(
            camera.accepts(Layer::Main, true, now + KEYFRAME_INTERVAL),
            (false, false)
        );

        camera.pause();
        assert_eq!(
            camera.accepts(Layer::Thumbnail, false, now + KEYFRAME_INTERVAL),
            (false, true)
        );
    }
}
