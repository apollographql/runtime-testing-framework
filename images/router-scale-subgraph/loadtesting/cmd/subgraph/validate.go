package main

import (
	"fmt"
	"strings"

	"github.com/apollosolutions/loadtesting/pkg/tools"
	log "github.com/sirupsen/logrus"
	"github.com/vektah/gqlparser/v2/ast"
)

func sym(offset, i, length int) string {
	if i == (length - 1) {
		return strings.Repeat(" ", offset*2) + "└"
	}
	if i < (length - 1) {
		return strings.Repeat(" ", offset*2) + "├"
	}
	return strings.Repeat(" ", offset*2)
}

func ValidateResponse(schema *ast.Schema, parentTypeName string, sel ast.SelectionSet, data map[string]interface{},
	fragments map[string]*ast.FragmentDefinition, depth int) string {

	builder := strings.Builder{}
	var parentType *ast.Definition

	switch parentTypeName {
	case "query":
		parentType = schema.Query
	case "mutation":
		parentType = schema.Mutation
	case "subscription":
		parentType = schema.Subscription
	case "_Entity":
		if val, ok := data["__typename"].(string); ok {
			parentTypeName = val
		} else {
			log.Errorf("No __typename but parent is entity.")
		}
	default:
		parentType = schema.Types[parentTypeName]
		if parentType.IsAbstractType() {
			log.Warnf("Validation warning, parent is %s and abstract.", parentTypeName)
			//return ""
		}
	}

	for i, selection := range sel {
		switch field := selection.(type) {
		case *ast.Field:

			if val, ok := data[field.Alias]; !ok {
				builder.WriteString(fmt.Sprintf("%s%s not found in response\n", sym(depth, i, len(sel)), field.Alias))
			} else {

				if field.Name == "__typename" {
					builder.WriteString(fmt.Sprintf("%s%s=%v\n", sym(depth, i, len(sel)), field.Alias, val))
					continue
				}

				realType := ""
				isArray := false
				isNullable := false
				var err error

				if field.Name != "_entities" {
					realType, isArray, isNullable, err = tools.FindFieldType(field.Name, parentTypeName, schema)
					if err != nil {
						builder.WriteString(fmt.Sprintf("%s%s cannot find type: %s\n", sym(depth, i, len(sel)), field.Alias, err.Error()))
						continue
					}
				} else {
					realType = "_Entity"
					isArray = true
					isNullable = false
				}

				if field.SelectionSet != nil {

					builder.WriteString(fmt.Sprintf("%s%s\n", sym(depth, i, len(sel)), field.Alias))
					// Object
					switch subField := data[field.Alias].(type) {
					case map[string]interface{}:
						builder.WriteString(ValidateResponse(schema, realType, field.SelectionSet, subField, fragments, depth+1))
					case []interface{}:
					case []map[string]interface{}:
						if !isArray {
							builder.WriteString(fmt.Sprintf("%s%s should be array\n", sym(depth, i, len(sel)), field.Alias))
						}
						if len(subField) == 0 {
							builder.WriteString(fmt.Sprintf("%s%s array is empty\n", sym(depth, i, len(sel)), field.Alias))
						} else {
							builder.WriteString(ValidateResponse(schema, realType, field.SelectionSet, subField[0], fragments, depth+1))
						}
					case nil:
						if !isNullable {
							builder.WriteString(fmt.Sprintf("%s%s is NULL but !Nullable\n", sym(depth, i, len(sel)), field.Alias))
						}
					}
				} else {
					// Is Scalar
					builder.WriteString(fmt.Sprintf("%s%s=%v\n", sym(depth, i, len(sel)), field.Alias, val))
				}
			}

		case *ast.InlineFragment:
			builder.WriteString(fmt.Sprintf("%sIFRAG on %s\n", sym(depth, i, len(sel)), field.TypeCondition))
			builder.WriteString(ValidateResponse(schema, field.TypeCondition, field.SelectionSet, data, fragments, depth))
		case *ast.FragmentSpread:
			frag := fragments[field.Name]
			if frag != nil {
				builder.WriteString(fmt.Sprintf("%sSPREAD %s on %s\n", sym(depth, i, len(sel)), frag.Name, frag.TypeCondition))
				builder.WriteString(ValidateResponse(schema, frag.TypeCondition, frag.SelectionSet, data, fragments, depth))
			} else {
				builder.WriteString(fmt.Sprintf("Fragment spread cannot be found: %s\n", field.Name))
			}
		default:
			builder.WriteString(fmt.Sprintf("AST type of %T not supported\n", field))
		}
	}
	return builder.String()
}
