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
}
