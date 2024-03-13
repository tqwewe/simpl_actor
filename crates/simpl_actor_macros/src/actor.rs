use heck::{ToShoutySnekCase, ToUpperCamelCase};
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

    fn expand_send_error_to_actor_error(
        &self,
        variant: &Ident,
        fields: &Punctuated<Field, Token![,]>,
    ) -> proc_macro2::TokenStream {
        let Self {
            actor_msg_ident, ..
        } = self;

        let field_idents: Punctuated<_, Token![,]> = fields
            .iter()
            .map(|field| field.ident.as_ref().unwrap())
            .collect();

        quote! {
            match err.0 {
                ::simpl_actor::Signal::Message(#actor_msg_ident::#variant {
                    __reply: _,
                    #field_idents
                }) => ::simpl_actor::ActorError::ActorNotRunning((
                    #field_idents
                )),
                _ => unimplemented!(),
            }
        }
    }

    fn expand_try_send_error_to_actor_error(
        &self,
        variant: &Ident,
        fields: &Punctuated<Field, Token![,]>,
    ) -> proc_macro2::TokenStream {
        let Self {
            actor_msg_ident, ..
        } = self;

        let field_idents: Punctuated<_, Token![,]> = fields
            .iter()
            .map(|field| field.ident.as_ref().unwrap())
            .collect();

        quote! {
            match err {
                ::tokio::sync::mpsc::error::TrySendError::Full(
                    ::simpl_actor::Signal::Message(
                        #actor_msg_ident::#variant {
                            __reply: _,
                            #field_idents
                        }
                    )
                ) => ::simpl_actor::ActorError::MailboxFull((
                    #field_idents
                )),
                ::tokio::sync::mpsc::error::TrySendError::Closed(
                    ::simpl_actor::Signal::Message(
                        #actor_msg_ident::#variant {
                            __reply: _,
                            #field_idents
                        }
                    )
                ) => ::simpl_actor::ActorError::ActorNotRunning((
                    #field_idents
                )),
                _ => unimplemented!(),
            }
        }
    }

    fn expand_timeout_send_error_to_actor_error(
        &self,
        variant: &Ident,
        fields: &Punctuated<Field, Token![,]>,
    ) -> proc_macro2::TokenStream {
        let Self {
            actor_msg_ident, ..
        } = self;

        let field_idents: Punctuated<_, Token![,]> = fields
            .iter()
            .map(|field| field.ident.as_ref().unwrap())
            .collect();

        quote! {
            match err {
                ::tokio::sync::mpsc::error::SendTimeoutError::Closed(
                    ::simpl_actor::Signal::Message(
                        #actor_msg_ident::#variant {
                            __reply: _,
                            #field_idents
                        }
                    )
                ) => ::simpl_actor::ActorError::ActorNotRunning((
                    #field_idents
                )),
                ::tokio::sync::mpsc::error::SendTimeoutError::Timeout(
                    ::simpl_actor::Signal::Message(
                        #actor_msg_ident::#variant {
                            __reply: _,
                            #field_idents
                        }
                    )
                ) => ::simpl_actor::ActorError::Timeout((
                    #field_idents
                )),
                _ => unimplemented!(),
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
            pub struct #actor_ref_ident {
                channel: ::tokio::sync::mpsc::Sender<::simpl_actor::Signal<#actor_msg_ident>>,
                stop_notify: ::std::sync::Arc<::tokio::sync::Notify>
            }
        }
    }

    fn expand_actor_ref_impl(&self) -> proc_macro2::TokenStream {
        let Self {
            actor_msg_ident,
            actor_ref_ident,
            messages,
            ..
        } = self;

        let methods = messages.iter().map(
            |Message {
                 sig,
                 variant,
                 fields,
             }| {
                 let field_idents: Vec<_> = fields
                     .iter()
                     .map(|field| field.ident.as_ref().unwrap())
                     .collect();
                let field_tys: Punctuated<_, Token![,]> = fields.iter().map(|field| &field.ty).collect();

                let mut sig = sig.clone();
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
                        parse_quote! { -> ::std::result::Result<(), ::simpl_actor::ActorError<( #field_tys )>> }
                    }
                    ReturnType::Type(_, ty) => {
                        parse_quote! { -> ::std::result::Result<#ty, ::simpl_actor::ActorError<( #field_tys )>> }
                    }
                };

                let mut timeout_sig = sig.clone();
                timeout_sig.ident = format_ident!("{}_timeout", sig.ident);
                timeout_sig.inputs.push(FnArg::Typed(parse_quote! { timeout: ::std::time::Duration }));

                let mut try_sig = sig.clone();
                try_sig.ident = format_ident!("try_{}", sig.ident);

                let mut async_sig = sig.clone();
                async_sig.ident = format_ident!("{}_async", async_sig.ident);
                async_sig.output =
                    parse_quote! { -> ::std::result::Result<(), ::simpl_actor::ActorError<( #field_tys )>> };

                let mut async_timeout_sig = async_sig.clone();
                async_timeout_sig.ident = format_ident!("{}_timeout", async_sig.ident);
                async_timeout_sig.inputs.push(FnArg::Typed(parse_quote! { timeout: ::std::time::Duration }));

                let mut try_async_sig = async_sig.clone();
                try_async_sig.asyncness = None;
                try_async_sig.ident = format_ident!("try_{}", async_sig.ident);
                try_async_sig.output =
                    parse_quote! { -> ::std::result::Result<(), ::simpl_actor::ActorError<( #field_tys )>> };

                let map_err = self.expand_send_error_to_actor_error(variant, fields);
                let timeout_map_err = self.expand_timeout_send_error_to_actor_error(variant, fields);
                let try_map_err = self.expand_try_send_error_to_actor_error(variant, fields);

                let normal_debug_msg = format!(
                    "cannot call non-async messages on self as this would deadlock - please use the {} variant instead\nthis assertion only occurs on debug builds, release builds will deadlock",
                    async_sig.ident
                );
                let timeout_debug_msg = format!(
                    "cannot call non-async messages on self as this would deadlock - please use the {} variant instead\nthis assertion only occurs on debug builds, release builds will deadlock",
                    async_timeout_sig.ident
                );
                let try_debug_msg = format!(
                    "cannot call non-async messages on self as this would deadlock - please use the {} variant instead\nthis assertion only occurs on debug builds, release builds will deadlock",
                    try_async_sig.ident
                );
                quote! {
                    pub #sig {
                        debug_assert!(
                            #actor_ref_global_ident.try_with(|_| {}).is_err(),
                            #normal_debug_msg
                        );
                        let (reply, rx) = ::tokio::sync::oneshot::channel();
                        self
                            .channel
                            .send(::simpl_actor::Signal::Message(#actor_msg_ident::#variant {
                                __reply: ::std::option::Option::Some(reply),
                                #( #field_idents ),*
                            }))
                            .await
                            .map_err(|err| #map_err)?;

                        rx.await.map_err(|_| ::simpl_actor::ActorError::ActorStopped)
                    }

                    pub #timeout_sig {
                        debug_assert!(
                            #actor_ref_global_ident.try_with(|_| {}).is_err(),
                            #timeout_debug_msg
                        );
                        let (reply, rx) = ::tokio::sync::oneshot::channel();
                        self
                            .channel
                            .send_timeout(
                                ::simpl_actor::Signal::Message(#actor_msg_ident::#variant {
                                    __reply: ::std::option::Option::Some(reply),
                                    #( #field_idents ),*
                                }),
                                timeout,
                            )
                            .await
                            .map_err(|err| #timeout_map_err)?;

                        rx.await.map_err(|_| ::simpl_actor::ActorError::ActorStopped)
                    }

                    pub #try_sig {
                        debug_assert!(
                            #actor_ref_global_ident.try_with(|_| {}).is_err(),
                            #try_debug_msg
                        );

                        let (reply, rx) = ::tokio::sync::oneshot::channel();
                        self
                            .channel
                            .try_send(::simpl_actor::Signal::Message(#actor_msg_ident::#variant {
                                __reply: ::std::option::Option::Some(reply),
                                #( #field_idents ),*
                            }))
                            .map_err(|err| #try_map_err)?;

                        rx.await.map_err(|_| ::simpl_actor::ActorError::ActorStopped)
                    }

                    pub #async_sig {
                        self
                            .channel
                            .send(::simpl_actor::Signal::Message(#actor_msg_ident::#variant {
                                __reply: ::std::option::Option::None,
                                #( #field_idents ),*
                            }))
                            .await
                            .map_err(|err| #map_err)?;

                        Ok(())
                    }

                    pub #async_timeout_sig {
                        self
                            .channel
                            .send_timeout(
                                ::simpl_actor::Signal::Message(#actor_msg_ident::#variant {
                                    __reply: ::std::option::Option::None,
                                    #( #field_idents ),*
                                }),
                                timeout,
                            )
                            .await
                            .map_err(|err| #timeout_map_err)?;

                        Ok(())
                    }

                    pub #try_async_sig {
                        self
                            .channel
                            .try_send(::simpl_actor::Signal::Message(#actor_msg_ident::#variant {
                                __reply: ::std::option::Option::None,
                                #( #field_idents ),*
                            }))
                            .map_err(|err| #try_map_err)?;

                        Ok(())
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

    fn expand_actor_ref_impl_actor_ref(&self) -> proc_macro2::TokenStream {
        let Self {
            actor_ref_ident, ..
        } = self;

        quote! {
            #[automatically_derived]
            #[async_trait::async_trait]
            impl ::simpl_actor::ActorRef for #actor_ref_ident {
                async fn stop_gracefully(&self) -> ::std::result::Result<(), ::simpl_actor::ActorError> {
                    self
                        .channel
                        .send(::simpl_actor::Signal::Stop)
                        .await
                        .map_err(|_| {
                            ::simpl_actor::ActorError::ActorNotRunning(())
                        })
                }

                fn stop_immediately(&self) {
                    self.stop_notify.notify_waiters();
                }

                async fn wait_for_stop(&self) {
                    self.channel.closed().await;
                }
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

        let task_local_ident =
            format_ident!("{}_ACTOR_REF", ident.to_string().TO_SHOUTY_SNEK_CASE());

        quote! {
            ::tokio::task_local! {
                #[doc(hidden)]
                static #task_local_ident: #actor_ref_ident;
            }

            #[automatically_derived]
            impl ::simpl_actor::Spawn for #ident {
                type Ref = #actor_ref_ident;

                fn spawn(self) -> Self::Ref {
                    fn handle_messages<'a>(
                        actor: &'a mut #ident,
                        rx: &'a mut ::tokio::sync::mpsc::Receiver<::simpl_actor::Signal<#actor_msg_ident>>,
                    ) -> ::core::pin::Pin<::std::boxed::Box<dyn ::core::future::Future<Output = ()> + ::core::marker::Send + 'a>> {
                        ::std::boxed::Box::pin(async move {
                            while let ::std::option::Option::Some(signal) = rx.recv().await {
                                match signal {
                                    ::simpl_actor::Signal::Message(msg) => match msg {
                                        #( #handlers )*
                                    }
                                    ::simpl_actor::Signal::Stop => {
                                        return;
                                    }
                                }
                            }
                        })
                    }

                    let (tx, rx) =
                        ::tokio::sync::mpsc::channel(<Self as ::simpl_actor::Actor>::channel_size());
                    let stop_notify = ::std::sync::Arc::new(::tokio::sync::Notify::new());
                    let actor_ref = #actor_ref_ident {
                        channel: tx,
                        stop_notify: ::std::sync::Arc::clone(&stop_notify)
                    };

                    ::tokio::spawn(#task_local_ident.scope(actor_ref.clone(), async move {
                        ::simpl_actor::run_actor_lifecycle::<Self, #actor_msg_ident, _>(
                            self,
                            rx,
                            stop_notify,
                            handle_messages
                        ).await
                    }));

                    actor_ref
                }

                fn actor_ref() -> Option<Self::Ref> {
                    #task_local_ident.try_with(|actor_ref| actor_ref.clone()).ok()
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
        let actor_ref_impl_actor_ref = self.expand_actor_ref_impl_actor_ref();
        let spawn_impl = self.expand_spawn_impl();

        tokens.extend(quote! {
            #item_impl

            #msg_enum
            #actor_ref_struct
            #actor_ref_impl
            #actor_ref_impl_actor_ref
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
