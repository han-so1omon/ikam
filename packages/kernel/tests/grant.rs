//! E140: section grants hand out and accept only the granted ranges.

use ikam_kernel::{Denied, FileGrant, Grant, Id, MemStore, Repo, windows_around};

fn doc() -> Vec<u8> {
    let mut v = Vec::new();
    for i in 0..60 {
        if i == 10 || i == 40 {
            v.extend_from_slice(format!("line {i}: revenue in Q3 grew\n").as_bytes());
        } else {
            v.extend_from_slice(format!("line {i}: nothing to see here\n").as_bytes());
        }
    }
    v
}

fn grant(b: &[u8], id: Id) -> Grant {
    let w = windows_around(b, b"Q3", 2);
    Grant {
        goal: "Q3 to Q4".into(),
        files: vec![FileGrant::new("notes/a.md", id, w.clone(), w)],
    }
}

fn replace(b: &[u8], from: &[u8], to: &[u8], nth: usize) -> Vec<u8> {
    let at = memchr::memmem::find_iter(b, from).nth(nth).unwrap();
    [&b[..at], to, &b[at + from.len()..]].concat()
}

#[test]
fn windows_cover_the_lines_around_each_mention() {
    let b = doc();
    let w = windows_around(&b, b"Q3", 2);
    assert_eq!(w.len(), 2);
    for r in &w {
        let text = &b[r.start as usize..r.end as usize];
        assert_eq!(text.split(|&c| c == b'\n').count() - 1, 5);
        assert!(memchr::memmem::find(text, b"Q3").is_some());
    }
}

#[test]
fn reads_hand_out_only_the_granted_ranges() {
    let mut repo = Repo::new(MemStore::default());
    let b = doc();
    let id = repo.put_content(&b).unwrap().id;
    let g = grant(&b, id);
    let f = g.file("notes/a.md").unwrap();
    let s = repo.session(1 << 20);
    let got = f.read(&s).unwrap();
    for (r, bytes) in &got {
        assert_eq!(&b[r.start as usize..r.end as usize], bytes.as_slice());
    }
    assert!(f.granted_bytes() < b.len() as u64 / 4);
    assert_eq!(
        g.may_read("notes/a.md", &id, 0..10),
        Err(Denied::Outside(0..10))
    );
    assert_eq!(g.may_read("notes/b.md", &id, 0..10), Err(Denied::NoFile));
    let r = f.read[0].clone();
    assert_eq!(
        g.may_read("notes/a.md", &id, r.start + 1..r.end - 1),
        Ok(())
    );
}

#[test]
fn edits_inside_the_windows_pass_and_others_are_refused() {
    let b = doc();
    let id = Id::of_content(&b);
    let g = grant(&b, id);
    // Both mentions changed in one edit: two regions, both granted.
    let both = replace(&replace(&b, b"Q3", b"Q4", 1), b"Q3", b"Q4", 0);
    assert_eq!(g.check_edit("notes/a.md", &id, &b, &both).unwrap().len(), 2);
    // A line added inside a window.
    let added = replace(&b, b"revenue in Q3", b"revenue in Q3 (restated)", 0);
    assert!(g.check_edit("notes/a.md", &id, &b, &added).is_ok());
    // A change far from any mention.
    let far = replace(&b, b"line 25:", b"line XX:", 0);
    assert!(matches!(
        g.check_edit("notes/a.md", &id, &b, &far),
        Err(Denied::Outside(_))
    ));
    // A granted change and an ungranted one together.
    let mixed = replace(&far, b"Q3", b"Q4", 0);
    assert!(matches!(
        g.check_edit("notes/a.md", &id, &b, &mixed),
        Err(Denied::Outside(_))
    ));
    // Another file, another version.
    assert_eq!(
        g.check_edit("notes/b.md", &id, &b, &both),
        Err(Denied::NoFile)
    );
    let other = Id::of_content(&both);
    assert_eq!(
        g.check_edit("notes/a.md", &other, &both, &b),
        Err(Denied::Stale)
    );
}

#[test]
fn an_edit_in_repeated_bytes_is_judged_by_the_alignment_that_fits() {
    // The window holds the middle of a run of the same byte; an insertion
    // there is minimal on the left (at the run's end) and on the right
    // (at its start). Only the right alignment is inside the window.
    let mut b = vec![b'x'; 100];
    b.push(b'\n');
    let id = Id::of_content(&b);
    let f = FileGrant::new(
        "a",
        id,
        std::iter::once(0..20).collect(),
        std::iter::once(0..20).collect(),
    );
    let mut new = b.clone();
    new.insert(10, b'x');
    assert!(f.check_edit(&b, &new).is_ok());
}

#[test]
fn sealed_grants_verify_and_changed_grants_do_not() {
    let b = doc();
    let id = Id::of_content(&b);
    let g = grant(&b, id);
    let key = [7u8; 32];
    let tag = g.seal(&key);
    assert!(g.verify(&key, &tag));
    assert!(!g.verify(&[8u8; 32], &tag));
    let mut wider = g.clone();
    wider.files[0].write.push(0..b.len() as u64);
    assert!(!wider.verify(&key, &tag));
    // The encoding does not depend on the order files are listed in.
    let mut two = g.clone();
    two.files.push(FileGrant::new(
        "b",
        id,
        std::iter::once(0..1).collect(),
        vec![],
    ));
    let mut swapped = two.clone();
    swapped.files.reverse();
    assert_eq!(two.seal(&key), swapped.seal(&key));
}
