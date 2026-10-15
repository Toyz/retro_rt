//! `rrt-dis`: disassembles a PS1 or PS2 executable with a project's symbol
//! file, and marks names in that file.
//!
//! ```text
//! rrt-dis SLUS_012.34 -s symbols/game.txt --stats
//! rrt-dis SLUS_012.34 -s symbols/game.txt -f DrawCar -f 0x80012345
//! rrt-dis SLUS_012.34 -s symbols/game.txt --mark DrawCar=0x80012345:func:likely
//! rrt-dis bios.bin --base 0xbfc00000 --from 0xbfc00000 --to 0xbfc00400
//! ```

#![forbid(unsafe_code)]

use std::path::PathBuf;
use std::process::ExitCode;

use rrt_dis::analysis::{Found, Image, Program, find_gp};
use rrt_dis::listing::Listing;
use rrt_dis::pseudo::Pseudo;
use rrt_dis::{Arch, Confidence, Kind, Symbol, Symbols, arch, exe};
use rrt_kit::cli::{Args, FromArg};

// `print!` and `println!` that end the program quietly when the reader has
// gone (`rrt-dis ... | head`) instead of panicking.
macro_rules! print {
    ($($t:tt)*) => { emit(format_args!($($t)*)) };
}
macro_rules! println {
    ($($t:tt)*) => {{ print!($($t)*); print!("\n") }};
}

fn emit(args: std::fmt::Arguments) {
    use std::io::Write;
    if let Err(e) = std::io::stdout().lock().write_fmt(args) {
        if e.kind() != std::io::ErrorKind::BrokenPipe {
            eprintln!("rrt-dis: {e}");
            std::process::exit(1);
        }
        std::process::exit(0);
    }
}

/// An address, in hex with or without `0x`.
#[derive(Clone, Copy, Debug)]
struct Addr(u32);

impl FromArg for Addr {
    fn from_arg(s: &str) -> Result<Addr, String> {
        let h = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")).unwrap_or(s);
        u32::from_str_radix(&h.replace('_', ""), 16).map(Addr).map_err(|_| format!("{s:?} is not a hex address"))
    }
}

/// Disassembles and decompiles an executable (any format `--list` shows, or
/// raw bytes with --base), naming addresses from a project's symbol file.
#[derive(rrt_macros::Args)]
#[args(crate = "rrt_kit::cli")]
struct Cli {
    /// The executable.
    #[arg(positional, value_name = "FILE")]
    input: Option<PathBuf>,
    /// List the backends and executable formats, and exit.
    list: bool,
    /// The project's symbol file. Its `func` symbols are certain function
    /// starts; --mark edits it in place.
    #[arg(short = 's', value_name = "FILE")]
    symbols: Option<PathBuf>,
    /// The backend (`--list` names them). Default: the one the executable
    /// says; r3000 for raw bytes.
    #[arg(value_name = "CPU")]
    arch: Option<String>,
    /// Where raw bytes load.
    #[arg(value_name = "ADDR")]
    base: Option<Addr>,
    /// The entry point (default: the executable's, or --base).
    #[arg(value_name = "ADDR")]
    entry: Option<Addr>,
    /// The global pointer's value (default: found in crt0, or the PS-X
    /// EXE's gp0).
    #[arg(value_name = "ADDR")]
    gp: Option<Addr>,
    /// List one function: a symbol name or an address. Repeatable.
    #[arg(short = 'f', value_name = "NAME|ADDR")]
    func: Vec<String>,
    /// List from this address (with --to).
    #[arg(value_name = "ADDR")]
    from: Option<Addr>,
    /// List up to this address (with --from).
    #[arg(value_name = "ADDR")]
    to: Option<Addr>,
    /// Decompile one function to pseudo-code: a symbol name or an address.
    /// Repeatable.
    #[arg(short = 'p', value_name = "NAME|ADDR")]
    pseudo: Vec<String>,
    /// Decompile every function, and say how many there were.
    pseudo_all: bool,
    /// Print what the analysis found: functions by how, jump tables,
    /// strings, code left unowned.
    stats: bool,
    /// Print the functions found that the symbol file does not name, as
    /// symbol-file lines to review and paste.
    functions: bool,
    /// Print the strings the code refers to.
    strings: bool,
    /// Leave addresses and encodings out of listings.
    no_addresses: bool,
    /// Name an address in the symbol file, NAME=ADDR[:KIND[:CONFIDENCE]]
    /// (`func`/`data`/`label`, `guess`/`likely`/`sure`). Repeatable.
    #[arg(value_name = "MARK")]
    mark: Vec<String>,
    /// Let --mark replace a `sure` name, or make a name less sure.
    force: bool,
}

