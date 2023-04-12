// Copyright 2023 Google LLC
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

extern crate proc_macro;
use proc_macro2::{Ident, Span, TokenStream};

use quote::{format_ident, quote, quote_spanned, ToTokens};
use syn::{
    parse_macro_input, punctuated::Punctuated, spanned::Spanned, Error, Generics, ItemTrait,
    Result, Token, TraitItem, TraitItemFn, TypeParamBound,
};

fn unimplemented(x: &impl Spanned, things: &str) -> Error {
    Error::new(
        x.span(),
        format!("{things} are not implemented for tinydyn"),
    )
}

fn generics_unimplemented(generics: &Generics) -> Result<()> {
    if let Some(where_clause) = &generics.where_clause {
        return Err(unimplemented(where_clause, "where clauses"));
    }
    if !generics.params.is_empty() {
        return Err(unimplemented(&generics.params, "generics"));
    }
    Ok(())
}

fn supertraits_unimplemented(supertraits: &Punctuated<TypeParamBound, Token![+]>) -> Result<()> {
    if !supertraits.is_empty() {
        return Err(unimplemented(&supertraits, "supertraits"));
    }
    Ok(())
}

fn unsafe_trait_unsupported(unsafety: &Option<Token![unsafe]>) -> Result<()> {
    if let Some(unsafety) = unsafety {
        return Err(unimplemented(unsafety, "unsafe traits"));
    }
    Ok(())
}

// TODO: refactor to properly separate out parsing logic and token generation logic.
struct CommonNames {
    tinydyn: Ident,
    trait_ident: Ident,
    trait_object: TokenStream,
    private: TokenStream,
    self_local: Ident,
    meta_local: Ident,
    vtable_ident: Ident,
    concrete: TokenStream,
}

impl CommonNames {
    fn new(trait_ident: Ident) -> Self {
        let tinydyn = format_ident!("tinydyn");
        let private = quote!(#tinydyn ::__private);
        let self_local = Ident::new("self_", Span::mixed_site());
        let meta_local = Ident::new("meta", Span::mixed_site());
        let trait_object = quote!(dyn #trait_ident);
        let vtable_ident = format_ident!("{trait_ident}Vtable");
        let concrete = "Concrete".parse().unwrap();
        Self {
            tinydyn,
            private,
            self_local,
            meta_local,
            trait_ident,
            trait_object,
            vtable_ident,
            concrete,
        }
    }
}

#[derive(Clone)]
struct ReceiverArg<'a> {
    type_: ReceiverType,
    ident: &'a Ident,
    elem: &'a syn::TypeReference,
}

impl<'a> ReceiverArg<'a> {
    fn new(receiver: &'a syn::Receiver, names: &'a CommonNames) -> Result<Self> {
        let syn::Type::Reference(elem) = &*receiver.ty else {
            return Err(unimplemented(receiver, "non-reference methods"));
        };
        let type_;
        let ident;
        match &*elem.elem {
            syn::Type::Path(path) if path.path.is_ident("Self") => {
                ident = &names.self_local;
                type_ = if elem.mutability.is_some() {
                    ReceiverType::MutableRef
                } else {
                    ReceiverType::SharedRef
                };
            }
            _ => return Err(unimplemented(receiver, "non-reference methods")),
        };
        Ok(Self { type_, elem, ident })
    }
}

#[derive(Clone, Copy)]
enum ReceiverType {
    /// `&self`
    SharedRef,

    /// `&mut self`
    MutableRef,
}

impl ToTokens for ReceiverArg<'_> {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        self.elem.to_tokens(tokens)
    }
}

impl From<&Option<Token![mut]>> for ReceiverType {
    fn from(mutability: &Option<Token![mut]>) -> Self {
        use ReceiverType::*;
        if mutability.is_some() {
            MutableRef
        } else {
            SharedRef
        }
    }
}

struct MethodArgInfo<'a> {
    needs_bare_transmute: BareConversionNeeded,
    orig_arg_type: &'a syn::Type,
    bare_arg_type: Box<syn::Type>,
    arg_ident: Ident,
    comma: Option<Token![,]>,
    colon: Option<Token![:]>,
    receiver: Option<ReceiverArg<'a>>,
}

