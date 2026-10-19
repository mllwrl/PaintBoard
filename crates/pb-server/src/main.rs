//! PaintBoard's server: boards shared by link, kept in one SQLite file.
//!
//! Settings come from the environment:
//! - `PB_ADDR`: where to listen, `0.0.0.0:8080` by default;
//! - `PB_DB`: the database file, `paintboard.db` by default;
//! - `PB_CREATE_KEY`: if set, making a board needs this key (else anyone can).
//!
//! Put it behind a proxy with TLS (e.g. Caddy) so links work over `wss://`.

#[cfg(not(target_arch = "wasm32"))]
mod server;
#[cfg(not(target_arch = "wasm32"))]
mod store;
#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests;

#[cfg(not(target_arch = "wasm32"))]
#[tokio::main]
async fn main() {
    use std::path::PathBuf;

    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let addr = std::env::var("PB_ADDR").unwrap_or_else(|_| "0.0.0.0:8080".into());
    let db = PathBuf::from(std::env::var("PB_DB").unwrap_or_else(|_| "paintboard.db".into()));
    let create_key = std::env::var("PB_CREATE_KEY")
        .ok()
        .filter(|k| !k.is_empty());

    let store = match store::Store::open(&db) {
        Ok(store) => store,
        Err(err) => {
            eprintln!("cannot open {}: {err}", db.display());
            std::process::exit(1);
        }
    };
    let shared = server::Shared::new(store, create_key);
    let saver = shared.clone();
    tokio::spawn(async move {
        let mut every = tokio::time::interval(server::SAVE_EVERY);
        loop {
            every.tick().await;
            let shared = saver.clone();
            let _ = tokio::task::spawn_blocking(move || shared.save_all()).await;
        }
    });

    let listener = match tokio::net::TcpListener::bind(&addr).await {
        Ok(listener) => listener,
        Err(err) => {
            eprintln!("cannot listen on {addr}: {err}");
            std::process::exit(1);
        }
    };
    log::info!("listening on {addr}, boards in {}", db.display());
    let app = server::router(shared.clone());
    let stopped = axum::serve(listener, app).with_graceful_shutdown(async {
        let _ = tokio::signal::ctrl_c().await;
        log::info!("stopping");
    });
    if let Err(err) = stopped.await {
        log::error!("server failed: {err}");
    }
    shared.save_all();
}

/// Nothing to run in the browser.
#[cfg(target_arch = "wasm32")]
fn main() {}
