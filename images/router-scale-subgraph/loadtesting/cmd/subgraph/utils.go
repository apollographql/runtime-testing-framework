package main

import (
	"encoding/json"
	"strings"
)

func to_string(data map[string]interface{}) string {
	resp, _ := json.MarshalIndent(data, "", "    ")
	return string(resp)
}

type StringArrayFlag []string

func (i *StringArrayFlag) String() string {
	return strings.Join(*i, ",")
}

func (i *StringArrayFlag) Set(value string) error {
	*i = append(*i, value)
	return nil
}
