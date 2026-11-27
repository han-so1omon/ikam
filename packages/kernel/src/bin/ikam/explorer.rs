//! E440: the explorer of `ikam host`: one page listing the projects a
//! caller may open, each with its links (the running project, its demos,
//! anything the config names), and one page per project. Host names given
//! with `explorer HOSTNAME` answer with it; `/p/NAME/` there serves the
//! project itself, so one address (such as one Tailscale Serve port) reaches them
//! all. What a project shows comes from its `title`, `about` and `link`
//! lines in the host's config.

use crate::host::{Link, Project, Role};
use crate::http::{Response, esc};
use crate::serve;

fn page(title: &str, body: &str) -> Response {
    Response::html(format!(
        "<!doctype html><meta charset=utf-8><meta name=viewport content=\"width=device-width\">\
<title>{t}</title><style>{}li{{margin:.2rem 0}}.g{{color:#777;font-size:.9em}}</style>\
<nav><a href=\"/\">projects</a></nav><h1>{t}</h1>{body}",
        serve::STYLE,
        t = esc(title),
    ))
}

fn title(p: &Project) -> &str {
    p.title.as_deref().unwrap_or(&p.name)
}

/// Where a link goes: URLs and `/paths` as given, other paths inside the
/// project (under `/p/NAME/`).
fn href(p: &Project, l: &Link) -> String {
    let u = l.url.as_str();
    let lower = u.to_ascii_lowercase();
    if u.starts_with('/') || lower.starts_with("https://") || lower.starts_with("http://") {
        u.to_string()
    } else {
        format!("/p/{}/{u}", p.name)
    }
}

fn a(href: &str, label: &str) -> String {
    format!("<a href=\"{}\">{}</a>", esc(href), esc(label))
}

/// The groups of a project's links, in the order each group first comes.
fn groups(p: &Project) -> Vec<(&str, Vec<&Link>)> {
    let mut out: Vec<(&str, Vec<&Link>)> = Vec::new();
    for l in &p.links {
        match out.iter_mut().find(|(g, _)| *g == l.group) {
            Some((_, ls)) => ls.push(l),
            None => out.push((&l.group, vec![l])),
        }
    }
    out
}

/// `/`: every project the caller opens, with its links.
pub fn index(open: &[(&Project, Role)]) -> Response {
    let mut body = String::from("<table><tr><th>Project</th><th>Links</th></tr>");
    for (p, _) in open {
        let mut links = vec![a(&format!("/p/{}/", p.name), "files")];
        for (g, ls) in groups(p) {
            let ls: Vec<String> = ls.iter().map(|l| a(&href(p, l), &l.label)).collect();
            links.push(format!(
                "<span class=g>{}:</span> {}",
                esc(g),
                ls.join(", ")
            ));
        }
        body += &format!(
            "<tr><td>{}<br><span class=s>{}</span></td><td>{}</td></tr>",
            a(&format!("/projects/{}", p.name), title(p)),
            esc(p.about.as_deref().unwrap_or("")),
            links.join(" · ")
        );
    }
    body += "</table>";
    page("Projects", &body)
}

/// `/projects/NAME`: one project's page.
pub fn project(p: &Project, role: Role) -> Response {
    let files = format!("/p/{}/", p.name);
    let mut body = match &p.about {
        Some(t) => format!("<p>{}</p>", esc(t)),
        None => String::new(),
    };
    body += &format!(
        "<p class=bar>{} · {} · <span class=s>your role: {}</span></p>\
<form action=\"{}\"><input name=q placeholder=\"search this project\"></form>",
        a(&files, "files"),
        a(&format!("{files}log"), "changes"),
        format!("{role:?}").to_lowercase(),
        esc(&format!("{files}search")),
    );
    for (g, ls) in groups(p) {
        body += &format!("<h2>{}</h2><ul>", esc(g));
        for l in ls {
            body += &format!("<li>{}</li>", a(&href(p, l), &l.label));
        }
        body += "</ul>";
    }
    page(title(p), &body)
}
