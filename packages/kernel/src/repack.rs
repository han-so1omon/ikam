//! Repack: re-plan the storage of all live content at once.
//!
//! Ingest decides one file at a time, so the result depends on arrival
//! order and cannot see that a template will pay off over many future
//! files. Repack re-ingests live content into a fresh in-memory store,
//! largest first, re-offering every recorded derivation as a candidate, then
//! drops stored bytes wherever a recorded derivation rebuilds them. It does
//! this twice, with and without template induction (one template per
//! cluster of similar contents, generalized over all of them, recursively
//! over the templates and fillers themselves for as long as the evaluator
//! estimates a saving), and keeps the smaller plan. The plan is verified completely and replaces the old
//! representation only if it is smaller. Ids never change, so trees, commits
//! and refs are untouched.
//!
//! `concat` and `fill` derivations are storage plans and are replaced by the
//! re-plan; the literals, templates and fillers they read are not kept for
//! their own sake. Every other derivation (functions, container packing,
//! proposals recorded through `put_derivation`) is a fact: it is
//! re-recorded, and its inputs are kept.

use std::cmp::Reverse;
use std::collections::{HashMap, HashSet};

use crate::dict::{DICT_REF, DICT_REF_LEN, is_plain, object_tag, tree_content};
use crate::ingest::derivation_size;
use crate::matcher::{self, Index};
use crate::repo::Cx;
use crate::{Arg, Derivation, Error, Id, MemStore, Object, Repo, Store, func, template};

/// Seed neighbours considered for each star centre.
const STAR_CANDIDATES: usize = 64;

#[derive(Debug, PartialEq, Eq)]
pub struct Repacked {
    /// Bytes of content and derivation objects before and after.
    pub before: usize,
    pub after: usize,
    /// False when the re-plan was not smaller and nothing changed.
    pub applied: bool,
}

/// Dictionary candidates re-planned in full per repack (best estimates).
const DICT_SHORTLIST: usize = 3;

impl<S: Store + Sync> Repo<S> {
    pub fn repack(&mut self) -> Result<Repacked, Error> {
        self.unshare_trees()?;
        self.gc()?;
        let (_, roots) = self.reachable()?;
        let (content, _) = self.closure(roots, |d| !func::is_plan(&d.func))?;
        let mut files: Vec<(Id, Vec<u8>)> = content
            .iter()
            .map(|id| Ok((*id, self.read_content(id)?)))
            .collect::<Result<_, Error>>()?;
        files.sort_by_key(|(id, bytes)| (Reverse(bytes.len()), *id));

        // Train only on what will be stored: content stored as bytes (content
        // rebuilt by a fact, e.g. a zip from its members, never is) and the
        // objects that outlive the re-plan (trees, commits, facts), whose
        // repeated content ids a dictionary can hold.
        let mut stored = Vec::new();
        for (id, bytes) in &files {
            if self.derivations(id)?.iter().all(|d| func::is_plan(&d.func)) {
                stored.push(bytes.as_slice());
            }
        }
        let structure = self.lasting_objects()?;
        stored.extend(structure.iter().map(|(_, o)| o.as_slice()));
        // Exact structure (templates) and statistics (a dictionary) compete
        // and combine differently per corpus: try each combination, keep the
        // smallest verified plan.
        // Plans are independent: build them in parallel, then compare them in
        // a fixed order, so the choice never depends on scheduling.
        let dicts = train_dictionaries(&stored, DICT_SHORTLIST);
        let options: Vec<(Option<&[u8]>, bool)> = (dicts.iter().map(|d| Some(d.as_slice())))
            .chain([None])
            .flat_map(|d| [(d, false), (d, true)])
            .collect();
        let plans: Vec<Result<Repo<MemStore>, Error>> = std::thread::scope(|scope| {
            let workers: Vec<_> = (options.iter())
                .map(|&(d, induce)| {
                    let (this, files) = (&*self, &files);
                    scope.spawn(move || this.replan(files, induce, d))
                })
                .collect();
            workers
                .into_iter()
                .map(|w| w.join().expect("planner panicked"))
                .collect()
        });
        let mut fresh: Option<(usize, Repo<MemStore>)> = None;
        for plan in plans {
            let plan = plan?;
            let size = plan.content_bytes()?;
            if fresh.as_ref().is_none_or(|(best, _)| size < *best) {
                fresh = Some((size, plan));
            }
        }
        let (_, fresh) = fresh.expect("at least one plan");
        let (before, after) = (self.content_bytes()?, fresh.content_bytes()?);
        if after >= before {
            return Ok(Repacked {
                before,
                after: before,
                applied: false,
            });
        }
        let keep: HashSet<Id> = fresh.store.ids()?.into_iter().collect();
        for id in &keep {
            // The fresh plan may encode existing content more compactly
            // (e.g. against the new dictionary); both decode identically.
            let encoded = fresh.store.read(id)?;
            if !self.store.write(*id, &encoded)? && self.store.read(id)? != encoded {
                self.store.replace(*id, &encoded)?;
            }
        }
        let refs: HashSet<Id> = self.store.refs()?.into_iter().map(|(_, id)| id).collect();
        for id in self.store.ids()? {
            if !keep.contains(&id) && !refs.contains(&id) && self.is_content_or_derivation(&id)? {
                self.store.delete(&id)?;
            }
        }
        if let Some((dict, _)) = fresh.dictionary() {
            let current = self.store.get_ref(DICT_REF)?;
            self.store.set_ref(DICT_REF, current, *dict)?;
        }
        self.reset_projections();
        // Objects the re-plan did not rebuild move to the new dictionary.
        for (id, canonical) in self.lasting_objects()? {
            let encoded = self.encode_object(&canonical);
            if self.store.read(&id)? != encoded {
                self.store.replace(id, &encoded)?;
            }
        }
        self.gc()?; // drop claims about content the new plan no longer holds
        Ok(Repacked {
            before,
            after,
            applied: true,
        })
    }

