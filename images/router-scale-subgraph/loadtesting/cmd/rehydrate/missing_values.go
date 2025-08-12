package main

import (
	"fmt"

	"github.com/apollosolutions/loadtesting/pkg/tools"
	log "github.com/sirupsen/logrus"
	"github.com/vektah/gqlparser/v2/ast"
)

func checkJoin(field *ast.FieldDefinition) string {
	for _, dir := range field.Directives {
		if dir.Name == "join__field" {
			return dir.Arguments.ForName("graph").Value.String()
		}
	}
	return ""
}

// doFillMissingArguments - recursively fill in any input argument that is missing a value
func doFillMissingArguments(selections ast.SelectionSet, parentType string, vars map[string]interface{}, fragments map[string]*ast.FragmentDefinition, subgraphs map[string]bool) (count int) {

	//t := Schema.Types[parentType]

	for _, selection := range selections {
		switch field := selection.(type) {
		case *ast.Field:
			count += 1

			var fieldDef *ast.FieldDefinition

			switch parentType {
			case "query":
				fieldDef = Schema.Query.Fields.ForName(field.Name)
			case "mutation":
				fieldDef = Schema.Mutation.Fields.ForName(field.Name)
			case "subscription":
				fieldDef = Schema.Subscription.Fields.ForName(field.Name)
			default:
				log.Debugf("Parent Type is: %s", parentType)
				fieldDef = Schema.Types[parentType].Fields.ForName(field.Name)
			}

			if fieldDef == nil {
				// This is a scalar
				continue
			}

			if joinGraph := checkJoin(fieldDef); joinGraph != "" {
				subgraphs[joinGraph] = true
			}

			// create a map of field arguments (by name) from the schema
			fieldArgs := make(map[string]*ast.ArgumentDefinition)

			for _, arg := range fieldDef.Arguments {
				fieldArgs[arg.Name] = arg
			}

			// we sometimes see arguments that aren't in the schema which cases validation
			// errors, create a list of valid arguments and reassign to AST.
			newArgList := make([]*ast.Argument, 0, len(field.Arguments))

			for _, arg := range field.Arguments {
				log.Debugf("Arg %s=%s", arg.Name, arg.Value.Raw)
				if arg.Value.Kind != ast.Variable {

					if arg.Value == nil || arg.Value.Raw == "" {

						fieldArg := fieldArgs[arg.Name]
						realName := tools.FindRealName(fieldArg.Type)

						log.Debugf("Arg Needs Value! %s=%s (rn: %s)", arg.Name, arg.Value.Raw, realName)

						/*if !fieldArg.Type.NonNull {
							log.Errorf("IS NULLABLE")
							continue
						}*/

						if fieldArg.DefaultValue != nil {

							log.Debugf("Has Default: %+v", fieldArg.DefaultValue)
							arg.Value = nil
							arg.Value = fieldArg.DefaultValue
							continue
						}

						argType := Schema.Types[realName]

						log.Debugf("Arg still Needs Value! %s=%s (rn: %s)", arg.Name, arg.Value.Raw, realName)
						if argType == nil {

							log.Errorf("No type found for %s %s", arg.Name, fieldArg.Type.NamedType)
							log.Errorf("%+v", *fieldArg.Type)
							if fieldArg.Type.Elem != nil {
								log.Errorf("%+v", *fieldArg.Type.Elem)
							}
							continue
						}

						newValue := tools.TypeToFakeData(Schema, argType, realName, 0, nil, nil)

						arg.Value = createValue(newValue)

					}
				}
				if _, ok := fieldArgs[arg.Name]; ok {
					// selection arg in arguments defined on schema

					newArgList = append(newArgList, arg)
				}
			}
			// if we found unused args reassign the list
			if len(newArgList) > len(field.Arguments) {
				field.Arguments = nil
				field.Arguments = newArgList
				log.Warnf("Reassigning arguments due to invalid args: %s", parentType)
			}

			for _, dir := range field.Directives {

				for _, arg := range dir.Arguments {
					if arg.Value.Kind != ast.Variable {
						if arg.Value == nil {
							log.Errorf("We need to fill this directive value in. %s (dir: %s)", arg.Name, dir.Name)
						}
					}
				}
			}

			if len(field.SelectionSet) > 0 {
				count += doFillMissingArguments(field.SelectionSet, fieldDef.Type.Name(), vars, fragments, subgraphs)
			}
		case *ast.InlineFragment:
			count += doFillMissingArguments(field.SelectionSet, field.TypeCondition, vars, fragments, subgraphs)
		case *ast.FragmentSpread:
			for _, dir := range field.Directives {
				for _, arg := range dir.Arguments {
					if arg.Value.Kind != ast.Variable {
						vars[arg.Value.Raw] = true
						log.Debugf("Adding variable %s", arg.Value.Raw)
					}
				}
			}
			if frag, ok := fragments[field.Name]; ok {
				count += doFillMissingArguments(frag.SelectionSet, frag.TypeCondition, vars, fragments, subgraphs)
			}

		default:
			log.Errorf("Unknown selection type: %+v", field)
		}
	}
	return count
}

// FillMissingArguments - for all fields with inputs, fill in any missing input value returns count of all selections
func FillMissingArguments(doc *ast.QueryDocument, vars map[string]interface{}, fragments map[string]*ast.FragmentDefinition, subgraphs map[string]bool) int {
	count := 0
	for _, op := range doc.Operations {
		count += doFillMissingArguments(op.SelectionSet, string(op.Operation), vars, fragments, subgraphs)
	}
	return count
}

// getVal - create an AST Value for a given interface{}
func getVal(v interface{}) *ast.Value {
	var val *ast.Value

	switch x := v.(type) {
	case int:
		val = &ast.Value{
			Kind: ast.IntValue,
			Raw:  fmt.Sprintf("%d", x),
		}
	case float64:
		val = &ast.Value{
			Kind: ast.FloatValue,
			Raw:  fmt.Sprintf("%f", x),
		}
	case string:
		val = &ast.Value{
			Kind: ast.StringValue,
			Raw:  x,
		}
	case *string:
		val = &ast.Value{
			Kind: ast.EnumValue,
			Raw:  *x,
		}
	case bool:
		val = &ast.Value{
			Kind: ast.BooleanValue,
			Raw:  fmt.Sprintf("%t", x),
		}
	case []interface{}:

		vals := make([]*ast.ChildValue, 0, len(x))
		for _, el := range x {
			vals = append(vals, &ast.ChildValue{
				Name:  "",
				Value: getVal(el),
			})
		}
		val = &ast.Value{
			Kind:     ast.ListValue,
			Children: vals,
		}
	case map[string]interface{}:
		vals := make([]*ast.ChildValue, 0, len(x))
		for k, v := range x {
			vals = append(vals, &ast.ChildValue{
				Name:  k,
				Value: getVal(v),
			})
		}
		val = &ast.Value{
			Kind:     ast.ObjectValue,
			Children: vals,
		}
	case nil:
		val = &ast.Value{
			Kind: ast.NullValue,
		}
	default:
		panic(fmt.Sprintf("Should not happen type is %T", x))
	}
	return val
}

// createChildVal - create an AST Child Value for a given map of strings to interface{}
/*func createChildVal(input map[string]interface{}) ast.ChildValueList {

	vals := make([]*ast.ChildValue, 0, len(input))
	for k, v := range input {

		vals = append(vals, &ast.ChildValue{
			Name:  k,
			Value: getVal(v),
		})
	}
	return vals
}*/

// createValue - create root value for a given map of strings to interface{}
func createValue(input interface{}) *ast.Value {

	return getVal(input)
}
