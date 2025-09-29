use rtf_config::templating::{Field, Template, ValidField};
use rtf_derive::Template;
use simple_test_case::test_case;

/// Construct a pending field
fn pending<T: ValidField>(s: &str) -> Field<T> {
    Field::Pending(s.to_string())
}

/// Construct a resolved field
fn resolved<T: ValidField>(t: impl Into<T>) -> Field<T> {
    Field::Resolved(t.into())
}

/// Used to test a single field has correctly implements Template
#[derive(Debug, Template)]
struct SingleField {
    foo: Field<String>,
}

/// Used to test multiple fields have correctly implements Template
#[derive(Debug, Template)]
struct MultiField {
    foo: Field<String>,
    bar: Field<String>,
    baz: Field<String>,
}

/// Used to test that an inner struct results in correctly implements Template
#[derive(Debug, Template)]
struct NestedStruct {
    foo: InnerStruct,
}

#[derive(Debug, Template)]
struct InnerStruct {
    inner: Field<String>,
}

/// Construct a pending inner struct
fn pending_inner(s: &str) -> InnerStruct {
    InnerStruct { inner: pending(s) }
}

/// Construct a resolved inner struct
fn resolved_inner(s: &str) -> InnerStruct {
    InnerStruct { inner: resolved(s) }
}

// Used to test that the combination of fields and structs results in correctly implements Template
#[derive(Debug, Template)]
struct MultiFieldWithInnerStruct {
    foo: Field<String>,
    bar: Field<String>,
    baz: InnerStruct,
}

/// Used to test that a skipped Field has no impact on the Template implementation
#[derive(Debug, Template)]
struct SkippedField {
    foo: Field<String>,
    #[template(skip)]
    #[allow(dead_code)]
    bar: Field<String>,
}

/// Used to check that a skipped non-field introduces no change to Template implementation
#[derive(Debug, Template)]
struct SkippedNotField {
    foo: Field<String>,
    #[template(skip)]
    #[allow(dead_code)]
    bar: String,
}

/// Used to test that nesting fields in multiple pub structs results in the correctly implements Template
#[derive(Debug, Template)]
struct MultiNestedStruct {
    inner: InnerStructWithInnerStruct,
}

#[derive(Debug, Template)]
struct InnerStructWithInnerStruct {
    inner: InnerStruct,
}

/// Construct a pending inner struct within an inner struct
fn pending_inner_within_inner(s: &str) -> InnerStructWithInnerStruct {
    InnerStructWithInnerStruct {
        inner: InnerStruct { inner: pending(s) },
    }
}

/// Construct a resolved inner struct within an inner struct
fn resolved_inner_within_inner(s: &str) -> InnerStructWithInnerStruct {
    InnerStructWithInnerStruct {
        inner: InnerStruct { inner: resolved(s) },
    }
}

/// Used test that the Template implementation works when applied to an enum
#[derive(Debug, Template)]
enum TemplateTypes {
    Field(Field<String>),
    SingleField(SingleField),
}

#[test_case(Box::new(SingleField {foo: pending("foo")}), true; "single_field_is_pending")]
#[test_case(Box::new(SingleField {foo: resolved("foo")}), false; "single_field_is_resolved")]
#[test_case(Box::new(MultiField {foo: pending("foo"), bar: pending("bar"), baz: pending("baz")}), true; "multi_field_all_fields_pending")]
#[test_case(Box::new(MultiField {foo: pending("foo"), bar: resolved("bar"), baz: resolved("baz")}), true; "multi_field_single_field_pending")]
#[test_case(Box::new(MultiField {foo: resolved("foo"), bar: resolved("bar"), baz: resolved("baz")}), false; "multi_field_all_fields_resolved")]
#[test_case(Box::new(NestedStruct {foo: pending_inner("inner")}), true; "inner_structs_field_is_pending")]
#[test_case(Box::new(NestedStruct {foo: resolved_inner("inner")}), false; "inner_structs_field_is_resolved")]
#[test_case(Box::new(MultiFieldWithInnerStruct {foo: pending("foo"), bar: pending("bar"), baz: pending_inner("inner")}), true; "multi_field_with_inner_struct_all_fields_pending")]
#[test_case(Box::new(MultiFieldWithInnerStruct {foo: resolved("foo"), bar: resolved("bar"), baz: pending_inner("inner")}), true; "multi_field_with_inner_struct_inner_field_pending")]
#[test_case(Box::new(MultiFieldWithInnerStruct {foo: pending("foo"), bar: resolved("bar"), baz: resolved_inner("inner")}), true; "multi_field_with_inner_struct_outer_field_pending")]
#[test_case(Box::new(MultiFieldWithInnerStruct {foo: resolved("foo"), bar: resolved("bar"), baz: resolved_inner("inner")}), false; "multi_field_with_inner_struct_resolved")]
#[test_case(Box::new(SkippedField{foo: resolved("foo"), bar: pending("bar")}), false; "skipped_field_does_make_status_pending")]
#[test_case(Box::new(SkippedNotField{foo: pending("foo"), bar: "bar".to_string()}), true; "skipped_not_field_pending")]
#[test_case(Box::new(SkippedNotField{foo: resolved("foo"), bar: "bar".to_string()}), false; "skipped_not_field_resolved")]
#[test_case(Box::new(MultiNestedStruct{inner: pending_inner_within_inner("inner")}), true; "nested_inner_structs_field_is_pending")]
#[test_case(Box::new(MultiNestedStruct{inner: resolved_inner_within_inner("inner")}), false; "nested_inner_structs_field_is_resolved")]
#[test_case(Box::new(TemplateTypes::Field(pending("foo"))), true; "field_in_enum_is_pending")]
#[test_case(Box::new(TemplateTypes::Field(resolved("foo"))), false; "field_in_enum_is_resolved")]
#[test_case(Box::new(TemplateTypes::SingleField(SingleField{foo: pending("foo")})), true; "struct_in_enum_is_pending")]
#[test_case(Box::new(TemplateTypes::SingleField(SingleField{foo: resolved("foo")})), false; "struct_in_enum_is_resolved")]
#[test]
fn has_pending_fields(t: Box<dyn Template>, expected: bool) {
    let res = t.has_pending_fields();
    assert!(
        res == expected,
        "expected has pending fields to be {expected:?}, got {res:?}"
    )
}

