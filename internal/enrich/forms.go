package enrich

import (
	"fmt"
	"math"
	"regexp"
	"strconv"
	"strings"

	"github.com/ProfessorBagholder/Bagholder/internal/py"
)

var FormMarks = []*regexp.Regexp{
	regexp.MustCompile(`(?i)\(YYYY\s*-\s*MM\s*-\s*DD\)`),
	regexp.MustCompile(`(?i)\brefer to (?:part|section|item)\b`),
	regexp.MustCompile(`(?i)\bselect (?:only )?one\b`),
	regexp.MustCompile(`(?i)\bcomplete (?:schedule|item|part)\b`),
	regexp.MustCompile(`(?i)\bif applicable\b`),
	regexp.MustCompile(`(?i)\bcheck (?:the )?box\b`),
	regexp.MustCompile(`(?i)\bdo not complete\b`),
	regexp.MustCompile(`\bof the [Ii]nstructions\b`),
}

const FormMarkMin = 3

const money = `\$?\s*([\d,]+(?:\.\d+)?)`

func IsForm(text string) bool {
	n := 0
	for _, m := range FormMarks {
		if m.MatchString(text) {
			n++
		}
	}
	return n >= FormMarkMin
}

func numOf(s string) (float64, bool) {
	f, err := strconv.ParseFloat(strings.ReplaceAll(s, ",", ""), 64)
	return f, err == nil
}

func moneyText(n float64) string {
	if n == math.Trunc(n) || n >= 1000 {
		return "$" + py.Commas(n)
	}
	return "$" + py.CommasFixed(n, 2)
}

func formDate(text, label string) string {
	m := py.RE(`(?i)` + label + `\s*(\d{4})\s*YYYY\s*(\d{1,2})\s*(\d{1,2})\s*MM`).FindStringSubmatch(text)
	if m == nil {
		return ""
	}
	mo, _ := strconv.Atoi(m[2])
	d, _ := strconv.Atoi(m[3])
	return fmt.Sprintf("%s-%02d-%02d", m[1], mo, d)
}

var monthNames = []string{"January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"}

func dayText(iso string) string {
	parts := strings.Split(iso, "-")
	if len(parts) != 3 {
		return ""
	}
	y, e1 := strconv.Atoi(parts[0])
	m, e2 := strconv.Atoi(parts[1])
	d, e3 := strconv.Atoi(parts[2])
	if e1 != nil || e2 != nil || e3 != nil || m < 1 || m > 12 {
		return ""
	}
	return fmt.Sprintf("%d %s %d", d, monthNames[m-1], y)
}

type Exact struct {
	Subject string
	Summary string
}

var (
	amountRE    = regexp.MustCompile(`(?i)Total dollar amount of securities distributed\s*` + money)
	buyersRE    = regexp.MustCompile(`(?i)Total number of unique\s*(?:purchasers)?\s*(\d[\d,]*)`)
	exemptionRE = regexp.MustCompile(`NI\s*45-106\s*([\d.]+)\s*\[([^\]]{3,60})\]`)
)

func Read45106F1(text string) *Exact {
	var amount, buyers *float64
	if m := amountRE.FindStringSubmatch(text); m != nil {
		if v, ok := numOf(m[1]); ok {
			amount = &v
		}
	}
	if m := buyersRE.FindStringSubmatch(text); m != nil {
		if v, ok := numOf(m[1]); ok {
			buyers = &v
		}
	}
	when := formDate(text, `Start date`)
	if when == "" {
		when = formDate(text, `End date`)
	}
	exemption := ""
	if m := exemptionRE.FindStringSubmatch(text); m != nil {
		exemption = "NI 45-106 " + m[1] + " (" + strings.ToLower(strings.TrimSpace(m[2])) + ")"
	}
	if amount == nil && buyers == nil {
		return nil
	}
	var parts []string
	if amount != nil {
		parts = append(parts, moneyText(*amount)+" distributed")
	}
	if buyers != nil {
		s := "s"
		if *buyers == 1 {
			s = ""
		}
		parts = append(parts, fmt.Sprintf("%d purchaser%s", int(*buyers), s))
	}
	head := parts[0]
	if len(parts) == 2 {
		head = strings.Join(parts, " from ")
	}
	if when != "" {
		head += " on " + dayText(when)
	}
	if exemption != "" {
		head += ", under " + exemption
	}
	subject := ""
	if amount != nil {
		subject = "Exempt distribution of " + moneyText(*amount)
	}
	return &Exact{Subject: subject, Summary: head + "."}
}

var readers = []struct {
	mark *regexp.Regexp
	read func(string) *Exact
}{{regexp.MustCompile(`(?i)Form\s*45-106F1|Report of Exempt Distribution`), Read45106F1}}

func ReadForm(text string) *Exact {
	head := text
	if len(head) > 4000 {
		head = head[:4000]
	}
	for _, r := range readers {
		if r.mark.MatchString(head) {
			out := r.read(text)
			if out != nil && out.Summary != "" {
				return out
			}
		}
	}
	return nil
}
