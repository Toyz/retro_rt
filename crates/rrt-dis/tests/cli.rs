//! The `rrt-dis` command on a small PS-X EXE: stats, a function's listing,
//! and marking names in a symbol file.

use std::path::PathBuf;
use std::process::Command;

/// A PS-X EXE at 0x80010000: main loads gp, calls f with a string, and
/// returns; f returns.
fn exe() -> Vec<u8> {
    let words: [u32; 16] = [
        0x3c1c_8001, // 80010000 lui gp, 0x8001
        0x279c_0100, //          addiu gp, gp, 0x100
        0x3c04_8001, //          lui a0, 0x8001
        0x0c00_4008, //          jal 0x80010020
        0x2484_0030, //          addiu a0, a0, 0x30
        0x03e0_0008, //          jr ra
        0,
        0,
        0x03e0_0008, // 80010020 f: jr ra
        0,
        0,
        0,
        u32::from_le_bytes(*b"hell"), // 80010030
        u32::from_le_bytes([b'o', 0, 0, 0]),
        0,
        0,
    ];
    let mut data = vec![0u8; 0x800];
    data[..8].copy_from_slice(b"PS-X EXE");
    data[0x10..0x14].copy_from_slice(&0x8001_0000u32.to_le_bytes());
    data[0x18..0x1c].copy_from_slice(&0x8001_0000u32.to_le_bytes());
    data[0x1c..0x20].copy_from_slice(&(words.len() as u32 * 4).to_le_bytes());
    data.extend(words.iter().flat_map(|w| w.to_le_bytes()));
    data
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rrt-dis-cli-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn run(args: &[&str]) -> (bool, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_rrt-dis")).args(args).output().unwrap();
    (out.status.success(), String::from_utf8(out.stdout).unwrap(), String::from_utf8(out.stderr).unwrap())
}

#[test]
fn stats_listing_and_marking() {
    let dir = scratch("all");
    let exe_path = dir.join("GAME.EXE");
    std::fs::write(&exe_path, exe()).unwrap();
    let syms = dir.join("game.txt");
    std::fs::write(&syms, "# game symbols\n0x80010000 main func # (sure) the entry\n").unwrap();
    let (exe_s, syms_s) = (exe_path.to_str().unwrap(), syms.to_str().unwrap());

    let (ok, out, err) = run(&[exe_s, "-s", syms_s, "--stats"]);
    assert!(ok, "{err}");
    assert!(out.contains("arch         R3000A"), "{out}");
    assert!(out.contains("gp           0x80010100"), "{out}");
    assert!(out.contains("functions    2 (entry 1, call 1)"), "{out}");
    assert!(out.contains("strings      1"), "{out}");

    let (ok, out, err) = run(&[exe_s, "-s", syms_s, "-f", "main"]);
    assert!(ok, "{err}");
    assert!(out.starts_with("main:"), "{out}");
    assert!(out.contains("jal     func_80010020"), "{out}");
    assert!(out.contains("&str_80010030 \"hello\""), "{out}");

    let (ok, out, err) = run(&[exe_s, "-s", syms_s, "--mark", "greet=0x80010020:func:likely"]);
    assert!(ok, "{err}");
    assert_eq!(out, "0x80010020 greet func # (likely)\n");
    assert_eq!(
        std::fs::read_to_string(&syms).unwrap(),
        "# game symbols\n0x80010000 main func # (sure) the entry\n0x80010020 greet func # (likely)\n",
        "the file keeps its lines and gains one"
    );
    let (ok, out, _) = run(&[exe_s, "-s", syms_s, "-f", "main", "--no-addresses"]);
    assert!(ok);
    assert!(out.contains("  jal     greet"), "the new name is used: {out}");

    let (ok, _, err) = run(&[exe_s, "-s", syms_s, "--mark", "main=0x80010004"]);
    assert!(!ok && err.contains("main already names 0x80010000"), "{err}");
    let (ok, _, err) = run(&[exe_s, "-f", "nothing"]);
    assert!(!ok && err.contains("no symbol \"nothing\""), "{err}");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn raw_bytes_need_a_base() {
    let dir = scratch("raw");
    let raw = dir.join("bios.bin");
    std::fs::write(&raw, [0x08, 0x00, 0xe0, 0x03, 0, 0, 0, 0]).unwrap();
    let raw_s = raw.to_str().unwrap();
    let (ok, _, err) = run(&[raw_s, "--stats"]);
    assert!(!ok && err.contains("give --base"), "{err}");
    let (ok, out, err) = run(&[raw_s, "--base", "bfc00000"]);
    assert!(ok, "{err}");
    assert!(out.contains("bfc00000  03e00008  jr      ra"), "{out}");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn marking_keeps_what_it_does_not_say_and_guards_sure_names() {
    let dir = scratch("guard");
    let syms = dir.join("game.txt");
    let original = "0x80010000 main func size=0x20 # (sure) the entry\n0x80010020 helper func # (likely) maybe\n";
    std::fs::write(&syms, original).unwrap();
    let s = syms.to_str().unwrap();
    let mark = |args: &[&str]| run(&[&["x", "-s", s], args].concat());

    // A sure name is not renamed by accident, a name not made less sure,
    // and a bad mark anywhere in the list leaves the file untouched.
    let (ok, _, err) = mark(&["--mark", "start=0x80010000"]);
    assert!(!ok && err.contains("is main, marked sure; --force"), "{err}");
    let (ok, _, err) = mark(&["--mark", "helper=0x80010020:func:guess"]);
    assert!(!ok && err.contains("--force to make it less sure"), "{err}");
    let (ok, _, err) = mark(&["--mark", "fine=0x80010040", "--mark", "bad#name=0x80010044"]);
    assert!(!ok && err.contains("without `#`"), "{err}");
    let (ok, _, err) = mark(&["--mark", "x=0x80010048:func:sure:extra"]);
    assert!(!ok && err.contains("want NAME=ADDR"), "{err}");
    assert_eq!(std::fs::read_to_string(&syms).unwrap(), original, "nothing written");

    // Renaming keeps the kind, size, confidence and note.
    let (ok, out, err) = mark(&["--mark", "help_draw=0x80010020"]);
    assert!(ok, "{err}");
    assert_eq!(out, "helper renamed to help_draw\n0x80010020 help_draw func # (likely) maybe\n");
    let (ok, out, err) = mark(&["--mark", "start=0x80010000", "--force"]);
    assert!(ok, "{err}");
    assert!(out.ends_with("0x80010000 start func size=0x20 # (sure) the entry\n"), "{out}");
    assert_eq!(
        std::fs::read_to_string(&syms).unwrap(),
        "0x80010000 start func size=0x20 # (sure) the entry\n0x80010020 help_draw func # (likely) maybe\n"
    );
    assert!(!dir.join(".game.txt.rrt-dis-tmp").exists(), "no temporary left behind");
    std::fs::remove_dir_all(&dir).ok();
}
