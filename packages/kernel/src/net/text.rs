//! A text form for nets (E270), so a net can be written and reviewed by
//! hand rather than built in code. One declaration per line:
//!
//! ```text
//! # comments run to the end of the line
//! net loop 1
//! place cpu 1              # a plain place with one anonymous token
//! place queued colored     # one named token per work item
//! transition propose agent propose: -> queued
//! transition start cmd bench: queued, cpu -> running
//! transition finish pure: running -> measured, cpu
//! transition merge human: waiting ->
//! ```
//!
//! A transition line is `transition NAME KIND [CAPABILITY]: INPUTS -> OUTPUTS`,
//! each side a comma-separated list of places, a place optionally prefixed
//! by a weight (`2*cpu`). `net` comes first and once; places come before
//! the transitions that use them. [`Net::parse`] checks the result with
//! [`Net::check`]; [`Net::to_text`] prints the canonical form, which parses
//! back to the same net. Errors name the line.

use super::{Arc, Kind, Net, Place, Transition, valid_name};
use crate::Error;

fn err(line: usize, msg: impl std::fmt::Display) -> Error {
    Error::InvalidName(format!("net text line {line}: {msg}"))
}

fn parse_side(line: usize, s: &str) -> Result<Vec<Arc>, Error> {
    let s = s.trim();
    if s.is_empty() {
        return Ok(vec![]);
    }
    s.split(',')
        .map(|a| {
            let a = a.trim();
            let (weight, place) = match a.split_once('*') {
                Some((w, p)) => (
                    w.trim()
                        .parse::<u32>()
                        .map_err(|_| err(line, format!("bad weight in {a:?}")))?,
                    p.trim(),
                ),
                None => (1, a),
            };
            if !valid_name(place) {
                return Err(err(line, format!("bad place name {place:?}")));
            }
            Ok(Arc {
                place: place.into(),
                weight,
            })
        })
        .collect()
}

fn side_text(arcs: &[Arc]) -> String {
    let v: Vec<String> = arcs
        .iter()
        .map(|a| {
            if a.weight == 1 {
                a.place.clone()
            } else {
                format!("{}*{}", a.weight, a.place)
            }
        })
        .collect();
    v.join(", ")
}

impl Net {
    /// Parse the text form and check the net.
    pub fn parse(text: &str) -> Result<Net, Error> {
        let mut net: Option<Net> = None;
        for (i, raw) in text.lines().enumerate() {
            let line = i + 1;
            let l = raw.split('#').next().unwrap_or("").trim();
            if l.is_empty() {
                continue;
            }
            let (word, rest) = l.split_once(char::is_whitespace).unwrap_or((l, ""));
            let rest = rest.trim();
            if word == "net" {
                if net.is_some() {
                    return Err(err(line, "a second net line"));
                }
                let f: Vec<&str> = rest.split_whitespace().collect();
                let [name, version] = f[..] else {
                    return Err(err(line, "expected: net NAME VERSION"));
                };
                if !valid_name(name) {
                    return Err(err(line, format!("bad net name {name:?}")));
                }
                net = Some(Net {
                    name: name.into(),
                    version: version
                        .parse()
                        .map_err(|_| err(line, format!("bad version {version:?}")))?,
                    places: vec![],
                    transitions: vec![],
                });
                continue;
            }
            let n = net
                .as_mut()
                .ok_or_else(|| err(line, "expected the net line first"))?;
            match word {
                "place" => {
                    let f: Vec<&str> = rest.split_whitespace().collect();
                    let (name, colored, initial) = match f[..] {
                        [name] => (name, false, 0),
                        [name, "colored"] => (name, true, 0),
                        [name, k] => (
                            name,
                            false,
                            k.parse()
                                .map_err(|_| err(line, format!("bad token count {k:?}")))?,
                        ),
                        _ => return Err(err(line, "expected: place NAME [COUNT | colored]")),
                    };
                    if !valid_name(name) {
                        return Err(err(line, format!("bad place name {name:?}")));
                    }
                    if n.place(name).is_some() {
                        return Err(err(line, format!("place {name} defined twice")));
                    }
                    n.places.push(Place {
                        name: name.into(),
                        colored,
                        initial,
                    });
                }
                "transition" => {
                    let (head, arcs) = rest
                        .split_once(':')
                        .ok_or_else(|| err(line, "expected a colon before the arcs"))?;
                    let h: Vec<&str> = head.split_whitespace().collect();
                    let (name, kind, cap) = match h[..] {
                        [name, kind] => (name, kind, None),
                        [name, kind, cap] => (name, kind, Some(cap)),
                        _ => {
                            return Err(err(
                                line,
                                "expected: transition NAME KIND [CAPABILITY]: IN -> OUT",
                            ));
                        }
                    };
                    let kind =
                        Kind::parse(kind).ok_or_else(|| err(line, format!("bad kind {kind:?}")))?;
                    let (ins, outs) = arcs
                        .split_once("->")
                        .ok_or_else(|| err(line, "expected -> between inputs and outputs"))?;
                    let t = Transition {
                        name: name.into(),
                        inputs: parse_side(line, ins)?,
                        outputs: parse_side(line, outs)?,
                        kind,
                        capability: cap.map(String::from),
                    };
                    if let Some(a) =
                        (t.inputs.iter().chain(&t.outputs)).find(|a| n.place(&a.place).is_none())
                    {
                        return Err(err(line, format!("undefined place {}", a.place)));
                    }
                    n.transitions.push(t);
                }
                _ => return Err(err(line, format!("unknown declaration {word:?}"))),
            }
            // Check what we have so far, so an error names this line.
            n.check().map_err(|e| err(line, e))?;
        }
        let net = net.ok_or_else(|| err(0, "no net line"))?;
        net.check()?;
        Ok(net)
    }

