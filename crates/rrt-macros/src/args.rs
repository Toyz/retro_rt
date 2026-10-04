//! `#[derive(Args)]`: a struct's fields as command-line arguments.

use proc_macro2::TokenStream;
use quote::quote;
use syn::{Attribute, Data, DeriveInput, Expr, Fields, GenericArgument, LitChar, LitStr, PathArguments, Type};

/// What the field's type makes it.
enum Shape {
    Flag,
    Optional(Type),
    Many(Type),
    One(Type),
}

fn inner(ty: &Type, wrapper: &str) -> Option<Type> {
    let Type::Path(p) = ty else { return None };
    let seg = p.path.segments.last()?;
    if seg.ident != wrapper {
        return None;
    }
    let PathArguments::AngleBracketed(a) = &seg.arguments else { return None };
    match a.args.first()? {
        GenericArgument::Type(t) => Some(t.clone()),
        _ => None,
    }
}

fn shape(ty: &Type) -> Shape {
    if let Type::Path(p) = ty
        && p.path.is_ident("bool")
    {
        return Shape::Flag;
    }
    if let Some(t) = inner(ty, "Option") {
        return Shape::Optional(t);
    }
    if let Some(t) = inner(ty, "Vec") {
        return Shape::Many(t);
    }
    Shape::One(ty.clone())
}

/// The value's name in the usage from its type: PATH, N, TEXT, VALUE.
fn value_name(ty: &Type) -> &'static str {
    let Type::Path(p) = ty else { return "VALUE" };
    match p.path.segments.last().map(|s| s.ident.to_string()).as_deref() {
        Some("PathBuf") => "PATH",
        Some("u8" | "u16" | "u32" | "u64" | "usize" | "i8" | "i16" | "i32" | "i64" | "isize") => "N",
        Some("f32" | "f64") => "NUMBER",
        Some("String") => "TEXT",
        Some("Size") => "WxH",
        _ => "VALUE",
    }
}

/// The doc comment's first paragraph, joined into one line.
fn doc(attrs: &[Attribute]) -> String {
    let mut lines = Vec::new();
    for a in attrs.iter().filter(|a| a.path().is_ident("doc")) {
        if let syn::Meta::NameValue(nv) = &a.meta
            && let Expr::Lit(l) = &nv.value
            && let syn::Lit::Str(s) = &l.lit
        {
            lines.push(s.value().trim().to_string());
        }
    }
    let para: Vec<String> = lines.into_iter().take_while(|l| !l.is_empty()).collect();
    para.join(" ")
}

#[derive(Default)]
struct FieldOpts {
    long: Option<String>,
    short: Option<char>,
    aliases: Vec<String>,
    default: Option<String>,
    value_name: Option<String>,
    positional: bool,
    flatten: bool,
}

fn field_opts(attrs: &[Attribute]) -> syn::Result<FieldOpts> {
    let mut o = FieldOpts::default();
    for a in attrs.iter().filter(|a| a.path().is_ident("arg")) {
        a.parse_nested_meta(|meta| {
            let p = &meta.path;
            if p.is_ident("long") {
                o.long = Some(meta.value()?.parse::<LitStr>()?.value());
            } else if p.is_ident("short") {
                o.short = Some(meta.value()?.parse::<LitChar>()?.value());
            } else if p.is_ident("alias") {
                o.aliases.push(meta.value()?.parse::<LitStr>()?.value());
            } else if p.is_ident("default") {
                o.default = Some(meta.value()?.parse::<LitStr>()?.value());
            } else if p.is_ident("value_name") {
                o.value_name = Some(meta.value()?.parse::<LitStr>()?.value());
            } else if p.is_ident("positional") {
                o.positional = true;
            } else if p.is_ident("flatten") {
                o.flatten = true;
            } else {
                return Err(meta
                    .error("unknown arg option: want long, short, alias, default, value_name, positional or flatten"));
            }
            Ok(())
        })?;
    }
    Ok(o)
}

