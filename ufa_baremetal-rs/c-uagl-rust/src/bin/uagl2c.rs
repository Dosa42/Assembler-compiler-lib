use c_uagl_converter::uagl2c::convert_file;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<_> = std::env::args_os().skip(1).take(2).collect();
    let [input, output] = args.as_slice() else {
        eprintln!("Usage: uagl2c <input.uagl> <output.c>");
        return ExitCode::FAILURE;
    };
    match convert_file(input, output) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
