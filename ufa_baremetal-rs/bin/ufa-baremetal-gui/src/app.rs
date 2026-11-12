//! The GUI application: a workbench around [`ufa_baremetal_core`] (and,
//! since this crate joined the same workspace, [`uagl_aarch64_assembler`]
//! too) — not just a viewer. Load a `.s` file or a directory (type a path,
//! click "Run", or drop a file/folder onto the window), see results
//! grouped by file with per-K-class filtering and a live PASS/FAIL/N-A
//! summary; cross-reference the bug-regression ledger in a second tab; or,
//! in the third "Assemble" tab, compile UAGL source straight to AArch64
//! assembly and immediately K-check the result, all in one process with no
//! subprocess or temp-file bridge between the two crates.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use eframe::egui;
use egui::{Color32, RichText};

use ufa_baremetal_core::codegen_audit::{self, AuditError};
use ufa_baremetal_core::k_invariants;
use ufa_baremetal_core::regression_map;
use ufa_baremetal_core::verdict::{Status, Verdict};

#[derive(PartialEq, Eq, Clone, Copy)]
enum Tab {
    Checks,
    Assemble,
    BugCoverage,
}

pub struct UfaApp {
    path_input: String,
    results: BTreeMap<String, Vec<Verdict>>,
    active_checks: HashSet<&'static str>,
    show_pass: bool,
    show_na: bool,
    status: String,
    error: Option<String>,
    tab: Tab,
    coverage_text: String,
    has_run: bool,

    // --- Assemble tab: uagl_aarch64_assembler is a workspace dependency
    // of this GUI now (a `path = "../../lib/uagl_aarch64_assembler"` crate,
    // not a subprocess), so compiling and checking both happen as ordinary
    // in-process function calls below.
    uagl_source_name: String,
    uagl_source: String,
    uagl_out_path: String,
    auto_check_generated: bool,
    generated_asm: Option<String>,
    assemble_error: Option<String>,
    assemble_status: String,
    generated_verdicts: Vec<Verdict>,
}

const DEFAULT_UAGL_SOURCE: &str = "△ main ○ VOID ═╗\n║ → 0 ◇\n╚═══ 1 ═══╝\n";

