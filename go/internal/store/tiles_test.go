package store

import (
	"reflect"
	"testing"
)

func TestTheRowIsTheDefaultUntilSavedAndThenWhatWasSaved(t *testing.T) {
	s := temp(t)
	if tiles, saved := s.Tiles(); saved || tiles != nil {
		t.Fatalf("never saved: %+v %v", tiles, saved)
	}
	before := s.DataVersion()
	got := s.SaveTiles([]Tile{{Symbol: "tnx", Exchange: "index"}, {Symbol: "usdcad", Exchange: "fx"}, {Symbol: "", Exchange: "x"}})
	want := []Tile{{Symbol: "TNX", Exchange: "INDEX"}, {Symbol: "USDCAD", Exchange: "FX"}}
	if !reflect.DeepEqual(got, want) {
		t.Fatalf("%+v", got)
	}
	if s.DataVersion() == before {
		t.Error("the row is part of the data version")
	}
	if tiles, saved := s.Tiles(); !saved || !reflect.DeepEqual(tiles, want) {
		t.Errorf("%+v %v", tiles, saved)
	}
	s.SaveTiles([]Tile{})
	if tiles, saved := s.Tiles(); !saved || len(tiles) != 0 {
		t.Errorf("an emptied row stays empty: %+v %v", tiles, saved)
	}
}
