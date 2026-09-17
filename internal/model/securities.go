package model

import (
	"strings"

	"github.com/ProfessorBagholder/Bagholder/internal/py"
	"github.com/ProfessorBagholder/Bagholder/internal/store"
	"github.com/ProfessorBagholder/Bagholder/internal/symbols"
)

var exchAlias = map[string]string{
	"TSXV":        "TSX-V",
	"TSX-V":       "TSX-V",
	"TSX VENTURE": "TSX-V",
	"CDNX":        "TSX-V",
	"VENTURE":     "TSX-V",
	"TORONTO":     "TSX",
	"TSX":         "TSX",
	"CBOE CANADA": "Cboe Canada",
	"CBOE CA":     "Cboe Canada",
	"NEO":         "Cboe Canada",
}

var micMap = map[string]string{
	"XTSV": "TSX-V",
	"XTSX": "TSX",
	"XNAS": "NASDAQ",
	"XNYS": "NYSE",
	"XASE": "NYSE American",
	"ARCX": "NYSE Arca",
	"XCNQ": "CSE",
	"NEOE": "Cboe Canada",
}

func ExchangeLabel(sec *store.Security) string {
	if sec == nil {
		return ""
	}
	raw := py.Strip(sec.PrimaryExchange)
	up := strings.ToUpper(raw)
	if v, ok := exchAlias[up]; ok {
		return v
	}
	if raw != "" {
		return raw
	}
	return micMap[strings.ToUpper(sec.PrimaryMic)]
}

func listingTicker(sym string) string { return symbols.ListingTicker(sym) }

func isAlphaVenue(sec *store.Security) bool {
	if sec == nil {
		return false
	}
	exch := strings.ToUpper(sec.PrimaryExchange)
	mic := strings.ToUpper(sec.PrimaryMic)
	return exch == "ALPHA EXCHANGE" || exch == "ALPHA" || mic == "XATS"
}

type Securities struct {
	ByID  map[string]*store.Security
	order []string
}

func NewSecurities(rows []store.Security) *Securities {
	s := &Securities{ByID: map[string]*store.Security{}}
	for i := range rows {
		r := &rows[i]
		if r.ID == "" {
			continue
		}
		if _, ok := s.ByID[r.ID]; !ok {
			s.order = append(s.order, r.ID)
		}
		s.ByID[r.ID] = r
	}
	return s
}

func (s *Securities) Preferred(sec *store.Security) *store.Security {
	if sec == nil || !isAlphaVenue(sec) {
		return sec
	}
	sym := listingTicker(sec.Symbol)
	ccy := sec.Currency
	if sym == "" {
		return sec
	}
	for _, id := range s.order {
		other := s.ByID[id]
		if other == sec || other.UnderlyingID != "" {
			continue
		}
		if listingTicker(other.Symbol) != sym {
			continue
		}
		if ccy != "" && other.Currency != "" && other.Currency != ccy {
			continue
		}
		if isAlphaVenue(other) || ExchangeLabel(other) == "" {
			continue
		}
		return other
	}
	return sec
}

func (s *Securities) Listing(securityID string) *store.Security {
	sec := s.ByID[securityID]
	if sec != nil && sec.UnderlyingID != "" {
		if under := s.ByID[sec.UnderlyingID]; under != nil {
			sec = under
		}
	}
	return s.Preferred(sec)
}

func (s *Securities) Exchange(securityID string) string {
	return ExchangeLabel(s.Listing(securityID))
}

func (s *Securities) Name(securityID, fallback string) string {
	if sec := s.Listing(securityID); sec != nil && sec.Name != "" {
		return sec.Name
	}
	return fallback
}

func (s *Securities) CashCurrencies() map[string]string {
	out := map[string]string{}
	for _, sid := range s.order {
		sec := s.ByID[sid]
		sym := strings.ToUpper(sec.Symbol)
		if sym == "CAD" || sym == "USD" || strings.HasPrefix(sid, "sec-c-") {
			ccy := strings.ToUpper(sec.Currency)
			if ccy == "" {
				ccy = sym
			}
			out[sid] = ccy
		}
	}
	return out
}
