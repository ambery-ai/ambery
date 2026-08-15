//! derive(Observe)：聚合覆盖断言（docs/observability.md）。
//! 作用在聚合体 struct（Harness）上，生成 `__observe_coverage` 方法——每个字段必须
//! 实现 `ambery_core::observe::Observable`，或显式 `#[observe(skip = "理由")]` 跳过，
//! 否则 E0277（编译期强制所有模块可观测）。

use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, Data, DeriveInput};

#[proc_macro_derive(Observe, attributes(observe))]
pub fn derive_observe(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let Data::Struct(s) = &input.data else {
        return syn::Error::new_spanned(&input, "Observe 只支持 struct")
            .to_compile_error()
            .into();
    };
    let mut errors: Option<syn::Error> = None;
    let mut acc = |e: syn::Error| match &mut errors {
        Some(acc) => acc.combine(e),
        None => errors = Some(e),
    };
    let mut checks = vec![];
    for f in s.fields.iter() {
        let mut skip = false;
        for attr in &f.attrs {
            if !attr.path().is_ident("observe") {
                continue;
            }
            let res = attr.parse_nested_meta(|m| {
                if m.path.is_ident("skip") {
                    skip = true;
                    // 理由必填：#[observe(skip)] 不带值 = 编译错误
                    let value = m
                        .value()
                        .map_err(|_| m.error("skip 必须写理由：#[observe(skip = \"...\")]"))?;
                    let _: syn::LitStr = value.parse()?;
                    Ok(())
                } else {
                    Err(m.error("只支持 #[observe(skip = \"理由\")]"))
                }
            });
            if let Err(e) = res {
                acc(e);
            }
        }
        if !skip {
            let Some(ident) = &f.ident else {
                acc(syn::Error::new_spanned(f, "Observe 只支持命名字段"));
                continue;
            };
            checks.push(quote! { require(&self.#ident); });
        }
    }
    if let Some(e) = errors {
        return e.to_compile_error().into();
    }
    let name = &input.ident;
    let (ig, tg, wc) = input.generics.split_for_impl();
    quote! {
        impl #ig #name #tg #wc {
            /// derive(Observe) 生成的覆盖断言：只类型检查不调用（docs/observability.md）
            #[doc(hidden)]
            #[allow(dead_code)]
            fn __observe_coverage(&self) {
                fn require<T: ::ambery_core::observe::Observable>(_: &T) {}
                #(#checks)*
            }
        }
    }
    .into()
}
