//! tesserac — the Tessera compiler driver.
//!
//! Subcommands:
//!   tesserac lex   <file.ter>            print the token stream (line:col)
//!   tesserac parse <file.ter>            print the AST
//!   tesserac check <file.ter>            type-check; print the typed IR
//!   tesserac build <file.ter> [-o out]   emit C and compile to a native exe (clang)
//!   tesserac run   <file.ter>            build to a temp exe and execute it

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use tessera_backend_c::emit_c;
use tessera_check::check;
use tessera_diag::SourceFile;
use tessera_lexer::{tokenize, Tok};
use tessera_parser::parse;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 3 {
        eprintln!("usage: tesserac <lex|parse|check|build|run> <file.ter> [-o out]");
        std::process::exit(2);
    }
    match args[1].as_str() {
        "lex" => run_lex(&args[2]),
        "parse" => run_parse(&args[2]),
        "check" => run_check(&args[2]),
        "build" => {
            let mut file = String::new();
            let mut out: Option<String> = None;
            let mut tier: i64 = 1;
            let mut i = 2;
            while i < args.len() {
                match args[i].as_str() {
                    "-o" => {
                        out = args.get(i + 1).cloned();
                        i += 2;
                    }
                    "--tier" => {
                        tier = tier_from(args.get(i + 1).map(|s| s.as_str()).unwrap_or("a"));
                        i += 2;
                    }
                    other => {
                        if !other.starts_with('-') && file.is_empty() {
                            file = other.to_string();
                        }
                        i += 1;
                    }
                }
            }
            if file.is_empty() {
                eprintln!("usage: tesserac build <file.ter> [-o out] [--tier a|b|c]");
                std::process::exit(2);
            }
            run_build(&file, out, tier);
        }
        "run" => run_exec(&args[2]),
        other => {
            eprintln!("unknown subcommand `{other}`");
            std::process::exit(2);
        }
    }
}

fn split_out(rest: &[String]) -> (String, Option<String>) {
    let mut file = None;
    let mut out = None;
    let mut i = 0;
    while i < rest.len() {
        if rest[i] == "-o" {
            out = rest.get(i + 1).cloned();
            i += 2;
        } else {
            file = Some(rest[i].clone());
            i += 1;
        }
    }
    (file.expect("missing input file"), out)
}

fn read(path: &str) -> String {
    fs::read_to_string(path).unwrap_or_else(|e| {
        eprintln!("error: cannot read {path}: {e}");
        std::process::exit(2);
    })
}

fn render(sf: &SourceFile, code: &str, span: (usize, usize), message: &str, help: &str) -> String {
    sf.render(code, message, span, "", help)
}

fn run_lex(path: &str) {
    let src = read(path);
    let sf = SourceFile::new(path, &src);
    let (toks, errs) = tokenize(&src);
    if !errs.is_empty() {
        for e in &errs {
            eprint!("{}", render(&sf, "E0001", (e.span.lo, e.span.hi), &e.message, e.help.as_deref().unwrap_or("")));
        }
        eprintln!("{} lexical error(s)", errs.len());
        std::process::exit(1);
    }
    for t in &toks {
        let (line, col) = sf.line_col(t.span.lo);
        let text = t.text(&src);
        let show = if t.tok == Tok::Eof { "<eof>".to_string() } else { text.to_string() };
        println!("{line:>4}:{col:<3} {:<9?} {}", t.tok, show);
    }
}

fn run_parse(path: &str) {
    let src = read(path);
    let sf = SourceFile::new(path, &src);
    print_front_diagnostics(&sf, &src);
    let (module, _errs) = parse(&src);
    println!("parsed {} item(s) OK", module.items.len());
    println!("{module:#?}");
}

fn run_check(path: &str) {
    let src = read(path);
    let sf = SourceFile::new(path, &src);
    print_front_diagnostics(&sf, &src);
    let (module, perrs) = parse(&src);
    if !perrs.is_empty() {
        for e in &perrs {
            eprint!("{}", render(&sf, "E0002", (e.span.lo, e.span.hi), &e.message, e.help.as_deref().unwrap_or("")));
        }
        eprintln!("{} parse error(s)", perrs.len());
        std::process::exit(1);
    }
    let (prog, cerrs) = check(&module);
    if !cerrs.is_empty() {
        for e in &cerrs {
            eprint!("{}", render(&sf, "E0003", (e.span.lo, e.span.hi), &e.message, e.help.as_deref().unwrap_or("")));
        }
        eprintln!("{} type error(s)", cerrs.len());
        std::process::exit(1);
    }
    println!("typechecked OK: {} function(s)", prog.funcs.len());
    println!("{prog:#?}");
}

/// Lex + parse + type-check. On failure, prints diagnostics and exits.
fn compile_program(path: &str) -> tessera_check::Program {
    let src = read(path);
    let sf = SourceFile::new(path, &src);

    let (_t, lex) = tokenize(&src);
    if !lex.is_empty() {
        for e in &lex {
            eprint!("{}", render(&sf, "E0001", (e.span.lo, e.span.hi), &e.message, e.help.as_deref().unwrap_or("")));
        }
        eprintln!("{} lexical error(s)", lex.len());
        std::process::exit(1);
    }

    let (module, perrs) = parse(&src);
    if !perrs.is_empty() {
        for e in &perrs {
            eprint!("{}", render(&sf, "E0002", (e.span.lo, e.span.hi), &e.message, e.help.as_deref().unwrap_or("")));
        }
        eprintln!("{} parse error(s)", perrs.len());
        std::process::exit(1);
    }

    let (prog, cerrs) = check(&module);
    if !cerrs.is_empty() {
        for e in &cerrs {
            eprint!("{}", render(&sf, "E0003", (e.span.lo, e.span.hi), &e.message, e.help.as_deref().unwrap_or("")));
        }
        eprintln!("{} type error(s)", cerrs.len());
        std::process::exit(1);
    }
    prog
}

