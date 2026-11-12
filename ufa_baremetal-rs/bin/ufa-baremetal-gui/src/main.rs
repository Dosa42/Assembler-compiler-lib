//! Native GUI entry point for the K01-K06 invariant checker.
//!
//! Thin by design: everything egui/eframe-specific that could plausibly
//! need adjusting for a future egui version lives in this file and
//! `app.rs`'s `impl eframe::App` block. The actual checking logic is
//! untouched — it's the same `ufa_baremetal_core` the CLI uses.

mod app;

use app::UfaApp;

fn main() -> eframe::Result {
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1180.0, 760.0])
            .with_min_inner_size([760.0, 480.0]),
        ..Default::default()
    };

    eframe::run_native("UFA Bare-Metal K-Check", native_options, Box::new(|cc| Ok(Box::new(UfaApp::new(cc)))))
}
