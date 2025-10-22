//! Helpers for supporting minimal templating of user config files.
use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{
    Deserialize, Deserializer, Serialize, Serializer,
    de::{self, DeserializeOwned, Visitor},
};
use std::{borrow::Cow, collections::HashMap, fmt, marker::PhantomData};

/// User facing descriptions of the reason that templating a [Field] failed.
///
/// Paired with an additional message to form an [Error].
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::Display, strum::EnumString)]
pub enum ErrorKind {
    #[strum(to_string = "Conflicting value and matrix definitions")]
    ConflictingValues,

    #[strum(to_string = "Empty array for matrix value")]
    EmptyMatrixValue,

    #[strum(to_string = "Inconsistent types for matrix value")]
    InconsistentMatrixValue,

    #[strum(to_string = "Invalid templating value")]
    InvalidData,

    #[strum(to_string = "Missing template values")]
    MissingValues,

    #[strum(to_string = "Unknown templating value")]
    UnknownValue,
}

impl crate::error::ErrorKind for ErrorKind {
    const HEADER: &str = "Templating failed";
}

// Type aliases for checks error handling.
// Elsewhere in the codebase we should always refer to these aliases rather than parameterising the
// generic types from the error module.

pub type Error = crate::error::Error<ErrorKind>;
pub type Errors = crate::error::Errors<ErrorKind>;
pub type ErrorBuilder = crate::error::ErrorBuilder<ErrorKind>;
pub type Result<T> = std::result::Result<T, Errors>;

/// In order to support controlled templating of config files with [Scalar] values we make use of a
/// wrapper [Field] type to identify where values need to be injected. A type that implements
/// [Template] supports walking its contents to locate and template fields using a provided map
/// of scalar values.
pub trait Template {
    /// Whether or not there are any pending [Field]s contained within this value.
    fn has_pending_fields(&self) -> bool;

    /// The list of template values that are required to template this type fully.
    fn required_values(&self) -> Vec<String>;

    /// Attempt to resolve all pending [Field]s, appending encountered errors to the `errs` vec
    /// provided.
    fn try_template(
        &mut self,
        path: &mut Vec<String>,
        values: &HashMap<String, Scalar>,
    ) -> Result<()>;

    /// Attempt to resolve all pending [Field]s when this type is a child of some parent
    /// [Template], appending encountered errors to the `errs` vec provided. The provided `tail`
    /// will be appended to `path` before calling through to [Template::try_template].
    fn try_template_nested(
        &mut self,
        path: &mut Vec<String>,
        tail: &str,
        values: &HashMap<String, Scalar>,
    ) -> Result<()> {
        let mut path = path.clone();
        path.push(tail.to_string());
        self.try_template(&mut path, values)
    }

