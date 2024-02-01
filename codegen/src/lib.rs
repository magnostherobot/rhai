//! This crate contains procedural macros to make creating Rhai plugin modules much easier.
//!
//! # Export an Entire Rust Module to a Rhai `Module`
//!
//! ```
//! use rhai::{EvalAltResult, FLOAT};
//! use rhai::plugin::*;
//! use rhai::module_resolvers::*;
//!
//! #[export_module]
//! mod advanced_math {
//!     pub const MYSTIC_NUMBER: FLOAT = 42.0;
//!
//!     pub fn euclidean_distance(x1: FLOAT, y1: FLOAT, x2: FLOAT, y2: FLOAT) -> FLOAT {
//!         ((y2 - y1).abs().powf(2.0) + (x2 -x1).abs().powf(2.0)).sqrt()
//!     }
//! }
//!
//! # fn main() -> Result<(), Box<EvalAltResult>> {
//! let mut engine = Engine::new();
//! let m = exported_module!(advanced_math);
//! let mut r = StaticModuleResolver::new();
//! r.insert("Math::Advanced", m);
//! engine.set_module_resolver(r);
//!
//! assert_eq!(engine.eval::<FLOAT>(
//!     r#"
//!         import "Math::Advanced" as math;
//!         math::euclidean_distance(0.0, 1.0, 0.0, math::MYSTIC_NUMBER)
//!     "#)?, 41.0);
//! #   Ok(())
//! # }
//! ```
//!
//! # Register a Rust Function with a Rhai `Module`
//!
//! ```
//! use rhai::{EvalAltResult, FLOAT, Module};
//! use rhai::plugin::*;
//! use rhai::module_resolvers::*;
//!
//! #[export_fn]
//! fn distance_function(x1: FLOAT, y1: FLOAT, x2: FLOAT, y2: FLOAT) -> FLOAT {
//!     ((y2 - y1).abs().powf(2.0) + (x2 -x1).abs().powf(2.0)).sqrt()
//! }
//!
//! # fn main() -> Result<(), Box<EvalAltResult>> {
//! let mut engine = Engine::new();
//! engine.register_fn("get_mystic_number", || 42.0 as FLOAT);
//! let mut m = Module::new();
//! set_exported_fn!(m, "euclidean_distance", distance_function);
//! let mut r = StaticModuleResolver::new();
//! r.insert("Math::Advanced", m);
//! engine.set_module_resolver(r);
//!
//! assert_eq!(engine.eval::<FLOAT>(
//!     r#"
//!         import "Math::Advanced" as math;
//!         math::euclidean_distance(0.0, 1.0, 0.0, get_mystic_number())
//!     "#)?, 41.0);
//! # Ok(())
//! # }
//! ```
//!
//! # Register a Plugin Function with an `Engine`
//!
//! ```
//! use rhai::{EvalAltResult, FLOAT, Module};
//! use rhai::plugin::*;
//! use rhai::module_resolvers::*;
//!
//! #[export_fn]
//! pub fn distance_function(x1: FLOAT, y1: FLOAT, x2: FLOAT, y2: FLOAT) -> FLOAT {
//!     ((y2 - y1).abs().powf(2.0) + (x2 -x1).abs().powf(2.0)).sqrt()
//! }
//!
//! # fn main() -> Result<(), Box<EvalAltResult>> {
//! let mut engine = Engine::new();
//! engine.register_fn("get_mystic_number", || { 42 as FLOAT });
//! register_exported_fn!(engine, "euclidean_distance", distance_function);
//!
//! assert_eq!(engine.eval::<FLOAT>(
//!         "euclidean_distance(0.0, 1.0, 0.0, get_mystic_number())"
//!     )?, 41.0);
//! # Ok(())
//! # }
//! ```
//!

use quote::quote;
use syn::{parse_macro_input, spanned::Spanned, DeriveInput};

mod attrs;
mod custom_type;
mod function;
mod module;
mod register;
mod rhai_module;

#[cfg(test)]
mod test;

