//! Command lines: a small parser every port's binaries share, and the trait
//! `#[derive(rrt::Args)]` implements.
//!
//! A program describes its arguments as [`Spec`]s (the derive writes them
//! from a struct's fields and doc comments); [`parse`] matches a command line
//! against them into [`Matches`]; [`Args::build`] turns those into the
//! struct. [`Args::parse_env`] does all of it for `std::env::args`, printing the
//! usage for `--help` and a message for a mistake.
//!
//! Accepted forms: `--name value`, `--name=value`, `-x value`, `-x` for a
//! flag, positionals in order, `--` to end the options (everything after is
//! positional). A flag or option given twice is an error unless it repeats
//! (`Vec`). An unknown option is an error naming the closest known one.

use std::collections::HashMap;
use std::fmt;
use std::path::PathBuf;
use std::str::FromStr;

/// What one argument is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    /// `--name`: present or not.
    Flag,
    /// `--name VALUE`, at most once.
    Value,
    /// `--name VALUE`, any number of times.
    Repeated,
    /// A positional value, in order.
    Positional,
    /// All remaining positional values.
    Rest,
}

/// One argument a program takes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Spec {
    /// The long name without dashes (`render-scale`), or the positional's
    /// name for the usage line.
    pub name: &'static str,
    /// Other long names accepted for it (`pad-log` for `record`).
    pub aliases: &'static [&'static str],
    /// The one-letter name, if any.
    pub short: Option<char>,
    /// What it is.
    pub kind: Kind,
    /// The value's name in the usage (`PATH`, `N`).
    pub value_name: &'static str,
    /// One line of help.
    pub help: &'static str,
    /// Must be given.
    pub required: bool,
    /// The default, as shown in the help.
    pub default: Option<&'static str>,
}

impl Spec {
    /// A spec with only a name and kind; set the rest with struct update.
    pub const fn new(name: &'static str, kind: Kind) -> Spec {
        Spec { name, aliases: &[], short: None, kind, value_name: "VALUE", help: "", required: false, default: None }
    }
}

/// A command line matched against specs: each option's values by its name,
/// each flag's presence, and the positionals.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Matches {
    values: HashMap<&'static str, Vec<String>>,
    positionals: Vec<String>,
    next_positional: usize,
}

/// A mistake on the command line, or a request for help or the version.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// `--help` or `-h`: print the usage and exit 0.
    Help,
    /// `--version`: print the version and exit 0.
    Version,
    /// An option nobody declared, and the closest one that is.
    Unknown {
        /// What was given.
        given: String,
        /// The nearest declared option, if one is near.
        suggestion: Option<String>,
    },
    /// An option that takes a value at the end of the line.
    MissingValue(String),
    /// A required argument not given.
    Missing(String),
    /// A value that did not convert: the argument, the value, why.
    Invalid {
        /// The argument's name.
        name: String,
        /// The value given.
        value: String,
        /// Why it is wrong.
        why: String,
    },
    /// A flag or single option given more than once.
    Twice(String),
    /// A positional value nobody takes.
    Extra(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Help => write!(f, "help requested"),
            Error::Version => write!(f, "version requested"),
            Error::Unknown { given, suggestion: Some(s) } => write!(f, "unknown option {given} (did you mean --{s}?)"),
            Error::Unknown { given, suggestion: None } => write!(f, "unknown option {given}"),
            Error::MissingValue(n) => write!(f, "--{n} needs a value"),
            Error::Missing(n) => write!(f, "{n} is required"),
            Error::Invalid { name, value, why } => write!(f, "{name}: {value:?} is not valid: {why}"),
            Error::Twice(n) => write!(f, "--{n} is given more than once"),
            Error::Extra(v) => write!(f, "unexpected argument {v:?}"),
        }
    }
}

impl std::error::Error for Error {}

/// A value an argument can hold, parsed from its text.
pub trait FromArg: Sized {
    /// The value, or why the text is not one.
    fn from_arg(s: &str) -> Result<Self, String>;
}

