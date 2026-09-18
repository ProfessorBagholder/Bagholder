package news

import "regexp"

var Market = [3]string{"*", "MARKET", ""}

var wireNames = regexp.MustCompile(`(?i)newswire(?:[^s]|$)|business ?wire|accesswire|newmediawire|marketwired|newsfile|cision|\bcnw\b|prweb`)

func KindOf(source string) string {
	if wireNames.MatchString(source) {
		return "release"
	}
	return "story"
}
