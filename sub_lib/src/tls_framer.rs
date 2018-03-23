// Copyright (c) 2017-2018, Substratum LLC (https://substratum.net) and/or its affiliates. All rights reserved.
use framer::FramedChunk;
use framer::Framer;
use utils::index_of;

const PRESERVE_HEADER_LEN: usize = 4;
const MAX_ALLOWED_LEN: usize = 16384;

pub struct TlsFramer {
    data_so_far: Vec<u8>
}

impl Framer for TlsFramer {
    fn add_data(&mut self, data: &[u8]) {
        self.data_so_far.extend(data);
    }

    fn take_frame(&mut self) -> Option<FramedChunk> {
        let data_so_far_len = self.data_so_far.len ();
        match TlsFramer::find_frame_offset (&self.data_so_far[..]) {
            None => {
                let split_point = if data_so_far_len < PRESERVE_HEADER_LEN {data_so_far_len} else {data_so_far_len - PRESERVE_HEADER_LEN};
                self.data_so_far = self.data_so_far.split_off (split_point);
                None
            },
            Some (offset) => {
                let mut from_offset = self.data_so_far.split_off (offset);
                let length = TlsFramer::to_usize(from_offset[3], from_offset[4]);
                if 5 + length <= from_offset.len () {
                    let leftovers = from_offset.split_off (5 + length);
                    let chunk = from_offset;
                    self.data_so_far = leftovers;
                    Some (FramedChunk { chunk, last_chunk: true })
                }
                else {
                    self.data_so_far = from_offset;
                    None
                }
            }
        }
    }
}

impl TlsFramer {
    pub fn new() -> TlsFramer {
        TlsFramer {
            data_so_far: Vec::new()
        }
    }

    fn find_frame_offset (data: &[u8]) -> Option<usize> {
        let mut accumulated_offset = 0;
        loop {
            match TlsFramer::search_for_frame_offset (&data[accumulated_offset..]) {
                Ok (offset) => return Some (accumulated_offset + offset),
                Err (0) => return None,
                Err (next_offset) => accumulated_offset += next_offset
            }
        }
    }

    fn search_for_frame_offset (data: &[u8]) -> Result<usize, usize> {
        match index_of (data, &[0x03]) {
            None => Err (0), // Err (0) means don't bother trying again
            Some (0) => Err (1), // Err (x) means try again starting at x
            Some (possible) => {
                let offset = possible - 1;
                if offset + 4 >= data.len () {return Err (0)} // Err (0) means don't bother trying again
                let length = TlsFramer::to_usize(data[offset + 3], data[offset + 4]);
                if TlsFramer::is_valid_content_type(data[offset + 0]) &&
                    TlsFramer::is_valid_protocol_version(data[offset + 1], data[offset + 2]) &&
                    TlsFramer::is_valid_length(length) {
                    Ok (offset)
                } else {
                    Err (offset + 1) // Err (x) means try again starting at x
                }
            }
        }
    }

    fn is_valid_content_type(candidate: u8) -> bool {
        (candidate >= 0x14) && (candidate <= 0x17)
    }

    fn is_valid_protocol_version(byte1: u8, byte2: u8) -> bool {
        (byte1 == 0x03) && ((byte2 == 0x01) || (byte2 == 0x03))
    }

    fn is_valid_length (length: usize) -> bool {
        length <= MAX_ALLOWED_LEN
    }

