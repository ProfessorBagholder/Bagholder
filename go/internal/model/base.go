package model

import (
	"sort"
	"strings"

	"github.com/ProfessorBagholder/Bagholder/internal/py"
	"github.com/ProfessorBagholder/Bagholder/internal/store"
)

func MigrateLegacyNotes(closed []*Slice, saved []store.TradeGroup, notes map[string]store.TradeNote) map[string]store.JournalEntry {
	if len(notes) == 0 {
		return map[string]store.JournalEntry{}
	}
	byKey := map[string]*Slice{}
	for _, s := range closed {
		byKey[sliceMemberKey(s)] = s
	}
	used := map[string]bool{}
	out := map[string]store.TradeNote{}
	var outOrder []string
	put := func(k string, v store.TradeNote) {
		if _, ok := out[k]; !ok {
			outOrder = append(outOrder, k)
		}
		out[k] = v
	}
	for _, rec := range saved {
		var members []*Slice
		for _, k := range rec.Members {
			if s := byKey[k]; s != nil {
				members = append(members, s)
			}
		}
		if len(members) == 0 {
			continue
		}
		for _, m := range members {
			used[sliceMemberKey(m)] = true
		}
		if n, ok := notes[rec.ID]; ok {
			put(rec.ID, n)
		}
	}
	type laneKey struct{ account, symbol, currency string }
	lanes := map[laneKey][]*Slice{}
	var laneOrder []laneKey
	for _, s := range closed {
		if used[sliceMemberKey(s)] {
			continue
		}
		k := laneKey{s.AccountID, s.Symbol, s.Currency}
		if _, ok := lanes[k]; !ok {
			laneOrder = append(laneOrder, k)
		}
		lanes[k] = append(lanes[k], s)
	}
	for _, lk := range laneOrder {
		members := lanes[lk]
		sort.SliceStable(members, func(i, j int) bool { return sliceLess(members[i], members[j]) })
		var cur []*Slice
		direction := ""
		started := false
		flush := func(cur []*Slice) {
			if len(cur) == 0 {
				return
			}
			keys := make([]string, 0, len(cur))
			for _, s := range cur {
				keys = append(keys, sliceMemberKey(s))
			}
			gid := GroupIDForKeys(keys)
			n, ok := notes[gid]
			if !ok {
				return
			}
			counts := map[string]int{}
			var order []string
			for _, s := range cur {
				if _, ok := counts[s.RT]; !ok {
					order = append(order, s.RT)
				}
				counts[s.RT]++
			}
			best := ""
			bestN := -1
			for _, rt := range order {
				if counts[rt] > bestN {
					best, bestN = rt, counts[rt]
				}
			}
			if best != "" {
				put(best, n)
			}
		}
		for _, s := range members {
			if started && s.OpenDirection != direction {
				flush(cur)
				cur = nil
			}
			direction = s.OpenDirection
			started = true
			cur = append(cur, s)
		}
		flush(cur)
	}
	journal := map[string]store.JournalEntry{}
	for _, k := range outOrder {
		v := out[k]
		tags := []string{}
		for _, t := range strings.Split(v.Tag, ",") {
			if t = py.Strip(t); t != "" {
				tags = append(tags, t)
			}
		}
		journal[k] = store.JournalEntry{Thesis: v.Thesis, Tags: tags, Grade: v.Grade}
	}
	return journal
}

type Book struct {
	Activities []*Act
	ActsByID   map[string]*Act
	Securities *Securities
	Fifo       *FIFOResult
	RawCount   int
}

func BuildBook(snapshot *store.Snapshot, today string) *Book {
	acts := NormalizeActivities(snapshot.Activities)
	securities := NewSecurities(snapshot.Securities)
	delivered := SynthesizeAssignmentShares(acts, securities)
	if len(delivered) > 0 {
		acts = append(acts, delivered...)
	}
	fifo := MatchFIFO(acts)
	synthetic := SynthesizeExpiries(acts, fifo.Open, today)
	if len(synthetic) > 0 {
		acts = append(acts, synthetic...)
		fifo = MatchFIFO(acts)
	}
	byID := make(map[string]*Act, len(acts))
	for _, a := range acts {
		byID[a.ID] = a
	}
	return &Book{Activities: acts, ActsByID: byID, Securities: securities, Fifo: fifo, RawCount: len(snapshot.Activities)}
}

