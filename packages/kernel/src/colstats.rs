//! Column statistics gathered while a table's rows are read anyway (E403),
//! after Kumo's ingest column stats: for each top-level field of JSON rows
//! (an object's keys, or an array's positions), its count and nulls; for
//! numbers, the mean and variance (Welford), min and max, and quantiles (a
//! merging t-digest, Dunning and Ertl); distinct values (HyperLogLog,
//! Flajolet et al., 2^12 registers, about 1.6% standard error); for strings
//! and booleans, value counts while a column has few values.
//!
//! `TableStats::observe` takes one row's bytes; `bm25_view_build_with_stats`
//! calls it from the BM25 view build's own pass over the rows, so the stats
//! cost no second read. `to_json` is the derived view that is kept.
//! A row that is not a JSON object or array counts only as a row.
//!
//! Zone maps (E405, off unless asked for with `TableStats::with_zones`):
//! per zone of so many rows in the order observed (a table's key order in
//! the BM25 build), per field, the rows that have it, its nulls, and the
//! min and max of its numbers and of its strings (as stored, escapes
//! included, compared bytewise), with the zone's first and last key.
//! `zones_to_read` names the zones a filter can match, so a filtered read
//! can skip the others.

use std::collections::{BTreeMap, HashMap};
use std::hash::{BuildHasher, BuildHasherDefault, DefaultHasher};

/// HyperLogLog precision: 2^P registers.
const P: u32 = 12;
/// Most distinct values a column's value counts follow.
pub const CATEGORIES_MAX: usize = 256;
/// t-digest compression (most centroids about this many).
const DELTA: f64 = 100.0;

