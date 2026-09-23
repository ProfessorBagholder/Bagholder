// Generated from rust/crates/store/src/bars.rs and the server's chart history. Do not
// edit: change the Rust type, then `BAGHOLDER_BLESS=1 cargo test -p bagholder-server the_pages_chart_types`.

import type { OkOr } from './common'

export type DayBar = { date: string, open: number | null, high: number | null, low: number | null, close: number, volume: number | null, };

export type TimeBar = { time: number, open: number | null, high: number | null, low: number | null, close: number, volume: number | null, };

export type ChartBars = Array<DayBar> | Array<TimeBar>;

export type ChartHistory = { ok: boolean, symbol: string, chartSymbol: string, source: string, tf: string, available: Array<string>, bars: ChartBars, pending: boolean, reason: string, };

export type HistoryAnswer = ChartHistory | OkOr;
