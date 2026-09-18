package news

import "strings"
import "testing"

func TestAWiresDatelineIsTakenOffItsOwnSummary(t *testing.T) {
	for _, c := range []struct{ raw, want string }{
		{"TORONTO, Sept. 08, 2026 (GLOBE NEWSWIRE) -- The company reported record revenue for the quarter.", "The company reported record revenue for the quarter."},
		{"VANCOUVER, BC / ACCESSWIRE / September 8, 2026 / The company closed its financing today.", "The company closed its financing today."},
		{"(TheNewswire) Varennes, Quebec - TheNewswire - le 8 septembre 2026 - La societe annonce une entente importante.", "La societe annonce une entente importante."},
	} {
		if got := SummaryText(c.raw, "Something else entirely"); got != c.want {
			t.Errorf("SummaryText(%.40q) = %q, want %q", c.raw, got, c.want)
		}
	}
}

func TestASummaryThatOnlyRepeatsTheHeadlineIsNotOne(t *testing.T) {
	head := "Aegis Announces August 2026 Distributions"
	if got := SummaryText(head, head); got != "" {
		t.Errorf("a repeat of the headline read as a summary: %q", got)
	}
	if got := SummaryText("<a href=\"x\">...</a>", head); got != "" {
		t.Errorf("a placeholder read as a summary: %q", got)
	}
	if got := SummaryText("N/A", head); got != "" {
		t.Errorf("a placeholder read as a summary: %q", got)
	}
}

func TestTagsComeOutAndALongSummaryIsCutAtASentence(t *testing.T) {
	if got := SummaryText("<p>The board <b>approved</b> the plan.</p>", "A headline"); got != "The board approved the plan." {
		t.Errorf("tags survived: %q", got)
	}
	long := strings.Repeat("The company said a great many things about the quarter. ", 20)
	got := SummaryText(long, "A headline")
	if len(got) > SummaryChars {
		t.Errorf("summary is %d characters, over the cap", len(got))
	}
	if !strings.HasSuffix(got, ".") && !strings.HasSuffix(got, "…") {
		t.Errorf("summary was cut mid-word: %q", got[len(got)-20:])
	}
}
