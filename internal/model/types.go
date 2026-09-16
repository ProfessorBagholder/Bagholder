// Package model is the derived trading model, computed from the SQLite store.
//
// Everything the page shows comes from here so that one list of trades feeds
// every tile, table and chart, and so the numbers can be tested.
//
//	activities  -> Normalize          (crypto, options, stock-dividend notices)
//	            -> MatchFIFO          (FIFO lots per account+symbol+currency, round-trip ids, option rolls)
//	            -> ApplyFX            (P&L in CAD on the fill dates)
//	            -> BuildTrades        (round trips + saved manual groups)
//	            -> BuildPositions     (open lots rolled up per symbol+account)
//	            -> BuildCashflow      (dividends, interest, withholding tax)
//	nav_history -> equity series, yearly time-weighted returns, drawdown
//	BuildView(filters) applies one filter object to all of the above.
//
// Currency: per-trade numbers are native. Anything that adds trades together uses
// the CAD value converted on the fill date with the Bank of Canada rate.
package model

import (
	"encoding/json"
	"time"

	"github.com/ProfessorBagholder/Bagholder/internal/store"
	"github.com/ProfessorBagholder/Bagholder/internal/symbols"
)

const (
	EPS        = 1e-10
	FXFallback = 1.35
)

var Grades = []string{"A", "B", "C", "F"}
var Kinds = []string{"Shares", "Options", "Crypto", "Futures"}
var Months = []string{"Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"}

// TimeTZ is the app's local zone, the one the calendar day is measured in.
var TimeTZ = store.ActivityPullTZ

// Now is the clock; a test may replace it.
var Now = time.Now

// TodayLocal is today's date in the app's zone.
func TodayLocal() string {
	return Now().In(TimeTZ).Format("2006-01-02")
}

// Act is a normalized activity: the store's row with a kind and flags, shared by
// pointer so the match's inferences (a multileg's quantity and price) show in the fills.
type Act = store.Activity

// Lot is one open lot of a book.
type Lot struct {
	Qty         float64  `json:"qty"`
	Price       float64  `json:"price"`
	Date        string   `json:"date"`
	When        string   `json:"when"`
	Commission  float64  `json:"commission"`
	Direction   string   `json:"direction"`
	AccountID   string   `json:"accountId"`
	AccountType string   `json:"accountType"`
	Symbol      string   `json:"symbol"`
	Name        string   `json:"name"`
	Currency    string   `json:"currency"`
	Kind        string   `json:"kind"`
	ActivityID  string   `json:"activityId"`
	SecurityID  string   `json:"securityId"`
	RT          string   `json:"rt"`
	Flags       []string `json:"flags"`
}

func (l *Lot) clone() *Lot {
	c := *l
	c.Flags = append([]string{}, l.Flags...)
	return &c
}

// Slice is one closed lot: a lot matched against a fill.
type Slice struct {
	ID              string   `json:"id"`
	RT              string   `json:"rt"`
	AccountID       string   `json:"accountId"`
	AccountType     string   `json:"accountType"`
	Account         string   `json:"account"`
	Symbol          string   `json:"symbol"`
	Name            string   `json:"name"`
	Currency        string   `json:"currency"`
	Kind            string   `json:"kind"`
	Side            string   `json:"side"`
	Quantity        float64  `json:"quantity"`
	EntryPrice      float64  `json:"entryPrice"`
	ExitPrice       float64  `json:"exitPrice"`
	EntryDate       string   `json:"entryDate"`
	ExitDate        string   `json:"exitDate"`
	EntryWhen       string   `json:"entryWhen"`
	ExitWhen        string   `json:"exitWhen"`
	HoldDays        int      `json:"holdDays"`
	Commission      float64  `json:"commission"`
	EntryCommission float64  `json:"entryCommission"`
	ExitCommission  float64  `json:"exitCommission"`
	Pnl             float64  `json:"pnl"`
	PnlCad          float64  `json:"pnlCad"`
	FeesCad         float64  `json:"-"`
	HasFeesCad      bool     `json:"-"`
	OpenDirection   string   `json:"openDirection"`
	BuyActivityID   string   `json:"buyActivityId"`
	SellActivityID  string   `json:"sellActivityId"`
	SecurityID      string   `json:"securityId"`
	Flags           []string `json:"flags"`
}