/// A field's value, as `observe` sees it.
#[derive(Clone, Copy)]
enum Val<'a> {
    Num(f64, &'a [u8]),
    Str(&'a [u8]),
    Bool(bool),
    Null,
    Other(&'a [u8]),
}

/// Distinct-value estimate: HyperLogLog with 2^P registers.
#[derive(Clone)]
pub struct Hll {
    reg: Vec<u8>,
}

impl Default for Hll {
    fn default() -> Hll {
        Hll {
            reg: vec![0; 1 << P],
        }
    }
}

fn hash(b: &[u8]) -> u64 {
    BuildHasherDefault::<DefaultHasher>::default().hash_one(b)
}

impl Hll {
    pub fn add(&mut self, b: &[u8]) {
        let h = hash(b);
        let i = (h >> (64 - P)) as usize;
        let rest = h << P | (1 << (P - 1));
        let rank = rest.leading_zeros() as u8 + 1;
        if rank > self.reg[i] {
            self.reg[i] = rank;
        }
    }

    pub fn estimate(&self) -> f64 {
        let m = self.reg.len() as f64;
        let sum: f64 = self.reg.iter().map(|r| 2f64.powi(-(*r as i32))).sum();
        let raw = 0.7213 / (1.0 + 1.079 / m) * m * m / sum;
        let zeros = self.reg.iter().filter(|r| **r == 0).count();
        if raw <= 2.5 * m && zeros > 0 {
            m * (m / zeros as f64).ln() // linear counting for small sets
        } else {
            raw
        }
    }
}

/// Quantiles: a merging t-digest with the k1 scale function.
#[derive(Clone, Default)]
pub struct TDigest {
    /// (mean, weight), sorted by mean.
    c: Vec<(f64, f64)>,
    buf: Vec<f64>,
    n: f64,
}

fn k1(q: f64) -> f64 {
    DELTA / (2.0 * std::f64::consts::PI) * (2.0 * q - 1.0).asin()
}

impl TDigest {
    pub fn add(&mut self, x: f64) {
        self.buf.push(x);
        if self.buf.len() >= 20 * DELTA as usize {
            self.merge();
        }
    }

    fn merge(&mut self) {
        if self.buf.is_empty() {
            return;
        }
        let mut all: Vec<(f64, f64)> = self.buf.drain(..).map(|x| (x, 1.0)).collect();
        all.append(&mut self.c);
        all.sort_by(|a, b| a.0.total_cmp(&b.0));
        let total: f64 = all.iter().map(|c| c.1).sum();
        let mut out: Vec<(f64, f64)> = Vec::with_capacity(2 * DELTA as usize);
        let (mut seen, mut cur) = (0.0, all[0]);
        let mut limit = k1(0.0) + 1.0;
        for &(m, w) in &all[1..] {
            let q = (seen + cur.1 + w) / total;
            if k1(q.min(1.0)) <= limit {
                cur.0 += (m - cur.0) * w / (cur.1 + w);
                cur.1 += w;
            } else {
                seen += cur.1;
                out.push(cur);
                limit = k1((seen / total).min(1.0)) + 1.0;
                cur = (m, w);
            }
        }
        out.push(cur);
        self.c = out;
        self.n = total;
    }

    /// The value at quantile `q` (0 to 1), interpolated between centroids.
    pub fn quantile(&mut self, q: f64) -> Option<f64> {
        self.merge();
        let c = &self.c;
        if c.is_empty() {
            return None;
        }
        let target = q * self.n;
        let mut seen = 0.0;
        for i in 0..c.len() {
            let mid = seen + c[i].1 / 2.0;
            if target < mid {
                if i == 0 {
                    return Some(c[0].0);
                }
                let prev = seen - c[i - 1].1 / 2.0;
                let t = (target - prev) / (mid - prev);
                return Some(c[i - 1].0 + t * (c[i].0 - c[i - 1].0));
            }
            seen += c[i].1;
        }
        Some(c[c.len() - 1].0)
    }

    pub fn centroids(&mut self) -> usize {
        self.merge();
        self.c.len()
    }
}

/// What one field holds over the rows that have it.
#[derive(Clone, Default)]
pub struct ColumnStats {
    pub count: u64,
    pub nulls: u64,
    pub numbers: u64,
    pub strings: u64,
    pub bools: u64,
    pub other: u64,
    pub mean: f64,
    m2: f64,
    pub min: f64,
    pub max: f64,
    pub distinct: Hll,
    pub quantiles: TDigest,
    /// Value counts of strings and booleans, while there are at most
    /// `CATEGORIES_MAX` distinct ones; `None` after that.
    pub values: Option<HashMap<Vec<u8>, u64>>,
    /// The field's place in each zone's columns (the order fields were
    /// first seen).
    idx: usize,
}

impl ColumnStats {
    fn add(&mut self, v: Val) {
        self.count += 1;
        let (seen, category): (&[u8], bool) = match v {
            Val::Null => {
                self.nulls += 1;
                return;
            }
            Val::Num(x, text) => {
                self.numbers += 1;
                let n = self.numbers as f64;
                if self.numbers == 1 {
                    (self.min, self.max) = (x, x);
                }
                self.min = self.min.min(x);
                self.max = self.max.max(x);
                let d = x - self.mean;
                self.mean += d / n;
                self.m2 += d * (x - self.mean);
                self.quantiles.add(x);
                (text, false)
            }
            Val::Str(s) => {
                self.strings += 1;
                (s, true)
            }
            Val::Bool(b) => {
                self.bools += 1;
                (if b { b"true" } else { b"false" }, true)
            }
            Val::Other(o) => {
                self.other += 1;
                (o, false)
            }
        };
        self.distinct.add(seen);
        if category && let Some(m) = &mut self.values {
            if let Some(n) = m.get_mut(seen) {
                *n += 1;
            } else if m.len() < CATEGORIES_MAX {
                m.insert(seen.to_vec(), 1);
            } else {
                self.values = None;
            }
        }
    }

    /// The field's place in each zone's `columns`.
    pub fn idx(&self) -> usize {
        self.idx
    }

    /// The population variance of the numbers.
    pub fn variance(&self) -> f64 {
        if self.numbers == 0 {
            0.0
        } else {
            self.m2 / self.numbers as f64
        }
    }
}

/// Statistics of a table's rows, by field name (an array's positions are
/// named "0", "1", ...).
#[derive(Clone, Default)]
pub struct TableStats {
    pub rows: u64,
    pub columns: BTreeMap<String, ColumnStats>,
    /// E405: zone maps, when asked for.
    pub zones: Option<Zones>,
}

/// One field within one zone.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ZoneColumn {
    pub count: u64,
    pub nulls: u64,
    /// Min and max of the field's numbers in the zone.
    pub num: Option<(f64, f64)>,
    /// Min and max of the field's strings in the zone, bytewise.
    pub str: Option<(Vec<u8>, Vec<u8>)>,
}

impl ZoneColumn {
    fn add(&mut self, v: Val) {
        self.count += 1;
        match v {
            Val::Null => self.nulls += 1,
            Val::Num(x, _) => {
                let (lo, hi) = self.num.get_or_insert((x, x));
                *lo = lo.min(x);
                *hi = hi.max(x);
            }
            Val::Str(s) => match &mut self.str {
                None => self.str = Some((s.to_vec(), s.to_vec())),
                Some((lo, hi)) => {
                    if s < &lo[..] {
                        *lo = s.to_vec();
                    } else if s > &hi[..] {
                        *hi = s.to_vec();
                    }
                }
            },
            Val::Bool(_) | Val::Other(_) => {}
        }
    }
}

/// One zone: its first and last key (when observed with keys), and its
/// columns by field place.
#[derive(Clone, Debug, Default)]
pub struct Zone {
    pub rows: u64,
    pub first_key: Option<String>,
    pub last_key: Option<String>,
    pub columns: Vec<ZoneColumn>,
}

/// Zone maps over a table's rows.
#[derive(Clone, Debug)]
pub struct Zones {
    pub zone_rows: u64,
    pub zones: Vec<Zone>,
}

/// A filter on one field, for `zones_to_read`.
pub enum Filter<'a> {
    /// A number from `lo` to `hi`, both included.
    Range(f64, f64),
    /// A string equal to this (its stored bytes, without the quotes).
    Eq(&'a [u8]),
}

fn skip_ws(b: &[u8], mut i: usize) -> usize {
    while i < b.len() && matches!(b[i], b' ' | b'\t' | b'\n' | b'\r') {
        i += 1;
    }
    i
}

/// The end (one past) of the JSON value starting at `i`, if well formed
/// enough to step over (strings, and brackets balanced outside strings).
fn value_end(b: &[u8], i: usize) -> Option<usize> {
    match *b.get(i)? {
        b'"' => {
            let mut j = i + 1;
            while j < b.len() {
                match b[j] {
                    b'\\' => j += 2,
                    b'"' => return Some(j + 1),
                    _ => j += 1,
                }
            }
            None
        }
        b'{' | b'[' => {
            let (mut depth, mut j) = (0usize, i);
            while j < b.len() {
                match b[j] {
                    b'"' => {
                        j = value_end(b, j)?;
                        continue;
                    }
                    b'{' | b'[' => depth += 1,
                    b'}' | b']' => {
                        depth -= 1;
                        if depth == 0 {
                            return Some(j + 1);
                        }
                    }
                    _ => {}
                }
                j += 1;
            }
            None
        }
        _ => {
            let mut j = i;
            while j < b.len() && !matches!(b[j], b',' | b'}' | b']' | b' ' | b'\t' | b'\n' | b'\r')
            {
                j += 1;
            }
            (j > i).then_some(j)
        }
    }
}

fn classify(t: &[u8]) -> Val<'_> {
    match t {
        b"null" => Val::Null,
        b"true" => Val::Bool(true),
        b"false" => Val::Bool(false),
        [b'"', s @ .., b'"'] => Val::Str(s),
        _ => match std::str::from_utf8(t)
            .ok()
            .and_then(|s| s.parse::<f64>().ok())
        {
            Some(x) if x.is_finite() && !t.starts_with(b"+") && t != b"inf" => Val::Num(x, t),
            _ => Val::Other(t),
        },
    }
}

