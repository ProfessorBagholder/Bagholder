package disclosures

import "testing"

func defaultPipeline() *Pipeline {
	s := NewSedar()
	return &Pipeline{Providers: []Provider{s, NewEdgar(nil)}, Sedar: s}
}

func TestStaleCategoryIsCorrected(t *testing.T) {
	p := defaultPipeline()
	if got := p.Categorize(Row{Source: "SEC", Type: "F-X", Category: "Offerings"}); got != Other {
		t.Errorf("Categorize(F-X) = %q", got)
	}
	if got := p.Categorize(Row{Source: "SEC", Type: "F-1", Category: "Other"}); got != Offerings {
		t.Errorf("Categorize(F-1) = %q", got)
	}
}

func TestUnknownSourceKeepsStored(t *testing.T) {
	p := defaultPipeline()
	if got := p.Categorize(Row{Source: "???", Type: "X", Category: "Financials"}); got != "Financials" {
		t.Errorf("Categorize(unknown source) = %q", got)
	}
}