macro_rules! from_str_args {
    ($($t:ty),*) => {$(
        impl FromArg for $t {
            fn from_arg(s: &str) -> Result<Self, String> {
                <$t as FromStr>::from_str(s).map_err(|e| e.to_string())
            }
        }
    )*};
}

from_str_args!(String, PathBuf, u8, u16, u32, u64, usize, i8, i16, i32, i64, isize, f32, f64, char);

impl FromArg for bool {
    /// `true`/`false`, `yes`/`no`, `on`/`off`, `1`/`0`.
    fn from_arg(s: &str) -> Result<bool, String> {
        match s.to_ascii_lowercase().as_str() {
            "true" | "yes" | "on" | "1" => Ok(true),
            "false" | "no" | "off" | "0" => Ok(false),
            _ => Err("want true or false".into()),
        }
    }
}

/// `WIDTHxHEIGHT`, as `640x480`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Size(pub u32, pub u32);

impl FromArg for Size {
    fn from_arg(s: &str) -> Result<Size, String> {
        let (w, h) = s.split_once(['x', 'X']).ok_or("want WIDTHxHEIGHT")?;
        let n = |v: &str| v.parse::<u32>().ok().filter(|n| *n > 0).ok_or("want positive whole numbers");
        Ok(Size(n(w)?, n(h)?))
    }
}

impl fmt::Display for Size {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}x{}", self.0, self.1)
    }
}

/// The edit distance between two names, for suggestions.
fn distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut prev = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cur = row[j + 1];
            row[j + 1] = (prev + usize::from(ca != *cb)).min(row[j] + 1).min(cur + 1);
            prev = cur;
        }
    }
    row[b.len()]
}

/// Matches `args` (without the program name) against `specs`.
pub fn parse(specs: &[Spec], args: impl IntoIterator<Item = String>) -> Result<Matches, Error> {
    let mut m = Matches::default();
    let mut args = args.into_iter();
    let find_long = |name: &str| specs.iter().find(|s| s.name == name || s.aliases.contains(&name));
    let mut options_done = false;
    while let Some(a) = args.next() {
        if options_done || a == "-" || !a.starts_with('-') {
            m.positionals.push(a);
            continue;
        }
        if a == "--" {
            options_done = true;
            continue;
        }
        if a == "--help" || a == "-h" {
            return Err(Error::Help);
        }
        if a == "--version" && find_long("version").is_none() {
            return Err(Error::Version);
        }
        let (spec, inline) = if let Some(long) = a.strip_prefix("--") {
            let (name, inline) = match long.split_once('=') {
                Some((n, v)) => (n, Some(v.to_string())),
                None => (long, None),
            };
            match find_long(name).filter(|s| !matches!(s.kind, Kind::Positional | Kind::Rest)) {
                Some(s) => (s, inline),
                None => {
                    let suggestion = specs
                        .iter()
                        .filter(|s| !matches!(s.kind, Kind::Positional | Kind::Rest))
                        .map(|s| (distance(name, s.name), s.name))
                        .filter(|(d, _)| *d <= 2.max(name.len() / 3))
                        .min()
                        .map(|(_, n)| n.to_string());
                    return Err(Error::Unknown { given: a, suggestion });
                }
            }
        } else {
            let mut chars = a[1..].chars();
            let (Some(c), None) = (chars.next(), chars.next()) else {
                return Err(Error::Unknown { given: a, suggestion: None });
            };
            match specs.iter().find(|s| s.short == Some(c)) {
                Some(s) => (s, None),
                None => return Err(Error::Unknown { given: a, suggestion: None }),
            }
        };
        let entry = m.values.entry(spec.name).or_default();
        match spec.kind {
            Kind::Flag => {
                if let Some(value) = inline {
                    return Err(Error::Invalid {
                        name: format!("--{}", spec.name),
                        value,
                        why: "a flag takes no value".into(),
                    });
                }
                if !entry.is_empty() {
                    return Err(Error::Twice(spec.name.into()));
                }
                entry.push(String::new());
            }
            Kind::Value | Kind::Repeated => {
                if spec.kind == Kind::Value && !entry.is_empty() {
                    return Err(Error::Twice(spec.name.into()));
                }
                let v = match inline {
                    Some(v) => v,
                    None => args.next().ok_or_else(|| Error::MissingValue(spec.name.into()))?,
                };
                entry.push(v);
            }
            Kind::Positional | Kind::Rest => unreachable!("filtered above"),
        }
    }
    let takes_rest = specs.iter().any(|s| s.kind == Kind::Rest);
    let fixed = specs.iter().filter(|s| s.kind == Kind::Positional).count();
    if !takes_rest && m.positionals.len() > fixed {
        return Err(Error::Extra(m.positionals[fixed].clone()));
    }
    for s in specs.iter().filter(|s| s.required) {
        let given = match s.kind {
            Kind::Positional | Kind::Rest => true,
            _ => m.values.contains_key(s.name),
        };
        if !given {
            return Err(Error::Missing(format!("--{}", s.name)));
        }
    }
    Ok(m)
}

