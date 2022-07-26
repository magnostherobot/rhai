use crate::{
    module::FuncInfo, plugin::*, tokenizer::is_valid_function_name, Engine, Module, Scope,
};
use core::{cmp::Ordering, fmt, iter};

#[cfg(feature = "no_std")]
use std::prelude::v1::*;

#[cfg(feature = "no_std")]
use alloc::borrow::Cow;

#[cfg(not(feature = "no_std"))]
use std::borrow::Cow;

impl Engine {
    /// Return [`Definitions`] that can be used to
    /// generate definition files that contain all
    /// the visible items in the engine.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use rhai::Engine;
    /// # fn main() -> std::io::Result<()> {
    /// let engine = Engine::new();
    /// engine
    ///     .definitions()
    ///     .write_to_dir(".rhai/definitions")?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn definitions(&self) -> Definitions {
        Definitions {
            engine: self,
            scope: None,
        }
    }

    /// Return [`Definitions`] that can be used to
    /// generate definition files that contain all
    /// the visible items in the engine and the
    /// given scope.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use rhai::{Engine, Scope};
    /// # fn main() -> std::io::Result<()> {
    /// let engine = Engine::new();
    /// let scope = Scope::new();
    /// engine
    ///     .definitions_with_scope(&scope)
    ///     .write_to_dir(".rhai/definitions")?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn definitions_with_scope<'e>(&'e self, scope: &'e Scope<'e>) -> Definitions<'e> {
        Definitions {
            engine: self,
            scope: Some(scope),
        }
    }
}

/// Definitions helper that is used to generate
/// definition files based on the contents of an [`Engine`].
#[must_use]
pub struct Definitions<'e> {
    engine: &'e Engine,
    scope: Option<&'e Scope<'e>>,
}

impl<'e> Definitions<'e> {
    /// Write all the definition files returned from [`iter_files`] to a directory.
    ///
    /// This function will create the directory path if it does not yet exist,
    /// it will also override any existing files as needed.
    #[cfg(all(not(feature = "no_std"), not(target_family = "wasm")))]
    pub fn write_to_dir(&self, path: impl AsRef<std::path::Path>) -> std::io::Result<()> {
        use std::fs;

        let path = path.as_ref();

        fs::create_dir_all(path)?;

        fs::write(
            path.join("__builtin__.d.rhai"),
            include_bytes!("builtin.d.rhai"),
        )?;
        fs::write(
            path.join("__builtin-operators__.d.rhai"),
            include_bytes!("builtin-operators.d.rhai"),
        )?;

        fs::write(path.join("__static__.d.rhai"), self.static_module())?;

        if self.scope.is_some() {
            fs::write(path.join("__scope__.d.rhai"), self.scope())?;
        }

        for (name, decl) in self.modules() {
            fs::write(path.join(format!("{name}.d.rhai")), decl)?;
        }

        Ok(())
    }

    /// Iterate over the generated definition files.
    ///
    /// The returned iterator yields all the definition files as (filename, content) pairs.
    pub fn iter_files(&self) -> impl Iterator<Item = (String, String)> + '_ {
        IntoIterator::into_iter([
            (
                "__builtin__.d.rhai".to_string(),
                include_str!("builtin.d.rhai").to_string(),
            ),
            (
                "__builtin-operators__.d.rhai".to_string(),
                include_str!("builtin-operators.d.rhai").to_string(),
            ),
            ("__static__.d.rhai".to_string(), self.static_module()),
        ])
        .chain(iter::from_fn(move || {
            if self.scope.is_some() {
                Some(("__scope__.d.rhai".to_string(), self.scope()))
            } else {
                None
            }
        }))
        .chain(
            self.modules()
                .map(|(name, def)| (format!("{name}.d.rhai"), def)),
        )
    }

    /// Return the definitions for the globally available
    /// items of the engine.
    ///
    /// The definitions will always start with `module static;`.
    #[must_use]
    pub fn static_module(&self) -> String {
        let mut s = String::from("module static;\n\n");

        let mut first = true;
        for m in &self.engine.global_modules {
            if !first {
                s += "\n\n";
            }
            first = false;
            m.write_definition(&mut s, self).unwrap();
        }

        s
    }

    /// Return the definitions for the available
    /// items of the scope.
    ///
    /// The definitions will always start with `module static;`,
    /// even if the scope is empty or none was provided.
    #[must_use]
    pub fn scope(&self) -> String {
        let mut s = String::from("module static;\n\n");

        if let Some(scope) = self.scope {
            scope.write_definition(&mut s).unwrap();
        }

        s
    }

    /// Return module name and definition pairs for each registered module.
    ///
    /// The definitions will always start with `module <module name>;`.
    ///
    /// If the feature `no_module` is enabled, this will yield no elements.
    pub fn modules(&self) -> impl Iterator<Item = (String, String)> + '_ {
        #[cfg(not(feature = "no_module"))]
        let m = {
            let mut m = self
                .engine
                .global_sub_modules
                .iter()
                .map(move |(name, module)| {
                    (
                        name.to_string(),
                        format!("module {name};\n\n{}", module.definition(self)),
                    )
                })
                .collect::<Vec<_>>();

            m.sort_by(|(name1, _), (name2, _)| name1.cmp(name2));
            m
        };

        #[cfg(feature = "no_module")]
        let m = Vec::new();

        m.into_iter()
    }
}