/// Attribute, when put on a Rust function, turns it into a _plugin function_.
///
/// # Usage
///
/// ```
/// # use rhai::{Engine, EvalAltResult};
/// use rhai::plugin::*;
///
/// #[export_fn]
/// fn my_plugin_function(x: i64) -> i64 {
///     x * 2
/// }
///
/// # fn main() -> Result<(), Box<EvalAltResult>> {
/// let mut engine = Engine::new();
///
/// register_exported_fn!(engine, "func", my_plugin_function);
///
/// assert_eq!(engine.eval::<i64>("func(21)")?, 42);
/// # Ok(())
/// # }
/// ```
#[proc_macro_attribute]
pub fn export_fn(
    args: proc_macro::TokenStream,
    input: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
    let mut output = quote! {
        #[allow(clippy::needless_pass_by_value)]
    };
    output.extend(proc_macro2::TokenStream::from(input.clone()));

    let parsed_params = match crate::attrs::outer_item_attributes(args.into(), "export_fn") {
        Ok(args) => args,
        Err(err) => return err.to_compile_error().into(),
    };
    let mut function_def = parse_macro_input!(input as function::ExportedFn);

    if !function_def.cfg_attrs().is_empty() {
        return syn::Error::new(
            function_def.cfg_attrs()[0].span(),
            "`cfg` attributes are not allowed for `export_fn`",
        )
        .to_compile_error()
        .into();
    }

    if let Err(e) = function_def.set_params(parsed_params) {
        return e.to_compile_error().into();
    }

    output.extend(function_def.generate());
    proc_macro::TokenStream::from(output)
}

/// Attribute, when put on a Rust module, turns it into a _plugin module_.
///
/// # Usage
///
/// ```
/// # use rhai::{Engine, Module, EvalAltResult};
/// use rhai::plugin::*;
///
/// #[export_module]
/// mod my_plugin_module {
///     pub fn foo(x: i64) -> i64 { x * 2 }
///     pub fn bar() -> i64 { 21 }
/// }
///
/// # fn main() -> Result<(), Box<EvalAltResult>> {
/// let mut engine = Engine::new();
///
/// let module = exported_module!(my_plugin_module);
///
/// engine.register_global_module(module.into());
///
/// assert_eq!(engine.eval::<i64>("foo(bar())")?, 42);
/// # Ok(())
/// # }
/// ```
#[proc_macro_attribute]
pub fn export_module(
    args: proc_macro::TokenStream,
    input: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
    let parsed_params = match crate::attrs::outer_item_attributes(args.into(), "export_module") {
        Ok(args) => args,
        Err(err) => return err.to_compile_error().into(),
    };
    let mut module_def = parse_macro_input!(input as module::Module);
    if let Err(e) = module_def.set_params(parsed_params) {
        return e.to_compile_error().into();
    }

    let tokens = module_def.generate();
    proc_macro::TokenStream::from(tokens)
}

/// Macro to generate a Rhai `Module` from a _plugin module_ defined via [`#[export_module]`][macro@export_module].
///
/// # Usage
///
/// ```
/// # use rhai::{Engine, Module, EvalAltResult};
/// use rhai::plugin::*;
///
/// #[export_module]
/// mod my_plugin_module {
///     pub fn foo(x: i64) -> i64 { x * 2 }
///     pub fn bar() -> i64 { 21 }
/// }
///
/// # fn main() -> Result<(), Box<EvalAltResult>> {
/// let mut engine = Engine::new();
///
/// let module = exported_module!(my_plugin_module);
///
/// engine.register_global_module(module.into());
///
/// assert_eq!(engine.eval::<i64>("foo(bar())")?, 42);
/// # Ok(())
/// # }
/// ```
#[proc_macro]
pub fn exported_module(module_path: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let module_path = parse_macro_input!(module_path as syn::Path);
    proc_macro::TokenStream::from(quote::quote! {
        #module_path::rhai_module_generate()
    })
}

/// Macro to combine a _plugin module_ into an existing module.
///
/// Functions and variables in the plugin module overrides any existing similarly-named
/// functions and variables in the target module.
///
/// This call is intended to be used within the [`def_package!`][crate::def_package] macro to define
/// a custom package based on a plugin module.
///
/// All sub-modules, if any, in the plugin module are _flattened_ and their functions/variables
/// registered at the top level because packages require so.
///
/// The text string name in the second parameter can be anything and is reserved for future use;
/// it is recommended to be an ID string that uniquely identifies the plugin module.
///
/// # Usage
///
/// ```
/// # use rhai::{Engine, Module, EvalAltResult};
/// use rhai::plugin::*;
///
/// #[export_module]
/// mod my_plugin_module {
///     pub fn foo(x: i64) -> i64 { x * 2 }
///     pub fn bar() -> i64 { 21 }
/// }
///
/// # fn main() -> Result<(), Box<EvalAltResult>> {
/// let mut engine = Engine::new();
///
/// let mut module = Module::new();
/// combine_with_exported_module!(&mut module, "my_plugin_module_ID", my_plugin_module);
///
/// engine.register_global_module(module.into());
///
/// assert_eq!(engine.eval::<i64>("foo(bar())")?, 42);
/// # Ok(())
/// # }
/// ```
#[proc_macro]
pub fn combine_with_exported_module(args: proc_macro::TokenStream) -> proc_macro::TokenStream {
    match crate::register::parse_register_macro(args) {
        Ok((module_expr, _export_name, module_path)) => proc_macro::TokenStream::from(quote! {
            #module_path::rhai_generate_into_module(#module_expr, true)
        }),
        Err(e) => e.to_compile_error().into(),
    }
}

