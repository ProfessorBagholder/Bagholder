//! The person's tax slips from Wealthsimple's Documents (brief 82, PR 1): the
//! documents list, its tax category, asked as the Documents page asks it
//! (`FetchIdentityPaginatedDocuments`), and each slip's file opened as the page
//! opens it: by the document's own link where the list states one, else by the
//! signed link for its stored file (`DocumentSignedUrlCreate`). Every one of the
//! person's slips for 2023 to 2025 came by a signed link (read 2026-11-12).
//!
//! A document is read once: its file is fixed, and a slip Wealthsimple amends is
//! a document of its own. What identifies the person is taken out of a slip's text
//! before it is kept (`slips::redact`). Its text is laid out by the caller (`text_of`: the
//! server's `pdftotext -layout`, page by page), so this crate runs no program.

use std::collections::BTreeSet;

use bagholder_broker::{Answer, Failure};
use bagholder_core::json::Value;
use bagholder_sources::reply::{Mismatch, Node};

/// The slips read: the kinds `slips` has a reader for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SlipKind {
    T3,
    T5,
    T5008,
}

impl SlipKind {
    pub const ALL: [SlipKind; 3] = [SlipKind::T3, SlipKind::T5, SlipKind::T5008];

    /// The list's `type` for it, as Wealthsimple states it.
    pub fn name(self) -> &'static str {
        match self {
            SlipKind::T3 => "t3",
            SlipKind::T5 => "t5",
            SlipKind::T5008 => "t5008",
        }
    }

    /// The kind a list's `type` names, where it is one read.
    pub fn of(name: &str) -> Option<SlipKind> {
        SlipKind::ALL.into_iter().find(|k| k.name() == name)
    }
}

/// Where a document's file is read from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Link {
    /// The link the list states with it.
    Own(String),
    /// The file Wealthsimple stores it in, opened by a link it signs.
    Stored { bucket: Option<String>, key: String },
}

/// One slip as the list states it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listed {
    /// Wealthsimple's id for the document.
    pub id: String,
    pub kind: SlipKind,
    /// Wealthsimple's id for the account it was issued for.
    pub account: String,
    /// The tax year: the year of its period.
    pub year: i16,
    /// When Wealthsimple made it (`createdAt`), as it states it: a later one for
    /// an account, kind and year is the slip amended.
    pub created: String,
    pub link: Link,
}

/// A date or an instant as the list writes one (`2025-02-20`, `2025-02-20T14:03:11.000Z`):
/// texts of one shape sort as time.
fn instant_text(s: &str) -> bool {
    let b = s.as_bytes();
    let day = b.len() >= 10 && b[4] == b'-' && b[7] == b'-' && b[..10].iter().enumerate().all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit());
    day && (b.len() == 10 || matches!(b[10], b'T' | b' '))
}

/// A document of the list read as a slip: `Ok(None)` for a kind not read (the
/// fee summaries, the contribution receipts and every other document the tax
/// category holds), an error for a slip whose entry cannot be read.
pub fn listed(doc: &Node) -> Result<Option<Listed>, Mismatch> {
    let Some(kind) = SlipKind::of(&doc.text("type")?.to_ascii_lowercase()) else { return Ok(None) };
    let id = doc.text("id")?.to_string();
    let account = doc.obj("account")?.text("id")?.to_string();
    let period = doc.day("period")?;
    let created = doc.text("createdAt")?;
    if !instant_text(created) {
        return Err(doc.field("createdAt")?.mismatch(format!("expected a date or an instant, found {created:?}")));
    }
    let link = match doc.opt_text("downloadUrl")? {
        Some(url) if !url.is_empty() => Link::Own(url.to_string()),
        _ => Link::Stored { bucket: doc.opt_text("s3BucketName")?.filter(|b| !b.is_empty()).map(str::to_string), key: doc.text("s3Key")?.to_string() },
    };
    Ok(Some(Listed { id, kind, account, year: period.year(), created: created.to_string(), link }))
}

/// Wealthsimple's Documents, as a slip read asks them.
pub trait Documents {
    /// Every entry of the documents list, tax category, every page of it.
    fn tax_documents(&mut self) -> Answer<Vec<Value>>;
    /// The link Wealthsimple signs for a stored file.
    fn signed_link(&mut self, bucket: Option<&str>, key: &str) -> Answer<String>;
    /// A file's bytes, from a link that carries its own permission.
    fn download(&mut self, url: &str) -> Answer<Vec<u8>>;
}

/// A slip read this time, its text as `text_of` laid it out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fetched {
    pub slip: Listed,
    pub text: String,
}

/// What one read of the list found.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Found {
    /// Each slip not read before, read now.
    pub fetched: Vec<Fetched>,
    /// Each slip that could not be read this time, and why: read again next time.
    pub unread: Vec<(String, String)>,
}

