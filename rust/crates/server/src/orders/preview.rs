//! The order ticket's figures (`SPEC.md` §4, Order ticket; `docs/plans/stage-3c-switch.md`,
//! §5): the working price, the stop loss and take profit prices, what is at risk and
//! what the target gains, the order's value in CAD and its share of the accounts'
//! value, and the cash or available margin after it. Worked out exactly from what the
//! person typed and what the quote states, on the server, so the page does no money
//! arithmetic; the page shows each as it comes. The order's value in CAD is at the
//! Bank of Canada's rate for the quote's currency today, as the figures hold it; a
//! currency they hold no rate for leaves it, and what follows from it, waiting.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use bagholder_core::{Dec, Rounding};

use crate::wire::Dec as Text;
use crate::wire::Fig;

/// What the ticket holds, as the page sends it: every amount as the text typed or
/// the quote's own figure, none where it is not set.
#[derive(Clone, Debug, Default, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct PreviewRequest {
    /// `BUY` or `SELL`.
    pub side: String,
    /// `MARKET`, `LIMIT`, `STOP` or `STOP_LIMIT`.
    #[serde(rename = "type")]
    pub kind: String,
    pub quantity: Option<String>,
    /// The order's value typed in Amount: the quantity becomes the whole units it buys.
    pub amount: Option<String>,
    pub limit: Option<String>,
    pub stop: Option<String>,
    pub sl: StopInput,
    pub tp: TargetInput,
    pub quote: QuoteInput,
    pub margin_rate: Option<String>,
    pub margin_available: Option<String>,
    pub cash: Option<String>,
    pub buying_power: Option<String>,
    /// The account borrows; `linked_margin`, it backs a margin account.
    pub margin: bool,
    pub linked_margin: bool,
    /// The accounts' value, CAD.
    pub nav: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct StopInput {
    pub on: bool,
    /// `stop` or `trail`.
    pub kind: String,
    /// `amt` or `pct`: a fixed stop typed as a price, or as a percent below the working price.
    pub price_unit: String,
    pub price: Option<String>,
    pub pct: Option<String>,
    pub trail: Option<String>,
    /// `pct` or `amt`: the trail's distance as a percent or an amount.
    pub unit: String,
}

#[derive(Clone, Debug, Default, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct TargetInput {
    pub on: bool,
    /// `amt` or `pct`.
    pub unit: String,
    pub price: Option<String>,
    pub pct: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct QuoteInput {
    pub last: Option<String>,
    pub ask: Option<String>,
    pub bid: Option<String>,
    /// Shares a unit: a contract's size, 1 otherwise.
    pub multiplier: Option<String>,
    pub currency: String,
}

/// The ticket's figures. An amount none where what it needs is not there.
#[derive(Clone, Debug, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    /// The price the order works at: the ask or bid for a market order, the stop
    /// for a stop order, the limit otherwise.
    pub entry: Option<Text>,
    /// The limit as the order carries it: typed, else the last price at an order's tick.
    pub limit: Option<Text>,
    /// The stop as the order carries it: typed, else 2% through the last price.
    pub stop: Option<Text>,
    pub quantity: Text,
    pub notional: Option<Text>,
    pub stop_loss_on: bool,
    pub take_profit_on: bool,
    pub trailing: bool,
    /// The trail's distance as typed (a percent or an amount), and in price.
    pub trail: Option<Text>,
    pub trail_distance: Option<Text>,
    pub stop_loss_pct_in: Text,
    pub stop_loss_price: Option<Text>,
    pub take_profit_pct_in: Text,
    pub take_profit_price: Option<Text>,
    /// What the stop loss loses and the target gains, in the instrument's currency.
    pub risk: Option<Text>,
    pub gain: Option<Text>,
    pub stop_loss_pct: Option<f64>,
    pub take_profit_pct: Option<f64>,
    pub reward_to_risk: Option<f64>,
    /// The order's value in CAD, and its share of the accounts' value: waiting where
    /// the quote's currency has no rate.
    pub cad: Option<Fig<Text>>,
    pub position_share: Option<Fig<f64>>,
    /// The margin account's available margin after the order.
    pub margin_after: Option<Fig<Text>>,
    /// What the review's last line shows: available margin after on a margin
    /// account, cash after on any other.
    pub after: Option<Fig<Text>>,
    /// The whole units the buying power covers at the working price (a Buy).
    pub max_quantity: Option<Text>,
}

/// A field that does not read as a decimal.
#[derive(Debug, PartialEq, Eq)]
pub struct Unread(pub String);