/// Macro to register a _plugin function_ (defined via [`#[export_fn]`][macro@export_fn]) into an `Engine`.
///
/// # Deprecated
///
/// This macro is deprecated as it performs no additional value.
///
/// This method will be removed in the next major version.
#[deprecated(since = "1.18.0")]
#[proc_macro]
pub fn register_exported_fn(args: proc_macro::TokenStream) -> proc_macro::TokenStream {
    match crate::register::parse_register_macro(args) {
        Ok((engine_expr, export_name, rust_mod_path)) => {
            let gen_mod_path = crate::register::generated_module_path(&rust_mod_path);
            proc_macro::TokenStream::from(quote! {
                #engine_expr.register_fn(#export_name, #gen_mod_path::dynamic_result_fn)
            })
        }
        Err(e) => e.to_compile_error().into(),
    }
}

/// Macro to register a _plugin function_ into a Rhai `Module`.
///
/// # Deprecated
///
/// This macro is deprecated as it performs no additional value.
///
/// This method will be removed in the next major version.
#[deprecated(since = "1.18.0")]
#[proc_macro]
pub fn set_exported_fn(args: proc_macro::TokenStream) -> proc_macro::TokenStream {
    match crate::register::parse_register_macro(args) {
        Ok((module_expr, export_name, rust_mod_path)) => {
            let gen_mod_path = crate::register::generated_module_path(&rust_mod_path);

            let mut tokens = quote! {
                let fx = FuncRegistration::new(#export_name).with_namespace(FnNamespace::Internal)
            };
            #[cfg(feature = "metadata")]
            tokens.extend(quote! {
                .with_params_info(#gen_mod_path::Token::PARAM_NAMES)
            });
            tokens.extend(quote! {
                ;
                #module_expr.set_fn_raw_with_options(fx, &#gen_mod_path::Token::param_types(), #gen_mod_path::Token().into());
            });
            tokens.into()
        }
        Err(e) => e.to_compile_error().into(),
    }
}

/// Macro to register a _plugin function_ into a Rhai `Module` and expose it globally.
///
/// # Deprecated
///
/// This macro is deprecated as it performs no additional value.
///
/// This method will be removed in the next major version.
#[deprecated(since = "1.18.0")]
#[proc_macro]
pub fn set_exported_global_fn(args: proc_macro::TokenStream) -> proc_macro::TokenStream {
    match crate::register::parse_register_macro(args) {
        Ok((module_expr, export_name, rust_mod_path)) => {
            let gen_mod_path = crate::register::generated_module_path(&rust_mod_path);

            let mut tokens = quote! {
                let fx = FuncRegistration::new(#export_name).with_namespace(FnNamespace::Global)
            };
            #[cfg(feature = "metadata")]
            tokens.extend(quote! {
                .with_params_info(#gen_mod_path::Token::PARAM_NAMES)
            });
            tokens.extend(quote! {
                ;
                #module_expr.set_fn_raw_with_options(fx, &#gen_mod_path::Token::param_types(), #gen_mod_path::Token().into());
            });
            tokens.into()
        }
        Err(e) => e.to_compile_error().into(),
    }
}

/// Macro to implement the [`CustomType`][rhai::CustomType] trait.
///
/// # Usage
///
/// ```
/// use rhai::{CustomType, TypeBuilder};
///
/// #[derive(Clone, CustomType)]
/// struct MyType {
///     foo: i64,
///     bar: bool,
///     baz: String
/// }
/// ```
#[proc_macro_derive(CustomType, attributes(rhai_type,))]
pub fn derive_custom_type(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let expanded = custom_type::derive_custom_type_impl(input);
    expanded.into()
}
