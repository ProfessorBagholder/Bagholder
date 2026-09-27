//! What the person follows (`docs/architecture.md` §6; `docs/plans/stage-5-interface-and-running.md`,
//! A2): the watched listings and the Markets tab's tile row. Each row is an
//! instrument of the book, found as §5 finds any instrument: by the instrument
//! the caller already found, else by a reference that identifies it, else made
//! new; never by a bare symbol here.

use rusqlite::{params, OptionalExtension};

use bagholder_core::instrument::{InstrumentKind, Reference};
use bagholder_core::{Currency, InstrumentId};

use crate::text::{self, at as at_text};
use crate::{new_uuid, Book, BookError, Result};

/// Whether the tile row was ever chosen: never chosen is the default row, chosen
/// empty is empty.
const TILES_CHOSEN: &str = "tiles.chosen";
/// Whether the earlier store's watchlist and tiles were carried into the book.
const CARRIED: &str = "following.carried";

/// What an instrument is called, as the person picked it where no record names it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListingName {
    pub symbol: String,
    /// The venue's market identifier code (ISO 10383), where known.
    pub venue_mic: Option<String>,
    /// The venue in words (`TSX-V`, `Index`, `CME`).
    pub venue_name: Option<String>,
    pub name: Option<String>,
}

/// A listing to follow, as the caller found it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListingDraft {
    /// The instrument it is, where the caller already knows (the page named it,
    /// or a listing of the book is called this now).
    pub found: Option<InstrumentId>,
    pub kind: InstrumentKind,
    pub currency: Currency,
    /// Its references: the identifying ones find it; each it does not hold yet is added.
    pub refs: Vec<Reference>,
    pub name: ListingName,
}

/// A watched listing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Watched {
    pub instrument: InstrumentId,
    pub added_at: jiff::Timestamp,
}

impl Book {
    /// The instrument `draft` is: the one its identifying references name
    /// themselves (`Book::own_instrument`), else the one found, else a new one
    /// with them; kept as named, so a succession parting two instruments again
    /// leaves the choice on the listing picked. A kind or currency other than
    /// the instrument's, or references naming two instruments, is refused.
    fn listing(&self, draft: &ListingDraft, at: jiff::Timestamp) -> Result<InstrumentId> {
        let mut named: Option<InstrumentId> = None;
        for r in draft.refs.iter().filter(|r| r.identifies()) {
            if let Some(id) = self.own_instrument(r)? {
                match named {
                    Some(n) if self.canonical(n)? != self.canonical(id)? => {
                        return Err(BookError::Refused(format!("{} {} names another instrument than {}", r.scheme, r.value, draft.name.symbol)));
                    }
                    Some(_) => {}
                    None => named = Some(id),
                }
            }
        }
        match (named, draft.found) {
            (Some(n), Some(f)) if self.canonical(n)? != self.canonical(f)? => {
                return Err(BookError::Refused(format!("{}'s references name another instrument than {f}", draft.name.symbol)));
            }
            (None, found) => named = found,
            _ => {}
        }
        let id = match named {
            Some(id) => {
                let held = self.instrument(id)?;
                if held.kind != draft.kind || held.currency != draft.currency {
                    return Err(BookError::Refused(format!(
                        "{} is a {} in {}, not a {} in {}",
                        draft.name.symbol, held.kind, held.currency, draft.kind, draft.currency
                    )));
                }
                id
            }
            None => {
                let id = InstrumentId::from_uuid(new_uuid(at));
                self.conn().execute(
                    "INSERT INTO instruments(id, kind, currency, created_at) VALUES (?, ?, ?, ?)",
                    params![id.to_string(), draft.kind.as_str(), draft.currency.as_str(), at_text(at)],
                )?;
                id
            }
        };
        for r in &draft.refs {
            if self.own_instrument(r)?.is_none() {
                self.add_instrument_ref(id, r)?;
            }
        }
        // what the person picked it as, which names it where no record does (a
        // record's name stands before it; the first pick stays)
        if self.listing_named(id)?.is_none() {
            let n = &draft.name;
            self.conn().execute(
                "INSERT INTO listings_named(instrument_id, symbol, venue_mic, venue_name, name, named_at) VALUES (?, ?, ?, ?, ?, ?)",
                params![id.to_string(), n.symbol, n.venue_mic, n.venue_name, n.name, at_text(at)],
            )?;
        }
        Ok(id)
    }

    /// What the person picked an instrument as, where no record names it: its
    /// own pick, else one of an instrument read as one with it.
    pub fn listing_named(&self, id: InstrumentId) -> Result<Option<ListingName>> {
        let head = self.canonical(id)?;
        let sql = format!(
            "SELECT symbol, venue_mic, venue_name, name FROM listings_named WHERE instrument_id IN {} ORDER BY instrument_id = ?1 DESC, named_at, instrument_id LIMIT 1",
            crate::identity::group_sql("?1")
        );
        Ok(self
            .conn()
            .query_row(&sql, [head.to_string()], |r| Ok(ListingName { symbol: r.get(0)?, venue_mic: r.get(1)?, venue_name: r.get(2)?, name: r.get(3)? }))
            .optional()?)
    }

    /// What the instrument is called now: its records' latest name, else what the
    /// person picked it as.
    pub fn current_name(&self, id: InstrumentId) -> Result<Option<ListingName>> {
        if let Some(n) = self.names(id)?.pop() {
            return Ok(Some(ListingName { symbol: n.symbol, venue_mic: n.venue_mic, venue_name: n.venue_name, name: n.name }));
        }
        self.listing_named(id)
    }

