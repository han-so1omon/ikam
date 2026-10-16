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
use std::collections::{BTreeSet, HashMap, HashSet};

use crate::dict::{
    DICT_REF, DICT_REF_LEN, MEASURE_LEVEL, STORE_LEVEL, block_frames, framer, is_plain, object_tag,
    tree_content, zstd_frame,
};
use crate::group::{GROUP_REF_LEN, member_span};
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
const DICT_SHORTLIST: usize = 4;

impl<S: Store + Sync> Repo<S> {
    /// Re-plan storage of all live content (applied only if smaller and
    /// verified), then rebuild the function library graph, and store a search
    /// index if it pays for the described search workload. Trees whose
    /// stored content the new plan no longer shares go back to object form
    /// (as before a re-plan), so a second repack finds nothing left (E060).
    pub fn repack(&mut self) -> Result<Repacked, Error> {
        let repacked = self.replan_store()?;
        self.function_library()?;
        self.unshare_trees()?;
        self.gc()?;
        // A search index, if it pays for the described workload (E065).
        let (_, roots) = self.reachable()?;
        let roots: Vec<Id> = roots
            .into_iter()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        self.plan_search_index(&roots)?;
        Ok(repacked)
    }

    fn replan_store(&mut self) -> Result<Repacked, Error> {
        self.unshare_trees()?;
        self.gc()?;
        let (_, roots) = self.reachable()?;
        let roots: Vec<Id> = roots
            .into_iter()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let (content, _) = self.closure(roots.clone(), |d| !func::is_plan(&d.func))?;
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
        let mut fresh: Option<(f64, Repo<MemStore>)> = None;
        for plan in plans {
            let plan = plan?;
            let cost = plan.plan_cost(&roots)?;
            if fresh.as_ref().is_none_or(|(best, _)| cost < *best) {
                fresh = Some((cost, plan));
            }
        }
        let (_, mut fresh) = fresh.expect("at least one plan");
        fresh.recompress(&roots)?;
        let (before, after) = (self.content_bytes()?, fresh.content_bytes()?);
        // Accept by the whole store, as the score counts it: the objects
        // that outlive the re-plan (trees, commits, graph chunks) move to the
        // new dictionary too, so their sizes change with it (E060).
        let (mut kept_now, mut kept_fresh) = (0, 0);
        for (id, canonical) in self.lasting_objects()? {
            if canonical.first() != Some(&b'D') {
                kept_now += self.store.read(&id)?.len();
                kept_fresh += fresh.encode_object(&canonical).len();
            }
        }
        if fresh.plan_cost(&roots)? + kept_fresh as f64 >= self.plan_cost(&roots)? + kept_now as f64
        {
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
        self.level = STORE_LEVEL;
        for (id, canonical) in self.lasting_objects()? {
            let encoded = self.encode_object(&canonical);
            if self.store.read(&id)? != encoded {
                self.store.replace(id, &encoded)?;
            }
        }
        self.level = MEASURE_LEVEL;
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
        fresh.proposed_order = self.proposed_order.clone();
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
        // A fact's inputs go in before its output, so the fact can be
        // verified as a candidate when the output is planned (E056).
        for (id, bytes) in inputs_first(files, &hints) {
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
        fresh.group_small()?;
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

    /// Re-encode every plain frame and compressible object at
    /// `STORE_LEVEL`, keeping a new encoding only if it costs less: stored
    /// bytes + `read_weight` x decode time, where a group is decoded up to
    /// each member's end per member read and any other content whole per
    /// read; a content is read once, plus once per derivation
    /// naming it. Plans are compared at the fast `MEASURE_LEVEL`; only the one
    /// kept pays for the slow level. Decoded content is checked against its
    /// id first.
    pub(crate) fn recompress(&mut self, roots: &[Id]) -> Result<(), Error> {
        self.level = STORE_LEVEL;
        let dict = self.dictionary().map(|(id, _)| *id);
        let model = self.read_model(roots)?;
        let bases = self.related_bases(dict)?;
        let (mut used, mut related) = (HashSet::new(), HashSet::new());
        for id in self.store.ids()? {
            let stored = self.store.read(&id)?;
            let encoded = match stored.first() {
                Some(b'B' | b'Z' | b'Y') => {
                    let bytes = self.decode_plain(&id, &stored)?;
                    if Id::of_content(&bytes) != id {
                        return Err(Error::Corrupt(id));
                    }
                    let reads = model.reads(&id, bytes.len());
                    let mut encoded = match Some(id) == dict {
                        // A dictionary is never encoded against a dictionary.
                        true => zstd_frame(&bytes, &[], STORE_LEVEL)
                            .map_or(stored.clone(), |z| [&b"Z"[..], &z].concat()),
                        false => self.encode_cheapest(&bytes, reads),
                    };
                    let mut cost = self.cost(&encoded, reads);
                    // A group may instead be stored as blocks, each its own
                    // frame with the dictionary: a member read decodes only
                    // the blocks it overlaps.
                    if model.is_group(&id)
                        && let Some((dict_id, dict_bytes)) = self.dictionary()
                    {
                        for block in BLOCKS.into_iter().filter(|&b| b < bytes.len()) {
                            let Some(body) = block_frames(&bytes, dict_bytes, block) else {
                                continue;
                            };
                            let k =
                                [&b"K"[..], &dict_id.as_bytes()[..DICT_REF_LEN], &body].concat();
                            let k_cost = k.len() as f64
                                + self.read_weight
                                    * model.block_reads(&id, bytes.len(), block) as f64;
                            if k_cost < cost {
                                (encoded, cost) = (k, k_cost);
                            }
                        }
                    }
                    // Or with related stored content as the frame's context
                    // (E057): read costs that content's decoding too.
                    if Some(id) != dict
                        && !used.contains(&id)
                        && let Some(base) = bases.most_related(&id, &bytes, &related)
                        && let Some(z) = zstd_frame(&bytes, &base.1, STORE_LEVEL)
                    {
                        let q = [&b"Q"[..], base.0.as_bytes(), &z].concat();
                        let q_cost = q.len() as f64
                            + self.read_weight * (reads + model.events(&id) * base.1.len()) as f64;
                        if q_cost < cost {
                            (encoded, cost) = (q, q_cost);
                            used.insert(base.0);
                            related.insert(id);
                        }
                    }
                    if cost < self.cost(&stored, reads) {
                        self.store.replace(id, &encoded)?;
                    } else {
                        related.remove(&id);
                    }
                    continue;
                }
                Some(_) if object_tag(&stored).is_some() && tree_content(&stored).is_none() => {
                    self.encode_object(&self.read_object(&id)?)
                }
                _ => continue,
            };
            if encoded.len() < stored.len() {
                self.store.replace(id, &encoded)?;
            }
        }
        self.level = MEASURE_LEVEL;
        self.reset_projections();
        Ok(())
    }

    /// How often each stored content is decoded when every root is read
    /// once (the score's read model, see `ReadModel`): a root once for
    /// itself, and any content once per read of each content whose read
    /// evaluates a derivation naming it (E059). A read evaluates no
    /// derivation of content stored plainly, else its first.
    pub(crate) fn read_model(&self, roots: &[Id]) -> Result<ReadModel, Error> {
        let mut readers: HashMap<Id, Vec<Id>> = HashMap::new();
        for (output, ds) in &self.ledger()?.by_output {
            if self.store.read(output).is_ok_and(|e| is_plain(&e)) {
                continue;
            }
            // A read caches what it decodes, so a source named by several
            // arguments of one derivation is decoded once for it (E048).
            let sources: BTreeSet<Id> = ds
                .iter()
                .take(1)
                .flat_map(|d| d.args.iter().map(Arg::id))
                .collect();
            sources
                .into_iter()
                .for_each(|id| readers.entry(id).or_default().push(*output));
        }
        let roots: HashSet<Id> = roots.iter().copied().collect();
        let mut times = HashMap::new();
        for id in readers.keys().chain(&roots) {
            read_times(*id, &roots, &readers, &mut times, 0);
        }
        let mut model = ReadModel {
            times,
            groups: HashMap::new(),
        };
        for id in self.store.ids()? {
            if let Some((group, start, end)) = member_span(&self.store.read(&id)?) {
                let span = (start, end, model.times(&id));
                model.groups.entry(group.to_vec()).or_default().push(span);
            }
        }
        Ok(model)
    }

    /// Stored bytes + `read_weight` x the measured cost of reading every
    /// root once: bytes decoded (weighted by codec) plus function work. The
    /// benchmark's own objective, measured rather than modelled (E059), so
    /// plans are chosen by it.
    pub(crate) fn plan_cost(&self, roots: &[Id]) -> Result<f64, Error> {
        let before = crate::dict::decode_cost();
        let mut work = 0;
        for id in roots {
            let mut cx = Cx::default();
            self.rebuild(id, &mut cx)?;
            work += cx.work;
        }
        let read = crate::dict::decode_cost() - before + work;
        Ok(self.content_bytes()? as f64 + self.read_weight * read as f64)
    }

    /// Contents stored plainly (not the dictionary) of at least
    /// `RELATED_MIN` bytes: the candidate contexts of "Q" encodings.
    fn related_bases(&self, dict: Option<Id>) -> Result<Bases, Error> {
        let mut bases = Bases::default();
        for id in self.store.ids()? {
            let stored = self.store.read(&id)?;
            if !matches!(stored[0], b'B' | b'Z' | b'Y') || Some(id) == dict {
                continue;
            }
            let bytes = self.decode_plain(&id, &stored)?;
            if bytes.len() >= RELATED_MIN && Id::of_content(&bytes) == id {
                for h in shingles(&bytes) {
                    bases.index.entry(h).or_default().push(bases.items.len());
                }
                bases.items.push((id, bytes));
            }
        }
        Ok(bases)
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

/// `files` (largest first) reordered so that every input of a hint comes
/// before the content it rebuilds: depth-first over hint arguments.
fn inputs_first<'a>(
    files: &'a [(Id, Vec<u8>)],
    hints: &HashMap<Id, Vec<Derivation>>,
) -> Vec<&'a (Id, Vec<u8>)> {
    let at: HashMap<Id, usize> = files
        .iter()
        .enumerate()
        .map(|(i, (id, _))| (*id, i))
        .collect();
    let (mut out, mut seen) = (Vec::with_capacity(files.len()), HashSet::new());
    fn visit<'a>(
        i: usize,
        files: &'a [(Id, Vec<u8>)],
        hints: &HashMap<Id, Vec<Derivation>>,
        at: &HashMap<Id, usize>,
        seen: &mut HashSet<usize>,
        out: &mut Vec<&'a (Id, Vec<u8>)>,
    ) {
        if !seen.insert(i) {
            return;
        }
        for d in hints.get(&files[i].0).into_iter().flatten() {
            for a in &d.args {
                if let Some(&j) = at.get(&a.id()) {
                    visit(j, files, hints, at, seen, out);
                }
            }
        }
        out.push(&files[i]);
    }
    for i in 0..files.len() {
        visit(i, files, hints, &at, &mut seen, &mut out);
    }
    out
}

/// Reads of `id` when every root is read once (see `Repo::read_model`),
/// memoised; a cycle or a chain past `STACK_GUARD` counts no further.
fn read_times(
    id: Id,
    roots: &HashSet<Id>,
    readers: &HashMap<Id, Vec<Id>>,
    memo: &mut HashMap<Id, usize>,
    depth: usize,
) -> usize {
    if let Some(&n) = memo.get(&id) {
        return n;
    }
    memo.insert(id, 0); // a cycle (malformed store) adds nothing
    let mut n = roots.contains(&id) as usize;
    if depth < crate::repo::STACK_GUARD {
        for r in readers.get(&id).into_iter().flatten() {
            n += read_times(*r, roots, readers, memo, depth + 1);
        }
    }
    memo.insert(id, n);
    n
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

/// Candidate zstd dictionaries for `samples`, each estimated to save more
/// than its own size: the `k` best estimates, plus the best of each
/// construction not among them. Candidates are trained (COVER, zstd's
/// trainer), raw evenly spaced pieces of the samples (relative Lempel-Ziv)
/// and raw pieces of the most repeated content, at a few sizes: no
/// construction or size wins on every corpus (COVER can return a tiny
/// dictionary when asked for a large one). The estimate ignores dedup, so
/// the re-plans decide among the shortlist, and every construction gets one.
fn train_dictionaries(samples: &[&[u8]], k: usize) -> Vec<Vec<u8>> {
    let samples: Vec<&[u8]> = samples.iter().copied().filter(|b| !b.is_empty()).collect();
    let total: usize = samples.iter().map(|b| b.len()).sum();
    let Some(alone) = dictionary_cost(&samples, &[]).filter(|_| samples.len() >= 8) else {
        return vec![];
    };
    // (estimate, construction, dictionary)
    let mut ranked = Vec::new();
    for size in [total / 100, total / 30, total / 10].map(|s| s.clamp(4096, 112_640)) {
        let built = [
            zstd::dict::from_samples(&samples, size).ok(),
            Some(raw_dictionary(&samples, size)),
            Some(repeat_dictionary(&samples, size)),
        ];
        for (construction, dict) in built.into_iter().enumerate() {
            if let Some(dict) = dict
                && let Some(cost) = dictionary_cost(&samples, &dict).filter(|&c| c < alone)
                && !ranked.iter().any(|(_, _, d)| *d == dict)
            {
                ranked.push((cost, construction, dict));
            }
        }
    }
    ranked.sort();
    let mut shortlist: Vec<usize> = (0..k.min(ranked.len())).collect();
    for construction in 0..3 {
        if let Some(i) = ranked.iter().position(|r| r.1 == construction)
            && !shortlist.contains(&i)
        {
            shortlist.push(i);
        }
    }
    shortlist.into_iter().map(|i| ranked[i].2.clone()).collect()
}

/// Estimated stored size of `samples` given `dict` (empty: none): the
/// dictionary plus each sample's smallest encoding, a dictionary encoding
/// paying for the id it names.
fn dictionary_cost(samples: &[&[u8]], dict: &[u8]) -> Option<usize> {
    let (mut alone, mut given) = (framer(&[])?, framer(dict)?);
    let mut cost = dict.len();
    for b in samples {
        let z = alone.compress(b).map_or(b.len(), |z| z.len().min(b.len()));
        cost += match dict.is_empty() {
            true => z,
            false => given
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

/// Seed positions and keys of `s` (the matcher's content-defined anchors).
fn seeds(s: &[u8]) -> impl Iterator<Item = (usize, u64)> + '_ {
    matcher::anchors(s).map(move |p| (p, matcher::seed_key(&s[p..p + matcher::SEED])))
}

/// A raw-content dictionary of the content that repeats most across
/// samples: 1 KiB pieces around the seeds found in the most samples, no
/// seed covered twice, the most shared last (zstd reaches the end of a
/// dictionary with the shortest offsets). Built from the corpus's repeats,
/// not from one sampling of it (Kuruppu, Puglisi and Zobel, SPIRE 2011),
/// and independent of sample order.
fn repeat_dictionary(samples: &[&[u8]], size: usize) -> Vec<u8> {
    const PIECE: usize = 1024;
    let mut found_in: HashMap<u64, usize> = HashMap::new();
    for s in samples {
        let keys: HashSet<u64> = seeds(s).map(|(_, k)| k).collect();
        keys.into_iter()
            .for_each(|k| *found_in.entry(k).or_default() += 1);
    }
    let mut pieces: Vec<(usize, &[u8], u64)> = Vec::new();
    for s in samples {
        for (p, k) in seeds(s) {
            if found_in[&k] > 1 {
                let start = p
                    .saturating_sub(PIECE / 2)
                    .min(s.len().saturating_sub(PIECE));
                pieces.push((found_in[&k], &s[start..(start + PIECE).min(s.len())], k));
            }
        }
    }
    pieces.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(b.1)));
    let (mut covered, mut chosen, mut len) = (HashSet::new(), Vec::new(), 0);
    for (_, piece, k) in pieces {
        if len + piece.len() > size || covered.contains(&k) {
            continue;
        }
        covered.extend(seeds(piece).map(|(_, k)| k));
        covered.insert(k);
        len += piece.len();
        chosen.push(piece);
    }
    chosen
        .iter()
        .rev()
        .flat_map(|p| p.iter().copied())
        .collect()
}

/// How often stored content is decoded when every file is read once: a
/// content once for itself plus once per derivation naming it (a
/// template per filled document, a slice source per sliced document); a
/// group up to each member's end per read of that member.
pub(crate) struct ReadModel {
    times: HashMap<Id, usize>,
    /// Per group prefix: each member's range and how often it is read.
    groups: HashMap<Vec<u8>, Vec<(usize, usize, usize)>>,
}

/// Smallest content worth a related-content context (E057).
const RELATED_MIN: usize = 2048;

/// Candidate contexts for "Q" encodings, indexed by shingle.
#[derive(Default)]
struct Bases {
    items: Vec<(Id, Vec<u8>)>,
    index: HashMap<u64, Vec<usize>>,
}

impl Bases {
    /// The candidate sharing the most shingles with `bytes` (not `id`
    /// itself, nor any content already encoded against another).
    fn most_related(&self, id: &Id, bytes: &[u8], related: &HashSet<Id>) -> Option<&(Id, Vec<u8>)> {
        if bytes.len() < RELATED_MIN {
            return None;
        }
        let mut shared: HashMap<usize, usize> = HashMap::new();
        for h in shingles(bytes) {
            for &i in self.index.get(&h).into_iter().flatten() {
                *shared.entry(i).or_default() += 1;
            }
        }
        let (i, _) = shared
            .into_iter()
            .filter(|&(i, _)| self.items[i].0 != *id && !related.contains(&self.items[i].0))
            .max_by_key(|&(i, n)| (n, std::cmp::Reverse(i)))?;
        Some(&self.items[i])
    }
}

/// Content-defined 16-byte shingles (about 1 in 16 positions), FNV-1a.
fn shingles(bytes: &[u8]) -> HashSet<u64> {
    bytes
        .windows(16)
        .map(|w| {
            w.iter().fold(0xcbf2_9ce4_8422_2325_u64, |h, &b| {
                (h ^ b as u64).wrapping_mul(0x0100_0000_01b3)
            })
        })
        .filter(|h| h % 16 == 0)
        .collect()
}

/// Block sizes a group may be stored in (see `dict::block_frames`).
const BLOCKS: [usize; 2] = [4 * 1024, 16 * 1024];

impl ReadModel {
    pub(crate) fn times(&self, id: &Id) -> usize {
        self.times.get(id).copied().unwrap_or(0)
    }

    /// Bytes decoded from content `id` (of `len` bytes) to read every file
    /// once.
    pub(crate) fn reads(&self, id: &Id, len: usize) -> usize {
        match self.groups.get(&id.as_bytes()[..GROUP_REF_LEN]) {
            Some(spans) => spans.iter().map(|&(_, end, n)| end * n).sum(),
            None => len * self.times(id),
        }
    }

    /// How many times content `id` is decoded (from its start) when every
    /// file is read once: once per member read for a group.
    pub(crate) fn events(&self, id: &Id) -> usize {
        match self.groups.get(&id.as_bytes()[..GROUP_REF_LEN]) {
            Some(spans) => spans.iter().map(|&(_, _, n)| n).sum(),
            None => self.times(id),
        }
    }

    fn is_group(&self, id: &Id) -> bool {
        self.groups.contains_key(&id.as_bytes()[..GROUP_REF_LEN])
    }

    /// Bytes decoded from group `id` (of `len` bytes) stored in blocks of
    /// `block` bytes: each member read decodes the blocks it overlaps.
    pub(crate) fn block_reads(&self, id: &Id, len: usize, block: usize) -> usize {
        let spans = self.groups.get(&id.as_bytes()[..GROUP_REF_LEN]);
        let covered = |s: usize, e: usize| (e.div_ceil(block) * block).min(len) - s / block * block;
        spans.map_or(len * self.times(id), |spans| {
            spans.iter().map(|&(s, e, n)| covered(s, e) * n).sum()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fact's inputs are planned before the content it rebuilds, whatever
    /// their sizes, so the fact can be verified as a candidate then (E056).
    #[test]
    fn replan_puts_fact_inputs_first() {
        let (big, small, other) = (
            Id::of_content(b"big"),
            Id::of_content(b"s"),
            Id::of_content(b"o"),
        );
        let files = vec![
            (big, vec![0; 30]),
            (other, vec![0; 20]),
            (small, vec![0; 10]),
        ];
        let fact = Derivation {
            output: big,
            func: func::concat(),
            args: vec![Arg::Whole(small)],
        };
        let hints = HashMap::from([(big, vec![fact]), (other, vec![]), (small, vec![])]);
        let order: Vec<Id> = inputs_first(&files, &hints)
            .iter()
            .map(|(id, _)| *id)
            .collect();
        assert_eq!(order, vec![small, big, other]);
    }

    /// A source named by several arguments of one derivation is decoded
    /// once when that derivation is read (reads cache what they decode), so
    /// the read model counts it once (E048).
    #[test]
    fn read_model_counts_a_source_once_per_derivation() {
        let mut repo = Repo::new(MemStore::default());
        let source = repo.put_content(b"0123456789abcdefghij").unwrap().id;
        let range = |start, len| Arg::Range {
            id: source,
            start,
            len,
        };
        let out = b"0123cdefgh";
        let args = vec![range(0, 4), range(12, 6)];
        repo.put_derivation(out, func::concat(), args)
            .unwrap()
            .unwrap();
        let out = Id::of_content(out);
        assert_eq!(repo.read_model(&[source, out]).unwrap().times(&source), 2);
    }

    /// Recompression at the storing level keeps every content readable,
    /// never grows an encoding, and keeps the dictionary itself out of
    /// dictionary encodings (decoding never chains).
    #[test]
    fn recompress_shrinks_and_keeps_the_dictionary_plain() {
        let mut repo = Repo::new(MemStore::default());
        let docs: Vec<Vec<u8>> = (0..60)
            .map(|i| {
                format!(
                    "entry {i}: {}\n",
                    "shared words for the dictionary ".repeat(4 + i % 9)
                )
                .into_bytes()
            })
            .collect();
        let sizes: Vec<usize> = docs.iter().map(Vec::len).collect();
        let dict = zstd::dict::from_continuous(&docs.concat(), &sizes, 2048).unwrap();
        let dict_id = repo.set_dictionary(dict).unwrap();
        let ids: Vec<Id> = docs
            .iter()
            .map(|d| repo.put_content(d).unwrap().id)
            .collect();
        let size = |repo: &Repo<MemStore>, id: &Id| repo.store.read(id).unwrap().len();
        let before: Vec<usize> = ids.iter().map(|id| size(&repo, id)).collect();
        repo.recompress(&ids).unwrap();
        assert!(matches!(repo.store.read(&dict_id).unwrap()[0], b'B' | b'Z'));
        for ((id, d), b) in ids.iter().zip(&docs).zip(before) {
            assert_eq!(&repo.read_content(id).unwrap(), d);
            assert!(size(&repo, id) <= b);
        }
        assert_eq!(repo.level, MEASURE_LEVEL);
    }

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

    /// The repeat dictionary depends on the samples, not their order, and
    /// holds only content found in more than one sample.
    #[test]
    fn repeat_dictionary_ignores_sample_order() {
        let shared = (0..400)
            .map(|i| format!("shared clause {i}; "))
            .collect::<String>();
        let docs: Vec<Vec<u8>> = (0..12)
            .map(|n| format!("{shared} unique text {}", "x".repeat(n * 97)).into_bytes())
            .collect();
        let forward: Vec<&[u8]> = docs.iter().map(Vec::as_slice).collect();
        let backward: Vec<&[u8]> = forward.iter().rev().copied().collect();
        let dict = repeat_dictionary(&forward, 8192);
        assert!(!dict.is_empty() && dict.len() <= 8192);
        assert_eq!(dict, repeat_dictionary(&backward, 8192));
        assert!(repeat_dictionary(&forward[..1], 8192).is_empty());
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
