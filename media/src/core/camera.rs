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
        // The VP9 P bit describes every packet of the picture. Switch only at B (beginning).
        Codec::Vp9 => {
            payload.first().is_some_and(|header| header & 0x08 != 0)
                && format::detect_vp9_keyframe(payload)
        }
        // Some H.264 encoders send the sequence parameter set separately before the IDR.
        // Its dimensions belong to the incoming layer and must reach the decoder first.
        Codec::H264 => {
            format::detect_h264_nalu(payload, 7) || format::detect_h264_keyframe(payload)
        }
        Codec::Av1 => format::detect_av1_keyframe(payload),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codec_boundaries_include_decoder_configuration_and_exclude_vp9_continuations() {
        assert!(starts_keyframe(Codec::H264, &[0x67, 0x42]));
        assert!(starts_keyframe(Codec::H264, &[0x65, 0x80]));
        assert!(!starts_keyframe(Codec::H264, &[0x61, 0x80]));
        assert!(starts_keyframe(Codec::Vp9, &[0x08]));
        assert!(!starts_keyframe(Codec::Vp9, &[0x04]));
    }

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
