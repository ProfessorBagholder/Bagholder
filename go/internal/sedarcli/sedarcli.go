package sedarcli

import (
	"encoding/json"
	"fmt"
	"io"
	"os"
	"strconv"

	"github.com/ProfessorBagholder/Bagholder/internal/disclosures"
)

const searchLimit = 100

func Main(args []string, out, errOut io.Writer) int {
	if len(args) == 0 || args[0] == "-h" || args[0] == "--help" || args[0] == "help" {
		fmt.Fprint(errOut, "bagholder sedar — SEDAR+ filings over HTTP\n"+
			"  bagholder sedar resolve <name|number>       profiles matching an issuer\n"+
			"  bagholder sedar filings <name|number> [n]   an issuer's filings (JSON)\n"+
			"  bagholder sedar newest [n]                  newest filings across SEDAR+\n"+
			"  bagholder sedar get <profileNo> <id> <dest.pdf>   download one document\n")
		return 2
	}
	s := disclosures.NewSedar()
	emit := func(v any) {
		raw, _ := json.MarshalIndent(v, "", "  ")
		fmt.Fprintln(out, string(raw))
	}
	failed := func(err error) int {
		raw, _ := json.Marshal(map[string]any{"ok": false, "error": err.Error()})
		fmt.Fprintln(out, string(raw))
		return 1
	}
	switch args[0] {
	case "resolve":
		if len(args) < 2 {
			fmt.Fprintln(errOut, "usage: bagholder sedar resolve <name|number>")
			return 2
		}
		profiles, err := s.ResolveProfile(args[1])
		if err != nil {
			return failed(err)
		}
		emit(map[string]any{"ok": true, "profiles": profiles})
	case "filings":
		if len(args) < 2 {
			fmt.Fprintln(errOut, "usage: bagholder sedar filings <name|number> [n]")
			return 2
		}
		limit := searchLimit
		if len(args) > 2 {
			if n, err := strconv.Atoi(args[2]); err == nil {
				limit = n
			}
		}
		res, err := s.ListFilings(args[1], "", limit)
		if err != nil {
			return failed(err)
		}
		emit(map[string]any{"ok": true, "profile": res.Profile, "scoped": res.Scoped, "filings": res.Filings})
	case "newest":
		limit := 30
		if len(args) > 1 {
			if n, err := strconv.Atoi(args[1]); err == nil {
				limit = n
			}
		}
		rows, err := s.Newest(limit)
		if err != nil {
			return failed(err)
		}
		emit(map[string]any{"ok": true, "filings": rows})
	case "get":
		if len(args) < 4 {
			fmt.Fprintln(errOut, "usage: bagholder sedar get <profileNo> <id> <dest.pdf>")
			return 2
		}
		data, ct, err := s.DownloadBytes(args[1], args[2], "")
		if err != nil {
			return failed(err)
		}
		if err := os.WriteFile(args[3], data, 0o644); err != nil {
			return failed(err)
		}
		raw, _ := json.Marshal(map[string]any{"ok": true, "path": args[3], "contentType": ct, "bytes": len(data)})
		fmt.Fprintln(out, string(raw))
	default:
		fmt.Fprintf(errOut, "unknown command %q\n", args[0])
		return 2
	}
	return 0
}
