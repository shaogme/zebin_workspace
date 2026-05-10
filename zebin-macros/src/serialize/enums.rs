use super::{record_state_impl, record_state_poll_logic, resolver_def, resolver_expr, state_def};
use crate::shared::{
    RecordStyle, VariantSpec, binder_slot_ident, has_schema, packed_begin_expr, resolver_name,
    resolver_slot_ident, state_name, variant_archived_name, variant_resolver_name,
    variant_state_name,
};
use quote::quote;
use syn::Ident;

pub fn enum_impl(name: &Ident, variants: &[VariantSpec<'_>]) -> proc_macro2::TokenStream {
    let state_name_outer = state_name(name);
    let resolver_name_outer = resolver_name(name);

    let mut variant_state_defs = Vec::new();
    let mut variant_state_impls = Vec::new();
    let mut variant_resolver_defs = Vec::new();
    let mut state_enum_variants = Vec::new();
    let mut resolver_enum_variants = Vec::new();
    let mut begin_arms = Vec::new();
    let mut poll_arms = Vec::new();

    for variant in variants {
        let variant_user_ident = variant.rename.as_ref().unwrap_or(variant.ident);
        let s_name = variant_state_name(name, variant_user_ident);
        let r_name = variant_resolver_name(name, variant_user_ident);
        let a_name = variant_archived_name(name, variant_user_ident);

        variant_state_defs.push(state_def(&s_name, &variant.record));
        variant_resolver_defs.push(resolver_def(&r_name, &variant.record));

        let stable_schema_key = if has_schema(&variant.record) {
            let key = variant
                .record
                .stable_schema_key
                .expect("schema-bearing records require an explicit stable schema key");
            quote! { #key }
        } else {
            quote! { 0 }
        };

        variant_state_impls.push(record_state_impl(
            &s_name,
            &r_name,
            &variant.record,
            &a_name,
            &stable_schema_key,
        ));

        state_enum_variants.push(quote! { #variant_user_ident(#s_name<'a>) });
        resolver_enum_variants.push(quote! { #variant_user_ident(#r_name) });

        // begin_serialize arm
        let variant_ident = variant.ident;
        let record = &variant.record;
        let begin_arm = match record.style {
            RecordStyle::Named => {
                let binders = record.fields.iter().map(|f| {
                    let id = f.ident.expect("named field has ident");
                    if f.skip {
                        quote! { #id: _ }
                    } else {
                        quote! { #id }
                    }
                });
                let init_fields = record.active_fields().map(|(i, f)| {
                    let binder = binder_slot_ident(record, i);
                    let s_ident = &f.state_ident;
                    let r_ident = resolver_slot_ident(record, i);
                    let ty = f.ty;
                    let begin = if let Some(b) = packed_begin_expr(f, quote! { #binder }) {
                        b
                    } else {
                        quote! { <#ty as zebin::Serialize>::begin_serialize(&#binder)? }
                    };
                    quote! { #s_ident: #begin, #r_ident: ::core::option::Option::None, }
                });
                quote! {
                    Self::#variant_ident { #(#binders),* } => {
                        Ok(#state_name_outer::#variant_user_ident(#s_name { _marker: ::core::marker::PhantomData, #(#init_fields)* }))
                    }
                }
            }
            RecordStyle::Unnamed => {
                let binders = record.fields.iter().enumerate().map(|(i, f)| {
                    if f.skip {
                        quote! { _ }
                    } else {
                        let b = binder_slot_ident(record, i);
                        quote! { #b }
                    }
                });
                let init_fields = record.active_fields().map(|(i, f)| {
                    let binder = binder_slot_ident(record, i);
                    let s_ident = &f.state_ident;
                    let r_ident = resolver_slot_ident(record, i);
                    let ty = f.ty;
                    let begin = if let Some(b) = packed_begin_expr(f, quote! { #binder }) {
                        b
                    } else {
                        quote! { <#ty as zebin::Serialize>::begin_serialize(&#binder)? }
                    };
                    quote! { #s_ident: #begin, #r_ident: ::core::option::Option::None, }
                });
                quote! {
                    Self::#variant_ident( #(#binders),* ) => {
                        Ok(#state_name_outer::#variant_user_ident(#s_name { _marker: ::core::marker::PhantomData, #(#init_fields)* }))
                    }
                }
            }
            RecordStyle::Unit => {
                quote! { Self::#variant_ident => Ok(#state_name_outer::#variant_user_ident(#s_name { _marker: ::core::marker::PhantomData })) }
            }
        };
        begin_arms.push(begin_arm);

        // poll arm
        let poll_logic =
            record_state_poll_logic(record, &a_name, &stable_schema_key, quote! { state });
        let res_expr = resolver_expr(record, &r_name, quote! { state });
        poll_arms.push(quote! {
            #state_name_outer::#variant_user_ident(state) => {
                #poll_logic
                Ok(::core::task::Poll::Ready(#resolver_name_outer::#variant_user_ident(#res_expr)))
            }
        });
    }

    quote! {
        #(#variant_state_defs)*
        #(#variant_state_impls)*
        #(#variant_resolver_defs)*
        pub enum #state_name_outer<'a> { #(#state_enum_variants),* }
        pub enum #resolver_name_outer { #(#resolver_enum_variants),* }
        impl<'a> zebin::SerializeState<'a> for #state_name_outer<'a> {
            type Resolver = #resolver_name_outer;
            fn poll<E: zebin::ByteSink + zebin::LayoutSink<'a> + ?Sized>(
                &mut self, encoder: &mut E,
            ) -> Result<::core::task::Poll<Self::Resolver>, zebin::ZebinError> {
                match self { #(#poll_arms),* }
            }
        }
        impl zebin::Serialize for #name {
            type State<'a> = #state_name_outer<'a> where Self: 'a;
            fn begin_serialize(&self) -> Result<Self::State<'_>, zebin::ZebinError> {
                match self { #(#begin_arms),* }
            }
        }
    }
}
