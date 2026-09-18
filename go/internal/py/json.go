package py

import "encoding/json"

type JSONText string

func (t *JSONText) UnmarshalJSON(b []byte) error {
	if len(b) > 0 && b[0] == '"' {
		var s string
		if err := json.Unmarshal(b, &s); err != nil {
			return err
		}
		*t = JSONText(s)
		return nil
	}
	var v any
	if err := json.Unmarshal(b, &v); err != nil {
		return err
	}
	*t = JSONText(S(v))
	return nil
}

type JSONNum struct {
	F  float64
	OK bool
}

func (n *JSONNum) UnmarshalJSON(b []byte) error {
	var v any
	if err := json.Unmarshal(b, &v); err != nil {
		return err
	}
	n.F, n.OK = NumOK(v)
	return nil
}

func (n JSONNum) Ptr() *float64 {
	if !n.OK {
		return nil
	}
	f := n.F
	return &f
}

type JSONLoose[T any] struct {
	V T
}

func (l *JSONLoose[T]) UnmarshalJSON(b []byte) error {
	var v T
	if json.Unmarshal(b, &v) == nil {
		l.V = v
	}
	return nil
}