    /// A verified fresh store holding exactly `files` and what they need.
    fn replan(
        &self,
        files: &[(Id, Vec<u8>)],
        induce: bool,
        dict: Option<&[u8]>,
    ) -> Result<Repo<MemStore>, Error> {
        let mut fresh = Repo::new(MemStore::default());
        fresh.read_weight = self.read_weight;
        if let Some(dict) = dict {
            fresh.set_dictionary(dict.to_vec())?;
        }
        let mut hints: HashMap<Id, Vec<Derivation>> = HashMap::new();
        for (id, _) in files {
            // Only facts: storage plans from earlier ingests would make the
            // re-plan depend on ingest order.
            let facts = self
                .derivations(id)?
                .into_iter()
                .filter(|d| !func::is_plan(&d.func))
                .collect();
            hints.insert(*id, facts);
        }
        if induce {
            induce_into(&mut fresh, files, &mut hints)?;
        }
        for (id, bytes) in files {
            fresh.put_content_with(bytes, 0, &hints[id])?;
        }
        // Re-record facts whose inputs survived the re-plan.
        for (id, _) in files {
            for d in self.derivations(id)? {
                if !func::is_plan(&d.func)
                    && d.args
                        .iter()
                        .all(|a| fresh.has_content(&a.id()).unwrap_or(false))
                {
                    fresh.record(d)?;
                }
            }
        }
        fresh.drop_derivable_bytes(files)?;
        fresh.prune(files)?;
        for (id, bytes) in files {
            if fresh.read_content(id)? != *bytes {
                return Err(Error::Corrupt(*id));
            }
        }
        if !fresh.fsck()?.is_empty() {
            return Err(Error::Exec("repack: re-plan failed verification".into()));
        }
        Ok(fresh)
    }

    /// Delete everything `files` do not need (unused templates, fillers,
    /// literals and container members left over from rejected candidates).
    fn prune(&mut self, files: &[(Id, Vec<u8>)]) -> Result<(), Error> {
        let (content, records) =
            self.closure(files.iter().map(|(id, _)| *id).collect(), |_| true)?;
        let dict = self.dictionary().map(|(id, _)| *id);
        for id in self.store.ids()? {
            if !content.contains(&id) && !records.contains(&id) && Some(id) != dict {
                self.store.delete(&id)?;
            }
        }
        self.reset_projections();
        Ok(())
    }