type Base struct {
	Today           string
	SyncedAt        string
	FX              map[string]float64
	Benchmark       map[string]float64
	Benchmarks      map[string]map[string]float64
	Distributions   map[string][]store.Distribution
	Quotes          map[string]store.Quote
	FxLast          string
	BenchmarkLast   string
	Activities      []*Act
	ActsByID        map[string]*Act
	Securities      *Securities
	Closed          []*Slice
	OpenLots        []*Lot
	Unmatched       []Unmatched
	Trades          []*Trade
	Positions       []*Position
	Cashflow        []CashRow
	Equity          []EquityPoint
	EquityByAccount map[string][]EquityPoint
	Accounts        []AccountRow
	Balances        []store.Balance
	Margin          []store.Margin
	Exposures       map[string]store.Exposure
	Watchlist       []store.Watch
	Tiles           []store.Tile
	TilesSaved      bool
	News            []store.NewsItem
	Universes       map[string][]store.Universe
	CashCurrencies  map[string]string
	ActivityCount   int
	LastPrices      map[string]LastPrice
}

func maxKey(m map[string]float64) string {
	best := ""
	for k := range m {
		if k > best {
			best = k
		}
	}
	return best
}

func BuildBase(snapshot *store.Snapshot, market *store.MarketData, journal map[string]store.JournalEntry, today string, book *Book) *Base {
	if today == "" {
		today = TodayLocal()
	}
	fx := market.FX
	if fx == nil {
		fx = map[string]float64{}
	}
	bench := market.Benchmark
	if bench == nil {
		bench = map[string]float64{}
	}
	benchmarks := map[string]map[string]float64{}
	for k, v := range market.Benchmarks {
		benchmarks[k] = v
	}
	if _, ok := benchmarks["SP500"]; !ok {
		benchmarks["SP500"] = bench
	}
	if book == nil {
		book = BuildBook(snapshot, today)
	}
	acts := book.Activities
	actsByID := book.ActsByID
	securities := book.Securities
	fifo := book.Fifo
	ApplyFX(fifo.Closed, fx)
	trades := BuildTrades(fifo.Closed, fifo.Open, snapshot.TradeGroups, actsByID, securities, journal)
	lastPrices := LastFillPrices(acts)
	quotes := market.Quotes
	if quotes == nil {
		quotes = map[string]store.Quote{}
	}
	positions := BuildPositions(fifo.Open, lastPrices, snapshot.Balances, snapshot.Accounts, securities, journal, today, quotes, actsByID)
	cashflow := BuildCashflow(acts, securities, fx)
	equity := EquitySeries(snapshot.NavHistory)
	byAccount := map[string][]EquityPoint{}
	for nick, pts := range snapshot.NavByAccount {
		byAccount[normAccountName(nick)] = EquitySeries(pts)
	}
	accounts := []AccountRow{}
	for _, acc := range snapshot.Accounts {
		nick := normAccountName(firstNonEmpty(acc.Nickname, acc.UnifiedAccountType, acc.Type))
		accounts = append(accounts, AccountRow{ID: acc.ID, Name: nick, Type: acc.UnifiedAccountType, Currency: acc.Currency, Status: acc.Status, Nav: copyPtr(acc.NetLiquidationValue)})
	}
	distributions := market.Distributions
	if distributions == nil {
		distributions = map[string][]store.Distribution{}
	}
	exposures := snapshot.Exposures
	if exposures == nil {
		exposures = map[string]store.Exposure{}
	}
	universes := map[string][]store.Universe{}
	for k, v := range snapshot.Universes {
		universes[k] = append([]store.Universe{}, v...)
	}
	return &Base{
		Today:           today,
		SyncedAt:        snapshot.SyncedAt,
		FX:              fx,
		Benchmark:       bench,
		Benchmarks:      benchmarks,
		Distributions:   distributions,
		Quotes:          quotes,
		FxLast:          maxKey(fx),
		BenchmarkLast:   maxKey(bench),
		Activities:      acts,
		ActsByID:        actsByID,
		Securities:      securities,
		Closed:          fifo.Closed,
		OpenLots:        fifo.Open,
		Unmatched:       fifo.Unmatched,
		Trades:          trades,
		Positions:       positions,
		Cashflow:        cashflow,
		Equity:          equity,
		EquityByAccount: byAccount,
		Accounts:        accounts,
		Balances:        append([]store.Balance{}, snapshot.Balances...),
		Margin:          append([]store.Margin{}, snapshot.Margin...),
		Exposures:       exposures,
		Watchlist:       append([]store.Watch{}, snapshot.Watchlist...),
		Tiles:           snapshot.Tiles,
		TilesSaved:      snapshot.TilesSaved,
		News:            append([]store.NewsItem{}, snapshot.News...),
		Universes:       universes,
		CashCurrencies:  securities.CashCurrencies(),
		ActivityCount:   book.RawCount,
		LastPrices:      lastPrices,
	}
}