fn field(name: &str, v: &Option<String>) -> Result<Option<Dec>, Unread> {
    match v.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        None => Ok(None),
        Some(t) => Dec::parse(&t.replace(',', "")).map(Some).map_err(|_| Unread(format!("{name} {t:?} is not a number"))),
    }
}

/// A price as an order may carry it: two decimals from $1, four below it.
pub fn tick(p: Dec) -> Dec {
    p.round(if p >= Dec::ONE { 2 } else { 4 }, Rounding::HalfUp)
}

fn cents(p: Dec) -> Dec {
    p.round(2, Rounding::HalfUp)
}

fn d(s: &str) -> Dec {
    Dec::parse(s).expect("a constant")
}

fn ratio(n: Dec, over: Dec) -> Option<f64> {
    (!over.is_zero()).then(|| n.div_rounded(over, 12, Rounding::HalfEven).ok()).flatten().map(|r| r.to_f64())
}

/// CAD per unit of the quote's currency today, or the gaps it waits on; `None`
/// before a quote has named a currency.
pub type ToCad = Option<Result<Dec, Vec<String>>>;

/// The ticket's figures, the rate to CAD for the quote's currency looked up in the
/// figures (the Bank of Canada's, for today).
pub fn preview_for(app: &std::sync::Arc<crate::app::App>, r: &PreviewRequest) -> Result<Preview, crate::http::ApiError> {
    use crate::http::ApiError;
    let named = r.quote.currency.trim();
    let to_cad: ToCad = if named.is_empty() {
        None
    } else {
        let currency = bagholder_core::Currency::parse(&named.to_uppercase()).map_err(|_| ApiError::BadRequest(format!("the quote's currency {named:?} is not a currency")))?;
        let f = app.figures.get().ok_or_else(|| ApiError::Failed("The figures are not open.".into()))?;
        let rate = f
            .read(|e| {
                let i = e.inputs();
                bagholder_engine::fx::rate(&i.facts.rates, &i.clock, currency, i.clock.today)
            })
            .ok_or_else(|| ApiError::Failed("The figures are not built yet.".into()))?;
        Some(rate.map_err(|g| g.words().into_iter().map(String::from).collect()))
    };
    preview(r, &to_cad).map_err(|Unread(why)| ApiError::BadRequest(why))
}

