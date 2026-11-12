//! Compile a single .uagl/.uaglbr file to assembly on stdout:
//! `cargo run --example run_one -- path/to/file.uagl [source_name]`
use uagl_aarch64_assembler::assemble_uagl_str;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("usage: run_one <path.uagl> [source_name]");
        std::process::exit(2);
    }
    let path = &args[1];
    let source_name = args.get(2).cloned().unwrap_or_else(|| path.clone());
    let src = std::fs::read_to_string(path).expect("read source file");
    let asm = assemble_uagl_str(&src, &source_name);
    print!("{}", asm);
}
