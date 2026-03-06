//! Helpers for supporting minimal templating of user config files.
use crate::{
    VariableDefinition,
    formats::CustomProviderDefinition,
    providers::file::{CustomProviderSection, StableSource},
};
use regex::Regex;
use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{
    Deserialize, Deserializer, Serialize, Serializer,
    de::{self, DeserializeOwned, Visitor},
};
use std::{
    borrow::{Borrow, Cow},
    collections::{HashMap, HashSet},
    fmt,
    hash::Hash,
    marker::PhantomData,
    sync::{Arc, LazyLock},
};

/// User facing descriptions of the reason that templating a [Field] failed.
///
/// Paired with an additional message to form an [Error].
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::Display, strum::EnumString)]
pub enum ErrorKind {
    #[strum(to_string = "Conflicting variable and matrix definitions")]
    ConflictingVariables,

    #[strum(to_string = "Default value for variable not in its allowed values")]
    DefaultNotInAllowedValues,

    #[strum(to_string = "Empty array for variable allowed values")]
    EmptyAllowedValues,

    #[strum(to_string = "Empty array for matrix variable")]
    EmptyMatrixVariable,

    #[strum(to_string = "Incompatible allowed values across variable definitions")]
    IncompatibleAllowedValues,

    #[strum(to_string = "Inconsistent types for matrix include maps")]
    InconsistentMatrixInclude,

    #[strum(to_string = "Inconsistent types for matrix variable")]
    InconsistentMatrixVariable,

    #[strum(to_string = "Invalid templating variable")]
    InvalidData,

    #[strum(
        to_string = "Missing template variables definition. Make sure the variable is defined in the scenario or environment config variable definitions"
    )]
    MissingVariable,

    #[strum(
        to_string = "Missing custom provider definition. Make sure the provider is declared in the config file that is using it."
    )]
    MissingCustomProvider,

    #[strum(to_string = "No matching conditional cases for provided variables")]
    NoMatchingCases,

    #[strum(
        to_string = "Unknown templating variable. Make sure a value is defined for this variable to resolve to."
    )]
    UnknownVariable,

    #[strum(to_string = "Variable value not in allowed values")]
    ValueNotAllowed,
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

/// A context for the [Template] trait.
///
/// This allows for tracking the provenance of where each variable has come from in order to
/// correctly handle relative paths.
#[derive(Debug, Clone)]
pub struct TemplateContext {
    variables: HashMap<String, Scalar>,
    variable_sources: HashMap<String, StableSource>,
    resolve_for: FileType,
    custom_provider_definitions: Arc<CustomProviderDefinitions>,
    variable_definitions: Vec<VariableDefinition>,
}

impl TemplateContext {
    pub fn new(
        variables: HashMap<String, Scalar>,
        variable_sources: HashMap<String, StableSource>,
        custom_provider_definitions: Arc<CustomProviderDefinitions>,
    ) -> Self {
        Self {
            variables,
            variable_sources,
            resolve_for: FileType::Environment,
            custom_provider_definitions,
            variable_definitions: Vec::new(),
        }
    }

    #[cfg(test)]
    pub(crate) fn new_stubbed(variables: HashMap<String, Scalar>) -> Self {
        Self::new(variables, Default::default(), Default::default())
    }

    pub fn variables(&self) -> &HashMap<String, Scalar> {
        &self.variables
    }

