//! Temperature by age (E263): aged contents are re-encoded at the cold
//! read weight and take fewer bytes; hot ones keep their encoding; every
//! version reads the same; a pass with nothing aged does nothing; a
//! content made hot again is promoted by the next pass.

use ikam_kernel::temperature::{COOLED_REF, TemperatureOpts};
use ikam_kernel::{Edit, Id, MemStore, Object, Repo, Store};

/// About the planner's heavy weight in the bench (E069), and its archive
/// weight (E099).
const HEAVY: f64 = 48.8;
const ARCHIVE: f64 = 0.00097;

fn text(file: usize, version: usize) -> Vec<u8> {
    let mut v = Vec::new();
    for line in 0..120 {
        v.extend_from_slice(
            format!("file {file} line {line}: an ordinary sentence of notes, v{version}\n")
                .as_bytes(),
        );
    }
    v
}

fn opts() -> TemperatureOpts {
    TemperatureOpts {
        hot_commits: 3,
        cold_read_weight: ARCHIVE,
    }
}

/// 10 files, 12 commits each rewriting every file.
fn history() -> (Repo<MemStore>, Vec<Vec<Id>>) {
    let mut repo = Repo::new(MemStore::default());
    repo.read_weight = HEAVY;
    let mut versions = Vec::new();
    for v in 0..12 {
        let ids: Vec<Id> = (0..10)
            .map(|f| repo.put_content(&text(f, v)).unwrap().id)
            .collect();
        let edits: Vec<Edit> = ids
            .iter()
            .enumerate()
            .map(|(f, id)| Edit::Put {
                path: format!("f{f}.txt"),
                id: *id,
            })
            .collect();
        let tree = repo.edit_tree(None, &edits).unwrap();
        repo.commit("main", tree, "edit").unwrap();
        versions.push(ids);
    }
    (repo, versions)
}

fn size(repo: &Repo<MemStore>, ids: &[Id]) -> usize {
    ids.iter()
        .map(|id| repo.store().read(id).unwrap().len())
        .sum()
}

fn reads_back(repo: &Repo<MemStore>, versions: &[Vec<Id>]) {
    for (v, ids) in versions.iter().enumerate() {
        for (f, id) in ids.iter().enumerate() {
            assert_eq!(repo.read_content(id).unwrap(), text(f, v));
        }
    }
    assert!(repo.fsck().unwrap().is_empty());
}

#[test]
fn aged_contents_shrink_and_hot_ones_stay() {
    let (mut repo, versions) = history();
    let old: Vec<Id> = versions[..9].concat();
    let new: Vec<Id> = versions[9..].concat();
    let (old_before, new_before) = (size(&repo, &old), size(&repo, &new));
    let encodings: Vec<Vec<u8>> = new
        .iter()
        .map(|id| repo.store().read(id).unwrap())
        .collect();
    let t = repo.temperature_pass(opts()).unwrap();
    // Each commit's 10 files and its tree's content.
    assert_eq!((t.hot, t.cold), (33, 99));
    assert!(t.cooled > 0 && t.promoted == 0, "{t:?}");
    assert!(t.after < t.before, "{t:?}");
    assert!(
        size(&repo, &old) * 10 < old_before * 9,
        "aged contents take 10% less or better"
    );
    assert_eq!(size(&repo, &new), new_before);
    for (id, e) in new.iter().zip(&encodings) {
        assert_eq!(
            &repo.store().read(id).unwrap(),
            e,
            "hot encodings untouched"
        );
    }
    reads_back(&repo, &versions);
    assert!(repo.store().get_ref(COOLED_REF).unwrap().is_some());
    repo.gc().unwrap();
    reads_back(&repo, &versions);
}

#[test]
fn a_second_pass_with_nothing_aged_does_nothing() {
    let (mut repo, versions) = history();
    repo.temperature_pass(opts()).unwrap();
    let ids = repo.store().ids().unwrap();
    let all: Vec<Id> = versions.concat();
    let before = size(&repo, &all);
    let t = repo.temperature_pass(opts()).unwrap();
    assert_eq!((t.cooled, t.promoted, t.before, t.after), (0, 0, 0, 0));
    assert_eq!(size(&repo, &all), before);
    assert_eq!(repo.store().ids().unwrap().len(), ids.len());
}

#[test]
fn a_reverted_version_is_promoted_by_the_next_pass() {
    let (mut repo, versions) = history();
    repo.temperature_pass(opts()).unwrap();
    let cooled = size(&repo, &versions[0][..1]);
    // Revert f0 to its first version: hot again.
    let head = repo.resolve("main").unwrap();
    let tree = match repo.get(&head).unwrap() {
        Object::Commit(c) => c.tree,
        _ => unreachable!(),
    };
    let edit = Edit::Put {
        path: "f0.txt".into(),
        id: versions[0][0],
    };
    let tree = repo.edit_tree(Some(tree), &[edit]).unwrap();
    repo.commit("main", tree, "revert f0").unwrap();
    let t = repo.temperature_pass(opts()).unwrap();
    assert!(t.promoted >= 1, "{t:?}");
    assert!(
        size(&repo, &versions[0][..1]) > cooled,
        "promoted back to a heavy-weight encoding"
    );
    reads_back(&repo, &versions);
}

#[test]
fn nothing_happens_unless_called() {
    let (repo, _) = history();
    assert!(repo.store().get_ref(COOLED_REF).unwrap().is_none());
}
