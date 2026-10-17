//! Fetching web pages, for link previews. Desktop only for now.

/// The start of the page at `url` as text (at most a megabyte), or why it could not be
/// fetched. Blocking: call it off the UI thread. Only `http` and `https` addresses.
pub fn fetch_page(url: &str) -> Result<String, String> {
    let lower = url.to_ascii_lowercase();
    if !(lower.starts_with("http://") || lower.starts_with("https://")) {
        return Err("only web addresses can be fetched".into());
    }
    imp::fetch_page(url)
}

#[cfg(not(target_arch = "wasm32"))]
mod imp {
    use std::io::Read;
    use std::time::Duration;

    use ureq::tls::{RootCerts, TlsConfig, TlsProvider};

    /// Pages larger than this are cut; the head is near the start.
    const MAX_BYTES: u64 = 1 << 20;

    pub fn fetch_page(url: &str) -> Result<String, String> {
        let tls = TlsConfig::builder()
            .provider(TlsProvider::NativeTls)
            .root_certs(RootCerts::PlatformVerifier)
            .build();
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .tls_config(tls)
            .timeout_global(Some(Duration::from_secs(10)))
            .user_agent("PaintBoard link preview")
            .build()
            .into();
        let response = agent.get(url).call().map_err(|e| e.to_string())?;
        let mut bytes = Vec::new();
        response
            .into_body()
            .into_reader()
            .take(MAX_BYTES)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }
}

#[cfg(target_arch = "wasm32")]
mod imp {
    pub fn fetch_page(_url: &str) -> Result<String, String> {
        Err("link previews are not available in the browser yet".into())
    }
}
