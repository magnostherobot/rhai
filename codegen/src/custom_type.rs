use proc_macro2::{Span, Ident, TokenStream, TokenTree};
use quote::{format_ident, quote, ToTokens};
use syn::{
    punctuated::Punctuated, spanned::Spanned, Data, DataStruct, DeriveInput, Expr, Field, Fields,
    MetaNameValue, Path, Token,
};

const ATTR: &str = "rhai_type";

const OPTION_NAME: &str = "name";
const OPTION_SKIP: &str = "skip";
const OPTION_GET: &str = "get";
const OPTION_GET_MUT: &str = "get_mut";
const OPTION_SET: &str = "set";
const OPTION_READONLY: &str = "readonly";
const OPTION_EXTRA: &str = "extra";

/// Derive the `CustomType` trait for a struct.
pub fn derive_custom_type_impl(input: DeriveInput) -> TokenStream {
    let type_name = input.ident;
    let mut display_name = quote! { stringify!(#type_name) };
    let mut field_accessors = Vec::new();
    let mut shared_field_accessors = Vec::new();
    let mut extras = Vec::new();
    let mut errors = Vec::new();

    for attr in input.attrs.iter().filter(|a| a.path().is_ident(ATTR)) {
        let config_list: Result<Punctuated<Expr, Token![,]>, _> =
            attr.parse_args_with(Punctuated::parse_terminated);

        match config_list {
            Ok(list) => {
                for expr in list {
                    match expr {
                        // Key-value
                        Expr::Assign(..) => {
                            let MetaNameValue { path, value, .. } =
                                syn::parse2::<MetaNameValue>(expr.to_token_stream()).unwrap();

                            if path.is_ident(OPTION_NAME) {
                                // Type name
                                display_name = value.to_token_stream();
                            } else if path.is_ident(OPTION_EXTRA) {
                                match syn::parse2::<Path>(value.to_token_stream()) {
                                    Ok(path) => extras.push(path.to_token_stream()),
                                    Err(err) => errors.push(err.into_compile_error()),
                                }
                            } else {
                                let key = path.get_ident().unwrap().to_string();
                                let msg = format!("invalid option: '{}'", key);
                                errors.push(syn::Error::new(path.span(), msg).into_compile_error());
                            }
                        }
                        // skip
                        Expr::Path(path) if path.path.is_ident(OPTION_SKIP) => {
                            println!("SKIPPED");
                        }
                        // any other identifier
                        Expr::Path(path) if path.path.get_ident().is_some() => {
                            let key = path.path.get_ident().unwrap().to_string();
                            let msg = format!("invalid option: '{}'", key);
                            errors.push(syn::Error::new(path.span(), msg).into_compile_error());
                        }
                        // Error
                        _ => errors.push(
                            syn::Error::new(expr.span(), "expecting identifier")
                                .into_compile_error(),
                        ),
                    }
                }
            }
            Err(err) => errors.push(err.into_compile_error()),
        }
    }

    match input.data {
        // struct Foo { ... }
        Data::Struct(DataStruct {
            fields: Fields::Named(ref f),
            ..
        }) => scan_fields(
            &f.named.iter().collect::<Vec<_>>(),
            &mut field_accessors,
            &mut shared_field_accessors,
            &mut errors,
            &type_name,
        ),

        // struct Foo(...);
        Data::Struct(DataStruct {
            fields: Fields::Unnamed(ref f),
            ..
        }) => scan_fields(
            &f.unnamed.iter().collect::<Vec<_>>(),
            &mut field_accessors,
            &mut shared_field_accessors,
            &mut errors,
            &type_name,
        ),

        // struct Foo;
        Data::Struct(DataStruct {
            fields: Fields::Unit,
            ..
        }) => (),

        // enum ...
        Data::Enum(_) => {
            return syn::Error::new(Span::call_site(), "enums are not yet implemented")
                .into_compile_error()
        }

        // union ...
        Data::Union(_) => {
            return syn::Error::new(Span::call_site(), "unions are not yet supported")
                .into_compile_error()
        }
    };

    let register = {
        let method = {
            quote! { builder.with_name(#display_name) }
        };

        #[cfg(feature = "metadata")]
        {
            let Ok(docs) = crate::attrs::doc_attributes(&input.attrs) else {
                return syn::Error::new(Span::call_site(), "failed to parse doc comments")
                    .into_compile_error();
            };
            // Not sure how to make a Vec<String> a literal, using a string instead.
            let docs = proc_macro2::Literal::string(&docs.join("\n"));
            quote! {  #method.with_comments(&#docs.lines().collect::<Vec<_>>()[..]); }
        }
        #[cfg(not(feature = "metadata"))]
        quote! { #method; }
    };

    let newtype_ident = format_ident!("_RhaiNewtype{}", type_name);

    quote! {
        impl CustomType for #type_name {
            fn build(mut builder: TypeBuilder<Self>) {
                #(#errors)*
                #register
                #(#field_accessors)*
                #(#extras(&mut builder);)*
            }
        }

        #[derive(Clone)]
        struct #newtype_ident(std::rc::Rc<std::cell::RefCell<#type_name>>);

        impl CustomType for #newtype_ident {
            fn build(mut builder: TypeBuilder<Self>) {
                #register
                #(#shared_field_accessors)*
            }
        }
    }
}

fn scan_fields(fields: &[&Field], accessors: &mut Vec<TokenStream>, shared_accessors: &mut Vec<TokenStream>, errors: &mut Vec<TokenStream>, type_name: &Ident) {
    for (i, &field) in fields.iter().enumerate() {
        let mut map_name = None;
        let mut get_fn = None;
        let mut get_mut_fn = None;
        let mut set_fn = None;
        let mut readonly = false;
        let mut skip = false;

        for attr in field.attrs.iter().filter(|a| a.path().is_ident(ATTR)) {
            let options_list: Result<Punctuated<Expr, Token![,]>, _> =
                attr.parse_args_with(Punctuated::parse_terminated);

            let options = match options_list {
                Ok(list) => list,
                Err(err) => {
                    errors.push(err.into_compile_error());
                    continue;
                }
            };

            for expr in options {
                let ident = match expr {
                    // skip
                    Expr::Path(path) if path.path.is_ident(OPTION_SKIP) => {
                        skip = true;

                        // `skip` cannot be used with any other attributes.
                        if get_fn.is_some()
                            || get_mut_fn.is_some()
                            || set_fn.is_some()
                            || map_name.is_some()
                            || readonly
                        {
                            let msg = format!("cannot use '{OPTION_SKIP}' with other attributes");
                            errors.push(syn::Error::new(path.span(), msg).into_compile_error());
                        }

                        continue;
                    }
                    // readonly
                    Expr::Path(path) if path.path.is_ident(OPTION_READONLY) => {
                        readonly = true;

                        if set_fn.is_some() {
                            let msg = format!("cannot use '{OPTION_READONLY}' with '{OPTION_SET}'");
                            errors
                                .push(syn::Error::new(path.path.span(), msg).into_compile_error());
                        }

                        path.path.get_ident().unwrap().clone()
                    }
                    // Key-value
                    Expr::Assign(..) => {
                        let MetaNameValue { path, value, .. } =
                            syn::parse2::<MetaNameValue>(expr.to_token_stream()).unwrap();

                        if path.is_ident(OPTION_NAME) {
                            // Type name
                            map_name = Some(value.to_token_stream());
                        } else if path.is_ident(OPTION_GET) {
                            match syn::parse2::<Path>(value.to_token_stream()) {
                                Ok(path) => get_fn = Some(path.to_token_stream()),
                                Err(err) => errors.push(err.into_compile_error()),
                            }
                        } else if path.is_ident(OPTION_GET_MUT) {
                            match syn::parse2::<Path>(value.to_token_stream()) {
                                Ok(path) => get_mut_fn = Some(path.to_token_stream()),
                                Err(err) => errors.push(err.into_compile_error()),
                            }
                        } else if path.is_ident(OPTION_SET) {
                            match syn::parse2::<Path>(value.to_token_stream()) {
                                Ok(path) => set_fn = Some(path.to_token_stream()),
                                Err(err) => errors.push(err.into_compile_error()),
                            }
                        } else if path.is_ident(OPTION_SKIP) || path.is_ident(OPTION_READONLY) {
                            let key = path.get_ident().unwrap().to_string();
                            let msg = format!("'{key}' cannot have value");
                            errors.push(syn::Error::new(path.span(), msg).into_compile_error());
                            continue;
                        } else {
                            let key = path.get_ident().unwrap().to_string();
                            let msg = format!("invalid option: '{key}'");
                            errors.push(syn::Error::new(path.span(), msg).into_compile_error());
                            continue;
                        }

                        path.get_ident().unwrap().clone()
                    }
                    // any other identifier
                    Expr::Path(path) if path.path.get_ident().is_some() => {
                        let key = path.path.get_ident().unwrap().to_string();
                        let msg = format!("invalid option: '{key}'");
                        errors.push(syn::Error::new(path.span(), msg).into_compile_error());
                        continue;
                    }

                    // Error
                    _ => {
                        errors.push(
                            syn::Error::new(expr.span(), "expecting identifier")
                                .into_compile_error(),
                        );
                        continue;
                    }
                };

                if skip {
                    let msg = format!("cannot use '{ident}' with '{OPTION_SKIP}'");
                    errors.push(syn::Error::new(attr.path().span(), msg).into_compile_error());
                }
            }
        }

        // If skipped don't do anything.
        if skip {
            continue;
        }

        // No field name - use field0, field1...
        let field_name = if let Some(ref field_name) = field.ident {
            quote! { #field_name }
        } else {
            if map_name.is_none() {
                let name = format!("field{i}");
                map_name = Some(quote! { #name });
            }
            let index = proc_macro2::Literal::usize_unsuffixed(i);
            quote! { #index }
        };

        // Override functions
        let get = match (get_mut_fn, get_fn) {
            (Some(func), _) => func,
            (None, Some(func)) => quote! { |obj: &mut Self| #func(&*obj) },
            (None, None) => quote! { |obj: &mut Self| obj.#field_name.clone() },
        };

        let set = set_fn.unwrap_or_else(|| quote! { |obj: &mut Self, val| obj.#field_name = val });
        let name = map_name.unwrap_or_else(|| quote! { stringify!(#field_name) });

        fn find_replace(find: &str, replace: &Ident, stream: &TokenStream) -> TokenStream {
            stream.clone().into_iter().map(|it| match it {
                TokenTree::Ident(x) if x == find => TokenTree::Ident(replace.clone()),
                x => x,
            }).collect()
        }

        shared_accessors.push({
            let encap_get = find_replace("Self", type_name, &get);
            let getter = quote! { |obj: &mut Self| (#encap_get)(&mut obj.0.borrow_mut()) };
            if readonly {
                quote! { builder.with_get(#name, #getter); }
            } else {
                let encap_set = find_replace("Self", type_name, &set);
                let setter = quote! { |obj: &mut Self, val| (#encap_set)(&mut obj.0.borrow_mut(), val) };
                quote! { builder.with_get_set(#name, #getter, #setter); }
            }
        });

        accessors.push({
            let method = if readonly {
                quote! { builder.with_get(#name, #get) }
            } else {
                quote! { builder.with_get_set(#name, #get, #set) }
            };

            #[cfg(feature = "metadata")]
            {
                match crate::attrs::doc_attributes(&field.attrs) {
                    Ok(docs) => {
                        // Not sure how to make a Vec<String> a literal, using a string instead.
                        let docs = proc_macro2::Literal::string(&docs.join("\n"));
                        quote! { #method.and_comments(&#docs.lines().collect::<Vec<_>>()[..]); }
                    }
                    Err(_) => {
                        errors.push(
                            syn::Error::new(
                                Span::call_site(),
                                format!(
                                    "failed to parse doc comments for field {}",
                                    quote! { #name }
                                ),
                            )
                            .into_compile_error(),
                        );
                        continue;
                    }
                }
            }
            #[cfg(not(feature = "metadata"))]
            quote! { #method; }
        });
    }
}