impl<'a> MethodArgInfo<'a> {
    fn new(
        arg_pair: syn::punctuated::Pair<&'a syn::FnArg, &'a Token![,]>,
        names: &'a CommonNames,
        arg_num: usize,
    ) -> Result<Self> {
        let arg = *arg_pair.value();
        let comma = arg_pair.punct().map(|&&x| x);
        let CommonNames {
            private,
            trait_object,
            ..
        } = names;
        Ok(match arg {
            syn::FnArg::Receiver(self_arg) => {
                let receiver_arg = ReceiverArg::new(self_arg, names)?;
                let pointer_to = match receiver_arg.type_ {
                    ReceiverType::SharedRef => quote!(*const),
                    ReceiverType::MutableRef => quote!(*mut),
                };
                MethodArgInfo {
                    arg_ident: receiver_arg.ident.clone(),
                    receiver: Some(receiver_arg),
                    colon: self_arg.colon_token,
                    needs_bare_transmute: BareConversionNeeded(false),
                    orig_arg_type: &*self_arg.ty,
                    bare_arg_type: Box::new(
                        syn::parse2(quote!(#private ::SelfPtr<#pointer_to #trait_object>)).unwrap(),
                    ),
                    comma,
                }
            }
            syn::FnArg::Typed(pat_type) => {
                let orig_arg_type = &pat_type.ty;
                let (bare_arg_type, needs_bare_transmute) = to_bare_arg_type(&orig_arg_type)?;
                MethodArgInfo {
                    arg_ident: Ident::new(&format!("arg{arg_num}"), Span::mixed_site()),
                    receiver: None,
                    colon: Some(pat_type.colon_token),
                    needs_bare_transmute,
                    orig_arg_type,
                    bare_arg_type,
                    comma,
                }
            }
        })
    }

    fn into_bare_input_pair(self) -> syn::punctuated::Pair<syn::BareFnArg, Token![,]> {
        let bare_arg = syn::BareFnArg {
            attrs: Vec::new(),
            name: Some((
                self.arg_ident.clone(),
                self.colon.unwrap_or_else(|| Token![:](Span::call_site())),
            )),
            ty: *self.bare_arg_type,
        };
        syn::punctuated::Pair::new(bare_arg, self.comma)
    }
}

struct BareConversionNeeded(pub bool);

struct TraitMethod<'a> {
    sig: &'a syn::Signature,
    args: Vec<MethodArgInfo<'a>>,
    bare_output: syn::ReturnType,
    output_needs_transmute: BareConversionNeeded,
    receiver: ReceiverArg<'a>,
}

impl<'a> TraitMethod<'a> {
    fn new(sig: &'a syn::Signature, names: &'a CommonNames) -> Result<Self> {
        let generics = &sig.generics;
        for generic_param in &generics.params {
            if !matches!(generic_param, syn::GenericParam::Lifetime(_)) {
                return Err(unimplemented(
                    &generics.params,
                    "non-lifetime method generic parameter",
                ));
            }
        }

        if let Some(where_clause) = &generics.where_clause {
            for predicate in &where_clause.predicates {
                if !matches!(predicate, syn::WherePredicate::Lifetime(_)) {
                    return Err(unimplemented(
                        where_clause,
                        "non-lifetime method where clause",
                    ));
                }
            }
        }
        let mut method_receiver = None;
        let args = sig
            .inputs
            .pairs()
            .enumerate()
            .map(|(arg_num, arg_pair)| {
                let arg_info = MethodArgInfo::new(arg_pair, names, arg_num)?;
                if let Some(arg_receiver) = &arg_info.receiver {
                    assert!(method_receiver.is_none(), "more than one receiver");
                    method_receiver = Some(arg_receiver.clone());
                }
                Ok(arg_info)
            })
            .collect::<Result<_>>()?;
        let Some(receiver) = method_receiver else {
            return Err(unimplemented(sig, "non-reference methods"));
        };
        let (bare_output, output_needs_transmute) = match &sig.output {
            syn::ReturnType::Default => (syn::ReturnType::Default, BareConversionNeeded(false)),
            syn::ReturnType::Type(arrow, ty) => {
                let (bare_arg_type, need_convert) = to_bare_arg_type(&*ty)?;
                (
                    syn::ReturnType::Type(arrow.clone(), bare_arg_type),
                    need_convert,
                )
            }
        };
        Ok(Self {
            receiver,
            sig,
            args,
            bare_output,
            output_needs_transmute,
        })
    }

    fn drain_bare_inputs(&mut self) -> syn::punctuated::Punctuated<syn::BareFnArg, Token![,]> {
        self.args
            .drain(..)
            .map(|method_info| method_info.into_bare_input_pair())
            .collect()
    }
}

/// All of the data necessary to build the module that impls for `tinydyn`.
struct TinydynImplModule {
    names: CommonNames,
    // trait_ident: Ident,