impl Matches {
    /// Whether flag `name` was given.
    pub fn flag(&self, name: &str) -> bool {
        self.values.get(name).is_some_and(|v| !v.is_empty())
    }

    /// Option `name`'s value, converted; None when not given.
    pub fn value<T: FromArg>(&self, name: &str) -> Result<Option<T>, Error> {
        match self.values.get(name).and_then(|v| v.last()) {
            None => Ok(None),
            Some(v) => convert(&format!("--{name}"), v).map(Some),
        }
    }

    /// Option `name`'s value converted, or `default` parsed when not given.
    pub fn value_or<T: FromArg>(&self, name: &str, default: &str) -> Result<T, Error> {
        match self.value(name)? {
            Some(v) => Ok(v),
            None => convert(&format!("--{name}"), default),
        }
    }

    /// Option `name`'s value converted; an error when it was not given.
    pub fn required<T: FromArg>(&self, name: &str) -> Result<T, Error> {
        self.value(name)?.ok_or_else(|| Error::Missing(format!("--{name}")))
    }

    /// Every value of repeated option `name`, converted, in order.
    pub fn values<T: FromArg>(&self, name: &str) -> Result<Vec<T>, Error> {
        let label = format!("--{name}");
        self.values.get(name).map_or(Ok(Vec::new()), |vs| vs.iter().map(|v| convert(&label, v)).collect())
    }

    /// The next positional value, converted; None when they have run out.
    pub fn positional<T: FromArg>(&mut self, name: &str) -> Result<Option<T>, Error> {
        let Some(v) = self.positionals.get(self.next_positional).cloned() else { return Ok(None) };
        self.next_positional += 1;
        convert(name, &v).map(Some)
    }

    /// The next positional value converted, or `default` parsed when they
    /// have run out.
    pub fn positional_or<T: FromArg>(&mut self, name: &str, default: &str) -> Result<T, Error> {
        match self.positional(name)? {
            Some(v) => Ok(v),
            None => convert(name, default),
        }
    }

    /// The next positional value converted; an error naming it when they
    /// have run out.
    pub fn positional_required<T: FromArg>(&mut self, name: &str) -> Result<T, Error> {
        self.positional(name)?.ok_or_else(|| Error::Missing(name.to_uppercase()))
    }

    /// Every positional value not yet taken, converted.
    pub fn rest<T: FromArg>(&mut self, name: &str) -> Result<Vec<T>, Error> {
        let rest = self.positionals[self.next_positional.min(self.positionals.len())..].to_vec();
        self.next_positional = self.positionals.len();
        rest.iter().map(|v| convert(name, v)).collect()
    }
}

fn convert<T: FromArg>(name: &str, v: &str) -> Result<T, Error> {
    T::from_arg(v).map_err(|why| Error::Invalid { name: name.into(), value: v.into(), why })
}

