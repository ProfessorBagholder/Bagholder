// Generated from rust/crates/store/src/activities.rs, rust/crates/store/src/csvimport.rs and the
// server's book-append answer. Do not edit: change the Rust type, then
// `BAGHOLDER_BLESS=1 cargo test -p bagholder-server the_pages_book_types`.

export type ActivityRow = { id: string, canonicalId: string | null, occurredAt: string, transactionDate: string, settlementDate: string, accountId: string, bookId: string, fifoId: string, accountType: string, activityType: string, activitySubType: string, description: string, direction: string, symbol: string, name: string, currency: string, quantity: number, unitPrice: number, commission: number, netCashAmount: number, category: string, balance: number | null, source: string, rawType: string, aftType: string, counterSymbol: string, securityId: string | null, };

export type Skipped = { row: number, message: string, raw: string, };

export type ImportReport = { ok: boolean, file: string, format: string, rows: number, added: number, duplicates: number, skipped: Array<Skipped>, skippedCount: number, footerStripped: boolean, countsByType: { [key in string]: number }, };

export type CsvFile = { path: string, name: string, size: number, mtime: number, };

export type WatchSet = { ok: boolean, error: string | null, path: string | null, };

export type ScannedFile = { file: string, unchanged: boolean, added: number, duplicates: number, format: string, } | { file: string, error: string, } | { file: string, unchanged: boolean, added: number, duplicates: number, format: string, rows: number, skippedCount: number, };

export type ScanReport = { ok: boolean, error: string | null, path: string | null, added: number | null, duplicates: number | null, files: Array<ScannedFile> | null, scannedAt: string | null, };

export type StatusFile = { file: string, added: number, duplicates: number, format: string, scannedAt: string, };

export type WatchStatus = { ok: boolean, path: string, watching: boolean, lastScan: string, files: Array<StatusFile>, };

export type Appended = { ok: boolean, added: number, duplicates: number, activity: ActivityRow | null, activities: Array<ActivityRow> | null, };
