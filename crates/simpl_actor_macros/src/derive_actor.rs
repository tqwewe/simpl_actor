use quote::{format_ident, quote, ToTokens};
use syn::{
    parse::{Parse, ParseStream},
    DeriveInput, Ident,
};

pub struct DeriveActor {
    ident: Ident,
}

impl ToTokens for DeriveActor {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        let ident = &self.ident;
        let actor_ref_ident = format_ident!("{ident}Ref");
        let name = ident.to_string();

        tokens.extend(quote! {
            #[automatically_derived]
            impl ::simpl_actor::Actor for #ident {
                type Ref = #actor_ref_ident;

                fn name(&self) -> ::std::borrow::Cow<'_, str> {
                    ::std::borrow::Cow::Borrowed(#name)
                }
            }
        });
    }
}

impl Parse for DeriveActor {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let input: DeriveInput = input.parse()?;
        let ident = input.ident;

        Ok(DeriveActor { ident })
    }
}
