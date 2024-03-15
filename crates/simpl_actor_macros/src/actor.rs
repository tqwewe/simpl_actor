use std::borrow::Cow;

use heck::{ToShoutySnekCase, ToUpperCamelCase};
use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote, ToTokens};
use syn::{
    parse::{Parse, ParseStream},
    parse_quote,
    punctuated::Punctuated,
    spanned::Spanned,
    Field, FnArg, GenericArgument, GenericParam, Generics, Ident, ImplItem, ItemImpl, Lifetime,
    LifetimeParam, Meta, ReturnType, Signature, Token, Type, Visibility,
};

pub struct Actor {
    item_impl: ItemImpl,
    ident: Ident,
    actor_generics: Generics,
    actor_msg_ident: Ident,
    actor_msg_generics: Option<TokenStream>,
    actor_ref_ident: Ident,
    actor_ref_global_ident: Ident,
    messages: Vec<Message>,
}

#[derive(Clone)]
struct Message {
    vis: Visibility,
    sig: Signature,
    variant: Ident,
    fields: Punctuated<Field, Token![,]>,
    generics: Generics,
}

impl Message {
    fn has_lifetime(&self) -> bool {
        self.generics.lifetimes().count() > 0
    }
}

impl From<(Visibility, Signature)> for Message {
    fn from((vis, sig): (Visibility, Signature)) -> Self {
        let variant = format_ident!("{}", sig.ident.to_string().to_upper_camel_case());
        let fields: Punctuated<Field, Token![,]> = sig
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
                let ty = match pat_type.ty.as_ref() {
                    Type::Reference(ty_ref) => {
                        let mut ty_ref = ty_ref.clone();
                        ty_ref.lifetime = Some(Lifetime::new(
                            &format!("'{}__{}", sig.ident, ident),
                            Span::call_site(),
                        ));
                        Cow::Owned(Type::Reference(ty_ref))
                    }
                    ty => Cow::Borrowed(ty),
                };

                parse_quote! {
                    #ident: #ty
                }
            })
            .collect();
        let generics_punctuated: Punctuated<_, Token![,]> = fields
            .iter()
            .filter_map(|field| match &field.ty {
                Type::Reference(ty_ref) => ty_ref
                    .lifetime
                    .clone()
                    .map(|lifetime| GenericParam::Lifetime(LifetimeParam::new(lifetime))),
                _ => None,
            })
            .chain(
                sig.generics
                    .params
                    .iter()
                    .filter(|param| matches!(param, GenericParam::Type(_)))
                    .cloned(),
            )
            .collect();
        let generics = if generics_punctuated.is_empty() {
            Generics::default()
        } else {
            parse_quote! { < #generics_punctuated > }
        };

        Message {
            vis,
            sig,
            variant,
            fields,
            generics,
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
                        Some(Message::from((
                            impl_item_fn.vis.clone(),
                            impl_item_fn.sig.clone(),
                        )))
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
            actor_generics,
            actor_msg_ident,
            messages,
            ..
        } = self;

        let variants = messages.iter().map(
            |Message {
                 sig,
                 variant,
                 fields,
                 ..
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
            enum #actor_msg_ident #actor_generics {
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
                _ => ::std::unimplemented!(),
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
                _ => ::std::unimplemented!(),
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
                _ => ::std::unimplemented!(),
            }
        }
    }

    fn expand_actor_ref_struct(&self) -> proc_macro2::TokenStream {
        let Self {
            actor_msg_ident,
            actor_msg_generics,
            actor_ref_ident,
            ..
        } = self;

        quote! {
            #[derive(Clone, Debug)]
            pub struct #actor_ref_ident {
                id: u64,
                mailbox: ::tokio::sync::mpsc::Sender<::simpl_actor::Signal<#actor_msg_ident #actor_msg_generics>>,
                stop_notify: ::std::sync::Arc<::tokio::sync::Notify>,
                links: ::std::sync::Arc<::tokio::sync::Mutex<::std::collections::HashMap<u64, ::simpl_actor::GenericActorRef>>>,
            }
        }
    }

    fn expand_actor_ref_impl(&self) -> proc_macro2::TokenStream {
        let Self {
            actor_msg_ident,
            actor_ref_ident,
            actor_ref_global_ident,
            messages,
            ..
        } = self;

        let methods = messages.iter().enumerate().map(
            |msg @ (i, Message {
                 vis,
                 sig,
                 variant,
                 fields,
                 generics,
             })| {
                 let field_idents: Vec<_> = fields
                     .iter()
                     .map(|field| field.ident.as_ref().unwrap())
                     .collect();
                let field_tys: Punctuated<_, Token![,]> = fields.iter().map(|field| &field.ty).collect();

                let mailbox = {
                    let generic_params: Punctuated<_, Token![,]> = messages.iter().enumerate().flat_map(|(j, Message { generics, .. })| {
                        generics
                            .lifetimes()
                            .map(move |lt| {
                                if i == j {
                                    GenericArgument::Lifetime(lt.lifetime.clone())
                                } else {
                                    GenericArgument::Lifetime(Lifetime::new("'static", Span::call_site()))
                                }
                            })
                            .chain(generics.type_params().map(move |tp| {
                                if i == j {
                                    let ident = &tp.ident;
                                    GenericArgument::Type(parse_quote! { #ident })
                                } else {
                                    GenericArgument::Type(parse_quote! { () })
                                }

                            }))
                    }).collect();
                    let all_generics = (!generic_params.is_empty()).then_some(quote! { < #generic_params > });
                    quote! {{
                        let mailbox: &::tokio::sync::mpsc::Sender<::simpl_actor::Signal<#actor_msg_ident #all_generics>> =
                            unsafe { ::std::mem::transmute(&self.mailbox) };
                        mailbox
                    }}
                };

                let mut sig = sig.clone();
                sig.constness = None;
                sig.asyncness = Some(Token![async](Span::call_site()));
                sig.generics = generics.clone();
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

                let async_methods = (!msg.1.has_lifetime()).then(|| quote! {
                    #[doc = "Sends the message asynchronously, not waiting for a response."]
                    #[allow(non_snake_case)]
                    #vis #async_sig {
                        #mailbox
                            .send(::simpl_actor::Signal::Message(#actor_msg_ident::#variant {
                                __reply: ::std::option::Option::None,
                                #( #field_idents ),*
                            }))
                            .await
                            .map_err(|err| #map_err)?;

                        ::std::result::Result::Ok(())
                    }

                    #[doc = "Sends the message asyncronously with a timeout for mailbox capacity."]
                    #[allow(non_snake_case)]
                    #vis #async_timeout_sig {
                        #mailbox
                            .send_timeout(
                                ::simpl_actor::Signal::Message(#actor_msg_ident::#variant {
                                    __reply: ::std::option::Option::None,
                                    #( #field_idents ),*
                                }),
                                timeout,
                            )
                            .await
                            .map_err(|err| #timeout_map_err)?;

                        ::std::result::Result::Ok(())
                    }

                    #[doc = "Attempts to immediately send the message asyncronously without waiting for a response or mailbox capacity."]
                    #[allow(non_snake_case)]
                    #vis #try_async_sig {
                        #mailbox
                            .try_send(::simpl_actor::Signal::Message(#actor_msg_ident::#variant {
                                __reply: ::std::option::Option::None,
                                #( #field_idents ),*
                            }))
                            .map_err(|err| #try_map_err)?;

                        ::std::result::Result::Ok(())
                    }
                });

                quote! {
                    #[doc = "Sends the messages, waits for processing, and returns a response."]
                    #[allow(non_snake_case)]
                    #vis #sig {
                        ::std::debug_assert!(
                            #actor_ref_global_ident.try_with(|_| {}).is_err(),
                            #normal_debug_msg
                        );

                        let (reply, rx) = ::tokio::sync::oneshot::channel();
                        #mailbox
                            .send(::simpl_actor::Signal::Message(#actor_msg_ident::#variant {
                                __reply: ::std::option::Option::Some(reply),
                                #( #field_idents ),*
                            }))
                            .await
                            .map_err(|err| #map_err)?;

                        rx.await.map_err(|_| ::simpl_actor::ActorError::ActorStopped)
                    }

                    #[doc = "Sends the message with a timeout for adding to the mailbox if the mailbox is full."]
                    #[allow(non_snake_case)]
                    #vis #timeout_sig {
                        ::std::debug_assert!(
                            #actor_ref_global_ident.try_with(|_| {}).is_err(),
                            #timeout_debug_msg
                        );

                        let (reply, rx) = ::tokio::sync::oneshot::channel();
                        #mailbox
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

                    #[doc = "Attempts to send the message immediately without waiting for mailbox capacity."]
                    #[allow(non_snake_case)]
                    #vis #try_sig {
                        ::std::debug_assert!(
                            #actor_ref_global_ident.try_with(|_| {}).is_err(),
                            #try_debug_msg
                        );

                        let (reply, rx) = ::tokio::sync::oneshot::channel();
                        #mailbox
                            .try_send(::simpl_actor::Signal::Message(#actor_msg_ident::#variant {
                                __reply: ::std::option::Option::Some(reply),
                                #( #field_idents ),*
                            }))
                            .map_err(|err| #try_map_err)?;

                        rx.await.map_err(|_| ::simpl_actor::ActorError::ActorStopped)
                    }

                    #async_methods
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
            #[::async_trait::async_trait]
            impl ::simpl_actor::ActorRef for #actor_ref_ident {
                fn id(&self) -> u64 {
                    self.id
                }

                fn is_alive(&self) -> bool {
                    !self.mailbox.is_closed()
                }

                async fn link_child<R: ::simpl_actor::ActorRef>(&self, child: &R) {
                    let this_actor_ref = ::simpl_actor::ActorRef::into_generic(self.clone());
                    ::simpl_actor::ActorRef::link_child(&this_actor_ref, child).await
                }

                async fn unlink_child<R: ::simpl_actor::ActorRef>(&self, child: &R) {
                    let this_actor_ref = ::simpl_actor::ActorRef::into_generic(self.clone());
                    ::simpl_actor::ActorRef::unlink_child(&this_actor_ref, child).await
                }

                async fn link_together<R: ::simpl_actor::ActorRef>(&self, actor_ref: &R) {
                    let this_actor_ref = ::simpl_actor::ActorRef::into_generic(self.clone());
                    ::simpl_actor::ActorRef::link_together(&this_actor_ref, actor_ref).await
                }

                async fn unlink_together<R: ::simpl_actor::ActorRef>(&self, actor_ref: &R) {
                    let this_actor_ref = ::simpl_actor::ActorRef::into_generic(self.clone());
                    ::simpl_actor::ActorRef::unlink_together(&this_actor_ref, actor_ref).await
                }

                async fn notify_link_died(
                    &self,
                    id: u64,
                    reason: ::simpl_actor::ActorStopReason,
                ) -> ::std::result::Result<(), ::simpl_actor::ActorError> {
                    self.mailbox
                        .send(::simpl_actor::Signal::LinkDied(id, reason))
                        .await
                        .map_err(|_| ::simpl_actor::ActorError::ActorNotRunning(()))
                }

                async fn stop_gracefully(&self) -> ::std::result::Result<(), ::simpl_actor::ActorError> {
                    self
                        .mailbox
                        .send(::simpl_actor::Signal::Stop)
                        .await
                        .map_err(|_| ::simpl_actor::ActorError::ActorNotRunning(()))
                }

                fn stop_immediately(&self) {
                    self.stop_notify.notify_waiters();
                }

                async fn wait_for_stop(&self) {
                    self.mailbox.closed().await;
                }

                fn into_generic(self) -> ::simpl_actor::GenericActorRef {
                    unsafe {
                        ::simpl_actor::GenericActorRef::from_parts(
                            self.id,
                            self.mailbox,
                            self.stop_notify,
                            self.links,
                        )
                    }
                }

                fn from_generic(actor_ref: ::simpl_actor::GenericActorRef) -> Self {
                    let (id, mailbox, stop_notify, links) = unsafe { actor_ref.into_parts() };
                    #actor_ref_ident {
                        id,
                        mailbox,
                        stop_notify,
                        links,
                    }
                }
            }
        }
    }

    fn expand_spawn_impl(&self) -> proc_macro2::TokenStream {
        let Self {
            ident,
            actor_msg_ident,
            actor_msg_generics,
            actor_ref_ident,
            actor_ref_global_ident,
            messages,
            ..
        } = self;

        let handlers: Vec<_> = messages
            .iter()
            .map(
                |Message {
                     sig,
                     variant,
                     fields,
                     ..
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
            )
            .collect();

        quote! {
            ::tokio::task_local! {
                #[doc(hidden)]
                static #actor_ref_global_ident: #actor_ref_ident;
            }

            #[automatically_derived]
            #[::async_trait::async_trait]
            impl ::simpl_actor::Spawn for #ident {
                type Ref = #actor_ref_ident;

                fn spawn(self) -> Self::Ref {
                    fn handle_message<'a>(
                        actor: &'a mut #ident,
                        msg: #actor_msg_ident #actor_msg_generics,
                    ) -> ::core::pin::Pin<::std::boxed::Box<dyn ::core::future::Future<Output = ()> + ::core::marker::Send + 'a>> {
                        ::std::boxed::Box::pin(async move {
                            match msg {
                                #( #handlers )*
                            }
                        })
                    }

                    let id = ::simpl_actor::new_actor_id();
                    let (tx, rx) =
                        ::tokio::sync::mpsc::channel(<Self as ::simpl_actor::Actor>::mailbox_size());
                    let stop_notify = ::std::sync::Arc::new(::tokio::sync::Notify::new());
                    let links = ::std::sync::Arc::new(::tokio::sync::Mutex::new(::std::collections::HashMap::new()));
                    let actor_ref = #actor_ref_ident {
                        id,
                        mailbox: tx,
                        stop_notify: ::std::sync::Arc::clone(&stop_notify),
                        links: ::std::sync::Arc::clone(&links),
                    };
                    let generic_actor_ref: ::simpl_actor::GenericActorRef =
                        ::simpl_actor::ActorRef::into_generic(actor_ref.clone());

                    ::tokio::spawn(#actor_ref_global_ident.scope(actor_ref.clone(), async move {
                        ::simpl_actor::CURRENT_ACTOR.scope(generic_actor_ref, async move {
                            ::simpl_actor::run_actor_lifecycle::<Self, #actor_msg_ident #actor_msg_generics, _>(
                                id,
                                self,
                                rx,
                                stop_notify,
                                links,
                                handle_message,
                            ).await
                        }).await
                    }));

                    actor_ref
                }

                async fn spawn_link(self) -> Self::Ref {
                    let link = ::simpl_actor::GenericActorRef::try_current();
                    let actor_ref = ::simpl_actor::Spawn::spawn(self);
                    match link {
                        ::std::option::Option::Some(parent_actor_ref) => {
                            ::simpl_actor::ActorRef::link_together(&actor_ref, &parent_actor_ref).await;
                        }
                        ::std::option::Option::None => {
                            ::std::panic!("spawn_link cannot be called outside any actors")
                        }
                    }
                    actor_ref
                }

                async fn spawn_child(self) -> Self::Ref {
                    let link = ::simpl_actor::GenericActorRef::try_current();
                    let actor_ref = ::simpl_actor::Spawn::spawn(self);
                    match link {
                        ::std::option::Option::Some(parent_actor_ref) => {
                            ::simpl_actor::ActorRef::link_child(&actor_ref, &parent_actor_ref).await;
                        }
                        ::std::option::Option::None => {
                            ::std::panic!("spawn_child cannot be called outside any actors")
                        }
                    }
                    actor_ref
                }

                fn try_actor_ref() -> ::std::option::Option<Self::Ref> {
                    #actor_ref_global_ident.try_with(|actor_ref| actor_ref.clone()).ok()
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
        let actor_ref_global_ident =
            format_ident!("{}_ACTOR_REF", ident.to_string().TO_SHOUTY_SNEK_CASE());
        let messages = Actor::extract_messages(&mut item_impl);
        let actor_generic_params: Punctuated<_, Token![,]> = messages
            .iter()
            .flat_map(|Message { sig, fields, .. }| {
                fields
                    .iter()
                    .filter_map(|field| match &field.ty {
                        Type::Reference(ty_ref) => ty_ref
                            .lifetime
                            .clone()
                            .map(|lifetime| GenericParam::Lifetime(LifetimeParam::new(lifetime))),
                        _ => None,
                    })
                    .chain(sig.generics.params.iter().filter_map(|param| match param {
                        GenericParam::Type(ty_param) => {
                            let mut ty_param = ty_param.clone();
                            ty_param.bounds = Punctuated::new();
                            Some(GenericParam::Type(ty_param))
                        }
                        _ => None,
                    }))
            })
            .collect();
        let has_generic_params = !actor_generic_params.is_empty();
        let actor_generics = Generics {
            lt_token: has_generic_params.then_some(Token![<](Span::call_site())),
            params: actor_generic_params,
            gt_token: has_generic_params.then_some(Token![>](Span::call_site())),
            where_clause: None,
        };
        let actor_msg_generics =
            {
                let mut generic_arguments: Punctuated<_, Token![,]> = Punctuated::new();
                generic_arguments.extend(actor_generics.lifetimes().map(|_| {
                    GenericArgument::Lifetime(Lifetime::new("'static", Span::call_site()))
                }));
                generic_arguments.extend(
                    actor_generics
                        .type_params()
                        .map(|_| GenericArgument::Type(Type::Tuple(parse_quote! { () }))),
                );
                (!generic_arguments.is_empty()).then_some(quote! { < #generic_arguments > })
            };

        Ok(Actor {
            item_impl,
            ident,
            actor_generics,
            actor_msg_ident,
            actor_msg_generics,
            actor_ref_ident,
            actor_ref_global_ident,
            messages,
        })
    }
}
