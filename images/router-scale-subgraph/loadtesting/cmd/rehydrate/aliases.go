package main

import (
	"fmt"
	"math/rand/v2"

	log "github.com/sirupsen/logrus"
	"github.com/vektah/gqlparser/v2/ast"
)

// doFixAliases - recursively fill in all needed field aliases
func doFixAliases(selections ast.SelectionSet, fragments map[string]*ast.FragmentDefinition, useCache map[string]bool) {

	var nameCache map[string]bool
	if useCache != nil {
		nameCache = useCache
	} else {
		nameCache = make(map[string]bool, len(selections))
	}

	suffix := rand.IntN(1000)

	// process all fragment spreads first to reserve their existing aliases in this selection
	// scope
	for _, selection := range selections {
		if field, ok := selection.(*ast.FragmentSpread); ok {

			frag := fragments[field.Name]
			if frag != nil {
				doFixAliases(frag.SelectionSet, fragments, nameCache)
			} else {
				log.Errorf("Fragment spread cannot be found: %s", field.Name)
			}
		}
	}

	for _, selection := range selections {
		switch field := selection.(type) {
		case *ast.Field:
			if field.Name == "__typename" {
				continue
			}
			if _, ok := nameCache[field.Alias]; ok {
				// name exists, write alias
				field.Alias = fmt.Sprintf("%s%d", field.Alias, suffix)
				suffix += 1
			} else {
				nameCache[field.Alias] = true
			}
			if len(field.SelectionSet) > 0 {
				doFixAliases(field.SelectionSet, fragments, nil)
			}
		case *ast.InlineFragment:
			doFixAliases(field.SelectionSet, fragments, nameCache)
			/*for _, inner := range field.SelectionSet {
				field, ok := inner.(*ast.Field)
				if ok {
					if field.Name == "__typename" {
						continue
					}
					if _, ok := nameCache[field.Alias]; ok {
						// name exists, write alias
						field.Alias = fmt.Sprintf("%s%d", field.Alias, suffix)
						suffix += 1
					} else {
						nameCache[field.Alias] = true
					}
					if len(field.SelectionSet) > 0 {
						doFixAliases(field.SelectionSet, fragments)
					}
				} else {
					log.Errorf("Selection in fragment isn't field: %+v", inner)
				}
			}*/
		case *ast.FragmentSpread:
			// ok, already processed above
		default:
			log.Errorf("Unknown selection type: %T %+v", field, field)
		}
	}
}

func FixAliases(doc *ast.QueryDocument, fragments map[string]*ast.FragmentDefinition) {
	nameCache := make(map[string]bool, 0)
	for _, frag := range doc.Fragments {
		doFixAliases(frag.SelectionSet, fragments, nameCache)
	}
	for _, op := range doc.Operations {
		doFixAliases(op.SelectionSet, fragments, nil)
	}

}
