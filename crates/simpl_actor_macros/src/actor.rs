use heck::ToUpperCamelCase;
use proc_macro2::Span;
use quote::{format_ident, quote, ToTokens};
use syn::{
    parse::{Parse, ParseStream},
    parse_quote,
    punctuated::Punctuated,
    spanned::Spanned,
    Field, FnArg, Ident, ImplItem, ItemImpl, Meta, ReturnType, Signature, Token, Type,
};

pub struct Actor {
    item_impl: ItemImpl,
    ident: Ident,
    actor_msg_ident: Ident,
    actor_ref_ident: Ident,
    messages: Vec<Message>,
}

#[derive(Clone)]
struct Message {
    sig: Signature,
    variant: Ident,
    fields: Punctuated<Field, Token![,]>,
}

impl From<Signature> for Message {
    fn from(sig: Signature) -> Self {
        let variant = format_ident!("{}", sig.ident.to_string().to_upper_camel_case());
        let fields = sig
            .inputs
            .iter()
            .filter_map(|input| match input {
                FnArg::Receiver(_) => None,
                FnArg::Typed(pat_type) => Some(pat_type),
            })
            .enumerate()
            .map::<Field, _>(|(i, pat_type)| {
                let ident = match pat_type.pat.as_ref() {
                    syn::Pat::Ident(pat_ident) => pat_ident.ident.clone(),
                    _ => format_ident!("__field{i}"),
                };
                let ty = &pat_type.ty;

                parse_quote! {
                    #ident: #ty
                }
            })
            .collect();

        Message {
            sig,
            variant,
            fields,
        }
    }
}

impl Actor {
    fn extract_messages(item_impl: &mut ItemImpl) -> Vec<Message> {
        item_impl
            .items
            .iter_mut()
            .filter_map(|item| match item {
                ImplItem::Fn(impl_item_fn) => {
                    let mut has_message = false;
                    impl_item_fn.attrs.retain(|attr| {
                        if has_message {
                            return true;
                        }
                        match &attr.meta {
                            Meta::Path(path) if path.segments.len() == 1 => {
                                if path.segments.first().unwrap().ident.to_string() == "message" {
                                    has_message = true;
                                    false
                                } else {
                                    true
                                }
                            }
                            _ => true,
                        }
                    });

                    if has_message {
                        Some(Message::from(impl_item_fn.sig.clone()))
                    } else {
                        None
                    }
                }
                _ => None,
            })
            .collect()
    }