    /// The canonical text form: the net line, places, then transitions, in
    /// their order, single spaces, no comments.
    pub fn to_text(&self) -> String {
        let mut out = format!("net {} {}\n", self.name, self.version);
        for p in &self.places {
            out += &match (p.colored, p.initial) {
                (true, _) => format!("place {} colored\n", p.name),
                (false, 0) => format!("place {}\n", p.name),
                (false, k) => format!("place {} {k}\n", p.name),
            };
        }
        for t in &self.transitions {
            let cap = t
                .capability
                .as_ref()
                .map(|c| format!(" {c}"))
                .unwrap_or_default();
            let ins = side_text(&t.inputs);
            let outs = side_text(&t.outputs);
            let arrow = match (ins.is_empty(), outs.is_empty()) {
                (true, true) => "->".to_string(),
                (true, false) => format!("-> {outs}"),
                (false, true) => format!("{ins} ->"),
                (false, false) => format!("{ins} -> {outs}"),
            };
            out += &format!("transition {} {}{cap}: {arrow}\n", t.name, t.kind.name());
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOOP: &str = "\
# the experiment loop
net loop 1
place cpu 1
place queued colored
place running colored
place measured colored
place waiting colored
transition propose agent propose: -> queued
transition start cmd bench: queued, cpu -> running
transition finish pure: running -> measured, cpu   # frees the slot
transition ask pure: measured -> waiting
transition merge human: waiting ->
";

    #[test]
    fn parses_prints_and_round_trips() {
        let net = Net::parse(LOOP).unwrap();
        let mut want = super::super::tests::loop_net();
        want.transitions[0].capability = Some("propose".into());
        want.transitions[1].capability = Some("bench".into());
        assert_eq!(net, want);
        let text = net.to_text();
        assert_eq!(Net::parse(&text).unwrap(), net);
        assert_eq!(
            Net::parse(&text).unwrap().to_text(),
            text,
            "canonical is a fixed point"
        );
        assert!(!text.contains('#'));
    }

    #[test]
    fn weights_round_trip() {
        let net =
            Net::parse("net w 1\nplace a 3\nplace b\ntransition t pure: 2*a -> b, 3*b\n").unwrap();
        assert_eq!(net.transitions[0].inputs[0].weight, 2);
        assert_eq!(net.transitions[0].outputs[1].weight, 3);
        assert_eq!(Net::parse(&net.to_text()).unwrap(), net);
    }

    #[test]
    fn errors_name_the_line() {
        let bad = [
            ("place a\n", 1, "net line first"),
            ("net n 1\nnet m 1\n", 2, "second net"),
            ("net n 1\nplace a\nplace a\n", 3, "twice"),
            (
                "net n 1\nplace a\ntransition t pure: a -> b\n",
                3,
                "undefined place b",
            ),
            (
                "net n 1\nplace a\ntransition t magic: a ->\n",
                3,
                "bad kind",
            ),
            ("net n 1\nplace a\ntransition t pure a ->\n", 3, "colon"),
            ("net n 1\nplace a\ntransition t pure: a\n", 3, "->"),
            (
                "net n 1\nplace a\ntransition t pure: 0*a ->\n",
                3,
                "weight 0",
            ),
            (
                "net n 1\nplace a\ntransition t human merge: a ->\n",
                3,
                "capability",
            ),
            (
                "net n 1\nplace a\ntransition a pure: a ->\n",
                3,
                "transition a",
            ),
            ("net n 1\nplace a\ntransition t pure: ->\n", 3, "no arcs"),
            ("net n 1\nplace a b c\n", 2, "expected: place"),
            ("net n 1\nfrobnicate\n", 2, "unknown declaration"),
            ("", 0, "no net line"),
        ];
        for (text, line, what) in bad {
            let e = Net::parse(text).unwrap_err().to_string();
            assert!(e.contains(&format!("line {line}:")), "{text:?}: {e}");
            assert!(e.contains(what), "{text:?}: {e}");
        }
    }
}
