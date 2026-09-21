package netio

import (
	"context"
	"errors"
	"fmt"
	"io"
	"sync"
	"time"
)

var ErrStalled = errors.New("no data")

type Guard struct {
	idle    time.Duration
	timer   *time.Timer
	cancel  context.CancelFunc
	mu      sync.Mutex
	stalled bool
}

func NewGuard(parent context.Context, idle time.Duration) (context.Context, *Guard) {
	ctx, cancel := context.WithCancel(parent)
	g := &Guard{idle: idle, cancel: cancel}
	if idle > 0 {
		g.timer = time.AfterFunc(idle, func() {
			g.mu.Lock()
			g.stalled = true
			g.mu.Unlock()
			cancel()
		})
	}
	return ctx, g
}

func (g *Guard) Touch() {
	if g.timer != nil {
		g.timer.Reset(g.idle)
	}
}

func (g *Guard) Stop() {
	if g.timer != nil {
		g.timer.Stop()
	}
	g.cancel()
}

func (g *Guard) Err(err error) error {
	if err == nil {
		return nil
	}
	g.mu.Lock()
	stalled := g.stalled
	g.mu.Unlock()
	if stalled {
		return fmt.Errorf("%w for %s", ErrStalled, g.idle)
	}
	return err
}

type body struct {
	rc io.ReadCloser
	g  *Guard
}

func (b *body) Read(p []byte) (int, error) {
	n, err := b.rc.Read(p)
	if n > 0 {
		b.g.Touch()
	}
	return n, b.g.Err(err)
}

func (b *body) Close() error {
	b.g.Stop()
	return b.rc.Close()
}

func (g *Guard) Body(rc io.ReadCloser) io.ReadCloser {
	g.Touch()
	return &body{rc: rc, g: g}
}
