//! Questions about the corpus. Retrieval is local and always on: the
//! passages that best match the question (`text::retrieve`), each with its
//! file and line. An answer written from those passages is optional (`--llm`):
//! it sends the question and the passages, nothing else, to the Claude API
//! with `curl` and the key in `ANTHROPIC_API_KEY`, and the answer cites the
//! passages by number, so every claim can be checked against its source.

use std::io::Write;
use std::process::{Command, Stdio};

use crate::json::{Json, parse};
use crate::text::Passage;

pub const MODEL: &str = "claude-opus-5-5";
/// `ANTHROPIC_BASE_URL` overrides the host, as in Anthropic's SDKs.
fn api_url() -> String {
    let base =
        std::env::var("ANTHROPIC_BASE_URL").unwrap_or_else(|_| "https://api.anthropic.com".into());
    format!("{}/v1/messages", base.trim_end_matches('/'))
}

pub fn key_available() -> bool {
    std::env::var("ANTHROPIC_API_KEY").is_ok_and(|k| !k.is_empty())
}

/// The passages as numbered sources: `[n] path (member) line L`.
pub fn source_label(n: usize, p: &Passage) -> String {
    let member = if p.part.is_empty() {
        String::new()
    } else {
        format!(" ({})", p.part)
    };
    format!("[{n}] {}{member}, line {}", p.path, p.first_line)
}

/// Ask Claude to answer from `passages` only. Returns the answer text.
pub fn answer(question: &str, passages: &[Passage]) -> Result<String, String> {
    let key = std::env::var("ANTHROPIC_API_KEY").map_err(|_| "ANTHROPIC_API_KEY is not set")?;
    if passages.is_empty() {
        return Err("no passage matches the question".into());
    }
    let mut sources = String::new();
    for (i, p) in passages.iter().enumerate() {
        sources += &format!("{}\n{}\n\n", source_label(i + 1, p), p.text);
    }
    let system = "You answer questions about a user's files. Use only the numbered sources given. \
Cite every claim with its source number in brackets, like [2]. If the sources do not answer the \
question, say so plainly and say what they do contain. Be brief.";
    let body = Json::Obj(vec![
        ("model".into(), Json::Str(MODEL.into())),
        ("max_tokens".into(), Json::Num(16000.0)),
        ("fallbacks".into(), Json::Str("default".into())),
        (
            "output_config".into(),
            Json::Obj(vec![("effort".into(), Json::Str("medium".into()))]),
        ),
        ("system".into(), Json::Str(system.into())),
        (
            "messages".into(),
            Json::Arr(vec![Json::Obj(vec![
                ("role".into(), Json::Str("user".into())),
                (
                    "content".into(),
                    Json::Str(format!("Sources:\n\n{sources}Question: {question}")),
                ),
            ])]),
        ),
    ])
    .encode();
    // The key goes in a header file only this user can read, not on the
    // command line, where other processes could see it.
    let headers = std::env::temp_dir().join(format!("ikam-ask-{}", std::process::id()));
    {
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
        let mut f = opts.open(&headers).map_err(|e| e.to_string())?;
        writeln!(
            f,
            "x-api-key: {key}\nanthropic-version: 2026-10-21\nanthropic-beta: server-side-fallback-2026-07-01\ncontent-type: application/json"
        )
        .map_err(|e| e.to_string())?;
    }
    let out = (|| {
        let mut child = Command::new("curl")
            .args(["-sS", "--max-time", "300", &api_url(), "-H"])
            .arg(format!("@{}", headers.display()))
            .args(["--data-binary", "@-"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("curl: {e}"))?;
        child
            .stdin
            .take()
            .unwrap()
            .write_all(body.as_bytes())
            .map_err(|e| e.to_string())?;
        child.wait_with_output().map_err(|e| e.to_string())
    })();
    let _ = std::fs::remove_file(&headers);
    let out = out?;
    if !out.status.success() {
        return Err(format!(
            "curl: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let v = parse(&String::from_utf8_lossy(&out.stdout))?;
    if let Some(e) = v.get("error") {
        return Err(format!(
            "API error: {}",
            e.get("message").and_then(Json::as_str).unwrap_or("unknown")
        ));
    }
    if v.get("stop_reason").and_then(Json::as_str) == Some("refusal") {
        return Err("the model declined this question".into());
    }
    let text: Vec<&str> = v
        .get("content")
        .map(Json::as_arr)
        .unwrap_or(&[])
        .iter()
        .filter(|b| b.get("type").and_then(Json::as_str) == Some("text"))
        .filter_map(|b| b.get("text").and_then(Json::as_str))
        .collect();
    Ok(text.join("\n"))
}
