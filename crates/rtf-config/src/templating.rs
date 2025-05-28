//! Helpers for supporting minimal templating of user config files.
use serde::{
    Deserialize, Deserializer, Serialize,
    de::{self, DeserializeOwned, Visitor},
};
use std::{collections::HashMap, fmt, marker::PhantomData};

/// User facing descriptions of the reason that templating a [Field] failed.
///
/// Paired with an additional message to form an [Error].
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::Display, strum::EnumString)]
pub enum ErrorKind {
    #[strum(to_string = "invalid templating value")]
    InvalidData,

    #[strum(to_string = "unknown templating value")]
    UnknownValue,
}

// Type aliases for validation error handling.
// Elsewhere in the codebase we should always refer to these aliases rather than parameterising the
// generic types from the error module.

pub type Error = crate::error::Error<ErrorKind>;
pub type Errors = crate::error::Errors<ErrorKind>;
pub type ErrorBuilder = crate::error::ErrorBuilder<ErrorKind>;
pub type Result<T> = std::result::Result<T, Errors>;

/// In order to support controlled templating of config files with [Scalar] values we make use of a
/// wrapper [Field] type to identify where values need to be injected. A type that implements
/// [Template] supports walking its contents to locate and resolve fields using a provided map
/// of scalar values.
pub trait Template {
    /// Whether or not there are any pending [Field]s contained within this value.
    fn has_pending_fields(&self) -> bool;

    /// Attempt to resolve all pending [Field]s, appending encountered errors to the `errs` vec
    /// provided.
    fn try_resolve(
        &mut self,
        path: &mut Vec<String>,
        values: &HashMap<String, Scalar>,
    ) -> Result<()>;

    /// Attempt to resolve all pending [Field]s when this type is a child of some parent
    /// [Template], appending encountered errors to the `errs` vec provided. The provided `tail`
    /// will be appended to `path` before calling through to [Template::try_resolve].
    fn try_resolve_nested(
        &mut self,
        path: &mut Vec<String>,
        tail: impl Into<String>,
        values: &HashMap<String, Scalar>,
    ) -> Result<()> {
        let mut path = path.clone();
        path.push(tail.into());
        self.try_resolve(&mut path, values)
    }

    /// Attempt to resolve all known [Field]s, reporting required values that are not present in
    /// the provided map. If there are any deserialization errors then then this method as an
    /// aggregate operation will fail.
    fn try_resolve_known(&mut self, values: &HashMap<String, Scalar>) -> Result<Vec<String>> {
        let all_errs = match self.try_resolve(&mut Vec::new(), values) {
            Ok(_) => return Ok(Vec::new()),
            Err(errs) => errs,
        };

        let mut errs = ErrorBuilder::new();
        let mut missing = Vec::new();

        for err in all_errs.into_iter() {
            match err.kind {
                ErrorKind::UnknownValue => missing.push(err.message),
                _ => errs.push_err(err),
            }
        }

        errs.into_result(missing)
    }
}

/// A [Field] wraps some scalar type that implements [Template] in order to mark it as
/// requriring a templated value coming from user provided values as part of resolving the config
/// file.
///
/// Fields must be resolved in order to be usable during a test run.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum Field<T>
where
    T: ValidField,
{
    /// A pending field that should be replaced with the named value when it is available.
    Pending(String),
    /// A field containing the final data needed for resolving the config file.
    Resolved(T),
}

impl<T> Field<T>
where
    T: ValidField,
{
    // FIXME: RR-78 will remove the need for this (required for MVP)
    pub(crate) fn as_resolved(&self) -> &T {
        match self {
            Self::Pending(_) => panic!("field is still pending"),
            Self::Resolved(t) => t,
        }
    }
}

impl<T> Template for Field<T>
where
    T: ValidField,
{
    fn has_pending_fields(&self) -> bool {
        matches!(self, Self::Pending(_))
    }

    fn try_resolve(
        &mut self,
        path: &mut Vec<String>,
        values: &HashMap<String, Scalar>,
    ) -> Result<()> {
        if let Self::Pending(value) = self {
            match values.get(value) {
                Some(raw) => match raw.clone().try_into() {
                    Ok(t) => *self = Self::Resolved(t),
                    Err(reason) => return Err(Errors::new(ErrorKind::InvalidData, reason, path)),
                },
                None => return Err(Errors::new(ErrorKind::UnknownValue, value.clone(), path)),
            }
        }

        Ok(())
    }
}