fn crate_path(attrs: &[Attribute]) -> syn::Result<syn::Path> {
    let mut path: syn::Path = syn::parse_quote!(::rrt::cli);
    for a in attrs.iter().filter(|a| a.path().is_ident("args")) {
        a.parse_nested_meta(|meta| {
            if meta.path.is_ident("crate") {
                path = meta.value()?.parse::<LitStr>()?.parse()?;
                Ok(())
            } else {
                Err(meta.error("unknown args option: want crate"))
            }
        })?;
    }
    Ok(path)
}

pub fn derive(input: DeriveInput) -> syn::Result<TokenStream> {
    let name = &input.ident;
    let Data::Struct(data) = &input.data else {
        return Err(syn::Error::new_spanned(name, "#[derive(Args)] needs a struct with named fields"));
    };
    let Fields::Named(fields) = &data.fields else {
        return Err(syn::Error::new_spanned(name, "#[derive(Args)] needs a struct with named fields"));
    };
    let cli = crate_path(&input.attrs)?;
    let about = doc(&input.attrs);
    let mut specs = Vec::new();
    let mut builds = Vec::new();
    for f in &fields.named {
        let ident = f.ident.as_ref().unwrap();
        let o = field_opts(&f.attrs)?;
        let ty = &f.ty;
        if o.flatten {
            specs.push(quote! { v.extend(<#ty as #cli::Args>::specs()); });
            builds.push(quote! { #ident: <#ty as #cli::Args>::build(m)? });
            continue;
        }
        let long = o.long.clone().unwrap_or_else(|| ident.to_string().replace('_', "-"));
        let help = doc(&f.attrs);
        let shape = shape(ty);
        let (kind, build, required) = match (&shape, o.positional) {
            (Shape::Flag, false) => (quote!(Flag), quote! { m.flag(#long) }, false),
            (Shape::Flag, true) => {
                return Err(syn::Error::new_spanned(f, "a bool is a flag; it cannot be positional"));
            }
            (Shape::Optional(t), false) => (quote!(Value), quote! { m.value::<#t>(#long)? }, false),
            (Shape::Optional(t), true) => (quote!(Positional), quote! { m.positional::<#t>(#long)? }, false),
            (Shape::Many(t), false) => (quote!(Repeated), quote! { m.values::<#t>(#long)? }, false),
            (Shape::Many(t), true) => (quote!(Rest), quote! { m.rest::<#t>(#long)? }, false),
            (Shape::One(t), false) => match &o.default {
                Some(d) => (quote!(Value), quote! { m.value_or::<#t>(#long, #d)? }, false),
                None => (quote!(Value), quote! { m.required::<#t>(#long)? }, true),
            },
            (Shape::One(t), true) => match &o.default {
                Some(d) => (quote!(Positional), quote! { m.positional_or::<#t>(#long, #d)? }, false),
                None => (quote!(Positional), quote! { m.positional_required::<#t>(#long)? }, true),
            },
        };
        let value_name = o.value_name.clone().unwrap_or_else(|| {
            let t = match &shape {
                Shape::Optional(t) | Shape::Many(t) | Shape::One(t) => t.clone(),
                Shape::Flag => ty.clone(),
            };
            value_name(&t).to_string()
        });
        let short = match o.short {
            Some(c) => quote!(Some(#c)),
            None => quote!(None),
        };
        let default = match &o.default {
            Some(d) => quote!(Some(#d)),
            None => quote!(None),
        };
        let aliases = &o.aliases;
        specs.push(quote! {
            v.push(#cli::Spec {
                name: #long,
                aliases: &[#(#aliases),*],
                short: #short,
                kind: #cli::Kind::#kind,
                value_name: #value_name,
                help: #help,
                required: #required,
                default: #default,
            });
        });
        builds.push(quote! { #ident: #build });
    }
    let (impl_g, ty_g, where_g) = input.generics.split_for_impl();
    Ok(quote! {
        impl #impl_g #cli::Args for #name #ty_g #where_g {
            fn about() -> &'static str {
                #about
            }
            fn specs() -> ::std::vec::Vec<#cli::Spec> {
                let mut v = ::std::vec::Vec::new();
                #(#specs)*
                v
            }
            fn build(m: &mut #cli::Matches) -> ::std::result::Result<Self, #cli::Error> {
                ::std::result::Result::Ok(#name { #(#builds),* })
            }
        }
    })
}