    /// Delete stored bytes of content that a recorded derivation rebuilds
    /// without them, largest first. The derivation is already recorded, so
    /// each deletion saves the full stored size. Each deletion is checked against
    /// the current state, so everything stays reconstructible.
    fn drop_derivable_bytes(&mut self, files: &[(Id, Vec<u8>)]) -> Result<(), Error> {
        for (id, bytes) in files {
            if !self.store.has(id) {
                continue;
            }
            let stored = self.store.read(id)?.len() as f64;
            for d in self.derivations(id)? {
                let mut cx = Cx::default();
                cx.stack_guard(*id); // the derivation may not read `id` itself
                let rebuilt = self.eval(&d.func, &d.args, &mut cx);
                // Evaluator: the stored bytes saved must outweigh the decode
                // work now needed to rebuild them.
                if rebuilt.as_ref().is_ok_and(|b| b == bytes)
                    && stored > self.read_weight * cx.work as f64
                {
                    self.store.delete(id)?;
                    break;
                }
            }
        }
        self.reset_projections();
        Ok(())
    }

    /// A tree stored as content pays for its pointer only if that content
    /// is derived or shared; otherwise the tree goes back to object form
    /// (gc then drops the content).
    fn unshare_trees(&mut self) -> Result<(), Error> {
        let used: HashSet<Id> = (self.ledger()?.by_output.values().flatten())
            .flat_map(|d| d.args.iter().map(Arg::id))
            .collect();
        for id in self.store.ids()? {
            if let Some(content) = tree_content(&self.store.read(&id)?)
                && self.store.has(&content)
                && !used.contains(&content)
            {
                let encoded = self.encode_object(&self.read_object(&id)?);
                self.store.replace(id, &encoded)?;
            }
        }
        Ok(())
    }

    /// Canonical encodings of stored trees, commits, claims and fact
    /// derivations: the objects a re-plan keeps as they are. Trees stored as
    /// content are not among them: the re-plan covers that content.
    fn lasting_objects(&self) -> Result<Vec<(Id, Vec<u8>)>, Error> {
        let mut out = Vec::new();
        for id in self.store.ids()? {
            let encoded = self.store.read(&id)?;
            if object_tag(&encoded).is_none() || tree_content(&encoded).is_some() {
                continue;
            }
            let canonical = self.read_object(&id)?;
            match Object::decode(&canonical)? {
                Object::Derivation(d) if func::is_plan(&d.func) => {}
                _ => out.push((id, canonical)),
            }
        }
        out.sort();
        Ok(out)
    }

    fn is_content_or_derivation(&self, id: &Id) -> Result<bool, Error> {
        let encoded = self.store.read(id)?;
        Ok(is_plain(&encoded) || object_tag(&encoded) == Some(b'D'))
    }

    pub(crate) fn content_bytes(&self) -> Result<usize, Error> {
        let mut total = 0;
        for id in self.store.ids()? {
            if self.is_content_or_derivation(&id)? {
                total += self.store.read(&id)?.len();
            }
        }
        Ok(total)
    }
}

/// Induce one template per cluster of similar `items`, offering each member
/// a `fill` hint. A cluster is kept only if the evaluator estimates it
/// shrinks storage (template + fillers + records, encoded, against the
/// members' own encodings). The templates and fillers kept are content too,
/// so they are clustered and templated again, with no fixed number of
/// levels: every kept level strictly shrinks the encoded total, so the
/// recursion ends. Ingest's cost comparison then decides which hints are
/// used; unused templates and fillers are pruned later.
fn induce_into(
    fresh: &mut Repo<MemStore>,
    items: &[(Id, Vec<u8>)],
    hints: &mut HashMap<Id, Vec<Derivation>>,
) -> Result<(), Error> {
    let mut artifacts: Vec<(Id, Vec<u8>)> = Vec::new();
    for group in template_groups(items) {
        let members: Vec<&[u8]> = group.iter().map(|&i| items[i].1.as_slice()).collect();
        let Some(t) = template::induce(&members) else {
            continue;
        };
        let t_id = Id::of_content(&t);
        let fitted: Vec<(usize, Vec<u8>)> = group
            .iter()
            .filter_map(|&i| template::fit(&t, &items[i].1).map(|f| (i, f)))
            .collect();
        let fill = |output, f| Derivation {
            output,
            func: func::fill(),
            args: vec![Arg::Whole(t_id), Arg::Whole(f)],
        };
        let before: usize = fitted
            .iter()
            .map(|(i, _)| fresh.encode_plain(&items[*i].1).len())
            .sum();
        let after: usize = fresh.encode_plain(&t).len()
            + fitted
                .iter()
                .map(|(i, f)| {
                    fresh.encode_plain(f).len()
                        + derivation_size(&fill(items[*i].0, Id::of_content(f)))
                })
                .sum::<usize>();
        if fitted.len() < 2 || after >= before {
            continue;
        }
        for (i, f) in fitted {
            hints
                .entry(items[i].0)
                .or_default()
                .push(fill(items[i].0, Id::of_content(&f)));
            artifacts.push((Id::of_content(&f), f));
        }
        artifacts.push((t_id, t));
    }
    if artifacts.is_empty() {
        return Ok(());
    }
    artifacts.sort_by_key(|(id, bytes)| (Reverse(bytes.len()), *id));
    artifacts.dedup_by_key(|(id, _)| *id);
    induce_into(fresh, &artifacts, hints)?;
    for (id, bytes) in &artifacts {
        fresh.put_content_with(bytes, 0, hints.get(id).map_or(&[][..], Vec::as_slice))?;
    }
    Ok(())
}

