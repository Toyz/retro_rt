//! `#[rrt::main]`, the program entry point for a retro_rt game.
//!
//! ```ignore
//! #[rrt::main(title = "hwtr", hz = rrt::app::rate::NTSC, size = (960, 720))]
//! fn main() -> Result<Hwtr, String> {
//!     let args = Args::parse()?;     // runs after logging starts
//!     Hwtr::load(&args.cue)
//! }
//! ```
//!
//! expands to
//!
//! ```ignore
//! fn main() {
//!     let config = ::rrt::app::Config::default().title("hwtr").hz(rrt::app::rate::NTSC).size((960, 720));
//!     ::rrt::app::launch(config, || -> Result<Hwtr, String> { /* the body */ });
//! }
//! ```
//!
//! Each `name = value` calls the `rrt_app::Config` builder method of that name, so
//! the settings are exactly `rrt_app::Config`'s, and a misspelt one is a
//! compile error naming the missing method. The function returns anything
//! that is `rrt_app::IntoGame`: the game, or a `Result` of it. The expansion
//! names `::rrt`, so the game depends on the `rrt` facade crate.

mod args;

use proc_macro::TokenStream;
use quote::quote;
use syn::parse::Parser;
use syn::punctuated::Punctuated;
use syn::{DeriveInput, Expr, ItemFn, MetaNameValue, ReturnType, Token, parse_macro_input};

/// Makes `fn main() -> impl IntoGame` the program: logging, the game from the
/// body, then the loop. The attribute's `name = value` pairs set
/// `rrt::app::Config` by its builder methods (`title`, `size`, `hz`, `vsync`,
/// `fps_cap`, `log`, `aspect`, `filter`, ...).
#[proc_macro_attribute]
pub fn main(attr: TokenStream, item: TokenStream) -> TokenStream {
    let pairs = match Punctuated::<MetaNameValue, Token![,]>::parse_terminated.parse(attr) {
        Ok(p) => p,
        Err(e) => return e.to_compile_error().into(),
    };
    let f = parse_macro_input!(item as ItemFn);
    if !f.sig.inputs.is_empty() || f.sig.asyncness.is_some() {
        return syn::Error::new_spanned(&f.sig, "#[rrt::main] takes `fn main() -> Game` with no arguments, not async")
            .to_compile_error()
            .into();
    }
    let ReturnType::Type(_, ret) = &f.sig.output else {
        return syn::Error::new_spanned(&f.sig, "#[rrt::main] needs the function to return the game")
            .to_compile_error()
            .into();
    };
    let mut setters = Vec::new();
    for p in pairs {
        let Some(name) = p.path.get_ident() else {
            return syn::Error::new_spanned(&p.path, "expected a Config setting name").to_compile_error().into();
        };
        let value: &Expr = &p.value;
        setters.push(quote! { .#name(#value) });
    }
    let attrs = &f.attrs;
    let vis = &f.vis;
    let name = &f.sig.ident;
    let body = &f.block;
    quote! {
        #(#attrs)*
        #vis fn #name() {
            let config = ::rrt::app::Config::default() #(#setters)*;
            ::rrt::app::launch(config, || -> #ret #body);
        }
    }
    .into()
}

/// Implements `rrt::cli::Args` for a struct with named fields: each field an
/// argument, its doc comment the help, the struct's doc comment the about.
///
/// By type: `bool` is a flag (`--name`); `Option<T>` an optional
/// `--name VALUE`; `Vec<T>` a repeatable one; any other `T` a required one,
/// or optional with `#[arg(default = "...")]`. `T` is anything
/// `rrt::cli::FromArg` (strings, paths, numbers, `Size`, your own).
///
/// `#[arg(...)]` on a field: `long = "name"` (default: the field name with
/// `-` for `_`), `short = 'x'`, `alias = "other"` (repeatable), `default =
/// "text"`, `value_name = "PATH"`, `positional` (in field order; a `Vec` takes
/// the rest), `flatten` (embed another `Args` struct's arguments, such as
/// `rrt::app::AppArgs`). `#[args(crate = "path")]` on the struct changes where
/// the `cli` module is found (default `::rrt::cli`).
///
/// ```ignore
/// /// Hot Wheels Turbo Racing, ported.
/// #[derive(rrt::Args)]
/// struct Cli {
///     /// The disc's CUE sheet.
///     #[arg(positional, default = "work/disc/game.cue")]
///     cue: PathBuf,
///     /// Start on this track.
///     #[arg(short = 't', default = "DESERT1")]
///     track: String,
///     #[arg(flatten)]
///     app: rrt::app::AppArgs,
/// }
/// ```
#[proc_macro_derive(Args, attributes(arg, args))]
pub fn derive_args(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    args::derive(input).unwrap_or_else(|e| e.to_compile_error()).into()
}
