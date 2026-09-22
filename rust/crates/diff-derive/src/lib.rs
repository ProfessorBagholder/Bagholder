//! `#[derive(Diff)]`: what moved between two values of a wire type, as the page's
//! patch operations (`bagholder_model::patch`).
//!
//! A struct is compared field by field under the name serde gives the field, so the
//! paths are the paths of the JSON the page holds; a field serde leaves out when it
//! is empty (`skip_serializing_if`) is set when it appears and deleted when it goes.
//! A row type names the field that tells its rows apart, `#[diff(key = id)]`, and a
//! list of them is matched by it. An enum is compared as its JSON (`#[diff(key = f)]`
//! names the field its rows are told apart by, when every variant writes one).
//!
//! Only the serde attributes that decide a field's JSON name or presence are read;
//! any other serde attribute that would change what is serialized is refused at
//! compile time, so the paths cannot drift from the JSON unnoticed.

use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, Data, DeriveInput, Fields, Ident, LitStr};

#[proc_macro_derive(Diff, attributes(diff))]
pub fn derive_diff(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    match expand(&input) {
        Ok(t) => t.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

/// serde's rename rules, as serde applies them to a field's Rust name.
fn renamed(rule: &str, name: &str, span: proc_macro2::Span) -> syn::Result<String> {
    Ok(match rule {
        "camelCase" => {
            let mut out = String::new();
            let mut up = false;
            for c in name.chars() {
                if c == '_' {
                    up = true;
                } else if up {
                    out.extend(c.to_uppercase());
                    up = false;
                } else {
                    out.push(c);
                }
            }
            out
        }
        "lowercase" | "snake_case" => name.to_string(),
        "UPPERCASE" => name.to_ascii_uppercase(),
        other => return Err(syn::Error::new(span, format!("Diff does not know serde's rename rule {other:?}"))),
    })
}

struct Container {
    rename_all: Option<String>,
    key: Option<Ident>,
}

fn container(input: &DeriveInput) -> syn::Result<Container> {
    let mut c = Container { rename_all: None, key: None };
    for attr in &input.attrs {
        if attr.path().is_ident("serde") {
            attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("rename_all") {
                    c.rename_all = Some(meta.value()?.parse::<LitStr>()?.value());
                } else if meta.path.is_ident("default") || meta.path.is_ident("untagged") {
                    if meta.input.peek(syn::Token![=]) {
                        meta.value()?.parse::<syn::Expr>()?;
                    }
                } else {
                    return Err(meta.error("Diff does not know what this serde attribute does to the JSON"));
                }
                Ok(())
            })?;
        } else if attr.path().is_ident("diff") {
            attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("key") {
                    c.key = Some(meta.value()?.parse::<Ident>()?);
                    Ok(())
                } else {
                    Err(meta.error("expected `key = <field>`"))
                }
            })?;
        }
    }
    Ok(c)
}

struct Field {
    rename: Option<String>,
    skip_if: Option<syn::ExprPath>,
}

fn field(f: &syn::Field) -> syn::Result<Field> {
    let mut out = Field { rename: None, skip_if: None };
    for attr in f.attrs.iter().filter(|a| a.path().is_ident("serde")) {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("rename") {
                out.rename = Some(meta.value()?.parse::<LitStr>()?.value());
            } else if meta.path.is_ident("skip_serializing_if") {
                out.skip_if = Some(meta.value()?.parse::<LitStr>()?.parse()?);
            } else if meta.path.is_ident("default") || meta.path.is_ident("deserialize_with") || meta.path.is_ident("alias") {
                // how it is read, not how it is written
                if meta.input.peek(syn::Token![=]) {
                    meta.value()?.parse::<syn::Expr>()?;
                }
            } else {
                return Err(meta.error("Diff does not know what this serde attribute does to the JSON"));
            }
            Ok(())
        })?;
    }
    Ok(out)
}

fn expand(input: &DeriveInput) -> syn::Result<proc_macro2::TokenStream> {
    let name = &input.ident;
    let (impl_g, ty_g, where_g) = input.generics.split_for_impl();
    let c = container(input)?;
    let patch = quote!(::bagholder_model::patch);
    let body = match &input.data {
        Data::Enum(_) => {
            quote!(#patch::as_json(self, new, path, ops))
        }
        Data::Struct(s) => {
            let Fields::Named(named) = &s.fields else {
                return Err(syn::Error::new_spanned(name, "Diff needs named fields"));
            };
            let mut steps = Vec::new();
            for f in &named.named {
                let ident = f.ident.as_ref().unwrap();
                let a = field(f)?;
                let raw = ident.to_string();
                let raw = raw.strip_prefix("r#").unwrap_or(&raw);
                let json = match (&a.rename, &c.rename_all) {
                    (Some(r), _) => r.clone(),
                    (None, Some(rule)) => renamed(rule, raw, ident.span())?,
                    (None, None) => raw.to_string(),
                };
                steps.push(match &a.skip_if {
                    None => quote!(#patch::field(&self.#ident, &new.#ident, #json, path, ops);),
                    Some(skip) => quote!(#patch::field_present(&self.#ident, &new.#ident, !#skip(&self.#ident), !#skip(&new.#ident), #json, path, ops);),
                });
            }
            quote!(#(#steps)*)
        }
        Data::Union(_) => return Err(syn::Error::new_spanned(name, "Diff is for structs and enums")),
    };
    let key = match (&c.key, &input.data) {
        (None, _) => quote!(),
        // an enum's rows are told apart by a field every variant writes
        (Some(k), Data::Enum(_)) => {
            let json = k.to_string();
            quote! {
                const KEY: Option<&'static str> = Some(#json);
                fn row_key(&self) -> Option<String> {
                    #patch::json_key_of(self, #json)
                }
            }
        }
        (Some(k), _) => {
            let Data::Struct(s) = &input.data else { unreachable!() };
            let f = s.fields.iter().find(|f| f.ident.as_ref() == Some(k)).ok_or_else(|| syn::Error::new_spanned(k, "no such field"))?;
            let a = field(f)?;
            let raw = k.to_string();
            let json = match (&a.rename, &c.rename_all) {
                (Some(r), _) => r.clone(),
                (None, Some(rule)) => renamed(rule, &raw, k.span())?,
                (None, None) => raw,
            };
            quote! {
                const KEY: Option<&'static str> = Some(#json);
                fn row_key(&self) -> Option<String> {
                    #patch::key_of(&self.#k)
                }
            }
        }
    };
    Ok(quote! {
        impl #impl_g #patch::Diff for #name #ty_g #where_g {
            #key
            fn diff(&self, new: &Self, path: &mut Vec<::serde_json::Value>, ops: &mut Vec<::serde_json::Value>) {
                #body
            }
        }
    })
}
