//! Word stemmers for ranked keyword search (E344): a word's suffixes cut so
//! that its forms ("index", "indexes", "indexing") count as one term.
//!
//! - `plural`: the S-stemmer (Harman, 1991): plural "-ies", "-es" and "-s"
//!   only, conservative and cheap.
//! - `porter`: Porter's algorithm (Porter, 1980, "An algorithm for suffix
//!   stripping"), the original five steps, written from the paper's rules.
//!
//! Both change only words of three or more ASCII lowercase letters; other
//! terms (numbers, identifiers with digits, other scripts) pass unchanged.

fn plain(w: &str) -> bool {
    w.len() > 2 && w.bytes().all(|b| b.is_ascii_lowercase())
}

/// Harman's S-stemmer.
pub fn plural(w: &str) -> String {
    if !plain(w) {
        return w.to_string();
    }
    if w.ends_with("ies") && !w.ends_with("eies") && !w.ends_with("aies") {
        return format!("{}y", &w[..w.len() - 3]);
    }
    if w.ends_with("es") && !w.ends_with("aes") && !w.ends_with("ees") && !w.ends_with("oes") {
        return w[..w.len() - 1].to_string();
    }
    if w.ends_with('s') && !w.ends_with("us") && !w.ends_with("ss") {
        return w[..w.len() - 1].to_string();
    }
    w.to_string()
}

/// Whether `w[i]` is a consonant in Porter's sense: not a, e, i, o, u, and
/// not a y after a consonant.
fn cons(w: &[u8], i: usize) -> bool {
    match w[i] {
        b'a' | b'e' | b'i' | b'o' | b'u' => false,
        b'y' => i == 0 || !cons(w, i - 1),
        _ => true,
    }
}

/// Porter's m: the number of vowel-consonant sequences in `w`.
fn measure(w: &[u8]) -> usize {
    let mut m = 0;
    let mut i = 0;
    while i < w.len() && cons(w, i) {
        i += 1;
    }
    loop {
        while i < w.len() && !cons(w, i) {
            i += 1;
        }
        if i >= w.len() {
            return m;
        }
        while i < w.len() && cons(w, i) {
            i += 1;
        }
        m += 1;
    }
}

fn has_vowel(w: &[u8]) -> bool {
    (0..w.len()).any(|i| !cons(w, i))
}

/// `*d`: ends with a double consonant.
fn double_cons(w: &[u8]) -> bool {
    let n = w.len();
    n >= 2 && w[n - 1] == w[n - 2] && cons(w, n - 1)
}

/// `*o`: ends consonant-vowel-consonant, the last not w, x or y.
fn cvc(w: &[u8]) -> bool {
    let n = w.len();
    n >= 3
        && cons(w, n - 3)
        && !cons(w, n - 2)
        && cons(w, n - 1)
        && !matches!(w[n - 1], b'w' | b'x' | b'y')
}

/// If `w` ends with `suf` and the stem before it meets `cond`, replace the
/// suffix with `rep`. `Some(changed)` when the suffix matched at all.
fn rule(w: &mut Vec<u8>, suf: &str, rep: &str, cond: impl Fn(&[u8]) -> bool) -> Option<bool> {
    let suf = suf.as_bytes();
    if !w.ends_with(suf) {
        return None;
    }
    let stem = w.len() - suf.len();
    if cond(&w[..stem]) {
        w.truncate(stem);
        w.extend_from_slice(rep.as_bytes());
        Some(true)
    } else {
        Some(false)
    }
}

/// The first rule of `rules` whose suffix matches decides (Porter's
/// longest match: the lists are in that order), whether or not its
/// condition holds.
fn first(w: &mut Vec<u8>, rules: &[(&str, &str)], cond: impl Fn(&[u8]) -> bool + Copy) {
    for (suf, rep) in rules {
        if rule(w, suf, rep, cond).is_some() {
            return;
        }
    }
}

const STEP2: [(&str, &str); 20] = [
    ("ational", "ate"),
    ("tional", "tion"),
    ("enci", "ence"),
    ("anci", "ance"),
    ("izer", "ize"),
    ("abli", "able"),
    ("alli", "al"),
    ("entli", "ent"),
    ("eli", "e"),
    ("ousli", "ous"),
    ("ization", "ize"),
    ("ation", "ate"),
    ("ator", "ate"),
    ("alism", "al"),
    ("iveness", "ive"),
    ("fulness", "ful"),
    ("ousness", "ous"),
    ("aliti", "al"),
    ("iviti", "ive"),
    ("biliti", "ble"),
];

const STEP3: [(&str, &str); 7] = [
    ("icate", "ic"),
    ("ative", ""),
    ("alize", "al"),
    ("iciti", "ic"),
    ("ical", "ic"),
    ("ful", ""),
    ("ness", ""),
];

const STEP4: [&str; 19] = [
    "al", "ance", "ence", "er", "ic", "able", "ible", "ant", "ement", "ment", "ent", "ion", "ou",
    "ism", "ate", "iti", "ous", "ive", "ize",
];

/// The suffix of `w` among `sufs` that Porter's step would take: the
/// longest one that matches.
fn longest<'a>(w: &[u8], sufs: impl Iterator<Item = &'a str>) -> Option<&'a str> {
    sufs.filter(|s| w.ends_with(s.as_bytes()))
        .max_by_key(|s| s.len())
}