/// The ticket's figures from what it holds, and the rate to CAD of its currency.
pub fn preview(r: &PreviewRequest, to_cad: &ToCad) -> Result<Preview, Unread> {
    let buy = r.side.eq_ignore_ascii_case("BUY");
    let dir = if buy { Dec::ONE } else { Dec::ONE.neg() };
    let hundred = d("100");
    let mul = |a: Dec, b: Dec| a.checked_mul(b).map_err(|e| Unread(e.to_string()));
    let sub = |a: Dec, b: Dec| a.checked_sub(b).map_err(|e| Unread(e.to_string()));
    let add = |a: Dec, b: Dec| a.checked_add(b).map_err(|e| Unread(e.to_string()));
    let div = |a: Dec, b: Dec| a.div_rounded(b, 12, Rounding::HalfEven).map_err(|e| Unread(e.to_string()));

    let last = field("the last price", &r.quote.last)?;
    let ask = field("the ask", &r.quote.ask)?;
    let bid = field("the bid", &r.quote.bid)?;
    let mult = field("the contract size", &r.quote.multiplier)?.filter(|m| m.is_positive()).unwrap_or(Dec::ONE);
    let typed_qty = field("the quantity", &r.quantity)?.unwrap_or(Dec::ZERO);
    let amount = field("the amount", &r.amount)?;
    let limit = match field("the limit price", &r.limit)? {
        Some(l) => Some(l),
        None => last.map(tick),
    };
    let stop = match field("the stop price", &r.stop)? {
        Some(s) => Some(s),
        None => last.map(|l| mul(l, if buy { d("1.02") } else { d("0.98") }).map(cents)).transpose()?,
    };
    let entry = match r.kind.as_str() {
        "MARKET" => (if buy { ask } else { bid }).or(last),
        "STOP" => stop,
        _ => limit,
    };
    // Amount typed sets the whole units it buys at the working price
    let qty = match (amount, entry) {
        (Some(a), Some(e)) if e.is_positive() => {
            let whole = div(a, mul(e, mult)?)?.round(0, Rounding::TowardZero);
            if whole.is_negative() { Dec::ZERO } else { whole }
        }
        (Some(_), _) => Dec::ZERO,
        (None, _) => typed_qty,
    };
    let notional = entry.map(|e| mul(mul(qty, e)?, mult)).transpose()?;
    let stop_loss_on = buy && r.sl.on;
    let take_profit_on = buy && r.tp.on;
    let trailing = r.sl.kind == "trail";
    let trail_pct = r.sl.unit == "pct";
    let trail = match field("the trail", &r.sl.trail)? {
        Some(t) => Some(t),
        None if trail_pct => Some(d("5")),
        None => entry.map(|e| mul(e, d("0.05")).map(cents)).transpose()?,
    };
    let trail_distance = match (entry, trail) {
        (Some(e), Some(t)) if trail_pct => Some(div(mul(e, t)?, hundred)?),
        (Some(_), Some(t)) => Some(t),
        _ => None,
    };
    let stop_loss_pct_in = field("the stop loss percent", &r.sl.pct)?.unwrap_or(d("5"));
    let stop_loss_price = match entry {
        None => None,
        Some(e) if trailing => trail_distance.map(|t| sub(e, mul(dir, t)?).map(cents)).transpose()?,
        Some(e) if r.sl.price_unit == "pct" => Some(cents(mul(e, sub(Dec::ONE, div(mul(dir, stop_loss_pct_in)?, hundred)?)?)?)),
        Some(e) => match field("the stop loss price", &r.sl.price)? {
            Some(p) => Some(p),
            None => Some(cents(mul(e, sub(Dec::ONE, mul(dir, d("0.05"))?)?)?)),
        },
    };
    let take_profit_pct_in = field("the take profit percent", &r.tp.pct)?.unwrap_or(d("10"));
    let take_profit_price = match entry {
        None => None,
        Some(e) if r.tp.unit == "pct" => Some(cents(mul(e, add(Dec::ONE, div(mul(dir, take_profit_pct_in)?, hundred)?)?)?)),
        Some(e) => match field("the take profit price", &r.tp.price)? {
            Some(p) => Some(p),
            None => Some(cents(mul(e, add(Dec::ONE, mul(dir, d("0.1"))?)?)?)),
        },
    };
    // a trail is defined by its distance, so its loss is that distance exactly; a
    // fixed stop or target is the price the order carries
    let per_unit_risk = match (entry, stop_loss_price) {
        (Some(_), Some(_)) if trailing => trail_distance,
        (Some(e), Some(s)) => Some(mul(dir, sub(e, s)?)?),
        _ => None,
    };
    let risk = per_unit_risk.map(|u| mul(mul(u, qty)?, mult)).transpose()?;
    let gain = match (entry, take_profit_price) {
        (Some(e), Some(t)) => Some(mul(mul(mul(dir, sub(t, e)?)?, qty)?, mult)?),
        _ => None,
    };
    let stop_loss_pct = match (entry, stop_loss_price) {
        (Some(e), _) if trailing => trail_distance.and_then(|t| ratio(t.neg(), e)),
        (Some(e), Some(s)) => ratio(mul(dir, sub(s, e)?)?, e),
        _ => None,
    };
    let take_profit_pct = match (entry, take_profit_price) {
        (Some(e), Some(t)) => ratio(mul(dir, sub(t, e)?)?, e),
        _ => None,
    };
    let reward_to_risk = match (risk, gain) {
        (Some(rk), Some(g)) if stop_loss_on && take_profit_on && rk.is_positive() => ratio(g, rk),
        _ => None,
    };
    // the order's value in CAD at today's rate: a currency with no rate waits, never 1:1
    let cad: Option<Fig<Dec>> = match (notional, to_cad) {
        (Some(n), Some(Ok(rate))) => Some(Fig::Stated(mul(n, *rate)?)),
        (Some(_), Some(Err(gaps))) => Some(Fig::Waits { gaps: gaps.clone() }),
        _ => None,
    };
    let nav = field("the accounts' value", &r.nav)?.filter(|n| n.is_positive());
    let position_share = match (&cad, nav) {
        (Some(Fig::Stated(c)), Some(n)) => ratio(*c, n).map(Fig::Stated),
        (Some(Fig::Waits { gaps }), Some(_)) => Some(Fig::Waits { gaps: gaps.clone() }),
        _ => None,
    };
    let rate = field("the margin rate", &r.margin_rate)?.unwrap_or(Dec::ONE);
    let margin_after = match (r.margin || r.linked_margin, field("the available margin", &r.margin_available)?, &cad) {
        (true, Some(m), Some(Fig::Stated(c))) => Some(Fig::Stated(sub(m, mul(mul(dir, *c)?, rate)?)?)),
        (true, Some(_), Some(Fig::Waits { gaps })) => Some(Fig::Waits { gaps: gaps.clone() }),
        _ => None,
    };
    let after = if r.margin {
        margin_after.clone()
    } else {
        match (field("the cash", &r.cash)?, notional) {
            (Some(c), Some(n)) => Some(Fig::Stated(sub(c, mul(dir, n)?)?)),
            _ => None,
        }
    };
    let fig_text = |v: Option<Fig<Dec>>| v.map(|f| match f { Fig::Stated(d) => Fig::Stated(Text(d)), Fig::Waits { gaps } => Fig::Waits { gaps } });
    let max_quantity = match (buy, field("the buying power", &r.buying_power)?, entry) {
        (true, Some(bp), Some(e)) if e.is_positive() => {
            let whole = div(bp, mul(e, mult)?)?.round(0, Rounding::TowardZero);
            Some(if whole.is_negative() { Dec::ZERO } else { whole })
        }
        _ => None,
    };
    let text = |v: Option<Dec>| v.map(Text);
    Ok(Preview {
        entry: text(entry),
        limit: text(limit),
        stop: text(stop),
        quantity: Text(qty),
        notional: text(notional),
        stop_loss_on,
        take_profit_on,
        trailing,
        trail: text(trail),
        trail_distance: text(trail_distance),
        stop_loss_pct_in: Text(stop_loss_pct_in),
        stop_loss_price: text(stop_loss_price),
        take_profit_pct_in: Text(take_profit_pct_in),
        take_profit_price: text(take_profit_price),
        risk: text(risk),
        gain: text(gain),
        stop_loss_pct,
        take_profit_pct,
        reward_to_risk,
        cad: fig_text(cad),
        position_share,
        margin_after: fig_text(margin_after),
        after: fig_text(after),
        max_quantity: text(max_quantity),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: &str) -> Option<String> {
        Some(s.into())
    }

    /// A Buy limit of 10 at 100, a 5% stop and a 10% target, quoted 99/101 last 100 in USD.
    fn ticket() -> PreviewRequest {
        PreviewRequest {
            side: "BUY".into(),
            kind: "LIMIT".into(),
            quantity: t("10"),
            limit: t("100"),
            sl: StopInput { on: true, kind: "stop".into(), price_unit: "pct".into(), pct: t("5"), unit: "pct".into(), ..Default::default() },
            tp: TargetInput { on: true, unit: "pct".into(), pct: t("10"), ..Default::default() },
            quote: QuoteInput { last: t("100"), ask: t("101"), bid: t("99"), multiplier: t("1"), currency: "USD".into() },
            nav: t("10000"),
            ..Default::default()
        }
    }

    fn dec(v: &Option<Text>) -> Option<Dec> {
        v.map(|t| t.0)
    }

    fn stated(v: &Option<Fig<Text>>) -> Option<Dec> {
        match v {
            Some(Fig::Stated(t)) => Some(t.0),
            Some(Fig::Waits { gaps }) => panic!("waits on {gaps:?}"),
            None => None,
        }
    }

    /// No rate: as before the quote names a currency.
    const NONE: ToCad = None;

    fn at(rate: &str) -> ToCad {
        Some(Ok(d(rate)))
    }

    #[test]
    fn a_buy_limit_with_a_5pc_stop_and_10pc_target_risks_50_to_gain_100() {
        let p = preview(&ticket(), &NONE).unwrap();
        assert_eq!((dec(&p.entry), dec(&p.notional)), (Some(d("100")), Some(d("1000"))));
        assert_eq!((dec(&p.stop_loss_price), dec(&p.take_profit_price)), (Some(d("95")), Some(d("110"))));
        assert_eq!((dec(&p.risk), dec(&p.gain)), (Some(d("50")), Some(d("100"))));
        assert_eq!(p.reward_to_risk, Some(2.0));
        assert!((p.stop_loss_pct.unwrap() + 0.05).abs() < 1e-12 && (p.take_profit_pct.unwrap() - 0.1).abs() < 1e-12);
    }

    #[test]
    fn a_trailing_stop_loses_exactly_its_distance() {
        let mut r = ticket();
        r.sl = StopInput { on: true, kind: "trail".into(), price_unit: "amt".into(), trail: t("5"), unit: "pct".into(), ..Default::default() };
        let p = preview(&r, &NONE).unwrap();
        assert!(p.trailing);
        assert_eq!((dec(&p.stop_loss_price), dec(&p.trail_distance), dec(&p.risk)), (Some(d("95")), Some(d("5")), Some(d("50"))));
    }

    #[test]
    fn a_sell_has_no_brackets_and_no_reward_to_risk() {
        let mut r = ticket();
        r.side = "SELL".into();
        let p = preview(&r, &NONE).unwrap();
        assert!(!p.stop_loss_on && !p.take_profit_on);
        assert_eq!(p.reward_to_risk, None);
    }

    #[test]
    fn cash_after_a_buy_is_the_cash_less_the_order_and_margin_after_less_its_margin() {
        let mut r = ticket();
        r.cash = t("5000");
        assert_eq!(stated(&preview(&r, &NONE).unwrap().after), Some(d("4000")));
        r.margin = true;
        r.margin_available = t("12680.45");
        r.margin_rate = t("0.3");
        let p = preview(&r, &at("1.3712")).unwrap();
        // 1000 USD at 1.3712 is 1371.20 CAD, a third of it drawn at a 30% rate
        assert_eq!((stated(&p.cad), stated(&p.margin_after), stated(&p.after)), (Some(d("1371.2")), Some(d("12269.09")), Some(d("12269.09"))));
        assert_eq!(p.position_share, Some(Fig::Stated(0.13712)));
    }

    #[test]
    fn defaults_come_from_the_quote_at_an_orders_tick() {
        let mut r = ticket();
        r.limit = None;
        r.stop = None;
        r.quote.last = t("0.62345");
        let p = preview(&r, &NONE).unwrap();
        assert_eq!(dec(&p.limit), Some(d("0.6235")), "four places under a dollar");
        assert_eq!(dec(&p.stop), Some(d("0.64")), "2% through the last, to the cent");
        r.quote.last = t("12.345");
        assert_eq!(dec(&preview(&r, &NONE).unwrap().limit), Some(d("12.35")));
        r.kind = "MARKET".into();
        assert_eq!(dec(&preview(&r, &NONE).unwrap().entry), Some(d("101")), "a market buy works at the ask");
    }

    #[test]
    fn max_is_the_whole_units_the_buying_power_covers() {
        let mut r = ticket();
        r.buying_power = t("1050");
        r.quote.multiplier = t("100");
        assert_eq!(dec(&preview(&r, &NONE).unwrap().max_quantity), Some(d("0")), "not one contract of 100 at 100");
        r.quote.multiplier = None;
        assert_eq!(dec(&preview(&r, &NONE).unwrap().max_quantity), Some(d("10")));
    }

    /// The order's value in CAD is the notional at the rate given for its currency,
    /// whatever the currency; a currency with no rate waits, and so does everything
    /// worked from it, never taken 1:1.
    #[test]
    fn the_value_in_cad_is_at_the_currencys_rate_and_waits_without_one() {
        for rate in ["1", "1.3712", "0.0093", "1.8", "0.7431"] {
            let mut r = ticket();
            r.margin = true;
            r.margin_available = t("12680.45");
            r.margin_rate = t("0.3");
            let p = preview(&r, &at(rate)).unwrap();
            let cad = d("1000").checked_mul(d(rate)).unwrap();
            assert_eq!(stated(&p.cad), Some(cad), "at {rate}");
            assert_eq!(p.position_share, ratio(cad, d("10000")).map(Fig::Stated), "at {rate}");
        }
        let mut r = ticket();
        r.margin = true;
        r.margin_available = t("12680.45");
        let gaps = vec!["rate-not-held".to_string()];
        let p = preview(&r, &Some(Err(gaps.clone()))).unwrap();
        let waits = Some(Fig::Waits { gaps: gaps.clone() });
        assert_eq!((&p.cad, &p.margin_after, &p.after), (&waits, &waits, &waits));
        assert_eq!(p.position_share, Some(Fig::Waits { gaps }));
        assert_eq!(dec(&p.notional), Some(d("1000")), "the order's own value stands in its own currency");
        // a cash account's cash after is in the order's currency: no rate needed
        r.margin = false;
        r.cash = t("5000");
        assert_eq!(stated(&preview(&r, &Some(Err(vec!["rate-not-held".into()]))).unwrap().after), Some(d("4000")));
        // before a quote names a currency there is no value in CAD to state
        assert_eq!(preview(&r, &NONE).unwrap().cad, None);
    }

    #[test]
    fn a_field_that_does_not_read_is_named() {
        let mut r = ticket();
        r.quantity = t("ten");
        assert_eq!(preview(&r, &NONE).unwrap_err(), Unread("the quantity \"ten\" is not a number".into()));
    }

    #[test]
    fn an_amount_typed_buys_the_whole_units_it_covers() {
        let mut r = ticket();
        r.amount = t("1,055");
        let p = preview(&r, &NONE).unwrap();
        assert_eq!((p.quantity.0, dec(&p.notional)), (d("10"), Some(d("1000"))));
    }
}