#[test_case(Box::new(SingleField {foo: pending("foo")}), vec!["foo"]; "single_field_value_required")]
#[test_case(Box::new(SingleField {foo: resolved("foo")}), vec![]; "single_field_no_value_required")]
#[test_case(Box::new(MultiField {foo: pending("foo"), bar: pending("bar"), baz: pending("baz")}), vec!["foo", "bar", "baz"]; "multi_field_all_fields_required")]
#[test_case(Box::new(MultiField {foo: pending("foo"), bar: resolved("bar"), baz: resolved("baz")}), vec!["foo"]; "multi_field_single_field_required")]
#[test_case(Box::new(MultiField {foo: resolved("foo"), bar: resolved("bar"), baz: resolved("baz")}), vec![]; "multi_field_no_fields_required")]
#[test_case(Box::new(NestedStruct {foo: pending_inner("inner")}), vec!["inner"]; "inner_structs_field_required")]
#[test_case(Box::new(NestedStruct {foo: resolved_inner("inner")}), vec![]; "inner_structs_no_fields_required")]
#[test_case(Box::new(MultiFieldWithInnerStruct {foo: pending("foo"), bar: pending("bar"), baz: pending_inner("inner")}), vec!["foo", "bar", "inner"]; "multi_field_with_inner_struct_all_fields_required")]
#[test_case(Box::new(MultiFieldWithInnerStruct {foo: resolved("foo"), bar: resolved("bar"), baz: pending_inner("inner")}), vec!["inner"]; "multi_field_with_inner_struct_inner_field_required")]
#[test_case(Box::new(MultiFieldWithInnerStruct {foo: pending("foo"), bar: resolved("bar"), baz: resolved_inner("inner")}), vec!["foo"]; "multi_field_with_inner_struct_outer_field_required")]
#[test_case(Box::new(MultiFieldWithInnerStruct {foo: resolved("foo"), bar: resolved("bar"), baz: resolved_inner("inner")}), vec![]; "multi_field_with_inner_struct_no_fields_required")]
#[test_case(Box::new(SkippedField{foo: resolved("foo"), bar: pending("bar")}), vec![]; "skipped_field_does_add_required_value")]
#[test_case(Box::new(SkippedNotField{foo: pending("foo"), bar: "bar".to_string()}), vec!["foo"]; "skipped_not_field_field_required")]
#[test_case(Box::new(SkippedNotField{foo: resolved("foo"), bar: "bar".to_string()}), vec![]; "skipped_not_field_no_field_required")]
#[test_case(Box::new(MultiNestedStruct{inner: pending_inner_within_inner("inner")}), vec!["inner"]; "nested_inner_structs_field_is_required")]
#[test_case(Box::new(MultiNestedStruct{inner: resolved_inner_within_inner("inner")}), vec![]; "nested_inner_structs_no_field_is_required")]
#[test_case(Box::new(TemplateTypes::Field(pending("foo"))), vec!["foo"]; "field_in_enum_is_required")]
#[test_case(Box::new(TemplateTypes::Field(resolved("foo"))), vec![]; "field_in_enum_is_not_required")]
#[test_case(Box::new(TemplateTypes::SingleField(SingleField{foo: pending("foo")})), vec!["foo"]; "struct_in_enum_field_is_required")]
#[test_case(Box::new(TemplateTypes::SingleField(SingleField{foo: resolved("foo")})), vec![]; "struct_in_enum_no_field_is_required")]
#[test]
fn required_values(t: Box<dyn Template>, expected: Vec<&str>) {
    let res = t.required_values();
    assert_eq!(
        res, expected,
        "expected required values to be {expected:?}, got {res:?}"
    )
}
