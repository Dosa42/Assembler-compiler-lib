use c_uagl_converter::linux_converter::{
    convert_source, gap_manifest, needs_ai, normalize_newlines, VERSION,
};
use c_uagl_converter::validator::{format_result, UaglValidator};
use chrono::Local;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::{env, fs, io, thread};

const SKIP_DIRS: &[&str] = &[
    "build",
    ".git",
    "Documentation",
    "tools",
    "scripts",
    "samples",
    "usr",
    "virt",
    "sound",
];

struct Args {
    linux_root: PathBuf,
    output_dir: PathBuf,
    subsystems: Vec<PathBuf>,
    workers: i64,
    validate: bool,
    single: Option<PathBuf>,
}

fn path_text(path: &Path) -> String {
    let value = path.to_string_lossy();
    let prefix = if value.starts_with("//") && !value.starts_with("///") {
        "//"
    } else if value.starts_with('/') {
        "/"
    } else {
        ""
    };
    let parts = value
        .split('/')
        .filter(|part| !part.is_empty() && *part != ".")
        .collect::<Vec<_>>()
        .join("/");
    if prefix.is_empty() && parts.is_empty() {
        ".".to_owned()
    } else {
        format!("{prefix}{parts}")
    }
}

fn display_name(path: &Path) -> String {
    path.file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned()
}

fn usage(name: &str) -> String {
    format!("usage: {name} [-h] [--subsystems SUBSYSTEMS [SUBSYSTEMS ...]] [--workers WORKERS]\n{indent}[--no-validate] [--single FILE] linux_root output_dir", indent = " ".repeat(name.len() + 8))
}

fn help(name: &str) {
    println!("{}\n\nLinux kernel → UAGL v3 converter\n\npositional arguments:\n  linux_root            Linux kernel source root\n  output_dir            Output directory\n\noptions:\n  -h, --help            show this help message and exit\n  --subsystems SUBSYSTEMS [SUBSYSTEMS ...]\n                        Subsystems to convert (e.g. kernel/sched arch/arm64)\n  --workers WORKERS\n  --no-validate\n  --single FILE         Convert single file (debug)", usage(name));
}

fn looks_like_option(value: &str) -> bool {
    value.starts_with('-') && value != "-" && value.parse::<f64>().is_err()
}

fn option_name(name: &str) -> Result<&str, String> {
    if name == "-h" {
        return Ok("--help");
    }
    let options = [
        "--help",
        "--subsystems",
        "--workers",
        "--no-validate",
        "--single",
    ];
    if options.contains(&name) {
        return Ok(name);
    }
    let mut matches = options
        .into_iter()
        .filter(|option| name.starts_with("--") && option.starts_with(name));
    match (matches.next(), matches.next()) {
        (Some(option), None) => Ok(option),
        (Some(first), Some(second)) => Err(format!(
            "ambiguous option: {name} could match {first}, {second}"
        )),
        _ => Err(format!("unrecognized arguments: {name}")),
    }
}

fn parse_args(name: &str, args: &[String]) -> Result<Option<Args>, String> {
    let mut positional = Vec::new();
    let mut subsystems = Vec::new();
    let mut workers = 6;
    let mut validate = true;
    let mut single = None;
    let mut parse_options = true;
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        index += 1;
        if parse_options && arg == "--" {
            parse_options = false;
            continue;
        }
        if !parse_options || !looks_like_option(arg) {
            positional.push(PathBuf::from(path_text(Path::new(arg))));
            continue;
        }
        let (flag, inline) = arg
            .split_once('=')
            .map_or((arg.as_str(), None), |(flag, value)| (flag, Some(value)));
        let option = option_name(flag)?;
        if option == "--help" || option == "--no-validate" {
            if let Some(value) = inline {
                return Err(format!(
                    "argument {option}: ignored explicit argument '{value}'"
                ));
            }
            if option == "--help" {
                help(name);
                return Ok(None);
            }
            validate = false;
        } else if option == "--subsystems" {
            let start = index;
            if inline.is_none() {
                while args
                    .get(index)
                    .is_some_and(|value| !looks_like_option(value))
                {
                    index += 1;
                }
            }
            if inline.is_none() && start == index {
                return Err("argument --subsystems: expected at least one argument".to_owned());
            }
            subsystems = inline
                .into_iter()
                .chain(args[start..index].iter().map(String::as_str))
                .map(|value| PathBuf::from(path_text(Path::new(value))))
                .collect();
        } else {
            let value = if let Some(value) = inline {
                value
            } else {
                let value = args
                    .get(index)
                    .filter(|value| !looks_like_option(value))
                    .ok_or_else(|| format!("argument {option}: expected one argument"))?;
                index += 1;
                value
            };
            if option == "--workers" {
                workers = value
                    .parse()
                    .map_err(|_| format!("argument --workers: invalid int value: '{value}'"))?;
            } else {
                single = Some(PathBuf::from(path_text(Path::new(value))));
            }
        }
    }

    match positional.len() {
        0 => return Err("the following arguments are required: linux_root, output_dir".to_owned()),
        1 => return Err("the following arguments are required: output_dir".to_owned()),
        2 => {}
        _ => {
            let extras = positional[2..]
                .iter()
                .map(|path| path_text(path))
                .collect::<Vec<_>>()
                .join(" ");
            return Err(format!("unrecognized arguments: {extras}"));
        }
    }
    let output_dir = positional.pop().expect("two arguments");
    let linux_root = positional.pop().expect("source argument");
    Ok(Some(Args {
        linux_root,
        output_dir,
        subsystems,
        workers,
        validate,
        single,
    }))
}

