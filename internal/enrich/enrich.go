package enrich

import (
	"encoding/hex"
	"html"
	"regexp"
	"strings"
	"unicode"
	"unicode/utf16"

	"github.com/ProfessorBagholder/Bagholder/internal/py"
)

const MaxText = 8000

var (
	tagsRE      = regexp.MustCompile(`<[^>]+>`)
	titleLitRE  = regexp.MustCompile(`/Title\s*\(((?:[^()\\]|\\.)*)\)`)
	titleHexRE  = regexp.MustCompile(`/Title\s*<([0-9A-Fa-f]+)>`)
	langTailRE  = regexp.MustCompile(`(?i)[_\-\s]*(FINAL|DRAFT|REVISED|v\d+|EN|FR|ENG?|FRE?|English|French)\b`)
	dateRE      = regexp.MustCompile(`\d{4}[-_]\d{2}[-_]\d{2}`)
	ctrlRE      = regexp.MustCompile("[\x00-\x08\x0b\x0c\x0e-\x1f\x7f�]")
	titlePunct  = " -‐–—'’&(),.:;/%+#?!\"“”"
	toolingRE   = regexp.MustCompile(`(?i)^\s*(Microsoft Word|Microsoft PowerPoint|Adobe \w+|Acrobat)\s*-\s*`)
	extRE       = regexp.MustCompile(`(?i)\.(pdf|docx?|pptx?|rtf|txt)\s*$`)
	noiseWordRE = regexp.MustCompile(`(?i)\b(PR|FINAL|NR|DRAFT|REVISED|v\d+)\b`)
	lettersRE   = regexp.MustCompile(`[A-Za-z]{3,}`)
	secHeaderRE = regexp.MustCompile(`(?i)^\s*[\w.\-]{1,12}\s+\d+\s+\S+\.(?:htm|html|txt|xml)\s+`)
	secExRE     = regexp.MustCompile(`(?i)^\s*(?:form\s+\S+\s+)?(?:exhibit\s+[\d.]+\s+){1,3}`)
	scriptRE    = regexp.MustCompile(`(?is)<(script|style|head)[^>]*>.*?</(script|style|head)>`)
	hedgeRE     = regexp.MustCompile(`(?i)\b(likely|probably|presumably|apparently|possibly|perhaps|seems?\s+to|appears?\s+to|may\s+be|might\s+be|could\s+be|suggests?\s+that|unclear|not\s+specified|unspecified|i\s+think|it\s+is\s+not\s+clear)\b`)
	preambleRE  = regexp.MustCompile(`(?i)^\s*(sure[,!.]?\s+)?(here(?:'?s| is| are)\b[^:]*:?\s*)`)
	labelRE     = regexp.MustCompile(`(?i)^\s*(title|summary|answer)\s*[:\-]\s*`)
	leadJunkRE  = regexp.MustCompile(`^[*#>\-\s]+`)
	specialRE   = regexp.MustCompile(`<\|[^>]*\|>`)
	exNumRE     = regexp.MustCompile(`\bex-?\d`)
	fiveDigRE   = regexp.MustCompile(`\d{5,}`)
	stopRE      = regexp.MustCompile(`[.!?]+`)
	initialRE   = regexp.MustCompile(`^(?:[a-z]\.)*[a-z]$`)
	lowerWordRE = regexp.MustCompile(`\b[a-z]{3,}\b`)
)

var abbrev = map[string]bool{"corp": true, "inc": true, "ltd": true, "co": true, "llc": true, "llp": true, "plc": true, "lp": true, "sa": true, "nv": true, "ag": true, "cie": true, "pte": true,
	"jr": true, "sr": true, "mr": true, "mrs": true, "ms": true, "dr": true, "prof": true, "st": true, "no": true, "nos": true, "vs": true, "etc": true, "approx": true, "al": true}

func decodeLatin1(b []byte) string {
	r := make([]rune, len(b))
	for i, c := range b {
		r[i] = rune(c)
	}
	return string(r)
}

func decodeUTF16(b []byte, big bool) string {
	u := make([]uint16, 0, len(b)/2)
	for i := 0; i+1 < len(b); i += 2 {
		if big {
			u = append(u, uint16(b[i])<<8|uint16(b[i+1]))
		} else {
			u = append(u, uint16(b[i+1])<<8|uint16(b[i]))
		}
	}
	return string(utf16.Decode(u))
}