impl TableStats {
    /// Stats that also keep zone maps, one zone for each `zone_rows` rows
    /// (at least 1).
    pub fn with_zones(zone_rows: u64) -> TableStats {
        TableStats {
            zones: Some(Zones {
                zone_rows: zone_rows.max(1),
                zones: Vec::new(),
            }),
            ..TableStats::default()
        }
    }

    /// `observe`, noting the row's key as its zone's first or last.
    pub fn observe_at(&mut self, key: &str, row: &[u8]) {
        self.observe(row);
        if let Some(z) = self.zones.as_mut().and_then(|z| z.zones.last_mut()) {
            if z.first_key.is_none() {
                z.first_key = Some(key.to_string());
            }
            let last = z.last_key.get_or_insert_default();
            last.clear();
            last.push_str(key);
        }
    }

    /// The zones (their places) a row matching `filter` on `column` can be
    /// in; every zone when there are no zone maps or no such column.
    pub fn zones_to_read(&self, column: &str, filter: &Filter) -> Vec<usize> {
        let Some(z) = &self.zones else {
            return Vec::new();
        };
        let Some(c) = self.columns.get(column) else {
            return (0..z.zones.len()).collect();
        };
        (0..z.zones.len())
            .filter(|&i| {
                let Some(zc) = z.zones[i].columns.get(c.idx) else {
                    return false; // the field first appears after this zone
                };
                match *filter {
                    Filter::Range(lo, hi) => zc.num.is_some_and(|(a, b)| a <= hi && b >= lo),
                    Filter::Eq(s) => zc
                        .str
                        .as_ref()
                        .is_some_and(|(a, b)| &a[..] <= s && s <= &b[..]),
                }
            })
            .collect()
    }