/// The usage text: `about`, a usage line, then one line per argument with its
/// help and default.
pub fn usage(program: &str, about: &str, specs: &[Spec]) -> String {
    let mut line = format!("usage: {program}");
    if specs.iter().any(|s| !matches!(s.kind, Kind::Positional | Kind::Rest)) {
        line.push_str(" [OPTIONS]");
    }
    for s in specs {
        match s.kind {
            Kind::Positional if s.required => line.push_str(&format!(" {}", s.name.to_uppercase())),
            Kind::Positional => line.push_str(&format!(" [{}]", s.name.to_uppercase())),
            Kind::Rest => line.push_str(&format!(" [{}]...", s.name.to_uppercase())),
            _ => {}
        }
    }
    let mut out = String::new();
    if !about.is_empty() {
        out.push_str(about.trim());
        out.push_str("\n\n");
    }
    out.push_str(&line);
    out.push_str("\n\n");
    let mut rows: Vec<(String, String)> = Vec::new();
    for s in specs {
        let left = match s.kind {
            Kind::Positional | Kind::Rest => format!("  {}", s.name.to_uppercase()),
            _ => {
                let short = s.short.map_or("    ".to_string(), |c| format!("-{c}, "));
                let value = if s.kind == Kind::Flag { String::new() } else { format!(" {}", s.value_name) };
                let alias = s.aliases.iter().map(|a| format!(", --{a}")).collect::<String>();
                format!("  {short}--{}{alias}{value}", s.name)
            }
        };
        let mut right = s.help.to_string();
        if s.kind == Kind::Repeated {
            right.push_str(" (repeatable)");
        }
        if let Some(d) = s.default {
            right.push_str(&format!(" [default: {d}]"));
        }
        if s.required && !matches!(s.kind, Kind::Positional | Kind::Rest) {
            right.push_str(" [required]");
        }
        rows.push((left, right));
    }
    rows.push(("  -h, --help".into(), "Print this help".into()));
    let width = rows.iter().map(|(l, _)| l.len()).max().unwrap_or(0) + 2;
    for (l, r) in rows {
        out.push_str(&format!("{l:width$}{r}\n"));
    }
    out
}

