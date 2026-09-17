package news

import "strings"

var Market = [3]string{"*", "MARKET", ""}

var wireMarks = []string{"wire", "newsfile", "cision", "cnw"}

func KindOf(source string) string {
	s := strings.ToLower(source)
	for _, m := range wireMarks {
		if strings.Contains(s, m) {
			return "release"
		}
	}
	return "story"
}