    // ------------------------------------------------------------------
    // the watchlist
    // ------------------------------------------------------------------

    /// The watched listings, newest first.
    pub fn watched(&self) -> Result<Vec<Watched>> {
        let mut stmt = self.conn().prepare_cached("SELECT instrument_id, added_at FROM watched ORDER BY added_at DESC, rowid DESC")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        let joined = self.joined()?;
        let mut out: Vec<Watched> = Vec::new();
        for row in rows {
            let (id, added) = row?;
            let id = text::parsed("watched", "instrument_id", &id, InstrumentId::parse)?;
            // instruments read as one are one watched listing, from its newest add
            let instrument = joined.get(&id).copied().unwrap_or(id);
            if !out.iter().any(|w| w.instrument == instrument) {
                out.push(Watched { instrument, added_at: text::instant("watched", "added_at", &added)? });
            }
        }
        Ok(out)
    }

    /// Follow `draft`'s listing from `at`: its instrument, as the book reads it.
    /// One already followed keeps when it was added.
    pub fn watch(&self, draft: &ListingDraft, at: jiff::Timestamp) -> Result<InstrumentId> {
        self.atomically(|| {
            let id = self.listing(draft, at)?;
            self.conn().execute("INSERT OR IGNORE INTO watched(instrument_id, added_at) VALUES (?, ?)", params![id.to_string(), at_text(at)])?;
            self.canonical(id)
        })
    }

    /// Stop following an instrument (and every one read as one with it);
    /// whether it was followed.
    pub fn unwatch(&self, id: InstrumentId) -> Result<bool> {
        let head = self.canonical(id)?;
        let sql = format!("DELETE FROM watched WHERE instrument_id IN {}", crate::identity::group_sql("?1"));
        Ok(self.conn().execute(&sql, [head.to_string()])? > 0)
    }

    // ------------------------------------------------------------------
    // the tile row
    // ------------------------------------------------------------------

    /// The tile row in its order, or none when it was never chosen.
    pub fn tiles(&self) -> Result<Option<Vec<InstrumentId>>> {
        if self.setting(TILES_CHOSEN)?.is_none() {
            return Ok(None);
        }
        let joined = self.joined()?;
        let mut stmt = self.conn().prepare_cached("SELECT instrument_id FROM tiles ORDER BY position")?;
        let ids = stmt.query_map([], |r| r.get::<_, String>(0))?;
        // each as the book reads it; instruments read as one are one tile, where it first stands
        let mut out: Vec<InstrumentId> = Vec::new();
        for s in ids {
            let id = text::parsed("tiles", "instrument_id", &s?, InstrumentId::parse)?;
            let id = joined.get(&id).copied().unwrap_or(id);
            if !out.contains(&id) {
                out.push(id);
            }
        }
        Ok(Some(out))
    }

    /// The tile row is `drafts`, in their order: their instruments.
    pub fn set_tiles(&self, drafts: &[ListingDraft], at: jiff::Timestamp) -> Result<Vec<InstrumentId>> {
        self.atomically(|| {
            let mut ids: Vec<(InstrumentId, InstrumentId)> = Vec::new();
            for d in drafts {
                let id = self.listing(d, at)?;
                let read = self.canonical(id)?;
                if ids.iter().any(|(_, r)| *r == read) {
                    return Err(BookError::Refused(format!("{} is on the tile row twice", d.name.symbol)));
                }
                ids.push((id, read));
            }
            self.conn().execute("DELETE FROM tiles", [])?;
            for (i, (id, _)) in ids.iter().enumerate() {
                self.conn().execute("INSERT INTO tiles(instrument_id, position) VALUES (?, ?)", params![id.to_string(), i as i64])?;
            }
            self.set_setting(TILES_CHOSEN, Some("true"), at)?;
            Ok(ids.into_iter().map(|(_, read)| read).collect())
        })
    }

    // ------------------------------------------------------------------
    // the earlier store's rows
    // ------------------------------------------------------------------

    /// Whether the earlier store's watchlist and tiles were carried in.
    pub fn following_carried(&self) -> Result<bool> {
        Ok(self.setting(CARRIED)?.is_some())
    }

    /// Carry the earlier store's watchlist (each with when it was added) and its
    /// tile row (none when it was never chosen) into the book, once, as one
    /// transaction: all of it or none.
    pub fn carry_following(&self, watched: &[(ListingDraft, jiff::Timestamp)], tiles: Option<&[ListingDraft]>, at: jiff::Timestamp) -> Result<()> {
        self.atomically(|| {
            if self.following_carried()? {
                return Ok(());
            }
            for (d, added) in watched {
                let id = self.listing(d, at)?;
                self.conn().execute("INSERT OR IGNORE INTO watched(instrument_id, added_at) VALUES (?, ?)", params![id.to_string(), at_text(*added)])?;
            }
            if let Some(t) = tiles {
                self.set_tiles(t, at)?;
            }
            self.conn().execute(
                "INSERT INTO settings(key, value, source, set_at) VALUES (?1, 'true', 'bagholder', ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value, source = excluded.source, set_at = excluded.set_at",
                params![CARRIED, at_text(at)],
            )?;
            Ok(())
        })
    }
}
