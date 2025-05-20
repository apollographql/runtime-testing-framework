//! Helpers for supporting minimal templating of user config files.
use serde::{
    Deserialize, Deserializer,
    de::{self, DeserializeOwned, Visitor},
};
use std::{collections::HashMap, fmt, marker::PhantomData};

/// Errors that can occur while attempting to resolve a [Field]
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("the provided value did not deserialize correctly for {path}: {reason}")]
    InvalidData { path: String, reason: String },

    #[error("{value:?} is not a known value")]
    UnknownValue { value: String },
}

pub trait Templatable {
    /// Whether or not there are any pending [Field]s contained within this value.
    fn has_pending_fields(&self) -> bool;

    /// Attempt to resolve all pending [Field]s, appending encountered errors to the `errs` vec
    /// provided.
    fn try_resolve(
        &mut self,
        values: &HashMap<String, serde_json::Value>,
        path: &mut Vec<&'static str>,
        errs: &mut Vec<Error>,
    );

    /// Attempt to resolve all known [Field]s, reporting required values that are not present in
    /// the provided map. If there are any deserialization errors then then this method as an
    /// aggregate operation will fail.
    fn try_resolve_known(
        &mut self,
        values: &HashMap<String, serde_json::Value>,
        path: &mut Vec<&'static str>,
    ) -> Result<Vec<String>, Vec<Error>> {
        let mut all_errs = Vec::new();
        self.try_resolve(values, path, &mut all_errs);

        let mut missing = Vec::new();
        let mut errs = Vec::new();

        for err in all_errs.into_iter() {
            match err {
                Error::UnknownValue { value } => missing.push(value),
                err => errs.push(err),
            }
        }

        if errs.is_empty() {
            Ok(missing)
        } else {
            Err(errs)
        }
    }
}

/// A [Field] wraps some scalar type that implements [Templatable] in order to mark it as
/// requriring a templated value coming from user provided values as part of resolving the config
/// file.
///
/// Fields must be resolved in order to be usable during a test run.
#[derive(Debug, Clone, PartialEq)]
pub enum Field<T>
where
    T: ValidField,
{
    /// A pending field that should be replaced with the named value when it is available.
    Pending(String),
    /// A field containing the final data needed for resolving the config file.
    Resolved(T),
}

impl<T> Templatable for Field<T>
where
    T: ValidField,
{
    fn has_pending_fields(&self) -> bool {
        matches!(self, Self::Pending(_))
    }

    fn try_resolve(
        &mut self,
        values: &HashMap<String, serde_json::Value>,
        path: &mut Vec<&'static str>,
        errs: &mut Vec<Error>,
    ) {
        if let Self::Pending(value) = self {
            match values.get(value) {
                Some(raw) => match serde_json::from_value(raw.clone()) {
                    Ok(t) => *self = Self::Resolved(t),
                    Err(e) => errs.push(Error::InvalidData {
                        path: path.join("."),
                        reason: e.to_string(),
                    }),
                },
                None => errs.push(Error::UnknownValue {
                    value: value.clone(),
                }),
            }
        }
    }
}

// See the comments around FieldVisitor below for details of how this works
impl<'de, T: ValidField> Deserialize<'de> for Field<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Field<T>, D::Error> {
        deserializer.deserialize_any(FieldVisitor(PhantomData))
    }
}

/// A custom serde [Visitor] for locating template strings of the form `"{{ some_value }}"` and
/// marking them as pending fields. Malformed templates are reported as deserialization errors and
/// fields that do not contain template strings are deserialized as normal by deferring to the
/// default deserializer implementation for the type found in the input. (If the type in the input
/// is incorrect for the target then this will result in a deserialization error as normal).
struct FieldVisitor<T>(PhantomData<T>);

/// In order to pass through to the correct deserializer for each type care about, we need to
/// explicitly implement `visit_$type` as the [Visitor] trait will error by default. To avoid ending
/// up with a lot of verbose boiler plate code for this we stamp out these methods with a macro as
/// they all have the same general form.
macro_rules! impl_visit_for {
    ( $($ty:ty, $method:ident, $deser:ident;)+ ) => {
        $(
            fn $method<E: de::Error>(self, v: $ty) -> Result<Self::Value, E> {
                Deserialize::deserialize(de::value::$deser::new(v)).map(|t| Field::Resolved(t))
            }
        )+
    };
}