/// Like `compile_program` but only prints (for `parse`/`check` where we don't exit
/// on diagnostics — we abort before reaching the next stage).
fn print_front_diagnostics(sf: &SourceFile, src: &str) {
    let (_t, lex) = tokenize(src);
    if !lex.is_empty() {
        for e in &lex {
            eprint!("{}", render(sf, "E0001", (e.span.lo, e.span.hi), &e.message, e.help.as_deref().unwrap_or("")));
        }
        eprintln!("{} lexical error(s); aborting", lex.len());
        std::process::exit(1);
    }
    let (_m, perrs) = parse(src);
    if !perrs.is_empty() {
        for e in &perrs {
            eprint!("{}", render(sf, "E0002", (e.span.lo, e.span.hi), &e.message, e.help.as_deref().unwrap_or("")));
        }
        eprintln!("{} parse error(s); aborting", perrs.len());
        std::process::exit(1);
    }
}

fn find_clang() -> Option<PathBuf> {
    if let Ok(p) = env::var("TESSERA_CLANG") {
        let p = PathBuf::from(p);
        if p.exists() {
            return Some(p);
        }
    }
    if Command::new("clang").arg("--version").output().map(|o| o.status.success()).unwrap_or(false) {
        return Some(PathBuf::from("clang"));
    }
    if let Ok(home) = env::var("USERPROFILE") {
        if let Some(p) = search_dir(&Path::new(&home).join("tools")) {
            return Some(p);
        }
    }
    if let Ok(pf) = env::var("ProgramFiles") {
        let cand = Path::new(&pf).join("LLVM").join("bin").join("clang.exe");
        if cand.exists() {
            return Some(cand);
        }
    }
    let cand = PathBuf::from(r"C:\Program Files\LLVM\bin\clang.exe");
    if cand.exists() {
        return Some(cand);
    }
    None
}

fn search_dir(dir: &Path) -> Option<PathBuf> {
    if let Ok(rd) = fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                let cand = p.join("bin").join("clang.exe");
                if cand.exists() {
                    return Some(cand);
                }
            }
        }
    }
    None
}

fn tier_from(s: &str) -> i64 {
    match s.to_ascii_lowercase().as_str() {
        "a" | "1" => 1,
        "b" | "2" => 2,
        "c" | "3" => 3,
        _ => 1,
    }
}

fn compile_c(cfile: &str, out: &str, tier: i64) {
    let clang = find_clang().unwrap_or_else(|| {
        eprintln!("error: clang not found. Set TESSERA_CLANG to clang.exe, or install LLVM.");
        std::process::exit(2);
    });
    let mut cmd = Command::new(&clang);
    cmd.arg("-O2").arg("-fwrapv");
    if tier >= 2 {
        cmd.arg("-mpopcnt");
    }
    if tier >= 3 {
        cmd.arg("-mavx2").arg("-mbmi2");
    }
    cmd.arg("-std=c11").arg("-w").arg(cfile).arg("-o").arg(out);
    let status = cmd.status();
    match status {
        Ok(s) if s.success() => {}
        Ok(s) => {
            eprintln!("error: clang exited with {s}");
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("error: failed to run {clang:?}: {e}");
            std::process::exit(1);
        }
    }
}

fn run_build(path: &str, out: Option<String>, tier: i64) {
    let prog = compile_program(path);
    let c = emit_c(&prog);
    let out = out.unwrap_or_else(|| default_out(path));
    let cfile = format!("{out}.c");
    fs::write(&cfile, &c).unwrap_or_else(|e| {
        eprintln!("error: cannot write {cfile}: {e}");
        std::process::exit(2);
    });
    compile_c(&cfile, &out, tier);
    println!("built {out} (C source: {cfile})");
}

fn run_exec(path: &str) {
    let prog = compile_program(path);
    let c = emit_c(&prog);
    let tmp = env::temp_dir();
    let base = tmp.join("tesserac_run");
    let cfile = base.with_extension("c");
    let exe = base.with_extension("exe");
    fs::write(&cfile, &c).unwrap_or_else(|e| {
        eprintln!("error: cannot write {cfile:?}: {e}");
        std::process::exit(2);
    });
    compile_c(cfile.to_str().unwrap(), exe.to_str().unwrap(), 1);
    let status = Command::new(&exe).status();
    match status {
        Ok(s) => std::process::exit(s.code().unwrap_or(0)),
        Err(e) => {
            eprintln!("error: failed to run {exe:?}: {e}");
            std::process::exit(1);
        }
    }
}

fn default_out(path: &str) -> String {
    let p = Path::new(path);
    let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("a");
    let parent = p.parent().filter(|d| !d.as_os_str().is_empty());
    let mut out = parent.map(|d| d.to_path_buf()).unwrap_or_else(|| PathBuf::from("."));
    out.push(format!("{stem}.exe"));
    out.to_string_lossy().into_owned()
}