func (s Slice) MarshalJSON() ([]byte, error) {
	type plain Slice
	if !s.HasFeesCad {
		return json.Marshal(plain(s))
	}
	type withFees struct {
		plain
		FeesCad float64 `json:"feesCad"`
	}
	return json.Marshal(withFees{plain(s), s.FeesCad})
}

// SlimSlice is a trade's leg as the page reads it.
type SlimSlice struct {
	Key            string   `json:"key"`
	Qty            float64  `json:"qty"`
	Entry          float64  `json:"entry"`
	Exit           float64  `json:"exit"`
	EntryDate      string   `json:"entryDate"`
	ExitDate       string   `json:"exitDate"`
	Pnl            float64  `json:"pnl"`
	PnlCad         float64  `json:"pnlCad"`
	Fees           float64  `json:"fees"`
	BuyActivityID  string   `json:"buyActivityId"`
	SellActivityID string   `json:"sellActivityId"`
	Flags          []string `json:"flags"`
}

// Fill is one execution as the trade page shows it.
type Fill struct {
	ID       string   `json:"id"`
	When     string   `json:"when"`
	Date     string   `json:"date"`
	Time     string   `json:"time"`
	Side     string   `json:"side"`
	Sub      string   `json:"sub"`
	Qty      float64  `json:"qty"`
	Price    float64  `json:"price"`
	Amount   float64  `json:"amount"`
	Fees     float64  `json:"fees"`
	Currency string   `json:"currency"`
	Flags    []string `json:"flags"`
}

// Summary is a trade's opening or closing side in one line.
type Summary struct {
	Qty   float64 `json:"qty"`
	Avg   float64 `json:"avg"`
	Fills int     `json:"fills"`
}

// TradeCore is a closed round trip without its legs and fills.
type TradeCore struct {
	ID            string   `json:"id"`
	Status        string   `json:"status"`
	Locked        bool     `json:"locked"`
	Symbol        string   `json:"symbol"`
	Underlying    string   `json:"underlying"`
	Name          string   `json:"name"`
	Exchange      string   `json:"exchange"`
	Kind          string   `json:"kind"`
	Currency      string   `json:"currency"`
	Account       string   `json:"account"`
	AccountID     string   `json:"accountId"`
	SecurityID    string   `json:"securityId"`
	Side          string   `json:"side"`
	OpenDirection string   `json:"openDirection"`
	Qty           float64  `json:"qty"`
	Mult          float64  `json:"mult"`
	Entry         float64  `json:"entry"`
	Exit          float64  `json:"exit"`
	EntryDate     string   `json:"entryDate"`
	ExitDate      string   `json:"exitDate"`
	EntryWhen     string   `json:"entryWhen"`
	ExitWhen      string   `json:"exitWhen"`
	HoldDays      int      `json:"holdDays"`
	Pnl           float64  `json:"pnl"`
	PnlCad        float64  `json:"pnlCad"`
	Fees          float64  `json:"fees"`
	FeesCad       float64  `json:"feesCad"`
	PnlPct        *float64 `json:"pnlPct"`
	LegCount      int      `json:"legCount"`
	Opened        Summary  `json:"opened"`
	Closed        Summary  `json:"closed"`
	NetCash       float64  `json:"netCash"`
	Flags         []string `json:"flags"`
	Grade         string   `json:"grade"`
	Thesis        string   `json:"thesis"`
	Tags          []string `json:"tags"`
}

// Trade is a closed round trip. Its legs and fills travel only when Detail is set.
type Trade struct {
	TradeCore
	Legs   []SlimSlice `json:"legs"`
	Fills  []Fill      `json:"fills"`
	Detail bool        `json:"-"`
}

type tradeFull struct {
	TradeCore
	Legs  []SlimSlice `json:"legs"`
	Fills []Fill      `json:"fills"`
}

// MarshalJSON writes the legs and fills only on the trade open on the page.
func (t Trade) MarshalJSON() ([]byte, error) {
	if t.Detail {
		return json.Marshal(tradeFull{t.TradeCore, nonNil(t.Legs), nonNilFills(t.Fills)})
	}
	return json.Marshal(t.TradeCore)
}

func nonNil(s []SlimSlice) []SlimSlice {
	if s == nil {
		return []SlimSlice{}
	}
	return s
}

func nonNilFills(s []Fill) []Fill {
	if s == nil {
		return []Fill{}
	}
	return s
}

// PositionLot is one open lot as the position page lists it.
type PositionLot struct {
	Opened     string   `json:"opened"`
	Qty        float64  `json:"qty"`
	Price      float64  `json:"price"`
	Basis      float64  `json:"basis"`
	Held       int      `json:"held"`
	Flags      []string `json:"flags"`
	ActivityID string   `json:"activityId"`
}

