use darling::{FromDeriveInput, FromField, FromVariant, ast};
use proc_macro2::TokenStream;
use quote::quote;
use syn::{Ident, Result, parse_macro_input};

#[proc_macro_derive(Template, attributes(template))]
pub fn derive(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let input = parse_macro_input!(input);

    let (ident, has_pending_fields, required_values, try_template) =
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

            fn required_values(&self) -> ::std::vec::Vec<String> {
                #required_values
            }

            fn try_template(
                &mut self,
                path: &mut ::std::vec::Vec<::std::string::String>,
                values: &::std::collections::HashMap<
                    ::std::string::String,
                    ::rtf_config::templating::Scalar
                >,
            ) -> ::rtf_config::templating::Result<()> {
                #try_template
            }
        }
    }
    .into()
}

#[derive(Debug, FromDeriveInput)]
#[darling(supports(struct_any, enum_newtype))]
struct InputMeta {
    ident: Ident,
    data: ast::Data<EnumMeta, FieldMeta>,
}

impl InputMeta {
    // All fields that weren't marked as skipped
    fn into_token_streams(self) -> Result<(Ident, TokenStream, TokenStream, TokenStream)> {
        let (has_pending_fields, required_values, try_template) = match self.data {
            ast::Data::Struct(s) => struct_token_streams(s.fields),
            ast::Data::Enum(v) => enum_token_streams(v),
        };

        Ok((
            self.ident,
            has_pending_fields,
            required_values,
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

    let inner = fields.iter().map(|f| {
        quote! { self.#f.has_pending_fields() }
    });
    let has_pending_fields = quote! {
        #(#inner)||*
    };

    let inner = fields.iter().map(|f| {
        quote! { vals.extend(self.#f.required_values()); }
    });
    let required_values = quote! {
        let mut vals = Vec::new();
        #(#inner)*
        vals
    };

    let inner = fields.iter().map(|f| {
        quote! {
            errs.append(self.#f.try_template_nested(path, stringify!(#f), values));
        }
    });
    let try_template = quote! {
        let mut errs = ::rtf_config::templating::ErrorBuilder::new();
        #(#inner)*
        errs.into_result(())
    };

    (has_pending_fields, required_values, try_template)
}

fn enum_token_streams(enum_meta: Vec<EnumMeta>) -> (TokenStream, TokenStream, TokenStream) {
    let variants: Vec<Ident> = enum_meta.into_iter().map(|v| v.ident).collect();

    let has_pending_fields = quote! {
        match self {
            #(Self::#variants(inner) => inner.has_pending_fields(),)*
        }
    };

    let required_values = quote! {
        match self {
            #(Self::#variants(inner) => inner.required_values(),)*
        }
    };

    let try_template = quote! {
        match self {
            #(Self::#variants(inner) => inner.try_template_nested(path, stringify!(#variants), values),)*
        }
    };

    (has_pending_fields, required_values, try_template)
}

#[derive(Debug, FromField)]
#[darling(attributes(template))]
struct FieldMeta {
    ident: Option<Ident>,
    #[darling(default)]
    skip: bool,
}

#[derive(Debug, FromVariant)]
struct EnumMeta {
    ident: Ident,
}
