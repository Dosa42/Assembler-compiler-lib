//! Inspect / verify a .uaglbr container: `cargo run --example decode_uaglbr -- path/to/file.uaglbr`
use uagl_aarch64_assembler::uaglbr::decode_uaglbr_bytes;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("usage: decode_uaglbr <path.uaglbr>");
        std::process::exit(2);
    }
    let data = std::fs::read(&args[1]).expect("read container file");
    match decode_uaglbr_bytes(&data) {
        Ok(bytes) => {
            println!("OK — {} bytes of restored UAGL source:", bytes.len());
            println!("{}", String::from_utf8_lossy(&bytes));
        }
        Err(e) => {
            eprintln!("REJECTED: {}", e);
            std::process::exit(1);
        }
    }
}
