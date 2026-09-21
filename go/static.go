package bagholder

import "embed"

//go:generate sh -c "cp ../ledger.html ../lightweight-charts.js ../favicon.png ."

// The page, the chart library and the icon. One tracked copy, at the repository root,
// shared by every build; `go generate ./...` brings it in here so the binary carries it
// and needs nothing beside it. The server still prefers a copy on disk, so editing the
// root file is all development needs.
//
//go:embed ledger.html lightweight-charts.js favicon.png
var Static embed.FS