/// A program's arguments as a struct: what `#[derive(rrt::Args)]`
/// implements.
pub trait Args: Sized {
    /// What the program is, for the help's first line.
    fn about() -> &'static str {
        ""
    }

    /// Every argument, in order (a flattened struct's included).
    fn specs() -> Vec<Spec>;

    /// The struct from matched arguments.
    fn build(m: &mut Matches) -> Result<Self, Error>;

    /// Parses `args` (without the program name).
    fn parse_from(args: impl IntoIterator<Item = String>) -> Result<Self, Error> {
        let specs = Self::specs();
        let mut m = parse(&specs, args)?;
        Self::build(&mut m)
    }

    /// The usage text for `program`.
    fn usage(program: &str) -> String {
        usage(program, Self::about(), &Self::specs())
    }

    /// Parses the process's arguments. `--help` prints the usage and exits
    /// 0, `--version` prints the crate version given and exits 0, and a
    /// mistake prints it with the usage and exits 2.
    fn parse_env(version: &str) -> Self {
        let mut all = std::env::args();
        let program = all
            .next()
            .and_then(|p| std::path::Path::new(&p).file_name().map(|f| f.to_string_lossy().into_owned()))
            .unwrap_or_else(|| "program".into());
        match Self::parse_from(all) {
            Ok(a) => a,
            Err(Error::Help) => {
                print!("{}", Self::usage(&program));
                std::process::exit(0);
            }
            Err(Error::Version) => {
                println!("{program} {version}");
                std::process::exit(0);
            }
            Err(e) => {
                eprintln!("{program}: {e}\n");
                eprint!("{}", Self::usage(&program));
                std::process::exit(2);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(s: &str) -> Vec<String> {
        s.split_whitespace().map(String::from).collect()
    }

    fn specs() -> Vec<Spec> {
        vec![
            Spec { short: Some('s'), value_name: "PATH", help: "write a PNG", ..Spec::new("shot", Kind::Value) },
            Spec { default: Some("1"), ..Spec::new("frames", Kind::Value) },
            Spec { aliases: &["pad-log"], ..Spec::new("record", Kind::Value) },
            Spec::new("press", Kind::Repeated),
            Spec { short: Some('f'), ..Spec::new("fullscreen", Kind::Flag) },
            Spec { required: true, ..Spec::new("disc", Kind::Positional) },
            Spec::new("extra", Kind::Rest),
        ]
    }

    #[test]
    fn every_form_matches() {
        let mut m =
            parse(&specs(), args("game.iso --shot a.png --frames=30 -f --press 1:x --press 2:o --pad-log p.txt b c"))
                .unwrap();
        assert_eq!(m.value::<PathBuf>("shot").unwrap(), Some(PathBuf::from("a.png")));
        assert_eq!(m.value_or::<u32>("frames", "1").unwrap(), 30);
        assert!(m.flag("fullscreen") && !m.flag("nothing"));
        assert_eq!(m.values::<String>("press").unwrap(), ["1:x", "2:o"]);
        assert_eq!(m.value::<String>("record").unwrap().as_deref(), Some("p.txt"), "by its alias");
        assert_eq!(m.positional::<String>("disc").unwrap().as_deref(), Some("game.iso"));
        assert_eq!(m.rest::<String>("extra").unwrap(), ["b", "c"]);
        let mut m = parse(&specs(), args("-s x.png disc -- --not-an-option")).unwrap();
        assert_eq!(m.value::<String>("shot").unwrap().as_deref(), Some("x.png"));
        assert_eq!(m.value_or::<u32>("frames", "1").unwrap(), 1, "the default");
        m.positional::<String>("disc").unwrap();
        assert_eq!(m.rest::<String>("extra").unwrap(), ["--not-an-option"]);
    }

    #[test]
    fn mistakes_say_what_is_wrong() {
        let e = |s: &str| parse(&specs(), args(s)).unwrap_err();
        assert_eq!(e("d --shto a"), Error::Unknown { given: "--shto".into(), suggestion: Some("shot".into()) });
        assert_eq!(e("d --zzzzzzzz"), Error::Unknown { given: "--zzzzzzzz".into(), suggestion: None });
        assert_eq!(e("d --shot"), Error::MissingValue("shot".into()));
        assert_eq!(e("d --shot a --shot b"), Error::Twice("shot".into()));
        assert_eq!(e("d -f -f"), Error::Twice("fullscreen".into()));
        assert!(matches!(e("d --fullscreen=yes"), Error::Invalid { .. }));
        assert_eq!(e("d -h"), Error::Help);
        assert_eq!(e("d --version"), Error::Version);
        let m = parse(&specs(), args("d --frames ten")).unwrap();
        assert!(matches!(m.value_or::<u32>("frames", "1"), Err(Error::Invalid { .. })));
        let no_rest = &specs()[..6];
        assert_eq!(parse(no_rest, args("a b")).unwrap_err(), Error::Extra("b".into()));
        let required = [Spec { required: true, ..Spec::new("cue", Kind::Value) }];
        assert_eq!(parse(&required, args("")).unwrap_err(), Error::Missing("--cue".into()));
        assert!(e("d --shto a").to_string().contains("did you mean --shot?"));
    }

    #[test]
    fn values_convert() {
        assert_eq!(Size::from_arg("640x480"), Ok(Size(640, 480)));
        assert!(Size::from_arg("640").is_err() && Size::from_arg("0x5").is_err());
        assert_eq!(Size(3, 4).to_string(), "3x4");
        assert_eq!(bool::from_arg("On"), Ok(true));
        assert!(bool::from_arg("maybe").is_err());
        assert!(u8::from_arg("300").is_err());
    }

    #[test]
    fn the_usage_lists_every_argument() {
        let u = usage("demo", "A demo.", &specs());
        assert!(u.starts_with("A demo.\n\nusage: demo [OPTIONS] DISC [EXTRA]...\n"));
        assert!(u.contains("-s, --shot PATH"));
        assert!(u.contains("--record, --pad-log VALUE"));
        assert!(u.contains("[default: 1]") && u.contains("(repeatable)") && u.contains("-h, --help"));
    }
}
