package netio

import (
	"context"
	"errors"
	"io"
	"net/http"
	"net/http/httptest"
	"testing"
	"time"
)

func fetch(t *testing.T, url string, idle time.Duration) ([]byte, error) {
	t.Helper()
	ctx, g := NewGuard(context.Background(), idle)
	defer g.Stop()
	req, _ := http.NewRequestWithContext(ctx, http.MethodGet, url, nil)
	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		return nil, g.Err(err)
	}
	defer resp.Body.Close()
	return io.ReadAll(g.Body(resp.Body))
}

func TestASlowButMovingBodyIsNeverCutOff(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		f := w.(http.Flusher)
		for i := 0; i < 8; i++ {
			w.Write([]byte("x"))
			f.Flush()
			time.Sleep(60 * time.Millisecond)
		}
	}))
	defer srv.Close()
	out, err := fetch(t, srv.URL, 150*time.Millisecond)
	if err != nil || len(out) != 8 {
		t.Fatalf("got %d bytes, %v: the whole transfer took longer than the idle limit but never stalled", len(out), err)
	}
}

func TestASilentBodyIsCutAfterTheIdleLimit(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Write([]byte("x"))
		w.(http.Flusher).Flush()
		time.Sleep(2 * time.Second)
	}))
	defer srv.Close()
	start := time.Now()
	_, err := fetch(t, srv.URL, 150*time.Millisecond)
	if !errors.Is(err, ErrStalled) {
		t.Fatalf("err = %v, want a stall", err)
	}
	if time.Since(start) > time.Second {
		t.Fatalf("took %s to notice the stall", time.Since(start))
	}
}

func TestSilenceBeforeTheHeadersIsAStallToo(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) { time.Sleep(2 * time.Second) }))
	defer srv.Close()
	_, err := fetch(t, srv.URL, 150*time.Millisecond)
	if !errors.Is(err, ErrStalled) {
		t.Fatalf("err = %v, want a stall", err)
	}
}
