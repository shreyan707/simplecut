//! SimpleCut, the desktop app: a window, a sound card and file dialogs around `simplecut-ui`.

mod audio;
mod dialogs;

use std::path::PathBuf;

use simplecut_core::AudioOutput;
use simplecut_ui::SimpleCutApp;

fn main() -> eframe::Result {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    install_panic_logging();
    // hardware video decoding where the machine has it; our own decoders otherwise
    let hardware = filmcraft_platform::register();
    log::info!("hardware decoding: {hardware:?}");

    let open: Option<PathBuf> = std::env::args_os().nth(1).map(PathBuf::from);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("SimpleCut")
            .with_inner_size([1360.0, 860.0])
            .with_min_inner_size([960.0, 600.0])
            .with_drag_and_drop(true),
        ..Default::default()
    };
    eframe::run_native(
        "SimpleCut",
        options,
        Box::new(move |cc| {
            let output = audio::CpalOut::new().map(|o| Box::new(o) as Box<dyn AudioOutput>);
            if output.is_none() {
                log::warn!("no sound card found; playing without sound");
            }
            let mut app = SimpleCutApp::new(&cc.egui_ctx, Box::new(dialogs::NativeDialogs), output);
            if let Some(path) = open {
                app.open_project_path(path);
            }
            Ok(Box::new(app))
        }),
    )
}

/// Record where a panic happened before the default handler runs. The interface and the background
/// threads catch their own panics, so this is for finding out why.
fn install_panic_logging() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current();
        let place = info.location().map(|l| format!("{}:{}", l.file(), l.line())).unwrap_or_default();
        log::error!("panic on thread '{}' at {place}: {info}", thread.name().unwrap_or("unnamed"));
        previous(info);
    }));
}