/// Star clusters (indices into `files`) of contents worth a shared template.
/// Largest unassigned content first becomes a centre; its seed neighbours
/// join if aligning with the centre shares more bytes than a `fill` record
/// costs. Stars avoid transitive chaining of unrelated contents, and the
/// test is absolute (shared bytes), so contents dominated by unique payload
/// still join when their boilerplate is shared.
fn template_groups(files: &[(Id, Vec<u8>)]) -> Vec<Vec<usize>> {
    let mut index = Index::default();
    for (id, bytes) in files {
        index.add(*id, bytes);
    }
    let position: HashMap<Id, usize> = files
        .iter()
        .enumerate()
        .map(|(i, (id, _))| (*id, i))
        .collect();
    let record = derivation_size(&Derivation {
        output: Id::of(b""),
        func: func::fill(),
        args: vec![Arg::Whole(Id::of(b"")), Arg::Whole(Id::of(b""))],
    });
    let mut assigned = vec![false; files.len()];
    let mut groups = Vec::new();
    for centre in 0..files.len() {
        if assigned[centre] {
            continue;
        }
        assigned[centre] = true;
        let mut group = vec![centre];
        for (n, _) in matcher::neighbors(&files[centre].1, &index, STAR_CANDIDATES).0 {
            let Some(&j) = position.get(&n) else { continue };
            if !assigned[j] && template::shared(&files[centre].1, &files[j].1) > record {
                assigned[j] = true;
                group.push(j);
            }
        }
        if group.len() >= 2 {
            groups.push(group);
        }
    }
    groups
}

/// Candidate zstd dictionaries for `samples`, best estimate first, at most
/// `k`, each estimated to save more than its own size. Candidates are
/// trained (COVER, zstd's trainer) and raw (evenly spaced pieces of the
/// samples themselves, as in relative Lempel-Ziv) at a few sizes: neither
/// construction nor any size wins on every corpus (COVER can return a tiny
/// dictionary when asked for a large one). The estimate ignores dedup, so
/// the re-plans decide among the shortlist.
fn train_dictionaries(samples: &[&[u8]], k: usize) -> Vec<Vec<u8>> {
    let samples: Vec<&[u8]> = samples.iter().copied().filter(|b| !b.is_empty()).collect();
    let total: usize = samples.iter().map(|b| b.len()).sum();
    let Some(alone) = dictionary_cost(&samples, &[]).filter(|_| samples.len() >= 8) else {
        return vec![];
    };
    let mut ranked = Vec::new();
    for size in [total / 100, total / 30, total / 10].map(|s| s.clamp(4096, 112_640)) {
        let trained = zstd::dict::from_samples(&samples, size).ok();
        for dict in trained.into_iter().chain([raw_dictionary(&samples, size)]) {
            if let Some(cost) = dictionary_cost(&samples, &dict).filter(|&c| c < alone)
                && !ranked.iter().any(|(_, d)| *d == dict)
            {
                ranked.push((cost, dict));
            }
        }
    }
    ranked.sort();
    ranked.into_iter().take(k).map(|(_, d)| d).collect()
}