    fn to_usize(hi_byte: u8, lo_byte: u8) -> usize {
        ((hi_byte as usize) << 8) | (lo_byte as usize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tls_framer_discards_all_but_a_few_bytes_of_unrecognized_data() {
        let mut subject = TlsFramer::new();

        subject.add_data(&vec!(0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07)[..]);
        let result = subject.take_frame();

        assert_eq!(result, None);
        assert_eq!(subject.data_so_far, vec!(0x04, 0x05, 0x06, 0x07));
    }

    #[test]
    fn tls_framer_rejects_unrecognized_content_type() {
        let mut subject = TlsFramer::new();

        subject.add_data(&vec!(0x00, 0x03, 0x03, 0x00, 0x03, 0x05, 0x06, 0x07)[..]);
        let result = subject.take_frame();

        assert_eq!(result, None);
        assert_eq!(subject.data_so_far, vec!(0x03, 0x05, 0x06, 0x07));
    }

    #[test]
    fn tls_framer_rejects_unrecognized_first_tls_version_byte() {
        let mut subject = TlsFramer::new();

        subject.add_data(&vec!(0x14, 0x01, 0x03, 0x00, 0x03, 0x05, 0x06, 0x07)[..]);
        let result = subject.take_frame();

        assert_eq!(result, None);
        assert_eq!(subject.data_so_far, vec!(0x03, 0x05, 0x06, 0x07));
    }

    #[test]
    fn tls_framer_rejects_unrecognized_second_tls_version_byte() {
        let mut subject = TlsFramer::new();

        subject.add_data(&vec!(0x15, 0x03, 0x02, 0x00, 0x03, 0x05, 0x06, 0x07)[..]);
        let result = subject.take_frame();

        assert_eq!(result, None);
        assert_eq!(subject.data_so_far, vec!(0x03, 0x05, 0x06, 0x07));
    }

    #[test]
    fn tls_framer_rejects_illegal_data_length () {
        let mut subject = TlsFramer::new ();
        let length = MAX_ALLOWED_LEN + 1;
        let byte1 = (length >> 8) as u8;
        let byte2 = (length & 0xFF) as u8;

        subject.add_data (&vec! (0x16, 0x03, 0x03, byte1, byte2, 0x05, 0x06, 0x07)[..]);
        let result = subject.take_frame ();

        assert_eq! (result, None);
        assert_eq! (subject.data_so_far, vec! (byte2, 0x05, 0x06, 0x07));
    }

    #[test]
    fn tls_framer_recognizes_four_content_types() {
        vec!(0x14u8, 0x15u8, 0x16u8, 0x17u8).iter().for_each(|content_type| {
            let mut subject = TlsFramer::new();

            subject.add_data(&vec!(*content_type, 0x03, 0x03, 0x00, 0x03, 0x01, 0x02, 0x03)[..]);
            let result = subject.take_frame();

            assert_eq!(result, Some(FramedChunk { chunk: vec!(*content_type, 0x03, 0x03, 0x00, 0x03, 0x01, 0x02, 0x03), last_chunk: true }));
            assert_eq!(subject.data_so_far, vec!());
        });
    }

    #[test]
    fn tls_framer_recognizes_two_tls_versions () {
        vec! (0x0301, 0x0303).iter ().for_each (|version| {
            let mut subject = TlsFramer::new ();
            let byte1 = (version >> 8) as u8;
            let byte2 = (version & 0xFF) as u8;

            subject.add_data (&vec! (0x17, byte1, byte2, 0x00, 0x03, 0x01, 0x02, 0x03)[..]);
            let result = subject.take_frame ();

            assert_eq! (result, Some (FramedChunk {chunk: vec! (0x17, byte1, byte2, 0x00, 0x03, 0x01, 0x02, 0x03), last_chunk: true}));
            assert_eq! (subject.data_so_far, vec! ());
        });
    }

    #[test]
    fn tls_framer_recognizes_packet_with_maximum_length () {
        let mut subject = TlsFramer::new ();
        let length = MAX_ALLOWED_LEN;
        let byte1 = (length >> 8) as u8;
        let byte2 = (length & 0xFF) as u8;

        subject.add_data (&vec! (0x16, 0x03, 0x03, byte1, byte2, 0x05, 0x06, 0x07)[..]);
        let result = subject.take_frame ();

        // We get no result here because we don't have 16K of data yet, but we can see that the
        // Framer is on the job because it has not discarded all but the last PRESERVE_HEADER_LEN bytes of data.
        assert_eq! (result, None);
        assert_eq! (subject.data_so_far, vec! (0x16, 0x03, 0x03, byte1, byte2, 0x05, 0x06, 0x07));
    }

    #[test]
    fn tls_framer_skips_garbage () {
        let mut subject = TlsFramer::new ();

        subject.add_data (&vec! (
            0x15, 0x03, 0x03, 0x80, // garbage
            0x15, 0x03, 0x03, 0x00, 0x03, 0x05, 0x06, 0x07, // packet
            0x01, 0x02 // garbage
        )[..]);
        let result = subject.take_frame ();

        assert_eq! (result, Some (FramedChunk {chunk: vec! (0x15, 0x03, 0x03, 0x00, 0x03, 0x05, 0x06, 0x07), last_chunk: true}));
        assert_eq! (subject.data_so_far, vec! (0x01, 0x02));
    }

    #[test]
    fn tls_framer_handles_multiple_packets () {
        let mut subject = TlsFramer::new ();

        subject.add_data (&vec! (
            0x14, 0x03, 0x03, 0x80, // garbage
            0x14, 0x03, 0x03, 0x00, 0x03, 0x05, 0x06, 0x07, // packet
            0x03, 0x02, // garbage
            0x17, 0x03, 0x01, 0x00, 0x02, 0x08, 0x09, // packet
            0x02, 0x01, // garbage
            0x16, 0x03, 0x03, 0x00, 0x01, 0x0A, // packet
            0xFF, 0xFF, 0xFF // garbage
        )[..]);
        let result1 = subject.take_frame ();
        let result2 = subject.take_frame ();
        let result3 = subject.take_frame ();
        let result4 = subject.take_frame ();

        assert_eq! (result1, Some (FramedChunk {chunk: vec! (0x14, 0x03, 0x03, 0x00, 0x03, 0x05, 0x06, 0x07), last_chunk: true}));
        assert_eq! (result2, Some (FramedChunk {chunk: vec! (0x17, 0x03, 0x01, 0x00, 0x02, 0x08, 0x09), last_chunk: true}));
        assert_eq! (result3, Some (FramedChunk {chunk: vec! (0x16, 0x03, 0x03, 0x00, 0x01, 0x0A), last_chunk: true}));
        assert_eq! (result4, None);
    }
}