fn main() -> ExitCode {
    let cli = Cli::parse_env(env!("CARGO_PKG_VERSION"));
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("rrt-dis: {e}");
            ExitCode::FAILURE
        }
    }
}

/// The input as an image.
struct Input {
    base: u32,
    bytes: Vec<u8>,
    entry: u32,
    /// The executable's global pointer, when it gives one.
    gp: Option<u32>,
    /// The backend when --arch does not say.
    arch: Option<&'static str>,
    /// The executable's own symbols, empty when stripped or raw.
    symbols: Symbols,
}

fn load(cli: &Cli, data: Vec<u8>) -> Result<Input, String> {
    if exe::format_of(&data).is_some() {
        let l = exe::load(&data)?;
        let (base, bytes) = l.flatten()?;
        return Ok(Input { base, bytes, entry: l.entry, gp: l.gp, arch: l.arch, symbols: l.symbols });
    }
    let names: Vec<&str> = exe::FORMATS.iter().map(|f| f.name).collect();
    let base =
        cli.base.ok_or_else(|| format!("not a known format ({}): give --base for raw bytes", names.join(", ")))?.0;
    Ok(Input { base, bytes: data, entry: base, gp: None, arch: Some("r3000"), symbols: Symbols::new() })
}

fn list() {
    println!("backends (--arch):");
    for b in arch::BACKENDS {
        let aliases = if b.aliases.is_empty() { String::new() } else { format!(" ({})", b.aliases.join(", ")) };
        println!("  {:<8} {}{aliases}", b.name, b.about);
    }
    println!("formats:");
    for f in exe::FORMATS {
        println!("  {:<8} {}", f.name, f.about);
    }
}

/// A `--mark`: the name, the address, and the kind and confidence when given.
struct Mark {
    name: String,
    addr: u32,
    kind: Option<Kind>,
    confidence: Option<Confidence>,
}

fn parse_mark(m: &str) -> Result<Mark, String> {
    let (name, rest) = m.split_once('=').ok_or_else(|| format!("--mark {m:?}: want NAME=ADDR[:KIND[:CONFIDENCE]]"))?;
    if name.is_empty() || name.contains(|c: char| c.is_whitespace() || c == '#') {
        return Err(format!("--mark {m:?}: a name is one word, without `#`"));
    }
    let mut parts = rest.split(':');
    let addr = Addr::from_arg(parts.next().unwrap_or(""))?.0;
    let kind = parts.next().map(str::parse::<Kind>).transpose()?;
    let confidence = parts.next().map(str::parse::<Confidence>).transpose()?;
    if parts.next().is_some() {
        return Err(format!("--mark {m:?}: want NAME=ADDR[:KIND[:CONFIDENCE]]"));
    }
    Ok(Mark { name: name.to_string(), addr, kind, confidence })
}

