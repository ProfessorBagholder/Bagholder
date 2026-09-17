package market

import (
	"sync"
	"time"
)

type Pacer struct {
	mu   sync.Mutex
	last map[string]time.Time
}

func NewPacer() *Pacer { return &Pacer{last: map[string]time.Time{}} }

func (p *Pacer) Pace(host string, seconds float64) {
	p.mu.Lock()
	wait := p.last[host].Add(time.Duration(seconds * float64(time.Second))).Sub(time.Now())
	if wait > 0 {
		p.mu.Unlock()
		time.Sleep(wait)
		p.mu.Lock()
	}
	p.last[host] = time.Now()
	p.mu.Unlock()
}
