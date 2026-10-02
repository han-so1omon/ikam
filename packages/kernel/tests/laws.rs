//! Property tests for the kernel laws in docs/plans/2026-10-02-rust-kernel.md.

use ikam_kernel::codec::{Body, Cdc, Markdown, Raw};
use ikam_kernel::{
    Codec, Entry, Error, FsStore, Id, Kind, MemStore, Object, Shape, Store, ingest, render,
};
use proptest::prelude::*;

fn any_object() -> impl Strategy<Value = Object> {
    let entry =
        (".{0,12}", any::<bool>(), any::<[u8; 32]>()).prop_map(|(label, is_blob, id)| Entry {
            label,
            kind: if is_blob { Kind::Blob } else { Kind::Node },
            id: Id::from_bytes(id),
        });
    prop_oneof![
        prop::collection::vec(any::<u8>(), 0..256).prop_map(Object::Blob),
        prop::collection::vec(entry, 0..8).prop_map(Object::Node),
    ]
}

/// Markdown-ish documents: headings, prose, blank lines, fences, no trailing
/// newline, CRLF — the cases a line splitter tends to get wrong.
fn markdownish() -> impl Strategy<Value = Vec<u8>> {
    let line = prop_oneof![
        Just("# Title\n".to_string()),
        Just("## Sub\r\n".to_string()),
        Just("\n".to_string()),
        Just("   \n".to_string()),
        Just("```\n".to_string()),
        Just("shared paragraph line\n".to_string()),
        "[a-z #]{0,20}\n",
        "[a-z]{0,8}",
    ];
    prop::collection::vec(line, 0..40).prop_map(|ls| ls.concat().into_bytes())
}

/// A codec that lies: its shape skips the last byte.
struct Lossy;

impl Codec for Lossy {
    fn name(&self) -> &str {
        "lossy"
    }

    fn split(&self, input: &[u8]) -> Option<Shape> {
        Some(Shape::leaf("", 0..input.len().saturating_sub(1)))
    }
}

/// A codec that proposes overlapping leaves.
struct Overlapping;

impl Codec for Overlapping {
    fn name(&self) -> &str {
        "overlap"
    }

    fn split(&self, input: &[u8]) -> Option<Shape> {
        let half = input.len() / 2;
        Some(Shape::group(
            "",
            vec![
                Shape::leaf("", 0..half + 1),
                Shape::leaf("", half..input.len()),
            ],
        ))
    }
}

fn roundtrip(codec: &dyn Codec, input: &[u8]) -> ikam_kernel::Ingested {
    let mut store = MemStore::default();
    let r = ingest(&mut store, codec, input).unwrap();
    assert_eq!(
        render(&store, &r.root).unwrap(),
        input,
        "codec {}",
        codec.name()
    );
    r
}

proptest! {
    #[test]
    fn encoding_is_canonical(obj in any_object()) {
        let bytes = obj.encode();
        prop_assert_eq!(&Object::decode(&bytes).unwrap(), &obj);
        prop_assert_eq!(Object::decode(&bytes).unwrap().encode(), bytes);
    }

    #[test]
    fn decode_never_panics(bytes in prop::collection::vec(any::<u8>(), 0..128)) {
        if let Ok(obj) = Object::decode(&bytes) {
            prop_assert_eq!(obj.encode(), bytes);
        }
    }

    #[test]
    fn store_get_put(obj in any_object()) {
        let mut store = MemStore::default();
        let (id, fresh) = store.put(&obj).unwrap();
        prop_assert!(fresh);
        prop_assert_eq!(store.get(&id).unwrap(), obj.clone());
        prop_assert!(!store.put(&obj).unwrap().1);
        prop_assert_eq!(store.len(), 1);
    }

    #[test]
    fn raw_and_cdc_roundtrip(input in prop::collection::vec(any::<u8>(), 0..20_000)) {
        prop_assert!(!roundtrip(&Raw, &input).fell_back);
        let small = Cdc { min: 64, avg: 256, max: 1024 };
        prop_assert!(!roundtrip(&small, &input).fell_back);
    }

    #[test]
    fn markdown_roundtrip(input in markdownish()) {
        prop_assert!(!roundtrip(&Markdown, &input).fell_back);
    }

    #[test]
    fn markdown_roundtrip_arbitrary_bytes(input in prop::collection::vec(any::<u8>(), 0..2_000)) {
        prop_assert!(!roundtrip(&Markdown, &input).fell_back);
    }

    #[test]
    fn bad_codecs_fall_back_losslessly(input in prop::collection::vec(any::<u8>(), 1..512)) {
        prop_assert!(roundtrip(&Lossy, &input).fell_back);
        prop_assert!(roundtrip(&Overlapping, &input).fell_back);
    }
}

#[test]
fn markdown_shape_follows_headings_and_paragraphs() {
    let doc = b"intro\n\n# A\none\ntwo\n\nthree\n```\n\n# not a heading\n```\n# B\nfour";
    let Body::Group(sections) = Markdown.split(doc).unwrap().body else {
        panic!()
    };
    let titles: Vec<_> = sections.iter().map(|s| s.label.as_str()).collect();
    assert_eq!(titles, ["", "# A", "# B"]);
    let Body::Group(a_paras) = &sections[1].body else {
        panic!()
    };
    // heading line, "one two" paragraph, and "three" plus the whole fence.
    assert_eq!(a_paras.len(), 3);
}

#[test]
fn shared_paragraphs_are_stored_once() {
    let mut store = MemStore::default();
    let shared = "A paragraph that both documents quote verbatim.\n\n";
    let a = format!("# One\n{shared}alpha\n");
    let b = format!("# Two\n{shared}beta\n");
    let ra = ingest(&mut store, &Markdown, a.as_bytes()).unwrap();
    let rb = ingest(&mut store, &Markdown, b.as_bytes()).unwrap();
    assert_ne!(ra.root, rb.root);
    // b's heading, its "beta" paragraph, its section node and its root are
    // new; the shared paragraph is not written again.
    assert_eq!(rb.objects_written, 4);
    assert_eq!(render(&store, &rb.root).unwrap(), b.as_bytes());
}

#[test]
fn fs_store_detects_tampering() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = FsStore::open(dir.path()).unwrap();
    let (id, _) = store.put(&Object::Blob(b"hello".to_vec())).unwrap();
    assert_eq!(store.get(&id).unwrap(), Object::Blob(b"hello".to_vec()));

    let hex = id.to_hex();
    let path = dir.path().join("objects").join(&hex[..2]).join(&hex[2..]);
    std::fs::write(path, b"Bjello").unwrap();
    assert!(matches!(store.get(&id), Err(Error::Corrupt(_))));
}

#[test]
fn id_hex_roundtrip_and_rejects_garbage() {
    let id = Object::Blob(vec![1, 2, 3]).id();
    assert_eq!(id.to_hex().parse::<Id>().unwrap(), id);
    assert!("xyz".parse::<Id>().is_err());
    assert!(id.to_hex().to_uppercase().parse::<Id>().is_err());
}
