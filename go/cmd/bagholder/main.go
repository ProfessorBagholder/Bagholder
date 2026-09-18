package main

import (
	"fmt"
	"os"
	_ "time/tzdata"

	"github.com/ProfessorBagholder/Bagholder/internal/app"
	"github.com/ProfessorBagholder/Bagholder/internal/disclosures"
	"github.com/ProfessorBagholder/Bagholder/internal/mcp"
	"github.com/ProfessorBagholder/Bagholder/internal/sedarcli"
)

func main() {
	args := os.Args[1:]
	if len(args) > 0 {
		switch args[0] {
		case "--version", "-v", "version":
			fmt.Println("Bagholder " + app.AppVersion)
			return
		case "mcp":
			cfg := app.ConfigFromEnv()
			a, err := app.New(cfg)
			if err != nil {
				fmt.Fprintln(os.Stderr, err.Error())
				os.Exit(1)
			}
			p := a.Pipeline()
			sedar, _ := p.Sedar.(*disclosures.Sedar)
			(&mcp.Server{Pipeline: p, Sedar: sedar}).Serve(os.Stdin, os.Stdout)
			return
		case "sedar":
			os.Exit(sedarcli.Main(args[1:], os.Stdout, os.Stderr))
		case "--help", "-h", "help":
			fmt.Fprint(os.Stderr, "bagholder                 run the app\n"+
				"bagholder mcp             the disclosures MCP server over stdio\n"+
				"bagholder sedar <cmd>     SEDAR+ filings over HTTP (resolve, filings, newest, get)\n"+
				"bagholder --version       print the version\n")
			os.Exit(2)
		}
	}
	cfg := app.ConfigFromEnv()
	if os.Getenv("BAGHOLDER_CHILD") == "1" || cfg.UpdatesOff {
		a, err := app.New(cfg)
		if err != nil {
			fmt.Fprintln(os.Stderr, err.Error())
			os.Exit(1)
		}
		os.Exit(a.Run())
	}
	os.Exit(app.Supervise(cfg.Home, app.UpdateHealthySec))
}
