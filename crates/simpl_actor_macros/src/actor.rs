use std::borrow::Cow;

use heck::{ToShoutySnekCase, ToUpperCamelCase};
use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote, quote_spanned, ToTokens};
use syn::{
    custom_keyword,
    parse::{Parse, ParseStream},
    parse_quote,
    punctuated::Punctuated,
    spanned::Spanned,
    Field, FnArg, GenericArgument, GenericParam, Generics, Ident, ImplItem, ItemImpl, Lifetime,
    LifetimeParam, Meta, PathArguments, ReturnType, Signature, Token, Type, Visibility,
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
    infallible: bool,
}

impl Message {
    fn has_lifetime(&self) -> bool {
        self.generics.lifetimes().count() > 0
    }
}

impl From<(Visibility, Signature, bool)> for Message {
    fn from((vis, mut sig, infallible): (Visibility, Signature, bool)) -> Self {
        let mut generics_punctuated: Punctuated<_, Token![,]> = Punctuated::new();
        let variant = format_ident!("{}", sig.ident.to_string().to_upper_camel_case());
        let fields: Punctuated<Field, Token![,]> = sig
            .inputs
            .iter_mut()
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
                fn inject_lifetimes(
                    name: &mut String,
                    ty: &mut Type,
                    lifetimes: &mut Punctuated<Lifetime, Token![,]>,
                ) {
                    match ty {
                        Type::Reference(ty_ref) => {
                            let lt = Lifetime::new(name, Span::call_site());
                            lifetimes.push(lt.clone());
                            ty_ref.lifetime = Some(lt);
                        }
                        Type::Array(ty_array) => {
                            name.push_str("_array");
                            inject_lifetimes(name, &mut ty_array.elem, lifetimes)
                        }
                        Type::Group(ty) => {
                            name.push_str("_group");
                            inject_lifetimes(name, &mut ty.elem, lifetimes)
                        }
                        Type::Paren(ty) => {
                            name.push_str("_paren");
                            inject_lifetimes(name, &mut ty.elem, lifetimes)
                        }
                        Type::Path(ty) => {
                            let original_len = name.len();
                            ty.path.segments.iter_mut().for_each(|segment| {
                                if let PathArguments::AngleBracketed(args) = &mut segment.arguments
                                {
                                    name.truncate(original_len);
                                    let segment_name = format!("_{}_0", segment.ident);
                                    name.push_str(&segment_name);

                                    args.args
                                        .iter_mut()
                                        .filter(|arg| {
                                            matches!(
                                                arg,
                                                GenericArgument::Lifetime(_)
                                                    | GenericArgument::Type(_)
                                            )
                                        })
                                        .enumerate()
                                        .for_each(|(i, arg)| {
                                            if let Some(pos) = name.rfind('_') {
                                                name.truncate(pos);
                                            }
                                            name.push_str(&format!("_{i}"));

                                            match arg {
                                                GenericArgument::Lifetime(lifetime) => {
                                                    *lifetime =
                                                        Lifetime::new(name, Span::call_site());
                                                    lifetimes.push(lifetime.clone());
                                                }
                                                GenericArgument::Type(ty) => {
                                                    inject_lifetimes(name, ty, lifetimes);
                                                }
                                                _ => {}
                                            }
                                        });
                                }
                            })
                        }
                        Type::Slice(ty) => {
                            name.push_str("_slice");
                            inject_lifetimes(name, &mut ty.elem, lifetimes)
                        }
                        Type::Tuple(ty) => {
                            name.push_str("_tuple_0");
                            ty.elems.iter_mut().enumerate().for_each(|(i, ty)| {
                                if let Some(pos) = name.rfind('_') {
                                    name.truncate(pos);
                                }
                                name.push_str(&format!("_{i}"));
                                inject_lifetimes(name, ty, lifetimes)
                            });
                        }
                        _ => {}
                    }
                }
                let mut name = format!("'{}__{}", sig.ident, ident);
                inject_lifetimes(&mut name, &mut pat_type.ty, &mut generics_punctuated);
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
        let generics_punctuated: Punctuated<_, Token![,]> = generics_punctuated
            .into_iter()
            .map(|lifetime| GenericParam::Lifetime(LifetimeParam::new(lifetime)))
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
            infallible,
        }
    }
}

