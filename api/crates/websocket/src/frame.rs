//! The framing layer of RFC 6455: reading a frame header, masking, and
//! writing frames. Nothing here does I/O, so every rule can be tested on bytes.

pub const OP_CONTINUATION: u8 = 0x0;
pub const OP_TEXT: u8 = 0x1;
pub const OP_BINARY: u8 = 0x2;
pub const OP_CLOSE: u8 = 0x8;
pub const OP_PING: u8 = 0x9;
pub const OP_PONG: u8 = 0xA;

/// The most a control frame may carry.
pub const MAX_CONTROL_PAYLOAD: usize = 125;

/// A frame header as read from a client.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    pub fin: bool,
    pub opcode: u8,
    pub len: u64,
    pub mask: [u8; 4],
}

impl Header {
    pub fn is_control(&self) -> bool {
        self.opcode & 0x8 != 0
    }
}

/// Why a header is not acceptable. Every one of them ends the connection
/// with status 1002.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeaderError {
    ReservedBits,
    ReservedOpcode,
    Unmasked,
    FragmentedControl,
    ControlTooLong,
    LengthNotMinimal,
    LengthTooLarge,
}

impl HeaderError {
    pub fn describe(self) -> &'static str {
        match self {
            Self::ReservedBits => "a reserved bit is set",
            Self::ReservedOpcode => "the opcode is reserved",
            Self::Unmasked => "the client did not mask its frame",
            Self::FragmentedControl => "a control frame is fragmented",
            Self::ControlTooLong => "a control frame is longer than 125 bytes",
            Self::LengthNotMinimal => "the length is not in its shortest form",
            Self::LengthTooLarge => "the length has its top bit set",
        }
    }
}

/// Reads one header from the start of `buf`. `Ok(None)` means the bytes so
/// far are a prefix of a header and more are needed; on success the second
/// value is how many bytes the header took, mask included.
pub fn parse_header(buf: &[u8]) -> Result<Option<(Header, usize)>, HeaderError> {
    if buf.len() < 2 {
        return Ok(None);
    }
    let (first, second) = (buf[0], buf[1]);
    if first & 0x70 != 0 {
        return Err(HeaderError::ReservedBits);
    }
    let opcode = first & 0x0f;
    if !matches!(
        opcode,
        OP_CONTINUATION | OP_TEXT | OP_BINARY | OP_CLOSE | OP_PING | OP_PONG
    ) {
        return Err(HeaderError::ReservedOpcode);
    }
    let fin = first & 0x80 != 0;
    let control = opcode & 0x8 != 0;
    if control && !fin {
        return Err(HeaderError::FragmentedControl);
    }
    if second & 0x80 == 0 {
        return Err(HeaderError::Unmasked);
    }
    let short = u64::from(second & 0x7f);
    let (len, length_bytes) = match short {
        126 => {
            if buf.len() < 4 {
                return Ok(None);
            }
            let len = u64::from(u16::from_be_bytes([buf[2], buf[3]]));
            if len < 126 {
                return Err(HeaderError::LengthNotMinimal);
            }
            (len, 2)
        }
        127 => {
            if buf.len() < 10 {
                return Ok(None);
            }
            let mut eight = [0u8; 8];
            eight.copy_from_slice(&buf[2..10]);
            let len = u64::from_be_bytes(eight);
            if len >> 63 != 0 {
                return Err(HeaderError::LengthTooLarge);
            }
            if len < 65536 {
                return Err(HeaderError::LengthNotMinimal);
            }
            (len, 8)
        }
        n => (n, 0),
    };
    if control && len > MAX_CONTROL_PAYLOAD as u64 {
        return Err(HeaderError::ControlTooLong);
    }
    let start = 2 + length_bytes;
    if buf.len() < start + 4 {
        return Ok(None);
    }
    let mask = [buf[start], buf[start + 1], buf[start + 2], buf[start + 3]];
    Ok(Some((
        Header {
            fin,
            opcode,
            len,
            mask,
        },
        start + 4,
    )))
}

/// XORs `data` with the mask key. `offset` is the position of `data` inside
/// the frame's payload, so a payload that arrives in pieces unmasks the same
/// as one that arrives whole.
pub fn apply_mask(data: &mut [u8], key: [u8; 4], offset: usize) {
    for (index, byte) in data.iter_mut().enumerate() {
        *byte ^= key[(offset + index) & 3];
    }
}

/// The header of a frame to send. A server sends unmasked frames; a mask is
/// for tests that play the client.
pub fn encode_header(fin: bool, opcode: u8, len: usize, mask: Option<[u8; 4]>) -> Vec<u8> {
    let mut out = Vec::with_capacity(14);
    out.push(if fin { 0x80 } else { 0 } | opcode);
    let mask_bit = if mask.is_some() { 0x80 } else { 0 };
    if len < 126 {
        out.push(mask_bit | len as u8);
    } else if len < 65536 {
        out.push(mask_bit | 126);
        out.extend_from_slice(&(len as u16).to_be_bytes());
    } else {
        out.push(mask_bit | 127);
        out.extend_from_slice(&(len as u64).to_be_bytes());
    }
    if let Some(key) = mask {
        out.extend_from_slice(&key);
    }
    out
}

