package tools

import (
	"fmt"
	"math/rand/v2"
	"strings"

	"github.com/vektah/gqlparser/v2/ast"

	log "github.com/sirupsen/logrus"
)

/*
type Enum struct {
	Value string
}

func (e *Enum) MarshalJSON() ([]byte, error) {

	return []byte(e.Value), nil
}*/

const MAX_NULLABLE_DEPTH = 3
const MAX_DEPTH = 10

type SelectionMeta struct {
	Children      map[string]*SelectionMeta
	TrueName      string
	Alias         string
	TypeCondition map[string]string // map of field aliases to type names
}

func (s *SelectionMeta) String() string {
	builder := strings.Builder{}
	builder.WriteString(s.TrueName)
	builder.WriteString(":")
	builder.WriteString(s.Alias)
	builder.WriteString(" { ")
	if s.Children != nil {
		for k := range s.Children {
			builder.WriteString(k)
			builder.WriteRune(' ')
		}
	}
	builder.WriteString("}")
	return builder.String()
}

func Op(opName *string) string {
	if opName != nil {
		return fmt.Sprintf("%s - ", *opName)
	}
	return ""
}

func fieldToFakeData(schema *ast.Schema, field *ast.FieldDefinition, childSelections *SelectionMeta,
	typeName string, opName *string, depth int) interface{} {

	childTypeName, isArray, isNullable, err := FindFieldType(field.Name, typeName, schema)
	if err != nil || childTypeName == "" {
		log.Errorf("TTFD: cant find type for %s %s", field.Name, typeName)
		panic("Cannot find type.")
	}

	log.Debugf("TTFD: Child %s (%s) isNullable:%t isArray:%t", field.Name, childTypeName, isNullable, isArray)

	// Detect type cyclic loops
	if childTypeName == typeName {
		log.Debugf("Tight loop detected, breaking if possible: %s", typeName)
		// This is a tight cycle, end it if we can
		if isArray {
			return []interface{}{}

		}
		if isNullable {
			return nil
		}
	}

	if depth > MAX_NULLABLE_DEPTH {
		log.Debugf("TTFD: Max nullable depth: p %s c %s (%s) %d nullable: %t array: %t.", typeName, field.Name, childTypeName, depth, isNullable, isArray)
		if isArray {
			return []interface{}{}

		}
		if isNullable {
			return nil
		}
	}
	if depth > MAX_DEPTH {
		log.Errorf("TTFD: Max depth: p %s c %s (%s) %d nullable: %t array: %t.", typeName, field.Name, childTypeName, depth, isNullable, isArray)
		panic("MAX DEPTH EXCEEDED")
	}

	loop := rand.IntN(10) + 3
	if !isArray {
		loop = 1
	}
	data := make([]interface{}, loop)
	for i := 0; i < loop; i++ {

		if CheckScalar(childTypeName) {
			data[i] = ScalarToFakeData(childTypeName)
		} else {
			t := schema.Types[childTypeName]
			if t == nil {
				// VERY BAD NEWS
				// TODO: Remove me after full validation
				log.Errorf("TTFD: Null type: |%s|", childTypeName)
				log.Errorf("TYPES")
				for k, v := range schema.Types {
					log.Errorf("%s %s", v.Kind, k)
				}
				panic("Bail")
			}

			log.Debugf("TTFD: Recurse %s", childTypeName)
			data[i] = TypeToFakeData(schema, t, childTypeName, depth+1, childSelections, opName)
		}
	}
	if isArray {
		return data
	} else {
		return data[0]
	}

}