    /// Add one row (its value bytes).
    pub fn observe(&mut self, row: &[u8]) {
        self.rows += 1;
        if let Some(z) = &mut self.zones
            && (self.rows - 1).is_multiple_of(z.zone_rows)
        {
            z.zones.push(Zone::default());
        }
        if let Some(z) = self.zones.as_mut().and_then(|z| z.zones.last_mut()) {
            z.rows += 1;
        }
        let mut i = skip_ws(row, 0);
        let (object, close) = match row.get(i) {
            Some(b'{') => (true, b'}'),
            Some(b'[') => (false, b']'),
            _ => return,
        };
        i += 1;
        let mut pos = 0usize;
        loop {
            i = skip_ws(row, i);
            if row.get(i) == Some(&close) {
                return;
            }
            let name = if object {
                let Some(end) = value_end(row, i).filter(|_| row[i] == b'"') else {
                    return;
                };
                let name = String::from_utf8_lossy(&row[i + 1..end - 1]).into_owned();
                i = skip_ws(row, end);
                if row.get(i) != Some(&b':') {
                    return;
                }
                i = skip_ws(row, i + 1);
                name
            } else {
                pos += 1;
                (pos - 1).to_string()
            };
            let Some(end) = value_end(row, i) else {
                return;
            };
            let next = self.columns.len();
            let col = match self.columns.get_mut(&name) {
                Some(c) => c,
                None => self.columns.entry(name).or_insert_with(|| ColumnStats {
                    values: Some(HashMap::new()),
                    idx: next,
                    ..ColumnStats::default()
                }),
            };
            let v = classify(&row[i..end]);
            col.add(v);
            if let Some(z) = self.zones.as_mut().and_then(|z| z.zones.last_mut()) {
                if z.columns.len() <= col.idx {
                    z.columns.resize(col.idx + 1, ZoneColumn::default());
                }
                z.columns[col.idx].add(v);
            }
            i = skip_ws(row, end);
            match row.get(i) {
                Some(b',') => i += 1,
                Some(c) if *c == close => return,
                _ => return,
            }
        }
    }

