//! PaintBoard desktop/web entry point: wires winit, wgpu and egui to the rest of the crates.

// Release builds on Windows open no console window next to the app.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod account;
mod app;
mod books;
mod editing;
mod export;
mod i18n;
mod my_templates;
mod navigation;
mod palette;
mod pictures;
mod plugin_api;
mod plugins;
mod session;
mod settings;
mod sharing;
mod sounds;
mod stylus;
mod talktrack;
mod templates;
mod ui;
mod updates;

use app::{App, UserEvent};
use winit::event_loop::EventLoop;

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("warn,pb_app=info,pb_render=info"),
    )
    .init();
    keep_crashes();
    // PaintBoard is running already: it opens the file (or just shows itself) instead.
    let file = std::env::args_os().nth(1).map(std::path::PathBuf::from);
    if pb_platform::instance::hand_over(file.as_deref()) {
        return;
    }
    let event_loop = EventLoop::<UserEvent>::with_user_event()
        .build()
        .expect("failed to create event loop");
    let mut app = App::new(event_loop.create_proxy());
    event_loop.run_app(&mut app).expect("event loop failed");
}

/// A crash is written down in `crash.log` in the app's data folder, with where it happened:
/// release builds have no console to show it in.
#[cfg(not(target_arch = "wasm32"))]
fn keep_crashes() {
    use std::io::Write;
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let backtrace = std::backtrace::Backtrace::force_capture();
        if let Ok(dir) = pb_platform::fs::app_data_dir() {
            let _ = std::fs::create_dir_all(&dir);
            let log = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(dir.join("crash.log"));
            if let Ok(mut log) = log {
                let _ = writeln!(
                    log,
                    "PaintBoard {} crashed at {} ms since 1970: {info}\n{backtrace}\n",
                    env!("CARGO_PKG_VERSION"),
                    pb_platform::now_ms(),
                );
            }
        }
        default(info);
    }));
}

#[cfg(target_arch = "wasm32")]
fn main() {
    use winit::platform::web::EventLoopExtWebSys;

    std::panic::set_hook(Box::new(console_error_panic_hook::hook));
    console_log::init_with_level(log::Level::Info).expect("failed to init logging");
    // The board and settings kept in the browser are read before the app opens them.
    wasm_bindgen_futures::spawn_local(async {
        pb_platform::fs::init().await;
        let event_loop = EventLoop::<UserEvent>::with_user_event()
            .build()
            .expect("failed to create event loop");
        let app = App::new(event_loop.create_proxy());
        event_loop.spawn_app(app);
    });
}