/// The symbol `m` leaves at its address. Naming an address that already has
/// a name keeps what the mark does not say (kind, size, confidence, note);
/// replacing a `sure` name, or lowering a name's confidence, needs `force`.
fn apply_mark(symbols: &Symbols, m: Mark, force: bool) -> Result<Symbol, String> {
    if let Some(other) = symbols.get(&m.name)
        && other.addr != m.addr
    {
        return Err(format!("{} already names 0x{:08x}", m.name, other.addr));
    }
    let Some(old) = symbols.at(m.addr) else {
        return Ok(Symbol {
            kind: m.kind.unwrap_or_default(),
            confidence: m.confidence,
            ..Symbol::new(m.name, m.addr)
        });
    };
    let confidence = m.confidence.or(old.confidence);
    if !force {
        if old.name != m.name && old.confidence == Some(Confidence::Sure) {
            return Err(format!("0x{:08x} is {}, marked sure; --force to rename it", m.addr, old.name));
        }
        if confidence < old.confidence {
            return Err(format!(
                "{} at 0x{:08x} is {}; --force to make it less sure",
                old.name,
                m.addr,
                old.confidence.map_or("", Confidence::as_str)
            ));
        }
    }
    Ok(Symbol { name: m.name, kind: m.kind.unwrap_or(old.kind), confidence, ..old.clone() })
}

/// Writes the symbol file so that a crash cannot leave it half written, only
/// when the text reads back as the same symbols, and only when the file is
/// still what was read (`read`, None when there was no file): an editor or
/// another tool that saved it meanwhile is not overwritten. The file keeps
/// its permissions.
fn save(path: &std::path::Path, symbols: &Symbols, read: Option<&str>) -> Result<(), String> {
    let text = symbols.to_text();
    match Symbols::parse(&text) {
        Ok(back) if back == *symbols => {}
        Ok(_) => return Err(format!("{}: the edit would not read back the same; nothing written", path.display())),
        Err(e) => return Err(format!("{}: the edit would not parse ({e}); nothing written", path.display())),
    }
    let name = path.file_name().map_or("symbols".into(), |n| n.to_string_lossy().into_owned());
    let tmp = path.with_file_name(format!(".{name}.rrt-dis-tmp"));
    std::fs::write(&tmp, text).map_err(|e| format!("{}: {e}", tmp.display()))?;
    let now = std::fs::read_to_string(path).ok();
    if now.as_deref() != read {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("{}: changed since it was read; nothing written, run again", path.display()));
    }
    if let Ok(meta) = std::fs::metadata(path) {
        let _ = std::fs::set_permissions(&tmp, meta.permissions());
    }
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("{}: {e}", path.display())
    })
}

/// The executable's own symbols with the project's over them: a project
/// name replaces whatever the executable called that address, and takes its
/// name from wherever the executable put it.
fn merge(mut own: Symbols, project: &Symbols) -> Symbols {
    if own.is_empty() {
        return project.clone();
    }
    for sym in project.iter() {
        if let Some(old) = own.at(sym.addr).map(|o| o.name.clone()) {
            own.remove(&old);
        }
        own.remove(&sym.name);
        own.mark(sym.clone());
    }
    own
}

