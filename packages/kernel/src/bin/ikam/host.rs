//! `ikam host --config FILE [PORT]`: one process serving several separate
//! projects (E313, E314). Each project is its own store, opened as `ikam
//! live` opens one, with its own users, limits and share key; nothing is
//! shared between stores. A project is chosen by the request's Host name
//! (the names in the config, and `NAME.localhost`), never by path, because
//! the pages and WebDAV answers of `ikam live` use absolute links.
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
//!
//! A panic while serving one project is caught and answered 500; the other
//! projects keep serving. `--public` is refused: every request needs a user.

use std::collections::BTreeMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use ikam_kernel::share::{Refused, Right, Scope, Share, import_share};
use ikam_kernel::{Id, Kind, merge::Merged};

use crate::auth::{self, Auth};
use crate::http::{self, Request, Response};
use crate::json::Json;
use crate::live::{self, Live};
use crate::{dav, ui};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Role {
    Read,
    Write,
    Owner,
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
}

pub struct Config {
    pub port: u16,
    pub projects: Vec<ProjectConfig>,
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
    /// BYTES`, `max-body NAME BYTES`; `#` starts a comment.
    pub fn parse(text: &str) -> Result<Config, String> {
        let mut port = 8750;
        let mut projects: Vec<ProjectConfig> = Vec::new();
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
                ["project", name, store, rest @ ..] if rest.len() <= 1 => {
                    if !valid_name(name) {
                        return Err(bad("a project name is a-z, 0-9 and -"));
                    }
                    if projects.iter().any(|p| p.name == *name) {
                        return Err(bad(&format!("project {name} declared twice")));
                    }
                    projects.push(ProjectConfig {
                        name: name.to_string(),
                        store: PathBuf::from(store),
                        refname: rest.first().unwrap_or(&"main").to_string(),
                        hosts: Vec::new(),
                        users: Vec::new(),
                        max_store: None,
                        max_body: None,
                    });
                }
                ["host", name, host] => {
                    let i = project(&mut projects, name)?;
                    let host = host.to_ascii_lowercase();
                    if host.is_empty() || host.contains(['/', ':', '@']) {
                        return Err(bad("host NAME HOSTNAME, a name without a port"));
                    }
                    projects[i].hosts.push(host);
                }
                ["user", name, role, line] => {
                    let i = project(&mut projects, name)?;
                    let role = match *role {
                        "read" => Role::Read,
                        "write" => Role::Write,
                        "owner" => Role::Owner,
                        _ => return Err(bad("a role is read, write or owner")),
                    };
                    Auth::parse(line).map_err(|e| bad(&e))?;
                    projects[i].users.push((role, line.to_string()));
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
        Ok(Config { port, projects })
    }
}

pub struct Project {
    pub name: String,
    pub live: Arc<Mutex<Live>>,
    users: Vec<(Role, Auth)>,
    hosts: Vec<String>,
    max_body: Option<usize>,
    key: [u8; 32],
}

pub struct Host {
    pub projects: BTreeMap<String, Project>,
    port: u16,
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
            let key = share_key(&p.store)?;
            let users = p
                .users
                .iter()
                .map(|(r, line)| Auth::parse(line).map(|a| (*r, a)))
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
                },
            );
        }
        Ok(Host {
            projects,
            port: config.port,
        })
    }

    /// The project a Host header names.
    fn project(&self, host: Option<&str>) -> Option<&Project> {
        let host = host?.to_ascii_lowercase();
        let name = match host.rsplit_once(':') {
            Some((n, p)) if p.bytes().all(|b| b.is_ascii_digit()) => n.to_string(),
            _ => host,
        };
        self.projects.values().find(|p| p.hosts.contains(&name))
    }

    pub fn handle(&self, req: Request) -> Response {
        let Some(p) = self.project(req.header("host")) else {
            return Response::text(404, "no such project");
        };
        if req.path == "/robots.txt" {
            return Response::text(200, "User-agent: *\nDisallow: /\n");
        }
        let role = p
            .users
            .iter()
            .filter(|(_, a)| a.check(&req).is_none())
            .map(|(r, _)| *r)
            .max();
        let Some(role) = role else {
            return Response::text(401, "password required").with(
                "WWW-Authenticate",
                "Basic realm=\"ikam\", charset=\"UTF-8\"",
            );
        };
        if p.max_body.is_some_and(|m| req.body.len() > m) {
            return Response::text(413, "body too large for this project");
        }
        match catch_unwind(AssertUnwindSafe(|| self.route(p, role, &req))) {
            Ok(r) => r,
            Err(_) => {
                eprintln!("{} {}: request failed", live::stamp(), p.name);
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
        if crate::writes(req) && role < Role::Write {
            return Response::text(403, "this user may only read");
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
        let out = l.repo.merge(&into, &branch, &msg);
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
        let repo = &mut sl.repo;
        let refused = |e: Refused| match e {
            Refused::Store(e) => Response::text(500, e),
            _ => no_share(),
        };
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
        let msg = format!("merge {name} from {}", s.name);
        let out = import_share(&mut sl.repo, &share, &mut pl.repo, &into, &msg);
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

/// `ikam host --config FILE [PORT]`: serve until stopped.
pub fn host_cmd(args: &[String]) -> Result<(), String> {
    let mut args = args.to_vec();
    if args.iter().any(|a| a == "--public") {
        return Err("ikam host has no --public: every request needs a project's user".into());
    }
    let file =
        crate::take_flag(&mut args, "--config").ok_or("usage: ikam host --config FILE [PORT]")?;
    let text = std::fs::read_to_string(&file).map_err(|e| format!("{file}: {e}"))?;
    let mut config = Config::parse(&text)?;
    if let Some(p) = args.first() {
        config.port = p.parse().map_err(|_| "PORT must be a number")?;
    }
    let max_body = config
        .projects
        .iter()
        .map(|p| p.max_body)
        .collect::<Option<Vec<_>>>()
        .and_then(|v| v.into_iter().max());
    let host = Arc::new(Host::open(config)?);
    auth::catch_stop_signals();
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
                        eprintln!("{} {}: commit failed: {e}", live::stamp(), p.name);
                        failed = true;
                    }
                }
                if stopping {
                    eprintln!("{} stopped", live::stamp());
                    std::process::exit(failed as i32);
                }
            }
        });
    }
    let t = live::stamp();
    for p in host.projects.values() {
        eprintln!("{t} project {}: {}", p.name, p.hosts.join(", "));
    }
    eprintln!("{t} host on 127.0.0.1:{}", host.port);
    let limits = http::Limits {
        max_body,
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
