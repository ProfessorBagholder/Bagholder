package model

import (
	"bytes"
	"encoding/json"
	"sync"

	"github.com/ProfessorBagholder/Bagholder/internal/py"
	"github.com/ProfessorBagholder/Bagholder/internal/store"
)

type inputs struct {
	accounts []store.Account
	balances []store.Balance
	journal  map[string]store.JournalEntry
}

type Model struct {
	st      *store.Store
	mu      sync.Mutex
	buildMu sync.Mutex
	rw      sync.RWMutex
	version string
	core    string
	base    *Base
	inputs  *inputs
	bookKey string
	book    *Book

	viewMu   sync.Mutex
	viewBase *Base
	views    map[string]viewEntry
}

func New(st *store.Store) *Model {
	return &Model{st: st}
}

func (m *Model) BaseModel(force bool) *Base {
	today := TodayLocal()
	full, core := m.st.Versions()
	version := full + "|" + today
	coreKey := core + "|" + today
	m.mu.Lock()
	if !force && m.base != nil && m.version == version {
		b := m.base
		m.mu.Unlock()
		return b
	}
	m.mu.Unlock()
	m.buildMu.Lock()
	defer m.buildMu.Unlock()
	m.mu.Lock()
	if !force && m.base != nil && m.version == version {
		b := m.base
		m.mu.Unlock()
		return b
	}
	var marked *Base
	var markedInputs *inputs
	if !force && m.base != nil && m.core == coreKey {
		marked, markedInputs = m.base, m.inputs
	}
	m.mu.Unlock()
	if marked != nil {
		return m.remark(marked, markedInputs, today, version, coreKey)
	}
	bookKey := m.st.BookVersion() + "|" + today
	m.mu.Lock()
	var book *Book
	if !force && m.bookKey == bookKey {
		book = m.book
	}
	m.mu.Unlock()
	snapshot := m.st.Snapshot(book == nil)
	market := m.st.MarketData()
	journal := m.st.Journal()
	if book == nil {
		book = BuildBook(&snapshot, today)
	}
	if len(journal) == 0 && len(snapshot.Notes) > 0 {
		probe := BuildBase(&snapshot, &market, map[string]store.JournalEntry{}, today, book)
		migrated := MigrateLegacyNotes(probe.Closed, snapshot.TradeGroups, snapshot.Notes)
		if len(migrated) > 0 {
			journal = m.st.SaveJournal(migrated)
			version = m.st.DataVersion() + "|" + today
		}
	}
	base := BuildBase(&snapshot, &market, journal, today, book)
	m.mu.Lock()
	m.version = version
	m.core = coreKey
	m.base = base
	m.inputs = &inputs{accounts: snapshot.Accounts, balances: snapshot.Balances, journal: journal}
	m.bookKey = bookKey
	m.book = book
	m.mu.Unlock()
	return base
}

func (m *Model) remark(base *Base, in *inputs, today, version, coreKey string) *Base {
	quotes := m.st.MarketData().Quotes
	if quotes == nil {
		quotes = map[string]store.Quote{}
	}
	fresh := *base
	fresh.Quotes = quotes
	fresh.Positions = BuildPositions(base.OpenLots, base.LastPrices, in.balances, in.accounts, base.Securities, in.journal, today, quotes, base.ActsByID)
	m.mu.Lock()
	m.version = version
	m.core = coreKey
	m.base = &fresh
	m.mu.Unlock()
	return &fresh
}

func (m *Model) Base() *Base { return m.BaseModel(false) }

const ViewCacheMax = 16

type viewEntry struct {
	data  []byte
	stamp int
}

func (m *Model) View(filters any, detail string) []byte {
	m.rw.RLock()
	defer m.rw.RUnlock()
	base := m.BaseModel(false)
	f := CleanFilters(filters)
	keyRaw, _ := json.Marshal(f)
	key := string(keyRaw) + "\x00" + detail
	m.viewMu.Lock()
	if m.viewBase == base {
		if e, ok := m.views[key]; ok {
			m.viewMu.Unlock()
			out := append([]byte{}, e.data...)
			if e.stamp >= 0 {
				copy(out[e.stamp:], py.NowStamp())
			}
			return out
		}
	}
	m.viewMu.Unlock()
	out := marshalView(BuildView(base, filters), detail)
	stamp := -1
	if i := bytes.Index(out, []byte(`"generated":"`)); i >= 0 && i+len(`"generated":"`)+20 <= len(out) {
		stamp = i + len(`"generated":"`)
	}
	m.viewMu.Lock()
	if m.viewBase != base || m.views == nil || len(m.views) >= ViewCacheMax {
		m.viewBase = base
		m.views = map[string]viewEntry{}
	}
	m.views[key] = viewEntry{data: out, stamp: stamp}
	m.viewMu.Unlock()
	return append([]byte{}, out...)
}

func marshalView(v *View, detail string) []byte {
	if detail != "" {
		w := *v
		for i, t := range v.Trades {
			if t.ID == detail {
				w.Trades = append([]*Trade{}, v.Trades...)
				c := *t
				c.Detail = true
				w.Trades[i] = &c
			}
		}
		for i, p := range v.Positions {
			if p.ID == detail {
				w.Positions = append([]*Position{}, v.Positions...)
				c := *p
				c.Detail = true
				w.Positions[i] = &c
			}
		}
		v = &w
	}
	out, err := json.Marshal(v)
	if err != nil {
		panic(err)
	}
	return out
}

