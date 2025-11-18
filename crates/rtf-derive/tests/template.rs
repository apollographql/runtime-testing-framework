#![allow(clippy::disallowed_names)]

use rtf_config::{
    Source,
    templating::{Field, Scalar, Template, TemplateContext, ValidField},
};
use rtf_derive::Template;
use simple_test_case::test_case;

/// Construct a pending field
fn p<T: ValidField>(s: &str) -> Field<T> {
    Field::Pending(s.to_string())
}

/// Construct a resolved field
fn r<T: ValidField>(t: impl Into<T>) -> Field<T> {
    Field::Resolved(t.into())
}

/// Used to test a single field has correctly implements Template
#[derive(Debug, Template)]
struct SingleField {
    foo: Field<String>,
}

fn sinf(foo: Field<String>) -> Box<SingleField> {
    Box::new(SingleField { foo })
}

// Used to test that a struct with no templatable fields correctly implements Template
#[derive(Debug, Template)]
struct NoTemplatableFields {
    #[template(skip)]
    #[allow(dead_code)]
    foo: String,
    #[template(skip)]
    #[allow(dead_code)]
    bar: String,
}

fn ntf(foo: &str, bar: &str) -> Box<NoTemplatableFields> {
    Box::new(NoTemplatableFields {
        foo: foo.to_string(),
        bar: bar.to_string(),
    })
}

/// Used to test multiple fields have correctly implements Template
#[derive(Debug, Template)]
struct MultiField {
    foo: Field<String>,
    bar: Field<String>,
    baz: Field<String>,
}

