//! PaintBoard desktop/web entry point: wires winit, wgpu and egui to the rest of the crates.

mod app;
mod editing;
mod export;
mod i18n;
mod navigation;
mod pictures;
mod session;
mod settings;
mod talktrack;
mod templates;
mod ui;

use app::{App, UserEvent};
use winit::event_loop::EventLoop;

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("warn,pb_app=info,pb_render=info"),
    )
    .init();
    let event_loop = EventLoop::<UserEvent>::with_user_event()
        .build()
        .expect("failed to create event loop");
    let mut app = App::new(event_loop.create_proxy());
    event_loop.run_app(&mut app).expect("event loop failed");
}

#[cfg(target_arch = "wasm32")]
fn main() {
    use winit::platform::web::EventLoopExtWebSys;

    std::panic::set_hook(Box::new(console_error_panic_hook::hook));
    console_log::init_with_level(log::Level::Info).expect("failed to init logging");
    let event_loop = EventLoop::<UserEvent>::with_user_event()
        .build()
        .expect("failed to create event loop");
    let app = App::new(event_loop.create_proxy());
    event_loop.spawn_app(app);
}