impl UfaApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        cc.egui_ctx.set_visuals(egui::Visuals::dark());

        UfaApp {
            path_input: String::new(),
            results: BTreeMap::new(),
            active_checks: k_invariants::names().into_iter().collect(),
            show_pass: true,
            show_na: false,
            status: "Enter a .s file or directory path and click Run — or drop one onto this window."
                .to_string(),
            error: None,
            tab: Tab::Checks,
            coverage_text: regression_map::coverage_report_string(),
            has_run: false,

            uagl_source_name: "input.uagl".to_string(),
            uagl_source: DEFAULT_UAGL_SOURCE.to_string(),
            uagl_out_path: String::new(),
            auto_check_generated: true,
            generated_asm: None,
            assemble_error: None,
            assemble_status: "Edit or drop a .uagl/.uaglbr file, then click Assemble.".to_string(),
            generated_verdicts: Vec::new(),
        }
    }

    fn run(&mut self, path: &Path) {
        self.error = None;
        self.has_run = true;

        if path.is_file() {
            match codegen_audit::audit_file(path) {
                Ok(verdicts) => {
                    let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("").to_string();
                    let fails = verdicts.iter().filter(|v| v.is_fail()).count();
                    self.status = if fails == 0 {
                        format!("Checked {name} — clean.")
                    } else {
                        format!("Checked {name} — {fails} FAIL(s).")
                    };
                    self.results = BTreeMap::from([(name, verdicts)]);
                }
                Err(e) => self.set_error(e),
            }
        } else if path.is_dir() {
            match codegen_audit::audit_directory(path, "*.s") {
                Ok(results) => {
                    let n = results.len();
                    let fails: usize = results.values().flatten().filter(|v| v.is_fail()).count();
                    let fail_files = results.values().filter(|vs| vs.iter().any(Verdict::is_fail)).count();
                    self.status = if fails == 0 {
                        format!("Checked {n} file(s) — all clean.")
                    } else {
                        format!("Checked {n} file(s) — {fails} FAIL(s) across {fail_files} file(s).")
                    };
                    self.results = results;
                }
                Err(e) => self.set_error(e),
            }
        } else {
            self.results.clear();
            self.error = Some(format!("Path not found: {}", path.display()));
        }
    }

    fn set_error(&mut self, e: AuditError) {
        self.results.clear();
        self.error = Some(e.to_string());
    }

    /// Loads a `.uagl`/`.uaglbr` file's content into the Assemble tab's
    /// source box (used by that tab's drag-and-drop). `.uaglbr` containers
    /// are decoded through the same path `assemble` itself uses, so what's
    /// shown in the editor after a drop is always plain UAGL text, never
    /// an encoded container.
    fn load_uagl_file(&mut self, path: &Path) {
        self.assemble_error = None;
        let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("input.uagl").to_string();
        match std::fs::read(path) {
            Ok(raw) => {
                let decoded = if name.ends_with(".uaglbr") {
                    uagl_aarch64_assembler::uaglbr::decode_uaglbr_bytes(&raw)
                        .map(|b| String::from_utf8_lossy(&b).into_owned())
                } else {
                    Ok(String::from_utf8_lossy(&raw).into_owned())
                };
                match decoded {
                    Ok(text) => {
                        self.uagl_source = text;
                        self.uagl_source_name =
                            if name.ends_with(".uaglbr") { name.trim_end_matches("br").to_string() } else { name };
                        self.assemble_status = format!("Loaded {}.", path.display());
                        self.generated_asm = None;
                        self.generated_verdicts.clear();
                    }
                    Err(e) => self.assemble_error = Some(format!("decoding {}: {e}", path.display())),
                }
            }
            Err(e) => self.assemble_error = Some(format!("reading {}: {e}", path.display())),
        }
    }

    /// Compiles the current source box via [`uagl_aarch64_assembler`] —
    /// an ordinary in-process call into a workspace-sibling crate, the
    /// same way `ufa-baremetal assemble` on the CLI does it — and, if
    /// `auto_check_generated` is on, immediately runs the K01-K06 checks
    /// against the result via [`codegen_audit::audit_text`] before it
    /// touches disk.
    fn assemble(&mut self) {
        self.assemble_error = None;
        self.generated_verdicts.clear();

        // The core codegen deliberately panics on some malformed-input
        // paths, mirroring the Python original's uncaught exceptions
        // (see uagl_aarch64_assembler::lib's doc comment); catch that here
        // so a bad paste in the editor shows a status message instead of
        // taking the whole GUI process down.
        let src = self.uagl_source.clone();
        let name = self.uagl_source_name.clone();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            uagl_aarch64_assembler::assemble_uagl_str(&src, &name)
        }));

        let asm = match result {
            Ok(asm) => asm,
            Err(_) => {
                self.assemble_error =
                    Some("compilation failed (malformed UAGL source) — see the source for the \
                          construct that triggered it"
                        .to_string());
                self.generated_asm = None;
                return;
            }
        };

        let lines = asm.matches('\n').count();
        self.generated_asm = Some(asm.clone());

        if self.auto_check_generated {
            match codegen_audit::audit_text(&asm, &self.uagl_source_name) {
                Ok(verdicts) => {
                    let fails = verdicts.iter().filter(|v| v.is_fail()).count();
                    self.assemble_status = if fails == 0 {
                        format!("Assembled {lines} line(s) — checked, clean.")
                    } else {
                        format!("Assembled {lines} line(s) — checked, {fails} FAIL(s).")
                    };
                    self.generated_verdicts = verdicts;
                }
                Err(e) => {
                    self.assemble_status = format!("Assembled {lines} line(s).");
                    self.assemble_error =
                        Some(format!("generated assembly failed its own parse-completeness preflight: {e}"));
                }
            }
        } else {
            self.assemble_status = format!("Assembled {lines} line(s) (not checked).");
        }
    }

    fn save_generated_asm(&mut self) {
        let Some(asm) = &self.generated_asm else { return };
        let path = PathBuf::from(self.uagl_out_path.trim());
        if path.as_os_str().is_empty() {
            self.assemble_error = Some("enter an output path first".to_string());
            return;
        }
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                if let Err(e) = std::fs::create_dir_all(parent) {
                    self.assemble_error = Some(format!("creating {}: {e}", parent.display()));
                    return;
                }
            }
        }
        match std::fs::write(&path, asm) {
            Ok(()) => self.assemble_status = format!("Saved to {}.", path.display()),
            Err(e) => self.assemble_error = Some(format!("writing {}: {e}", path.display())),
        }
    }

    fn copy_text(&self) -> String {
        let mut out = String::new();
        for verdicts in self.results.values() {
            for v in verdicts {
                if !self.active_checks.contains(v.k_class) {
                    continue;
                }
                if !self.passes_display_filter(v) {
                    continue;
                }
                out.push_str(&v.render());
                out.push('\n');
            }
        }
        out
    }

    fn passes_display_filter(&self, v: &Verdict) -> bool {
        match v.status {
            Status::Fail => true,
            Status::Pass => self.show_pass,
            Status::NotApplicable => self.show_na,
        }
    }
}