    /// Helper for filtering allowed templating variables based on [VariableDefinition]s present in a
    /// config file. This is also where defaults defined in variable definitions are applied, being
    /// overwritten by any explicitly provided variables coming from `all_variables`.
    pub(crate) fn for_config_file<'a>(
        &self,
        file_source: &StableSource,
        resolve_for: Option<FileType>,
        variable_definitions: impl Iterator<Item = &'a VariableDefinition>,
    ) -> Self {
        let mut new = self.clone();
        if let Some(resolve_for) = resolve_for {
            new.resolve_for = resolve_for;
        }

        new.variable_definitions = variable_definitions.cloned().collect();
        new.variables
            .retain(|k, _| new.variable_definitions.iter().any(|val| &val.name == k));

        for vd in new.variable_definitions.iter() {
            if let Some(default) = vd.default.as_ref() {
                new.variables.entry(vd.name.clone()).or_insert_with(|| {
                    new.variable_sources
                        .insert(vd.name.clone(), file_source.clone());

                    default.clone()
                });
            }
        }

        new
    }

    pub fn extend(&mut self, source: StableSource, variables: HashMap<String, Scalar>) {
        for k in variables.keys() {
            self.variable_sources.insert(k.to_owned(), source.clone());
        }

        self.variables.extend(variables);
    }

    pub fn extend_with_sources(
        &mut self,
        sources: HashMap<String, StableSource>,
        variables: HashMap<String, Scalar>,
    ) {
        self.variable_sources.extend(sources);
        self.variables.extend(variables);
    }

    pub fn get<Q>(&self, key: &Q) -> Option<&Scalar>
    where
        String: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        self.variables.get(key)
    }

    pub fn get_with_source<Q>(&self, key: &Q) -> Option<(&StableSource, &Scalar)>
    where
        String: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        let s = self.variables.get(key)?;
        let source = self
            .variable_sources
            .get(key)
            .unwrap_or(&StableSource::TestPlan);

        Some((source, s))
    }

    pub fn variable_definition(&self, key: impl AsRef<str>) -> Option<&VariableDefinition> {
        self.variable_definitions
            .iter()
            .find(|vd| vd.name == key.as_ref())
    }

    pub fn custom_provider_definition(
        &self,
        key: &str,
    ) -> Option<(StableSource, &CustomProviderDefinition)> {
        self.custom_provider_definitions.get(key, self.resolve_for)
    }
}

#[derive(Debug, Default, Copy, Clone, PartialEq, Deserialize, Serialize)]
pub enum FileType {
    #[default]
    Environment,
    Scenario,
}

#[derive(Debug, Default, Clone, PartialEq, Deserialize, Serialize)]
pub struct CustomProviderDefinitions {
    pub(crate) test_plan: HashMap<String, CustomProviderDefinition>,
    pub(crate) scenario: HashMap<String, CustomProviderDefinition>,
    pub(crate) environment: HashMap<String, CustomProviderDefinition>,
}

impl CustomProviderDefinitions {
    fn get(
        &self,
        k: &str,
        resolve_for: FileType,
    ) -> Option<(StableSource, &CustomProviderDefinition)> {
        let opt = match resolve_for {
            FileType::Scenario => self
                .scenario
                .get(k)
                .map(|def| (CustomProviderSection::Scenario.as_stable_source(k), def)),
            FileType::Environment => self
                .environment
                .get(k)
                .map(|def| (CustomProviderSection::Environment.as_stable_source(k), def)),
        };

        opt.or_else(|| {
            self.test_plan
                .get(k)
                .map(|def| (CustomProviderSection::TestPlan.as_stable_source(k), def))
        })
    }
}

/// In order to support controlled templating of config files with [Scalar] variables we make use of a
/// wrapper [Field] type to identify where variables need to be injected. A type that implements
/// [Template] supports walking its contents to locate and template fields using a provided map
/// of scalar variables.
pub trait Template {
    /// The list of template variables that are required to template this type fully.
    fn required_variables(&self) -> Vec<String>;

    /// Check that the provided [TemplateContext] is sufficient for templating all fields and
    /// custom providers under this type.
    fn validate_context(
        &self,
        path: &mut Vec<String>,
        allowed_variables: &HashSet<&String>,
        file_source: &StableSource,
        ctx: &TemplateContext,
    ) -> Result<()>;

    fn validate_context_nested(
        &self,
        path: &mut Vec<String>,
        tail: &str,
        allowed_variables: &HashSet<&String>,
        file_source: &StableSource,
        ctx: &TemplateContext,
    ) -> Result<()> {
        let mut path = path.clone();
        path.push(tail.to_string());
        self.validate_context(&mut path, allowed_variables, file_source, ctx)
    }

    /// Attempt to resolve all pending [Field]s, appending encountered errors to the `errs` vec
    /// provided.
    fn try_template(
        &mut self,
        path: &mut Vec<String>,
        file_source: &StableSource,
        ctx: &TemplateContext,
    ) -> Result<()>;

    /// Attempt to resolve all pending [Field]s when this type is a child of some parent
    /// [Template], appending encountered errors to the `errs` vec provided. The provided `tail`
    /// will be appended to `path` before calling through to [Template::try_template].
    fn try_template_nested(
        &mut self,
        path: &mut Vec<String>,
        tail: &str,
        file_source: &StableSource,
        ctx: &TemplateContext,
    ) -> Result<()> {
        let mut path = path.clone();
        path.push(tail.to_string());
        self.try_template(&mut path, file_source, ctx)
    }