    fn expand_msg_enum(&self) -> proc_macro2::TokenStream {
        let Self {
            actor_msg_ident,
            messages,
            ..
        } = self;

        let variants = messages.iter().map(
            |Message {
                 sig,
                 variant,
                 fields,
             }| {
                let ret_ty: Type = match &sig.output {
                    ReturnType::Default => parse_quote! { () },
                    ReturnType::Type(_, ty) => parse_quote! { #ty },
                };

                quote! {
                    #variant {
                        __reply: ::std::option::Option<::tokio::sync::oneshot::Sender<#ret_ty>>,
                        #fields
                    }
                }
            },
        );

        quote! {
            enum #actor_msg_ident {
                #( #variants ),*
            }
        }
    }

    fn expand_actor_ref_struct(&self) -> proc_macro2::TokenStream {
        let Self {
            actor_msg_ident,
            actor_ref_ident,
            ..
        } = self;

        quote! {
            #[derive(Clone, Debug)]
            pub struct #actor_ref_ident(::tokio::sync::mpsc::Sender<#actor_msg_ident>);
        }
    }

    fn expand_actor_ref_impl(&self) -> proc_macro2::TokenStream {
        let Self {
            actor_msg_ident,
            actor_ref_ident,
            messages,
            ..
        } = self;

        let methods = messages.iter().cloned().map(
            |Message {
                 mut sig,
                 variant,
                 fields,
             }| {
                sig.constness = None;
                sig.asyncness = Some(Token![async](Span::call_site()));
                sig.inputs = [parse_quote! { &self }]
                    .into_iter()
                    .chain(
                        fields
                            .iter()
                            .map(|field| FnArg::Typed(parse_quote! { #field })),
                    )
                    .collect();
                sig.output = match sig.output {
                    ReturnType::Default => {
                        parse_quote! { -> ::std::result::Result<(), ::tokio::sync::oneshot::error::RecvError> }
                    },
                    ReturnType::Type(_, ty) => {
                        parse_quote! { -> ::std::result::Result<#ty, ::tokio::sync::oneshot::error::RecvError> }
                    },
                };

                let field_idents = fields.iter().map(|field| field.ident.as_ref().unwrap());

                quote! {
                    pub #sig {
                        let (reply, rx) = ::tokio::sync::oneshot::channel();
                        let _ = self
                            .0
                            .send(#actor_msg_ident::#variant {
                                __reply: ::std::option::Option::Some(reply),
                                #( #field_idents ),*
                            })
                            .await;

                        rx.await
                    }
                }
            },
        );

        quote! {
            #[automatically_derived]
            impl #actor_ref_ident {
                #( #methods )*
            }
        }
    }

    fn expand_spawn_impl(&self) -> proc_macro2::TokenStream {
        let Self {
            ident,
            actor_msg_ident,
            actor_ref_ident,
            messages,
            ..
        } = self;

        let handlers = messages.iter().map(
            |Message {
                 sig,
                 variant,
                 fields,
             }| {
                let fn_ident = &sig.ident;
                let dot_await = if sig.asyncness.is_some() {
                    quote! { .await }
                } else {
                    quote! {}
                };
                let field_idents: Vec<_> = fields
                    .iter()
                    .map(|field| field.ident.as_ref().unwrap())
                    .collect();

                quote! {
                    #actor_msg_ident::#variant {
                        __reply,
                        #( #field_idents ),*
                    } => {
                        let res = actor.#fn_ident( #( #field_idents ),* ) #dot_await;
                        if let ::std::option::Option::Some(reply) = __reply {
                            let _ = reply.send(res);
                        }
                    }
                }
            },
        );

        quote! {
            #[automatically_derived]
            impl ::simpl_actor::Spawn for #ident {
                type Ref = #actor_ref_ident;

                fn spawn(self) -> Self::Ref {
                    fn handle_messages<'a>(
                        actor: &'a mut #ident,
                        rx: &'a mut ::tokio::sync::mpsc::Receiver<#actor_msg_ident>,
                    ) -> ::core::pin::Pin<::std::boxed::Box<dyn ::core::future::Future<Output = ()> + ::core::marker::Send + 'a>> {
                        ::std::boxed::Box::pin(async move {
                            while let ::std::option::Option::Some(msg) = rx.recv().await {
                                match msg {
                                    #( #handlers )*
                                }
                            }
                        })
                    }

                    let tx = ::simpl_actor::spawn_actor::<
                        Self,
                        #actor_msg_ident,
                        _,
                    >(self, <Self as ::simpl_actor::Actor>::channel_size(), handle_messages);
                    #actor_ref_ident(tx)
                }
            }
        }
    }
}

impl ToTokens for Actor {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        let item_impl = &self.item_impl;
        let msg_enum = self.expand_msg_enum();
        let actor_ref_struct = self.expand_actor_ref_struct();
        let actor_ref_impl = self.expand_actor_ref_impl();
        let spawn_impl = self.expand_spawn_impl();

        tokens.extend(quote! {
            #item_impl

            #msg_enum
            #actor_ref_struct
            #actor_ref_impl
            #spawn_impl
        });
    }
}

impl Parse for Actor {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut item_impl: ItemImpl = input.parse()?;

        let ident = match item_impl.self_ty.as_ref() {
            Type::Path(type_path) => type_path
                .path
                .segments
                .last()
                .as_ref()
                .ok_or_else(|| syn::Error::new(type_path.path.span(), "missing ident from path"))?
                .ident
                .clone(),
            _ => {
                return Err(syn::Error::new(
                    item_impl.self_ty.span(),
                    "expected a path or ident",
                ))
            }
        };
        let actor_msg_ident = format_ident!("{ident}Msg");
        let actor_ref_ident = format_ident!("{ident}Ref");
        let messages = Actor::extract_messages(&mut item_impl);

        Ok(Actor {
            item_impl,
            ident,
            actor_msg_ident,
            actor_ref_ident,
            messages,
        })
    }
}