    /// The derived view kept for the table: one JSON object, fields in name
    /// order, numbers as Rust formats them, value counts most common first
    /// (up to 20).
    pub fn to_json(&mut self) -> String {
        let mut out = format!("{{\"rows\":{},\"columns\":{{", self.rows);
        let esc = |s: &str| {
            let mut o = String::from("\"");
            for ch in s.chars() {
                match ch {
                    '"' => o.push_str("\\\""),
                    '\\' => o.push_str("\\\\"),
                    c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
                    c => o.push(c),
                }
            }
            o + "\""
        };
        for (i, (name, c)) in self.columns.iter_mut().enumerate() {
            if i > 0 {
                out.push(',');
            }
            out += &format!(
                "{}:{{\"count\":{},\"nulls\":{},\"numbers\":{},\"strings\":{},\"bools\":{},\"other\":{},\"distinct\":{:.0}",
                esc(name),
                c.count,
                c.nulls,
                c.numbers,
                c.strings,
                c.bools,
                c.other,
                c.distinct.estimate()
            );
            if c.numbers > 0 {
                out += &format!(
                    ",\"min\":{},\"max\":{},\"mean\":{},\"variance\":{}",
                    c.min,
                    c.max,
                    c.mean,
                    c.variance()
                );
                let qs: Vec<String> = [0.01, 0.1, 0.25, 0.5, 0.75, 0.9, 0.99]
                    .iter()
                    .map(|q| format!("\"{q}\":{}", c.quantiles.quantile(*q).unwrap()))
                    .collect();
                out += &format!(",\"quantiles\":{{{}}}", qs.join(","));
            }
            if let Some(m) = &c.values {
                let mut v: Vec<(&Vec<u8>, &u64)> = m.iter().collect();
                v.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
                let top: Vec<String> = v
                    .iter()
                    .take(20)
                    .map(|(k, n)| format!("[{},{n}]", esc(&String::from_utf8_lossy(k))))
                    .collect();
                out += &format!(",\"values\":[{}]", top.join(","));
            }
            out.push('}');
        }
        out.push('}');
        if let Some(z) = &self.zones {
            // Fields by place, then per zone each field's count, nulls and
            // ranges (numbers as Rust formats them).
            let mut names = vec![""; self.columns.len()];
            for (name, c) in &self.columns {
                names[c.idx] = name;
            }
            let names: Vec<String> = names.iter().map(|n| esc(n)).collect();
            out += &format!(
                ",\"zones\":{{\"zone_rows\":{},\"fields\":[{}],\"zones\":[",
                z.zone_rows,
                names.join(",")
            );
            for (i, zone) in z.zones.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                let key = |k: &Option<String>| k.as_deref().map_or("null".into(), esc);
                out += &format!(
                    "{{\"rows\":{},\"first\":{},\"last\":{},\"columns\":[",
                    zone.rows,
                    key(&zone.first_key),
                    key(&zone.last_key)
                );
                for (j, c) in zone.columns.iter().enumerate() {
                    if j > 0 {
                        out.push(',');
                    }
                    out += &format!("[{},{}", c.count, c.nulls);
                    match c.num {
                        Some((a, b)) => out += &format!(",{a},{b}"),
                        None => out += ",null,null",
                    }
                    match &c.str {
                        Some((a, b)) => {
                            out += &format!(
                                ",{},{}",
                                esc(&String::from_utf8_lossy(a)),
                                esc(&String::from_utf8_lossy(b))
                            )
                        }
                        None => out += ",null,null",
                    }
                    out.push(']');
                }
                out += "]}";
            }
            out += "]}";
        }
        out + "}"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stats_match_an_exact_pass() {
        let mut s = TableStats::default();
        let mut xs = Vec::new();
        let mut words = std::collections::HashSet::new();
        for i in 0..50_000u64 {
            let x = ((i * 2654435761) % 100_000) as f64 / 7.0;
            xs.push(x);
            let w = format!("w{}", i % 3000);
            words.insert(w.clone());
            let row = if i % 10 == 0 {
                format!(
                    r#"{{"x": {x}, "w": "{w}", "c": "{}", "n": null, "o": {{"a": [1, "]"]}}}}"#,
                    i % 3
                )
            } else {
                format!(
                    r#"{{"x":{x},"w":"{w}","c":"{}","b":{}}}"#,
                    i % 3,
                    i % 2 == 0
                )
            };
            s.observe(row.as_bytes());
        }
        s.observe(b"not json");
        s.observe(b"[1, \"a\\\"b\", true]");
        assert_eq!(s.rows, 50_002);
        let x = &mut s.columns.get_mut("x").unwrap().clone();
        assert_eq!(x.numbers, 50_000);
        let mean = xs.iter().sum::<f64>() / xs.len() as f64;
        let var = xs.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / xs.len() as f64;
        assert!((x.mean - mean).abs() < 1e-6 * mean.abs());
        assert!((x.variance() - var).abs() < 1e-6 * var);
        xs.sort_by(f64::total_cmp);
        assert_eq!((x.min, x.max), (xs[0], xs[xs.len() - 1]));
        for q in [0.01, 0.5, 0.99] {
            let est = x.quantiles.quantile(q).unwrap();
            let rank = xs.partition_point(|v| *v < est) as f64 / xs.len() as f64;
            assert!((rank - q).abs() < 0.01, "q {q}: rank {rank}");
        }
        let w = &s.columns["w"];
        let d = w.distinct.estimate();
        assert!(
            (d - words.len() as f64).abs() < 0.05 * words.len() as f64,
            "{d}"
        );
        assert!(
            w.values.is_none(),
            "3000 words is more than the categories kept"
        );
        let c = &s.columns["c"];
        assert_eq!(c.values.as_ref().unwrap().len(), 3);
        assert_eq!(c.values.as_ref().unwrap()[&b"0"[..].to_vec()], 16_667);
        assert_eq!(s.columns["n"].nulls, 5_000);
        assert_eq!(s.columns["o"].other, 5_000);
        assert_eq!(s.columns["b"].bools, 45_000);
        assert_eq!(s.columns["1"].strings, 1);
        assert!(s.to_json().starts_with("{\"rows\":50002,"));
        assert!(!s.to_json().contains("\"zones\""));
    }

