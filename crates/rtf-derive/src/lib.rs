use darling::{FromDeriveInput, FromField, ast};
use proc_macro::{self, TokenStream};
use quote::quote;
use syn::{Error, Ident, Result, parse_macro_input};

#[proc_macro_derive(Template, attributes(template))]
pub fn derive(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input);

    let (ident, fields) = match InputMeta::from_derive_input(&input) {
        Ok(meta) => match meta.into_parts() {
            Ok(parts) => parts,
            Err(err) => return err.into_compile_error().into(),
        },
        Err(err) => return err.write_errors().into(),
    };

    let has_pending_fields = fields.iter().map(|f| {
        quote! { self.#f.has_pending_fields() }
    });
    let required_values = fields.iter().map(|f| {
        quote! { vals.extend(self.#f.required_values()); }
    });
    let try_template = fields.iter().map(|f| {
        quote! {
            errs.append(self.#f.try_template_nested(path, stringify!(#f), values));
        }
    });

    quote! {
        impl ::rtf_config::templating::Template for #ident {
            fn has_pending_fields(&self) -> ::std::primitive::bool {
                #(#has_pending_fields)||*
            }

            fn required_values(&self) -> ::std::vec::Vec<String> {
                let mut vals = Vec::new();
                #(#required_values)*
                vals
            }

            fn try_template(
                &mut self,
                path: &mut ::std::vec::Vec<::std::string::String>,
                values: &::std::collections::HashMap<
                    ::std::string::String,
                    ::rtf_config::templating::Scalar
                >,
            ) -> ::rtf_config::templating::Result<()> {
                let mut errs = ::rtf_config::templating::ErrorBuilder::new();
                #(#try_template)*
                errs.into_result(())
            }
        }
    }
    .into()
}

#[derive(Debug, FromDeriveInput)]
#[darling(supports(struct_any))]
struct InputMeta {
    ident: Ident,
    data: ast::Data<(), FieldMeta>,
}

impl InputMeta {
    // All fields that weren't marked as skipped
    fn into_parts(self) -> Result<(Ident, Vec<Ident>)> {
        let fields = match self.data {
            ast::Data::Struct(s) => s.fields,
            ast::Data::Enum(_) => {
                return Err(Error::new(self.ident.span(), "expected struct, found enum"));
            }
        };

        let field_idents = fields
            .into_iter()
            .filter(|fm| !fm.skip)
            .flat_map(|fm| fm.ident)
            .collect();

        Ok((self.ident, field_idents))
    }
}

#[derive(Debug, FromField)]
#[darling(attributes(template))]
struct FieldMeta {
    ident: Option<Ident>,
    #[darling(default)]
    skip: bool,
}