fn run(cli: Cli) -> Result<(), String> {
    if cli.list {
        list();
        return Ok(());
    }
    let read = match &cli.symbols {
        Some(path) if path.exists() => {
            Some(std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?)
        }
        Some(path) if cli.mark.is_empty() => return Err(format!("{}: no such file", path.display())),
        _ => None,
    };
    let mut symbols = match (&cli.symbols, &read) {
        (Some(path), Some(text)) => Symbols::parse(text).map_err(|e| format!("{}: {e}", path.display()))?,
        _ => Symbols::new(),
    };
    if !cli.mark.is_empty() {
        let path = cli.symbols.as_ref().ok_or("--mark needs the symbol file (-s)")?;
        // Every mark is checked before the file is touched: all or nothing.
        for m in &cli.mark {
            let sym = apply_mark(&symbols, parse_mark(m)?, cli.force)?;
            if let Some(old) = symbols.at(sym.addr)
                && old.name != sym.name
            {
                println!("{} renamed to {}", old.name, sym.name);
            }
            println!("{}", sym.to_line());
            symbols.mark(sym);
        }
        save(path, &symbols, read.as_deref())?;
    }
    let listing_asked = !cli.func.is_empty()
        || !cli.pseudo.is_empty()
        || cli.pseudo_all
        || cli.from.is_some()
        || cli.stats
        || cli.functions
        || cli.strings;
    if !cli.mark.is_empty() && !listing_asked {
        return Ok(());
    }

    let input = cli.input.as_ref().ok_or("no executable given (FILE)")?;
    let data = std::fs::read(input).map_err(|e| format!("{}: {e}", input.display()))?;
    let Input { base, bytes, entry, gp: exe_gp, arch: default_arch, symbols: own } = load(&cli, data)?;
    let symbols = merge(own, &symbols);
    let name = cli.arch.as_deref().or(default_arch).ok_or("the executable does not say its CPU: give --arch")?;
    let backend = arch::by_name(name).ok_or_else(|| {
        let names: Vec<&str> = arch::BACKENDS.iter().map(|b| b.name).collect();
        format!("--arch {name:?}: want one of {}", names.join(", "))
    })?;
    let arch = (backend.make)();
    let arch: &dyn Arch = arch.as_ref();
    let image = Image { base, bytes: &bytes };
    let entry = cli.entry.map_or(entry, |a| a.0);
    if u64::from(base) + bytes.len() as u64 > u64::from(u32::MAX) - 16 {
        eprintln!("rrt-dis: warning: the image runs to the top of the address space; its last bytes are not analysed");
    }
    if !image.contains(entry) {
        eprintln!(
            "rrt-dis: warning: the entry 0x{entry:08x} is outside the image (0x{base:08x}..0x{:08x})",
            image.end()
        );
    }
    let gp = cli.gp.map(|a| a.0).or(exe_gp).or_else(|| find_gp(arch, image, entry));
    let program = Program::analyze(arch, image, entry, gp, &symbols);
    let mut listing = Listing::new(&program, &symbols);
    listing.addresses = !cli.no_addresses;

    if cli.stats {
        stats(&program, &listing);
    }
    if cli.functions {
        for (&start, f) in &program.funcs {
            if symbols.at(start).is_none() {
                let confidence = if f.found.is_certain() { Confidence::Likely } else { Confidence::Guess };
                let sym = Symbol {
                    kind: Kind::Func,
                    size: Some(f.end - f.start),
                    confidence: Some(confidence),
                    note: format!("found by {:?}", f.found).to_lowercase(),
                    ..Symbol::new(format!("func_{start:08x}"), start)
                };
                println!("{}", sym.to_line());
            }
        }
    }
    if cli.strings {
        for (addr, (s, sites)) in program.strings(4) {
            println!("0x{addr:08x} {s:?}  # {} ref{}", sites.len(), if sites.len() == 1 { "" } else { "s" });
        }
    }
    let func_at = |f: &str| -> Result<u32, String> {
        let addr = symbols
            .addr(f)
            .or_else(|| Addr::from_arg(f).ok().map(|a| a.0))
            .ok_or_else(|| format!("no symbol {f:?}"))?;
        match program.funcs.get(&addr) {
            Some(_) => Ok(addr),
            None => program.func_of(addr).map(|g| g.start).ok_or_else(|| format!("no function at 0x{addr:08x}")),
        }
    };
    for f in &cli.func {
        print!("{}", listing.function(func_at(f)?).unwrap_or_default());
    }
    if !cli.pseudo.is_empty() || cli.pseudo_all {
        let pseudo = Pseudo::new(&listing);
        for f in &cli.pseudo {
            print!("{}", pseudo.function(func_at(f)?).unwrap_or_default());
        }
        if cli.pseudo_all {
            for &start in program.funcs.keys() {
                print!("{}", pseudo.function(start).unwrap_or_default());
            }
            println!("// {} functions decompiled", program.funcs.len());
        }
    }
    match (cli.from, cli.to) {
        (Some(from), Some(to)) => print!("{}", listing.range(from.0, to.0)),
        (Some(from), None) => print!("{}", listing.range(from.0, from.0.saturating_add(0x100))),
        (None, Some(_)) => return Err("--to needs --from".into()),
        (None, None) => {}
    }
    if !listing_asked {
        print!("{}", listing.range(image.base, image.end()));
    }
    Ok(())
}