fn mf(foo: Field<String>, bar: Field<String>, baz: Field<String>) -> Box<MultiField> {
    Box::new(MultiField { foo, bar, baz })
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

fn ns(inner: Field<String>) -> Box<NestedStruct> {
    Box::new(NestedStruct {
        foo: InnerStruct { inner },
    })
}

// Used to test that the combination of fields and structs results in correctly implements Template
#[derive(Debug, Template)]
struct MultiFieldWithInnerStruct {
    foo: Field<String>,
    bar: Field<String>,
    baz: InnerStruct,
}

fn mfwis(
    foo: Field<String>,
    bar: Field<String>,
    inner: Field<String>,
) -> Box<MultiFieldWithInnerStruct> {
    Box::new(MultiFieldWithInnerStruct {
        foo,
        bar,
        baz: InnerStruct { inner },
    })
}

/// Used to test that a skipped Field has no impact on the Template implementation
#[derive(Debug, Template)]
struct SkippedField {
    foo: Field<String>,
    #[template(skip)]
    #[allow(dead_code)]
    bar: Field<String>,
}

fn skipf(foo: Field<String>, bar: Field<String>) -> Box<SkippedField> {
    Box::new(SkippedField { foo, bar })
}

/// Used to check that a skipped non-field introduces no change to Template implementation
#[derive(Debug, Template)]
struct SkippedNotField {
    foo: Field<String>,
    #[template(skip)]
    #[allow(dead_code)]
    bar: String,
}

fn skipnf(foo: Field<String>, bar: &str) -> Box<SkippedNotField> {
    Box::new(SkippedNotField {
        foo,
        bar: bar.to_string(),
    })
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

fn mns(inner: Field<String>) -> Box<MultiNestedStruct> {
    Box::new(MultiNestedStruct {
        inner: InnerStructWithInnerStruct {
            inner: InnerStruct { inner },
        },
    })
}

/// Used test that the Template implementation works when applied to an enum
#[derive(Debug, Template)]
enum TemplateTypes {
    Field(Field<String>),
    SingleField(SingleField),
    Unit,
}

fn ttf(inner: Field<String>) -> Box<TemplateTypes> {
    Box::new(TemplateTypes::Field(inner))
}

fn ttsf(foo: Field<String>) -> Box<TemplateTypes> {
    Box::new(TemplateTypes::SingleField(SingleField { foo }))
}

#[test_case(sinf(p("foo")), true; "single_field_is_pending")]
#[test_case(sinf(r("foo")), false; "single_field_is_resolved")]
#[test_case(ntf("foo", "bar"), false; "no_templatable_fields_is_resolved")]
#[test_case(mf(p("foo"), p("bar"), p("baz")), true; "multi_field_all_fields_pending")]
#[test_case(mf(p("foo"), r("bar"), r("baz")), true; "multi_field_single_field_pending")]
#[test_case(mf(r("foo"), r("bar"), r("baz")), false; "multi_field_all_fields_resolved")]
#[test_case(ns(p("inner")), true; "inner_structs_field_is_pending")]
#[test_case(ns(r("inner")), false; "inner_structs_field_is_resolved")]
#[test_case(mfwis(p("foo"), p("bar"), p("inner")), true; "multi_field_with_inner_struct_all_fields_pending")]
#[test_case(mfwis(r("foo"), r("bar"), p("inner")), true; "multi_field_with_inner_struct_inner_field_pending")]
#[test_case(mfwis(p("foo"), r("bar"), r("inner")), true; "multi_field_with_inner_struct_outer_field_pending")]
#[test_case(mfwis(r("foo"), r("bar"), r("inner")), false; "multi_field_with_inner_struct_resolved")]
#[test_case(skipf(r("foo"), p("bar")), false; "skipped_field_does_make_status_pending")]
#[test_case(skipnf(p("foo"), "bar"), true; "skipped_not_field_pending")]
#[test_case(skipnf(r("foo"), "bar"), false; "skipped_not_field_resolved")]
#[test_case(mns(p("inner")), true; "nested_inner_structs_field_is_pending")]
#[test_case(mns(r("inner")), false; "nested_inner_structs_field_is_resolved")]
#[test_case(ttf(p("foo")), true; "field_in_enum_is_pending")]
#[test_case(ttf(r("foo")), false; "field_in_enum_is_resolved")]
#[test_case(ttsf(p("foo")), true; "struct_in_enum_is_pending")]
#[test_case(ttsf(r("foo")), false; "struct_in_enum_is_resolved")]
#[test]
fn has_pending_fields(t: Box<dyn Template>, expected: bool) {
    let res = t.has_pending_fields();
    assert!(
        res == expected,
        "expected has pending fields to be {expected:?}, got {res:?}"
    )
}

#[test_case(sinf(p("foo")), &["foo"]; "single_field_field_required")]
#[test_case(sinf(r("foo")), &[]; "single_field_no_fields_required")]
#[test_case(ntf("foo", "bar"), &[]; "no_templatable_fields_no_fields_required")]
#[test_case(mf(p("foo"), p("bar"), p("baz")), &["foo", "bar", "baz"]; "multi_field_all_fields_required")]
#[test_case(mf(p("foo"), r("bar"), r("baz")), &["foo"]; "multi_field_single_field_required")]
#[test_case(mf(r("foo"), r("bar"), r("baz")), &[]; "multi_field_no_fields_required")]
#[test_case(ns(p("inner")), &["inner"]; "inner_structs_field_required")]
#[test_case(ns(r("inner")), &[]; "inner_structs_no_fields_required")]
#[test_case(mfwis(p("foo"), p("bar"), p("inner")), &["foo", "bar", "inner"]; "multi_field_with_inner_struct_all_fields_required")]
#[test_case(mfwis(r("foo"), r("bar"), p("inner")), &["inner"]; "multi_field_with_inner_struct_inner_field_required")]
#[test_case(mfwis(p("foo"), r("bar"), r("inner")), &["foo"]; "multi_field_with_inner_struct_outer_field_required")]
#[test_case(mfwis(r("foo"), r("bar"), r("inner")), &[]; "multi_field_with_inner_struct_no_fields_required")]
#[test_case(skipf(r("foo"), p("bar")), &[]; "skipped_field_does_add_required_variable")]
#[test_case(skipnf(p("foo"), "bar"), &["foo"]; "skipped_not_field_field_required")]
#[test_case(skipnf(r("foo"), "bar"), &[]; "skipped_not_field_no_field_required")]
#[test_case(mns(p("inner")), &["inner"]; "nested_inner_structs_field_is_required")]
#[test_case(mns(r("inner")), &[]; "nested_inner_structs_no_field_is_required")]
#[test_case(ttf(p("foo")), &["foo"]; "field_in_enum_is_required")]
#[test_case(ttf(r("foo")), &[]; "field_in_enum_is_not_required")]
#[test_case(ttsf(p("foo")), &["foo"]; "struct_in_enum_field_is_required")]
#[test_case(ttsf(r("foo")), &[]; "struct_in_enum_no_field_is_required")]
#[test]
fn required_variables(t: Box<dyn Template>, expected: &[&str]) {
    let res = t.required_variables();
    assert_eq!(
        res.as_slice(),
        expected,
        "expected required variables to be {expected:?}, got {res:?}"
    )
}

macro_rules! template_context {
    ($slice:expr) => {{
        let mut m = ::std::collections::HashMap::new();
        for k in $slice {
            m.insert(k.to_string(), Scalar::from(k.to_string()));
        }

        TemplateContext::new(m, Source::local("/"), Default::default())
    }};
}

#[test_case(sinf(p("foo")), &["foo"]; "single_field")]
#[test_case(sinf(p("foo")), &["foo", "bar"]; "single_field_unused_variable")]
#[test_case(ntf("foo", "bar"), &[]; "no_templatable_fields")]
#[test_case(mf(p("foo"), p("bar"), p("baz")), &["foo", "bar", "baz"]; "multi_field")]
#[test_case(ns(p("inner")), &["inner"]; "nested_struct")]
#[test_case(mfwis(p("foo"), p("bar"), p("inner")), &["foo", "bar", "inner"]; "multi_field_with_inner_struct")]
#[test_case(skipf(p("foo"), p("bar")), &["foo", "bar"]; "skipped_field")]
#[test_case(skipnf(p("foo"), "bar"), &["foo"]; "skipped_not_field")]
#[test_case(mns(p("inner")), &["inner"]; "nested_inner_structs")]
#[test_case(ttf(p("foo")), &["foo"]; "field_in_enum")]
#[test_case(ttsf(p("foo")), &["foo"]; "struct_in_enum")]
#[test]
fn try_template_all_fields(mut t: Box<dyn Template>, variables: &[&str]) {
    let template_ctx = template_context!(variables);
    let res = t.try_template(&mut Vec::new(), &Source::local("/"), &template_ctx);
    assert!(
        res.is_ok(),
        "expected to template successfully, got {res:?}"
    )
}

#[test_case(sinf(p("foo")); "single_field")]
#[test_case(sinf(p("foo")); "single_field_unused_variable")]
#[test_case(mf(p("foo"), p("bar"), p("baz")); "multi_field")]
#[test_case(ns(p("inner")); "nested_struct")]
#[test_case(mfwis(p("foo"), p("bar"), p("inner")); "multi_field_with_inner_struct")]
#[test_case(skipf(p("foo"), p("bar")); "skipped_field")]
#[test_case(skipnf(p("foo"), "bar"); "skipped_not_field")]
#[test_case(mns(p("inner")); "nested_inner_structs")]
#[test_case(ttf(p("foo")); "field_in_enum")]
#[test_case(ttsf(p("foo")); "struct_in_enum")]
#[test]
fn try_template_unknown_variable_error(mut t: Box<dyn Template>) {
    let template_ctx = template_context!(["unused"]);

    let res = t.try_template(&mut Vec::new(), &Source::local("/"), &template_ctx);
    assert!(res.is_err(), "expected templating to fail, got {res:?}");
    let errors = res.unwrap_err();
    assert!(
        errors
            .iter()
            .all(|e| matches!(e.kind, rtf_config::templating::ErrorKind::UnknownVariable)),
        "expected all errors to be UnknownVariable, got {:?}",
        errors
    );
}

#[test]
fn template_enum_unit_skipped() {
    let mut t = TemplateTypes::Unit;
    let template_ctx = template_context!(["unused"]);

    assert!(
        !t.has_pending_fields(),
        "A unit type enum variant should never have pending fields"
    );

    assert!(
        t.required_variables().is_empty(),
        "A unit type enum variant should have no required variables"
    );

    let res = t.try_template(&mut Vec::new(), &Source::local("/"), &template_ctx);
    assert!(res.is_ok(), "A unit type enum should template successfully");
}