    // trait_object: TokenStream,
    // private: TokenStream,
    // vtable_build_expr: TokenStream,
    vtable_entries: Vec<TokenStream>,
    vtable_callers: Vec<TokenStream>,
    /// This is statically alloc'd for every (trait, concrete).
    static_vtable_type: TokenStream,
    /// This builds the `static_vtable_type` for this (trait, concrete).
    static_vtable_expr: TokenStream,
    /// This extra data is carried along in DynPtr.
    metadata_type: TokenStream,
    /// When building a wide pointer, this gets the metadata.
    /// This might build a vtable or get a static one.
    metadata_getter: TokenStream,
}

impl ToTokens for TinydynImplModule {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        tokens.extend([self.to_token_stream()])
    }

    fn into_token_stream(self) -> TokenStream
    where
        Self: Sized,
    {
        let Self {
            static_vtable_type,
            static_vtable_expr,
            metadata_type,
            metadata_getter,
            vtable_callers,
            vtable_entries,
            names:
                CommonNames {
                    vtable_ident,
                    trait_ident,
                    trait_object,
                    tinydyn,
                    private,
                    concrete,
                    ..
                },
            ..
        } = self;

        let mod_ident = format_ident!("__tinydyn_impl_{trait_ident}");
        let newtype_ident = format_ident!("{trait_ident}Newtype");

        quote!(mod #mod_ident {
            use super::*;

            #[derive(Copy, Clone)]
            pub struct #vtable_ident {
                #(#vtable_entries,)*
            }

            #[repr(transparent)]
            pub struct #newtype_ident <T>(T);

            unsafe impl #tinydyn ::PlainDyn for #trait_object {
                type Metadata = #metadata_type;
                type StaticVTable = #static_vtable_type;
                type LocalNewtype<T> = #newtype_ident <T>;
            }

            unsafe impl #tinydyn ::DynTrait for #trait_object {
                type Plain = #trait_object;
                type RemoveSend = #trait_object;
                type RemoveSync = #trait_object;
            }

            unsafe impl #tinydyn ::DynTrait for #trait_object + Send {
                type Plain = #trait_object;
                type RemoveSend = #trait_object;
                type RemoveSync = #trait_object + Send;
            }

            unsafe impl #tinydyn ::DynTrait for #trait_object + Sync {
                type Plain = #trait_object;
                type RemoveSend = #trait_object + Sync;
                type RemoveSync = #trait_object;
            }

            unsafe impl #tinydyn ::DynTrait for #trait_object + Send + Sync {
                type Plain = #trait_object;
                type RemoveSend = #trait_object + Sync;
                type RemoveSync = #trait_object + Send;
            }

            unsafe impl<#concrete> #tinydyn ::BuildDynMeta<#trait_object> for #newtype_ident <#concrete>
            where
                #concrete: #trait_ident,
            {
                const STATIC_VTABLE: #static_vtable_type = #static_vtable_expr;

                fn metadata() -> #metadata_type {
                    #metadata_getter
                }
            }
            unsafe impl<T> #tinydyn ::Implements<#trait_object> for #newtype_ident <T> where T: #trait_ident {}
            unsafe impl<T> #tinydyn ::Implements<#trait_object + Send> for #newtype_ident <T> where T: #trait_ident + Send {}
            unsafe impl<T> #tinydyn ::Implements<#trait_object + Sync> for #newtype_ident <T> where T: #trait_ident + Sync {}
            unsafe impl<T> #tinydyn ::Implements<#trait_object + Send + Sync> for #newtype_ident <T> where T: #trait_ident + Send + Sync {}

            impl<Trait> #trait_ident for #private ::DynTarget<Trait>
            where
                Trait: ?Sized + #tinydyn ::DynTrait<Plain = #trait_object>,
            {
                #(#vtable_callers)*
            }
        })
    }

    fn to_token_stream(&self) -> TokenStream {
        self.clone().into_token_stream()
    }
}

