//! `ikam host --config FILE [PORT]`: one process serving several separate
//! projects (E313, E314). Each project is its own store, opened as `ikam
//! live` opens one, with its own users, limits and share key; nothing is
//! shared between stores. A project is chosen by the request's Host name
//! (the names in the config, and `NAME.localhost`). On a host name given
//! with `explorer HOSTNAME` (E440, `explorer.rs`), `/` lists the projects
//! the caller opens with their links, and `/p/NAME/...` serves project NAME
//! as its own host name would; its pages resolve their links from there
//! (WebDAV mounts still take the project's own host name).
//!
//! A request names one project and is checked against that project's users
//! only, so a password for one project opens no other. Unknown host names
//! and wrong passwords get the same answers whatever exists.
//!
//! Roles: `read` (GET, HEAD, PROPFIND), `write` (also edits and commits),
//! `owner` (also repack, gc, fsck and the project's shares). Crossing
//! between projects happens only through a share (`ikam_kernel::share`):
//!
//! - `POST /shares/issue?name=&to=&scope=&right=&expires=` (source owner):
//!   pin the current commit; `scope` is `path:PREFIX`, `section:PATH@A-B,..`,
//!   `fragments:ID,..`, `records:START..END` (END may be empty) or
//!   `graph:PATH:PREFIX`; `right` is `read`, `propose` or `merge`; `expires`
//!   is seconds from now.
//! - `GET /shares`, `POST /shares/revoke?name=`, `POST /shares/accept?name=`
//!   (source owner; accept merges a proposal branch into the project).
//! - `/from/SOURCE/SHARE/ACTION` on the receiving project: `files`,
//!   `read?path=[&range=A-B]`, `search?q=`, `records?start=&end=`,
//!   `fragment?id=`, `nodes?prefix=` (read role), `propose?path=` with the
//!   new bytes as body (write role, propose right), and `merge[?into=REF]`
//!   (receiver owner, merge right). Every refusal of a share answers 404,
//!   as if the share did not exist.
//! - E416 (off): `share-rank NAME` lets NAME's folder (`path:`) shares
//!   answer `GET /from/NAME/SHARE/rank?q=[&k=][&vector=X,Y,..][&fuse=rrf|
//!   convex][&alpha=]`: the scope's files ranked as `ikam mcp`'s `rank`
//!   tool ranks a tree (E388), with BM25 statistics from the scope's files
//!   alone, so the answer is the one a store holding only the scope would
//!   give and nothing outside the scope moves a score. `file-vectors NAME
//!   PREFIX` loads vectors for NAME's files (`--file-vectors`), which the
//!   ranking fuses when the query gives a vector.
//! - E417 (off): `lines=1` on `merge` or `/shares/accept` merges a file
//!   both sides changed line by line (`MergeOptions::lines`): edits to
//!   different lines are both kept, edits to the same lines answer 409.
//!
//! A panic while serving one project is caught and answered 500; the other
//! projects keep serving. `--public` is refused: every request needs a user
//! or a token.
//!
//! Tokens and rates (E363, `tokens.rs`): `token NAME ROLE ID:HASH
//! [rate=N/s] [expires=SECS]` lets a program in with `Authorization: Bearer`
//! (`ikam token` makes one); `rate NAME N/s` limits all of a project's
//! callers together. Over a rate, a request gets 429 and `Retry-After`.
//!
//! Heavy jobs (E362): `heavy-jobs N` lets at most N maintenance jobs
//! (`/api/repack`, `/api/gc`, `/api/fsck`) run at once across all projects;
//! the others wait their turn in order, for up to `heavy-wait SECS` (60 by
//! default), then get 503 with `Retry-After`. Without the line, as before,
//! every owner's job runs at once.

use std::collections::{BTreeMap, VecDeque};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, SystemTime};

use ikam_kernel::bm25::Bm25Mem;
use ikam_kernel::merge::{MergeOptions, Merged};
use ikam_kernel::share::{Refused, Right, SHARES_REF, Scope, Share, import_share_with};
use ikam_kernel::{Id, Kind};

use crate::auth::{self, Auth};
use crate::http::{self, Request, Response};
use crate::json::Json;
use crate::live::{self, Live};
use crate::tokens::{Bucket, Rate, Token};
use crate::{dav, explorer, ui};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Role {
    Read,
    Write,
    Owner,
}

/// E440: a link on a project's explorer page, in a group such as
/// `project` or `demo` (any word; groups show in the order they first come).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    pub group: String,
    pub url: String,
    pub label: String,
}

/// One project as the config file declares it.
pub struct ProjectConfig {
    pub name: String,
    pub store: PathBuf,
    pub refname: String,
    pub hosts: Vec<String>,
    pub users: Vec<(Role, String)>,
    pub max_store: Option<u64>,
    pub max_body: Option<usize>,
    /// E363: role, `ID:HASH` and the words after it.
    pub tokens: Vec<(Role, String, Vec<String>)>,
    /// E363: a rate for all the project's callers together.
    pub rate: Option<Rate>,
    /// E440: what the explorer shows for the project.
    pub title: Option<String>,
    pub about: Option<String>,
    pub links: Vec<Link>,
    /// E416: the project's folder shares answer ranked and vector search.
    pub share_rank: bool,
    /// E416: vectors for the project's files (`--file-vectors PREFIX`).
    pub file_vectors: Option<String>,
}

pub struct Config {
    pub port: u16,
    pub projects: Vec<ProjectConfig>,
    /// E362: heavy jobs at once across the host, and how long one waits.
    pub heavy: Option<(usize, Duration)>,
    /// E440: host names that answer with the explorer.
    pub explorers: Vec<String>,
}

fn valid_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 63
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

impl Config {
    /// Lines: `port N`, `project NAME STORE_DIR [REF]`, `host NAME HOSTNAME`,
    /// `user NAME ROLE USER:SALT:HASH` (from `ikam passwd`), `max-store NAME
    /// BYTES`, `max-body NAME BYTES`, `token NAME ROLE ID:HASH [rate=N/s]
    /// [expires=SECS]` and `rate NAME N/s` (E363); `explorer HOSTNAME`,
    /// `title NAME TEXT`, `about NAME TEXT` and `link NAME GROUP URL LABEL`
    /// (E440); `#` starts a comment.
    pub fn parse(text: &str) -> Result<Config, String> {
        let mut port = 8750;
        let mut projects: Vec<ProjectConfig> = Vec::new();
        let mut explorers: Vec<String> = Vec::new();
        let (mut heavy_jobs, mut heavy_wait) = (None, Duration::from_secs(60));
        for (n, line) in text.lines().enumerate() {
            let line = line.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            let bad = |what: &str| format!("config line {}: {what}", n + 1);
            let words: Vec<&str> = line.split_whitespace().collect();
            let project = |projects: &mut Vec<ProjectConfig>, name: &str| {
                let i = projects.iter().position(|p| p.name == name);
                i.ok_or_else(|| bad(&format!("no project {name} declared above")))
            };
            match words.as_slice() {
                ["port", p] => port = p.parse().map_err(|_| bad("port N"))?,
                ["heavy-jobs", n] => match n.parse() {
                    Ok(n) if n > 0 => heavy_jobs = Some(n),
                    _ => return Err(bad("heavy-jobs N, at least 1")),
                },
                ["heavy-wait", secs] => match secs.parse() {
                    Ok(s) => heavy_wait = Duration::from_secs(s),
                    _ => return Err(bad("heavy-wait SECS")),
                },
                ["project", name, store, rest @ ..] if rest.len() <= 1 => {
                    if !valid_name(name) {
                        return Err(bad("a project name is a-z, 0-9 and -"));
                    }
                    if projects.iter().any(|p| p.name == *name) {
                        return Err(bad(&format!("project {name} declared twice")));
                    }
                    let refname = rest.first().unwrap_or(&"main").to_string();
                    if reserved_ref(&refname) {
                        return Err(bad(&format!(
                            "ref {refname} is kept for shares, imports and proposals"
                        )));
                    }
                    projects.push(ProjectConfig {
                        name: name.to_string(),
                        store: PathBuf::from(store),
                        refname,
                        hosts: Vec::new(),
                        users: Vec::new(),
                        max_store: None,
                        max_body: None,
                        tokens: Vec::new(),
                        rate: None,
                        title: None,
                        about: None,
                        links: Vec::new(),
                        share_rank: false,
                        file_vectors: None,
                    });
                }
                ["host", name, host] => {
                    let i = project(&mut projects, name)?;
                    projects[i].hosts.push(
                        host_name(host)
                            .ok_or_else(|| bad("host NAME HOSTNAME, a name without a port"))?,
                    );
                }
                ["explorer", host] => explorers.push(
                    host_name(host)
                        .ok_or_else(|| bad("explorer HOSTNAME, a name without a port"))?,
                ),
                ["title" | "about", name, text @ ..] if !text.is_empty() => {
                    let i = project(&mut projects, name)?;
                    let text = Some(text.join(" "));
                    if words[0] == "title" {
                        projects[i].title = text;
                    } else {
                        projects[i].about = text;
                    }
                }
                ["link", name, group, url, label @ ..] if !label.is_empty() => {
                    let i = project(&mut projects, name)?;
                    if !valid_name(group) {
                        return Err(bad("a link group is a-z, 0-9 and -"));
                    }
                    if !link_url(url) {
                        return Err(bad(
                            "a link is an http(s) URL, a /path, or a path inside the project",
                        ));
                    }
                    projects[i].links.push(Link {
                        group: group.to_string(),
                        url: url.to_string(),
                        label: label.join(" "),
                    });
                }
                ["user", name, role, line] => {
                    let i = project(&mut projects, name)?;
                    let role =
                        parse_role(role).ok_or_else(|| bad("a role is read, write or owner"))?;
                    Auth::parse(line).map_err(|e| bad(&e))?;
                    projects[i].users.push((role, line.to_string()));
                }
                ["token", name, role, line, rest @ ..] => {
                    let i = project(&mut projects, name)?;
                    let role =
                        parse_role(role).ok_or_else(|| bad("a role is read, write or owner"))?;
                    let t = Token::parse(role, line, rest, std::time::Instant::now())
                        .map_err(|e| bad(&e))?;
                    if projects.iter().any(|p| {
                        p.tokens
                            .iter()
                            .any(|(_, l, _)| l.starts_with(&format!("{}:", t.id)))
                    }) {
                        return Err(bad("this token is in the config already"));
                    }
                    let rest = rest.iter().map(|w| w.to_string()).collect();
                    projects[i].tokens.push((role, line.to_string(), rest));
                }
                ["share-rank", name] => {
                    let i = project(&mut projects, name)?;
                    projects[i].share_rank = true;
                }
                ["file-vectors", name, prefix] => {
                    let i = project(&mut projects, name)?;
                    projects[i].file_vectors = Some(prefix.to_string());
                }
                ["rate", name, rate] => {
                    let i = project(&mut projects, name)?;
                    projects[i].rate = Some(Rate::parse(rate).map_err(|e| bad(&e))?);
                }
                ["max-store", name, bytes] | ["max-body", name, bytes] => {
                    let i = project(&mut projects, name)?;
                    let b = crate::parse_bytes(bytes).ok_or_else(|| bad("BYTES (such as 25M)"))?;
                    if words[0] == "max-store" {
                        projects[i].max_store = Some(b);
                    } else {
                        projects[i].max_body = Some(b as usize);
                    }
                }
                ["public", ..] => return Err(bad("host mode has no public projects")),
                _ => return Err(bad(&format!("not understood: {line}"))),
            }
        }
        if projects.is_empty() {
            return Err("the config declares no project".into());
        }
        let mut seen: BTreeMap<String, &str> = BTreeMap::new();
        for p in &projects {
            if !p.users.iter().any(|(r, _)| *r == Role::Owner) {
                return Err(format!("project {} has no owner", p.name));
            }
            let implicit = format!("{}.localhost", p.name);
            for h in p.hosts.iter().chain([&implicit]) {
                if let Some(other) = seen.insert(h.clone(), &p.name)
                    && other != p.name
                {
                    return Err(format!("host {h} is named by {other} and {}", p.name));
                }
            }
        }
        if let Some(h) = explorers.iter().find(|h| seen.contains_key(*h)) {
            return Err(format!("host {h} is named by {} and an explorer", seen[h]));
        }
        Ok(Config {
            port,
            projects,
            heavy: heavy_jobs.map(|n| (n, heavy_wait)),
            explorers,
        })
    }
}

/// A host name as the config gives it, lower case, without a port.
fn host_name(s: &str) -> Option<String> {
    let h = s.to_ascii_lowercase();
    (!h.is_empty() && !h.contains(['/', ':', '@'])).then_some(h)
}