impl eframe::App for UfaApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // Native drag-and-drop: dropping a .s file or a folder onto the
        // window runs it immediately, the way a desktop tool should feel.
        // Raw input (including dropped files) lives on the Context, not
        // the Ui, so we reach it via `ui.ctx()`. Which tab is active
        // decides what a drop means: on the Assemble tab a dropped
        // .uagl/.uaglbr file loads into the source editor; everywhere
        // else it's treated as a .s file/directory to check, as before.
        let dropped_path: Option<PathBuf> =
            ui.ctx().input(|i| i.raw.dropped_files.iter().find_map(|f| f.path.clone()));
        if let Some(p) = dropped_path {
            if self.tab == Tab::Assemble {
                self.load_uagl_file(&p);
            } else {
                self.path_input = p.display().to_string();
                self.run(&p);
            }
        }

        // eframe 0.34+ / egui: `App::update(&Context, ...)` was replaced by
        // the required `App::ui(&mut Ui, ...)`, and `SidePanel`/
        // `TopBottomPanel` were merged into a single `Panel` type (same
        // `.top()`/`.left()`/etc. constructors, `width`-named builder
        // methods renamed to `size`-named ones). Every panel/window
        // `.show()` now takes the `&mut Ui` this method receives, in place
        // of the old `&Context`. Confirmed against a real build's compiler
        // output plus current egui source/docs — not guessed twice in a
        // row.
        egui::Panel::top("top").show(ui, |ui| {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.heading("UFA Bare-Metal K-Check");
                ui.add_space(12.0);
                ui.selectable_value(&mut self.tab, Tab::Checks, "Checks");
                ui.selectable_value(&mut self.tab, Tab::Assemble, "Assemble");
                ui.selectable_value(&mut self.tab, Tab::BugCoverage, "Bug Coverage");
            });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label("Path:");
                let edit = ui.add(
                    egui::TextEdit::singleline(&mut self.path_input)
                        .desired_width(560.0)
                        .hint_text(".s file or directory — or drop one on this window"),
                );
                let run_clicked = ui.button("Run").clicked();
                let enter_pressed = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if run_clicked || enter_pressed {
                    let p = PathBuf::from(self.path_input.trim());
                    self.run(&p);
                }
                if ui.button("Copy results").clicked() {
                    let text = self.copy_text();
                    // egui removed `PlatformOutput::copied_text` in favor of
                    // a command queue (`OutputCommand::CopyText`, pushed via
                    // `output_mut`); `Context::copy_text()` is the current
                    // convenience wrapper for exactly that. Confirmed via
                    // the current egui changelog + Context docs after the
                    // old field-assignment form failed to compile.
                    ui.ctx().copy_text(text);
                }
            });
            ui.add_space(6.0);
        });

        egui::Panel::left("filters").resizable(false).default_size(200.0).show(ui, |ui| {
            ui.add_space(8.0);
            ui.strong("K-checks");
            ui.add_space(2.0);
            for name in k_invariants::names() {
                let mut on = self.active_checks.contains(name);
                if ui.checkbox(&mut on, name).changed() {
                    if on {
                        self.active_checks.insert(name);
                    } else {
                        self.active_checks.remove(name);
                    }
                }
            }
            ui.horizontal(|ui| {
                if ui.small_button("All").clicked() {
                    self.active_checks = k_invariants::names().into_iter().collect();
                }
                if ui.small_button("None").clicked() {
                    self.active_checks.clear();
                }
            });

            ui.add_space(10.0);
            ui.separator();
            ui.add_space(10.0);
            ui.strong("Show");
            ui.checkbox(&mut self.show_pass, "PASS");
            ui.checkbox(&mut self.show_na, "NOT_APPLICABLE");
            ui.label(RichText::new("FAIL always shown").weak().italics());

            ui.add_space(10.0);
            ui.separator();
            ui.add_space(10.0);
            ui.strong("Summary");
            let all: Vec<&Verdict> = self.results.values().flatten().collect();
            let pass = all.iter().filter(|v| v.is_pass()).count();
            let fail = all.iter().filter(|v| v.is_fail()).count();
            let na = all.iter().filter(|v| v.status == Status::NotApplicable).count();
            ui.label(format!("Files:  {}", self.results.len()));
            ui.label(RichText::new(format!("PASS:   {pass}")).color(Color32::from_rgb(120, 190, 120)));
            ui.label(
                RichText::new(format!("FAIL:   {fail}")).color(if fail > 0 {
                    Color32::from_rgb(235, 100, 100)
                } else {
                    ui.visuals().text_color()
                }),
            );
            ui.label(RichText::new(format!("N/A:    {na}")).weak());
        });

        egui::CentralPanel::default().show(ui, |ui| {
            // `self.error` is Checks-tab-specific state (set by `run()`);
            // scoping this to `Tab::Checks` keeps a stale path/audit error
            // from covering up the Assemble or Bug Coverage tab's own
            // content after switching tabs.
            if self.tab == Tab::Checks {
                if let Some(err) = &self.error {
                    ui.add_space(8.0);
                    ui.colored_label(Color32::from_rgb(235, 100, 100), err);
                    ui.add_space(4.0);
                    ui.label(RichText::new(&self.status).weak());
                    return;
                }
            }

            match self.tab {
                Tab::Checks => self.show_checks_tab(ui),
                Tab::Assemble => self.show_assemble_tab(ui),
                Tab::BugCoverage => show_bug_coverage_tab(ui, &self.coverage_text),
            }
        });
    }
}


