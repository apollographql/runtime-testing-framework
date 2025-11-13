use darling::{
    FromDeriveInput, FromField, FromVariant,
    ast::{self, Fields},
};
use proc_macro2::{Span, TokenStream};
use quote::quote;
use syn::{Ident, Result, parse_macro_input};

#[proc_macro_derive(Template, attributes(template))]
pub fn derive(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let input = parse_macro_input!(input);

    let (ident, has_pending_fields, required_variables, try_template) =
        match InputMeta::from_derive_input(&input) {
            Ok(meta) => match meta.into_token_streams() {
                Ok(parts) => parts,
                Err(err) => return err.into_compile_error().into(),
            },
            Err(err) => return err.write_errors().into(),
        };

    quote! {
        impl ::rtf_config::templating::Template for #ident {
            fn has_pending_fields(&self) -> ::std::primitive::bool {
                #has_pending_fields
            }

            fn required_variables(&self) -> ::std::vec::Vec<String> {
                #required_variables
            }

            fn try_template(
                &mut self,
                path: &mut ::std::vec::Vec<::std::string::String>,
                source: &::rtf_config::Source,
                variables: &::rtf_config::templating::TemplateVariables,
            ) -> ::rtf_config::templating::Result<()> {
                #try_template
            }
        }
    }
    .into()
}

#[derive(Debug, FromDeriveInput)]
struct InputMeta {
    ident: Ident,
    data: ast::Data<EnumMeta, FieldMeta>,
}

impl InputMeta {
    // All fields that weren't marked as skipped
    fn into_token_streams(self) -> Result<(Ident, TokenStream, TokenStream, TokenStream)> {
        let (has_pending_fields, required_variables, try_template) = match self.data {
            ast::Data::Struct(s) => struct_token_streams(s.fields),

            ast::Data::Enum(v) if v.is_empty() => {
                return Err(syn::Error::new(
                    self.ident.span(),
                    "derive Template not supported for empty enums",
                ));
            }

            ast::Data::Enum(v) => enum_token_streams(self.ident.span(), v)?,
        };

        Ok((
            self.ident,
            has_pending_fields,
            required_variables,
            try_template,
        ))
    }
}

fn struct_token_streams(field_meta: Vec<FieldMeta>) -> (TokenStream, TokenStream, TokenStream) {
    let fields: Vec<Ident> = field_meta
        .into_iter()
        .filter(|fm| !fm.skip)
        .flat_map(|fm| fm.ident)
        .collect();

    // We have valid structs that need to implement Template but do not have any templatable Fields
    // For this, we need to special case structs with "no fields" (in reality these structs have
    // fields, they will all have been marked as skipped)
    if fields.is_empty() {
        return (
            quote! {false},
            quote! {::std::vec::Vec::new()},
            quote! {Ok(())},
        );
    }

    let inner = fields.iter().map(|f| {
        quote! { self.#f.has_pending_fields() }
    });
    let has_pending_fields = quote! {
        #(#inner)||*
    };

    let inner = fields.iter().map(|f| {
        quote! { vals.extend(self.#f.required_variables()); }
    });
    let required_variables = quote! {
        let mut vals = Vec::new();
        #(#inner)*
        vals
    };

    let inner = fields.iter().map(|f| {
        quote! {
            errs.append(self.#f.try_template_nested(path, stringify!(#f), source, variables));
        }
    });
    let try_template = quote! {
        let mut errs = ::rtf_config::templating::ErrorBuilder::new();
        #(#inner)*
        errs.into_result(())
    };

    (has_pending_fields, required_variables, try_template)
}

fn enum_token_streams(
    span: Span,
    enum_meta: Vec<EnumMeta>,
) -> Result<(TokenStream, TokenStream, TokenStream)> {
    let variants: Vec<Ident> = enum_meta
        .into_iter()
        .filter(|v| !v.skip && !v.fields.is_empty())
        .map(|v| v.ident)
        .collect();

    if variants.is_empty() {
        return Err(syn::Error::new(
            span,
            "At least one templatable enum variant must exist",
        ));
    }

    let has_pending_fields = quote! {
        match self {
            #(Self::#variants(inner) => inner.has_pending_fields(),)*
            _ => false,
        }
    };

    let required_variables = quote! {
        match self {
            #(Self::#variants(inner) => inner.required_variables(),)*
            _ => Vec::new(),
        }
    };

    let try_template = quote! {
        match self {
            #(Self::#variants(inner) => inner.try_template(path, source, variables),)*
            _ => Ok(()),
        }
    };

    Ok((has_pending_fields, required_variables, try_template))
}

#[derive(Debug, FromField)]
#[darling(attributes(template))]
struct FieldMeta {
    ident: Option<Ident>,
    #[darling(default)]
    skip: bool,
}

#[derive(Debug, FromVariant)]
#[darling(attributes(template))]
#[darling(supports(newtype, unit))]
struct EnumMeta {
    ident: Ident,
    fields: Fields<FieldMeta>,
    #[darling(default)]
    skip: bool,
}
