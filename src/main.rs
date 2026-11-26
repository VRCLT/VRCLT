#![windows_subsystem = "windows"]
mod audio;
mod captions;
mod config;
mod hotkeys;
mod labels;
mod osc;
mod platform;
mod provider;
mod runtime;
mod setup;
mod theme;
mod ui;
mod voices;
mod vr;

use eframe::egui::{IconData, ViewportBuilder};
use std::env;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = env::args().skip(1).collect();
    if args.first().is_some_and(|s| s == "--nemotron-worker") {
        if let Err(e) = audio::reuse::worker_main() {
            use std::io::Write;
            let _ = writeln!(
                std::io::stdout(),
                "{}",
                format!("{e:#}").replace(['\r', '\n'], " ")
            );
            std::process::exit(1);
        }
        return Ok(());
    }
    if args.iter().any(|s| s == "--help" || s == "-h") {
        println!("vrclt [run] [--app vrchat|discord|custom]");
        return Ok(());
    }
    let profile = args
        .windows(2)
        .find(|w| w[0] == "--app")
        .map(|w| w[1].clone());
    let focus = !args.iter().any(|arg| arg == "--steamvr-startup");
    // Keep both guards for the full process lifetime, including setup and stopped sessions.
    let Some(_native) = platform::single_instance("Local\\vrclt.rust.console", focus)? else {
        return Ok(());
    };
    let Some(_legacy) = platform::single_instance("Local\\vrclt.single-instance", focus)? else {
        return Ok(());
    };
    let path = config::Config::path();
    let img = image::load_from_memory(include_bytes!("../assets/icon.png"))
        .unwrap()
        .resize(128, 128, image::imageops::FilterType::Triangle)
        .to_rgba8();
    let icon = IconData {
        width: img.width(),
        height: img.height(),
        rgba: img.into_raw(),
    };
    let options = eframe::NativeOptions {
        viewport: ViewportBuilder::default()
            .with_title("VRCLT")
            .with_decorations(false)
            .with_min_inner_size([780., 540.])
            .with_icon(icon.clone()),
        window_builder: Some(Box::new(|builder| {
            builder.with_inner_size(platform::console_size())
        })),
        centered: true,
        ..Default::default()
    };
    Ok(eframe::run_native(
        "VRCLT",
        options,
        Box::new(move |cc| Ok(Box::new(ui::Studio::new(cc, path, profile, icon)))),
    )?)
}
