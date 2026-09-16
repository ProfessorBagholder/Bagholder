// Package news reads the wires for the symbols the book holds and watches: TMX
// Money's news for Canadian listings and Nasdaq's for US ones.
package news

import "strings"

// Market is the market-wide feed as a listing of its own: Nasdaq's latest news.
var Market = [3]string{"*", "MARKET", ""}

var wireMarks = []string{"wire", "newsfile", "cision", "cnw"}

// KindOf is what an item is, told by where it came from: a wire carries the company's own release, a publisher writes a story.
func KindOf(source string) string {
	s := strings.ToLower(source)
	for _, m := range wireMarks {
		if strings.Contains(s, m) {
			return "release"
		}
	}
	return "story"
}