// See the comments around FieldVisitor below for details of how this works
impl<'de, T: ValidField> Deserialize<'de> for Field<T> {
    fn deserialize<D: Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Field<T>, D::Error> {
        deserializer.deserialize_any(FieldVisitor(PhantomData))
    }
}

/// A custom serde [Visitor] for locating template strings of the form `"{{ some_value }}"` and
/// marking them as pending fields. Malformed templates are reported as deserialization errors and
/// fields that do not contain template strings are deserialized as normal by deferring to the
/// default deserializer implementation for the type found in the input. (If the type in the input
/// is incorrect for the target then this will result in a deserialization error as normal).
struct FieldVisitor<T>(PhantomData<T>);

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
    fn visit_str<E: de::Error>(self, value: &str) -> std::result::Result<Self::Value, E> {
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

    fn visit_bool<E: de::Error>(self, v: bool) -> std::result::Result<Self::Value, E> {
        Deserialize::deserialize(de::value::BoolDeserializer::new(v)).map(|t| Field::Resolved(t))
    }

    // The default impls for i8, i16 and i32 will forward to this
    fn visit_i64<E: de::Error>(self, v: i64) -> std::result::Result<Self::Value, E> {
        Deserialize::deserialize(de::value::I64Deserializer::new(v)).map(|t| Field::Resolved(t))
    }

    // The default impls for u8, u16 and u32 will forward to this
    fn visit_u64<E: de::Error>(self, v: u64) -> std::result::Result<Self::Value, E> {
        Deserialize::deserialize(de::value::U64Deserializer::new(v)).map(|t| Field::Resolved(t))
    }

    // The default impl for f32 will forward to this
    fn visit_f64<E: de::Error>(self, v: f64) -> std::result::Result<Self::Value, E> {
        Deserialize::deserialize(de::value::F64Deserializer::new(v)).map(|t| Field::Resolved(t))
    }
}

// NOTE: We are wrapping this as a newtype to avoid exposing serde_json::Number as part of the
// public API.

/// Represents a number, whether integer or floating point.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Number(serde_json::Number);

impl fmt::Display for Number {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A scalar that is valid to be used as a template value for a [Field].
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum Scalar {
    /// Represents a number, whether integer or floating point.
    Number(Number),
    /// Represents a boolean
    Bool(bool),
    /// Represents a string
    String(String),
}

impl fmt::Display for Scalar {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Number(n) => write!(f, "{n}"),
            Self::Bool(b) => write!(f, "{b}"),
            Self::String(s) => write!(f, "{s}"),
        }
    }
}

/// A [Scalar] type which may appear inside of a templated [Field] within a config file.
pub trait ValidField:
    fmt::Debug + Clone + DeserializeOwned + TryFrom<Scalar, Error = String>
{
}

impl ValidField for bool {}
impl TryFrom<Scalar> for bool {
    type Error = String;

    fn try_from(value: Scalar) -> std::result::Result<Self, Self::Error> {
        match value {
            Scalar::Bool(v) => Ok(v),
            value => Err(format!("invalid value `{value}`, expected bool")),
        }
    }
}

impl ValidField for String {}
impl TryFrom<Scalar> for String {
    type Error = String;

    fn try_from(value: Scalar) -> std::result::Result<Self, Self::Error> {
        match value {
            Scalar::String(v) => Ok(v),
            value => Err(format!("invalid value `{value}`, expected String")),
        }
    }
}

impl ValidField for f64 {}
impl TryFrom<Scalar> for f64 {
    type Error = String;

    fn try_from(value: Scalar) -> std::result::Result<Self, Self::Error> {
        let maybe_float = match &value {
            Scalar::Number(Number(v)) => v.as_f64(),
            _ => None,
        };

        maybe_float.ok_or_else(|| format!("invalid value `{value}`, expected f64"))
    }
}

// We want to handle all signed and unsigned integers in a similar way so we're stamping out the
// impls we need to satisfy ValidField using a macro.
macro_rules! impl_integer_scalars {
    ( $([$($ty:ty),+] => $as_method:ident;)+ ) => {
        $($(
            impl ValidField for $ty {}

            impl From<$ty> for Scalar {
                fn from(value: $ty) -> Self {
                    Scalar::Number(Number(serde_json::Number::from(value)))
                }
            }

            impl TryFrom<Scalar> for $ty {
                type Error = String;

                fn try_from(value: Scalar) -> std::result::Result<Self, Self::Error> {
                    let maybe_t = match &value {
                        Scalar::Number(Number(v)) => v.$as_method().map(|n| n as $ty),
                        _ => None
                    };

                    maybe_t.ok_or_else(|| format!("invalid value `{value}`, expected {}", stringify!($ty)))
                }
            }
        )+)+
    };
}