impl TinydynImplModule {
    fn new(trait_item: ItemTrait) -> Result<Self> {
        let ItemTrait {
            generics,
            ident: trait_ident,
            supertraits,
            items,
            unsafety,
            ..
        } = trait_item;
        generics_unimplemented(&generics)?;
        supertraits_unimplemented(&supertraits)?;
        unsafe_trait_unsupported(&unsafety)?;

        let names = CommonNames::new(trait_ident);
        let CommonNames {
            self_local,
            private,
            trait_ident,
            vtable_ident,
            concrete,
            meta_local,
            ..
        } = &names;

        let fn_items: Vec<TraitItemFn> = items
            .into_iter()
            .map(|item| match item {
                TraitItem::Fn(fn_item) => Ok(fn_item),
                _ => Err(unimplemented(&item, "non-function items")),
            })
            .collect::<Result<_>>()?;

        // vtable:
        // - entries: the function pointer fields in the vtable
        // - builders: the field initializers for the concrete type's vtable
        // - callers: the trait impl methods on DynTarget that call a trait method from vtable
        let mut vtable_entries: Vec<TokenStream> = Vec::new();
        let mut vtable_builders: Vec<TokenStream> = Vec::new();
        let mut vtable_callers: Vec<TokenStream> = Vec::new();
        let methods: Vec<TraitMethod> = fn_items
            .iter()
            .map(|fn_item| TraitMethod::new(&fn_item.sig, &names))
            .collect::<Result<_>>()?;
        for mut method in methods {
            let sig = method.sig;
            let entry_ident = sig.ident.clone();
            vtable_builders.push(quote!(
                #entry_ident: core::mem::transmute(
                    <#concrete as #trait_ident>:: #entry_ident as *const ())
            ));
            let erased_cons = match method.receiver.type_ {
                ReceiverType::SharedRef => quote!(self_ref),
                ReceiverType::MutableRef => quote!(self_mut),
            };
            let mut impl_sig = sig.clone();
            let mut call_args = Vec::new();
            let mut args_to_bare = Vec::new();
            for (mut pair, arg) in impl_sig.inputs.pairs_mut().zip(&method.args) {
                // Replace with our custom argument name
                let &MethodArgInfo {
                    orig_arg_type,
                    ref bare_arg_type,
                    ref arg_ident,
                    ..
                } = arg;
                if let syn::FnArg::Typed(pat_type) = pair.value_mut() {
                    pat_type.pat = Box::new(syn::Pat::Ident(syn::PatIdent {
                        attrs: Vec::new(),
                        by_ref: None,
                        mutability: None,
                        ident: arg_ident.clone(),
                        subpat: None,
                    }));
                }

                // Erase lifetimes and prepare for the bare fn (pointer)
                // `transmute` doesn't work with generic arguments, but `transmute_copy` does.
                if arg.needs_bare_transmute.0 {
                    args_to_bare.push(quote!(
                        let #arg_ident = #private
                            ::runtime_layout_verified_transmute::<#orig_arg_type, #bare_arg_type>
                            (#arg_ident);
                    ));
                }
                // The argument in the vtable method call
                call_args.push(arg_ident.to_token_stream());
            }

            let bare_inputs: Punctuated<syn::BareFnArg, Token![,]> = method.drain_bare_inputs();

            let mut vtable_call = quote!((#meta_local . #entry_ident)(#(#call_args,)*));
            // don't forget to transmute the output type if it needs it
            if let (syn::ReturnType::Type(_, out_ty), syn::ReturnType::Type(_, bare_ty)) =
                (&sig.output, &method.bare_output)
            {
                if method.output_needs_transmute.0 {
                    let out_ty = &*out_ty;
                    vtable_call = quote!(#private ::runtime_layout_verified_transmute::<#bare_ty, #out_ty>(
                            #vtable_call));
                }
            }

            let fn_pointer = syn::TypeBareFn {
                lifetimes: None,
                unsafety: sig.unsafety.clone(),
                abi: sig.abi.clone(),
                fn_token: sig.fn_token.clone(),
                paren_token: sig.paren_token.clone(),
                inputs: bare_inputs,
                variadic: None,
                output: method.bare_output,
            };
            vtable_entries.push(quote!(#entry_ident: #fn_pointer));
            vtable_callers.push(quote!(
                #[inline(always)]
                #impl_sig {
                    let #meta_local = #private ::DynTarget::meta(self);
                    let #self_local = #private ::DynTarget:: #erased_cons (self);
                    unsafe {
                        #(#args_to_bare)*
                        #vtable_call
                    }
                }
            ));
        }

        let vtable_build_expr = quote!(
            unsafe {
                #vtable_ident {
                    #(#vtable_builders,)*
                }
            }
        );
        let static_vtable_type; // This is statically alloc'd for every (trait, concrete).
        let static_vtable_expr; // This builds the above.
        let metadata_type; // This extra data is carried along in DynPtr.
        let metadata_getter; // When building a wide pointer, this gets the metadata.

        if fn_items.len() <= 1 {
            static_vtable_type = quote!(#private ::InlineVTable);
            static_vtable_expr = static_vtable_type.clone();
            metadata_type = vtable_ident.to_token_stream();
            metadata_getter = vtable_build_expr;
        } else {
            static_vtable_type = vtable_ident.to_token_stream();
            static_vtable_expr = vtable_build_expr;
            metadata_type = quote!(&'static #vtable_ident);
            metadata_getter = quote!(&Self::STATIC_VTABLE);
        }

        Ok(Self {
            vtable_entries,
            vtable_callers,
            static_vtable_type,
            static_vtable_expr,
            metadata_type,
            metadata_getter,
            names,
        })

        // self.static_vtable_type.to_tokens()
    }
}

/// Returns (bare fn type, whether it needed the conversion)
fn to_bare_arg_type(arg_type: &syn::Type) -> Result<(Box<syn::Type>, BareConversionNeeded)> {
    use syn::fold::Fold;
    struct ReplaceLifetimesWith<'a> {
        replace_with: syn::Lifetime,
        needed_replace: &'a mut bool,
    }
    impl Fold for ReplaceLifetimesWith<'_> {
        fn fold_lifetime(&mut self, lt: syn::Lifetime) -> syn::Lifetime {
            if lt == self.replace_with {
                lt
            } else {
                *self.needed_replace = true;
                self.replace_with.clone()
            }
        }
        fn fold_type_reference(&mut self, mut i: syn::TypeReference) -> syn::TypeReference {
            if !matches!(&i.lifetime, Some(lt) if *lt == self.replace_with) {
                *self.needed_replace = true;
                i.lifetime = Some(self.replace_with.clone());
            }
            i
        }
    }
    let mut needed_replace = false;
    let bare_type = Box::new(
        ReplaceLifetimesWith {
            replace_with: syn::parse_str("'static").unwrap(),
            needed_replace: &mut needed_replace,
        }
        .fold_type(arg_type.clone()),
    );
    Ok((bare_type, BareConversionNeeded(needed_replace)))
}

fn tinydyn_mod_impl(trait_item: ItemTrait) -> Result<TokenStream> {
    TinydynImplModule::new(trait_item).map(ToTokens::into_token_stream)
}

/// Marks a local trait as tinydyn-aware, letting it be used inside of [`Ref`] and [`RefMut`].
///
/// This implements [`DynTrait`] and [`PlainDyn`] for the targeted trait object.
/// This defines an alternate smaller vtable layout that erases layout and drop information.
///
/// While you *can* use tinydyn-aware traits as regular `dyn Trait` trait objects, it's not
/// recommended as it creates two vtables.
///
/// # Example
///
/// ```ignore
/// use tinydyn::{self, tinydyn};
///
/// #[tinydyn]
/// trait Foo {
///     fn blah(&self) -> i32;
///     fn blue(&self) -> i32 { 10 }
/// }
///
/// impl Foo for i32 {
///     fn blah(&self) -> i32 { *self + 1 }
/// }
///
/// // Use `dyn Foo` to reference the trait `Foo` even though it never creates the
/// // regular vtable for `dyn Foo`.
/// let x: tinydyn::Ref<dyn Foo> = Ref::new(&15);
/// assert_eq!(x.blah(), 16);
/// assert_eq!(x.blue(), 10);
/// ```
#[proc_macro_attribute]
pub fn tinydyn(
    params: proc_macro::TokenStream,
    item: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
    if let Some(first_tt) = params.into_iter().next() {
        return quote_spanned!(
            first_tt.span().into()=>
            compile_error!("params must be empty");
        )
        .into();
    }
    let original_tokens = item.clone();
    let input = parse_macro_input!(item as ItemTrait);
    tinydyn_mod_impl(input)
        .map(move |mod_impl| {
            let mut mod_impl = proc_macro::TokenStream::from(mod_impl);
            mod_impl.extend([
                "#[deny(elided_lifetimes_in_paths)]"
                    .parse::<proc_macro::TokenStream>()
                    .unwrap()
                    .into(),
                original_tokens,
            ]);
            mod_impl
        })
        .unwrap_or_else(|e| e.into_compile_error().into())
}