impl Module {
    fn definition(&self, def: &Definitions) -> String {
        let mut s = String::new();
        self.write_definition(&mut s, def).unwrap();
        s
    }

    fn write_definition(&self, writer: &mut dyn fmt::Write, def: &Definitions) -> fmt::Result {
        let mut first = true;

        let mut vars = self.iter_var().collect::<Vec<_>>();
        vars.sort_by(|(a, _), (b, _)| a.cmp(b));

        for (name, _) in vars {
            if !first {
                writer.write_str("\n\n")?;
            }
            first = false;

            write!(writer, "const {name}: ?;")?;
        }

        let mut func_infos = self.iter_fn().collect::<Vec<_>>();
        func_infos.sort_by(|a, b| match a.metadata.name.cmp(&b.metadata.name) {
            Ordering::Equal => match a.metadata.params.cmp(&b.metadata.params) {
                Ordering::Equal => (a.metadata.params_info.join("")
                    + a.metadata.return_type.as_str())
                .cmp(&(b.metadata.params_info.join("") + b.metadata.return_type.as_str())),
                o => o,
            },
            o => o,
        });

        for f in func_infos {
            if !first {
                writer.write_str("\n\n")?;
            }
            first = false;

            if f.metadata.access == FnAccess::Private {
                continue;
            }

            #[cfg(not(feature = "no_custom_syntax"))]
            let operator = def.engine.custom_keywords.contains_key(&f.metadata.name)
                || (!f.metadata.name.contains('$') && !is_valid_function_name(&f.metadata.name));

            #[cfg(feature = "no_custom_syntax")]
            let operator =
                !f.metadata.name.contains('$') && !is_valid_function_name(&f.metadata.name);

            f.write_definition(writer, def, operator)?;
        }

        Ok(())
    }
}

impl FuncInfo {
    fn write_definition(
        &self,
        writer: &mut dyn fmt::Write,
        def: &Definitions,
        operator: bool,
    ) -> fmt::Result {
        for comment in &*self.metadata.comments {
            writeln!(writer, "{comment}")?;
        }

        if operator {
            writer.write_str("op ")?;
        } else {
            writer.write_str("fn ")?;
        }

        if let Some(name) = self.metadata.name.strip_prefix("get$") {
            write!(writer, "get {name}(")?;
        } else if let Some(name) = self.metadata.name.strip_prefix("set$") {
            write!(writer, "set {name}(")?;
        } else {
            write!(writer, "{}(", self.metadata.name)?;
        }

        let mut first = true;
        for i in 0..self.metadata.params {
            if !first {
                writer.write_str(", ")?;
            }
            first = false;

            let (param_name, param_type) = self
                .metadata
                .params_info
                .get(i)
                .map(|s| {
                    let mut s = s.splitn(2, ':');
                    (
                        s.next().unwrap_or("_").split(' ').last().unwrap(),
                        s.next()
                            .map(|ty| def_type_name(ty, def.engine))
                            .unwrap_or(Cow::Borrowed("?")),
                    )
                })
                .unwrap_or(("_", "?".into()));

            if operator {
                write!(writer, "{param_type}")?;
            } else {
                write!(writer, "{param_name}: {param_type}")?;
            }
        }

        write!(
            writer,
            ") -> {};",
            def_type_name(&self.metadata.return_type, def.engine)
        )?;

        Ok(())
    }
}

/// We have to transform some of the types.
///
/// This is highly inefficient and is currently based on
/// trial and error with the core packages.
///
/// It tries to flatten types, removing `&` and `&mut`,
/// and paths, while keeping generics.
///
/// Associated generic types are also rewritten into regular
/// generic type parameters.
fn def_type_name<'a>(ty: &'a str, engine: &'a Engine) -> Cow<'a, str> {
    let ty = engine.format_type_name(ty).replace("crate::", "");
    let ty = ty.strip_prefix("&mut").unwrap_or(&*ty).trim();
    let ty = ty.split("::").last().unwrap();

    let ty = ty
        .strip_prefix("RhaiResultOf<")
        .and_then(|s| s.strip_suffix('>'))
        .map(str::trim)
        .unwrap_or(ty);

    ty.replace("Iterator<Item=", "Iterator<")
        .replace("Dynamic", "?")
        .replace("INT", "int")
        .replace("FLOAT", "float")
        .replace("&str", "String")
        .replace("ImmutableString", "String")
        .into()
}

impl Scope<'_> {
    fn write_definition(&self, writer: &mut dyn fmt::Write) -> fmt::Result {
        let mut first = true;
        for (name, constant, _) in self.iter_raw() {
            if !first {
                writer.write_str("\n\n")?;
            }
            first = false;

            let kw = if constant { "const" } else { "let" };

            write!(writer, "{kw} {name};")?;
        }

        Ok(())
    }
}