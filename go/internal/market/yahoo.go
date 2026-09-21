package market

import (
	"errors"
	"fmt"
	"strings"
	"sync"
	"time"

	"github.com/ProfessorBagholder/Bagholder/internal/browserhttp"
)

const (
	YahooWarmURL    = "https://finance.yahoo.com/quote/AAPL/"
	YahooCrumbURL   = "https://query1.finance.yahoo.com/v1/test/getcrumb"
	YahooSummaryURL = "https://query1.finance.yahoo.com/v10/finance/quoteSummary/%s?modules=%s&crumb=%s"
)

var ErrYahooCrumb = errors.New("yahoo: no crumb")

type YahooDoer interface {
	Get(rawURL string, headers map[string]string) (*browserhttp.Response, error)
}

type Yahoo struct {
	Open func() (YahooDoer, error)

	turn         sync.Mutex
	mu           sync.Mutex
	nextAt       time.Time
	backoffUntil time.Time

	open    sync.Mutex
	session YahooDoer
	crumb   string
}

func openBrowser() (YahooDoer, error) {
	s, err := browserhttp.New(TimeoutSec, true)
	if err != nil {
		return nil, err
	}
	return s, nil
}

func NewYahoo() *Yahoo {
	return &Yahoo{Open: openBrowser}
}

func (y *Yahoo) SetBackoff(until time.Time) {
	y.mu.Lock()
	y.backoffUntil = until
	y.mu.Unlock()
}

func (y *Yahoo) take() bool {
	for {
		y.mu.Lock()
		now := time.Now()
		if now.Before(y.backoffUntil) {
			y.mu.Unlock()
			return false
		}
		wait := y.nextAt.Sub(now)
		if wait <= 0 {
			y.nextAt = now.Add(time.Duration(YahooMinIntervalSec * float64(time.Second)))
			y.mu.Unlock()
			return true
		}
		y.mu.Unlock()
		time.Sleep(wait)
	}
}

func (y *Yahoo) refused() {
	y.mu.Lock()
	y.backoffUntil = time.Now().Add(YahooBackoffSec * time.Second)
	y.mu.Unlock()
}

func (y *Yahoo) Paced(call func() (*browserhttp.Response, error)) (*browserhttp.Response, error) {
	y.turn.Lock()
	defer y.turn.Unlock()
	if !y.take() {
		return nil, ErrBackingOff
	}
	resp, err := call()
	if err != nil {
		return nil, err
	}
	if resp.Status == 429 {
		y.refused()
	}
	return resp, nil
}

func (y *Yahoo) Session() (YahooDoer, string, error) {
	y.open.Lock()
	defer y.open.Unlock()
	if y.session != nil {
		return y.session, y.crumb, nil
	}
	session, err := y.Open()
	if err != nil {
		return nil, "", err
	}
	if _, err := session.Get(YahooWarmURL, nil); err != nil {
		return nil, "", err
	}
	resp, err := session.Get(YahooCrumbURL, nil)
	if err != nil {
		return nil, "", err
	}
	crumb := strings.TrimSpace(string(resp.Body))
	if crumb == "" || len(crumb) > 32 {
		return nil, "", ErrYahooCrumb
	}
	y.session, y.crumb = session, crumb
	return session, crumb, nil
}

func (y *Yahoo) Summary(symbol, modules string) (*browserhttp.Response, error) {
	session, crumb, err := y.Session()
	if err != nil {
		return nil, err
	}
	return y.Paced(func() (*browserhttp.Response, error) {
		return session.Get(fmt.Sprintf(YahooSummaryURL, symbol, modules, crumb), nil)
	})
}
