//! The heap cost of the largest `brushset.plist` of each shape that the plist
//! ceilings accept: one that fills [`MAX_PLIST_BYTES`], [`MAX_PLIST_VALUES`]
//! or [`MAX_PLIST_PAYLOAD_BYTES`], or two of them at once.
//!
//! One test, because the counting allocator measures the whole binary.
mod common;
mod counting_alloc;

use brushkit_preview::procreate::{MAX_PLIST_BYTES, MAX_PLIST_PAYLOAD_BYTES, MAX_PLIST_VALUES};
use brushkit_preview::{preview_brushset, PreviewOptions};
use common::{brushset_plist, shared_array_values, shared_arrays_plist, zip_with};
use counting_alloc::{live, peak, reset_peak};
use plist::{Dictionary, Value};

const MIB: usize = 1 << 20;
const BUDGET: usize = 8 * MIB;
/// The root dictionary, its three keys, the set name, the member array, its
/// one member and the payload array. Each payload dictionary adds itself, its
/// key and its value.
const WIDE_DICTIONARIES: usize = (MAX_PLIST_VALUES - 8) / 3;
/// The root dictionary, its two keys, the set name and the member array.
const WIDE_MEMBERS: usize = MAX_PLIST_VALUES - 5;
/// The largest shared-array tree the value guard accepts.
const ACCEPTED_LEVELS: usize = 3;
const ACCEPTED_FANOUT: usize = {
    let mut fanout = 1;
    while shared_array_values(ACCEPTED_LEVELS, fanout + 1) <= MAX_PLIST_VALUES {
        fanout += 1;
    }
    fanout
};

/// A `brushset.plist` whose `payload` holds `dictionaries` one-entry
/// dictionaries. The plist writer shares the equal key and value strings but
/// writes every dictionary.
fn wide_plist(name: &str, dictionaries: usize) -> Value {
    let mut entry = Dictionary::new();
    entry.insert("k".into(), Value::String("v".into()));
    let mut root = Dictionary::new();
    root.insert("name".into(), Value::String(name.into()));
    root.insert(
        "brushes".into(),
        Value::Array(vec![Value::String("a".into())]),
    );
    root.insert(
        "payload".into(),
        Value::Array(vec![Value::Dictionary(entry); dictionaries]),
    );
    Value::Dictionary(root)
}

fn binary(value: &Value) -> Vec<u8> {
    let mut out = Vec::new();
    value.to_writer_binary(&mut out).expect("binary plist");
    out
}

fn xml(value: &Value) -> Vec<u8> {
    let mut out = Vec::new();
    value.to_writer_xml(&mut out).expect("xml plist");
    out
}

/// `xml` with whitespace before `</plist>` up to [`MAX_PLIST_BYTES`].
fn padded_xml(value: &Value) -> Vec<u8> {
    let mut out = xml(value);
    let close = out.len() - b"</plist>".len();
    assert_eq!(&out[close..], b"</plist>");
    let pad = MAX_PLIST_BYTES - out.len();
    out.splice(close..close, std::iter::repeat_n(b' ', pad));
    out
}

/// A binary plist with unreferenced zero bytes after its objects, up to
/// [`MAX_PLIST_BYTES`]. The offset table moves past the padding.
fn padded_binary(value: &Value) -> Vec<u8> {
    let plist = binary(value);
    let trailer = plist.len() - 32;
    let table = u64::from_be_bytes(plist[trailer + 24..].try_into().unwrap()) as usize;
    let pad = MAX_PLIST_BYTES - plist.len();
    let mut out = plist[..table].to_vec();
    out.resize(table + pad, 0);
    out.extend_from_slice(&plist[table..trailer + 24]);
    out.extend_from_slice(&((table + pad) as u64).to_be_bytes());
    out
}

/// A `brushset.plist` listing [`WIDE_MEMBERS`] distinct uuids, each as long
/// as fits [`MAX_PLIST_BYTES`] in `encode`. A uuid costs at least its width.
fn wide_members(encode: fn(&Value) -> Vec<u8>) -> Vec<u8> {
    (1..=MAX_PLIST_BYTES / WIDE_MEMBERS)
        .rev()
        .map(|width| {
            let uuids = (0..WIDE_MEMBERS).map(|i| Value::String(format!("{i:0>width$}")));
            let mut root = Dictionary::new();
            root.insert("name".into(), Value::String("Set".into()));
            root.insert("brushes".into(), Value::Array(uuids.collect()));
            encode(&Value::Dictionary(root))
        })
        .find(|plist| plist.len() <= MAX_PLIST_BYTES)
        .expect("a uuid width fits")
}

#[test]
fn the_largest_accepted_plists_stay_within_the_heap_budget() {
    let xml_name = xml(&wide_plist("", 0));
    let xml_string = "x".repeat(MAX_PLIST_BYTES - xml_name.len());
    // plist decodes an accepted string once per parse, and each decode holds
    // the UTF-16 units and the UTF-8 copy at once. The keys and the member
    // uuid take the other 12 bytes.
    let cjk_string = "\u{5b57}".repeat((MAX_PLIST_PAYLOAD_BYTES - 12) / 3);
    // Every payload dictionary expands its key and value, one byte each.
    let cjk = (MAX_PLIST_PAYLOAD_BYTES - 2 * WIDE_DICTIONARIES - 64) / 3;
    let dictionaries_and_string = wide_plist(&"\u{5b57}".repeat(cjk), WIDE_DICTIONARIES);

    let cases = [
        (
            "xml dictionaries",
            padded_xml(&wide_plist("Set", WIDE_DICTIONARIES)),
        ),
        ("xml string", xml(&wide_plist(&xml_string, 0))),
        ("xml members", wide_members(xml)),
        ("binary string", brushset_plist(&cjk_string, &["a"])),
        (
            "binary dictionaries and string",
            padded_binary(&dictionaries_and_string),
        ),
        ("binary members", wide_members(binary)),
        (
            "shared arrays",
            shared_arrays_plist(ACCEPTED_LEVELS, ACCEPTED_FANOUT),
        ),
    ];
    let mut failures = Vec::new();
    for (name, plist) in &cases {
        assert!(
            plist.len() <= MAX_PLIST_BYTES,
            "{name}: {} bytes",
            plist.len()
        );
        let zip = zip_with(&[("brushset.plist", plist)]);
        let before = live();
        reset_peak();
        let outcome = preview_brushset(&zip, PreviewOptions { max_cell: 8 });
        let growth = peak() - before;
        println!("{name}: {} plist bytes, {growth} heap bytes", plist.len());
        if let Err(e) = outcome {
            failures.push(format!("{name} was rejected: {e}"));
        }
        if growth >= BUDGET {
            failures.push(format!("{name} grew the heap by {growth} bytes"));
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}