/// A whole frame as bytes, masked when a key is given.
pub fn encode_frame(fin: bool, opcode: u8, payload: &[u8], mask: Option<[u8; 4]>) -> Vec<u8> {
    let mut out = encode_header(fin, opcode, payload.len(), mask);
    let start = out.len();
    out.extend_from_slice(payload);
    if let Some(key) = mask {
        apply_mask(&mut out[start..], key, 0);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: [u8; 4] = [0x37, 0xfa, 0x21, 0x3d];

    #[test]
    fn rfc_6455_masked_hello() {
        // Section 5.7: a single-frame masked text message containing "Hello".
        let wire = [
            0x81, 0x85, 0x37, 0xfa, 0x21, 0x3d, 0x7f, 0x9f, 0x4d, 0x51, 0x58,
        ];
        let (header, used) = parse_header(&wire).unwrap().unwrap();
        assert_eq!(
            header,
            Header {
                fin: true,
                opcode: OP_TEXT,
                len: 5,
                mask: KEY
            }
        );
        let mut payload = wire[used..].to_vec();
        apply_mask(&mut payload, header.mask, 0);
        assert_eq!(payload, b"Hello");
        assert_eq!(encode_frame(true, OP_TEXT, b"Hello", Some(KEY)), wire);
    }

    #[test]
    fn rfc_6455_unmasked_server_frames() {
        assert_eq!(
            encode_frame(true, OP_TEXT, b"Hello", None),
            [0x81, 0x05, 0x48, 0x65, 0x6c, 0x6c, 0x6f]
        );
        assert_eq!(
            encode_frame(true, OP_PING, b"Hello", None),
            [0x89, 0x05, 0x48, 0x65, 0x6c, 0x6c, 0x6f]
        );
        // Fragmented "Hel" + "lo".
        assert_eq!(
            encode_frame(false, OP_TEXT, b"Hel", None),
            [0x01, 0x03, 0x48, 0x65, 0x6c]
        );
        assert_eq!(
            encode_frame(true, OP_CONTINUATION, b"lo", None),
            [0x80, 0x02, 0x6c, 0x6f]
        );
    }

    #[test]
    fn rfc_6455_length_forms() {
        // 256 bytes use the 16-bit form, 64 KiB the 64-bit form.
        let h = encode_header(true, OP_BINARY, 256, None);
        assert_eq!(h, [0x82, 0x7e, 0x01, 0x00]);
        let h = encode_header(true, OP_BINARY, 65536, None);
        assert_eq!(h, [0x82, 0x7f, 0, 0, 0, 0, 0, 1, 0, 0]);
    }

    #[test]
    fn a_header_in_pieces_asks_for_more_until_it_is_whole() {
        let wire = encode_frame(true, OP_BINARY, &[7u8; 300], Some(KEY));
        for cut in 0..8 {
            assert_eq!(parse_header(&wire[..cut]), Ok(None), "cut at {cut}");
        }
        let (header, used) = parse_header(&wire[..8]).unwrap().unwrap();
        assert_eq!((header.len, used), (300, 8));
    }

    #[test]
    fn mask_offset_matches_a_payload_unmasked_in_one_piece() {
        let plain: Vec<u8> = (0..23).collect();
        let mut whole = plain.clone();
        apply_mask(&mut whole, KEY, 0);
        let mut pieces = plain.clone();
        let (a, b) = pieces.split_at_mut(7);
        apply_mask(a, KEY, 0);
        apply_mask(b, KEY, 7);
        assert_eq!(whole, pieces);
    }

    #[test]
    fn headers_that_must_be_refused() {
        let cases: [(&[u8], HeaderError); 9] = [
            (&[0xc1, 0x80, 0, 0, 0, 0], HeaderError::ReservedBits),
            (&[0xa1, 0x80, 0, 0, 0, 0], HeaderError::ReservedBits),
            (&[0x91, 0x80, 0, 0, 0, 0], HeaderError::ReservedBits),
            (&[0x83, 0x80, 0, 0, 0, 0], HeaderError::ReservedOpcode),
            (&[0x8b, 0x80, 0, 0, 0, 0], HeaderError::ReservedOpcode),
            (&[0x81, 0x01, 0x41], HeaderError::Unmasked),
            (&[0x09, 0x80, 0, 0, 0, 0], HeaderError::FragmentedControl),
            (
                &[0x89, 0xfe, 0, 126, 0, 0, 0, 0],
                HeaderError::ControlTooLong,
            ),
            (
                &[0x82, 0xfe, 0, 125, 0, 0, 0, 0],
                HeaderError::LengthNotMinimal,
            ),
        ];
        for (wire, expected) in cases {
            assert_eq!(parse_header(wire), Err(expected), "{wire:02x?}");
        }
        assert_eq!(
            parse_header(&[0x82, 0xff, 0, 0, 0, 0, 0, 0, 0xff, 0xff, 0, 0, 0, 0]),
            Err(HeaderError::LengthNotMinimal)
        );
        assert_eq!(
            parse_header(&[0x82, 0xff, 0x80, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]),
            Err(HeaderError::LengthTooLarge)
        );
    }

    #[test]
    fn control_frame_at_the_limit_is_fine() {
        let wire = encode_frame(true, OP_PING, &[1u8; 125], Some(KEY));
        let (header, _) = parse_header(&wire).unwrap().unwrap();
        assert_eq!(header.len, 125);
    }
}
