# Stage 3a research log (verbatim evidence; folded into the plan's Verification)

## 1. Split adjustment of daily closes (2026-09-23)
- Yahoo chart `close` is split-adjusted: NVDA 2024-06-04 close reads 116.437 (traded ~1164 before the 10:1 split of 2024-06-10). The same reply carries `events.splits` {date 1718026200, numerator 10, denominator 1} when asked with `events=split`. Undoable from its own reply.
- TMX getTimeSeriesData is split-adjusted too: SHOP 2022-06-24 close 49.709 (traded ~497 before the 10:1 split effective 2022-06-29), identical to Yahoo's adjusted series; the reply states no split. Not undoable from its own reply.
- Decision: daily closes of listings from Yahoo, stored as traded by undoing the adjustment with the splits in the same reply; TMX not used for closes.

## 4. Cboe delayed chains (first capture 2026-09-24T01:57Z, after the 2026-09-23 session)
- The chain served at 01:57Z on the 24th has timestamp "2026-09-23 03:54:59" (UTC per Last-Modified), still lists contracts expiring 2026-09-22, and its SPY current_price = prev_day_close = close = 773.38, which is Yahoo's close for 2026-09-22 (Yahoo's 2026-09-23 close: 767.81). The chain had not been updated for the whole 2026-09-23 session.
- So: a chain can be a session behind. Its own timestamp must be checked against the session it is supposed to carry; a stale chain is a meaning failure, not data.
- Next capture: during and after the 2026-09-24 session.

## 3. The Bank's rates before 2017 (2026-09-24)
- Valet daily averages (FX<CUR>CAD) begin 2017-01-03; Valet LEGACY_NOON_RATES (IEXE0101 = USD_NOON) cover 2007-05-01..2017-04-28.
- Statistics Canada table 10-10-0008-01, "Foreign exchange rates in Canadian dollars, Bank of Canada, daily" (noon spot rates, 44 series), covers 1950-10-02..2017-04-28 through its Web Data Service: USD noon is vector 121716. It equals the Bank's own noon series where both exist (2008-01-02 0.9927, 2008-01-03 0.9905, 2008-01-04 0.9991 in both) and answers 1995 (1995-01-03 1.4035).
- StatCan's table 33-10-0036-01 (daily averages) returns nothing for USD before 2017 (2003 range empty), so it does not extend Valet's daily average backward.
- StatCan answers JSON numbers (read from their text); it states a non-business day explicitly (value null, statusCode 1); it refuses bursts (empty body), so it is paced.
- Decision: the Bank's daily average (Valet) from 2017-01-03; before that, the Bank's noon rate from Statistics Canada 10-10-0008 (one source, 1950 onward); Valet's legacy noon group is not needed. No pre-2007 gap.
