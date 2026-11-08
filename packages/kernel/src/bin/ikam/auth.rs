//! What `ikam live` needs to be reached from other machines safely: names it
//! answers to besides 127.0.0.1 (`--allow-host`), an optional password on
//! every request (HTTP basic auth, which Finder's WebDAV client can send), and
//! a clean stop on SIGTERM or SIGINT that commits pending edits first.
//!
//! The password is kept as `IKAM_AUTH=user:salt:hash`, made by `ikam passwd`,
//! where hash is BLAKE3 over salt and the password. BLAKE3 is fast, so the
//! line should stay in a file only its owner can read.

use std::sync::atomic::{AtomicBool, Ordering};

use crate::http::{Request, Response};

/// Hosts a request may name: 127.0.0.1 and localhost on the port, and each
/// allowed name with or without a port (a proxy such as Tailscale Serve may
/// send the name alone).
pub struct Hosts {
    local: [String; 2],
    names: Vec<String>,
}

impl Hosts {
    pub fn new(port: u16, names: Vec<String>) -> Hosts {
        Hosts {
            local: [format!("127.0.0.1:{port}"), format!("localhost:{port}")],
            names: names.into_iter().map(|n| n.to_ascii_lowercase()).collect(),
        }
    }

    pub fn ok(&self, host: &str) -> bool {
        let host = host.to_ascii_lowercase();
        if self.local.contains(&host) {
            return true;
        }
        let name = match host.rsplit_once(':') {
            Some((n, p)) if !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()) => n,
            _ => host.as_str(),
        };
        self.names.iter().any(|n| n == name)
    }
}

pub struct Auth {
    user: String,
    salt: Vec<u8>,
    hash: blake3::Hash,
}

impl Auth {
    /// From `IKAM_AUTH`; none when it is unset or empty.
    pub fn from_env() -> Result<Option<Auth>, String> {
        match std::env::var("IKAM_AUTH") {
            Ok(v) if !v.is_empty() => Auth::parse(&v).map(Some),
            _ => Ok(None),
        }
    }

    pub fn parse(line: &str) -> Result<Auth, String> {
        let bad = || "IKAM_AUTH: expected user:salt:hash from `ikam passwd`".to_string();
        let mut parts = line.trim().splitn(3, ':');
        let (Some(user), Some(salt), Some(hash)) = (parts.next(), parts.next(), parts.next())
        else {
            return Err(bad());
        };
        if user.is_empty() {
            return Err(bad());
        }
        Ok(Auth {
            user: user.into(),
            salt: unhex(salt).ok_or_else(bad)?,
            hash: blake3::Hash::from_hex(hash).map_err(|_| bad())?,
        })
    }

    /// The `IKAM_AUTH` value for `user` and `password`, with a fresh salt.
    pub fn line(user: &str, password: &str) -> Result<String, String> {
        if user.is_empty() || user.contains(':') {
            return Err("the user name must be non-empty and have no ':'".into());
        }
        let mut salt = [0u8; 16];
        use std::io::Read;
        std::fs::File::open("/dev/urandom")
            .and_then(|mut f| f.read_exact(&mut salt))
            .map_err(|e| format!("random salt: {e}"))?;
        let salt_hex: String = salt.iter().map(|b| format!("{b:02x}")).collect();
        Ok(format!("{user}:{salt_hex}:{}", digest(&salt, password)))
    }

    fn accepts(&self, user: &str, password: &str) -> bool {
        // blake3::Hash compares in constant time; the user check is not
        // secret enough to need it.
        let same_hash = digest(&self.salt, password) == self.hash;
        same_hash && user == self.user
    }

    /// None if the request carries the right user and password; otherwise
    /// the 401 that asks for them.
    pub fn check(&self, req: &Request) -> Option<Response> {
        let ok = req
            .header("authorization")
            .and_then(|h| {
                let (scheme, rest) = h.split_once(' ')?;
                scheme.eq_ignore_ascii_case("basic").then_some(rest.trim())
            })
            .and_then(base64)
            .and_then(|b| String::from_utf8(b).ok())
            .and_then(|s| {
                let (u, p) = s.split_once(':')?;
                Some(self.accepts(u, p))
            })
            .unwrap_or(false);
        if ok {
            None
        } else {
            Some(Response::text(401, "password required").with(
                "WWW-Authenticate",
                "Basic realm=\"ikam\", charset=\"UTF-8\"",
            ))
        }
    }
}

fn digest(salt: &[u8], password: &str) -> blake3::Hash {
    let mut h = blake3::Hasher::new_derive_key("ikam live basic auth v1");
    h.update(salt);
    h.update(password.as_bytes());
    h.finalize()
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) || s.is_empty() {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect()
}