    #[test]
    fn zone_maps_match_an_exact_pass_and_skip_what_cannot_match() {
        let rows: Vec<(String, String)> = (0..10_000u64)
            .map(|i| {
                let x = ((i * 2654435761) % 100_000) as f64 / 7.0;
                let v = if i % 7 == 0 {
                    format!(r#"[{i}, null, "w{}"]"#, i % 50)
                } else {
                    format!(r#"[{i}, {x}, "w{}"]"#, i % 50)
                };
                (format!("k{i:06}"), v)
            })
            .collect();
        let mut s = TableStats::with_zones(1000);
        let mut plain = TableStats::default();
        for (k, v) in &rows {
            s.observe_at(k, v.as_bytes());
            plain.observe(v.as_bytes());
        }
        // Zones change none of the other stats.
        let full = s.to_json();
        assert!(full.starts_with(&plain.to_json()[..plain.to_json().len() - 1]));
        let z = s.zones.as_ref().unwrap();
        assert_eq!(z.zones.len(), 10);
        for (n, zone) in z.zones.iter().enumerate() {
            let part = &rows[n * 1000..(n + 1) * 1000];
            assert_eq!(zone.first_key.as_deref(), Some(&part[0].0[..]));
            assert_eq!(zone.last_key.as_deref(), Some(&part[999].0[..]));
            let xs: Vec<f64> = part
                .iter()
                .filter_map(|(_, v)| v.split(", ").nth(1)?.parse().ok())
                .collect();
            let x = &zone.columns[s.columns["1"].idx];
            assert_eq!(x.count, 1000);
            assert_eq!(x.nulls as usize, 1000 - xs.len());
            let lo = xs.iter().copied().fold(f64::INFINITY, f64::min);
            let hi = xs.iter().copied().fold(f64::NEG_INFINITY, f64::max);
            assert_eq!(x.num, Some((lo, hi)));
            let id = &zone.columns[s.columns["0"].idx];
            assert_eq!(id.num, Some(((n * 1000) as f64, (n * 1000 + 999) as f64)));
            assert_eq!(
                zone.columns[s.columns["2"].idx].str,
                Some((b"w0".to_vec(), b"w9".to_vec()))
            );
        }
        // A range on the row number reads one zone; on the float every one;
        // a word outside every zone's range reads none.
        assert_eq!(
            s.zones_to_read("0", &Filter::Range(2500.0, 2600.0)),
            vec![2]
        );
        assert_eq!(
            s.zones_to_read("1", &Filter::Range(0.0, 20_000.0)).len(),
            10
        );
        assert!(s.zones_to_read("2", &Filter::Eq(b"x")).is_empty());
        assert_eq!(s.zones_to_read("9", &Filter::Eq(b"x")).len(), 10);
        assert!(full.contains("\"zones\":{\"zone_rows\":1000,"));
    }
}