impl_integer_scalars!(
    [i8, i16, i32, i64, isize] => as_i64;
    [u8, u16, u32, u64, usize] => as_u64;
);

#[cfg(test)]
mod tests {
    use super::*;
    use simple_test_case::test_case;

    // From / TryFrom impls to help with creating test data

    impl From<bool> for Scalar {
        fn from(value: bool) -> Self {
            Scalar::Bool(value)
        }
    }

    impl From<String> for Scalar {
        fn from(value: String) -> Self {
            Scalar::String(value)
        }
    }

    impl TryFrom<f64> for Scalar {
        type Error = &'static str;

        fn try_from(value: f64) -> std::result::Result<Self, &'static str> {
            Ok(Scalar::Number(Number(
                serde_json::Number::from_f64(value)
                    .ok_or("NaN and infinite floats are not supported")?,
            )))
        }
    }

    // Helper macro to create a HashMap<String, Scalar>.
    // Intended to be used for easily creating test values for testing templating
    macro_rules! values_map {
        () => {
            ::std::collections::HashMap::<String, $crate::templating::Scalar>::new()
        };

        ($($k:expr => $v:expr),+) => {{
            let mut m = ::std::collections::HashMap::new();

            $(
                m.insert($k.to_string(), $crate::templating::Scalar::try_from($v).unwrap());
            )+

            m
        }};
    }

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

    // Test data structs for the Template tests below

    #[derive(Debug, PartialEq, Deserialize)]
    struct T {
        foo: Field<bool>,
        bar: U,
    }

    impl Template for T {
        fn has_pending_fields(&self) -> bool {
            self.foo.has_pending_fields() || self.bar.has_pending_fields()
        }

        fn try_resolve(
            &mut self,
            path: &mut Vec<String>,
            values: &HashMap<String, Scalar>,
        ) -> Result<()> {
            let mut errs = ErrorBuilder::new();
            if let Err(e) = self.foo.try_resolve_nested(path, "foo", values) {
                errs.extend(e);
            };
            if let Err(e) = self.bar.try_resolve_nested(path, "bar", values) {
                errs.extend(e);
            };

            errs.into_result(())
        }
    }

    #[derive(Debug, PartialEq, Deserialize)]
    struct U {
        baz: Field<u32>,
    }

    impl Template for U {
        fn has_pending_fields(&self) -> bool {
            self.baz.has_pending_fields()
        }

        fn try_resolve(
            &mut self,
            path: &mut Vec<String>,
            values: &HashMap<String, Scalar>,
        ) -> Result<()> {
            self.baz.try_resolve_nested(path, "baz", values)
        }
    }

    const RESOLVE_NO_PENDING: &str = "
foo: true
bar:
  baz: 17";

    const RESOLVE_ALL_PENDING: &str = r#"
foo: "{{ value_A }}"
bar:
  baz: "{{ value_B }}""#;

    #[test]
    fn resolving_with_nothing_to_template_works() {
        let mut t: T = serde_yaml::from_str(RESOLVE_NO_PENDING).unwrap();
        assert!(!t.has_pending_fields(), "shouldn't have any pending fields");

        let res = t.try_resolve(&mut Vec::new(), &values_map!());

        assert!(res.is_ok(), "expected no errors, got {res:?}");
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
        assert!(t.has_pending_fields(), "should have pending fields");

        let vals = values_map!(
            "value_A" => true,
            "value_B" => 17
        );
        let res = t.try_resolve(&mut Vec::new(), &vals);

        assert!(res.is_ok(), "expected no errors, got {res:?}");
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
        assert!(t.has_pending_fields(), "should have pending fields");

        let vals = values_map!("value_A" => true);
        let res = t.try_resolve_known(&vals);

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
        assert!(t.has_pending_fields(), "should have pending fields");

        let vals = values_map!();
        let res = t.try_resolve_known(&vals);

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
        assert!(t.has_pending_fields(), "should have pending fields");

        let vals = values_map!("value_A" => true, "value_B" => 1.23);
        let res = t.try_resolve_known(&vals);

        assert!(res.is_err(), "expected errors, got {res:?}");

        let err = res.unwrap_err().into_vec().remove(0);
        assert_eq!(
            err,
            Error {
                kind: ErrorKind::InvalidData,
                path: "bar.baz".to_string(),
                message: "invalid value `1.23`, expected u32".to_string()
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