// TypeToFakeData - given a type definition create a response object with the same signature
func TypeToFakeData(schema *ast.Schema, t *ast.Definition, typeName string, depth int, selections *SelectionMeta, opName *string) interface{} {

	isInput := false
	isInterface := false

	switch t.Kind {
	case ast.Enum:
		for _, val := range t.EnumValues {
			log.Debugf("TTFD: Enum: %s (for type %s)", val.Name, typeName)
		}
		val := t.EnumValues[rand.IntN(len(t.EnumValues))].Name
		return &val

	case ast.Scalar:
		log.Debugf("TTFD: Custom Scalar: %s %s", t.Name, typeName)
		return "CUSTOM SCALAR"
	case ast.InputObject:
		log.Debugf("TTFD: Input: ")
		isInput = true
	case ast.Object:
		// handled below
	case ast.Union:
		log.Debugf("TTFD: Union: choosing: %s", t.Types[0])
		//isUnion = true
		typeName = t.Types[0]
		t = schema.Types[typeName]
	case ast.Interface:
		isInterface = true
		log.Debugf("TTFD: Interface")
	default:
		log.Errorf("TTFD: TYPE NOT HANDLED: %s %s", typeName, t.Kind)
		panic("Type not handled in TTFD")
	}

	result := make(map[string]interface{})

	// for interfaces try to get either the type condition of the selection or
	// fallback to any implementing type
	if isInterface {
		if selections != nil && len(selections.TypeCondition) > 0 {
			newType := GetItem(selections.TypeCondition)
			log.Debugf("TTFD: Renaming interface to type condition: %s->%s", typeName, newType)
			typeName = newType
			t = schema.Types[newType]
		} else {
			types := schema.GetPossibleTypes(t)
			if len(types) > 0 {
				t = types[0]
				log.Warnf("TTFD: Interface detected, changing type from %s to %s", typeName, t.Name)
				typeName = t.Name
			} else {
				log.Errorf("TTFD: No possible types found for interface: typeName %s kind %s", typeName, string(t.Kind))
				panic(fmt.Sprintf("No possible types found for interfac: typeName %s kind %s", typeName, string(t.Kind)))
			}
		}
	}

	if !isInput {
		result["__typename"] = typeName
	}

	if selections != nil {
		log.Debugf("TTFD: %s %s %s", t.Name, typeName, selections.String())

		for k, v := range selections.Children {
			log.Debugf("%s%s%s:%s SEL", strings.Repeat(" ", depth*2), Op(opName), v.TrueName, k)

			if v.TrueName == "__typename" {
				result[v.TrueName] = typeName
				continue
			}
			called := false
			for _, f := range t.Fields {
				log.Errorf("TTFD: Search in %s field name is: %s", typeName, f.Name)
				if f.Name == v.TrueName {
					result[k] = fieldToFakeData(schema, f, v, typeName, opName, depth)
					called = true
					break
				}
			}
			if !called {
				log.Errorf("Field (%s) not found in type (%s) for selection '%s'", v.TrueName, typeName, v.Alias)
			}
		}
	} else {
		log.Debugf("TTFD: %s %s", t.Name, typeName)
		for _, field := range t.Fields {
			log.Debugf("%s%s%s SEL", strings.Repeat(" ", depth*2), Op(opName), field.Name)
			result[field.Name] = fieldToFakeData(schema, field, nil, typeName, opName, depth)
		}
	}

	/*for _, field := range t.Fields {

		alias := field.Name
		var childSelections *SelectionMeta

		if selections != nil {
			selection, ok := selections.Children[field.Name]

			if !ok {
				log.Debugf("%s%s%s  NOT SELECTED", strings.Repeat(" ", depth*2), Op(opName), field.Name)
				continue
			}
			alias = selection.Alias
			childSelections = selection

			log.Infof("%s%s%s:%s SEL", strings.Repeat(" ", depth*2), Op(opName), field.Name, selection.Alias)

		} else {
			log.Infof("%s%s%s SEL", strings.Repeat(" ", depth*2), Op(opName), field.Name)

		}

		childTypeName, isArray, isNullable, err := FindFieldType(field.Name, typeName, schema)
		if err != nil || childTypeName == "" {
			log.Errorf("ttfd: cant find type for %s %s", field.Name, typeName)
			panic("Cannot find type.")
		}

		log.Debugf("TTFD: Child %s (%s) isNullable:%t isArray:%t", field.Name, childTypeName, isNullable, isArray)

		// Detect type cyclic loops
		if childTypeName == typeName {
			log.Debugf("Tight loop detected, breaking if possible: %s", typeName)
			// This is a tight cycle, end it if we can
			if isArray {
				result[alias] = []interface{}{}
				continue
			}
			if isNullable {
				result[alias] = nil
				continue
			}
		}

		if depth > MAX_NULLABLE_DEPTH {
			log.Debugf("TTFD: Max nullable depth: p %s c %s (%s) %d nullable: %t array: %t.", typeName, field.Name, childTypeName, depth, isNullable, isArray)
			if isArray {
				result[alias] = []interface{}{}
				continue
			}
			if isNullable {
				result[alias] = nil
				continue
			}
		}
		if depth > MAX_DEPTH {
			log.Errorf("TTFD: Max depth: p %s c %s (%s) %d nullable: %t array: %t.", typeName, field.Name, childTypeName, depth, isNullable, isArray)
			panic("MAX DEPTH EXCEEDED")
		}

		loop := rand.IntN(10) + 3
		if !isArray {
			loop = 1
		}
		data := make([]interface{}, loop)
		for i := 0; i < loop; i++ {

			if CheckScalar(childTypeName) {
				data[i] = ScalarToFakeData(childTypeName)
			} else {
				t := schema.Types[childTypeName]
				if t == nil {
					// VERY BAD NEWS
					// TODO: Remove me after full validation
					log.Errorf("TTFD: Null type: |%s|", childTypeName)
					log.Errorf("TYPES")
					for k, v := range schema.Types {
						log.Errorf("%s %s", v.Kind, k)
					}
					continue
				}

				log.Debugf("TTFD: Recurse %s", childTypeName)
				data[i] = TypeToFakeData(schema, t, childTypeName, depth+1, childSelections, opName)
			}
		}
		if isArray {
			result[alias] = data
		} else {
			result[alias] = data[0]
		}

	}*/
	return result
}
