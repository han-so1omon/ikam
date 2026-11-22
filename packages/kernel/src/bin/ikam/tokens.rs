//! Tokens and rate limits for `ikam host` (E363): what a program needs to
//! reach one project without a person's password, and what keeps one
//! project's callers from using up a shared host.
//!
//! - A token opens one project, with a role no higher than the one its
//!   config line gives, until it expires. A client sends it as
//!   `Authorization: Bearer ikt_ID_SECRET`. The config holds `ID:HASH`
//!   only (BLAKE3 over the 32-byte secret, as `ikam token` prints it), so
//!   a leaked config gives no token. An unknown, wrong, expired or other
//!   project's token gets the same 401 as a wrong password.
//! - A rate is `N/s`, `N/m` or `N/h`: a bucket of N requests that refills
//!   at that rate, so a quiet caller may send N at once. A token may have
//!   its own (`rate=`), and a project one for all its callers together
//!   (`rate NAME N/s`). A request over either gets 429 with `Retry-After`
//!   in whole seconds, before it reaches the project. Rates count requests
//!   that passed the password or token check only.
//!
//! Not here: tokens for a part of a project (a path or a share's scope);
//! limits on failed checks; keeping counts across a restart (buckets start
//! full).

use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::host::Role;

/// Requests a caller may send: `per` requests each `every`, at most `per`
/// at once.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rate {
    pub per: u32,
    pub every: Duration,
}

impl Rate {
    pub fn parse(s: &str) -> Result<Rate, String> {
        let bad = || format!("a rate is N/s, N/m or N/h, not {s}");
        let (n, unit) = s.split_once('/').ok_or_else(bad)?;
        let per: u32 = n.parse().map_err(|_| bad())?;
        let every = match unit {
            "s" => Duration::from_secs(1),
            "m" => Duration::from_secs(60),
            "h" => Duration::from_secs(3600),
            _ => return Err(bad()),
        };
        if per == 0 {
            return Err(bad());
        }
        Ok(Rate { per, every })
    }
}

/// A token bucket.
#[derive(Debug)]
pub struct Bucket {
    rate: Rate,
    left: f64,
    at: Instant,
}

impl Bucket {
    pub fn new(rate: Rate, now: Instant) -> Bucket {
        Bucket {
            rate,
            left: rate.per as f64,
            at: now,
        }
    }

    /// Take one request at `now`; if none is left, how long until one is.
    pub fn take(&mut self, now: Instant) -> Result<(), Duration> {
        let per_sec = self.rate.per as f64 / self.rate.every.as_secs_f64();
        let gained = now.saturating_duration_since(self.at).as_secs_f64() * per_sec;
        self.left = (self.left + gained).min(self.rate.per as f64);
        self.at = now;
        if self.left >= 1.0 {
            self.left -= 1.0;
            Ok(())
        } else {
            Err(Duration::from_secs_f64((1.0 - self.left) / per_sec))
        }
    }
}

/// One `token` line of a host config.
pub struct Token {
    pub id: String,
    hash: blake3::Hash,
    pub role: Role,
    /// Seconds since 1970; none: never.
    pub expires: Option<u64>,
    pub bucket: Option<Mutex<Bucket>>,
}

fn digest(secret: &[u8]) -> blake3::Hash {
    blake3::derive_key("ikam host token v1", secret).into()
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect()
}

fn valid_id(id: &str) -> bool {
    id.len() == 16
        && id
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

/// A new token: (`ID:HASH` for the config, the token to give the client).
pub fn make() -> Result<(String, String), String> {
    let mut bytes = [0u8; 40];
    use std::io::Read;
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut bytes))
        .map_err(|e| format!("random token: {e}"))?;
    let (id, secret) = bytes.split_at(8);
    let id = hex(id);
    Ok((
        format!("{id}:{}", digest(secret).to_hex()),
        format!("ikt_{id}_{}", hex(secret)),
    ))
}