func ExtractPDFSubject(data []byte) string {
	m := titleLitRE.FindSubmatch(data)
	hexMatch := false
	if m == nil {
		m = titleHexRE.FindSubmatch(data)
		hexMatch = m != nil
	}
	if m == nil {
		return ""
	}
	raw := m[1]
	if hexMatch && len(raw)%2 == 0 {
		if decoded, err := hex.DecodeString(string(raw)); err == nil {
			raw = decoded
		}
	}
	var s string
	switch {
	case len(raw) >= 2 && raw[0] == 0xfe && raw[1] == 0xff:
		s = decodeUTF16(raw[2:], true)
	case len(raw) >= 2 && raw[0] == 0xff && raw[1] == 0xfe:
		s = decodeUTF16(raw[2:], false)
	default:
		s = decodeLatin1(raw)
	}
	return cleanSubject(s)
}

func Readable(s string) bool {
	text := py.Strip(s)
	if text == "" || ctrlRE.MatchString(text) {
		return false
	}
	body := 0
	sane, letters := 0, 0
	for _, c := range text {
		if unicode.IsSpace(c) {
			continue
		}
		body++
		if unicode.IsLetter(c) {
			letters++
			sane++
		} else if unicode.IsDigit(c) || strings.ContainsRune(titlePunct, c) {
			sane++
		}
	}
	if body == 0 {
		return false
	}
	return float64(sane) >= float64(body)*0.9 && float64(letters) >= float64(body)*0.4
}

func cleanSubject(s string) string {
	s = toolingRE.ReplaceAllString(s, "")
	s = extRE.ReplaceAllString(s, "")
	s = strings.ReplaceAll(s, "_", " ")
	s = dateRE.ReplaceAllString(s, " ")
	s = langTailRE.ReplaceAllString(s, " ")
	s = noiseWordRE.ReplaceAllString(s, " ")
	s = strings.Trim(py.CollapseSpace(s), " -–—·")
	if !lettersRE.MatchString(s) {
		return ""
	}
	low := strings.ToLower(s)
	if low == "news release" || low == "press release" || low == "document" {
		return ""
	}
	if Readable(s) {
		return s
	}
	return ""
}

func stripSecHeader(text string) string {
	text = replaceFirst(secHeaderRE, text)
	text = replaceFirst(secExRE, text)
	return py.Strip(text)
}

func replaceFirst(re *regexp.Regexp, text string) string {
	if loc := re.FindStringIndex(text); loc != nil {
		return text[:loc[0]] + text[loc[1]:]
	}
	return text
}

func HTMLText(data []byte) string {
	s := strings.ToValidUTF8(string(data), "�")
	s = scriptRE.ReplaceAllString(s, " ")
	s = tagsRE.ReplaceAllString(s, " ")
	return stripSecHeader(py.Strip(py.CollapseSpace(html.UnescapeString(s))))
}

func PDFTextClean(data []byte) string {
	return py.Strip(py.CollapseSpace(PDFText(data)))
}

func DocumentText(data []byte, contentType string) string {
	ct := strings.ToLower(contentType)
	if strings.Contains(ct, "pdf") || (len(data) >= 5 && string(data[:5]) == "%PDF-") {
		return PDFTextClean(data)
	}
	return HTMLText(data)
}

const prompt = "Below is the text of a company regulatory filing. In ONE short sentence, at most 20 words, say what it contains or announces — name the actual documents, events, or figures, not the company. If it is a cover form listing exhibits, name those exhibits. Do not restate the form type or begin with 'This filing'.\n\nFILING TEXT:\n%s\n\nSUMMARY (one sentence):"

const titlePrompt = "Give a short, specific title for this company filing: a noun phrase of at most 8 words naming what it is — the documents, event, or figures it contains. Not a form code, not the company name alone, no quotes, no preamble.\n\nFILING TEXT:\n%s\n\nTitle:"

const SummaryWaitSec = 25

type Enricher struct {
	Model *LocalModel
}

func (e *Enricher) SummaryAvailable() bool { return e.Model.Available() }

func (e *Enricher) WaitForSummary(seconds float64) bool { return e.Model.WaitReady(seconds) }

func (e *Enricher) SummaryStatus() string {
	if PDFPending() {
		return "downloading"
	}
	return e.Model.Status()
}