/// E440: an explorer link: `http://` or `https://`, a path on this host
/// (`/...`, not `//`), or a path inside the project (`file?path=a.md`). Any
/// other scheme (`javascript:`) is refused.
fn link_url(u: &str) -> bool {
    let lower = u.to_ascii_lowercase();
    if lower.starts_with("https://") || lower.starts_with("http://") {
        return u.len() > lower.find("//").unwrap() + 2;
    }
    let head = u.split(['/', '?', '#']).next().unwrap_or("");
    !u.is_empty() && !u.starts_with("//") && !head.contains(':') && !u.contains('\\')
}

fn parse_role(s: &str) -> Option<Role> {
    match s {
        "read" => Some(Role::Read),
        "write" => Some(Role::Write),
        "owner" => Some(Role::Owner),
        _ => None,
    }
}

/// Refs that shares write in each project's store: the share log, imports
/// (`import/SOURCE/SHARE`) and proposals (`proposals/SHARE`). A project's own
/// data ref may not be one of them, or a share would move it.
fn reserved_ref(name: &str) -> bool {
    name == SHARES_REF || name.starts_with("import/") || name.starts_with("proposals/")
}

pub struct Project {
    pub name: String,
    pub live: Arc<Mutex<Live>>,
    users: Vec<(Role, Auth)>,
    hosts: Vec<String>,
    max_body: Option<usize>,
    key: [u8; 32],
    tokens: Vec<Token>,
    bucket: Option<Mutex<Bucket>>,
    pub title: Option<String>,
    pub about: Option<String>,
    pub links: Vec<Link>,
    /// E416 (`share-rank`): the BM25 index of each recently ranked share's
    /// scope, by the share's encoding (which names its commit and scope),
    /// newest last. `None`: shares do not rank.
    ranked: Option<Ranked>,
}

/// E416: scope indexes by share encoding, newest last.
type Ranked = Mutex<VecDeque<(Vec<u8>, Arc<Bm25Mem>)>>;

/// E416: scope indexes a project keeps for ranking through its shares.
const RANKED_SCOPES: usize = 8;

pub struct Host {
    pub projects: BTreeMap<String, Project>,
    port: u16,
    heavy: Option<Heavy>,
    explorers: Vec<String>,
}

/// E362: at most `max` heavy jobs at once across the host's projects; the
/// rest wait in the order they came, each for up to `wait`.
pub struct Heavy {
    max: usize,
    wait: Duration,
    state: Mutex<HeavyState>,
    turn: Condvar,
}

#[derive(Default)]
struct HeavyState {
    running: usize,
    waiting: VecDeque<u64>,
    next: u64,
}

/// A running heavy job; its place is given up when dropped.
pub struct Permit<'a>(&'a Heavy);

impl Drop for Permit<'_> {
    fn drop(&mut self) {
        self.0.state.lock().unwrap().running -= 1;
        self.0.turn.notify_all();
    }
}

impl Heavy {
    pub fn new(max: usize, wait: Duration) -> Heavy {
        Heavy {
            max,
            wait,
            state: Mutex::new(HeavyState::default()),
            turn: Condvar::new(),
        }
    }

    /// Wait for a turn; none if `wait` passed first.
    pub fn enter(&self) -> Option<Permit<'_>> {
        let mut s = self.state.lock().unwrap();
        let me = s.next;
        s.next += 1;
        s.waiting.push_back(me);
        let deadline = std::time::Instant::now() + self.wait;
        loop {
            if s.waiting.front() == Some(&me) && s.running < self.max {
                s.waiting.pop_front();
                s.running += 1;
                self.turn.notify_all();
                return Some(Permit(self));
            }
            let now = std::time::Instant::now();
            if now >= deadline {
                s.waiting.retain(|t| *t != me);
                self.turn.notify_all();
                return None;
            }
            s = self.turn.wait_timeout(s, deadline - now).unwrap().0;
        }
    }

    /// Jobs running and waiting.
    #[cfg(test)]
    pub fn load(&self) -> (usize, usize) {
        let s = self.state.lock().unwrap();
        (s.running, s.waiting.len())
    }
}

/// The project's share key, kept in `STORE/share.key`, made on first use.
fn share_key(store: &std::path::Path) -> Result<[u8; 32], String> {
    let path = store.join("share.key");
    if let Ok(b) = std::fs::read(&path) {
        return b
            .try_into()
            .map_err(|_| format!("{}: not a 32-byte key", path.display()));
    }
    let mut key = [0u8; 32];
    use std::io::{Read, Write};
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut key))
        .map_err(|e| format!("random key: {e}"))?;
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
    opts.open(&path)
        .and_then(|mut f| f.write_all(&key).and_then(|_| f.sync_all()))
        .map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(key)
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// A host name without its port, lower case.
fn bare_host(host: &str) -> String {
    let host = host.to_ascii_lowercase();
    match host.rsplit_once(':') {
        Some((n, p)) if p.bytes().all(|b| b.is_ascii_digit()) => n.to_string(),
        _ => host,
    }
}

/// The caller's role in a project, from a token or the best user whose
/// password matches, and the token if one let them in.
fn role<'a>(p: &'a Project, req: &Request) -> (Option<Role>, Option<&'a Token>) {
    let token = crate::tokens::bearer(&p.tokens, req.header("authorization"), now());
    let role = match token {
        Some(t) => Some(t.role),
        None => p
            .users
            .iter()
            .filter(|(_, a)| a.check(req).is_none())
            .map(|(r, _)| *r)
            .max(),
    };
    (role, token)
}

fn password_required() -> Response {
    Response::text(401, "password required").with(
        "WWW-Authenticate",
        "Basic realm=\"ikam\", charset=\"UTF-8\"",
    )
}

/// E440: a MOVE or COPY Destination under the explorer (`/p/NAME/dav/..`,
/// or a URL with that path) as the project sees it (`/dav/..`). One outside
/// the project is left as it is, and WebDAV refuses it.
fn local_destination(d: &str, base: &str) -> String {
    let path = match d.find("://") {
        Some(i) => {
            let after = &d[i + 3..];
            &after[after.find('/').unwrap_or(after.len())..]
        }
        None => d,
    };
    match path.strip_prefix(base) {
        Some(rest) if rest.starts_with('/') => rest.to_string(),
        _ => d.to_string(),
    }
}

/// E440: a page of the project served under the explorer resolves its
/// links from the project's path, not from `/`.
fn set_base(r: &mut Response, base: &str) {
    const ROOT: &[u8] = b"<base href=\"/\">";
    let html = r
        .headers
        .iter()
        .any(|(k, v)| k.eq_ignore_ascii_case("content-type") && v.starts_with("text/html"));
    let head = &r.body[..r.body.len().min(256)];
    if let Some(i) = html
        .then(|| head.windows(ROOT.len()).position(|w| w == ROOT))
        .flatten()
    {
        let tag = format!("<base href=\"{}\">", http::esc(base));
        r.body.splice(i..i + ROOT.len(), tag.into_bytes());
    }
}

fn ok_json(j: Json) -> Response {
    Response::new(200).body("application/json", j.encode())
}

fn no_share() -> Response {
    Response::text(404, "no such share")
}

impl Host {
    pub fn open(config: Config) -> Result<Host, String> {
        let mut projects = BTreeMap::new();
        let mut stores: BTreeMap<PathBuf, String> = BTreeMap::new();
        for p in config.projects {
            let dir = p.store.to_str().ok_or("a store path must be UTF-8")?;
            let mut l = Live::open(dir, &p.refname)?;
            let real = std::fs::canonicalize(&p.store).map_err(|e| e.to_string())?;
            if let Some(other) = stores.insert(real, p.name.clone()) {
                return Err(format!(
                    "projects {other} and {} name the same store; each project needs its own",
                    p.name
                ));
            }
            l.max_store = p.max_store;
            l.index_all(false)?;
            if let Some(prefix) = &p.file_vectors {
                l.load_file_vectors(prefix)?;
            }
            let key = share_key(&p.store)?;
            let users = p
                .users
                .iter()
                .map(|(r, line)| Auth::parse(line).map(|a| (*r, a)))
                .collect::<Result<_, _>>()?;
            let start = std::time::Instant::now();
            let tokens = p
                .tokens
                .iter()
                .map(|(r, line, words)| {
                    let words: Vec<&str> = words.iter().map(String::as_str).collect();
                    Token::parse(*r, line, &words, start)
                })
                .collect::<Result<_, _>>()?;
            let mut hosts = p.hosts.clone();
            hosts.push(format!("{}.localhost", p.name));
            projects.insert(
                p.name.clone(),
                Project {
                    name: p.name,
                    live: Arc::new(Mutex::new(l)),
                    users,
                    hosts,
                    max_body: p.max_body,
                    key,
                    tokens,
                    bucket: p.rate.map(|r| Mutex::new(Bucket::new(r, start))),
                    title: p.title,
                    about: p.about,
                    links: p.links,
                    ranked: p.share_rank.then(|| Mutex::new(VecDeque::new())),
                },
            );
        }
        Ok(Host {
            projects,
            port: config.port,
            heavy: config.heavy.map(|(n, wait)| Heavy::new(n, wait)),
            explorers: config.explorers,
        })
    }

    /// The project a Host header names.
    fn project(&self, host: Option<&str>) -> Option<&Project> {
        let name = bare_host(host?);
        self.projects.values().find(|p| p.hosts.contains(&name))
    }

