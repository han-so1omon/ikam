//! E411: safetensors files cut at tensor boundaries with `Repo::tensor_cuts`.

use ikam_kernel::{Edit, MemStore, Repo, Store};

fn safetensors(tensors: &[Vec<u8>]) -> Vec<u8> {
    let mut header = String::from("{\"__metadata__\":{\"format\":\"pt\"}");
    let mut at = 0;
    for (i, t) in tensors.iter().enumerate() {
        header.push_str(&format!(
            ",\"t{i}\":{{\"dtype\":\"F32\",\"shape\":[{}],\"data_offsets\":[{at},{}]}}",
            t.len() / 4,
            at + t.len()
        ));
        at += t.len();
    }
    header.push('}');
    while header.len() % 8 != 0 {
        header.push(' ');
    }
    let mut out = (header.len() as u64).to_le_bytes().to_vec();
    out.extend(header.as_bytes());
    for t in tensors {
        out.extend(t);
    }
    out
}

fn tensor(seed: u64, n: usize) -> Vec<u8> {
    let mut x = seed;
    (0..n)
        .flat_map(|_| {
            x = x
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (((x >> 40) as f32) / (1u64 << 24) as f32 - 0.5).to_le_bytes()
        })
        .collect()
}

#[test]
fn checkpoints_cut_at_tensors_read_back_exact() {
    // Two checkpoints that share their first two tensors, and a file that
    // only looks like one (a bad header) to be stored whole.
    let a = vec![tensor(1, 20_000), tensor(2, 30_000), tensor(3, 10_000)];
    let mut b = a.clone();
    b[2] = tensor(4, 10_000);
    let mut bad = safetensors(&a);
    bad[8] = b'[';
    let files = [safetensors(&a), safetensors(&b), bad];
    let mut objects = Vec::new();
    for on in [false, true] {
        let mut repo = Repo::new(MemStore::default());
        repo.tensor_cuts = on;
        let ids: Vec<_> = files
            .iter()
            .map(|f| repo.put_content(f).unwrap().id)
            .collect();
        let edits: Vec<_> = ids
            .iter()
            .enumerate()
            .map(|(i, id)| Edit::Put {
                path: format!("m/{i}.safetensors"),
                id: *id,
            })
            .collect();
        let tree = repo.edit_tree(None, &edits).unwrap();
        repo.commit("main", tree, "load").unwrap();
        for round in 0..2 {
            for (id, f) in ids.iter().zip(&files) {
                assert_eq!(repo.read_content(id).unwrap(), *f, "{on} {round}");
                let (start, len) = (f.len() as u64 / 2, 5000u64);
                assert_eq!(
                    repo.read_range(id, start, len).unwrap().0,
                    f[start as usize..(start + len) as usize]
                );
            }
            assert!(repo.fsck_deep().unwrap().is_empty());
            repo.repack().unwrap();
        }
        objects.push(repo.store().ids().unwrap().len());
    }
    // Cut, each tensor is its own content: more objects than whole files.
    assert!(objects[1] > objects[0], "{objects:?}");
}