impl Actor {
    fn extract_messages(item_impl: &mut ItemImpl) -> syn::Result<Vec<Message>> {
        let mut errors = Vec::new();
        let messages = item_impl
            .items
            .iter_mut()
            .filter_map(|item| match item {
                ImplItem::Fn(impl_item_fn) => {
                    let mut has_message = false;
                    let mut infallible = matches!(impl_item_fn.sig.output, ReturnType::Default);
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
                            Meta::List(list) if list.path.segments.len() == 1 => {
                                if list.path.segments.first().unwrap().ident.to_string()
                                    == "message"
                                {
                                    has_message = true;
                                    match list.parse_args_with(|input: ParseStream| {
                                        custom_keyword!(infallible);
                                        let kw: infallible = input.parse()?;
                                        Ok(kw)
                                    }) {
                                        Ok(_) => {
                                            infallible = true;
                                        }
                                        Err(err) => {
                                            errors.push(err);
                                        }
                                    }
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
                            infallible,
                        )))
                    } else {
                        None
                    }
                }
                _ => None,
            })
            .collect();

        if !errors.is_empty() {
            let mut iter = errors.into_iter();
            let first = iter.next().unwrap();
            let err = iter.fold(first, |err, mut errors| {
                errors.combine(err);
                errors
            });
            return Err(err);
        }

        Ok(messages)
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
                ::simpl_actor::internal::Signal::Message(#actor_msg_ident::#variant {
                    __reply: _,
                    #field_idents
                }) => ::simpl_actor::SendError::ActorNotRunning((
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
                mailbox: ::tokio::sync::mpsc::UnboundedSender<::simpl_actor::internal::Signal<#actor_msg_ident #actor_msg_generics>>,
                abort_handle: ::futures::stream::AbortHandle,
                links: ::std::sync::Arc<::std::sync::Mutex<::std::collections::HashMap<u64, ::simpl_actor::GenericActorRef>>>,
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
                 ..
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
                        let mailbox: &::tokio::sync::mpsc::UnboundedSender<::simpl_actor::internal::Signal<#actor_msg_ident #all_generics>> =
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
                        parse_quote! { -> ::std::result::Result<(), ::simpl_actor::SendError<( #field_tys )>> }
                    }
                    ReturnType::Type(_, ty) => {
                        parse_quote! { -> ::std::result::Result<#ty, ::simpl_actor::SendError<( #field_tys )>> }
                    }
                };

                let mut timeout_sig = sig.clone();
                timeout_sig.ident = format_ident!("{}_timeout", sig.ident);
                timeout_sig.inputs.push(FnArg::Typed(parse_quote! { timeout: ::std::time::Duration }));

                let mut try_sig = sig.clone();
                try_sig.ident = format_ident!("try_{}", sig.ident);

                let mut async_sig = sig.clone();
                async_sig.asyncness = None;
                async_sig.ident = format_ident!("{}_async", async_sig.ident);
                async_sig.output =
                    parse_quote! { -> ::std::result::Result<(), ::simpl_actor::SendError<( #field_tys )>> };

                let map_err = self.expand_send_error_to_actor_error(variant, fields);

                let normal_debug_msg = format!(
                    "cannot call non-async messages on self as this would deadlock - please use the {} variant instead\nthis assertion only occurs on debug builds, release builds will deadlock",
                    async_sig.ident
                );

                let async_methods = (!msg.1.has_lifetime()).then(|| quote! {
                    /// Sends the message asynchronously, not waiting for a response.
                    ///
                    /// If the message returns an error, the actor will panic.
                    #[allow(non_snake_case)]
                    #vis #async_sig {
                        #mailbox
                            .send(::simpl_actor::internal::Signal::Message(#actor_msg_ident::#variant {
                                __reply: ::std::option::Option::None,
                                #( #field_idents ),*
                            }))
                            .map_err(|err| #map_err)?;

                        ::std::result::Result::Ok(())
                    }
                });

                quote! {
                    /// Sends the messages, waits for processing, and returns a response.
                    #[allow(non_snake_case)]
                    #vis #sig {
                        ::std::debug_assert!(
                            #actor_ref_global_ident.try_with(|_| {}).is_err(),
                            #normal_debug_msg
                        );

                        let (reply, rx) = ::tokio::sync::oneshot::channel();
                        #mailbox
                            .send(::simpl_actor::internal::Signal::Message(#actor_msg_ident::#variant {
                                __reply: ::std::option::Option::Some(reply),
                                #( #field_idents ),*
                            }))
                            .map_err(|err| #map_err)?;

                        rx.await.map_err(|_| ::simpl_actor::SendError::ActorStopped)
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
            impl ::simpl_actor::ActorRef for #actor_ref_ident {
                fn id(&self) -> u64 {
                    self.id
                }

                fn is_alive(&self) -> bool {
                    !self.mailbox.is_closed()
                }

                fn link_child<R: ::simpl_actor::ActorRef>(&self, child: &R) {
                    let this_actor_ref = ::simpl_actor::ActorRef::into_generic(::std::clone::Clone::clone(self));
                    ::simpl_actor::ActorRef::link_child(&this_actor_ref, child)
                }

                fn unlink_child<R: ::simpl_actor::ActorRef>(&self, child: &R) {
                    let this_actor_ref = ::simpl_actor::ActorRef::into_generic(::std::clone::Clone::clone(self));
                    ::simpl_actor::ActorRef::unlink_child(&this_actor_ref, child)
                }

                fn link_together<R: ::simpl_actor::ActorRef>(&self, actor_ref: &R) {
                    let this_actor_ref = ::simpl_actor::ActorRef::into_generic(::std::clone::Clone::clone(self));
                    ::simpl_actor::ActorRef::link_together(&this_actor_ref, actor_ref)
                }

                fn unlink_together<R: ::simpl_actor::ActorRef>(&self, actor_ref: &R) {
                    let this_actor_ref = ::simpl_actor::ActorRef::into_generic(::std::clone::Clone::clone(self));
                    ::simpl_actor::ActorRef::unlink_together(&this_actor_ref, actor_ref)
                }

                fn notify_link_died(
                    &self,
                    id: u64,
                    reason: ::simpl_actor::ActorStopReason,
                ) -> ::std::result::Result<(), ::simpl_actor::SendError> {
                    self.mailbox
                        .send(::simpl_actor::internal::Signal::LinkDied(id, reason))
                        .map_err(|_| ::simpl_actor::SendError::ActorNotRunning(()))
                }

                fn stop_gracefully(&self) -> ::std::result::Result<(), ::simpl_actor::SendError> {
                    self
                        .mailbox
                        .send(::simpl_actor::internal::Signal::Stop)
                        .map_err(|_| ::simpl_actor::SendError::ActorNotRunning(()))
                }

                fn kill(&self) {
                    self.abort_handle.abort()
                }

                async fn wait_for_stop(&self) {
                    self.mailbox.closed().await;
                }

                fn into_generic(self) -> ::simpl_actor::GenericActorRef {
                    unsafe {
                        ::simpl_actor::GenericActorRef::from_parts(
                            self.id,
                            self.mailbox,
                            self.abort_handle,
                            self.links,
                        )
                    }
                }

                fn from_generic(actor_ref: ::simpl_actor::GenericActorRef) -> Self {
                    let (id, mailbox, abort_handle, links) = unsafe { actor_ref.into_parts() };
                    #actor_ref_ident {
                        id,
                        mailbox,
                        abort_handle,
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
                     infallible,
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

                    let output_span = match &sig.output {
                        ReturnType::Default => sig.output.span(),
                        ReturnType::Type(_, ty) => ty.span(),
                    };

                    let infallible_ty_assertion = (!infallible).then_some(quote_spanned! {output_span=>
                        : ::std::result::Result<_, _>
                    });
                    let infallible_block =
                        (!infallible).then_some(quote_spanned! {output_span=>
                            if let ::std::result::Result::Err(err) = res {
                                let err: ::simpl_actor::BoxError = ::std::convert::From::from(err);
                                match actor.on_panic(::simpl_actor::PanicErr::new(err)).await {
                                    ::std::result::Result::Ok(reason) => {
                                        return reason;
                                    }
                                    ::std::result::Result::Err(err) => {
                                        return ::std::option::Option::Some(::simpl_actor::ActorStopReason::Panicked(::simpl_actor::PanicErr::new(err)));
                                    }
                                }
                            }
                        });

                    quote_spanned! {output_span=>
                        #actor_msg_ident::#variant {
                            __reply,
                            #( #field_idents ),*
                        } => {
                            let res #infallible_ty_assertion = actor.#fn_ident( #( #field_idents ),* ) #dot_await;
                            if let ::std::option::Option::Some(reply) = __reply {
                                let _ = reply.send(res);
                                return None;
                            }
                            #infallible_block

                            None
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
            impl ::simpl_actor::Spawn for #ident {
                type Ref = #actor_ref_ident;

                fn spawn(self) -> Self::Ref {
                    fn handle_message<'a>(
                        actor: &'a mut #ident,
                        msg: #actor_msg_ident #actor_msg_generics,
                    ) -> ::core::pin::Pin<::std::boxed::Box<dyn ::core::future::Future<Output = ::std::option::Option<::simpl_actor::ActorStopReason>> + ::core::marker::Send + 'a>> {
                        ::std::boxed::Box::pin(async move {
                            match msg {
                                #( #handlers )*
                            }
                        })
                    }

                    let id = ::simpl_actor::internal::new_actor_id();
                    let (tx, rx) =
                        ::tokio::sync::mpsc::unbounded_channel();
                    let (abort_handle, abort_registration) = ::futures::stream::AbortHandle::new_pair();
                    let links = ::std::sync::Arc::new(::std::sync::Mutex::new(::std::collections::HashMap::new()));
                    let actor_ref = #actor_ref_ident {
                        id,
                        mailbox: tx,
                        abort_handle,
                        links: ::std::sync::Arc::clone(&links),
                    };
                    let generic_actor_ref: ::simpl_actor::GenericActorRef =
                        ::simpl_actor::ActorRef::into_generic(::std::clone::Clone::clone(&actor_ref));

                    ::tokio::spawn(#actor_ref_global_ident.scope(::std::clone::Clone::clone(&actor_ref), async move {
                        ::simpl_actor::CURRENT_ACTOR.scope(generic_actor_ref, async move {
                            ::simpl_actor::internal::run_actor_lifecycle::<Self, #actor_msg_ident #actor_msg_generics, _>(
                                id,
                                self,
                                rx,
                                abort_registration,
                                links,
                                handle_message,
                            ).await
                        }).await
                    }));

                    actor_ref
                }

                fn spawn_link(self) -> Self::Ref {
                    let link = ::simpl_actor::GenericActorRef::try_current();
                    let actor_ref = ::simpl_actor::Spawn::spawn(self);
                    match link {
                        ::std::option::Option::Some(parent_actor_ref) => {
                            ::simpl_actor::ActorRef::link_together(&actor_ref, &parent_actor_ref);
                        }
                        ::std::option::Option::None => {
                            ::std::panic!("spawn_link cannot be called outside any actors")
                        }
                    }
                    actor_ref
                }

                fn spawn_child(self) -> Self::Ref {
                    let link = ::simpl_actor::GenericActorRef::try_current();
                    let actor_ref = ::simpl_actor::Spawn::spawn(self);
                    match link {
                        ::std::option::Option::Some(parent_actor_ref) => {
                            ::simpl_actor::ActorRef::link_child(&actor_ref, &parent_actor_ref);
                        }
                        ::std::option::Option::None => {
                            ::std::panic!("spawn_child cannot be called outside any actors")
                        }
                    }
                    actor_ref
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
        let messages = Actor::extract_messages(&mut item_impl)?;
        let actor_generic_params: Punctuated<_, Token![,]> = messages
            .iter()
            .flat_map(|Message { generics, .. }| generics.params.clone())
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
