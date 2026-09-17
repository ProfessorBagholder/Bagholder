package store

import (
	"database/sql"
	"errors"
	"fmt"
	"sync"
	"testing"

	"github.com/ProfessorBagholder/Bagholder/internal/py"
)

func TestAnAbandonedTransactionIsRolledBackNotHandedOn(t *testing.T) {
	s := temp(t)
	err := s.tx(func(tx *sql.Tx) error {
		if _, err := tx.Exec("INSERT INTO meta(key, value) VALUES ('probe', 'uncommitted')"); err != nil {
			return err
		}
		return errors.New("a call site that raised before its commit")
	})
	if err == nil {
		t.Fatal("the failure is reported")
	}
	var v string
	if err := s.DB().QueryRow("SELECT value FROM meta WHERE key='probe'").Scan(&v); err != sql.ErrNoRows {
		t.Errorf("and cannot see, or commit, the abandoned write: %v %q", err, v)
	}
	if got := s.GetMeta("probe"); got != "" {
		t.Errorf("the abandoned write never lands: %q", got)
	}
}

func TestConcurrentReadersAndWritersAgree(t *testing.T) {
	s := temp(t)
	var mu sync.Mutex
	var failures []string
	var wg sync.WaitGroup
	guard := func(f func()) {
		defer wg.Done()
		defer func() {
			if r := recover(); r != nil {
				mu.Lock()
				failures = append(failures, fmt.Sprint(r))
				mu.Unlock()
			}
		}()
		f()
	}
	wg.Add(5)
	go guard(func() {
		for i := 0; i < 40; i++ {
			s.UpsertQuote(fmt.Sprintf("SYM%d", i%5), Quote{Price: py.Ptr(float64(i)), Currency: "CAD"}, "tmx")
		}
	})
	for r := 0; r < 4; r++ {
		go guard(func() {
			for i := 0; i < 40; i++ {
				s.DataVersion()
				s.StatusCounts()
				s.Quotes()
			}
		})
	}
	wg.Wait()
	if len(failures) != 0 {
		t.Fatalf("no reader or writer failed: %v", failures)
	}
	if n := s.DB().Stats().OpenConnections; n > 4 {
		t.Errorf("the pool does not grow without bound: %d", n)
	}
}
