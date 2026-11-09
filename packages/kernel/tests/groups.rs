//! Groups formed by one repack must survive later ingests under other refs
//! (E123).

use ikam_kernel::{Id, Kind, MemStore, Object, Repo, Store, TreeEntry};

fn commit_files<S: Store>(repo: &mut Repo<S>, refname: &str, files: &[Vec<u8>]) {
    let entries = files
        .iter()
        .enumerate()
        .map(|(i, bytes)| TreeEntry {
            name: format!("f{i:03}"),
            kind: Kind::File,
            id: repo.put_content(bytes).unwrap().id,
        })
        .collect();
    let tree = repo.put(&Object::tree(entries).unwrap()).unwrap();
    repo.commit(refname, tree, "").unwrap();
}

/// The id of the group a member entry ("S" group[8] start len) names.
fn group_of<S: Store>(repo: &Repo<S>, member: &Id) -> Option<Id> {
    let entry = repo.store().read(member).ok()?;
    let prefix = entry.get(1..9).filter(|_| entry[0] == b'S')?;
    let found = repo.store().ids_with_prefix(prefix).ok()?;
    found
        .into_iter()
        .find(|g| repo.store().read(g).is_ok_and(|e| e.first() != Some(&b'S')))
}

/// Ingest indexes every stored content, groups included, so a new file can
/// pick a group as the neighbour it anti-unifies with. That plan re-derives
/// the group as a `fill` and drops the group's stored bytes, but members
/// read the group's stored encoding, not its derivation: dropping it lost
/// every member (the 4-ingest corruption of E123). The group must stay
/// stored, and the store must stay fully readable.
#[test]
fn ingest_never_drops_a_group_its_members_read() {
    let mut repo = Repo::new(MemStore::default());
    // Similar small notes, each with a token of its own: they group, and
    // the group's stored bytes stay large (the tokens do not compress). The
    // tokens are long enough for the matcher's seeds to find the group as a
    // neighbour, and the file below differs once per note, so slicing it
    // costs more than the template that credits the group's bytes back.
    let mut seed = 0x9e37_79b9_7f4a_7c15u64;
    let notes: Vec<Vec<u8>> = (0..40)
        .map(|i| {
            let token: String = (0..120)
                .map(|_| {
                    seed = seed
                        .wrapping_mul(6364136223846793005)
                        .wrapping_add(1442695040888963407);
                    char::from(b'a' + (seed >> 59) as u8 % 26)
                })
                .collect();
            format!("{i:02} {token} item\n").into_bytes()
        })
        .collect();
    commit_files(&mut repo, "c/notes", &notes);
    repo.repack().unwrap();
    assert!(repo.fsck().unwrap().is_empty());
    let group = group_of(&repo, &Id::of_content(&notes[0])).expect("the notes are grouped");
    let joined = repo.read_content(&group).unwrap();

    // A second ingest under another ref: one file shaped like the group.
    let similar = String::from_utf8(joined)
        .unwrap()
        .replace("item", "ITEM")
        .into_bytes();
    commit_files(&mut repo, "c/other", std::slice::from_ref(&similar));

    assert!(
        repo.store().has(&group),
        "the group's stored bytes are kept"
    );
    assert!(repo.fsck().unwrap().is_empty());
    for n in notes.iter().chain([&similar]) {
        assert_eq!(&repo.read_content(&Id::of_content(n)).unwrap(), n);
    }
    repo.repack().unwrap();
    assert!(repo.fsck().unwrap().is_empty());
    for n in notes.iter().chain([&similar]) {
        assert_eq!(&repo.read_content(&Id::of_content(n)).unwrap(), n);
    }
}
