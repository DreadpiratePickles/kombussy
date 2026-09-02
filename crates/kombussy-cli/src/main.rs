//! `kombussy` — convert a font between sfnt, WOFF and WOFF2 on the command line.
//!
//! Deliberately dependency-free: the argument surface is four flags, and a
//! parser crate would outweigh the code it replaced.

use kombussy_core::{convert, decode, detect, Format};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::ExitCode;

const USAGE: &str = "\
kombussy — OpenType/WOFF/WOFF2 converter

USAGE:
    kombussy --to <format> [--output <file>] [input]
    kombussy --info [input]

ARGS:
    input               Font to read. Reads stdin when omitted or '-'.

OPTIONS:
    -t, --to <format>   Target container: ttf, otf, woff, woff2
    -o, --output <file> Write here. Writes stdout when omitted.
    -i, --info          Print the detected format and table listing, convert nothing.
    -h, --help          Show this message.
";

struct Args {
    target: Option<String>,
    output: Option<PathBuf>,
    input: Option<PathBuf>,
    info: bool,
}

fn parse_args() -> Result<Args, String> {
    let mut args = Args {
        target: None,
        output: None,
        input: None,
        info: false,
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                print!("{USAGE}");
                std::process::exit(0);
            }
            "-i" | "--info" => args.info = true,
            "-t" | "--to" => args.target = Some(it.next().ok_or("--to needs a format")?),
            "-o" | "--output" => {
                args.output = Some(PathBuf::from(it.next().ok_or("--output needs a path")?))
            }
            "-" => args.input = None,
            other if other.starts_with('-') => return Err(format!("unknown option '{other}'")),
            other => args.input = Some(PathBuf::from(other)),
        }
    }
    if args.target.is_none() && !args.info {
        return Err("nothing to do: pass --to <format> or --info".into());
    }
    Ok(args)
}

fn target_format(name: &str) -> Result<Format, String> {
    match name {
        "ttf" | "otf" | "sfnt" => Ok(Format::Sfnt),
        "woff" | "woff1" => Ok(Format::Woff1),
        "woff2" => Ok(Format::Woff2),
        other => Err(format!(
            "unknown target format '{other}' (expected ttf, otf, woff or woff2)"
        )),
    }
}

fn read_input(path: &Option<PathBuf>) -> Result<Vec<u8>, String> {
    match path {
        Some(p) => std::fs::read(p).map_err(|e| format!("cannot read {}: {e}", p.display())),
        None => {
            let mut buf = Vec::new();
            std::io::stdin()
                .read_to_end(&mut buf)
                .map_err(|e| format!("cannot read stdin: {e}"))?;
            Ok(buf)
        }
    }
}

fn run() -> Result<(), String> {
    let args = parse_args()?;
    let input = read_input(&args.input)?;

    if args.info {
        let format = detect(&input).map_err(|e| e.to_string())?;
        let font = decode(&input).map_err(|e| e.to_string())?;
        println!("container : {format:?}");
        println!("flavor    : 0x{:08x}", font.flavor);
        println!("tables    : {}", font.tables.len());
        let mut rows: Vec<_> = font.tables.iter().collect();
        rows.sort_by_key(|t| t.tag);
        for t in rows {
            println!(
                "  {}  {:>9} bytes",
                String::from_utf8_lossy(&t.tag),
                t.data.len()
            );
        }
        if args.target.is_none() {
            return Ok(());
        }
    }

    let target = target_format(args.target.as_deref().unwrap_or("woff2"))?;
    let output = convert(&input, target).map_err(|e| e.to_string())?;

    match &args.output {
        Some(path) => std::fs::write(path, &output)
            .map_err(|e| format!("cannot write {}: {e}", path.display()))?,
        None => std::io::stdout()
            .write_all(&output)
            .map_err(|e| format!("cannot write stdout: {e}"))?,
    }
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("kombussy: {message}");
            ExitCode::FAILURE
        }
    }
}
