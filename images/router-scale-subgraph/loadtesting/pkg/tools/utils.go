package tools

import (
	"fmt"
	"math/rand"

	log "github.com/sirupsen/logrus"
	"github.com/vektah/gqlparser/v2/ast"
)

const letterBytes = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ"
const (
	letterIdxBits = 6                    // 6 bits to represent a letter index
	letterIdxMask = 1<<letterIdxBits - 1 // All 1-bits, as many as letterIdxBits
)

func GetRandString(n int) string {
	b := make([]byte, n)
	for i := 0; i < n; {
		if idx := int(rand.Int63() & letterIdxMask); idx < len(letterBytes) {
			b[i] = letterBytes[idx]
			i++
		}
	}
	return string(b)
}

var Internal = [...]string{"Float", "Boolean", "ID", "Int", "String"}

func CheckScalar(name string) bool {

	for _, scalar := range Internal {
		if name == scalar {
			return true
		}
	}
	return false
}

func ScalarToFakeData(typeName string) interface{} {
	switch typeName {
	case "String":
		return GetRandString(20)
	case "Int":
		return rand.Intn(100)
	case "Float":
		return rand.Float64()
	case "Boolean":
		random := rand.Intn(100)
		return (random >= 50)
	case "ID":
		return GetRandString(5)
	default:
		log.Errorf("Unknown type in ScalarToFakeData: %s", typeName)
	}
	return nil
}

// GetItem - get a random item from a map
func GetItem[T any](data map[string]T) T {
	for _, v := range data {
		return v
	}
	var empty T
	return empty
}

// FindRealName - find the name for a type in the AST.
func FindRealName(t *ast.Type) string {
	if t.NamedType != "" {
		return t.NamedType
	} else {
		if t.Elem != nil {
			return FindRealName(t.Elem)
		}
		panic(fmt.Sprintf("Cannot understand type: %+v", t))
	}
}

// Remove - remove an item at index from a slice of type T
func Remove[T any](slice []T, index int) []T {
	return append(slice[:index], slice[index+1:]...)[:len(slice)-1]
}

func FindFieldType(fieldName, parent string, schema *ast.Schema) (realType string, isArray, isNullable bool, err error) {
	isNullable = true
	isArray = false
	switch parent {
	case string(ast.Query):
		fallthrough
	case "Query":
		//log.Infof("FFT: find query token %s", fieldToken)
		for _, field := range schema.Query.Fields {
			if field.Name == fieldName {
				realType = FindRealName(field.Type)
				if field.Type.String()[0] == '[' {
					isArray = true
				}
				if field.Type.String()[len(field.Type.String())-1] == '!' {
					isNullable = false
				}
				return realType, isArray, isNullable, err
			}
		}
	case string(ast.Mutation):
		fallthrough
	case "Mutation":
		for _, field := range schema.Mutation.Fields {
			if field.Name == fieldName {
				realType = FindRealName(field.Type)
				if field.Type.String()[0] == '[' {
					isArray = true
				}
				if field.Type.String()[len(field.Type.String())-1] == '!' {
					isNullable = false
				}
				return realType, isArray, isNullable, err
			}
		}
	case string(ast.Subscription):
		fallthrough
	case "Subscription":
		for _, field := range schema.Subscription.Fields {
			if field.Name == fieldName {
				realType = FindRealName(field.Type)
				if field.Type.String()[0] == '[' {
					isArray = true
				}
				if field.Type.String()[len(field.Type.String())-1] == '!' {
					isNullable = false
				}
				return realType, isArray, isNullable, err
			}
		}
	default:
		//log.Infof("Case is Type: %s", fieldToken)

		if t, ok := schema.Types[parent]; ok {
			if t.IsAbstractType() {
				log.Warnf("FFT: %s ia abstract.", parent)
				//panic("Type is abstract, do not use FindFieldType with abstract types.")
			}

			for _, field := range t.Fields {
				if field.Name == fieldName {
					realType = FindRealName(field.Type)

					if field.Type.String()[0] == '[' {
						isArray = true
					}
					if field.Type.String()[len(field.Type.String())-1] == '!' {
						isNullable = false
					}
					log.Tracef("FFT: %s Nullable:%t Array:%t", field.Type.String(), isNullable, isArray)
					return realType, isArray, isNullable, err
				}
			}
		} else {
			log.Errorf("FFT: Cannot find type: %s", parent)
			panic("Cannot find type.")
		}
	}
	log.Errorf("FFT: Cannot find type for: %s parent: %s", fieldName, parent)
	panic("Should never be here.")
	//return "", false, false, fmt.Errorf("cannot find type for: %s parent: %s", fieldName, parent)
}
