//! Fetching web pages (link previews, update checks) and files (updates). Blocking: call
//! these off the UI thread. Desktop only for now.

use std::path::Path;

/// The start of the page at `url` as text (at most a megabyte), or why it could not be
/// fetched. Blocking: call it off the UI thread. Only `http` and `https` addresses.
pub fn fetch_page(url: &str) -> Result<String, String> {
    let lower = url.to_ascii_lowercase();
    if !(lower.starts_with("http://") || lower.starts_with("https://")) {
        return Err("only web addresses can be fetched".into());
    }
    imp::fetch_page(url)
}

/// GETs `url` with `headers` (name, value): the answer's status and body (at most a
/// megabyte), whatever the status, or why there is no answer.
pub fn get(url: &str, headers: &[(&str, &str)]) -> Result<(u16, String), String> {
    imp::get(url, headers)
}

/// Downloads `url` into the file `dest` (replacing it), telling `progress` the bytes so
/// far and the size if known. Returns the size written.
pub fn download(
    url: &str,
    headers: &[(&str, &str)],
    dest: &Path,
    progress: &mut dyn FnMut(u64, Option<u64>),
) -> Result<u64, String> {
    imp::download(url, headers, dest, progress)
}

#[cfg(not(target_arch = "wasm32"))]
mod imp {
    use std::io::Write;
    use std::path::Path;

    use std::io::Read;
    use std::time::Duration;

    use ureq::tls::{RootCerts, TlsConfig, TlsProvider};

    /// Pages larger than this are cut; the head is near the start.
    const MAX_BYTES: u64 = 1 << 20;

    /// An agent over the system's TLS. `timeout` limits the whole request; a body that
    /// stops coming ends it anyway.
    fn agent(timeout: Option<Duration>, user_agent: &str, status_as_error: bool) -> ureq::Agent {
        let tls = TlsConfig::builder()
            .provider(TlsProvider::NativeTls)
            .root_certs(RootCerts::PlatformVerifier)
            .build();
        ureq::Agent::config_builder()
            .tls_config(tls)
            .timeout_global(timeout)
            .timeout_connect(Some(Duration::from_secs(15)))
            .timeout_recv_body(Some(Duration::from_secs(60)))
            .user_agent(user_agent)
            .http_status_as_error(status_as_error)
            .build()
            .into()
    }

    pub fn fetch_page(url: &str) -> Result<String, String> {
        let agent = agent(
            Some(Duration::from_secs(10)),
            "PaintBoard link preview",
            true,
        );
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

    fn call(
        agent: &ureq::Agent,
        url: &str,
        headers: &[(&str, &str)],
    ) -> Result<ureq::http::Response<ureq::Body>, String> {
        let mut request = agent.get(url);
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        request.call().map_err(|e| e.to_string())
    }

    pub fn get(url: &str, headers: &[(&str, &str)]) -> Result<(u16, String), String> {
        let agent = agent(Some(Duration::from_secs(20)), "PaintBoard", false);
        let response = call(&agent, url, headers)?;
        let status = response.status().as_u16();
        let mut bytes = Vec::new();
        response
            .into_body()
            .into_reader()
            .take(MAX_BYTES)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        Ok((status, String::from_utf8_lossy(&bytes).into_owned()))
    }

    pub fn download(
        url: &str,
        headers: &[(&str, &str)],
        dest: &Path,
        progress: &mut dyn FnMut(u64, Option<u64>),
    ) -> Result<u64, String> {
        // No limit on the whole download, which may be slow.
        let agent = agent(None, "PaintBoard", true);
        let response = call(&agent, url, headers)?;
        let total = response.body().content_length();
        let mut reader = response.into_body().into_reader();
        let mut file = std::fs::File::create(dest).map_err(|e| e.to_string())?;
        let mut buffer = vec![0; 64 * 1024];
        let mut done = 0u64;
        progress(0, total);
        loop {
            let n = match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(n) => n,
                Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(err) => return Err(err.to_string()),
            };
            file.write_all(&buffer[..n]).map_err(|e| e.to_string())?;
            done += n as u64;
            progress(done, total);
        }
        file.sync_all().map_err(|e| e.to_string())?;
        Ok(done)
    }
}

#[cfg(target_arch = "wasm32")]
mod imp {
    use std::path::Path;

    pub fn fetch_page(_url: &str) -> Result<String, String> {
        Err("link previews are not available in the browser yet".into())
    }

    pub fn get(_url: &str, _headers: &[(&str, &str)]) -> Result<(u16, String), String> {
        Err("not available in the browser".into())
    }

    pub fn download(
        _url: &str,
        _headers: &[(&str, &str)],
        _dest: &Path,
        _progress: &mut dyn FnMut(u64, Option<u64>),
    ) -> Result<u64, String> {
        Err("not available in the browser".into())
    }
}
