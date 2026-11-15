//! A net lowered to dense arrays (E278, opt-in), for simulation: many steps
//! in memory, such as trying an order of work before running it. E277
//! measured this form at 26x (10 places) to over 2,000x (1,000 places)
//! faster than `Net::fire` on string-keyed markings, with no compile step.
//!
//! Plain places only: a colored place would unfold to one place per color,
//! which the caller can do before lowering.

use super::{ANON, Marking, Net};
use crate::Error;

/// Place indices, initial counts, and each transition's input thresholds
/// and output additions, by index.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Table {
    pub places: Vec<String>,
    pub transitions: Vec<String>,
    pub initial: Vec<u64>,
    pub inputs: Vec<Vec<(usize, u64)>>,
    pub outputs: Vec<Vec<(usize, u64)>>,
}

impl Net {
    /// Lower a checked net with no colored places to a [`Table`].
    pub fn lower(&self) -> Result<Table, Error> {
        self.check()?;
        if let Some(p) = self.places.iter().find(|p| p.colored) {
            return Err(Error::InvalidName(format!(
                "colored place {} (unfold colors before lowering)",
                p.name
            )));
        }
        let idx = |name: &str| self.places.iter().position(|p| p.name == name).unwrap();
        let side = |arcs: &[super::Arc]| {
            arcs.iter()
                .map(|a| (idx(&a.place), a.weight as u64))
                .collect()
        };
        Ok(Table {
            places: self.places.iter().map(|p| p.name.clone()).collect(),
            transitions: self.transitions.iter().map(|t| t.name.clone()).collect(),
            initial: self.places.iter().map(|p| p.initial as u64).collect(),
            inputs: self.transitions.iter().map(|t| side(&t.inputs)).collect(),
            outputs: self.transitions.iter().map(|t| side(&t.outputs)).collect(),
        })
    }
}

impl Net {
    /// The same net with each colored place split into one plain place per
    /// color (`place.color`) and each transition that binds a color copied
    /// once per color (`transition.color`), in the order given (E285).
    /// Firing `t` with token `c` here is firing `t.c` there, so the result
    /// lowers to a [`Table`]. Transitions that bind no color are kept as
    /// they are. Colors must be valid names and distinct.
    pub fn unfold(&self, colors: &[&str]) -> Result<Net, Error> {
        self.check()?;
        let mut seen = std::collections::BTreeSet::new();
        for c in colors {
            if !super::valid_name(c) || !seen.insert(*c) {
                return Err(Error::InvalidName(format!("color {c}")));
            }
        }
        let colored = |p: &str| self.place(p).is_some_and(|pl| pl.colored);
        let mut places = Vec::new();
        for p in &self.places {
            if p.colored {
                for c in colors {
                    places.push(super::Place {
                        name: format!("{}.{c}", p.name),
                        colored: false,
                        initial: 0,
                    });
                }
            } else {
                places.push(p.clone());
            }
        }
        let mut transitions = Vec::new();
        for t in &self.transitions {
            if !self.binds_color(t) {
                transitions.push(t.clone());
                continue;
            }
            for c in colors {
                let map = |arcs: &[super::Arc]| {
                    arcs.iter()
                        .map(|a| super::Arc {
                            place: if colored(&a.place) {
                                format!("{}.{c}", a.place)
                            } else {
                                a.place.clone()
                            },
                            weight: a.weight,
                        })
                        .collect()
                };
                transitions.push(super::Transition {
                    name: format!("{}.{c}", t.name),
                    inputs: map(&t.inputs),
                    outputs: map(&t.outputs),
                    kind: t.kind,
                    capability: t.capability.clone(),
                });
            }
        }
        let out = Net {
            name: self.name.clone(),
            version: self.version,
            places,
            transitions,
        };
        out.check()?;
        Ok(out)
    }
}

impl Table {
    pub fn is_enabled(&self, m: &[u64], t: usize) -> bool {
        self.inputs[t].iter().all(|&(p, w)| m[p] >= w)
    }

    /// Fire `t` in place; false (and `m` unchanged) when not enabled.
    pub fn fire(&self, m: &mut [u64], t: usize) -> bool {
        if !self.is_enabled(m, t) {
            return false;
        }
        for &(p, w) in &self.inputs[t] {
            m[p] -= w;
        }
        for &(p, w) in &self.outputs[t] {
            m[p] += w;
        }
        true
    }