    /// E440: the project an explorer path (`/p/NAME/...`) names, with the
    /// rest of the path.
    fn explorer_project<'a>(&self, host: &str, path: &'a str) -> Option<(&Project, &'a str)> {
        if !self.explorers.contains(&bare_host(host)) {
            return None;
        }
        let rest = path.strip_prefix("/p/")?;
        let (name, tail) = rest.split_once('/').unwrap_or((rest, ""));
        Some((self.projects.get(name)?, tail))
    }

    /// The body limit for a request: its project's, by host name or by
    /// explorer path; none at all for a host that names no project.
    fn body_limit(&self, host: &str, path: &str) -> Option<usize> {
        match self.explorer_project(host, path) {
            Some((p, _)) => p.max_body,
            None => self.project(Some(host)).map_or(Some(0), |p| p.max_body),
        }
    }

    pub fn handle(&self, req: Request) -> Response {
        if self
            .explorers
            .contains(&bare_host(req.header("host").unwrap_or("")))
        {
            return self.explore(req);
        }
        let Some(p) = self.project(req.header("host")) else {
            return Response::text(404, "no such project");
        };
        self.serve(p, req)
    }

    /// E440: the explorer: `/` lists the projects the caller's user or token
    /// opens, `/projects/NAME` is one project's page with its links, and
    /// `/p/NAME/...` is the project itself, as its own host name serves it.
    /// A project the caller cannot open, or that does not exist, asks for a
    /// password, as a wrong password does.
    fn explore(&self, mut req: Request) -> Response {
        if req.path == "/robots.txt" {
            return Response::text(200, "User-agent: *\nDisallow: /\n");
        }
        if req.path.starts_with("/p/") {
            let host = req.header("host").unwrap_or("");
            let Some((p, tail)) = self.explorer_project(host, &req.path) else {
                return password_required();
            };
            let base = format!("/p/{}", p.name);
            let dav = tail == "dav" || tail.starts_with("dav/");
            req.path = format!("/{tail}");
            for (k, v) in req.headers.iter_mut() {
                if k.eq_ignore_ascii_case("destination") {
                    *v = local_destination(v, &base);
                }
            }
            let mut r = self.serve(p, req);
            if !dav {
                set_base(&mut r, &format!("{base}/"));
            }
            return r;
        }
        let open: Vec<(&Project, Role)> = self
            .projects
            .values()
            .filter_map(|p| Some((p, role(p, &req).0?)))
            .collect();
        if open.is_empty() {
            return password_required();
        }
        if !matches!(req.method.as_str(), "GET" | "HEAD") {
            return Response::text(405, "the explorer only reads");
        }
        match req.path.as_str() {
            "/" => explorer::index(&open),
            path => match path.strip_prefix("/projects/") {
                Some(name) => match open.iter().find(|(p, _)| p.name == name) {
                    Some((p, r)) => explorer::project(p, *r),
                    None => Response::text(404, "no such project"),
                },
                None => Response::text(404, "no such page"),
            },
        }
    }

    /// One request to one project: its user or token, rates, body limit,
    /// then the route, with a panic answered 500.
    fn serve(&self, p: &Project, req: Request) -> Response {
        if req.path == "/robots.txt" {
            return Response::text(200, "User-agent: *\nDisallow: /\n");
        }
        let (role, token) = role(p, &req);
        let Some(role) = role else {
            return password_required();
        };
        // E363: the project's rate, then the token's.
        let at = std::time::Instant::now();
        let buckets = [p.bucket.as_ref(), token.and_then(|t| t.bucket.as_ref())];
        for b in buckets.into_iter().flatten() {
            if let Err(wait) = b.lock().unwrap().take(at) {
                let secs = wait.as_secs_f64().ceil().max(1.0) as u64;
                return Response::text(429, "too many requests for this project")
                    .with("Retry-After", secs.to_string());
            }
        }
        if p.max_body.is_some_and(|m| req.body.len() > m) {
            return Response::text(413, "body too large for this project");
        }
        match catch_unwind(AssertUnwindSafe(|| self.route(p, role, &req))) {
            Ok(r) => r,
            Err(_) => {
                say!("{} {}: request failed", live::stamp(), p.name);
                Response::text(500, "this project failed to answer")
            }
        }
    }

    fn route(&self, p: &Project, role: Role, req: &Request) -> Response {
        let post = req.method == "POST";
        if post && req.header("x-ikam").is_none() {
            return Response::text(403, "missing X-Ikam header");
        }
        if let Some(rest) = req.path.strip_prefix("/shares") {
            if role < Role::Owner {
                return Response::text(403, "the project's owner manages its shares");
            }
            return match (req.method.as_str(), rest) {
                ("GET", "" | "/") => self.list(p),
                ("POST", "/issue") => self.issue(p, req),
                ("POST", "/revoke") => {
                    let name = req.param("name").unwrap_or_default();
                    match p.live.lock().unwrap().repo.share_revoke(&name) {
                        Ok(true) => Response::text(200, format!("revoked {name}")),
                        Ok(false) => no_share(),
                        Err(e) => Response::text(500, e.to_string()),
                    }
                }
                ("POST", "/accept") => self.accept(p, req),
                _ => Response::text(404, "no such page"),
            };
        }
        if let Some(rest) = req.path.strip_prefix("/from/") {
            return self.from(p, role, req, rest);
        }
        let admin = post && matches!(req.path.as_str(), "/api/repack" | "/api/gc" | "/api/fsck");
        if admin && role < Role::Owner {
            return Response::text(403, "the project's owner runs maintenance");
        }
        // E362: a heavy job waits for its turn across the host.
        let _turn = match (&self.heavy, admin) {
            (Some(h), true) => match h.enter() {
                Some(p) => Some(p),
                None => {
                    let secs = h.wait.as_secs().max(1).to_string();
                    return Response::text(503, "the host is busy with maintenance; try again")
                        .with("Retry-After", &secs);
                }
            },
            _ => None,
        };
        if crate::writes(req) && role < Role::Write {
            return Response::text(403, "this user may only read");
        }
        // E360: promoting a draft writes the project's main ref, as a merge does.
        if req.path == "/api/tool/draft_promote" && role < Role::Owner {
            return Response::text(403, "the project's owner promotes drafts");
        }
        if req.path == dav::PREFIX || req.path.starts_with("/dav/") {
            dav::handle(&mut p.live.lock().unwrap(), req)
        } else {
            ui::handle(&p.live, req)
        }
    }

    fn list(&self, p: &Project) -> Response {
        let mut l = p.live.lock().unwrap();
        let rows = match l.repo.shares() {
            Ok(r) => r,
            Err(e) => return Response::text(500, e.to_string()),
        };
        let shares = rows
            .iter()
            .filter_map(|(_, b)| Share::decode(b))
            .map(|s| {
                Json::Obj(vec![
                    ("name".into(), Json::Str(s.name)),
                    ("to".into(), Json::Str(s.to)),
                    ("commit".into(), Json::Str(s.commit.to_hex())),
                    (
                        "right".into(),
                        Json::Str(format!("{:?}", s.right).to_lowercase()),
                    ),
                    ("scope".into(), Json::Str(format!("{:?}", s.scope))),
                    ("expires".into(), Json::Num(s.expires as f64)),
                ])
            })
            .collect();
        ok_json(Json::Arr(shares))
    }

    fn issue(&self, p: &Project, req: &Request) -> Response {
        let r = (|| -> Result<Response, String> {
            let param = |k: &str| req.param(k).ok_or(format!("missing {k}="));
            let name = param("name")?;
            if !valid_name(&name) {
                return Err("a share name is a-z, 0-9 and -".into());
            }
            let to = param("to")?;
            if to == p.name || !self.projects.contains_key(&to) {
                return Err(format!("no other project {to}"));
            }
            let right = match param("right")?.as_str() {
                "read" => Right::Read,
                "propose" => Right::Propose,
                "merge" => Right::Merge,
                _ => return Err("right is read, propose or merge".into()),
            };
            let secs: u64 = param("expires")?
                .parse()
                .map_err(|_| "expires=SECONDS from now")?;
            let mut l = p.live.lock().unwrap();
            l.flush(true)?;
            let commit = l.head().ok_or("the project has no commit yet")?;
            let scope = parse_scope(&mut l, &param("scope")?)?;
            let share = Share {
                name,
                from: p.name.clone(),
                to,
                commit,
                scope,
                right,
                expires: now().saturating_add(secs),
            };
            let tag = l
                .repo
                .share_issue(&share, &p.key)
                .map_err(|e| e.to_string())?;
            l.log(format!("shared {} with {}", share.name, share.to));
            Ok(ok_json(Json::Obj(vec![
                ("name".into(), Json::Str(share.name)),
                ("commit".into(), Json::Str(commit.to_hex())),
                ("tag".into(), Json::Str(Id::from_bytes(tag).to_hex())),
            ])))
        })();
        r.unwrap_or_else(|e| Response::text(400, e))
    }

    /// Merge the proposal branch of share `name` into the source project.
    fn accept(&self, p: &Project, req: &Request) -> Response {
        let name = req.param("name").unwrap_or_default();
        let mut l = p.live.lock().unwrap();
        if let Err(e) = l.flush(true) {
            return Response::text(500, e);
        }
        let branch = format!("proposals/{name}");
        if l.repo.resolve(&branch).is_err() {
            return Response::text(404, "no such proposal");
        }
        let into = l.refname.clone();
        let msg = format!("accept proposal {name}");
        let out = l.repo.merge_with(&into, &branch, &msg, merge_options(req));
        let _ = l.sync();
        merged(out.map_err(|e| e.to_string()))
    }

    /// `/from/SOURCE/SHARE/ACTION` on the receiving project `p`.
    fn from(&self, p: &Project, role: Role, req: &Request, rest: &str) -> Response {
        let mut parts = rest.splitn(3, '/');
        let (Some(src), Some(name), Some(action)) = (parts.next(), parts.next(), parts.next())
        else {
            return no_share();
        };
        let Some(s) = self.projects.get(src).filter(|s| s.name != p.name) else {
            return no_share();
        };
        let need = match (req.method.as_str(), action) {
            ("POST", "propose") => (Role::Write, Right::Propose),
            ("POST", "merge") => (Role::Owner, Right::Merge),
            ("GET" | "HEAD", _) => (Role::Read, Right::Read),
            _ => return Response::text(405, "method not allowed"),
        };
        if role < need.0 {
            return Response::text(403, "this user may not do that through a share");
        }
        if need.1 == Right::Merge {
            return self.merge(s, p, name, req);
        }
        let mut sl = s.live.lock().unwrap();
        let share = match checked(&mut sl, s, name, &p.name, need.1) {
            Some(share) => share,
            None => return no_share(),
        };
        let refused = |e: Refused| match e {
            Refused::Store(e) => Response::text(500, e),
            _ => no_share(),
        };
        if action == "rank" {
            return match &s.ranked {
                Some(cache) => rank(&mut sl, cache, &share, req).unwrap_or_else(refused),
                None => no_share(),
            };
        }
        let repo = &mut sl.repo;
        let r = match action {
            "files" => repo.share_files(&share).map(|f| {
                ok_json(Json::Arr(
                    f.into_iter().map(|(p, _)| Json::Str(p)).collect(),
                ))
            }),
            "read" => {
                let path = live::norm(&req.param("path").unwrap_or_default());
                let range = match req.param("range").map(|r| parse_range(&r)) {
                    Some(Some(r)) => Some(r),
                    Some(None) => return Response::text(400, "range=START-END"),
                    None => None,
                };
                repo.share_read(&share, &path, range)
                    .map(|b| Response::new(200).body("application/octet-stream", b))
            }
            "search" => {
                let q = req.param("q").unwrap_or_default();
                repo.share_search(&share, q.as_bytes())
                    .map(|h| ok_json(Json::Arr(h.into_iter().map(Json::Str).collect())))
            }
            "records" => {
                let start = req.param("start").unwrap_or_default();
                let end = req.param("end").filter(|e| !e.is_empty());
                repo.share_records(&share, &start, end.as_deref())
                    .map(|rows| {
                        ok_json(Json::Arr(
                            rows.into_iter()
                                .map(|(k, v)| {
                                    Json::Arr(vec![
                                        Json::Str(k),
                                        Json::Str(String::from_utf8_lossy(&v).into_owned()),
                                    ])
                                })
                                .collect(),
                        ))
                    })
            }
            "fragment" => match req.param("id").and_then(|i| i.parse::<Id>().ok()) {
                Some(id) => repo
                    .share_fragment(&share, &id)
                    .map(|b| Response::new(200).body("application/octet-stream", b)),
                None => return Response::text(400, "id=HEX"),
            },
            "nodes" => {
                let prefix = req.param("prefix").unwrap_or_default();
                repo.share_nodes(&share, &prefix).map(|n| {
                    ok_json(Json::Arr(
                        n.into_iter().map(|n| Json::Str(n.label)).collect(),
                    ))
                })
            }
            "propose" => {
                let path = live::norm(&req.param("path").unwrap_or_default());
                let msg = format!("proposal from {} through {name}", p.name);
                repo.share_propose(&share, &[(path, req.body.clone())], &msg)
                    .map(|_| Response::text(200, format!("proposed on proposals/{name}")))
            }
            _ => return no_share(),
        };
        r.unwrap_or_else(refused)
    }

    /// Merge share `name` of `s` into `p`, holding both projects; they are
    /// locked in name order so two merges cannot wait on each other.
    fn merge(&self, s: &Project, p: &Project, name: &str, req: &Request) -> Response {
        let (mut sl, mut pl) = if s.name < p.name {
            let a = s.live.lock().unwrap();
            (a, p.live.lock().unwrap())
        } else {
            let b = p.live.lock().unwrap();
            (s.live.lock().unwrap(), b)
        };
        let Some(share) = checked(&mut sl, s, name, &p.name, Right::Merge) else {
            return no_share();
        };
        if let Err(e) = pl.flush(true) {
            return Response::text(500, e);
        }
        let into = req.param("into").unwrap_or_else(|| pl.refname.clone());
        if reserved_ref(&into) {
            return Response::text(400, "that ref is kept for shares");
        }
        match sl.repo.share_import_bytes(&share) {
            Ok(n) if !pl.has_room(n) => {
                return Response::text(507, "this project's store is full");
            }
            Ok(_) => {}
            Err(Refused::NotMergeable) => {
                return Response::text(409, "this kind of scope can be read but not merged");
            }
            Err(Refused::Store(e)) => return Response::text(500, e),
            Err(_) => return no_share(),
        }
        let msg = format!("merge {name} from {}", s.name);
        let opts = merge_options(req);
        let out = import_share_with(&mut sl.repo, &share, &mut pl.repo, &into, &msg, opts);
        let _ = pl.sync();
        match out {
            Ok(m) => merged(Ok(m)),
            Err(Refused::NotMergeable) => {
                Response::text(409, "this kind of scope can be read but not merged")
            }
            Err(Refused::Store(e)) => Response::text(500, e),
            Err(_) => no_share(),
        }
    }
}