impl<'de, T> Visitor<'de> for FieldVisitor<T>
where
    T: ValidField,
{
    type Value = Field<T>;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a bool, integer, float or string")
    }

    /// When visiting a string we need to check to see if we have a valid template pattern or not.
    /// If we do then we extract the value name from it an return a Pending, otherwise we defer to
    /// the default handling for strings as with our other supported scalar types.
    /// If we detect a malformed template then we error _here_ rather treating it as a string and
    /// potentially leading to confusing runtime behaviour.
    fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
        let ident = match value.strip_prefix("{{ ") {
            Some(s) => s
                .strip_suffix(" }}")
                .ok_or(E::custom("unclosed field template"))?,

            None => {
                return Deserialize::deserialize(de::value::StrDeserializer::new(value))
                    .map(|t| Field::Resolved(t));
            }
        };

        let is_valid_identifier = !ident.is_empty()
            && ident.starts_with(char::is_alphabetic)
            && ident.chars().all(|ch| ch == '_' || ch.is_alphanumeric());

        if is_valid_identifier {
            Ok(Field::Pending(ident.to_string()))
        } else {
            Err(E::custom(
                "expected value identifier with a single space either side",
            ))
        }
    }

    impl_visit_for!(
        bool, visit_bool, BoolDeserializer;

        u8, visit_u8, U8Deserializer;
        u16, visit_u16, U16Deserializer;
        u32, visit_u32, U32Deserializer;
        u64, visit_u64, U64Deserializer;

        i8, visit_i8, I8Deserializer;
        i16, visit_i16, I16Deserializer;
        i32, visit_i32, I32Deserializer;
        i64, visit_i64, I64Deserializer;

        f32, visit_f32, F32Deserializer;
        f64, visit_f64, F64Deserializer;
    );
}

/// A marker trait for the types which may appear inside of a templated [Field] within a config file.
///
/// This trait is [sealed][0] to enforce that only known scalar types are supported.
///
/// [0]: https://rust-lang.github.io/api-guidelines/future-proofing.html#sealed-traits-protect-against-downstream-implementations-c-sealed
pub trait ValidField: fmt::Debug + Clone + DeserializeOwned + private::Sealed {}

mod private {
    use super::ValidField;
    pub trait Sealed {}

    // Stamp out the marker trait implementations we need for marking types as being templatable.
    macro_rules! impl_valid_field {
        ($($ty:ty),+) => {
            $(
                impl Sealed for $ty {}
                impl ValidField for $ty {}
            )+
        };
    }