/// Porter's stemmer.
pub fn porter(word: &str) -> String {
    if !plain(word) {
        return word.to_string();
    }
    let mut w = word.as_bytes().to_vec();
    // Step 1a.
    first(
        &mut w,
        &[("sses", "ss"), ("ies", "i"), ("ss", "ss"), ("s", "")],
        |_| true,
    );
    // Step 1b.
    if w.ends_with(b"eed") {
        rule(&mut w, "eed", "ee", |s| measure(s) > 0);
    } else if rule(&mut w, "ed", "", has_vowel) == Some(true)
        || rule(&mut w, "ing", "", has_vowel) == Some(true)
    {
        if w.ends_with(b"at") || w.ends_with(b"bl") || w.ends_with(b"iz") {
            w.push(b'e');
        } else if double_cons(&w) && !matches!(w[w.len() - 1], b'l' | b's' | b'z') {
            w.pop();
        } else if measure(&w) == 1 && cvc(&w) {
            w.push(b'e');
        }
    }
    // Step 1c.
    rule(&mut w, "y", "i", has_vowel);
    // Steps 2 and 3: the longest matching suffix, if its stem has m > 0.
    for rules in [&STEP2[..], &STEP3[..]] {
        if let Some(suf) = longest(&w, rules.iter().map(|r| r.0)) {
            let rep = rules.iter().find(|r| r.0 == suf).unwrap().1;
            rule(&mut w, suf, rep, |s| measure(s) > 0);
        }
    }
    // Step 4: m > 1, and "ion" only after s or t.
    if let Some(suf) = longest(&w, STEP4.iter().copied()) {
        rule(&mut w, suf, "", |s| {
            measure(s) > 1 && (suf != "ion" || s.ends_with(b"s") || s.ends_with(b"t"))
        });
    }
    // Step 5a.
    if w.ends_with(b"e") {
        let s = &w[..w.len() - 1];
        if measure(s) > 1 || (measure(s) == 1 && !cvc(s)) {
            w.pop();
        }
    }
    // Step 5b.
    if measure(&w) > 1 && double_cons(&w) && w.ends_with(b"l") {
        w.pop();
    }
    String::from_utf8(w).expect("ASCII in, ASCII out")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Examples from Porter's paper and its published vocabulary.
    #[test]
    fn porter_paper_examples() {
        for (w, s) in [
            ("caresses", "caress"),
            ("ponies", "poni"),
            ("ties", "ti"),
            ("caress", "caress"),
            ("cats", "cat"),
            ("feed", "feed"),
            ("agreed", "agre"),
            ("plastered", "plaster"),
            ("bled", "bled"),
            ("motoring", "motor"),
            ("sing", "sing"),
            ("conflated", "conflat"),
            ("troubled", "troubl"),
            ("sized", "size"),
            ("hopping", "hop"),
            ("tanned", "tan"),
            ("falling", "fall"),
            ("hissing", "hiss"),
            ("fizzed", "fizz"),
            ("failing", "fail"),
            ("filing", "file"),
            ("happy", "happi"),
            ("sky", "sky"),
            ("relational", "relat"),
            ("conditional", "condit"),
            ("rational", "ration"),
            ("valenci", "valenc"),
            ("digitizer", "digit"),
            ("vietnamization", "vietnam"),
            ("predication", "predic"),
            ("operator", "oper"),
            ("feudalism", "feudal"),
            ("decisiveness", "decis"),
            ("hopefulness", "hope"),
            ("callousness", "callous"),
            ("formaliti", "formal"),
            ("sensitiviti", "sensit"),
            ("sensibiliti", "sensibl"),
            ("triplicate", "triplic"),
            ("formative", "form"),
            ("formalize", "formal"),
            ("electriciti", "electr"),
            ("electrical", "electr"),
            ("hopeful", "hope"),
            ("goodness", "good"),
            ("revival", "reviv"),
            ("allowance", "allow"),
            ("inference", "infer"),
            ("airliner", "airlin"),
            ("adjustable", "adjust"),
            ("defensible", "defens"),
            ("irritant", "irrit"),
            ("replacement", "replac"),
            ("adjustment", "adjust"),
            ("dependent", "depend"),
            ("adoption", "adopt"),
            ("homologou", "homolog"),
            ("communism", "commun"),
            ("activate", "activ"),
            ("angulariti", "angular"),
            ("homologous", "homolog"),
            ("effective", "effect"),
            ("bowdlerize", "bowdler"),
            ("probate", "probat"),
            ("rate", "rate"),
            ("cease", "ceas"),
            ("controll", "control"),
            ("roll", "roll"),
            ("generalizations", "gener"),
            ("oscillators", "oscil"),
            ("indexing", "index"),
            ("indexes", "index"),
            ("python3", "python3"),
            ("é", "é"),
        ] {
            assert_eq!(porter(w), s, "{w}");
        }
    }

    #[test]
    fn plural_examples() {
        for (w, s) in [
            ("queries", "query"),
            ("indexes", "indexe"),
            ("files", "file"),
            ("tools", "tool"),
            ("status", "status"),
            ("class", "class"),
            ("toes", "toe"),
            ("is", "is"),
        ] {
            assert_eq!(plural(w), s, "{w}");
        }
    }
}