/// E416: `GET /from/SOURCE/SHARE/rank` on a folder share, after its check:
/// the scope's files at the share's commit ranked for `q` (and `vector`),
/// with BM25 over those files alone. Any other scope is refused (404).
fn rank(l: &mut Live, cache: &Ranked, share: &Share, req: &Request) -> Result<Response, Refused> {
    if !matches!(share.scope, Scope::Path(_)) {
        return Err(Refused::Outside);
    }
    let q = req.param("q").unwrap_or_default();
    let k = match req.param("k").map(|k| k.parse::<usize>()) {
        None => 10,
        Some(Ok(k)) => k.clamp(1, 1000),
        Some(Err(_)) => return Ok(Response::text(400, "k=N")),
    };
    let vector: Option<Vec<f32>> = match req.param("vector").filter(|v| !v.is_empty()) {
        None => None,
        Some(v) => match v.split(',').map(|x| x.trim().parse::<f32>()).collect() {
            Ok(v) => Some(v),
            Err(_) => return Ok(Response::text(400, "vector=X,Y,.. (numbers)")),
        },
    };
    let fuse = match (req.param("fuse").as_deref(), req.param("alpha")) {
        (Some("rrf"), _) => live::Fuse::Rrf,
        (Some("convex") | None, None) => live::Fuse::Convex(ikam_kernel::fusion::DEFAULT_ALPHA),
        (Some("convex") | None, Some(a)) => match a.parse::<f32>() {
            Ok(a) if (0.0..=1.0).contains(&a) => live::Fuse::Convex(a),
            _ => return Ok(Response::text(400, "alpha is a number from 0 to 1")),
        },
        _ => return Ok(Response::text(400, "fuse is convex or rrf")),
    };
    let files: BTreeMap<String, Id> = l.repo.share_files(share)?.into_iter().collect();
    let key = share.encode();
    let found = {
        let c = cache.lock().unwrap();
        c.iter().find(|(k, _)| *k == key).map(|(_, i)| i.clone())
    };
    let index = match found {
        Some(i) => i,
        None => {
            let i = Arc::new(l.scope_index(&files).map_err(Refused::Store)?);
            let mut c = cache.lock().unwrap();
            if c.len() >= RANKED_SCOPES {
                c.pop_front();
            }
            c.push_back((key, i.clone()));
            i
        }
    };
    let empty = std::collections::HashMap::new();
    let (vectors, dim) = match &l.ranker {
        Some(r) => (&r.vectors, r.dim),
        None => (&empty, 0),
    };
    let hybrid = vector.is_some() && dim > 0;
    match live::rank_in(&index, &files, vectors, dim, &q, vector.as_deref(), k, fuse) {
        Ok(hits) => Ok(ok_json(Json::Obj(vec![
            ("hybrid".into(), Json::Bool(hybrid)),
            (
                "hits".into(),
                Json::Arr(
                    hits.into_iter()
                        .map(|(p, s)| Json::Arr(vec![Json::Str(p), Json::Num(s as f64)]))
                        .collect(),
                ),
            ),
        ]))),
        Err(e) => Ok(Response::text(400, e)),
    }
}

/// The logged share `name` of `s`, if it lets project `to` use `right` now.
fn checked(l: &mut Live, s: &Project, name: &str, to: &str, right: Right) -> Option<Share> {
    let share = l.repo.share_named(name).ok()??;
    if share.from != s.name {
        return None;
    }
    let tag = share.seal(&s.key);
    l.repo
        .share_check(&share, &tag, &s.key, to, right, now())
        .ok()?;
    Some(share)
}

fn merged(out: Result<Merged, String>) -> Response {
    match out {
        Ok(Merged::UpToDate) => Response::text(200, "up to date"),
        Ok(Merged::FastForward(c)) | Ok(Merged::Commit(c)) => {
            Response::text(200, format!("merged as {}", c.to_hex()))
        }
        Ok(Merged::Conflicts(c)) => Response::text(
            409,
            c.iter()
                .map(|c| format!("conflict: {c:?}\n"))
                .collect::<String>(),
        ),
        Err(e) => Response::text(500, e),
    }
}

/// E417 (off): `lines=1` on a merge or an accept merges a file both sides
/// changed line by line; without it, any such file is a conflict.
fn merge_options(req: &Request) -> MergeOptions {
    MergeOptions {
        lines: req.param("lines").as_deref() == Some("1"),
    }
}

/// `A-B` as the byte range A..B.
fn parse_range(s: &str) -> Option<std::ops::Range<u64>> {
    let (a, b) = s.split_once('-')?;
    let (a, b) = (a.parse().ok()?, b.parse().ok()?);
    (a <= b).then_some(a..b)
}

fn parse_scope(l: &mut Live, s: &str) -> Result<Scope, String> {
    let (kind, rest) = s.split_once(':').ok_or("scope is KIND:VALUE")?;
    Ok(match kind {
        "path" => Scope::Path(live::norm(rest)),
        "section" => {
            let (path, ranges) = rest.split_once('@').ok_or("section:PATH@A-B,..")?;
            let path = live::norm(path);
            let id = match l.stat(&path)? {
                Some((Kind::File, id)) => id,
                _ => return Err(format!("no file {path}")),
            };
            let ranges = ranges
                .split(',')
                .map(parse_range)
                .collect::<Option<Vec<_>>>()
                .ok_or("section:PATH@A-B,..")?;
            Scope::Section { path, id, ranges }
        }
        "fragments" => Scope::Fragments(
            rest.split(',')
                .map(|i| i.parse::<Id>().map_err(|_| format!("not an id: {i}")))
                .collect::<Result<_, _>>()?,
        ),
        "records" => {
            let (start, end) = rest.split_once("..").ok_or("records:START..END")?;
            Scope::Records {
                start: start.into(),
                end: (!end.is_empty()).then(|| end.into()),
            }
        }
        "graph" => {
            let (path, prefix) = rest.split_once(':').ok_or("graph:PATH:PREFIX")?;
            Scope::Graph {
                path: live::norm(path),
                prefix: prefix.into(),
            }
        }
        _ => return Err("scope kinds: path, section, fragments, records, graph".into()),
    })
}

