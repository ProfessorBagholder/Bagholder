package news

import (
	"regexp"
	"strings"

	"github.com/ProfessorBagholder/Bagholder/internal/py"
)

const SummaryChars = 400 // a sentence or two of what a source said beneath its headline

// What a wire prints before its first sentence: the wire's name, a city and a date, in either
// language and in either house style — `(TheNewswire) Varennes, Quebec - TheNewswire - le 8 septembre
// 2026 - `, `TORONTO, Sept. 08, 2026 (GLOBE NEWSWIRE) -- `, `VANCOUVER, BC / ACCESSWIRE / September 8,
// 2026 / `. The row already carries the wire and the day, so the dateline is taken off rather than
// shown as the summary's opening words.
const wires = `globe ?newswire|business ?wire|cnw(?: group)?|newsfile(?: corp)?|accesswire|the ?news ?wire|pr ?newswire|newmediawire|marketwired|cision`

var datelines = []*regexp.Regexp{
	regexp.MustCompile(`(?i)^.{0,80}?(?:\((?:` + wires + `)[^)]*\)|(?:` + wires + `))[^.]{0,60}?[-–—]{1,2}\s+`),
	regexp.MustCompile(`(?i)^[^./]{0,60}/\s*(?:` + wires + `)\s*/[^./]{0,40}/\s*`),
}

var leadingDate = regexp.MustCompile(`^(?:le\s+)?\d{1,2}(?:er)?\s+[a-zA-ZéûÉÛ]{3,10}\.?\s+\d{4}\s*[-–—,]?\s+|^[A-Za-zéûÉÛ]{3,10}\.?\s+\d{1,2},?\s+\d{4}\s*[-–—,]?\s+`)

var tagRE = regexp.MustCompile(`<[^>]+>`)
var wordish = regexp.MustCompile(`[^0-9A-Za-zÀ-ÿ]`)

// A wire's dateline off the front of its own summary, however many times it prints one, and the
// bare date some leave behind (`le 8 septembre 2026 – `).
func StripDateline(text string) string {
	for i := 0; i < 3; i++ {
		cut := text
		for _, p := range datelines {
			cut = replaceFirst(p, cut, "")
		}
		cut = strings.TrimLeft(replaceFirst(leadingDate, strings.TrimLeft(cut, " -–—,/"), ""), " -–—,/")
		if cut == text {
			return text
		}
		text = cut
	}
	return text
}

func replaceFirst(re *regexp.Regexp, s, with string) string {
	loc := re.FindStringIndex(s)
	if loc == nil {
		return s
	}
	return s[:loc[0]] + with + s[loc[1]:]
}

// What a source said under the headline, as plain text: tags out, one space between words, cut at a
// sentence end rather than mid-word. A summary that only repeats the headline is not one, and
// neither is a feed's markup (Google's `description` is an anchor and a publisher).
func SummaryText(raw, headline string) string {
	text := StripDateline(CleanText(tagRE.ReplaceAllString(py.S(raw), " ")))
	// a source that carries a placeholder instead of a summary ("...", "-", "N/A") has none
	if len(wordish.ReplaceAllString(text, "")) < 12 {
		return ""
	}
	t, h := NewsText(text), NewsText(headline)
	if text == "" || t == h || (strings.HasPrefix(t, h) && h != "" && len(text) < len(headline)+12) {
		return ""
	}
	if len(text) <= SummaryChars {
		return text
	}
	cut := text[:SummaryChars]
	stop := maxInt(strings.LastIndex(cut, ". "), maxInt(strings.LastIndex(cut, "? "), strings.LastIndex(cut, "! ")))
	if stop > SummaryChars/2 {
		return strings.TrimSpace(cut[:stop+1])
	}
	return strings.TrimSpace(strings.TrimRight(cut, " ") + "…")
}

func maxInt(a, b int) int {
	if a > b {
		return a
	}
	return b
}