    /// Attempt to resolve all known [Field]s, reporting required values that are not present in
    /// the provided map. If there are any deserialization errors then then this method as an
    /// aggregate operation will fail.
    fn try_template_known(&mut self, values: &HashMap<String, Scalar>) -> Result<Vec<String>> {
        let all_errs = match self.try_template(&mut Vec::new(), values) {
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

impl<T> Template for Option<T>
where
    T: Template,
{
    fn has_pending_fields(&self) -> bool {
        self.as_ref()
            .map(|inner| inner.has_pending_fields())
            .unwrap_or_default()
    }

    fn required_values(&self) -> Vec<String> {
        self.as_ref()
            .map(|inner| inner.required_values())
            .unwrap_or_default()
    }

    fn try_template(
        &mut self,
        path: &mut Vec<String>,
        values: &HashMap<String, Scalar>,
    ) -> Result<()> {
        self.as_mut()
            .map(|inner| inner.try_template(path, values))
            .unwrap_or(Ok(()))
    }
}

impl<T> Template for Vec<T>
where
    T: Template,
{
    fn has_pending_fields(&self) -> bool {
        self.iter().any(|elem| elem.has_pending_fields())
    }

    fn required_values(&self) -> Vec<String> {
        self.iter()
            .flat_map(|elem| elem.required_values())
            .collect()
    }

    fn try_template(
        &mut self,
        path: &mut Vec<String>,
        values: &HashMap<String, Scalar>,
    ) -> Result<()> {
        let mut errs = ErrorBuilder::new();

        for elem in self.iter_mut() {
            errs.append(elem.try_template(path, values))
        }

        errs.into_result(())
    }
}

impl<K, T> Template for HashMap<K, T>
where
    K: AsRef<str>,
    T: Template,
{
    fn has_pending_fields(&self) -> bool {
        self.values().any(|elem| elem.has_pending_fields())
    }

    fn required_values(&self) -> Vec<String> {
        self.values()
            .flat_map(|elem| elem.required_values())
            .collect()
    }

    fn try_template(
        &mut self,
        path: &mut Vec<String>,
        values: &HashMap<String, Scalar>,
    ) -> Result<()> {
        let mut errs = ErrorBuilder::new();

        for (name, f) in self.iter_mut() {
            errs.append(f.try_template_nested(path, name.as_ref(), values));
        }

        errs.into_result(())
    }
}

/// A [Field] wraps some scalar type that implements [Template] in order to mark it as
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

impl<T> JsonSchema for Field<T>
where
    T: JsonSchema + ValidField,
{
    fn schema_name() -> Cow<'static, str> {
        let t_type = T::json_schema(&mut SchemaGenerator::default())
            .to_value()
            .get("type")
            .cloned()
            .unwrap_or(serde_json::Value::String("Scalar".into()));

        format!("Templatable {}", t_type.as_str().unwrap()).into()
    }

    fn schema_id() -> Cow<'static, str> {
        format!("Field<{}>", T::schema_id()).into()
    }

    fn inline_schema() -> bool {
        false
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        let t_type = T::json_schema(generator)
            .to_value()
            .get("type")
            .cloned()
            .unwrap_or(serde_json::Value::String("Scalar".into()));

        json_schema!({
          "description": format!(
              "A templatable {} that can be replaced with a user specified value at runtime",
              t_type.as_str().unwrap()
          ),
          "oneOf": [
            {
              "description": "The value that should be templated.",
              "type": "string",
              "pattern": r#"^\{\{ \w+ \}\}$"#
            },
            {
              "description": "Statically provided data.",
              "type": t_type
            }
          ]
        })
    }
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

impl<T> Default for Field<T>
where
    T: Default + ValidField,
{
    fn default() -> Self {
        Field::Resolved(T::default())
    }
}

impl<T> Template for Field<T>
where
    T: ValidField,
{
    fn has_pending_fields(&self) -> bool {
        matches!(self, Self::Pending(_))
    }

    fn required_values(&self) -> Vec<String> {
        match self {
            Self::Pending(field_name) => vec![field_name.clone()],
            _ => Vec::new(),
        }
    }

    fn try_template(
        &mut self,
        path: &mut Vec<String>,
        values: &HashMap<String, Scalar>,
    ) -> Result<()> {
        if let Self::Pending(value) = self {
            match values.get(value) {
                Some(raw) => match T::try_from_scalar(raw.clone()) {
                    Ok(t) => *self = Self::Resolved(t),
                    Err(reason) => {
                        return Err(Errors::new(ErrorKind::InvalidData, reason, path));
                    }
                },
                None => return Err(Errors::new(ErrorKind::UnknownValue, value.clone(), path)),
            }
        }

        Ok(())
    }
}

impl<T: ValidField> Serialize for Field<T> {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::Pending(s) => serializer.serialize_str(&format!("{{{{ {s} }}}}")),
            Self::Resolved(t) => t.serialize(serializer),
        }
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

impl<T> Visitor<'_> for FieldVisitor<T>
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

            None if value.starts_with("{{") => {
                return Err(E::custom(
                    "malformed template string: expected a single space after '{{'",
                ));
            }

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

/// # Number
///
/// Represents a number, whether integer or floating point.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema)]
pub struct Number(serde_json::Number);

impl fmt::Display for Number {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// # Scalar
///
/// A scalar that is valid to be used as a template value for a [Field].
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(untagged, expecting = "expecting a valid Number, Boolean or String")]
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

impl From<&str> for Scalar {
    fn from(value: &str) -> Self {
        Scalar::String(value.to_string())
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

/// A [Scalar] type which may appear inside of a templated [Field] within a config file.
pub trait ValidField: fmt::Debug + Clone + Serialize + DeserializeOwned {
    /// We need this try_from operation to always return a String error on failure so we can report
    /// invalid types being used in Test Plans to the user. This doesn't work for a `Field<Scalar>`
    /// as Scalar::try_from(Scalar) has an error type of Infallible which can't be constructed (and
    /// therefore can't be turned into a string).
    fn try_from_scalar(s: Scalar) -> std::result::Result<Self, String>;
}

impl ValidField for Scalar {
    fn try_from_scalar(s: Scalar) -> std::result::Result<Self, String> {
        Ok(s)
    }
}

impl ValidField for bool {
    fn try_from_scalar(s: Scalar) -> std::result::Result<Self, String> {
        Self::try_from(s)
    }
}

impl TryFrom<Scalar> for bool {
    type Error = String;

    fn try_from(value: Scalar) -> std::result::Result<Self, Self::Error> {
        match value {
            Scalar::Bool(v) => Ok(v),
            value => Err(format!("invalid value `{value}`, expected bool")),
        }
    }
}

impl ValidField for String {
    fn try_from_scalar(s: Scalar) -> std::result::Result<Self, String> {
        Self::try_from(s)
    }
}

impl TryFrom<Scalar> for String {
    type Error = String;

    fn try_from(value: Scalar) -> std::result::Result<Self, Self::Error> {
        match value {
            Scalar::String(v) => Ok(v),
            value => Err(format!("invalid value `{value}`, expected String")),
        }
    }
}

impl ValidField for f64 {
    fn try_from_scalar(s: Scalar) -> std::result::Result<Self, String> {
        Self::try_from(s)
    }
}

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
            impl ValidField for $ty {
                fn try_from_scalar(s: Scalar) -> std::result::Result<Self, String> {
                    Self::try_from(s)
                }
            }

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
    use rtf_derive::Template;
    use simple_test_case::test_case;

    #[derive(Debug, PartialEq, Deserialize)]
    #[serde(
        untagged,
        expecting = "expecting a Field that can resolve into a usize, isize, f64, bool or String"
    )]
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
    fn field_parse_works(s: &str, expected: Target) {
        let target: Target = serde_yaml::from_str(s).unwrap();
        assert_eq!(target, expected)
    }

    #[derive(Debug, PartialEq, Deserialize)]
    struct StringField {
        field: Field<String>,
    }

    #[test_case("foo"; "ascii")]
    #[test_case("BAR"; "upper case")]
    #[test_case("世界"; "unicode")]
    #[test_case("baz1"; "non leading digit")]
    #[test_case("foo_bar"; "internal underscore")]
    #[test]
    fn field_parse_valid_identifiers(raw: &str) {
        let s = format!("field: \"{{{{ {raw} }}}}\"");
        let res: serde_yaml::Result<StringField> = serde_yaml::from_str(&s);
        assert!(res.is_ok(), "expected ok, got {res:?}");

        let StringField { field } = res.unwrap();
        assert_eq!(field, Field::Pending(raw.to_string()));
    }

    #[test_case("_foo"; "leading underscore")]
    #[test_case("1foo"; "leading digit")]
    #[test_case("foo!bar"; "contains punctuation")]
    #[test_case("foo bar"; "contains whitespace")]
    #[test_case(""; "no value name")]
    #[test_case("🦊"; "emoji")]
    #[test]
    fn field_parse_invalid_identifiers(raw: &str) {
        let s = format!("field: \"{{{{ {raw} }}}}\"");
        let res: serde_yaml::Result<StringField> = serde_yaml::from_str(&s);
        assert!(res.is_err(), "expected error, got {res:?}");
    }

    #[test_case(r#""{{foo }}""#; "no space before value name")]
    #[test_case(r#""{{ foo}}""#; "no space after value name")]
    #[test_case(r#""{{foo}}""#; "no spaces before or after value name")]
    #[test_case(r#""{{ foo""#; "unclosed template")]
    #[test_case(r#""{{ foo }""#; "single closing curly")]
    #[test_case(r#""{{  foo }}""#; "additional leading space")]
    #[test_case(r#""{{ \tfoo }}""#; "leading tab")]
    #[test_case(r#""{{ foo  }}""#; "additional trailing space")]
    #[test_case(r#""{{ foo\t }}""#; "trailing tab")]
    #[test]
    fn field_parse_malformed_template_string(raw: &str) {
        let res: serde_yaml::Result<StringField> = serde_yaml::from_str(&format!("field: {raw}"));
        assert!(res.is_err(), "expected error, got {res:?}");
    }

    #[test]
    fn field_parse_single_leading_curly_permitted() {
        let raw = r#"field: "{ \"some\": { \"valid\": [\"json\", \"data\"] }}""#;
        let res: serde_yaml::Result<StringField> = serde_yaml::from_str(raw);

        assert!(res.is_ok(), "expected ok, got {res:?}");
        assert!(matches!(res.unwrap().field, Field::Resolved(_)));
    }

    // Helper functions for constructing pending and resolved fields
    fn p<T: ValidField>(s: &str) -> Field<T> {
        Field::Pending(s.to_string())
    }
    fn r<T: ValidField>(t: impl Into<T>) -> Field<T> {
        Field::Resolved(t.into())
    }

    // Test struct for Option<Field<T>>
    #[derive(Debug, Template)]
    struct OptionField {
        foo: Option<Field<String>>,
    }
    fn of(foo: Option<Field<String>>) -> Box<OptionField> {
        Box::new(OptionField { foo })
    }

    // Test struct for Vec<Field<T>>
    #[derive(Debug, Template)]
    struct VecField {
        foo: Vec<Field<String>>,
    }
    fn vf(foo: &[Field<String>]) -> Box<VecField> {
        Box::new(VecField { foo: foo.to_vec() })
    }

    // Test struct for HashMap<K, Field<T>>
    #[derive(Debug, Template)]
    struct HashMapField {
        foo: HashMap<String, Field<String>>,
    }
    macro_rules! field_map {
        ($slice:expr) => {{
            let mut m = ::std::collections::HashMap::new();
            for field in $slice {
                match field {
                    Field::Pending(key) => {
                        m.insert(key.clone(), field.clone());
                    }
                    Field::Resolved(value) => {
                        m.insert(value.clone(), field.clone());
                    }
                }
            }
            m
        }};
    }
    fn hmf(map: &[Field<String>]) -> Box<HashMapField> {
        let foo = field_map!(map);
        Box::new(HashMapField { foo })
    }

    #[test_case(of(Some(p("foo"))), true; "optional field is some pending")]
    #[test_case(of(Some(r("foo"))), false; "optional field is some resolved")]
    #[test_case(of(None), false; "optional field is none is resolved")]
    #[test_case(vf(&[p("foo"), p("bar"), p("baz")]), true; "vec all entries are pending")]
    #[test_case(vf(&[p("foo"), r("bar"), r("baz")]), true; "vec one entry is pending")]
    #[test_case(vf(&[p("foo")]), true; "vec single entry is pending")]
    #[test_case(vf(&[r("foo"), r("bar"), r("baz")]), false; "vec all entries are resolved")]
    #[test_case(vf(&[r("foo")]), false; "vec single entry is resolved")]
    #[test_case(vf(&[]), false; "vec no entries is resolved")]
    #[test_case(hmf(&[p("foo"), p("bar"), p("baz")]), true; "hash map multiple entries pending")]
    #[test_case(hmf(&[p("foo"), r("bar"), r("baz")]), true; "hash map single entry pending")]
    #[test_case(hmf(&[p("foo")]), true; "hash map one entry pending")]
    #[test_case(hmf(&[r("foo"), r("bar"), r("baz")]), false; "hash map all entries resolved")]
    #[test_case(hmf(&[r("foo")]), false; "hash map single entry resolved")]
    #[test_case(hmf(&[]), false; "hash map no entries resolved")]
    #[test]
    fn template_has_pending_fields(t: Box<dyn Template>, expected: bool) {
        let res = t.has_pending_fields();
        assert_eq!(
            res, expected,
            "tests that has_pending_fields has expected value"
        )
    }

    #[test_case(of(Some(p("foo"))), &["foo"]; "some optional field is required")]
    #[test_case(of(Some(r("foo"))), &[]; "some optional field is not required")]
    #[test_case(of(None), &[]; "none optional field is not required")]
    #[test_case(vf(&[p("foo"), p("bar"), p("baz")]), &["bar", "baz", "foo"]; "vec all entries are required")]
    #[test_case(vf(&[p("foo"), r("bar"), r("baz")]), &["foo"]; "vec one entry is required")]
    #[test_case(vf(&[p("foo")]), &["foo"]; "vec single entry is required")]
    #[test_case(vf(&[r("foo"), r("bar"), r("baz")]), &[]; "vec no entries are required")]
    #[test_case(vf(&[r("foo")]), &[]; "vec single entry is not required")]
    #[test_case(vf(&[]), &[]; "vec no entries not required")]
    #[test_case(hmf(&[p("foo"), p("bar"), p("baz")]), &["bar", "baz", "foo"]; "hash map multiple entries are required")]
    #[test_case(hmf(&[p("foo"), r("bar"), r("baz")]), &["foo"]; "hash map single entry is required")]
    #[test_case(hmf(&[p("foo")]), &["foo"]; "hash map one entry is required")]
    #[test_case(hmf(&[r("foo"), r("bar"), r("baz")]), &[]; "hash map no entries are required")]
    #[test_case(hmf(&[r("foo")]), &[]; "hash map single entry is not required")]
    #[test_case(hmf(&[]), &[]; "hash map no entries not required")]
    #[test]
    fn template_required_values(t: Box<dyn Template>, expected: &[&str]) {
        let mut res = t.required_values();
        res.sort(); // Sorting so values are in a determistic order for the assert_eq

        assert_eq!(
            res.as_slice(),
            expected,
            "expected required values to be {expected:?}, got {res:?}"
        )
    }

    macro_rules! values_map {
        ($slice:expr) => {{
            let mut m = ::std::collections::HashMap::new();
            for k in $slice {
                m.insert(k.to_string(), Scalar::from(k.to_string()));
            }
            m
        }};
    }

    #[test_case(of(Some(p("foo"))), &["foo"]; "some optional field templates")]
    #[test_case(of(None), &[]; "none optional field templates")]
    #[test_case(vf(&[p("foo"), p("bar"), p("baz")]), &["foo", "bar", "baz"]; "multiple vec entries template")]
    #[test_case(vf(&[p("foo")]), &["foo"]; "single vec entry templates")]
    #[test_case(vf(&[]), &[]; "no vec entries templates")]
    #[test_case(hmf(&[p("foo"), p("bar"), p("baz")]), &["foo", "bar", "baz"]; "multiple hash map entries template")]
    #[test_case(hmf(&[p("foo")]), &["foo"]; "single hash map entry templates")]
    #[test_case(hmf(&[]), &[]; "no hash map entries templates")]
    #[test]
    fn template_try_template_success(mut t: Box<dyn Template>, values: &[&str]) {
        let values = values_map!(values);
        let res = t.try_template(&mut Vec::new(), &values);
        assert!(
            res.is_ok(),
            "expected to template successfully, got {res:?}"
        )
    }

    #[test_case(of(Some(p("foo"))), &["foo"]; "some optional field")]
    #[test_case(vf(&[p("foo"), p("bar"), p("baz")]), &["bar", "baz", "foo"]; "multiple vec entries")]
    #[test_case(vf(&[p("foo")]), &["foo"]; "single vec entry")]
    #[test_case(hmf(&[p("foo"), p("bar"), p("baz")]), &["bar", "baz", "foo"]; "multiple hash map entries")]
    #[test_case(hmf(&[p("foo")]), &["foo"]; "single hash map entry")]
    #[test]
    fn template_try_template_unknown_value_error(
        mut t: Box<dyn Template>,
        expected_err_messages: &[&str],
    ) {
        let values = values_map!(["unused"]);

        let res = t.try_template(&mut Vec::new(), &values);
        assert!(res.is_err(), "expected templating to fail, got {res:?}");
        let errors = res.unwrap_err();
        assert!(
            errors
                .iter()
                .all(|e| matches!(e.kind, ErrorKind::UnknownValue)),
            "expected all errors to be UnknownValue, got {:?}",
            errors
        );
        let mut messages: Vec<&str> = errors.iter().map(|e| e.message.as_str()).collect();
        messages.sort();
        assert_eq!(messages.as_slice(), expected_err_messages);
    }
}