// PositionCore is an open position without its lots and fills.
type PositionCore struct {
	ID            string        `json:"id"`
	Symbol        string        `json:"symbol"`
	Underlying    string        `json:"underlying"`
	Name          string        `json:"name"`
	Exchange      string        `json:"exchange"`
	Kind          string        `json:"kind"`
	Account       string        `json:"account"`
	AccountID     string        `json:"accountId"`
	Currency      string        `json:"currency"`
	SecurityID    string        `json:"securityId"`
	Short         bool          `json:"short"`
	Qty           float64       `json:"qty"`
	Mult          float64       `json:"mult"`
	Avg           float64       `json:"avg"`
	Cost          float64       `json:"cost"`
	Fees          float64       `json:"fees"`
	Last          float64       `json:"last"`
	LastAt        string        `json:"lastAt"`
	PriceSource   string        `json:"priceSource"`
	PriceChange   *float64      `json:"priceChange"`
	PercentChange *float64      `json:"percentChange"`
	DayChange     *float64      `json:"dayChange"`
	MV            float64       `json:"mv"`
	Unreal        float64       `json:"unreal"`
	UnrealPct     *float64      `json:"unrealPct"`
	Held          int           `json:"held"`
	Opened        string        `json:"opened"`
	WsQty         *float64      `json:"wsQty"`
	RT            *string       `json:"rt"`
	Lots          []PositionLot `json:"lots"`
	Grade         string        `json:"grade"`
	Thesis        string        `json:"thesis"`
	Tags          []string      `json:"tags"`
	Alloc         float64       `json:"alloc"`
}

// Position is an open position. Its fills travel only when Detail is set.
type Position struct {
	PositionCore
	Fills  []Fill `json:"fills"`
	Detail bool   `json:"-"`
}

type positionFull struct {
	PositionCore
	Fills []Fill `json:"fills"`
}

// MarshalJSON writes the fills only on the holding open on the page.
func (p Position) MarshalJSON() ([]byte, error) {
	if p.Detail {
		return json.Marshal(positionFull{p.PositionCore, nonNilFills(p.Fills)})
	}
	return json.Marshal(p.PositionCore)
}

// CashRow is one Dividend, Interest, Withholding tax or Interest charge activity.
type CashRow struct {
	ID        string   `json:"id"`
	Date      string   `json:"date"`
	Time      string   `json:"time"`
	Symbol    string   `json:"symbol"`
	Name      string   `json:"name"`
	Kind      string   `json:"kind"`
	Account   string   `json:"account"`
	AccountID string   `json:"accountId"`
	Qty       *float64 `json:"qty"`
	Per       *float64 `json:"per"`
	Amount    float64  `json:"amount"`
	Currency  string   `json:"currency"`
	AmountCad float64  `json:"amountCad"`
}

// Unmatched is a sell no lot could be found for.
type Unmatched struct {
	Symbol      string  `json:"symbol"`
	Currency    string  `json:"currency"`
	Side        string  `json:"side"`
	Quantity    float64 `json:"quantity"`
	Price       float64 `json:"price"`
	Date        string  `json:"date"`
	Description string  `json:"description"`
	AccountID   string  `json:"accountId"`
	Account     string  `json:"account"`
	ActivityID  string  `json:"activityId"`
}

// FIFOResult is what the match produces.
type FIFOResult struct {
	Closed    []*Slice
	Open      []*Lot
	Unmatched []Unmatched
}

// EquityPoint is one day of the equity series.
type EquityPoint struct {
	D   string   `json:"d"`
	V   float64  `json:"v"`
	Dep *float64 `json:"dep"`
}

// LastPrice is the newest fill with a price for a symbol.
type LastPrice struct {
	Price float64 `json:"price"`
	Date  string  `json:"date"`
}

// AccountRow is an account as the view lists it.
type AccountRow struct {
	ID       string   `json:"id"`
	Name     string   `json:"name"`
	Type     string   `json:"type"`
	Currency string   `json:"currency"`
	Status   string   `json:"status"`
	Nav      *float64 `json:"nav"`
}

func isOption(symbol string) bool  { return symbols.IsOption(symbol) }
func underlying(symbol string) string { return symbols.Underlying(symbol) }
func multiplier(symbol string) float64 { return symbols.Multiplier(symbol) }
