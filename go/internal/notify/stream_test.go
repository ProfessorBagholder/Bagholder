package notify

import (
	"fmt"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/ProfessorBagholder/Bagholder/internal/py"
	"github.com/ProfessorBagholder/Bagholder/internal/store"
)

type chunks struct {
	mu  sync.Mutex
	out []string
}

func (c *chunks) Write(p []byte) (int, error) {
	c.mu.Lock()
	defer c.mu.Unlock()
	c.out = append(c.out, string(p))
	return len(p), nil
}

func (c *chunks) all() []string {
	c.mu.Lock()
	defer c.mu.Unlock()
	return append([]string{}, c.out...)
}

func streamed(t *testing.T, n *Notifier, after *int64, alive func() bool) []string {
	t.Helper()
	w := &chunks{}
	done := make(chan struct{})
	go func() {
		n.Stream(w, func() {}, after, alive, 0.05)
		close(done)
	}()
	select {
	case <-done:
	case <-time.After(5 * time.Second):
		t.Fatalf("the stream did not end: %q", w.all())
	}
	return w.all()
}

func TestTheStreamSendsEveryRowAfterTheIDThePageBringsWithPingsBetween(t *testing.T) {
	n, st := setUp(t)
	n.SetSettings(map[string]any{"fills": true})
	old := n.Emit("fills", "old", "Old", "b", nil)
	st.MarkNotificationsSeen([]int64{old.ID})
	first := n.Emit("fills", "first", "First", "b", nil)
	var second *store.Notification
	ticks := 0
	got := streamed(t, n, py.PtrInt64(old.ID), func() bool {
		ticks++
		if ticks == 3 {
			second = n.Emit("fills", "second", "Second", "b", nil)
		}
		return ticks <= 3
	})
	if len(got) != 4 {
		t.Fatalf("alive said no: the stream ends: %q", got)
	}
	hello, row1, ping, row2 := got[0], got[1], got[2], got[3]
	if hello != ": bagholder\n\n" {
		t.Fatalf("hello: %q", hello)
	}
	if !strings.HasPrefix(row1, fmt.Sprintf("id: %d\ndata: ", first.ID)) || !strings.Contains(row1, `"title":"First"`) {
		t.Fatal(row1)
	}
	if ping != ": ping\n\n" {
		t.Fatalf("nothing new by the heartbeat: a comment keeps the connection: %q", ping)
	}
	if !strings.HasPrefix(row2, fmt.Sprintf("id: %d\ndata: ", second.ID)) {
		t.Fatal(row2)
	}
	log := fakeTools(t)
	fakeMacApp(t, n)
	t.Setenv(ModeEnv, "")
	t.Setenv("DISPLAY", ":0")
	var third *store.Notification
	ticks = 0
	got = streamed(t, n, nil, func() bool {
		ticks++
		if ticks == 2 {
			third = n.Emit("fills", "third", "Third", "b", nil)
		}
		return ticks <= 2
	})
	if len(got) != 3 || got[0] != ": bagholder\n\n" || got[1] != ": ping\n\n" {
		t.Fatalf("no id: only what is made after the stream opens: %q", got)
	}
	chunk := got[2]
	if !strings.HasPrefix(chunk, fmt.Sprintf("id: %d\n", third.ID)) || !strings.Contains(chunk, `"seenAt":"20`) {
		t.Fatal("a row the server posts itself still reaches the history, already seen: " + chunk)
	}
	waitCalls(t, log, 1)
}
