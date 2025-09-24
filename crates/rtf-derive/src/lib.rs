use proc_macro::{self, TokenStream};
use quote::quote;
use syn::{DeriveInput, parse_macro_input};

#[proc_macro_derive(Template, attributes(template))]
pub fn derive(input: TokenStream) -> TokenStream {
    let DeriveInput { ident, .. } = parse_macro_input!(input);

    let output = quote! {
        impl ::rtf_config::templating::Template for #ident {
            fn has_pending_fields(&self) -> ::std::primitive::bool {
                false
            }

            fn required_values(&self) -> ::std::vec::Vec<String> {
                vec![]
            }

            fn try_template(
                &mut self,
                path: &mut ::std::vec::Vec<::std::string::String>,
                values: &::std::collections::HashMap<
                    ::std::string::String,
                    ::rtf_config::templating::Scalar
                >,
            ) -> ::rtf_config::templating::Result<()> {
                ::rtf_config::templating::Result::Ok(())
            }
        }
    };

    output.into()
}
