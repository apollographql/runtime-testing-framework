package main

func GetFederationSymbols(version int) string {

	// add version-specific changes on key directive, as well as adding the new directives for federation 2
	if version == 1 {
		return `
	directive @key(fields: _FieldSet!) repeatable on OBJECT | INTERFACE
	directive @requires(fields: _FieldSet!) on FIELD_DEFINITION
	directive @provides(fields: _FieldSet!) on FIELD_DEFINITION
	directive @extends on OBJECT | INTERFACE
	directive @external on FIELD_DEFINITION
	directive @tag(name: String!) repeatable on
	  | ARGUMENT_DEFINITION
	  | ENUM
	  | ENUM_VALUE
	  | FIELD_DEFINITION
	  | INPUT_FIELD_DEFINITION
	  | INPUT_OBJECT
	  | INTERFACE
	  | OBJECT
	  | SCALAR
	  | UNION
	scalar _Any
	scalar _FieldSet
`
	} else if version == 2 {
		return `
	directive @authenticated on FIELD_DEFINITION | OBJECT | INTERFACE | SCALAR | ENUM
	directive @composeDirective(name: String!) repeatable on SCHEMA
	directive @extends on OBJECT | INTERFACE
	directive @external on OBJECT | FIELD_DEFINITION
	directive @key(fields: FieldSet!, resolvable: Boolean = true) repeatable on OBJECT | INTERFACE
	directive @inaccessible on
	  | ARGUMENT_DEFINITION
	  | ENUM
	  | ENUM_VALUE
	  | FIELD_DEFINITION
	  | INPUT_FIELD_DEFINITION
	  | INPUT_OBJECT
	  | INTERFACE
	  | OBJECT
	  | SCALAR
	  | UNION
	directive @interfaceObject on OBJECT
	directive @link(import: [String!], url: String!) repeatable on SCHEMA
	directive @override(from: String!, label: String) on FIELD_DEFINITION
	directive @policy(policies: [[federation__Policy!]!]!) on 
	  | FIELD_DEFINITION
	  | OBJECT
	  | INTERFACE
	  | SCALAR
	  | ENUM
	directive @provides(fields: FieldSet!) on FIELD_DEFINITION
	directive @requires(fields: FieldSet!) on FIELD_DEFINITION
	directive @requiresScopes(scopes: [[federation__Scope!]!]!) on 
	  | FIELD_DEFINITION
	  | OBJECT
	  | INTERFACE
	  | SCALAR
	  | ENUM
	directive @shareable repeatable on FIELD_DEFINITION | OBJECT
	directive @tag(name: String!) repeatable on
	  | ARGUMENT_DEFINITION
	  | ENUM
	  | ENUM_VALUE
	  | FIELD_DEFINITION
	  | INPUT_FIELD_DEFINITION
	  | INPUT_OBJECT
	  | INTERFACE
	  | OBJECT
	  | SCALAR
	  | UNION
	scalar _Any
	scalar FieldSet
	scalar federation__Policy
	scalar federation__Scope
`
	}
	return ""
}
