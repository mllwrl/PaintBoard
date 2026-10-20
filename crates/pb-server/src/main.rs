//! PaintBoard's server: boards shared by link, kept in one SQLite file.
//!
//! Settings come from the environment:
//! - `PB_ADDR`: where to listen, `0.0.0.0:8080` by default;
//! - `PB_DB`: the database file, `paintboard.db` by default;
//! - `PB_CREATE_KEY`: if set, making a board needs this key (else anyone can);
//! - `PB_WEB`: the built web version (`target/web`) to serve, so a board's link opens it
//!   in a browser.
//!
//! - `PB_MAX_BOARDS`: the most boards the server keeps (no limit if unset);
//! - `PB_BACKUP`: a folder for a daily copy of the database (the newest 7 are kept);
//! - `PB_TRUST_PROXY`: set to `1` behind a proxy (Caddy) that puts the client's address
//!   in `X-Forwarded-For`, so limits per address count clients, not the proxy;
//! - `PB_METRICS`: if set to `1`, `GET /metrics` shows counts for Prometheus (close it to
//!   the public at the proxy);
//! - `PB_PREVIEWS`: if set to `1`, the server fetches the heads of web pages for link
//!   previews in the web version (see `preview`).
//!
//! Put it behind a proxy with TLS (e.g. Caddy) so links work over `wss://`.

#[cfg(not(target_arch = "wasm32"))]
mod limits;
#[cfg(not(target_arch = "wasm32"))]
mod metrics;
#[cfg(not(target_arch = "wasm32"))]
mod preview;
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
    let web = std::env::var_os("PB_WEB").map(PathBuf::from);

    let store = match store::Store::open(&db) {
        Ok(store) => store,
        Err(err) => {
            eprintln!("cannot open {}: {err}", db.display());
            std::process::exit(1);
        }
    };
    let mut shared = server::Shared::with_web(store, create_key, web);
    if let Some(max) = std::env::var("PB_MAX_BOARDS")
        .ok()
        .and_then(|m| m.parse().ok())
    {
        std::sync::Arc::get_mut(&mut shared)
            .expect("not shared yet")
            .max_boards = Some(max);
    }
    if std::env::var("PB_TRUST_PROXY").is_ok_and(|v| v == "1") {
        std::sync::Arc::get_mut(&mut shared)
            .expect("not shared yet")
            .limits
            .trust_proxy = true;
    }
    if std::env::var("PB_METRICS").is_ok_and(|v| v == "1") {
        std::sync::Arc::get_mut(&mut shared)
            .expect("not shared yet")
            .serve_metrics = true;
    }
    if std::env::var("PB_PREVIEWS").is_ok_and(|v| v == "1") {
        std::sync::Arc::get_mut(&mut shared)
            .expect("not shared yet")
            .previews = Some(preview::Previews::new());
    }
    if let Some(dir) = std::env::var_os("PB_BACKUP").map(PathBuf::from) {
        let backer = shared.clone();
        tokio::spawn(async move {
            let mut every = tokio::time::interval(std::time::Duration::from_secs(86_400));
            loop {
                every.tick().await;
                let (shared, dir) = (backer.clone(), dir.clone());
                let _ = tokio::task::spawn_blocking(move || shared.back_up(&dir, 7)).await;
            }
        });
    }
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
    let app = server::router(shared.clone())
        .into_make_service_with_connect_info::<std::net::SocketAddr>();
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