fn stats<A: Arch + ?Sized>(p: &Program<'_, A>, l: &Listing<'_, '_, A>) {
    let mut by: std::collections::BTreeMap<Found, usize> = Default::default();
    for f in p.funcs.values() {
        *by.entry(f.found).or_default() += 1;
    }
    let found: Vec<String> = by.iter().map(|(k, n)| format!("{} {n}", format!("{k:?}").to_lowercase())).collect();
    println!("arch         {}", p.arch.name());
    println!("image        0x{:08x}..0x{:08x}", p.image.base, p.image.end());
    let regions: Vec<String> = p.code.iter().map(|(s, e)| format!("0x{s:08x}..0x{e:08x}")).collect();
    println!("code         {}, {} words unowned", regions.join(" "), p.unowned_words());
    println!("gp           {}", p.gp.map_or("unknown".into(), |g| format!("0x{g:08x}")));
    println!("functions    {} ({})", p.funcs.len(), found.join(", "));
    println!("jump tables  {}", p.jump_tables.len());
    let resolved = p.indirect.values().filter(|s| p.slots.get(s).is_some_and(|t| t.len() == 1)).count();
    let indirect: u32 = p.funcs.values().map(|f| f.indirect_calls).sum();
    println!("indirect     {indirect} calls, {resolved} resolved through interface slots");
    let mut worst: Vec<(u32, u32)> =
        p.funcs.values().filter(|f| f.unresolved_jumps > 0).map(|f| (f.unresolved_jumps, f.start)).collect();
    worst.sort_by(|a, b| b.cmp(a));
    let most: Vec<String> = worst.iter().take(3).map(|(n, a)| format!("{n} in {}", l.name(*a))).collect();
    let total: u32 = worst.iter().map(|w| w.0).sum();
    match most.is_empty() {
        true => println!("unresolved   0 jumps"),
        false => {
            println!("unresolved   {total} jumps ({}{})", most.join(", "), if worst.len() > 3 { ", ..." } else { "" })
        }
    }
    println!("strings      {}", p.strings(4).len());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_refuses_a_file_changed_since_it_was_read_and_keeps_permissions() {
        let dir = std::env::temp_dir().join(format!("rrt-dis-save-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("game.txt");
        let read = "0x80010000 main func\n";
        std::fs::write(&path, read).unwrap();
        let mut symbols = Symbols::parse(read).unwrap();
        symbols.insert("helper", 0x8001_0020);

        // Someone saved the file in between: it is left as they wrote it.
        std::fs::write(&path, "0x80010000 main func # (sure)\n").unwrap();
        let err = save(&path, &symbols, Some(read)).unwrap_err();
        assert!(err.contains("changed since it was read"), "{err}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "0x80010000 main func # (sure)\n");
        assert!(!dir.join(".game.txt.rrt-dis-tmp").exists());

        // A file that appeared when none was read is not overwritten either.
        let fresh = dir.join("new.txt");
        std::fs::write(&fresh, "# mine\n").unwrap();
        assert!(save(&fresh, &symbols, None).is_err());

        std::fs::write(&path, read).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();
        }
        save(&path, &symbols, Some(read)).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "0x80010000 main func\n0x80010020 helper\n");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o640);
        }
        std::fs::remove_dir_all(&dir).ok();
    }
}
