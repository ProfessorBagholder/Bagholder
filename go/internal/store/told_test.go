package store

import "testing"

func TestAnEventTheStreamHasMetIsRecognised(t *testing.T) {
	s := temp(t)
	if got := s.EventsTold("news:QNC@TSX-V", []string{"a", "b"}); len(got) != 0 {
		t.Fatalf("an untouched stream has met %v", got)
	}
	s.MarkTold("news:QNC@TSX-V", []string{"a", "b"}, "")
	got := s.EventsTold("news:QNC@TSX-V", []string{"a", "b", "c"})
	if !got["a"] || !got["b"] || got["c"] {
		t.Errorf("met = %v, want a and b only", got)
	}
}

func TestAStreamKeepsItsOwnEvents(t *testing.T) {
	s := temp(t)
	s.MarkTold("news:QNC@TSX-V", []string{"a"}, "")
	if s.EventsTold("news:CH@TSX-V", []string{"a"})["a"] {
		t.Error("an event met on one stream counts on another")
	}
}

func TestBlankEventsAreNeitherStoredNorAsked(t *testing.T) {
	s := temp(t)
	if n := s.MarkTold("filings:QNC:SEDAR+", []string{"", "   "}, ""); n != 0 {
		t.Errorf("stored %d blank events", n)
	}
	if got := s.EventsTold("filings:QNC:SEDAR+", []string{""}); len(got) != 0 {
		t.Errorf("met = %v for a blank event", got)
	}
}

func TestMoreEventsThanOneChunkAreAllRecognised(t *testing.T) {
	s := temp(t)
	events := make([]string, 950)
	for i := range events {
		events[i] = "e" + itoa(i)
	}
	s.MarkTold("news:BIG@TSX", events, "")
	got := s.EventsTold("news:BIG@TSX", events)
	if len(got) != len(events) {
		t.Errorf("met %d of %d across chunks", len(got), len(events))
	}
}

func itoa(i int) string {
	if i == 0 {
		return "0"
	}
	out := ""
	for i > 0 {
		out = string(rune('0'+i%10)) + out
		i /= 10
	}
	return out
}
