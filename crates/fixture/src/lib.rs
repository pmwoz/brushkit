//! Descriptor byte writers for building `.abr` test fixtures.
//!
//! This crate MUST NOT depend on `brushkit-abr`. It is a second, independent
//! reading of the `.abr` format, and a fixture built from a reader's own
//! encoder could not catch a builder and a parser that drifted together. CI
//! asserts the missing dependency edge.
//!
//! Two key encodings appear on the wire and both are reachable here. A
//! length-prefixed key takes `&[u8]`; an OSType key writes a zero length
//! followed by exactly four bytes and takes `&[u8; 4]`.

use byteorder::{BigEndian, WriteBytesExt};

pub fn write_key(buf: &mut Vec<u8>, key: &[u8]) {
    buf.write_u32::<BigEndian>(key.len() as u32).unwrap();
    buf.extend_from_slice(key);
}

pub fn write_bool(buf: &mut Vec<u8>, key: &[u8], val: bool) {
    write_key(buf, key);
    buf.extend_from_slice(b"bool");
    buf.write_u8(val as u8).unwrap();
}

pub fn write_doub_ostype(buf: &mut Vec<u8>, key: &[u8; 4], val: f64) {
    buf.write_u32::<BigEndian>(0).unwrap();
    buf.extend_from_slice(key);
    buf.extend_from_slice(b"doub");
    buf.write_f64::<BigEndian>(val).unwrap();
}

pub fn write_unit_float(buf: &mut Vec<u8>, key: &[u8; 4], unit: &[u8; 4], val: f64) {
    buf.write_u32::<BigEndian>(0).unwrap();
    buf.extend_from_slice(key);
    buf.extend_from_slice(b"UntF");
    buf.extend_from_slice(unit);
    buf.write_f64::<BigEndian>(val).unwrap();
}

pub fn write_named_unit_float(buf: &mut Vec<u8>, key: &[u8], unit: &[u8; 4], val: f64) {
    write_key(buf, key);
    buf.extend_from_slice(b"UntF");
    buf.extend_from_slice(unit);
    buf.write_f64::<BigEndian>(val).unwrap();
}

pub fn write_enum(buf: &mut Vec<u8>, key: &[u8], enum_type: &[u8], value: &[u8]) {
    write_key(buf, key);
    buf.extend_from_slice(b"enum");
    write_key(buf, enum_type);
    write_key(buf, value);
}

/// The full four-item `brVr` shape: `bVTy` control selector, `fStp`, the
/// `jitter` percentage, and a zero `Mnm `.
pub fn write_brvr(buf: &mut Vec<u8>, key: &[u8], control: i32, jitter_pct: f64) {
    write_key(buf, key);
    buf.extend_from_slice(b"Objc");
    buf.write_u32::<BigEndian>(1).unwrap();
    buf.write_u16::<BigEndian>(0).unwrap();
    write_key(buf, b"brVr");
    buf.write_u32::<BigEndian>(4).unwrap();
    write_key(buf, b"bVTy");
    buf.extend_from_slice(b"long");
    buf.write_i32::<BigEndian>(control).unwrap();
    write_key(buf, b"fStp");
    buf.extend_from_slice(b"long");
    buf.write_i32::<BigEndian>(1).unwrap();
    write_key(buf, b"jitter");
    buf.extend_from_slice(b"UntF");
    buf.extend_from_slice(b"#Prc");
    buf.write_f64::<BigEndian>(jitter_pct).unwrap();
    write_key(buf, b"Mnm ");
    buf.extend_from_slice(b"UntF");
    buf.extend_from_slice(b"#Prc");
    buf.write_f64::<BigEndian>(0.0).unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Byte pins. Nothing else in the workspace can catch a "tidy" of these
    /// writers, because every fixture built from them would move together with
    /// the change.
    #[test]
    fn primitives_emit_the_documented_bytes() {
        let mut b = Vec::new();
        write_key(&mut b, b"Spcn");
        assert_eq!(b, [0x00, 0x00, 0x00, 0x04, 0x53, 0x70, 0x63, 0x6E]);

        let mut b = Vec::new();
        write_bool(&mut b, b"useTip", true);
        assert_eq!(
            b,
            [0, 0, 0, 6, b'u', b's', b'e', b'T', b'i', b'p', b'b', b'o', b'o', b'l', 1]
        );

        let mut b = Vec::new();
        write_doub_ostype(&mut b, b"Spcn", 25.0);
        assert_eq!(&b[..12], b"\x00\x00\x00\x00Spcndoub");
        assert_eq!(&b[12..], &25.0f64.to_be_bytes());

        let mut b = Vec::new();
        write_unit_float(&mut b, b"Angl", b"#Ang", 90.0);
        assert_eq!(&b[..16], b"\x00\x00\x00\x00AnglUntF#Ang");
        assert_eq!(&b[16..], &90.0f64.to_be_bytes());

        let mut b = Vec::new();
        write_named_unit_float(&mut b, b"jitter", b"#Prc", 40.0);
        assert_eq!(&b[..18], b"\x00\x00\x00\x06jitterUntF#Prc");
        assert_eq!(&b[18..], &40.0f64.to_be_bytes());

        let mut b = Vec::new();
        write_enum(&mut b, b"BlnM", b"BlnM", b"Nrml");
        assert_eq!(
            b,
            b"\x00\x00\x00\x04BlnMenum\x00\x00\x00\x04BlnM\x00\x00\x00\x04Nrml"
        );
    }

    /// The two key encodings are chosen by the caller here, never inferred from
    /// the key's length the way `abr::writer::write_id` infers it. A four-byte
    /// key therefore has two legal encodings and the fixture picks one on
    /// purpose.
    #[test]
    fn a_four_byte_key_has_two_encodings() {
        let mut named = Vec::new();
        write_key(&mut named, b"Spcn");
        let mut ostype = Vec::new();
        write_doub_ostype(&mut ostype, b"Spcn", 0.0);
        assert_eq!(&named[..4], &[0, 0, 0, 4]);
        assert_eq!(&ostype[..4], &[0, 0, 0, 0]);
    }
}
