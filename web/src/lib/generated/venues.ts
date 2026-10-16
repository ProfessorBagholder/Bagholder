// Generated from rust/crates/core/src/listing.rs. Do not edit: change the table there, then
// `BAGHOLDER_BLESS=1 cargo test -p bagholder-server the_pages_venue_table`.

/** Each venue's code and the words it is written in (upper case), the code itself among them. */
export const VENUE_WORDS: ReadonlyArray<readonly [string, readonly string[]]> = [
  ["XTSE", ["TSX", "TORONTO", "TORONTO STOCK EXCHANGE", "XTSE"]],
  ["XTSX", ["TSX-V", "TSXV", "TSX VENTURE", "TSX VENTURE EXCHANGE", "VENTURE", "CDNX", "XTSX", "XTSV"]],
  ["XCNQ", ["CSE", "CANADIAN SECURITIES EXCHANGE", "XCNQ", "CNSX"]],
  ["NEOE", ["CBOE CANADA", "CBOE CA", "NEO", "NEOE", "AQL"]],
  ["XATS", ["ALPHA", "ALPHA EXCHANGE", "XATS"]],
  ["XNAS", ["NASDAQ", "XNAS"]],
  ["XNYS", ["NYSE", "XNYS"]],
  ["ARCX", ["NYSE ARCA", "ARCA", "ARCX"]],
  ["XASE", ["NYSE AMERICAN", "AMEX", "XASE"]],
  ["BATS", ["BATS", "CBOE BZX", "CBOE"]],
  ["IEXG", ["IEX", "IEXG"]],
]

/** Each venue a key holds, by the name a person reads it under. */
export const VENUE_NAMES: ReadonlyArray<readonly [string, string]> = [
  ["XTSE", "TSX"],
  ["XTSX", "TSX-V"],
  ["XCNQ", "CSE"],
  ["NEOE", "Cboe Canada"],
  ["XATS", "Alpha"],
  ["US", "US"],
  ["CRYPTO", "crypto"],
  ["INDEX", "index"],
  ["OCC", "option"],
]

export const CANADA_MICS: readonly string[] = ["XTSE", "XTSX", "XCNQ", "XATS", "NEOE"]

export const US_MICS: readonly string[] = ["XNAS", "XNYS", "ARCX", "XASE", "BATS", "IEXG"]

export const VENUE_SUFFIXES: readonly string[] = [".TO", ".V", ".CN", ".NE"]

export const US = "US"
export const CRYPTO = "CRYPTO"
export const INDEX = "INDEX"
export const OCC = "OCC"
export const MARKET = "MARKET"
