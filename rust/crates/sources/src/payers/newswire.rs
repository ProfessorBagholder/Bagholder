//! Cision's newswire.ca (research 2): the channel every company release reached
//! (`/CNW/`). An organization's page (`newswire.ca/news/<organization>/`) lists
//! its releases, newest first, each a link of class `newsreleaseconsolidatelink`
//! with its time and title; a release carries its moment (`<meta name='date'>`)
//! and its body (`<section class="release-body">`), whose tables each follow a
//! heading.

use bagholder_core::jiff::Timestamp;

use crate::html::{html_tables, unescape};
use crate::reply::Mismatch;

pub const HOST: &str = "www.newswire.ca";

/// One release an organization's page lists.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listed {
    pub title: String,
    /// The path on newswire.ca (`/news-releases/…html`).
    pub path: String,
}

fn text(html: &str) -> String {
    let mut out = String::new();
    let mut inside = false;
    for c in html.chars() {
        match c {
            '<' => inside = true,
            '>' => {
                inside = false;
                out.push(' ');
            }
            c if !inside => out.push(c),
            _ => {}
        }
    }
    unescape(&out).split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The releases an organization's page lists, newest first.
pub fn listed(html: &str) -> Result<Vec<Listed>, Mismatch> {
    let mut out: Vec<Listed> = Vec::new();
    for part in html.split("class=\"newsreleaseconsolidatelink").skip(1) {
        let Some(h) = part.find("href=\"") else { continue };
        let rest = &part[h + 6..];
        let Some(end) = rest.find('"') else { continue };
        let path = rest[..end].to_string();
        let Some(close) = rest.find("</a>") else { continue };
        let body = &rest[end..close];
        // the title is the link's heading; the time and the lead are beside it
        let heading = match (body.find("<h3"), body.find("</h3>")) {
            (Some(a), Some(b)) if a < b => &body[a..b],
            _ => body,
        };
        // the heading leads with the release's time in a <small>; the title follows
        let title = match (heading.find("<small"), heading.find("</small>")) {
            (Some(a), Some(b)) if a < b => text(&format!("{}{}", &heading[..a], &heading[b + "</small>".len()..])),
            _ => text(heading),
        };
        if !out.iter().any(|l| l.path == path) {
            out.push(Listed { title, path });
        }
    }
    if out.is_empty() {
        return Err(Mismatch { path: "a.newsreleaseconsolidatelink".into(), why: "the organization's page lists no releases".into() });
    }
    Ok(out)
}

/// A release's moment and its body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    pub at: Timestamp,
    /// The body as text, whitespace collapsed.
    pub body: String,
    /// Each table in the body with the heading nearest before it.
    pub tables: Vec<(String, Vec<Vec<String>>)>,
}

pub fn release(html: &str) -> Result<Release, Mismatch> {
    let m = |path: &str, why: &str| Mismatch { path: path.into(), why: why.into() };
    let meta = html.find("<meta name='date'").ok_or_else(|| m("meta[name=date]", "the release states no date"))?;
    let content = &html[meta..];
    let c = content.find("content=\"").ok_or_else(|| m("meta[name=date]", "the release states no date"))? + 9;
    let end = content[c..].find('"').ok_or_else(|| m("meta[name=date]", "the release states no date"))? + c;
    let at: Timestamp = content[c..end].parse().map_err(|_| m("meta[name=date]", &format!("{:?} is not an instant", &content[c..end])))?;
    let start = html.find("release-body").ok_or_else(|| m("section.release-body", "the release has no body"))?;
    let body_html = &html[start..];
    let body_end = body_html.find("</section>").unwrap_or(body_html.len());
    let body_html = &body_html[..body_end];
    let mut tables = Vec::new();
    let mut from = 0;
    while let Some(t) = body_html[from..].find("<table") {
        let open = from + t;
        let Some(close) = body_html[open..].find("</table>") else { break };
        let close = open + close + "</table>".len();
        // the heading: the text since the table before, from its last `Table `
        // (`Table II – Monthly Distributions`) where it names one
        let before = text(&body_html[from..open]);
        let heading = before.rfind("Table ").map_or(before.as_str(), |i| &before[i..]).to_string();
        for table in html_tables(&body_html[open..close]) {
            tables.push((heading.clone(), table));
        }
        from = close;
    }
    Ok(Release { at, body: text(body_html), tables })
}