/// Estimated stored size of `samples` given `dict` (empty: none): the
/// dictionary plus each sample's smallest encoding, a dictionary encoding
/// paying for the id it names.
fn dictionary_cost(samples: &[&[u8]], dict: &[u8]) -> Option<usize> {
    let mut with_dict = zstd::bulk::Compressor::with_dictionary(3, dict).ok()?;
    with_dict
        .set_parameter(zstd::zstd_safe::CParameter::DictIdFlag(false))
        .ok()?;
    let mut cost = dict.len();
    for b in samples {
        let z = zstd::bulk::compress(b, 3).map_or(b.len(), |z| z.len().min(b.len()));
        cost += match dict.is_empty() {
            true => z,
            false => with_dict
                .compress(b)
                .map_or(z, |d| (d.len() + DICT_REF_LEN).min(z)),
        };
    }
    Some(cost)
}

/// A raw-content dictionary: `size` bytes of 1 KiB pieces taken at even
/// steps through the samples (Hoobin, Puglisi and Zobel, PVLDB 2011).
fn raw_dictionary(samples: &[&[u8]], size: usize) -> Vec<u8> {
    const PIECE: usize = 1024;
    let joined = samples.concat();
    let step = (joined.len() / (size / PIECE).max(1)).max(PIECE);
    joined
        .chunks(step)
        .filter_map(|c| c.get(..PIECE))
        .take(size / PIECE)
        .flatten()
        .copied()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ingest is greedy, so pair templates may overfit; induction finds one
    /// template generalized over the whole cluster, and no document keeps
    /// its bytes.
    #[test]
    fn induction_shares_one_template_across_a_cluster() {
        let invoice = |n: u32| -> Vec<u8> {
            (0..20u32)
                .map(|k| {
                    let v = (n.wrapping_mul(2654435761) ^ k.wrapping_mul(40503)) % 100_000;
                    format!("<section id=\"{k}\"><label>Line item {k}: standard terms and conditions apply; see master agreement clause {k}.</label><value>{v:x}</value></section>\n")
                })
                .collect::<String>()
                .into_bytes()
        };
        let files: Vec<(Id, Vec<u8>)> = (0..30)
            .map(invoice)
            .map(|d| (Id::of_content(&d), d))
            .collect();
        let plan = Repo::new(MemStore::default())
            .replan(&files, true, None)
            .unwrap();
        let templates: Vec<Id> = files
            .iter()
            .map(|(id, _)| {
                let d = plan.derivations(id).unwrap();
                d.into_iter()
                    .find(|d| d.func == func::fill())
                    .expect("fill")
                    .args[0]
                    .id()
            })
            .collect();
        assert!(
            templates.iter().all(|t| *t == templates[0]),
            "one shared template"
        );
        assert!(
            files.iter().all(|(id, _)| !plan.store().has(id)),
            "no document keeps its bytes"
        );
    }

    /// Templates and fillers are content too, so induction templates them
    /// again, with no fixed number of levels: in the induced plan for a
    /// revision history, some template or filler list is itself a fill.
    #[test]
    fn induction_layers_templates_without_a_level_limit() {
        let spec = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../docs/ikam/ikam-sheet-specification.md");
        let mut docs = vec![std::fs::read_to_string(spec).unwrap()];
        for i in 1..=20usize {
            let mut lines: Vec<String> = docs.last().unwrap().lines().map(String::from).collect();
            let k = (i * 37) % lines.len();
            lines[k].push_str(&format!(" (edit {i})"));
            let mut v = lines.join("\n") + "\n";
            if i % 5 == 0 {
                v.push_str(&format!(
                    "\n## Added section {i}\nNew text for revision {i}.\n"
                ));
            }
            docs.push(v);
        }
        let files: Vec<(Id, Vec<u8>)> = docs
            .into_iter()
            .map(|d| (Id::of_content(d.as_bytes()), d.into_bytes()))
            .collect();
        let plan = Repo::new(MemStore::default())
            .replan(&files, true, None)
            .unwrap();
        let fill_of = |id: &Id| {
            plan.derivations(id)
                .unwrap()
                .into_iter()
                .find(|d| d.func == func::fill())
        };
        let level1: Vec<Id> = files
            .iter()
            .filter_map(|(id, _)| fill_of(id))
            .flat_map(|d| d.args.into_iter().map(|a| a.id()))
            .collect();
        assert!(!level1.is_empty(), "revisions use templates");
        assert!(
            level1.iter().any(|part| fill_of(part).is_some()),
            "some template or fillers are themselves templated"
        );
    }
}