impl UfaApp {
    fn show_checks_tab(&self, ui: &mut egui::Ui) {
        ui.add_space(4.0);
        ui.label(RichText::new(&self.status).weak());
        ui.add_space(6.0);

        if self.results.is_empty() {
            if !self.has_run {
                ui.label("No results yet.");
            }
            return;
        }

        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            for (file, verdicts) in &self.results {
                let fail_count = verdicts.iter().filter(|v| v.is_fail()).count();
                let visible: Vec<&Verdict> = verdicts
                    .iter()
                    .filter(|v| self.active_checks.contains(v.k_class) && self.passes_display_filter(v))
                    .collect();
                if visible.is_empty() {
                    continue;
                }

                let header_text = if fail_count > 0 {
                    RichText::new(format!("{file}  —  {fail_count} FAIL")).color(Color32::from_rgb(235, 100, 100)).strong()
                } else {
                    RichText::new(file.as_str()).strong()
                };

                egui::CollapsingHeader::new(header_text)
                    .id_salt(file.as_str())
                    .default_open(fail_count > 0)
                    .show(ui, |ui| {
                        for v in visible {
                            show_verdict(ui, v);
                        }
                    });
                ui.add_space(4.0);
            }
        });
    }

    /// The Assemble tab: a UAGL source editor on the left, the generated
    /// AArch64 assembly (plus, if `auto_check_generated` is on, its
    /// K01-K06 verdicts) on the right. Both `uagl_aarch64_assembler` and
    /// `ufa_baremetal_core` are called directly from `self.assemble()` —
    /// ordinary in-process function calls into two workspace-sibling
    /// crates, not a subprocess and not a file round-trip.
    fn show_assemble_tab(&mut self, ui: &mut egui::Ui) {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label("Source name:");
            ui.add(
                egui::TextEdit::singleline(&mut self.uagl_source_name)
                    .desired_width(220.0)
                    .hint_text("input.uagl"),
            );
            ui.add_space(12.0);
            if ui.button("Assemble").clicked() {
                self.assemble();
            }
            ui.checkbox(&mut self.auto_check_generated, "Check result (K01–K06)");
        });
        ui.add_space(2.0);
        ui.label(RichText::new(&self.assemble_status).weak());
        if let Some(err) = &self.assemble_error {
            ui.colored_label(Color32::from_rgb(235, 100, 100), err);
        }
        ui.label(RichText::new("Drop a .uagl/.uaglbr file onto this window to load it here.").weak().italics());
        ui.add_space(6.0);

        ui.columns(2, |cols| {
            cols[0].label(RichText::new("UAGL source").strong());
            cols[0].add_space(2.0);
            let src_width = cols[0].available_width();
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(&mut cols[0], |ui| {
                ui.add(
                    egui::TextEdit::multiline(&mut self.uagl_source)
                        .desired_rows(26)
                        .desired_width(src_width)
                        .code_editor(),
                );
            });

            cols[1].label(RichText::new("Generated AArch64 assembly").strong());
            cols[1].add_space(2.0);
            match &self.generated_asm {
                None => {
                    cols[1].label("Not assembled yet.");
                }
                Some(asm) => {
                    let asm = asm.clone();
                    egui::ScrollArea::vertical().auto_shrink([false, false]).max_height(300.0).show(
                        &mut cols[1],
                        |ui| {
                            ui.add(egui::Label::new(RichText::new(&asm).monospace()).selectable(true));
                        },
                    );
                    cols[1].add_space(6.0);
                    cols[1].horizontal(|ui| {
                        ui.label("Save to:");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.uagl_out_path)
                                .desired_width(220.0)
                                .hint_text("path/to/output.s"),
                        );
                        if ui.button("Save .s").clicked() {
                            self.save_generated_asm();
                        }
                    });

                    if !self.generated_verdicts.is_empty() {
                        cols[1].add_space(10.0);
                        cols[1].separator();
                        cols[1].add_space(6.0);
                        let fail = self.generated_verdicts.iter().filter(|v| v.is_fail()).count();
                        let pass = self.generated_verdicts.iter().filter(|v| v.is_pass()).count();
                        let na = self
                            .generated_verdicts
                            .iter()
                            .filter(|v| v.status == Status::NotApplicable)
                            .count();
                        cols[1].label(
                            RichText::new(format!("PASS={pass}  FAIL={fail}  N/A={na}")).strong(),
                        );
                        cols[1].add_space(4.0);
                        let verdicts = self.generated_verdicts.clone();
                        egui::ScrollArea::vertical().auto_shrink([false, false]).max_height(260.0).show(
                            &mut cols[1],
                            |ui| {
                                for v in &verdicts {
                                    if v.is_fail() || self.passes_display_filter(v) {
                                        show_verdict(ui, v);
                                    }
                                }
                            },
                        );
                    }
                }
            }
        });
    }
}

