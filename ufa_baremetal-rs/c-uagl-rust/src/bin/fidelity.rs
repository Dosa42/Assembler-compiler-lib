use c_uagl_converter::fidelity::report_file;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<_> = std::env::args().skip(1).take(3).collect();
    let [c_path, uagl_path, report_path] = args.as_slice() else {
        eprintln!("Usage: fidelity <c_path> <uagl_path> <report_path>");
        return ExitCode::FAILURE;
    };
    match report_file(c_path, uagl_path, report_path) {
        Ok(report) => {
            print!("{report}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