func FirstSentence(out string) string {
	out = py.Strip(out)
	for _, loc := range stopRE.FindAllStringIndex(out, -1) {
		end := loc[1]
		if end < len(out) && !unicode.IsSpace(rune(out[end])) {
			continue
		}
		before := out[:loc[0]]
		word := lastWord(before)
		if out[loc[0]:loc[1]] == "." && (abbrev[word] || initialRE.MatchString(word)) {
			continue
		}
		rest := strings.TrimLeft(out[loc[1]:], " \t\n\r\x0b\x0c")
		if rest != "" {
			r := []rune(rest)[0]
			if !(unicode.IsUpper(r) || unicode.IsDigit(r) || r == '"' || r == '“' || r == '(') {
				continue
			}
		}
		return py.Strip(out[:loc[1]])
	}
	return out
}

func lastWord(s string) string {
	idx := strings.LastIndexAny(s, " \t\n\r\x0b\x0c([\"'")
	w := s
	if idx >= 0 {
		w = s[idx+1:]
	}
	return strings.Trim(strings.ToLower(w), "\"'([")
}

func Hedged(out string) bool { return hedgeRE.MatchString(out) }

func stripPreamble(out string) string {
	out = specialRE.ReplaceAllString(out, " ")
	out = py.Strip(py.CollapseSpace(out))
	out = leadJunkRE.ReplaceAllString(out, "")
	for i := 0; i < 2; i++ {
		out = preambleRE.ReplaceAllString(out, "")
		out = labelRE.ReplaceAllString(out, "")
		out = leadJunkRE.ReplaceAllString(out, "")
	}
	return py.Strip(strings.Trim(strings.Trim(py.Strip(out), "\""), "*"))
}

func cutText(text string) string {
	r := []rune(text)
	if len(r) > MaxText {
		return string(r[:MaxText])
	}
	return text
}

func (e *Enricher) Summarize(text string) string {
	text = py.Strip(text)
	if text == "" {
		return ""
	}
	out := FirstSentence(stripPreamble(e.Model.Chat(strings.Replace(prompt, "%s", cutText(text), 1), 90)))
	if len(py.Fields(out)) < 4 || !lowerWordRE.MatchString(out) {
		return ""
	}
	if Hedged(out) {
		return ""
	}
	return cutRunes(out, 240)
}

func cutRunes(s string, n int) string {
	r := []rune(s)
	if len(r) > n {
		return string(r[:n])
	}
	return s
}

func isJunkTitle(s string) bool {
	low := strings.ToLower(s)
	if low == "" {
		return true
	}
	if !Readable(s) {
		return true
	}
	return strings.Contains(low, ".htm") || strings.Contains(low, ".xml") || strings.Contains(low, ".pdf") || strings.Contains(low, "exhibit") || exNumRE.MatchString(low) || fiveDigRE.MatchString(low)
}

func (e *Enricher) TitleFromModel(text string) string {
	text = py.Strip(text)
	if text == "" {
		return ""
	}
	out := stripPreamble(e.Model.Chat(strings.Replace(titlePrompt, "%s", cutText(text), 1), 40))
	out = py.Strip(strings.TrimRight(out, ".:"))
	words := py.Fields(out)
	if len(words) > 9 {
		words = words[:9]
	}
	out = strings.Join(words, " ")
	low := strings.ToLower(out)
	if len(words) < 3 || strings.Contains(low, "title") || strings.HasPrefix(low, "here") || isJunkTitle(out) || Hedged(out) {
		return ""
	}
	return cutRunes(out, 90)
}

type Result struct {
	Subject string
	Summary string
	Final   bool
}

func (e *Enricher) EnrichDocument(source string, data []byte, contentType string) Result {
	isPDF := len(data) >= 5 && string(data[:5]) == "%PDF-"
	subject := ""
	if isPDF {
		subject = ExtractPDFSubject(data)
	}
	if isJunkTitle(subject) {
		subject = ""
	}
	text := DocumentText(data, contentType)
	if exact := ReadForm(text); exact != nil {
		s := exact.Subject
		if s == "" {
			s = subject
		}
		return Result{Subject: s, Summary: exact.Summary, Final: true}
	}
	if IsForm(text) {
		return Result{Subject: subject, Final: true}
	}
	summary := e.Summarize(text)
	if subject == "" {
		subject = e.TitleFromModel(text)
	}
	return Result{Subject: subject, Summary: summary}
}
