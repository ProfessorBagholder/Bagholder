package py

import (
	"encoding/json"
	"reflect"
	"testing"
)

func TestJSONTextReadsEveryValueAsSWould(t *testing.T) {
	var got struct {
		A, B, C, D, E, F JSONText
	}
	if err := json.Unmarshal([]byte(`{"A": "x", "B": 219.41, "C": 5350000000000, "D": true, "E": null, "F": 1e22}`), &got); err != nil {
		t.Fatal(err)
	}
	want := []string{"x", "219.41", "5350000000000.0", "True", "", "1e+22"}
	if have := []string{string(got.A), string(got.B), string(got.C), string(got.D), string(got.E), string(got.F)}; !reflect.DeepEqual(have, want) {
		t.Errorf("texts = %q, want %q", have, want)
	}
}

func TestJSONNumReadsEveryValueAsNumOKWould(t *testing.T) {
	var got struct {
		A, B, C, D, E, F, G JSONNum
	}
	if err := json.Unmarshal([]byte(`{"A": 28.5, "B": "51", "C": " 7 ", "D": true, "E": null, "F": "N/A", "G": {"raw": 1}}`), &got); err != nil {
		t.Fatal(err)
	}
	want := []JSONNum{{28.5, true}, {51, true}, {7, true}, {1, true}, {0, false}, {0, false}, {0, false}}
	if have := []JSONNum{got.A, got.B, got.C, got.D, got.E, got.F, got.G}; !reflect.DeepEqual(have, want) {
		t.Errorf("nums = %v, want %v", have, want)
	}
	if got.E.Ptr() != nil || got.A.Ptr() == nil || *got.A.Ptr() != 28.5 {
		t.Error("Ptr")
	}
}

func TestJSONLooseKeepsTheRowsThatFitAndBlanksTheRest(t *testing.T) {
	type row struct {
		Symbol JSONText `json:"symbol"`
	}
	var got []JSONLoose[row]
	if err := json.Unmarshal([]byte(`[{"symbol": "RY"}, "junk", 3, null, {"symbol": "TD"}]`), &got); err != nil {
		t.Fatal(err)
	}
	want := []string{"RY", "", "", "", "TD"}
	have := []string{}
	for _, r := range got {
		have = append(have, string(r.V.Symbol))
	}
	if !reflect.DeepEqual(have, want) {
		t.Errorf("rows = %q, want %q", have, want)
	}
}