type Detail struct {
	ID    string      `json:"id"`
	Legs  []SlimSlice `json:"legs"`
	Fills []Fill      `json:"fills"`
}

func TradeDetail(base *Base, tradeID string) *Detail {
	for _, t := range base.Trades {
		if t.ID == tradeID {
			return &Detail{ID: tradeID, Legs: nonNil(t.Legs), Fills: nonNilFills(t.Fills)}
		}
	}
	for _, p := range base.Positions {
		if p.ID == tradeID {
			return &Detail{ID: tradeID, Legs: []SlimSlice{}, Fills: nonNilFills(p.Fills)}
		}
	}
	return nil
}

type HeldSymbol struct {
	Symbol   string `json:"symbol"`
	Exchange string `json:"exchange"`
	Currency string `json:"currency"`
	Kind     string `json:"kind"`
}

func HeldSymbols(base *Base) []HeldSymbol {
	out := []HeldSymbol{}
	seen := map[string]bool{}
	for _, p := range base.Positions {
		if seen[p.Symbol] {
			continue
		}
		seen[p.Symbol] = true
		out = append(out, HeldSymbol{p.Symbol, p.Exchange, p.Currency, p.Kind})
	}
	return out
}

type ArchiveSymbol struct {
	Symbol   string `json:"symbol"`
	Exchange string `json:"exchange"`
	Currency string `json:"currency"`
	Kind     string `json:"kind"`
	Start    string `json:"start"`
}

func IntradayArchiveSymbols(base *Base, since string) []ArchiveSymbol {
	since = cut(since, 10)
	if since == "" {
		since = shiftDate(base.Today, -365)
	}
	out := map[string]ArchiveSymbol{}
	want := func(rec HeldSymbol, start string) {
		cur, ok := out[rec.Symbol]
		if !ok || start < cur.Start {
			out[rec.Symbol] = ArchiveSymbol{rec.Symbol, rec.Exchange, rec.Currency, rec.Kind, start}
		}
	}
	charted := func(rec HeldSymbol) HeldSymbol {
		if rec.Kind == "Options" {
			under := underlying(rec.Symbol)
			if under != "" && under != "—" {
				return HeldSymbol{under, rec.Exchange, rec.Currency, "Shares"}
			}
		}
		return rec
	}
	for _, t := range base.Trades {
		if t.ExitDate >= since {
			start := t.EntryDate
			if since > start {
				start = since
			}
			want(charted(HeldSymbol{t.Symbol, t.Exchange, t.Currency, t.Kind}), start)
		}
	}
	for _, p := range base.Positions {
		start := p.Opened
		if start == "" {
			start = since
		}
		if since > start {
			start = since
		}
		want(charted(HeldSymbol{p.Symbol, p.Exchange, p.Currency, p.Kind}), start)
	}
	rows := []ArchiveSymbol{}
	for _, k := range store.SortedKeys(out) {
		rows = append(rows, out[k])
	}
	return rows
}

type PayerSymbol struct {
	Symbol   string `json:"symbol"`
	Exchange string `json:"exchange"`
	Currency string `json:"currency"`
}

func PayerSymbols(base *Base) []PayerSymbol {
	payers := map[string]bool{}
	for _, r := range base.Cashflow {
		if r.Kind == "Dividend" {
			payers[r.Symbol] = true
		}
	}
	out := []PayerSymbol{}
	seen := map[string]bool{}
	for _, p := range base.Positions {
		if payers[p.Symbol] && !seen[p.Symbol] && !p.Short {
			seen[p.Symbol] = true
			ex := p.Exchange
			if ex == "Crypto" {
				ex = ""
			}
			out = append(out, PayerSymbol{p.Symbol, ex, p.Currency})
		}
	}
	return out
}

func (m *Model) ApplyJournal(entries map[string]store.JournalEntry) {
	m.rw.Lock()
	defer m.rw.Unlock()
	m.mu.Lock()
	defer m.mu.Unlock()
	base := m.base
	if base == nil {
		return
	}
	if entries == nil {
		entries = map[string]store.JournalEntry{}
	}
	for _, t := range base.Trades {
		t.Grade, t.Thesis, t.Tags = journalOf(entries, t.ID)
	}
	for _, p := range base.Positions {
		p.Grade, p.Thesis, p.Tags = journalOf(entries, p.ID)
	}
	if m.inputs != nil {
		m.inputs.journal = entries
	}
	full, core := m.st.Versions()
	m.version = full
	m.core = core + "|" + base.Today
	m.viewMu.Lock()
	m.views = nil
	m.viewMu.Unlock()
}

func (m *Model) Invalidate(book bool) {
	m.mu.Lock()
	defer m.mu.Unlock()
	m.version = ""
	m.core = ""
	m.base = nil
	m.inputs = nil
	if book {
		m.bookKey = ""
		m.book = nil
	}
}