fn report_error(error: &io::Error) -> String {
    let kind = match error.kind() {
        io::ErrorKind::NotFound => "FileNotFoundError",
        io::ErrorKind::PermissionDenied => "PermissionError",
        io::ErrorKind::AlreadyExists => "FileExistsError",
        io::ErrorKind::NotADirectory => "NotADirectoryError",
        io::ErrorKind::IsADirectory => "IsADirectoryError",
        _ => "OSError",
    };
    format!("{kind}: {error}")
}

#[derive(Default)]
struct FileResult {
    source: PathBuf,
    output: Option<PathBuf>,
    error: Option<String>,
    lines_in: usize,
    lines_out: usize,
    has_module: bool,
    warnings: Vec<String>,
    gaps: Option<usize>,
}

impl FileResult {
    fn json(&self) -> Value {
        let mut value = json!({
            "src": path_text(&self.source),
            "status": if self.output.is_some() { "converted" } else { "error" },
            "output": self.output.as_deref().map(path_text),
            "error": self.error,
            "lines_in": self.lines_in,
            "lines_out": self.lines_out,
            "has_module": self.has_module,
            "warnings": self.warnings,
        });
        if let Some(count) = self.gaps {
            value["gaps"] = json!(count);
        }
        value
    }
}

struct SavedConversion {
    gaps: usize,
    needs_ai: usize,
}

fn write_conversion(
    source: &Path,
    output: &Path,
    result: &mut FileResult,
) -> io::Result<SavedConversion> {
    let input = fs::read(source)?;
    let input = String::from_utf8_lossy(&input);
    let input = normalize_newlines(&input);
    result.lines_in = input.matches('\n').count();
    let source_text = path_text(source);
    let output_text = path_text(output);
    let converted = convert_source(&input, &source_text)?;
    result.has_module = converted.has_module;
    result.warnings = converted.warnings;
    result.lines_out = converted.lines_out;

    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(output, &converted.uagl)?;
    fs::write(
        output.with_extension("gaps.json"),
        gap_manifest(&source_text, &output_text, &converted.gaps),
    )?;
    let gaps = converted.gaps.len();
    result.gaps = Some(gaps);
    result.output = Some(output.to_owned());
    Ok(SavedConversion {
        gaps,
        needs_ai: needs_ai(&converted.gaps),
    })
}

fn convert_file(source: &Path, output: &Path) -> FileResult {
    let mut result = FileResult {
        source: source.to_owned(),
        ..FileResult::default()
    };
    if let Err(error) = write_conversion(source, output, &mut result) {
        result.error = Some(report_error(&error));
    }
    result
}

fn skipped(path: &Path) -> bool {
    path.components().any(|part| {
        part.as_os_str()
            .to_str()
            .is_some_and(|name| SKIP_DIRS.contains(&name))
    })
}

fn collect_files(path: &Path, files: &mut Vec<PathBuf>) -> io::Result<()> {
    if skipped(path) {
        return Ok(());
    }
    let entries = match fs::read_dir(path) {
        Ok(entries) => entries,
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::NotFound
                    | io::ErrorKind::PermissionDenied
                    | io::ErrorKind::NotADirectory
            ) =>
        {
            return Ok(())
        }
        Err(error) => return Err(error),
    };
    for entry in entries {
        let entry = entry?;
        let child = entry.path();
        let kind = entry.file_type()?;
        if kind.is_dir() {
            collect_files(&child, files)?;
        } else if !skipped(&child)
            && matches!(
                child.extension().and_then(|extension| extension.to_str()),
                Some("c" | "h")
            )
            && (kind.is_file() || kind.is_symlink() && child.is_file())
        {
            files.push(child);
        }
    }
    Ok(())
}

