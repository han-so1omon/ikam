//! E410: float content stored as byte planes ("V") with `Repo::float_planes`.

use ikam_kernel::{Edit, MemStore, Repo, Store};

fn floats(seed: u64, n: usize) -> Vec<u8> {
    let mut x = seed;
    (0..n)
        .flat_map(|_| {
            x = x
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let u = (x >> 40) as f32 / (1u64 << 24) as f32 - 0.5;
            (u * 0.04).to_le_bytes()
        })
        .collect()
}

fn stored_tags(repo: &Repo<MemStore>) -> Vec<u8> {
    let s = repo.store();
    s.ids()
        .unwrap()
        .iter()
        .map(|id| s.read(id).unwrap()[0])
        .collect()
}

#[test]
fn float_content_is_stored_as_planes_and_reads_back() {
    // Float runs of several lengths (one not a multiple of 4), and text.
    let mut contents: Vec<Vec<u8>> = vec![floats(1, 40_000), floats(2, 5_000)];
    let mut odd = floats(3, 9_000);
    odd.extend_from_slice(b"xy");
    contents.push(odd);
    contents.push(
        (0..4000)
            .flat_map(|i| format!("line {i} of some text\n").into_bytes())
            .collect(),
    );
    for on in [false, true] {
        let mut repo = Repo::new(MemStore::default());
        repo.float_planes = on;
        let ids: Vec<_> = contents
            .iter()
            .map(|c| repo.put_content(c).unwrap().id)
            .collect();
        let edits: Vec<_> = ids
            .iter()
            .enumerate()
            .map(|(i, id)| Edit::Put {
                path: format!("c/{i}"),
                id: *id,
            })
            .collect();
        let tree = repo.edit_tree(None, &edits).unwrap();
        repo.commit("main", tree, "load").unwrap();
        repo.repack().unwrap();
        let planes = stored_tags(&repo).iter().filter(|&&t| t == b'V').count();
        assert_eq!(planes > 0, on, "planes stored: {planes}");
        for (id, c) in ids.iter().zip(&contents) {
            assert_eq!(repo.read_content(id).unwrap(), *c);
            // A range from the middle decodes the whole frame, then cuts it.
            let (start, len) = (c.len() as u64 / 3, 1001u64);
            assert_eq!(
                repo.read_range(id, start, len).unwrap().0,
                c[start as usize..(start + len) as usize]
            );
        }
        assert!(repo.fsck_deep().unwrap().is_empty());
    }
}