    /// Enabled transitions, in order, into `out`.
    pub fn enabled_into(&self, m: &[u64], out: &mut Vec<usize>) {
        out.clear();
        out.extend((0..self.inputs.len()).filter(|&t| self.is_enabled(m, t)));
    }

    /// A seeded random walk of at most `steps` firings from `m`: at each
    /// step the enabled transitions in order, one picked by `pick(count)`.
    /// Returns the steps taken (fewer when the net deadlocks).
    pub fn walk(&self, m: &mut [u64], steps: u64, mut pick: impl FnMut(usize) -> usize) -> u64 {
        let mut enabled = Vec::with_capacity(self.inputs.len());
        for step in 0..steps {
            self.enabled_into(m, &mut enabled);
            if enabled.is_empty() {
                return step;
            }
            let t = enabled[pick(enabled.len())];
            self.fire(m, t);
        }
        steps
    }

    /// The same counts as a [`Marking`] (anonymous tokens, no zeros).
    pub fn marking(&self, m: &[u64]) -> Marking {
        let mut out = Marking::new();
        for (p, &c) in m.iter().enumerate().filter(|(_, c)| **c > 0) {
            out.entry(self.places[p].clone())
                .or_default()
                .insert(ANON.into(), c);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::super::Net;

    #[test]
    fn agrees_with_the_interpreter() {
        let net = Net::parse(
            "net t 1\nplace a 3\nplace b\nplace c 1\n\
             transition ab pure: 2*a -> b\ntransition bc pure: b, c -> 2*c\n\
             transition ca pure: 2*c -> 3*a\n",
        )
        .unwrap();
        let table = net.lower().unwrap();
        let mut m = table.initial.clone();
        let mut im = net.initial_marking();
        let mut x = 7u64;
        for _ in 0..200 {
            let mut en = vec![];
            table.enabled_into(&m, &mut en);
            let ien: Vec<usize> = (0..net.transitions.len())
                .filter(|&t| net.is_enabled(&im, &net.transitions[t], None))
                .collect();
            assert_eq!(en, ien);
            if en.is_empty() {
                break;
            }
            x = x.wrapping_mul(6364136223846793005).wrapping_add(1);
            let t = en[(x >> 33) as usize % en.len()];
            assert!(table.fire(&mut m, t));
            im = net.fire(&im, &net.transitions[t], None).unwrap();
            assert_eq!(table.marking(&m), im);
        }
        assert!(
            Net::parse("net c 1\nplace q colored\ntransition t pure: -> q\n")
                .unwrap()
                .lower()
                .is_err()
        );
    }

    #[test]
    fn unfolded_table_agrees_with_colored_interpreter() {
        let net = crate::net::tests::loop_net();
        let colors: Vec<String> = (0..12).map(|i| format!("E{i:02}")).collect();
        let cs: Vec<&str> = colors.iter().map(String::as_str).collect();
        let t = net.unfold(&cs).unwrap().lower().unwrap();
        assert!(net.unfold(&["a", "a"]).is_err());
        for seed in 1..40u64 {
            let mut x = seed;
            let mut next = move || {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                x
            };
            let mut m = net.initial_marking();
            let mut tm = t.initial.clone();
            let mut en = Vec::new();
            for _ in 0..400 {
                let mut list = Vec::new();
                for tr in &net.transitions {
                    if net.binds_color(tr) {
                        for c in &cs {
                            if net.is_enabled(&m, tr, Some(c)) {
                                list.push((tr, Some(*c)));
                            }
                        }
                    } else if net.is_enabled(&m, tr, None) {
                        list.push((tr, None));
                    }
                }
                t.enabled_into(&tm, &mut en);
                assert_eq!(list.len(), en.len());
                if list.is_empty() {
                    break;
                }
                let k = (next() % list.len() as u64) as usize;
                let (tr, c) = list[k];
                m = net.fire(&m, tr, c).unwrap();
                assert!(t.fire(&mut tm, en[k]));
                let name = match c {
                    Some(c) => format!("{}.{c}", tr.name),
                    None => tr.name.clone(),
                };
                assert_eq!(t.transitions[en[k]], name);
            }
            // Same counts, place by place.
            for (i, p) in t.places.iter().enumerate() {
                let (base, tok) = p.split_once('.').unwrap_or((p, crate::net::ANON));
                let want = m.get(base).and_then(|x| x.get(tok)).copied().unwrap_or(0);
                assert_eq!(tm[i], want, "place {p}");
            }
        }
    }
}