fn discover_files(args: &Args) -> io::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    if args.subsystems.is_empty() {
        collect_files(&args.linux_root, &mut files)?;
        files.sort();
    } else {
        for subsystem in &args.subsystems {
            let mut group = Vec::new();
            collect_files(&args.linux_root.join(subsystem), &mut group)?;
            group.sort();
            files.extend(group);
        }
    }
    Ok(files)
}

fn batch_output(source: &Path, root: &Path, output: &Path) -> PathBuf {
    let relative = source
        .strip_prefix(root)
        .unwrap_or_else(|_| Path::new(source.file_name().unwrap_or_default()));
    output.join(relative.with_extension("uagl"))
}

fn convert_batch(
    files: &[PathBuf],
    args: &Args,
    workers: usize,
) -> (Vec<FileResult>, usize, usize) {
    // Keep writes to a shared destination in discovery order, even if two
    // subsystems overlap or a .c and a .h share a basename.
    let mut destinations = BTreeMap::<PathBuf, Vec<usize>>::new();
    for (index, source) in files.iter().enumerate() {
        destinations
            .entry(batch_output(source, &args.linux_root, &args.output_dir))
            .or_default()
            .push(index);
    }
    let jobs = destinations.into_iter().collect::<Vec<_>>();
    let next_job = AtomicUsize::new(0);
    let mut results = Vec::with_capacity(files.len());
    let mut pending = HashMap::new();
    let mut converted = 0;
    let mut errors = 0;
    let total = files.len();

    thread::scope(|scope| {
        let (send, receive) = mpsc::channel();
        for _ in 0..workers.min(jobs.len()) {
            let send = send.clone();
            let jobs = &jobs;
            let next_job = &next_job;
            scope.spawn(move || {
                while let Some((output, indices)) =
                    jobs.get(next_job.fetch_add(1, Ordering::Relaxed))
                {
                    for &index in indices {
                        let result = convert_file(&files[index], output);
                        if send.send((index, result)).is_err() {
                            return;
                        }
                    }
                }
            });
        }
        drop(send);
        for (index, result) in receive {
            pending.insert(index, result);
            while let Some(result) = pending.remove(&results.len()) {
                if let Some(error) = &result.error {
                    errors += 1;
                    let detail: String = error.chars().take(60).collect();
                    println!("  ❌ {}: {detail}", display_name(&result.source));
                } else {
                    converted += 1;
                    if converted % 20 == 0 {
                        println!("  [{converted}/{total}] converted, {errors} errors");
                    }
                }
                results.push(result);
            }
        }
    });
    (results, converted, errors)
}

fn validate_batch(results: &[FileResult], workers: usize) -> (usize, usize) {
    let outputs: Vec<String> = results
        .iter()
        .filter_map(|result| result.output.as_deref().map(path_text))
        .collect();
    let next = AtomicUsize::new(0);
    let passed = AtomicUsize::new(0);
    let validator = UaglValidator::new();
    thread::scope(|scope| {
        for _ in 0..workers.min(outputs.len()) {
            scope.spawn(|| {
                while let Some(path) = outputs.get(next.fetch_add(1, Ordering::Relaxed)) {
                    if validator.validate(path).is_ok_and(|result| result.passed) {
                        passed.fetch_add(1, Ordering::Relaxed);
                    }
                }
            });
        }
    });
    let passed = passed.into_inner();
    (passed, outputs.len() - passed)
}

fn pretty_ascii(value: &Value) -> String {
    let serialized = serde_json::to_string_pretty(value).expect("report contains only JSON values");
    let mut output = String::new();
    for c in serialized.chars() {
        if c <= '\u{7e}' {
            output.push(c);
        } else {
            for unit in c.encode_utf16(&mut [0; 2]) {
                write!(output, "\\u{unit:04x}").expect("writing to a string succeeds");
            }
        }
    }
    output
}