/// Standard base64, padding optional.
fn base64(s: &str) -> Option<Vec<u8>> {
    let val = |c: u8| -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        } as u32)
    };
    let s = s.trim_end_matches('=').as_bytes();
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    for chunk in s.chunks(4) {
        let mut n = 0u32;
        for (i, &c) in chunk.iter().enumerate() {
            n |= val(c)? << (18 - 6 * i);
        }
        match chunk.len() {
            4 => out.extend_from_slice(&[(n >> 16) as u8, (n >> 8) as u8, n as u8]),
            3 => out.extend_from_slice(&[(n >> 16) as u8, (n >> 8) as u8]),
            2 => out.push((n >> 16) as u8),
            _ => return None,
        }
    }
    Some(out)
}

static STOP: AtomicBool = AtomicBool::new(false);

/// Ask for SIGTERM (what launchd and systemd send) and SIGINT (Ctrl-C) to set
/// a flag instead of killing the process; `stopping` reads it.
pub fn catch_stop_signals() {
    #[cfg(unix)]
    {
        unsafe extern "C" {
            fn signal(signum: i32, handler: usize) -> usize;
        }
        extern "C" fn on_signal(_: i32) {
            STOP.store(true, Ordering::SeqCst);
        }
        // SIGINT and SIGTERM are 2 and 15 on Linux and macOS alike.
        for sig in [2, 15] {
            unsafe {
                signal(sig, on_signal as extern "C" fn(i32) as usize);
            }
        }
    }
}

pub fn stopping() -> bool {
    STOP.load(Ordering::SeqCst)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(auth: Option<&str>) -> Request {
        Request {
            method: "GET".into(),
            path: "/".into(),
            query: String::new(),
            headers: auth
                .map(|a| vec![("Authorization".into(), a.into())])
                .unwrap_or_default(),
            body: Vec::new(),
        }
    }

    #[test]
    fn hosts() {
        let h = Hosts::new(8737, vec!["Mac.tail1234.ts.net".into()]);
        assert!(h.ok("127.0.0.1:8737"));
        assert!(h.ok("localhost:8737"));
        assert!(h.ok("mac.tail1234.ts.net"));
        assert!(h.ok("mac.tail1234.ts.net:443"));
        assert!(!h.ok("127.0.0.1:9999"));
        assert!(!h.ok("127.0.0.1"));
        assert!(!h.ok("evil.example"));
        assert!(!h.ok("mac.tail1234.ts.net.evil.example"));
        assert!(!h.ok("mac.tail1234.ts.net:443x"));
        assert!(!Hosts::new(8737, vec![]).ok("mac.tail1234.ts.net"));
    }

    #[test]
    fn base64_decodes() {
        assert_eq!(base64("ZXJpYzpodW50ZXIy").unwrap(), b"eric:hunter2");
        assert_eq!(base64("YQ==").unwrap(), b"a");
        assert_eq!(base64("YWI=").unwrap(), b"ab");
        assert!(base64("Y").is_none());
        assert!(base64("!!!!").is_none());
    }

    #[test]
    fn password_check() {
        let auth = Auth::parse(&Auth::line("eric", "hunter2:with colon").unwrap()).unwrap();
        let enc = |s: &str| {
            const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
            let mut out = String::new();
            for c in s.as_bytes().chunks(3) {
                let n = (c[0] as u32) << 16
                    | (*c.get(1).unwrap_or(&0) as u32) << 8
                    | *c.get(2).unwrap_or(&0) as u32;
                for i in 0..4 {
                    if i <= c.len() {
                        out.push(T[(n >> (18 - 6 * i) & 63) as usize] as char);
                    } else {
                        out.push('=');
                    }
                }
            }
            out
        };
        assert!(
            auth.check(&req(Some(&format!(
                "Basic {}",
                enc("eric:hunter2:with colon")
            ))))
            .is_none()
        );
        for bad in [
            None,
            Some(format!("Basic {}", enc("eric:hunter3"))),
            Some(format!("Basic {}", enc("bob:hunter2:with colon"))),
            Some(format!("Bearer {}", enc("eric:hunter2:with colon"))),
            Some("Basic not-base64".into()),
        ] {
            let r = auth.check(&req(bad.as_deref())).expect("refused");
            assert_eq!(r.status, 401);
        }
        // A fresh salt each time.
        assert_ne!(
            Auth::line("eric", "x").unwrap(),
            Auth::line("eric", "x").unwrap()
        );
        assert!(Auth::parse("eric:zz:00").is_err());
        assert!(Auth::line("a:b", "x").is_err());
    }
}
