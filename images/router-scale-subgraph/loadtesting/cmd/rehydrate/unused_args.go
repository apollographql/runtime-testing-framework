package main

import (
	"fmt"

	log "github.com/sirupsen/logrus"
	"github.com/vektah/gqlparser/v2/ast"
)

func getTypeStr(realType string, isArray, isNullable bool) string {
	if isArray {
		if isNullable {
			return fmt.Sprintf("[%s]!", realType)
		} else {
			return fmt.Sprintf("[%s]", realType)
		}
	} else {
		if isNullable {
			return fmt.Sprintf("%s!", realType)
		} else {
			return realType
		}
	}
}

// doFindUsedVariables - recursively walk AST to look for usage of variables in field inputs
func doFindUsedVariables(schema *ast.Schema, parent string, selections ast.SelectionSet, vars map[string]string, fragments map[string]*ast.FragmentDefinition) {

	var parentType *ast.Definition
	switch parent {
	case "query":
		if schema.Query == nil {
			panic("schema.Query is nil")
		}
		parentType = schema.Query
	case "mutation":
		parentType = schema.Mutation
	case "subscription":
		parentType = schema.Subscription
	default:
		parentType = schema.Types[parent]

	}

	if parentType == nil {
		log.Errorf("Cannot find type for %s", parent)
		panic("Parent type cannot be null.")
	}

	for _, selection := range selections {
		switch field := selection.(type) {
		case *ast.Field:

			if field.Name == "__typename" {
				continue
			}

			typeField := parentType.Fields.ForName(field.Name)
			if typeField == nil {
				log.Errorf("Parent is %s field is %s type is nil.", parent, field.Name)
				panic("Type field can't be null.")
			}

			for _, arg := range field.Arguments {
				//log.Infof("Arg %s=%s", arg.Name, arg.Value.Raw)
				if arg.Value.Kind == ast.Variable {
					vars[arg.Value.Raw] = typeField.Type.String()
					log.Debugf("Adding variable %s", arg.Value.Raw)
				}
			}

			for _, dir := range field.Directives {
				var schemaDir *ast.DirectiveDefinition
				for _, sdir := range schema.Directives {
					if sdir.Name == dir.Name {
						schemaDir = sdir
					}
				}
				if schemaDir == nil {
					panic("Can't find directive in schema.")
				}
				for _, arg := range dir.Arguments {
					if arg.Value.Kind == ast.Variable {
						vars[arg.Value.Raw] = schemaDir.Arguments.ForName(arg.Name).Type.String()
						log.Debugf("Adding directive variable %s", arg.Value.Raw)
					}
				}
			}

			if len(field.SelectionSet) > 0 {
				doFindUsedVariables(schema, typeField.Type.Name(), field.SelectionSet, vars, fragments)
			}
		case *ast.InlineFragment:
			doFindUsedVariables(schema, field.TypeCondition, field.SelectionSet, vars, fragments)
		case *ast.FragmentSpread:
			for _, dir := range field.Directives {
				var schemaDir *ast.DirectiveDefinition
				for _, sdir := range schema.Directives {
					if sdir.Name == dir.Name {
						schemaDir = sdir
					}
				}
				if schemaDir == nil {
					panic("Can't find directive in schema.")
				}

				for _, arg := range dir.Arguments {
					if arg.Value.Kind == ast.Variable {
						vars[arg.Value.Raw] = schemaDir.Arguments.ForName(arg.Name).Type.String()
						log.Debugf("Adding variable %s", arg.Value.Raw)
					}
				}
			}
			if frag, ok := fragments[field.Name]; ok {
				doFindUsedVariables(schema, frag.TypeCondition, frag.SelectionSet, vars, fragments)
			}

		default:
			log.Errorf("Unknown selection type: %+v", field)
		}
	}
}

// FindUsedVariables - find all used variables in a document and return a map of them
func FindUsedVariables(schema *ast.Schema, doc *ast.QueryDocument, fragments map[string]*ast.FragmentDefinition) map[string]string {
	vars := make(map[string]string, 0)
	for _, op := range doc.Operations {
		doFindUsedVariables(schema, string(op.Operation), op.SelectionSet, vars, fragments)
	}
	return vars
}
