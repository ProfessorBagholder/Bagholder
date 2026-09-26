//! The one door from the figure path to the market tables the earlier store still
//! keeps (`docs/plans/stage-5-interface-and-running.md`, A2; stage 6 moves them):
//! the classifier's records and the universes, answered as [`Tables`] asks. The
//! earlier store's floats become decimals here, from the shortest text that is
//! each float, which is the text the source sent.

use bagholder_core::instrument::InstrumentKind;
use bagholder_core::Currency;


use std::sync::Arc;

use super::markets::{Record, Tables, UniverseRow};
use super::news::{FiledRelease, NewsRow};
use crate::market_context::Built;

/// A float the earlier store kept, as the decimal its text states; none for one
/// that states no number.
fn dec_of(v: f64) -> Option<bagholder_core::Dec> {
    if !v.is_finite() {
        return None;
    }
    bagholder_core::Dec::parse(&format!("{v}")).ok()
}

fn record(e: &bagholder_model::exposure::Exposure) -> Record {
    let weights = |ws: &[(String, f64)], normalize: bool| -> Vec<(String, bagholder_core::Dec)> {
        ws.iter()
            .filter_map(|(name, w)| {
                // a sector under the Portfolio's name, whatever alias the record was read
                // under; a weight in no sector (cash, other, blank) is left unclassified
                let name = if normalize { bagholder_model::exposure::norm_sector(name) } else { name.clone() };
                if name.is_empty() {
                    return None;
                }
                Some((name, dec_of(*w)?))
            })
            .collect()
    };
    Record { sectors: weights(&e.sectors, true), countries: weights(&e.countries, false) }
}

/// The earlier store's market tables, as the figure path asks of them.
pub struct Door<'a> {
    pub built: &'a Built,
    pub app: &'a Arc<crate::app::App>,
}

impl Tables for Door<'_> {
    fn holding(&self, kind: InstrumentKind, security: &str, underlying: &str, currency: Currency) -> Option<Record> {
        let exposures = &self.built.base.exposures;
        let found = match kind {
            // a contract is its underlying's exposure, under the share's record
            InstrumentKind::OptionContract => bagholder_model::exposure::underlying_exposure(exposures, underlying, currency.as_str()),
            _ => exposures.get(security),
        };
        found.map(record)
    }

    fn listing(&self, symbol: &str, exchange: &str, currency: Currency) -> Option<Record> {
        self.built.base.exposures.get(&bagholder_model::venues::watch_exposure_key(symbol, exchange, currency.as_str())).map(record)
    }

    fn universe(&self, key: &str) -> Vec<UniverseRow> {
        let rows = self.built.base.universes.0.iter().find(|(k, _)| k == key).map(|(_, r)| r.as_slice()).unwrap_or(&[]);
        rows.iter()
            .map(|r| UniverseRow {
                symbol: r.symbol.clone(),
                name: r.name.clone(),
                value: dec_of(r.value).unwrap_or(bagholder_core::Dec::ZERO),
                // the source states it in percent
                percent_change: r.percent_change.filter(|c| c.is_finite()).map(|c| c / 100.0),
                sector: r.sector.clone(),
                country: r.country.clone(),
            })
            .collect()
    }

    fn news(&self) -> Arc<Vec<NewsRow>> {
        self.built
            .news
            .get_or_init(|| {
                Arc::new(
                    self.built
                        .base
                        .news
                        .iter()
                        .map(|n| NewsRow { id: n.id.clone(), symbol: n.symbol.clone(), exchange: n.exchange.clone(), headline: n.headline.clone(), wire: n.wire.clone(), url: n.url.clone(), published_at: n.published_at.clone(), kind: n.kind.clone() })
                        .collect(),
                )
            })
            .clone()
    }

    fn filed_releases(&self, scope: &str, chip: Option<(&str, &str)>) -> Result<Arc<Vec<FiledRelease>>, String> {
        let key = match chip {
            Some((s, e)) => format!("chip|{s}|{e}"),
            None => format!("scope|{scope}"),
        };
        let mut kept = self.built.filed.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(rows) = kept.get(&key) {
            return Ok(rows.clone());
        }
        let rows = Arc::new(crate::feeds::filed_releases(self.app, scope, chip)?);
        kept.insert(key, rows.clone());
        Ok(rows)
    }
}

#[cfg(test)]
mod tests {
    /// A record's sectors under the Portfolio's twelve names, whatever the source
    /// called them; a weight in no sector (cash, other) left for `Not classified`.
    #[test]
    fn a_record_s_sectors_are_the_portfolio_s_and_cash_is_none_of_them() {
        let e = bagholder_model::exposure::Exposure {
            sectors: vec![("Media & Telecommunications".into(), 0.6), ("technology".into(), 0.3), ("Cash".into(), 0.1)],
            countries: vec![("Canada".into(), 1.0)],
        };
        let r = super::record(&e);
        let names: Vec<&str> = r.sectors.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["Communication Services", "Information Technology"]);
    }
}