impl Token {
    /// `ID:HASH` with its role, `rate=` and `expires=` words.
    pub fn parse(role: Role, line: &str, words: &[&str], now: Instant) -> Result<Token, String> {
        let bad = || "a token is ID:HASH from `ikam token`".to_string();
        let (id, hash) = line.split_once(':').ok_or_else(bad)?;
        if !valid_id(id) {
            return Err(bad());
        }
        let hash = blake3::Hash::from_hex(hash).map_err(|_| bad())?;
        let mut t = Token {
            id: id.to_string(),
            hash,
            role,
            expires: None,
            bucket: None,
        };
        for w in words {
            match w.split_once('=') {
                Some(("rate", r)) => {
                    t.bucket = Some(Mutex::new(Bucket::new(Rate::parse(r)?, now)));
                }
                Some(("expires", e)) => {
                    let e = e
                        .parse()
                        .map_err(|_| "expires= is seconds since 1970".to_string())?;
                    t.expires = Some(e);
                }
                _ => return Err(format!("not understood: {w}")),
            }
        }
        Ok(t)
    }

    /// Whether `secret` (hex) is this token's and it has not expired at
    /// `now_secs`.
    fn accepts(&self, secret: &str, now_secs: u64) -> bool {
        let Some(secret) = unhex(secret).filter(|s| s.len() == 32) else {
            return false;
        };
        // blake3::Hash compares in constant time.
        digest(&secret) == self.hash && self.expires.is_none_or(|e| now_secs < e)
    }
}

/// The token among `tokens` that a request's `Authorization: Bearer`
/// header names and proves, if any.
pub fn bearer<'a>(tokens: &'a [Token], header: Option<&str>, now_secs: u64) -> Option<&'a Token> {
    let (scheme, rest) = header?.split_once(' ')?;
    if !scheme.eq_ignore_ascii_case("bearer") {
        return None;
    }
    let (id, secret) = rest.trim().strip_prefix("ikt_")?.split_once('_')?;
    tokens
        .iter()
        .find(|t| t.id == id)
        .filter(|t| t.accepts(secret, now_secs))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rates_parse_and_buckets_refill() {
        assert_eq!(
            Rate::parse("5/m").unwrap(),
            Rate {
                per: 5,
                every: Duration::from_secs(60)
            }
        );
        for bad in ["5", "0/s", "x/s", "5/d", "-1/s"] {
            assert!(Rate::parse(bad).is_err(), "{bad}");
        }
        let t0 = Instant::now();
        let mut b = Bucket::new(Rate::parse("3/s").unwrap(), t0);
        for _ in 0..3 {
            assert!(b.take(t0).is_ok());
        }
        let wait = b.take(t0).unwrap_err();
        assert!(wait > Duration::from_millis(300) && wait <= Duration::from_millis(334));
        assert!(b.take(t0 + Duration::from_millis(340)).is_ok());
        assert!(b.take(t0 + Duration::from_millis(340)).is_err());
        // A long quiet time fills the bucket to N, no more.
        let later = t0 + Duration::from_secs(60);
        for _ in 0..3 {
            assert!(b.take(later).is_ok());
        }
        assert!(b.take(later).is_err());
    }

    #[test]
    fn tokens_prove_their_secret_until_they_expire() {
        let now = Instant::now();
        let (line, token) = make().unwrap();
        let t = Token::parse(Role::Read, &line, &["expires=2000"], now).unwrap();
        let tokens = [t];
        let h = format!("Bearer {token}");
        assert!(bearer(&tokens, Some(&h), 1999).is_some());
        assert!(bearer(&tokens, Some(&h), 2000).is_none());
        assert!(bearer(&tokens, Some(&h.replace("Bearer", "bearer")), 10).is_some());
        let mut wrong = token.clone();
        let last = wrong.pop().unwrap();
        wrong.push(if last == '0' { '1' } else { '0' });
        assert!(bearer(&tokens, Some(&format!("Bearer {wrong}")), 10).is_none());
        assert!(bearer(&tokens, Some(&format!("Basic {token}")), 10).is_none());
        assert!(bearer(&tokens, Some("Bearer ikt_short"), 10).is_none());
        assert!(bearer(&tokens, None, 10).is_none());
        let (other, _) = make().unwrap();
        assert_ne!(other, line);
        for bad in ["nocolon", "ABCDEF0123456789:00", "0123:00"] {
            assert!(Token::parse(Role::Read, bad, &[], now).is_err(), "{bad}");
        }
        assert!(Token::parse(Role::Read, &line, &["rate=9"], now).is_err());
        assert!(Token::parse(Role::Read, &line, &["scope=x"], now).is_err());
    }
}