/// `ikam host --config FILE [--keep-on-hangup] [PORT]`: serve until stopped
/// (SIGTERM, SIGINT or SIGHUP; `--keep-on-hangup` ignores SIGHUP, E374).
pub fn host_cmd(args: &[String]) -> Result<(), String> {
    let mut args = args.to_vec();
    if args.iter().any(|a| a == "--public") {
        return Err("ikam host has no --public: every request needs a project's user".into());
    }
    let keep_on_hangup = crate::take_switch(&mut args, "--keep-on-hangup");
    let file = crate::take_flag(&mut args, "--config")
        .ok_or("usage: ikam host --config FILE [--keep-on-hangup] [PORT]")?;
    let text = std::fs::read_to_string(&file).map_err(|e| format!("{file}: {e}"))?;
    let mut config = Config::parse(&text)?;
    if let Some(p) = args.first() {
        config.port = p.parse().map_err(|_| "PORT must be a number")?;
    }
    let host = Arc::new(Host::open(config)?);
    auth::catch_stop_signals(keep_on_hangup);
    {
        let host = host.clone();
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(Duration::from_millis(250));
                let stopping = auth::stopping();
                let mut failed = false;
                for p in host.projects.values() {
                    let Ok(mut l) = p.live.lock() else { continue };
                    if let Err(e) = l.flush(stopping) {
                        say!("{} {}: commit failed: {e}", live::stamp(), p.name);
                        failed = true;
                    }
                }
                if stopping {
                    say!("{} stopped ({})", live::stamp(), auth::stop_signal());
                    std::process::exit(failed as i32);
                }
            }
        });
    }
    let t = live::stamp();
    for p in host.projects.values() {
        say!("{t} project {}: {}", p.name, p.hosts.join(", "));
    }
    if !host.explorers.is_empty() {
        say!("{t} explorer: {}", host.explorers.join(", "));
    }
    say!("{t} host on 127.0.0.1:{}", host.port);
    // Each project's body limit applies before its body is read: the
    // server asks for the limit of the project the Host header names. An
    // unknown host gets no body at all; its answer is 404 either way.
    let b = host.clone();
    let body_limit: &'static http::BodyLimit =
        Box::leak(Box::new(move |h: &str, path: &str| b.body_limit(h, path)));
    let limits = http::Limits {
        body_limit: Some(body_limit),
        ..http::Limits::default()
    };
    let port = host.port;
    let h = host.clone();
    http::serve(port, Arc::new(move |req| h.handle(req)), limits)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("ikam-host-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    fn user(name: &str, pw: &str) -> String {
        Auth::line(name, pw).unwrap()
    }

    fn b64(s: &str) -> String {
        const A: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let b = s.as_bytes();
        let mut out = String::new();
        for c in b.chunks(3) {
            let n = (c[0] as u32) << 16
                | (*c.get(1).unwrap_or(&0) as u32) << 8
                | *c.get(2).unwrap_or(&0) as u32;
            for i in 0..4 {
                if i <= c.len() {
                    out.push(A[(n >> (18 - 6 * i) & 63) as usize] as char);
                } else {
                    out.push('=');
                }
            }
        }
        out
    }

    fn req(
        method: &str,
        host: &str,
        who: Option<(&str, &str)>,
        target: &str,
        body: &[u8],
    ) -> Request {
        let (path, query) = target.split_once('?').unwrap_or((target, ""));
        let mut headers = vec![("Host".to_string(), format!("{host}:8750"))];
        if let Some((u, p)) = who {
            headers.push((
                "Authorization".into(),
                format!("Basic {}", b64(&format!("{u}:{p}"))),
            ));
        }
        if method == "POST" {
            headers.push(("X-Ikam".into(), "1".into()));
        }
        Request {
            method: method.into(),
            path: path.into(),
            query: query.into(),
            headers,
            body: body.to_vec(),
        }
    }

    /// Projects a and b, each with an owner, a writer and a reader, and c
    /// with an owner only; a holds public and private files.
    fn host(name: &str) -> (Host, Vec<PathBuf>) {
        host_with(name, "")
    }

    /// As `host`, with more config lines.
    fn host_with(name: &str, extra: &str) -> (Host, Vec<PathBuf>) {
        host_with(name, "")
    }

    /// `host` with more config lines.
    fn host_with(name: &str, extra: &str) -> (Host, Vec<PathBuf>) {
        let dirs: Vec<PathBuf> = ["a", "b", "c"]
            .iter()
            .map(|p| dir(&format!("{name}-{p}")))
            .collect();
        let mut text = String::from("# test\nport 8750\n");
        for (p, d) in ["a", "b", "c"].iter().zip(&dirs) {
            text += &format!("project {p} {}\n", d.display());
            text += &format!("host {p} {p}.example-host\n");
            text += &format!("user {p} owner {}\n", user(&format!("{p}-owner"), "pw"));
            if *p != "c" {
                text += &format!("user {p} write {}\n", user(&format!("{p}-writer"), "pw"));
                text += &format!("user {p} read {}\n", user(&format!("{p}-reader"), "pw"));
            }
        }
        text += "max-body c 1K\n";
        text += extra;
        let h = Host::open(Config::parse(&text).unwrap()).unwrap();
        {
            let mut l = h.projects["a"].live.lock().unwrap();
            l.apply(vec![
                live::Op::Put("pub/one.md".into(), b"alpha shared\n".to_vec()),
                live::Op::Put("pub/two.md".into(), b"beta\n".to_vec()),
                live::Op::Put("priv/secret.md".into(), b"alpha private\n".to_vec()),
            ])
            .unwrap();
            l.flush(true).unwrap();
        }
        (h, dirs)
    }

    fn done(dirs: Vec<PathBuf>) {
        for d in dirs {
            let _ = std::fs::remove_dir_all(d);
        }
    }

    const OA: Option<(&str, &str)> = Some(("a-owner", "pw"));
    const OB: Option<(&str, &str)> = Some(("b-owner", "pw"));

    #[test]
    fn config_refuses_what_would_join_projects() {
        let u = user("x", "pw");
        let bad = [
            "project a /tmp/x\n".to_string(),
            format!("project a /tmp/x\nuser a owner {u}\nproject a /tmp/y\nuser a owner {u}\n"),
            format!(
                "project a /tmp/x\nuser a owner {u}\nproject b /tmp/y\nuser b owner {u}\nhost a same\nhost b same\n"
            ),
            format!("project a /tmp/x\nuser a owner {u}\npublic a\n"),
            format!("project A /tmp/x\nuser A owner {u}\n"),
            format!("project a /tmp/x\nuser a admin {u}\n"),
            format!("user a owner {u}\n"),
            format!(
                "project a /tmp/x\nuser a owner {u}\nproject b /tmp/y\nuser b owner {u}\nhost b a.localhost\n"
            ),
            // A share would move these refs.
            format!("project a /tmp/x shares\nuser a owner {u}\n"),
            format!("project a /tmp/x import/b/pub\nuser a owner {u}\n"),
            format!("project a /tmp/x proposals/pub\nuser a owner {u}\n"),
        ];
        for t in bad {
            assert!(Config::parse(&t).is_err(), "{t}");
        }
        let d = dir("same");
        let t = format!(
            "project a {0}\nuser a owner {u}\nproject b {0}/.\nuser b owner {u}\n",
            d.display()
        );
        assert!(Host::open(Config::parse(&t).unwrap()).is_err());
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn explorer_lines_are_checked() {
        let base = format!("project a /x\nuser a owner {}\n", user("o", "pw"));
        let parse = |more: &str| Config::parse(&format!("{base}{more}"));
        let c = parse(
            "explorer Explorer.Example\ntitle a The  A project\nabout a What a is.\n\
             link a demo https://x.example/a Demo one\nlink a project file?path=a.md The file\n\
             link a demo /projects/a Its page\n",
        )
        .unwrap();
        assert_eq!(c.explorers, ["explorer.example"]);
        let p = &c.projects[0];
        assert_eq!(p.title.as_deref(), Some("The A project"));
        assert_eq!(p.about.as_deref(), Some("What a is."));
        assert_eq!(p.links.len(), 3);
        assert_eq!(p.links[1].label, "The file");
        for bad in [
            "explorer a.localhost",
            "host a a.example\nexplorer a.example",
            "explorer x.example:8443",
            "link a demo javascript:alert(1) x",
            "link a demo JAVASCRIPT:alert(1) x",
            "link a demo //elsewhere.example/ x",
            "link a demo https:// x",
            "link a Demo https://x.example/ x",
            "link a demo https://x.example/",
            "link b demo https://x.example/ x",
            "title a",
        ] {
            assert!(parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_explorer_shows_and_serves_only_the_projects_a_caller_opens() {
        let (h, dirs) = host_with(
            "explorer",
            "explorer explorer.example\ntitle a Alpha <lab>\nabout a Notes & files.\n\
             link a project https://a.example/run The running one\n\
             link a demo file?path=pub/one.md The first note\n\
             link b demo https://b.example/ Beta demo\n",
        );
        let ex = "explorer.example";
        let ar = Some(("a-reader", "pw"));
        let aw = Some(("a-writer", "pw"));
        let text = |r: Response| (r.status, String::from_utf8_lossy(&r.body).into_owned());
        // No password, a wrong one, or a project it does not open: 401.
        let r = h.handle(req("GET", ex, None, "/", b""));
        assert_eq!(r.status, 401);
        assert!(r.headers.iter().any(|(k, _)| k == "WWW-Authenticate"));
        assert_eq!(
            h.handle(req("GET", ex, Some(("a-reader", "no")), "/", b""))
                .status,
            401
        );
        assert_eq!(h.handle(req("GET", ex, ar, "/p/b/", b"")).status, 401);
        assert_eq!(h.handle(req("GET", ex, ar, "/p/nope/", b"")).status, 401);
        // The list holds a only, escaped, with its links.
        let (st, list) = text(h.handle(req("GET", ex, ar, "/", b"")));
        assert_eq!(st, 200);
        assert!(list.contains("Alpha &lt;lab&gt;") && list.contains("Notes &amp; files."));
        assert!(
            list.contains("href=\"/p/a/file?path=pub/one.md\""),
            "{list}"
        );
        assert!(list.contains("href=\"https://a.example/run\""));
        assert!(!list.contains("/projects/b") && !list.contains("Beta demo"));
        let (st, page) = text(h.handle(req("GET", ex, ar, "/projects/a", b"")));
        assert_eq!(st, 200);
        assert!(page.contains("<h2>project</h2>") && page.contains("<h2>demo</h2>"));
        assert!(page.contains("your role: read"));
        assert_eq!(h.handle(req("GET", ex, ar, "/projects/b", b"")).status, 404);
        assert_eq!(h.handle(req("GET", ex, OB, "/projects/a", b"")).status, 404);
        assert_eq!(h.handle(req("POST", ex, ar, "/", b"")).status, 405);
        // The project under its path: pages resolve links from there.
        let (st, files) = text(h.handle(req("GET", ex, ar, "/p/a/", b"")));
        assert_eq!(st, 200);
        assert!(files.contains("<base href=\"/p/a/\">") && files.contains("pub"));
        let (st, note) = text(h.handle(req("GET", ex, ar, "/p/a/dav/pub/one.md", b"")));
        assert_eq!((st, note.as_str()), (200, "alpha shared\n"));
        assert_eq!(
            h.handle(req("PUT", ex, ar, "/p/a/dav/pub/x.md", b"x"))
                .status,
            403
        );
        assert_eq!(
            h.handle(req("PUT", ex, aw, "/p/a/dav/pub/x.md", b"x"))
                .status,
            201
        );
        // MOVE: a Destination in the project, as a URL or a path, lands in
        // it; one in another project is refused.
        let mv = |to: &str| {
            let mut r = req("MOVE", ex, aw, "/p/a/dav/pub/x.md", b"");
            r.headers.push(("Destination".into(), to.into()));
            h.handle(r).status
        };
        assert_eq!(mv("/p/b/dav/x.md"), 400);
        assert_eq!(mv("https://explorer.example/p/a/dav/pub/y.md"), 201);
        let mut l = h.projects["a"].live.lock().unwrap();
        assert!(matches!(l.stat("pub/y.md"), Ok(Some((Kind::File, _)))));
        assert!(matches!(l.stat("pub/x.md"), Ok(None)));
        drop(l);
        // The project's own host name serves as before, with the root base.
        let (st, own) = text(h.handle(req("GET", "a.example-host", ar, "/", b"")));
        assert_eq!(st, 200);
        assert!(own.contains("<base href=\"/\">"));
        // Body limits follow the project the path names.
        assert_eq!(
            h.body_limit("explorer.example:8750", "/p/c/dav/big"),
            Some(1024)
        );
        assert_eq!(h.body_limit("explorer.example", "/p/a/dav/x"), None);
        assert_eq!(h.body_limit("explorer.example", "/"), Some(0));
        assert_eq!(h.body_limit("c.example-host", "/dav/x"), Some(1024));
        done(dirs);
    }

    #[test]
    fn projects_are_separate() {
        let (h, dirs) = host("separate");
        let get = |host: &str, who, path: &str| h.handle(req("GET", host, who, path, b""));
        assert_eq!(get("a.example-host", OA, "/dav/pub/one.md").status, 200);
        assert_eq!(get("a.localhost", OA, "/dav/pub/one.md").status, 200);
        // b's users and b's host name see none of a.
        assert_eq!(get("a.example-host", OB, "/dav/pub/one.md").status, 401);
        assert_eq!(get("b.example-host", OB, "/dav/pub/one.md").status, 404);
        assert_eq!(get("b.example-host", OA, "/dav/").status, 401);
        let listing = h.handle(req("PROPFIND", "b.example-host", OB, "/dav/", b""));
        assert!(!String::from_utf8_lossy(&listing.body).contains("pub"));
        // Unknown names, no name and a wrong password answer alike.
        let unknown = get("z.example-host", OA, "/dav/");
        assert_eq!(
            (unknown.status, unknown.body.clone()),
            (404, b"no such project".to_vec())
        );
        assert_eq!(
            h.handle(Request {
                headers: vec![],
                ..req("GET", "", None, "/", b"")
            })
            .status,
            404
        );
        let wrong = get("a.example-host", Some(("a-owner", "nope")), "/dav/");
        let none = get("a.example-host", None, "/dav/");
        assert_eq!((wrong.status, wrong.body), (none.status, none.body));
        // Search in b finds nothing of a.
        let s = get("b.example-host", OB, "/api/search?q=alpha");
        assert!(!String::from_utf8_lossy(&s.body).contains("pub/one.md"));
        done(dirs);
    }

    fn bearer(method: &str, host: &str, token: &str, target: &str, body: &[u8]) -> Request {
        let mut r = req(method, host, None, target, body);
        r.headers
            .push(("Authorization".into(), format!("Bearer {token}")));
        r
    }

    #[test]
    fn tokens_open_one_project_with_their_role_and_rates_answer_429() {
        let dirs: Vec<PathBuf> = ["a", "b"]
            .iter()
            .map(|p| dir(&format!("tok-{p}")))
            .collect();
        let (ra, ta) = crate::tokens::make().unwrap();
        let (wa, twa) = crate::tokens::make().unwrap();
        let (old, told) = crate::tokens::make().unwrap();
        let (rb, tb) = crate::tokens::make().unwrap();
        let mut text = String::new();
        for (p, d) in ["a", "b"].iter().zip(&dirs) {
            text += &format!("project {p} {}\n", d.display());
            text += &format!("user {p} owner {}\n", user(&format!("{p}-owner"), "pw"));
        }
        text += &format!("token a read {ra}\ntoken a write {wa} rate=3/m\n");
        text += &format!("token a owner {old} expires=1000\ntoken b read {rb}\n");
        text += "rate b 5/m\n";
        // One token in two projects would join them.
        let twice = format!("{text}token b read {ra}\n");
        assert!(Config::parse(&twice).is_err());
        assert!(Config::parse(&format!("{text}token a read nope\n")).is_err());
        assert!(Config::parse(&format!("{text}rate a 5\n")).is_err());
        let h = Host::open(Config::parse(&text).unwrap()).unwrap();
        let st = |r: Request| h.handle(r).status;
        // A read token reads its project only, and writes nothing.
        assert_eq!(st(req("GET", "a.localhost", OA, "/", b"")), 200);
        assert_eq!(st(bearer("GET", "a.localhost", &ta, "/", b"")), 200);
        assert_eq!(
            st(bearer("PUT", "a.localhost", &ta, "/dav/x.md", b"x")),
            403
        );
        assert_eq!(st(bearer("GET", "b.localhost", &ta, "/", b"")), 401);
        assert_eq!(st(bearer("GET", "a.localhost", &tb, "/", b"")), 401);
        // An expired owner token, a wrong secret, an owner action with a
        // write token.
        assert_eq!(st(bearer("GET", "a.localhost", &told, "/", b"")), 401);
        let wrong = format!("{}0", &ta[..ta.len() - 1]);
        let wrong = if wrong == ta {
            format!("{}1", &ta[..ta.len() - 1])
        } else {
            wrong
        };
        assert_eq!(st(bearer("GET", "a.localhost", &wrong, "/", b"")), 401);
        assert_eq!(st(bearer("POST", "a.localhost", &twa, "/api/gc", b"")), 403);
        // The write token's own rate: 3 a minute, the first taken above.
        assert_eq!(
            st(bearer("PUT", "a.localhost", &twa, "/dav/w.md", b"w")),
            201
        );
        assert_eq!(
            st(bearer("GET", "a.localhost", &twa, "/dav/w.md", b"")),
            200
        );
        let over = h.handle(bearer("GET", "a.localhost", &twa, "/dav/w.md", b""));
        assert_eq!(over.status, 429);
        let retry: u64 = over
            .headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("retry-after"))
            .unwrap()
            .1
            .parse()
            .unwrap();
        assert!((1..=20).contains(&retry), "{retry}");
        // Others in a are not slowed by it.
        assert_eq!(st(bearer("GET", "a.localhost", &ta, "/dav/w.md", b"")), 200);
        // b's rate counts its token and its owner together.
        for _ in 0..3 {
            assert_eq!(st(bearer("GET", "b.localhost", &tb, "/", b"")), 200);
        }
        let ob = Some(("b-owner", "pw"));
        assert_eq!(st(req("GET", "b.localhost", ob, "/", b"")), 200);
        assert_eq!(st(req("GET", "b.localhost", ob, "/", b"")), 200);
        assert_eq!(st(req("GET", "b.localhost", ob, "/", b"")), 429);
        assert_eq!(st(bearer("GET", "b.localhost", &tb, "/", b"")), 429);
        // A wrong password is not counted, and still gets 401.
        assert_eq!(
            st(req("GET", "b.localhost", Some(("b-owner", "no")), "/", b"")),
            401
        );
        assert_eq!(st(req("GET", "a.localhost", OA, "/", b"")), 200);
        done(dirs);
    }

    #[test]
    fn heavy_jobs_take_turns_across_projects() {
        let ms = Duration::from_millis;
        let h = Heavy::new(1, ms(50));
        let first = h.enter().unwrap();
        assert_eq!(h.load(), (1, 0));
        let t = std::time::Instant::now();
        assert!(h.enter().is_none());
        assert!(t.elapsed() >= ms(50));
        assert_eq!(h.load(), (1, 0), "a waiter that gave up leaves the queue");
        // A waiter goes in once the running job ends.
        let h = Arc::new(Heavy::new(1, Duration::from_secs(10)));
        let held = h.enter().unwrap();
        let h2 = h.clone();
        let waiter = std::thread::spawn(move || {
            let t = std::time::Instant::now();
            h2.enter().map(|_| t.elapsed())
        });
        std::thread::sleep(ms(100));
        assert_eq!(h.load(), (1, 1));
        drop(held);
        let waited = waiter.join().unwrap().unwrap();
        assert!(waited >= ms(90), "{waited:?}");
        assert_eq!(h.load(), (0, 0));
        let two = Heavy::new(2, ms(10));
        let (_a, _b) = (two.enter().unwrap(), two.enter().unwrap());
        assert!(two.enter().is_none());
        drop(first);
        // Through the host: a heavy job past the wait gets 503; other
        // requests do not wait.
        let (hst, dirs) = host("heavy");
        let mut text = String::from("heavy-jobs 1\nheavy-wait 0\n");
        for (p, d) in ["a", "b"].iter().zip(&dirs) {
            text += &format!("project {p} {}\n", d.display());
            text += &format!("user {p} owner {}\n", user(&format!("{p}-owner"), "pw"));
        }
        drop(hst);
        assert!(Config::parse("heavy-jobs 0\n").is_err());
        let h = Host::open(Config::parse(&text).unwrap()).unwrap();
        assert_eq!(
            h.handle(req("POST", "b.localhost", OB, "/api/fsck", b""))
                .status,
            200
        );
        let busy = h.heavy.as_ref().unwrap().enter().unwrap();
        let r = h.handle(req("POST", "b.localhost", OB, "/api/fsck", b""));
        assert_eq!(r.status, 503);
        assert!(
            r.headers
                .iter()
                .any(|(k, v)| k == "Retry-After" && v == "1")
        );
        assert_eq!(
            h.handle(req("GET", "a.localhost", OA, "/", b"")).status,
            200
        );
        drop(busy);
        assert_eq!(
            h.handle(req("POST", "a.localhost", OA, "/api/fsck", b""))
                .status,
            200
        );
        done(dirs);
    }

    #[test]
    fn roles_limit_what_a_user_does() {
        let (h, dirs) = host("roles");
        let r = |m: &str, who: &str, path: &str| {
            h.handle(req(m, "a.example-host", Some((who, "pw")), path, b"x"))
                .status
        };
        assert_eq!(r("GET", "a-reader", "/dav/pub/one.md"), 200);
        assert_eq!(r("PUT", "a-reader", "/dav/new.md"), 403);
        assert_eq!(r("PUT", "a-writer", "/dav/new.md"), 201);
        assert_eq!(r("POST", "a-writer", "/api/gc"), 403);
        assert_eq!(
            r(
                "POST",
                "a-writer",
                "/shares/issue?name=s&to=b&scope=path:pub&right=read&expires=60"
            ),
            403
        );
        assert_eq!(r("GET", "a-writer", "/shares"), 403);
        assert_eq!(r("POST", "a-owner", "/api/fsck"), 200);
        // E360: the agent tools follow the same roles; promoting a draft
        // writes the main ref, so it is the owner's.
        assert_eq!(r("GET", "a-reader", "/api/tool/status"), 200);
        assert_eq!(r("POST", "a-reader", "/api/tool/write"), 403);
        assert_eq!(r("POST", "a-writer", "/api/tool/draft_promote"), 403);
        assert_eq!(r("POST", "a-owner", "/api/tool/draft_promote"), 400);
        // Per-project body limit.
        let big = vec![b'x'; 2048];
        assert_eq!(
            h.handle(req(
                "PUT",
                "c.localhost",
                Some(("c-owner", "pw")),
                "/dav/big",
                &big
            ))
            .status,
            413
        );
        assert_eq!(
            h.handle(req("PUT", "a.localhost", OA, "/dav/big", &big))
                .status,
            201
        );
        done(dirs);
    }

    #[test]
    fn shares_open_only_their_scope_and_merge_on_the_owners_word() {
        let (h, dirs) = host("shares");
        let post =
            |host: &str, who, path: &str, body: &[u8]| h.handle(req("POST", host, who, path, body));
        let get = |host: &str, who, path: &str| h.handle(req("GET", host, who, path, b""));
        let issue = post(
            "a.localhost",
            OA,
            "/shares/issue?name=pub&to=b&scope=path:pub&right=merge&expires=600",
            b"",
        );
        assert_eq!(
            issue.status,
            200,
            "{}",
            String::from_utf8_lossy(&issue.body)
        );
        // b reads inside the scope, nothing outside; c and a's own users not at all.
        let one = get(
            "b.localhost",
            Some(("b-reader", "pw")),
            "/from/a/pub/read?path=pub/one.md",
        );
        assert_eq!((one.status, one.body), (200, b"alpha shared\n".to_vec()));
        assert_eq!(
            get(
                "b.localhost",
                OB,
                "/from/a/pub/read?path=pub/one.md&range=0-5"
            )
            .body,
            b"alpha"
        );
        for path in [
            "priv/secret.md",
            "pub/../priv/secret.md",
            "pub",
            "pubx/one.md",
        ] {
            assert_eq!(
                get("b.localhost", OB, &format!("/from/a/pub/read?path={path}")).status,
                404,
                "{path}"
            );
        }
        assert_eq!(
            get(
                "c.localhost",
                Some(("c-owner", "pw")),
                "/from/a/pub/read?path=pub/one.md"
            )
            .status,
            404
        );
        assert_eq!(get("a.localhost", OA, "/from/a/pub/files").status, 404);
        assert_eq!(get("b.localhost", OB, "/from/a/missing/files").status, 404);
        let files = get("b.localhost", OB, "/from/a/pub/files");
        assert_eq!(files.body, br#"["pub/one.md","pub/two.md"]"#);
        let hits = get("b.localhost", OB, "/from/a/pub/search?q=alpha");
        assert_eq!(hits.body, br#"["pub/one.md"]"#);
        // Only b's owner merges; the copy is b's own.
        assert_eq!(
            post(
                "b.localhost",
                Some(("b-writer", "pw")),
                "/from/a/pub/merge",
                b""
            )
            .status,
            403
        );
        let m = post("b.localhost", OB, "/from/a/pub/merge", b"");
        assert_eq!(m.status, 200, "{}", String::from_utf8_lossy(&m.body));
        assert_eq!(
            get("b.localhost", OB, "/dav/pub/one.md").body,
            b"alpha shared\n"
        );
        assert_eq!(get("b.localhost", OB, "/dav/priv/secret.md").status, 404);
        // A read-only share cannot be merged or proposed to.
        post(
            "a.localhost",
            OA,
            "/shares/issue?name=ro&to=b&scope=path:pub&right=read&expires=600",
            b"",
        );
        assert_eq!(post("b.localhost", OB, "/from/a/ro/merge", b"").status, 404);
        assert_eq!(
            post(
                "b.localhost",
                OB,
                "/from/a/ro/propose?path=pub/two.md",
                b"x"
            )
            .status,
            404
        );
        // Revoked: reads stop at once; what was merged stays.
        assert_eq!(
            post("a.localhost", OA, "/shares/revoke?name=pub", b"").status,
            200
        );
        assert_eq!(get("b.localhost", OB, "/from/a/pub/files").status, 404);
        assert_eq!(
            get("b.localhost", OB, "/dav/pub/one.md").body,
            b"alpha shared\n"
        );
        // Expired shares refuse; an issue for an unknown project is refused.
        post(
            "a.localhost",
            OA,
            "/shares/issue?name=gone&to=b&scope=path:pub&right=read&expires=0",
            b"",
        );
        assert_eq!(get("b.localhost", OB, "/from/a/gone/files").status, 404);
        assert_eq!(
            post(
                "a.localhost",
                OA,
                "/shares/issue?name=x&to=zz&scope=path:pub&right=read&expires=60",
                b""
            )
            .status,
            400
        );
        done(dirs);
    }

    /// E416: `hits` of a `rank` answer, as (path, score).
    fn hits(body: &[u8]) -> (bool, Vec<(String, f32)>) {
        let j = crate::json::parse(std::str::from_utf8(body).unwrap()).unwrap();
        let hybrid = matches!(j.get("hybrid"), Some(Json::Bool(true)));
        let hits = j
            .get("hits")
            .unwrap()
            .as_arr()
            .iter()
            .map(|h| match h.as_arr() {
                [Json::Str(p), Json::Num(s)] => (p.clone(), *s as f32),
                _ => panic!("{h:?}"),
            })
            .collect();
        (hybrid, hits)
    }

    #[test]
    fn ranked_and_vector_search_through_a_share_stays_in_its_scope() {
        let owner = user("o", "pw");
        let c = Config::parse(&format!(
            "project a st\nuser a owner {owner}\nshare-rank a\nfile-vectors a v\n"
        ))
        .unwrap();
        assert!(c.projects[0].share_rank);
        assert_eq!(c.projects[0].file_vectors.as_deref(), Some("v"));
        let c = Config::parse(&format!("project a st\nuser a owner {owner}\n")).unwrap();
        assert!(!c.projects[0].share_rank && c.projects[0].file_vectors.is_none());
        assert!(
            Config::parse(&format!(
                "project a st\nuser a owner {owner}\nshare-rank b\n"
            ))
            .is_err()
        );
        let (h, dirs) = host_with("rank", "share-rank a\n");
        let post = |host: &str, who, path: &str| h.handle(req("POST", host, who, path, b""));
        let get = |host: &str, who, path: &str| h.handle(req("GET", host, who, path, b""));
        // a's files (with near matches outside pub/), and c holding only
        // a's pub/ files: the answer through the share must be c's own.
        let files: &[(&str, &[u8], [f32; 2])] = &[
            ("pub/one.md", b"alpha shared\n", [1.0, 0.0]),
            ("pub/two.md", b"beta\n", [0.0, 1.0]),
            ("pub/three.md", b"alpha alpha gamma\n", [0.5, 0.5]),
            ("priv/secret.md", b"alpha private\n", [1.0, 0.1]),
            ("priv/near.md", b"alpha alpha alpha gamma\n", [0.9, 0.0]),
            ("pubx/side.md", b"alpha gamma gamma\n", [1.0, 0.0]),
        ];
        let vecs = format!("{}.vecs", dirs[0].display());
        let paths: String = files.iter().map(|f| format!("{}\n", f.0)).collect();
        std::fs::write(format!("{vecs}.paths"), paths).unwrap();
        let flat: Vec<f32> = files.iter().flat_map(|f| f.2).collect();
        std::fs::write(
            format!("{vecs}.f32"),
            ikam_kernel::vectors::from_f32s(&flat),
        )
        .unwrap();
        let vecs = vecs.as_str();
        for (p, only_pub) in [("a", false), ("c", true)] {
            let mut l = h.projects[p].live.lock().unwrap();
            let ops = files
                .iter()
                .filter(|f| !only_pub || f.0.starts_with("pub/"))
                .map(|f| live::Op::Put(f.0.into(), f.1.to_vec()))
                .collect();
            l.apply(ops).unwrap();
            l.flush(true).unwrap();
            assert!(l.load_file_vectors(vecs).unwrap() >= 3);
        }
        let issue = "/shares/issue?name=pub&to=b&scope=path:pub&right=read&expires=600";
        assert_eq!(post("a.localhost", OA, issue).status, 200);
        let want = |q: &str, v: Option<&[f32]>, k: usize, fuse: live::Fuse| {
            let mut c = h.projects["c"].live.lock().unwrap();
            c.rank_files(q, v, k, fuse).unwrap()
        };
        let convex = live::Fuse::Convex(ikam_kernel::fusion::DEFAULT_ALPHA);
        type Case<'a> = (&'a str, &'a str, Option<&'a [f32]>, usize, live::Fuse);
        let cases: &[Case] = &[
            ("alpha", "", None, 10, convex),
            ("alpha gamma", "&k=1", None, 1, convex),
            ("alpha", "&vector=1,0", Some(&[1.0, 0.0]), 10, convex),
            (
                "gamma",
                "&vector=0.2,1&alpha=0.3",
                Some(&[0.2, 1.0]),
                10,
                live::Fuse::Convex(0.3),
            ),
            (
                "alpha",
                "&vector=1,0&fuse=rrf",
                Some(&[1.0, 0.0]),
                10,
                live::Fuse::Rrf,
            ),
        ];
        for (q, more, v, k, fuse) in cases {
            let r = get(
                "b.localhost",
                Some(("b-reader", "pw")),
                &format!("/from/a/pub/rank?q={}{more}", q.replace(' ', "+")),
            );
            assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
            let (hybrid, got) = hits(&r.body);
            assert_eq!(hybrid, v.is_some());
            assert!(got.iter().all(|(p, _)| p.starts_with("pub/")), "{got:?}");
            assert!(!got.is_empty());
            assert_eq!(got, want(q, *v, *k, *fuse), "{q}{more}");
        }
        // Files outside the scope move no score: a's private files change
        // and a's owner shares again; the answer is the same.
        let before = get("b.localhost", OB, "/from/a/pub/rank?q=alpha&vector=1,0").body;
        put_files(&h, "a", &[("priv/more.md", b"alpha alpha beta beta\n")]);
        assert_eq!(post("a.localhost", OA, issue).status, 200);
        assert_eq!(
            get("b.localhost", OB, "/from/a/pub/rank?q=alpha&vector=1,0").body,
            before
        );
        // A malformed query after the check is a 400.
        assert_eq!(
            get("b.localhost", OB, "/from/a/pub/rank?q=a&vector=1").status,
            400
        );
        assert_eq!(
            get("b.localhost", OB, "/from/a/pub/rank?q=a&vector=x").status,
            400
        );
        // Refusals stay 404: another project, the source itself, an unknown
        // share, a scope that is not a folder, a project without
        // `share-rank`, and a revoked or expired share.
        let c_owner = Some(("c-owner", "pw"));
        assert_eq!(
            get("c.localhost", c_owner, "/from/a/pub/rank?q=alpha").status,
            404
        );
        assert_eq!(
            get("a.localhost", OA, "/from/a/pub/rank?q=alpha").status,
            404
        );
        assert_eq!(
            get("b.localhost", OB, "/from/a/nope/rank?q=alpha").status,
            404
        );
        let sec = "/shares/issue?name=sec&to=b&scope=section:pub/one.md@0-5&right=read&expires=600";
        assert_eq!(post("a.localhost", OA, sec).status, 200);
        assert_eq!(
            get(
                "b.localhost",
                OB,
                "/from/a/sec/read?path=pub/one.md&range=0-5"
            )
            .status,
            200
        );
        assert_eq!(
            get("b.localhost", OB, "/from/a/sec/rank?q=alpha").status,
            404
        );
        let cs = "/shares/issue?name=cs&to=b&scope=path:pub&right=read&expires=600";
        assert_eq!(post("c.localhost", c_owner, cs).status, 200);
        assert_eq!(get("b.localhost", OB, "/from/c/cs/files").status, 200);
        assert_eq!(
            get("b.localhost", OB, "/from/c/cs/rank?q=alpha").status,
            404
        );
        let gone = "/shares/issue?name=gone&to=b&scope=path:pub&right=read&expires=0";
        assert_eq!(post("a.localhost", OA, gone).status, 200);
        assert_eq!(
            get("b.localhost", OB, "/from/a/gone/rank?q=alpha").status,
            404
        );
        assert_eq!(
            post("a.localhost", OA, "/shares/revoke?name=pub").status,
            200
        );
        assert_eq!(
            get("b.localhost", OB, "/from/a/pub/rank?q=alpha").status,
            404
        );
        for ext in ["paths", "f32"] {
            let _ = std::fs::remove_file(format!("{vecs}.{ext}"));
        }
        done(dirs);
    }

    fn put_files(h: &Host, p: &str, files: &[(&str, &[u8])]) {
        let mut l = h.projects[p].live.lock().unwrap();
        let ops = files
            .iter()
            .map(|f| live::Op::Put(f.0.into(), f.1.to_vec()))
            .collect();
        l.apply(ops).unwrap();
        l.flush(true).unwrap();
    }

    #[test]
    fn line_merges_through_shares_only_when_asked() {
        // E417: with `lines=1`, a merge or an accept keeps both sides'
        // edits to different lines of one file; the same lines answer 409.
        let (h, dirs) = host("lines");
        let post = |host: &str, who, path: &str, body: &[u8]| {
            let r = h.handle(req("POST", host, who, path, body));
            (r.status, String::from_utf8_lossy(&r.body).into_owned())
        };
        let put = |host: &str, who, path: &str, body: &[u8]| {
            let r = h.handle(req("PUT", host, who, path, body));
            assert!(r.status < 300, "{path}: {}", r.status);
        };
        let get = |host: &str, who, path: &str| h.handle(req("GET", host, who, path, b"")).body;
        let issue = "/shares/issue?name=pub&to=b&scope=path:pub&right=merge&expires=600";
        put(
            "a.localhost",
            OA,
            "/dav/pub/lines.md",
            b"one\ntwo\nthree\nfour\n",
        );
        assert_eq!(post("a.localhost", OA, issue, b"").0, 200);
        assert_eq!(post("b.localhost", OB, "/from/a/pub/merge", b"").0, 200);
        // b edits line 1, a edits line 4, and a shares again.
        put(
            "b.localhost",
            OB,
            "/dav/pub/lines.md",
            b"ONE\ntwo\nthree\nfour\n",
        );
        put(
            "a.localhost",
            OA,
            "/dav/pub/lines.md",
            b"one\ntwo\nthree\nFOUR\n",
        );
        assert_eq!(post("a.localhost", OA, issue, b"").0, 200);
        let (st, body) = post("b.localhost", OB, "/from/a/pub/merge", b"");
        assert_eq!(st, 409, "{body}");
        assert!(body.contains("Path(\"pub/lines.md\")"), "{body}");
        let (st, body) = post("b.localhost", OB, "/from/a/pub/merge?lines=1", b"");
        assert_eq!(st, 200, "{body}");
        assert_eq!(
            get("b.localhost", OB, "/dav/pub/lines.md"),
            b"ONE\ntwo\nthree\nFOUR\n"
        );
        // Both edit line 2: refused, b's file as it was.
        put(
            "b.localhost",
            OB,
            "/dav/pub/lines.md",
            b"ONE\nb2\nthree\nFOUR\n",
        );
        put(
            "a.localhost",
            OA,
            "/dav/pub/lines.md",
            b"one\na2\nthree\nFOUR\n",
        );
        assert_eq!(post("a.localhost", OA, issue, b"").0, 200);
        let (st, body) = post("b.localhost", OB, "/from/a/pub/merge?lines=1", b"");
        assert_eq!(st, 409, "{body}");
        assert!(body.contains("Lines"), "{body}");
        assert_eq!(
            get("b.localhost", OB, "/dav/pub/lines.md"),
            b"ONE\nb2\nthree\nFOUR\n"
        );
        // A proposal accepted by a's owner merges by lines when asked.
        let prop = "/shares/issue?name=p&to=b&scope=path:pub&right=propose&expires=600";
        assert_eq!(post("a.localhost", OA, prop, b"").0, 200);
        let (st, _) = post(
            "b.localhost",
            Some(("b-writer", "pw")),
            "/from/a/p/propose?path=pub/lines.md",
            b"one\na2\nthree\nFOUR\nfive, from b\n",
        );
        assert_eq!(st, 200);
        put(
            "a.localhost",
            OA,
            "/dav/pub/lines.md",
            b"one, a\na2\nthree\nFOUR\n",
        );
        assert_eq!(post("a.localhost", OA, "/shares/accept?name=p", b"").0, 409);
        let (st, body) = post("a.localhost", OA, "/shares/accept?name=p&lines=1", b"");
        assert_eq!(st, 200, "{body}");
        assert_eq!(
            get("a.localhost", OA, "/dav/pub/lines.md"),
            b"one, a\na2\nthree\nFOUR\nfive, from b\n"
        );
        done(dirs);
    }

    #[test]
    fn a_merge_keeps_to_the_receivers_store_limit() {
        let (h, dirs) = host("quota");
        let post = |host: &str, who, path: &str| h.handle(req("POST", host, who, path, b""));
        let get = |host: &str, who, path: &str| h.handle(req("GET", host, who, path, b""));
        let issue = post(
            "a.localhost",
            OA,
            "/shares/issue?name=pub&to=b&scope=path:pub&right=merge&expires=600",
        );
        assert_eq!(issue.status, 200);
        {
            let mut b = h.projects["b"].live.lock().unwrap();
            let used = b.used_bytes();
            b.max_store = Some(used + 4);
        }
        assert_eq!(post("b.localhost", OB, "/from/a/pub/merge").status, 507);
        assert_eq!(get("b.localhost", OB, "/dav/pub/one.md").status, 404);
        assert_eq!(
            post("b.localhost", OB, "/from/a/pub/merge?into=shares").status,
            400
        );
        h.projects["b"].live.lock().unwrap().max_store = None;
        assert_eq!(post("b.localhost", OB, "/from/a/pub/merge").status, 200);
        assert_eq!(get("b.localhost", OB, "/dav/pub/one.md").status, 200);
        done(dirs);
    }

    #[test]
    fn section_and_fragment_merges_keep_to_the_store_limit() {
        let (h, dirs) = host("quota-scopes");
        let post = |host: &str, who, path: &str| h.handle(req("POST", host, who, path, b""));
        let two = match h.projects["a"].live.lock().unwrap().stat("pub/two.md") {
            Ok(Some((_, id))) => id,
            _ => panic!("no pub/two.md"),
        };
        for (name, scope) in [
            ("sec", "section:pub/one.md@0-12".to_string()),
            ("frag", format!("fragments:{}", two.to_hex())),
        ] {
            let issue =
                format!("/shares/issue?name={name}&to=b&scope={scope}&right=merge&expires=600");
            assert_eq!(post("a.localhost", OA, &issue).status, 200);
            let merge = format!("/from/a/{name}/merge");
            {
                let mut b = h.projects["b"].live.lock().unwrap();
                let used = b.used_bytes();
                b.max_store = Some(used + 4);
            }
            assert_eq!(post("b.localhost", OB, &merge).status, 507, "{name}");
            h.projects["b"].live.lock().unwrap().max_store = None;
            assert_eq!(post("b.localhost", OB, &merge).status, 200, "{name}");
        }
        done(dirs);
    }

    #[test]
    fn proposals_wait_for_the_source_owner() {
        let (h, dirs) = host("propose");
        let post =
            |host: &str, who, path: &str, body: &[u8]| h.handle(req("POST", host, who, path, body));
        post(
            "a.localhost",
            OA,
            "/shares/issue?name=p&to=b&scope=path:pub&right=propose&expires=600",
            b"",
        );
        assert_eq!(
            post(
                "b.localhost",
                Some(("b-reader", "pw")),
                "/from/a/p/propose?path=pub/two.md",
                b"x"
            )
            .status,
            403
        );
        assert_eq!(
            post(
                "b.localhost",
                Some(("b-writer", "pw")),
                "/from/a/p/propose?path=priv/secret.md",
                b"x"
            )
            .status,
            404
        );
        let r = post(
            "b.localhost",
            Some(("b-writer", "pw")),
            "/from/a/p/propose?path=pub/two.md",
            b"beta, edited\n",
        );
        assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
        let get = |who, path: &str| h.handle(req("GET", "a.localhost", who, path, b""));
        assert_eq!(get(OA, "/dav/pub/two.md").body, b"beta\n");
        assert_eq!(
            post(
                "a.localhost",
                Some(("a-writer", "pw")),
                "/shares/accept?name=p",
                b""
            )
            .status,
            403
        );
        assert_eq!(
            post("a.localhost", OA, "/shares/accept?name=p", b"").status,
            200
        );
        assert_eq!(get(OA, "/dav/pub/two.md").body, b"beta, edited\n");
        done(dirs);
    }

    #[test]
    fn a_failing_project_leaves_the_others_serving() {
        let (h, dirs) = host("fault");
        let live = h.projects["a"].live.clone();
        // Poison a's lock, as a panic while serving it would.
        let _ = std::thread::spawn(move || {
            let _l = live.lock().unwrap();
            panic!("a fails");
        })
        .join();
        assert_eq!(
            h.handle(req("GET", "a.localhost", OA, "/dav/pub/one.md", b""))
                .status,
            500
        );
        assert_eq!(
            h.handle(req("PUT", "b.localhost", OB, "/dav/x.md", b"x"))
                .status,
            201
        );
        assert_eq!(
            h.handle(req("GET", "b.localhost", OB, "/dav/x.md", b""))
                .body,
            b"x"
        );
        done(dirs);
    }

    /// E416 bench: ranking through a folder share against a project that
    /// holds only the scope (keys and scores), how often whole-store BM25
    /// statistics would order the scope differently, and the cost. Synthetic
    /// Zipf text; `E416_FILES` files a side (2,000), `E416_QUERIES` (300).
    /// `cargo test --release --features embed --bin ikam share_rank_bench --
    /// --ignored --nocapture`.
    #[test]
    #[ignore]
    fn share_rank_bench() {
        use std::time::Instant;
        let env = |k: &str, d: usize| {
            std::env::var(k)
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(d)
        };
        let (n, nq) = (env("E416_FILES", 2000), env("E416_QUERIES", 300));
        let (h, dirs) = host_with("rankbench", "share-rank a\n");
        let mut seed = 0x9e37_79b9_7f4a_7c15u64;
        let mut rnd = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        // Zipf over 20,000 words: cumulative weights 1/(rank+1).
        let vocab: Vec<String> = (0..20_000).map(|i| format!("w{i:x}q")).collect();
        let mut cum = Vec::with_capacity(vocab.len());
        let mut t = 0.0f64;
        for r in 0..vocab.len() {
            t += 1.0 / (r as f64 + 1.0);
            cum.push(t);
        }
        let word = |rnd: &mut dyn FnMut() -> u64| {
            let x = (rnd() >> 11) as f64 / (1u64 << 53) as f64 * t;
            &vocab[cum.partition_point(|c| *c < x).min(vocab.len() - 1)]
        };
        let doc = |rnd: &mut dyn FnMut() -> u64| {
            let len = 100 + (rnd() % 500) as usize;
            (0..len)
                .map(|_| word(rnd).as_str())
                .collect::<Vec<_>>()
                .join(" ")
                + "\n"
        };
        const DIM: usize = 64;
        let embed = |text: &str| {
            let mut v = vec![0f32; DIM];
            for w in text.split_whitespace() {
                let h = blake3::hash(w.as_bytes()).as_bytes()[0] as usize % DIM;
                v[h] += 1.0;
            }
            let n = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-9);
            v.iter().map(|x| x / n).collect::<Vec<f32>>()
        };
        let mut files: Vec<(String, String)> = Vec::new();
        let mut queries: Vec<String> = Vec::new();
        for i in 0..n {
            let text = doc(&mut rnd);
            let ws: Vec<&str> = text.split_whitespace().collect();
            let q: Vec<&str> = (0..3).map(|_| ws[rnd() as usize % ws.len()]).collect();
            let q = q.join(" ");
            if i % 4 == 0 {
                // A near copy outside the scope that matches the query better.
                files.push((format!("priv/near{i}.md"), format!("{text}{q} {q}\n")));
            }
            if queries.len() < nq {
                queries.push(q);
            }
            files.push((format!("pub/d{i}.md"), text));
            files.push((format!("priv/p{i}.md"), doc(&mut rnd)));
            if i % 4 == 1 {
                files.push((format!("pubx/x{i}.md"), doc(&mut rnd)));
            }
        }
        let t0 = Instant::now();
        let vecs = format!("{}.vecs", dirs[0].display());
        let paths: String = files.iter().map(|f| format!("{}\n", f.0)).collect();
        std::fs::write(format!("{vecs}.paths"), paths).unwrap();
        let flat: Vec<f32> = files.iter().flat_map(|f| embed(&f.1)).collect();
        std::fs::write(
            format!("{vecs}.f32"),
            ikam_kernel::vectors::from_f32s(&flat),
        )
        .unwrap();
        for (p, only_pub) in [("a", false), ("c", true)] {
            let mut l = h.projects[p].live.lock().unwrap();
            // c gets exactly a's pub/ files, host()'s two included.
            let mut ops: Vec<live::Op> = files
                .iter()
                .filter(|f| !only_pub || f.0.starts_with("pub/"))
                .map(|f| live::Op::Put(f.0.clone(), f.1.clone().into_bytes()))
                .collect();
            if only_pub {
                ops.push(live::Op::Put(
                    "pub/one.md".into(),
                    b"alpha shared\n".to_vec(),
                ));
                ops.push(live::Op::Put("pub/two.md".into(), b"beta\n".to_vec()));
            }
            l.apply(ops).unwrap();
            l.flush(true).unwrap();
            l.load_file_vectors(&vecs).unwrap();
            l.index_files();
        }
        let load = t0.elapsed().as_secs_f64();
        let issue = "/shares/issue?name=pub&to=b&scope=path:pub&right=read&expires=6000";
        assert_eq!(
            h.handle(req("POST", "a.localhost", OA, issue, b"")).status,
            200
        );
        let convex = live::Fuse::Convex(ikam_kernel::fusion::DEFAULT_ALPHA);
        let mut report = String::new();
        let enc = |v: &[f32]| {
            v.iter()
                .map(|x| x.to_string())
                .collect::<Vec<_>>()
                .join(",")
        };
        let pct = |mut v: Vec<f64>, p: f64| {
            v.sort_by(f64::total_cmp);
            v[((v.len() as f64 - 1.0) * p).round() as usize] * 1e3
        };
        for (mode, more) in [("keyword", 0), ("fused convex", 1), ("fused rrf", 2)] {
            let (mut same, mut outside, mut share_t, mut ref_t, mut own_t) =
                (0, 0, Vec::new(), Vec::new(), Vec::new());
            let mut first = None;
            for q in &queries {
                let qv = embed(q);
                let (v, extra, fuse) = match more {
                    0 => (None, String::new(), convex),
                    1 => (Some(qv.as_slice()), format!("&vector={}", enc(&qv)), convex),
                    _ => (
                        Some(qv.as_slice()),
                        format!("&vector={}&fuse=rrf", enc(&qv)),
                        live::Fuse::Rrf,
                    ),
                };
                let target = format!("/from/a/pub/rank?q={}{extra}", q.replace(' ', "+"));
                let t = Instant::now();
                let r = h.handle(req("GET", "b.localhost", OB, &target, b""));
                let dt = t.elapsed().as_secs_f64();
                if first.is_none() {
                    first = Some(dt);
                } else {
                    share_t.push(dt);
                }
                assert_eq!(r.status, 200);
                let (_, got) = hits(&r.body);
                outside += got.iter().filter(|(p, _)| !p.starts_with("pub/")).count();
                let t = Instant::now();
                let want = h.projects["c"]
                    .live
                    .lock()
                    .unwrap()
                    .rank_files(q, v, 10, fuse)
                    .unwrap();
                ref_t.push(t.elapsed().as_secs_f64());
                same += (got == want) as usize;
                let t = Instant::now();
                let own = h.projects["a"]
                    .live
                    .lock()
                    .unwrap()
                    .rank_files(q, v, 10, fuse)
                    .unwrap();
                own_t.push(t.elapsed().as_secs_f64());
                let _ = own;
            }
            report += &format!(
                "{mode}: same keys and scores {same}/{}; hits outside the scope {outside}; first call {:.1} ms; share p50 {:.2} ms p95 {:.2} ms; reference (c, in process) p50 {:.2} ms; a's own whole-store rank p50 {:.2} ms\n",
                queries.len(),
                first.unwrap() * 1e3,
                pct(share_t.clone(), 0.5),
                pct(share_t, 0.95),
                pct(ref_t, 0.5),
                pct(own_t, 0.5),
            );
        }
        // Whole-store statistics, filtered to the scope: how often the top
        // 10 would be ordered differently, and does a private edit move them?
        let mut differ = 0;
        for q in &queries {
            let mut a = h.projects["a"].live.lock().unwrap();
            let all = a.rank_files(q, None, usize::MAX >> 1, convex).unwrap();
            let filtered: Vec<String> = all
                .iter()
                .filter(|(p, _)| p.starts_with("pub/"))
                .take(10)
                .map(|(p, _)| p.clone())
                .collect();
            drop(a);
            let want: Vec<String> = h.projects["c"]
                .live
                .lock()
                .unwrap()
                .rank_files(q, None, 10, convex)
                .unwrap()
                .into_iter()
                .map(|x| x.0)
                .collect();
            differ += (filtered != want) as usize;
        }
        report += &format!(
            "whole-store BM25 filtered to the scope: top 10 ordered differently for {differ}/{} queries\n",
            queries.len()
        );
        // A private file added: does an in-scope score move under each?
        let q = &queries[0];
        let score = |h: &Host, scoped: bool| -> f32 {
            if scoped {
                let r = h.handle(req(
                    "GET",
                    "b.localhost",
                    OB,
                    &format!("/from/a/pub/rank?q={}", q.replace(' ', "+")),
                    b"",
                ));
                hits(&r.body).1[0].1
            } else {
                let mut a = h.projects["a"].live.lock().unwrap();
                a.rank_files(q, None, usize::MAX >> 1, convex)
                    .unwrap()
                    .into_iter()
                    .find(|x| x.0.starts_with("pub/"))
                    .unwrap()
                    .1
            }
        };
        let (s0, g0) = (score(&h, true), score(&h, false));
        put_files(
            &h,
            "a",
            &[("priv/extra.md", format!("{q} {q} {q}\n").as_bytes())],
        );
        assert_eq!(
            h.handle(req("POST", "a.localhost", OA, issue, b"")).status,
            200
        );
        let (s1, g1) = (score(&h, true), score(&h, false));
        report += &format!(
            "one private file added: best in-scope score through the share {s0} -> {s1}; with whole-store statistics {g0} -> {g1}\n"
        );
        let (terms, postings) = {
            let mut a = h.projects["a"].live.lock().unwrap();
            let share = a.repo.share_named("pub").unwrap().unwrap();
            let files: BTreeMap<String, Id> =
                a.repo.share_files(&share).unwrap().into_iter().collect();
            let t = Instant::now();
            let idx = a.scope_index(&files).unwrap();
            report += &format!(
                "scope index build: {} files in {:.1} ms\n",
                files.len(),
                t.elapsed().as_secs_f64() * 1e3
            );
            idx.size()
        };
        report += &format!(
            "files: {} in a ({} under pub/), load {load:.1} s; scope index {terms} terms, {postings} postings\n",
            files.len() + 3,
            n + 2
        );
        println!("{report}");
        for ext in ["paths", "f32"] {
            let _ = std::fs::remove_file(format!("{vecs}.{ext}"));
        }
        done(dirs);
    }

    /// E363: what a token check and the rate buckets add to a request.
    /// `cargo test --release --bin ikam host::tests::token_overhead --
    /// --ignored --nocapture`.
    #[test]
    #[ignore]
    fn token_overhead() {
        let d = dir("tok-cost");
        let (line, token) = crate::tokens::make().unwrap();
        let (rated, rtoken) = crate::tokens::make().unwrap();
        let text = format!(
            "project a {}\nuser a owner {}\ntoken a read {line}\ntoken a read {rated} rate=1000000/s\n",
            d.display(),
            user("a-owner", "pw")
        );
        let h = Host::open(Config::parse(&text).unwrap()).unwrap();
        {
            let mut l = h.projects["a"].live.lock().unwrap();
            l.apply(vec![live::Op::Put("one.md".into(), b"alpha\n".to_vec())])
                .unwrap();
            l.flush(true).unwrap();
        }
        let n = 20_000;
        let time = |make: &dyn Fn() -> Request, want: u16| {
            let reqs: Vec<Request> = (0..n).map(|_| make()).collect();
            let t = std::time::Instant::now();
            for r in reqs {
                assert_eq!(h.handle(r).status, want);
            }
            t.elapsed().as_secs_f64() * 1e6 / n as f64
        };
        for round in 0..3 {
            let pw = time(&|| req("GET", "a.localhost", OA, "/dav/one.md", b""), 200);
            let tk = time(
                &|| bearer("GET", "a.localhost", &token, "/dav/one.md", b""),
                200,
            );
            let rt = time(
                &|| bearer("GET", "a.localhost", &rtoken, "/dav/one.md", b""),
                200,
            );
            let bad = time(
                &|| bearer("GET", "a.localhost", &token[..20], "/dav/one.md", b""),
                401,
            );
            let mut b = crate::tokens::Bucket::new(
                crate::tokens::Rate::parse("1000000000/s").unwrap(),
                std::time::Instant::now(),
            );
            let t = std::time::Instant::now();
            for _ in 0..n {
                let _ = b.take(std::time::Instant::now());
            }
            let bk = t.elapsed().as_secs_f64() * 1e6 / n as f64;
            println!(
                "round {round}: password {pw:.2} us, token {tk:.2} us, token with a rate {rt:.2} us, refused token {bad:.2} us, one bucket take {bk:.3} us"
            );
        }
        // A burst against a project rate: how fast 429s are answered.
        let text = format!("{text}rate a 100/s\n");
        let h = Host::open(Config::parse(&text).unwrap()).unwrap();
        let t = std::time::Instant::now();
        let (mut ok, mut limited) = (0, 0);
        for _ in 0..n {
            match h
                .handle(bearer("GET", "a.localhost", &token, "/dav/one.md", b""))
                .status
            {
                200 => ok += 1,
                429 => limited += 1,
                s => panic!("{s}"),
            }
        }
        let us = t.elapsed().as_secs_f64() * 1e6 / n as f64;
        println!(
            "burst of {n} at rate 100/s: {ok} answered, {limited} refused, {us:.2} us a request"
        );
        let _ = std::fs::remove_dir_all(d);
    }

    /// E313: what routing through the host adds to a request, against the
    /// same handlers called as `ikam live` calls them. Run with
    /// `cargo test --release --bin ikam host::tests::overhead -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn overhead() {
        let (h, dirs) = host("overhead");
        let n = 20_000;
        let reqs = |host: &str| {
            (0..n)
                .map(|_| req("GET", host, OA, "/dav/pub/one.md", b""))
                .collect::<Vec<_>>()
        };
        let live = h.projects["a"].live.clone();
        for round in 0..3 {
            let direct = reqs("a.localhost");
            let t = std::time::Instant::now();
            for r in &direct {
                assert_eq!(dav::handle(&mut live.lock().unwrap(), r).status, 200);
            }
            let d = t.elapsed().as_secs_f64() * 1e6 / n as f64;
            let hosted = reqs("a.localhost");
            let t = std::time::Instant::now();
            for r in hosted {
                assert_eq!(h.handle(r).status, 200);
            }
            let w = t.elapsed().as_secs_f64() * 1e6 / n as f64;
            let auth = Auth::parse(&user("a-owner", "pw")).unwrap();
            let one = req("GET", "a.localhost", OA, "/", b"");
            let t = std::time::Instant::now();
            for _ in 0..n {
                assert!(auth.check(&one).is_none());
            }
            let a = t.elapsed().as_secs_f64() * 1e6 / n as f64;
            println!(
                "round {round}: direct {d:.2} us, through the host {w:.2} us, one password check {a:.2} us"
            );
        }
        done(dirs);
    }
}
