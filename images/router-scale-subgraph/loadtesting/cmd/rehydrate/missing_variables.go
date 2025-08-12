package main

import (
	"math/rand/v2"

	"github.com/apollosolutions/loadtesting/pkg/tools"
	log "github.com/sirupsen/logrus"
	"github.com/vektah/gqlparser/v2/ast"
)

// CreateVariableValues - fill in variable values based on the schema returns a value for each variable
//   - doc the parsed query document
//   - okVars are a list of variable names that we know are good
func CreateVariableValues(doc *ast.QueryDocument, okVars map[string]string) map[string]interface{} {
	result := make(map[string]interface{}, 0)

	for _, op := range doc.Operations {

		// check to see if the variables for this op are all found
		for _, variable := range op.VariableDefinitions {
			if _, ok := okVars[variable.Variable]; ok {
				// If this variable is used in the query keep it around
				log.Debugf("Found %s in used variables", variable.Variable)
			} else {
				log.Errorf("Cannot find variable: %s", variable.Variable)
				panic("Cannot find variable.")
			}

			isArray := false
			loop := 1

			if variable.Type.String()[0] == '[' {
				isArray = true
				loop = 4
			}

			data := make([]interface{}, loop)

			for i := 0; i < loop; i++ {

				switch findRealName(variable.Type) {
				case "String":
					data[i] = RandStringBytesMaskImprSrcUnsafe(20)
				case "Int":
					data[i] = rand.IntN(100)
				case "Float":
					data[i] = rand.Float64()
				case "Boolean":
					random := rand.IntN(100)
					data[i] = (random >= 50)
				case "ID":
					data[i] = RandStringBytesMaskImprSrcUnsafe(10)
				default:
					if Schema != nil {
						log.Debugf("Found input object: %s", variable.Type.Name())
						t := Schema.Types[variable.Type.Name()]
						data[i] = tools.TypeToFakeData(Schema, t, variable.Type.Name(), 0, nil, nil)

						//for _, field := range t.Fields {
						//	data[i] = tools.TypeToFakeData(t, variable.Type.Name(), Schema)
						//}
					} else {
						log.Errorf("Unknown type: %s", tools.FindRealName(variable.Type))
					}
				}
			}

			if isArray {
				result[variable.Variable] = data
			} else {
				result[variable.Variable] = data[0]
			}
		}
	}
	return result
}