fn run_batch(args: &Args) -> io::Result<()> {
    let start = Local::now().naive_local();
    let separator = "═".repeat(60);
    println!("\n{separator}\n  Linux Kernel → UAGL v3 CONVERTER v{VERSION}\n{separator}");
    println!("  Source: {}", path_text(&args.linux_root));
    println!("  Output: {}", path_text(&args.output_dir));
    if !args.subsystems.is_empty() {
        let subsystems = args
            .subsystems
            .iter()
            .map(|path| path_text(path))
            .collect::<Vec<_>>()
            .join(", ");
        println!("  Subsystems: {subsystems}");
    }
    println!();

    let files = discover_files(args)?;
    let total = files.len();
    println!("[ PHASE 1 ] Discovered {total} files\n");
    if total == 0 {
        println!("ERROR: No files found.");
        return Ok(());
    }

    fs::create_dir_all(&args.output_dir)?;
    println!("[ PHASE 2 ] Converting...");
    let workers = usize::try_from(args.workers)
        .ok()
        .filter(|count| *count > 0)
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "max_workers must be greater than 0",
            )
        })?;
    let (results, converted, errors) = convert_batch(&files, args, workers);
    println!("\n  Conversion: {converted} OK, {errors} errors\n");

    let (val_pass, val_fail) = if args.validate {
        println!("[ PHASE 3 ] Validating...");
        let (passed, failed) = validate_batch(&results, workers);
        println!("  Validation: {passed} pass, {failed} fail\n");
        (passed, failed)
    } else {
        (0, 0)
    };
    let elapsed = (Local::now().naive_local() - start)
        .num_microseconds()
        .unwrap_or_default() as f64
        / 1_000_000.0;
    let elapsed_report = format!("{elapsed:.2}")
        .parse::<f64>()
        .expect("finite elapsed time");
    let modules = results.iter().filter(|result| result.has_module).count();
    let mut timestamp = start.format("%Y-%m-%dT%H:%M:%S").to_string();
    let micros = start.and_utc().timestamp_subsec_micros();
    if micros > 0 {
        write!(timestamp, ".{micros:06}").expect("writing to a string succeeds");
    }
    let report = json!({
        "version": VERSION,
        "timestamp": timestamp,
        "elapsed": elapsed_report,
        "source": path_text(&args.linux_root),
        "totals": {
            "files": total,
            "converted": converted,
            "errors": errors,
            "modules": modules,
            "lines_in": results.iter().map(|result| result.lines_in).sum::<usize>(),
            "lines_out": results.iter().map(|result| result.lines_out).sum::<usize>(),
            "val_pass": val_pass,
            "val_fail": val_fail,
        },
        "results": results.iter().map(FileResult::json).collect::<Vec<_>>(),
    });

    println!("{separator}\n  LINUX CONVERSION COMPLETE\n{separator}");
    println!("  Files:    {converted}/{total}");
    println!("  Modules:  {modules} (with module_init/exit)");
    println!("  Valid:    {val_pass}/{}", val_pass + val_fail);
    println!("  Time:     {elapsed:.1}s");
    let report_path = args.output_dir.join("LINUX_CONVERSION_REPORT.json");
    fs::write(&report_path, pretty_ascii(&report))?;
    println!("  Report:   {}", path_text(&report_path));
    Ok(())
}

fn run_single(args: &Args, source: &Path) -> io::Result<()> {
    let named = source.with_extension("uagl");
    let output = args.output_dir.join(named.file_name().unwrap_or_default());
    let mut result = FileResult::default();
    let saved = write_conversion(source, &output, &mut result)?;
    println!(
        "Gap manifest: {}  ({} entries, {} need AI)",
        path_text(&output.with_extension("gaps.json")),
        saved.gaps,
        saved.needs_ai
    );
    println!(
        "Output: {}  ({} lines)",
        path_text(&output),
        result.lines_out
    );
    if args.validate {
        let validation = UaglValidator::new().validate(&path_text(&output))?;
        println!("{}", format_result(&validation));
    }
    Ok(())
}

fn main() -> ExitCode {
    let mut arguments = env::args();
    let executable = arguments
        .next()
        .unwrap_or_else(|| "linux_converter".to_owned());
    let name = display_name(Path::new(&executable));
    let arguments = arguments.collect::<Vec<_>>();
    let args = match parse_args(&name, &arguments) {
        Ok(Some(args)) => args,
        Ok(None) => return ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{}\n{name}: error: {error}", usage(&name));
            return ExitCode::from(2);
        }
    };
    let result = if let Some(source) = &args.single {
        run_single(&args, source)
    } else {
        run_batch(&args)
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{}", report_error(&error));
            ExitCode::FAILURE
        }
    }
}
