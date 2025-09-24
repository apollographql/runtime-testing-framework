use proc_macro::{self, TokenStream};
use quote::quote;
use syn::{DeriveInput, parse_macro_input};

#[proc_macro_derive(IterFields)]
pub fn derive(input: TokenStream) -> TokenStream {
    let DeriveInput { ident, .. } = parse_macro_input!(input);

    let output = quote! {
        impl ::rtf_config::templating::IterFields for #ident {
            fn iter_fields(&self) -> impl ::std::iter::Iterator<Item = &::rtf_config::templating::TypedField> {
                todo!("implement me")
            }

            fn iter_fields_mut(&mut self) -> impl ::std::iter::Iterator<Item = &mut ::rtf_config::templating::TypedField> {
                todo!("implement me")
            }
        }
    };

    output.into()
}
