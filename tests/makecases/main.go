package main

import (
	"fmt"
	"os"
	"path/filepath"
	"sort"

	"github.com/ProfessorBagholder/Bagholder/internal/cases"
)

func main() {
	dir := filepath.Join("tests", "cases")
	if len(os.Args) > 1 {
		dir = os.Args[1]
	}
	paths, _ := filepath.Glob(filepath.Join(dir, "*.json"))
	sort.Strings(paths)
	if len(paths) == 0 {
		fmt.Fprintln(os.Stderr, "no cases under "+dir)
		os.Exit(1)
	}
	for _, path := range paths {
		previous, _ := os.ReadFile(path)
		doc, err := cases.Load(path)
		if err != nil {
			fmt.Fprintln(os.Stderr, path+": "+err.Error())
			os.Exit(1)
		}
		raw, err := cases.Raw(previous)
		if err != nil {
			fmt.Fprintln(os.Stderr, path+": "+err.Error())
			os.Exit(1)
		}
		raw["expect"] = cases.Generic(cases.Expect(&doc.Snapshot, doc.MarketData(), doc.Today, doc.Filters, doc.Journal))
		for _, k := range []string{"filters", "journal"} {
			if raw[k] == nil {
				raw[k] = map[string]any{}
			}
		}
		if err := os.WriteFile(path, cases.NewWriter(previous).Marshal(raw), 0o644); err != nil {
			fmt.Fprintln(os.Stderr, path+": "+err.Error())
			os.Exit(1)
		}
		fmt.Println("wrote", path)
	}
}