    impl_valid_field!(
        u8, u16, u32, u64, usize, i8, i16, i32, i64, isize, f32, f64, bool, String
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use simple_test_case::test_case;

    #[derive(Debug, PartialEq, Deserialize)]
    #[serde(untagged)]
    enum Target {
        U { usize: Field<usize> },
        I { isize: Field<isize> },
        F { f64: Field<f64> },
        B { bool: Field<bool> },
        S { str: Field<String> },
    }

    #[test_case("usize: 42", Target::U { usize: Field::Resolved(42) }; "usize")]
    #[test_case("isize: -42", Target::I { isize: Field::Resolved(-42) }; "isize")]
    #[test_case("f64: 1.23", Target::F { f64: Field::Resolved(1.23) }; "f64")]
    #[test_case("bool: true", Target::B { bool: Field::Resolved(true) }; "bool")]
    #[test_case("str: testing", Target::S { str: Field::Resolved("testing".to_string()) }; "string")]
    #[test_case("usize: \"{{ foo }}\"", Target::U { usize: Field::Pending("foo".to_string()) }; "template usize")]
    #[test_case("isize: \"{{ foo }}\"", Target::I { isize: Field::Pending("foo".to_string()) }; "template isize")]
    #[test_case("f64: \"{{ foo }}\"", Target::F { f64: Field::Pending("foo".to_string()) }; "template f64")]
    #[test_case("bool: \"{{ foo }}\"", Target::B { bool: Field::Pending("foo".to_string()) }; "template bool")]
    #[test_case("str: \"{{ foo }}\"", Target::S { str: Field::Pending("foo".to_string()) }; "template string")]
    #[test]
    fn parsing_a_field_works(s: &str, expected: Target) {
        let target: Target = serde_yaml::from_str(s).unwrap();
        assert_eq!(target, expected)
    }

    #[derive(Debug, PartialEq, Deserialize)]
    struct S {
        field: Field<usize>,
    }

    #[test_case("foo"; "ascii")]
    #[test_case("BAR"; "upper case")]
    #[test_case("世界"; "unicode")]
    #[test_case("baz1"; "non leading digit")]
    #[test_case("foo_bar"; "internal underscore")]
    #[test]
    fn valid_value_identifiers_are_accepted(raw: &str) {
        let s = format!("field: \"{{{{ {raw} }}}}\"");
        let res: serde_yaml::Result<S> = serde_yaml::from_str(&s);
        assert!(res.is_ok(), "expected ok, got {res:?}");

        let S { field } = res.unwrap();
        assert_eq!(field, Field::Pending(raw.to_string()));
    }

    #[test_case("_foo"; "leading underscore")]
    #[test_case("1foo"; "leading digit")]
    #[test_case("foo!bar"; "contains punctuation")]
    #[test_case("foo bar"; "contains whitespace")]
    #[test_case(""; "no value name")]
    #[test_case("🦊"; "emoji")]
    #[test]
    fn invalid_value_identifiers_are_rejected(raw: &str) {
        let s = format!("field: \"{{{{ {raw} }}}}\"");
        let res: serde_yaml::Result<S> = serde_yaml::from_str(&s);
        assert!(res.is_err(), "expected error, got {res:?}");
    }

    #[test_case("\"{{foo }}\""; "no space before value name")]
    #[test_case("\"{{ foo}}\""; "no space after value name")]
    #[test_case("\"{{foo}}\""; "no spaces before or after value name")]
    #[test_case("\"{{ foo\""; "unclosed template")]
    #[test_case("\"{{ foo }\""; "single closing curly")]
    #[test_case("\"{ foo }}\""; "single opening curly")]
    #[test_case("\"{{  foo }}\""; "additional leading space")]
    #[test_case("\"{{ \tfoo }}\""; "leading tab")]
    #[test_case("\"{{ foo  }}\""; "additional trailing space")]
    #[test_case("\"{{ foo\t }}\""; "trailing tab")]
    #[test]
    fn malformed_templates_error(raw: &str) {
        let res: serde_yaml::Result<S> = serde_yaml::from_str(&format!("field: {raw}"));
        assert!(res.is_err(), "expected error, got {res:?}");
    }

    /// A simple example struct that contains a top level field as well as a field within a nested
    /// type.
    #[derive(Debug, PartialEq, Deserialize)]
    struct T {
        foo: Field<bool>,
        bar: U,
    }

    impl Templatable for T {
        fn has_pending_fields(&self) -> bool {
            self.foo.has_pending_fields() || self.bar.has_pending_fields()
        }

        fn try_resolve(
            &mut self,
            values: &HashMap<String, serde_json::Value>,
            path: &mut Vec<&'static str>,
            errs: &mut Vec<Error>,
        ) {
            let mut foo_path = path.clone();
            foo_path.push("foo");
            self.foo.try_resolve(values, &mut foo_path, errs);

            path.push("bar");
            self.bar.try_resolve(values, path, errs);
        }
    }

    #[derive(Debug, PartialEq, Deserialize)]
    struct U {
        baz: Field<u32>,
    }

    impl Templatable for U {
        fn has_pending_fields(&self) -> bool {
            self.baz.has_pending_fields()
        }

        fn try_resolve(
            &mut self,
            values: &HashMap<String, serde_json::Value>,
            path: &mut Vec<&'static str>,
            errs: &mut Vec<Error>,
        ) {
            path.push("baz");
            self.baz.try_resolve(values, path, errs);
        }
    }

    const RESOLVE_NO_PENDING: &str = "
foo: true
bar:
  baz: 17
";

    const RESOLVE_ALL_PENDING: &str = r#"
foo: "{{ value_A }}"
bar:
  baz: "{{ value_B }}"
"#;

    // Helper macro to create a HashMap<String, serde_json::Value> where the Value can be any valid json object.
    // Intended to be used for easily creating test values for testing templating
    macro_rules! values_map {
        () => {
            ::std::collections::HashMap::<String, ::serde_json::Value>::new()
        };

        ($($k:expr => $v:expr),+) => {{
            let mut m = ::std::collections::HashMap::new();

            $(
                m.insert($k.to_string(), ::serde_json::json!($v));
            )+

            m
        }};
    }

    #[test]
    fn resolving_with_nothing_to_template_works() {
        let mut t: T = serde_yaml::from_str(RESOLVE_NO_PENDING).unwrap();
        assert!(!t.has_pending_fields());

        let mut errs = Vec::new();
        t.try_resolve(&values_map!(), &mut Vec::new(), &mut errs);

        assert!(errs.is_empty(), "expected no errors, got {errs:?}");
        assert_eq!(
            t,
            T {
                foo: Field::Resolved(true),
                bar: U {
                    baz: Field::Resolved(17)
                }
            }
        );
    }

    #[test]
    fn resolving_with_all_values_available_works() {
        let mut t: T = serde_yaml::from_str(RESOLVE_ALL_PENDING).unwrap();
        assert!(t.has_pending_fields());

        let mut errs = Vec::new();
        let vals = values_map!(
            "value_A" => true,
            "value_B" => 17
        );
        t.try_resolve(&vals, &mut Vec::new(), &mut errs);

        assert!(errs.is_empty(), "expected no errors, got {errs:?}");
        assert_eq!(
            t,
            T {
                foo: Field::Resolved(true),
                bar: U {
                    baz: Field::Resolved(17)
                }
            }
        );
    }

    #[test]
    fn resolving_with_some_values_available_works() {
        let mut t: T = serde_yaml::from_str(RESOLVE_ALL_PENDING).unwrap();
        assert!(t.has_pending_fields());

        let vals = values_map!("value_A" => true);
        let res = t.try_resolve_known(&vals, &mut Vec::new());

        assert!(res.is_ok(), "expected no errors, got {res:?}");

        let missing = res.unwrap();
        assert_eq!(missing, vec!["value_B".to_string()]);

        assert_eq!(
            t,
            T {
                foo: Field::Resolved(true),
                bar: U {
                    baz: Field::Pending("value_B".to_string()),
                }
            }
        );
    }

    #[test]
    fn resolving_with_no_values_available_works() {
        let mut t: T = serde_yaml::from_str(RESOLVE_ALL_PENDING).unwrap();
        assert!(t.has_pending_fields());

        let vals = values_map!();
        let res = t.try_resolve_known(&vals, &mut Vec::new());

        assert!(res.is_ok(), "expected no errors, got {res:?}");

        let missing = res.unwrap();
        assert_eq!(missing, vec!["value_A".to_string(), "value_B".to_string()]);

        assert_eq!(
            t,
            T {
                foo: Field::Pending("value_A".to_string()),
                bar: U {
                    baz: Field::Pending("value_B".to_string()),
                }
            }
        );
    }

    #[test]
    fn resolving_with_invalid_data_errors() {
        let mut t: T = serde_yaml::from_str(RESOLVE_ALL_PENDING).unwrap();
        assert!(t.has_pending_fields());

        let vals = values_map!("value_A" => true, "value_B" => 1.23);
        let res = t.try_resolve_known(&vals, &mut Vec::new());

        assert!(res.is_err(), "expected no errors, got {res:?}");

        let err = res.unwrap_err().remove(0);
        assert_eq!(
            err,
            Error::InvalidData {
                path: "bar.baz".to_string(),
                reason: "invalid type: floating point `1.23`, expected u32".to_string()
            }
        );

        assert_eq!(
            t,
            T {
                foo: Field::Resolved(true),
                bar: U {
                    baz: Field::Pending("value_B".to_string()),
                }
            }
        );
    }
}