    /// Attempt to resolve all known [Field]s, reporting required variables that are not present in
    /// the provided map. If there are any deserialization errors then then this method as an
    /// aggregate operation will fail.
    fn try_template_known(
        &mut self,
        path: &mut Vec<String>,
        file_source: &StableSource,
        ctx: &TemplateContext,
    ) -> Result<Vec<String>> {
        let all_errs = match self.try_template(path, file_source, ctx) {
            Ok(_) => return Ok(Vec::new()),
            Err(errs) => errs,
        };

        let mut errs = ErrorBuilder::new();
        let mut missing = Vec::new();

        for err in all_errs.into_iter() {
            match err.kind {
                ErrorKind::UnknownVariable => missing.push(err.message),
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
    fn required_variables(&self) -> Vec<String> {
        self.as_ref()
            .map(|inner| inner.required_variables())
            .unwrap_or_default()
    }

    fn validate_context(
        &self,
        path: &mut Vec<String>,
        allowed_variables: &HashSet<&String>,
        file_source: &StableSource,
        ctx: &TemplateContext,
    ) -> Result<()> {
        self.as_ref()
            .map(|inner| inner.validate_context(path, allowed_variables, file_source, ctx))
            .unwrap_or(Ok(()))
    }

    fn try_template(
        &mut self,
        path: &mut Vec<String>,
        file_source: &StableSource,
        ctx: &TemplateContext,
    ) -> Result<()> {
        self.as_mut()
            .map(|inner| inner.try_template(path, file_source, ctx))
            .unwrap_or(Ok(()))
    }
}

impl<T> Template for Vec<T>
where
    T: Template,
{
    fn required_variables(&self) -> Vec<String> {
        self.iter()
            .flat_map(|elem| elem.required_variables())
            .collect()
    }

    fn validate_context(
        &self,
        path: &mut Vec<String>,
        allowed_variables: &HashSet<&String>,
        file_source: &StableSource,
        ctx: &TemplateContext,
    ) -> Result<()> {
        let mut errs = ErrorBuilder::new();

        for elem in self.iter() {
            errs.append(elem.validate_context(path, allowed_variables, file_source, ctx));
        }

        errs.into_result(())
    }

    fn try_template(
        &mut self,
        path: &mut Vec<String>,
        file_source: &StableSource,
        ctx: &TemplateContext,
    ) -> Result<()> {
        let mut errs = ErrorBuilder::new();

        for elem in self.iter_mut() {
            errs.append(elem.try_template(path, file_source, ctx))
        }

        errs.into_result(())
    }
}

impl<K, T> Template for HashMap<K, T>
where
    K: AsRef<str>,
    T: Template,
{
    fn required_variables(&self) -> Vec<String> {
        self.values()
            .flat_map(|elem| elem.required_variables())
            .collect()
    }

    fn validate_context(
        &self,
        path: &mut Vec<String>,
        allowed_variables: &HashSet<&String>,
        file_source: &StableSource,
        ctx: &TemplateContext,
    ) -> Result<()> {
        let mut errs = ErrorBuilder::new();

        for (name, f) in self.iter() {
            errs.append(f.validate_context_nested(
                path,
                name.as_ref(),
                allowed_variables,
                file_source,
                ctx,
            ));
        }

        errs.into_result(())
    }

    fn try_template(
        &mut self,
        path: &mut Vec<String>,
        file_source: &StableSource,
        ctx: &TemplateContext,
    ) -> Result<()> {
        let mut errs = ErrorBuilder::new();

        for (name, f) in self.iter_mut() {
            errs.append(f.try_template_nested(path, name.as_ref(), file_source, ctx));
        }

        errs.into_result(())
    }
}

/// A [Field] wraps some scalar type that implements [Template] in order to mark it as
/// requiring a templated variable coming from user provided variables as part of resolving the config
/// file.
///
/// Fields must be resolved in order to be usable during a test run.
#[derive(Debug, Clone, PartialEq)]
pub enum Field<T>
where
    T: ValidField,
{
    /// A pending field that should be replaced with the named variable when it is available.
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
              "A templatable {} that can be replaced with a user specified variable at runtime",
              t_type.as_str().unwrap()
          ),
          "oneOf": [
            {
              "description": "The variable that should be templated.",
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
    pub(crate) fn as_resolved(&self) -> &T {
        match self {
            Self::Pending(_) => panic!("field is still pending"),
            Self::Resolved(t) => t,
        }
    }

    pub(crate) fn into_resolved(self) -> T {
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
    fn required_variables(&self) -> Vec<String> {
        match self {
            Self::Pending(var) => vec![var.clone()],
            Self::Resolved(_) => Vec::new(),
        }
    }

    fn validate_context(
        &self,
        path: &mut Vec<String>,
        allowed_variables: &HashSet<&String>,
        _file_source: &StableSource,
        ctx: &TemplateContext,
    ) -> Result<()> {
        match self {
            Self::Pending(var) if !allowed_variables.contains(var) => {
                Err(Errors::new(ErrorKind::UnknownVariable, var, path))
            }

            Self::Pending(var) if ctx.get(var).is_none() => Err(Errors::new(
                ErrorKind::MissingVariable,
                format!(
                    "  - {var}: {:?}",
                    ctx.variable_definition(var)
                        .map(|vd| vd.description.as_str())
                        .unwrap_or_default()
                ),
                path,
            )),

            _ => Ok(()),
        }
    }

    fn try_template(
        &mut self,
        path: &mut Vec<String>,
        _file_source: &StableSource,
        ctx: &TemplateContext,
    ) -> Result<()> {
        if let Self::Pending(variable) = self {
            match ctx.get(variable) {
                Some(raw) => {
                    // Check allowed_values constraint before resolving.
                    // This is a panic because check_templating_will_work should have caught this.
                    if let Some(vd) = ctx.variable_definition(variable.as_str())
                        && let Some(allowed) = &vd.allowed_values
                    {
                        assert!(
                            allowed.contains(raw),
                            "variable '{variable}' has value '{raw}' not in allowed values {allowed:?}. \
                             This should have been caught by check_templating_will_work(). \
                             Path: {}",
                            path.join(".")
                        );
                    }

                    match T::try_from_scalar(raw.clone()) {
                        Ok(t) => *self = Self::Resolved(t),
                        Err(reason) => {
                            return Err(Errors::new(ErrorKind::InvalidData, reason, path));
                        }
                    }
                }
                None => {
                    return Err(Errors::new(
                        ErrorKind::UnknownVariable,
                        variable.clone(),
                        path,
                    ));
                }
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

/// A custom serde [Visitor] for locating template strings of the form `"{{ some_variable }}"` and
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
    /// If we do then we extract the variable name from it an return a Pending, otherwise we defer to
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
                "expected variable identifier with a single space either side",
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
/// A scalar that is valid to be used as a template variable for a [Field].
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
    fn from(variable: bool) -> Self {
        Scalar::Bool(variable)
    }
}

impl From<String> for Scalar {
    fn from(variable: String) -> Self {
        Scalar::String(variable)
    }
}

impl From<&str> for Scalar {
    fn from(variable: &str) -> Self {
        Scalar::String(variable.to_string())
    }
}

impl TryFrom<f64> for Scalar {
    type Error = &'static str;

    fn try_from(variable: f64) -> std::result::Result<Self, &'static str> {
        Ok(Scalar::Number(Number(
            serde_json::Number::from_f64(variable)
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

    fn try_from(variable: Scalar) -> std::result::Result<Self, Self::Error> {
        match variable {
            Scalar::Bool(v) => Ok(v),
            variable => Err(format!("invalid variable `{variable}`, expected bool")),
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

    fn try_from(variable: Scalar) -> std::result::Result<Self, Self::Error> {
        match variable {
            Scalar::String(v) => Ok(v),
            variable => Err(format!("invalid variable `{variable}`, expected String")),
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

    fn try_from(variable: Scalar) -> std::result::Result<Self, Self::Error> {
        let maybe_float = match &variable {
            Scalar::Number(Number(v)) => v.as_f64(),
            _ => None,
        };

        maybe_float.ok_or_else(|| format!("invalid variable `{variable}`, expected f64"))
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
                fn from(variable: $ty) -> Self {
                    Scalar::Number(Number(serde_json::Number::from(variable)))
                }
            }

            impl TryFrom<Scalar> for $ty {
                type Error = String;

                fn try_from(variable: Scalar) -> std::result::Result<Self, Self::Error> {
                    let maybe_t = match &variable {
                        Scalar::Number(Number(v)) => v.$as_method().map(|n| n as $ty),
                        _ => None
                    };

                    maybe_t.ok_or_else(|| format!("invalid variable `{variable}`, expected {}", stringify!($ty)))
                }
            }
        )+)+
    };
}

impl_integer_scalars!(
    [i8, i16, i32, i64, isize] => as_i64;
    [u8, u16, u32, u64, usize] => as_u64;
);

/// Regex matching `${variable_name}` interpolation patterns used in templated file content.
pub static RE_TEMPLATE_VAR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"\$\{(.*?)\}"#).expect("valid regex"));

/// Extract all `${variable}` names from `content` in the order they appear.
pub fn extract_template_vars(content: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    RE_TEMPLATE_VAR
        .captures_iter(content)
        .map(|cap| {
            let (_, [val]) = cap.extract();
            val.to_string()
        })
        .filter(|val| seen.insert(val.clone()))
        .collect()
}

/// Substitute `${k}` patterns in `content` with values from `variables`.
///
/// Returns `Ok(interpolated)` when all variables are resolved, or
/// `Err(unresolved)` with the names of any remaining unknown variables.
pub fn interpolate_variables(
    content: &str,
    variables: &HashMap<String, Scalar>,
) -> std::result::Result<String, Vec<String>> {
    let mut result = content.to_string();

    for (k, v) in variables.iter() {
        result = result.replace(&format!("${{{k}}}"), &v.to_string());
    }

    let unresolved = extract_template_vars(&result);

    if unresolved.is_empty() {
        Ok(result)
    } else {
        Err(unresolved)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rtf_derive::Template;
    use simple_test_case::test_case;

    #[test]
    fn template_variables_for_config_file_defaults_used_correctly() {
        let all_variables: HashMap<String, Scalar> = [
            ("a".into(), 1.into()),
            ("b".into(), "foo".into()),
            ("c".into(), true.into()),
        ]
        .into_iter()
        .collect();

        let definitions = [
            VariableDefinition {
                name: "a".into(),
                description: String::new(),
                default: Some(2.into()),
                allowed_values: None,
            },
            VariableDefinition {
                name: "b".into(),
                description: String::new(),
                default: None,
                allowed_values: None,
            },
            VariableDefinition {
                name: "d".into(),
                description: String::new(),
                default: Some("bar".into()),
                allowed_values: None,
            },
        ];

        let original = TemplateContext::new_stubbed(all_variables);
        let for_config_file =
            original.for_config_file(&StableSource::TestPlan, None, definitions.iter());

        // a has an explicit variable so it overrides the default
        // b has an explicit variable and no default
        // c is not in the definitions so it is filtered out
        // d has no explicit variable so we take the default
        let expected: HashMap<String, Scalar> = [
            ("a".into(), 1.into()),
            ("b".into(), "foo".into()),
            ("d".into(), "bar".into()),
        ]
        .into_iter()
        .collect();

        assert_eq!(for_config_file.variables(), &expected);
    }

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
    #[test_case(""; "no variable name")]
    #[test_case("🦊"; "emoji")]
    #[test]
    fn field_parse_invalid_identifiers(raw: &str) {
        let s = format!("field: \"{{{{ {raw} }}}}\"");
        let res: serde_yaml::Result<StringField> = serde_yaml::from_str(&s);
        assert!(res.is_err(), "expected error, got {res:?}");
    }

    #[test_case(r#""{{foo }}""#; "no space before variable name")]
    #[test_case(r#""{{ foo}}""#; "no space after variable name")]
    #[test_case(r#""{{foo}}""#; "no spaces before or after variable name")]
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
                    Field::Pending(variable) => {
                        m.insert(variable.clone(), field.clone());
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
    fn template_required_variables(t: Box<dyn Template>, expected: &[&str]) {
        let mut res = t.required_variables();
        res.sort(); // Sorting so variables are in a deterministic order for the assert_eq

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

            TemplateContext::new_stubbed(m)
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
    fn template_try_template_success(mut t: Box<dyn Template>, variable: &[&str]) {
        let template_ctx = template_context!(variable);
        let res = t.try_template(&mut Vec::new(), &StableSource::TestPlan, &template_ctx);
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
    fn template_try_template_unknown_variable_error(
        mut t: Box<dyn Template>,
        expected_err_messages: &[&str],
    ) {
        let template_ctx = template_context!(["unused"]);

        let res = t.try_template(&mut Vec::new(), &StableSource::TestPlan, &template_ctx);
        assert!(res.is_err(), "expected templating to fail, got {res:?}");
        let errors = res.unwrap_err();
        assert!(
            errors
                .iter()
                .all(|e| matches!(e.kind, ErrorKind::UnknownVariable)),
            "expected all errors to be UnknownVariable, got {:?}",
            errors
        );
        let mut messages: Vec<&str> = errors.iter().map(|e| e.message.as_str()).collect();
        messages.sort();
        assert_eq!(messages.as_slice(), expected_err_messages);
    }

    // Helper to create a TemplateContext with variable_definitions
    fn ctx_with_variable_definitions(
        variables: HashMap<String, Scalar>,
        variable_definitions: Vec<crate::VariableDefinition>,
    ) -> TemplateContext {
        let base = TemplateContext::new_stubbed(variables);
        base.for_config_file(&StableSource::TestPlan, None, variable_definitions.iter())
    }

    fn allowed(vals: &[&str]) -> Option<Vec<Scalar>> {
        Some(vals.iter().map(|s| (*s).into()).collect())
    }

    #[test_case(
        Field::Pending("foo".into()),
        "any_value",
        None;
        "no allowed values"
    )]
    #[test_case(
        Field::Pending("foo".into()),
        "a",
        allowed(&["a", "b"]);
        "value in allowed values"
    )]
    #[test]
    fn field_try_template_allowed_values_passes(
        mut field: Field<String>,
        value: &str,
        allowed_values: Option<Vec<Scalar>>,
    ) {
        let vd = crate::VariableDefinition {
            name: "foo".into(),
            description: String::new(),
            default: None,
            allowed_values,
        };
        let ctx = ctx_with_variable_definitions(
            [("foo".into(), value.into())].into_iter().collect(),
            vec![vd],
        );

        let res = field.try_template(&mut Vec::new(), &StableSource::TestPlan, &ctx);
        assert!(res.is_ok(), "expected ok, got {res:?}");
        assert_eq!(field, Field::Resolved(value.to_string()));
    }

    #[test]
    #[should_panic(expected = "variable 'foo' has value 'c' not in allowed values")]
    fn field_try_template_value_not_in_allowed_values_panics() {
        // This scenario should be caught by check_templating_will_work() before reaching
        // try_template. If it gets here, it's a bug - hence the panic.
        let mut field: Field<String> = Field::Pending("foo".to_string());
        let vd = crate::VariableDefinition {
            name: "foo".into(),
            description: String::new(),
            default: None,
            allowed_values: allowed(&["a", "b"]),
        };
        let ctx = ctx_with_variable_definitions(
            [("foo".into(), "c".into())].into_iter().collect(),
            vec![vd],
        );

        let _ = field.try_template(&mut Vec::new(), &StableSource::TestPlan, &ctx);
    }

    #[test]
    fn field_try_template_resolved_field_bypasses_allowed_values_check() {
        // A resolved field should not check allowed_values since it's already resolved
        let mut field: Field<String> = Field::Resolved("any_value".to_string());
        let vd = crate::VariableDefinition {
            name: "foo".into(),
            description: String::new(),
            default: None,
            allowed_values: allowed(&["a", "b"]),
        };
        let ctx = ctx_with_variable_definitions(
            [("foo".into(), "unused".into())].into_iter().collect(),
            vec![vd],
        );

        let res = field.try_template(&mut Vec::new(), &StableSource::TestPlan, &ctx);
        assert!(res.is_ok(), "expected ok, got {res:?}");
        // Field should remain unchanged
        assert_eq!(field, Field::Resolved("any_value".to_string()));
    }

    fn variables(pairs: &[(&str, &str)]) -> HashMap<String, Scalar> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), Scalar::String(v.to_string())))
            .collect()
    }

    #[test_case("hello ${name}!", &[("name", "world")], "hello world!"; "known variable substituted")]
    #[test_case("${a} and ${b}", &[("a", "foo"), ("b", "bar")], "foo and bar"; "multiple variables substituted")]
    #[test_case("plain string", &[], "plain string"; "no patterns passes through")]
    #[test]
    fn interpolate_variables_success(content: &str, var_pairs: &[(&str, &str)], expected: &str) {
        let vars = variables(var_pairs);
        assert_eq!(
            interpolate_variables(content, &vars),
            Ok(expected.to_string())
        );
    }

    #[test_case("${known} and ${unknown}", &[("known", "value")], &["unknown"]; "unknown variables reported")]
    #[test_case("${x} and ${x}", &[], &["x"]; "duplicate unknown variable deduplicated")]
    #[test]
    fn interpolate_variables_errors(
        content: &str,
        var_pairs: &[(&str, &str)],
        expected_unresolved: &[&str],
    ) {
        let vars = variables(var_pairs);
        let expected: Vec<String> = expected_unresolved.iter().map(|s| s.to_string()).collect();
        assert_eq!(interpolate_variables(content, &vars), Err(expected));
    }
}