/// Each slip the list states that `kept` does not hold, its file read and laid out
/// by `text_of` (None: the bytes have no text, and the slip is read again next
/// time). The list failing is the read failing; one slip failing is that slip's.
pub fn read_new(src: &mut dyn Documents, kept: &BTreeSet<String>, text_of: &dyn Fn(&[u8]) -> Option<String>) -> Answer<Found> {
    let entries = src.tax_documents()?;
    let mut found = Found::default();
    for entry in &entries {
        let node = Node::root(entry);
        let slip = match listed(&node) {
            Ok(Some(s)) => s,
            Ok(None) => continue,
            Err(m) => {
                let id = node.text("id").unwrap_or("a document with no id").to_string();
                found.unread.push((id, format!("FetchIdentityPaginatedDocuments: {m}")));
                continue;
            }
        };
        if kept.contains(&slip.id) {
            continue;
        }
        let url = match &slip.link {
            Link::Own(u) => Ok(u.clone()),
            Link::Stored { bucket, key } => src.signed_link(bucket.as_deref(), key),
        };
        let bytes = url.and_then(|u| src.download(&u));
        match bytes {
            Ok(b) => match text_of(&b) {
                Some(text) => found.fetched.push(Fetched { slip, text }),
                None => found.unread.push((slip.id, format!("its file ({} bytes) has no text that can be read", b.len()))),
            },
            // a lapsed sign-in is the whole read's: nothing more is asked with it
            Err(f @ Failure::Lapsed(_)) => return Err(f),
            Err(f) => found.unread.push((slip.id, f.to_string())),
        }
    }
    Ok(found)
}

/// What one read of the list kept.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Kept {
    /// Each slip read and kept this time, with what its reader made of it: a slip
    /// refused on its own arithmetic is kept as it came all the same, so a reader
    /// put right reads it without asking again.
    pub new: Vec<(Listed, Result<(), String>)>,
    /// Each slip that could not be read this time, and why.
    pub unread: Vec<(String, String)>,
}

/// The broker read a list of the slips is noted as.
pub const READ_PART: &str = "tax slips";

/// The first day of a year the list is read again for the year just ended, and the
/// day from which that year's slips are all in: CRA's last day to issue a T5 or a
/// T5008 is the last day of February, and a T3 the 31st of March, 90 days after a
/// trust's year end (brief 82: past years once; the year just ended once a day from
/// February until its slips are all in, then none).
pub const READ_FROM: (i8, i8) = (2, 1);
pub const ALL_IN_FROM: (i8, i8) = (4, 1);

/// Whether the list is due today, its last read on `last` (both days in
/// Wealthsimple's filing zone): never read; else once a day from 1 February until
/// a read on or after 1 April, then not until the next February.
pub fn list_due(last: Option<jiff::civil::Date>, today: jiff::civil::Date) -> bool {
    let Some(last) = last else { return true };
    let on = |(m, d): (i8, i8)| jiff::civil::date(today.year(), m, d);
    today >= on(READ_FROM) && last < today && last < on(ALL_IN_FROM)
}

/// One read of the list: each slip not kept before read, what identifies the
/// person taken out of it (`slips::redact`), and kept in `book`; the read noted
/// (`READ_PART`) once the list has answered.
pub fn keep_new(book: &bagholder_book::Book, connection: bagholder_core::ConnectionId, src: &mut dyn Documents, text_of: &dyn Fn(&[u8]) -> Option<String>, now: jiff::Timestamp) -> Result<Kept, String> {
    keep_new_as(book, connection, src, text_of, crate::slips::REDACTOR, now)
}

/// [`keep_new`] under the redaction version `redactor`: a slip kept without its
/// text by another version is read again.
pub fn keep_new_as(book: &bagholder_book::Book, connection: bagholder_core::ConnectionId, src: &mut dyn Documents, text_of: &dyn Fn(&[u8]) -> Option<String>, redactor: i64, now: jiff::Timestamp) -> Result<Kept, String> {
    let kept = book.tax_slip_documents(connection, redactor).map_err(|e| e.to_string())?;
    let found = read_new(src, &kept, text_of).map_err(|f| f.to_string())?;
    let read = book.broker_read(connection, READ_PART, now).map_err(|e| e.to_string())?;
    let mut out = Kept { new: Vec::new(), unread: found.unread };
    for f in found.fetched {
        // what identifies the person is taken out before anything is kept, and the
        // slip is read from what is kept; one it cannot be taken out of is kept with
        // no text, so it is not fetched again until the redaction changes
        // (`SPEC.md` tax-slips-5)
        let redacted = crate::slips::redact(&f.text);
        let slip = bagholder_book::tax_slips::TaxSlip {
            document: f.slip.id.clone(),
            account_key: f.slip.account.clone(),
            kind: f.slip.kind.name().to_string(),
            year: f.slip.year,
            created: f.slip.created.clone(),
            text: redacted.as_ref().ok().cloned(),
        };
        book.keep_tax_slip(connection, &slip, redactor, &read).map_err(|e| e.to_string())?;
        let reading = redacted.map_err(|why| format!("kept without its text, as what identifies the person could not be taken out of it: {why}")).and_then(|text| crate::slips::read(f.slip.kind, &text)).and_then(|s| {
            if s.year() == f.slip.year { Ok(()) } else { Err(format!("the slip states the year {}, its document {}", s.year(), f.slip.year)) }
        });
        out.new.push((f.slip, reading));
    }
    Ok(out)
}
