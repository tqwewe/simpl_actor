mod actor;
mod derive_actor;

use actor::Actor;
use derive_actor::DeriveActor;
use proc_macro::TokenStream;
use quote::ToTokens;
use syn::parse_macro_input;

#[proc_macro_attribute]
pub fn actor(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let actor = parse_macro_input!(item as Actor);
    TokenStream::from(actor.into_token_stream())
}

#[proc_macro_derive(Actor)]
pub fn derive_actor(input: TokenStream) -> TokenStream {
    let derive_actor = parse_macro_input!(input as DeriveActor);
    TokenStream::from(derive_actor.into_token_stream())
}
