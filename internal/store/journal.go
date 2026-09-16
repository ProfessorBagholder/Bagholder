package store

import (
	"encoding/json"
	"strings"

	"github.com/ProfessorBagholder/Bagholder/internal/py"
)

// JournalEntry is a trade's or position's thesis, tags and grade.
type JournalEntry struct {
	Thesis string   `json:"thesis"`
	Tags   []string `json:"tags"`
	Grade  string   `json:"grade"`
}

var grades = map[string]bool{"A": true, "B": true, "C": true, "F": true}

// CleanJournalEntry validates one entry, nil when it carries nothing.
func CleanJournalEntry(val any) *JournalEntry {
	m, ok := val.(map[string]any)
	if !ok {
		return nil
	}
	thesis := py.S(m["thesis"])
	grade := strings.ToUpper(strings.TrimSpace(py.S(m["grade"])))
	if !grades[grade] {
		grade = ""
	}
	tags := []string{}
	var rawTags []any
	switch t := m["tags"].(type) {
	case string:
		for _, x := range strings.Split(t, ",") {
			rawTags = append(rawTags, x)
		}
	case []any:
		rawTags = t
	case []string:
		for _, x := range t {
			rawTags = append(rawTags, x)
		}
	}
	for _, t := range rawTags {
		s := strings.TrimSpace(py.S(t))
		if s != "" && !py.Contains(tags, s) {
			tags = append(tags, s)
		}
	}
	if thesis == "" && grade == "" && len(tags) == 0 {
		return nil
	}
	return &JournalEntry{Thesis: thesis, Tags: tags, Grade: grade}
}

func cleanJournal(raw any) map[string]JournalEntry {
	out := map[string]JournalEntry{}
	m, ok := raw.(map[string]any)
	if !ok {
		return out
	}
	for key, val := range m {
		kid := strings.TrimSpace(key)
		if e := CleanJournalEntry(val); kid != "" && e != nil {
			out[kid] = *e
		}
	}
	return out
}

// Journal is the v2 journal: trade or position id -> entry.
func (s *Store) Journal() map[string]JournalEntry {
	raw := s.GetMeta(JournalMeta)
	if raw == "" {
		return map[string]JournalEntry{}
	}
	var data any
	if json.Unmarshal([]byte(raw), &data) != nil {
		return map[string]JournalEntry{}
	}
	return cleanJournal(data)
}

func journalToAny(entries map[string]JournalEntry) map[string]any {
	out := map[string]any{}
	for k, e := range entries {
		tags := make([]any, len(e.Tags))
		for i, t := range e.Tags {
			tags[i] = t
		}
		out[k] = map[string]any{"thesis": e.Thesis, "tags": tags, "grade": e.Grade}
	}
	return out
}

// SaveJournal cleans and saves the whole journal.
func (s *Store) SaveJournal(entries map[string]JournalEntry) map[string]JournalEntry {
	clean := cleanJournal(journalToAny(entries))
	b, _ := json.Marshal(clean)
	s.SetMeta(JournalMeta, string(b))
	return clean
}

// SaveJournalEntry merges one entry; an entry with no thesis, grade or tags deletes the key.
func (s *Store) SaveJournalEntry(key string, entry any) map[string]JournalEntry {
	kid := strings.TrimSpace(key)
	if kid == "" {
		return s.Journal()
	}
	current := s.Journal()
	if clean := CleanJournalEntry(entry); clean != nil {
		current[kid] = *clean
	} else {
		delete(current, kid)
	}
	b, _ := json.Marshal(current)
	s.SetMeta(JournalMeta, string(b))
	return current
}
