package main

import "testing"

func TestMerge(t *testing.T) {

	d1 := map[string]interface{}{
		"a": map[string]interface{}{
			"b": "blah",
			"c": 234,
		},
	}

	d2 := map[string]interface{}{
		"a": map[string]interface{}{
			"x": "blah",
			"y": 234,
		},
	}

	d3 := mergeInterface(d1, d2)

	d3Cast := d3.(map[string]interface{})
	innerMap := d3Cast["a"].(map[string]interface{})
	if _, ok := innerMap["x"]; !ok {
		t.Fatalf("x not in merge %+v", innerMap)
	}
	if _, ok := innerMap["b"]; !ok {
		t.Fatalf("b not in merge %+v", innerMap)
	}
}