fn show_verdict(ui: &mut egui::Ui, v: &Verdict) {
    let (color, label) = match v.status {
        Status::Pass => (Color32::from_rgb(120, 190, 120), "PASS"),
        Status::Fail => (Color32::from_rgb(235, 100, 100), "FAIL"),
        Status::NotApplicable => (Color32::from_gray(150), "N/A"),
    };

    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(format!("[{}]", v.k_class)).strong().monospace());
        ui.label(RichText::new(label).color(color).strong().monospace());
        if let Some(line) = v.line_no {
            ui.label(RichText::new(format!(":{line}")).weak().monospace());
        }
        if let Some(func) = &v.function {
            if !func.is_empty() {
                ui.label(RichText::new(format!("in {func}()")).weak().monospace());
            }
        }
    });
    if !v.diagnostic.is_empty() {
        ui.add(egui::Label::new(v.diagnostic.as_str()).selectable(true));
    }
    if !v.suggested_fix.is_empty() && v.is_fail() {
        ui.add(egui::Label::new(RichText::new(format!("fix: {}", v.suggested_fix)).italics().weak()).selectable(true));
    }
    ui.add_space(6.0);
    ui.separator();
}

fn show_bug_coverage_tab(ui: &mut egui::Ui, coverage_text: &str) {
    ui.add_space(4.0);
    ui.label(
        "Rust port of `python -m ufa_baremetal.regression_map` — the bug -> K-class coverage \
ledger from brief §4.5. This is the same table that backs `ufa-baremetal bug-coverage` on the CLI.",
    );
    ui.add_space(8.0);
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        ui.add(egui::Label::new(RichText::new(coverage_text).monospace()).selectable(true));
    });
}
