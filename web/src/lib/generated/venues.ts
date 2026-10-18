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
  ["XCHI", ["NYSE TEXAS", "NYSE CHICAGO", "XCHI"]],
  ["TXSE", ["TXSE", "TEXAS STOCK EXCHANGE"]],
  ["OTCM", ["OTCM", "OTCQ", "OTCB", "PINX", "PINC", "PINI", "PINL", "PSGM", "EXPM", "CAVE", "OTCD", "OTCN", "OTCI", "OTCO", "XOTC", "OOTC", "OTC", "OTC MARKETS", "OTC MARKETS GROUP", "OTC LINK", "OTCQX", "OTCQB", "PINK", "OTC PINK", "PINK SHEETS", "OTCBB", "OTC BB", "OTC BULLETIN BOARD", "GREY MARKET", "OTC GREY", "EXPERT MARKET", "OTCID"]],
]

/** Each venue a key holds, by the name a person reads it under. */
export const VENUE_NAMES: ReadonlyArray<readonly [string, string]> = [
  ["XTSE", "TSX"],
  ["XTSX", "TSX-V"],
  ["XCNQ", "CSE"],
  ["NEOE", "Cboe Canada"],
  ["XATS", "Alpha"],
  ["US", "US"],
  ["OTC", "OTC"],
  ["CRYPTO", "crypto"],
  ["INDEX", "index"],
  ["OCC", "option"],
]

export const CANADA_MICS: readonly string[] = ["XTSE", "XTSX", "XCNQ", "XATS", "NEOE"]

export const US_MICS: readonly string[] = ["XNAS", "XNYS", "ARCX", "XASE", "XCHI", "BATS", "IEXG", "TXSE"]

export const OTC_MICS: readonly string[] = ["OTCM", "OTCQ", "OTCB", "PINX", "PINC", "PINI", "PINL", "PSGM", "EXPM", "CAVE", "OTCD", "OTCN", "OTCI", "OTCO", "XOTC", "OOTC"]

/** The characters a symbol or a venue's words are trimmed of and a symbol drops, as the server reads space. */
export const SPACES: readonly string[] = ["\u{9}", "\u{a}", "\u{b}", "\u{c}", "\u{d}", "\u{20}", "\u{85}", "\u{a0}", "\u{1680}", "\u{2000}", "\u{2001}", "\u{2002}", "\u{2003}", "\u{2004}", "\u{2005}", "\u{2006}", "\u{2007}", "\u{2008}", "\u{2009}", "\u{200a}", "\u{2028}", "\u{2029}", "\u{202f}", "\u{205f}", "\u{3000}"]

export const VENUE_SUFFIXES: readonly string[] = [".TO", ".V", ".CN", ".NE"]

export const US = "US"
export const OTC = "OTC"
export const CRYPTO = "CRYPTO"
export const INDEX = "INDEX"
export const OCC = "OCC"
export const MARKET = "MARKET"
