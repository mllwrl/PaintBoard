//! Finding out about a newer PaintBoard on GitHub, and installing it (desktop).
//!
//! The rules (versions, which file to download, when to look again) are plain functions
//! here; the app runs the checks and downloads on threads of their own.

use std::cmp::Ordering;
use std::fmt;

/// GitHub's answer about the newest published release.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub const LATEST_URL: &str = "https://api.github.com/repos/mllwrl/PaintBoard/releases/latest";

/// Automatic checks happen at most this often, and "Later" hides a version this long.
pub const DAY_MS: i64 = 24 * 60 * 60 * 1000;

/// Setting this environment variable (to anything but `0`) turns automatic checks off,
/// for tests and smoke runs.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub const NO_CHECK_VAR: &str = "PB_NO_UPDATE_CHECK";

/// A version like `1.2.3` or `1.2.3-beta.1`, compared as semver does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Version {
    numbers: [u64; 3],
    /// The pre-release identifiers after `-`; none for a release.
    pre: Vec<String>,
}

impl Version {
    /// From `1.2.3`, `v1.2.3`, `1.2` or `1.2.3-rc.1+build`; `None` if it is not one.
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        let text = text
            .strip_prefix('v')
            .or_else(|| text.strip_prefix('V'))
            .unwrap_or(text);
        let text = text.split('+').next()?;
        let (core, pre) = match text.split_once('-') {
            Some((core, pre)) => (core, Some(pre)),
            None => (text, None),
        };
        let mut numbers = [0; 3];
        let mut parts = core.split('.');
        for (i, number) in numbers.iter_mut().enumerate() {
            match parts.next() {
                Some(part) if !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()) => {
                    *number = part.parse().ok()?;
                }
                // `1.2` is `1.2.0`; the major number must be there.
                None if i > 0 => {}
                _ => return None,
            }
        }
        if parts.next().is_some() {
            return None;
        }
        let pre = match pre {
            Some(pre) => {
                let ids: Vec<String> = pre.split('.').map(str::to_owned).collect();
                if ids.iter().any(String::is_empty) {
                    return None;
                }
                ids
            }
            None => Vec::new(),
        };
        Some(Self { numbers, pre })
    }

    /// This app's own version.
    pub fn current() -> Self {
        Self::parse(env!("CARGO_PKG_VERSION")).expect("the package version is a version")
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        self.numbers.cmp(&other.numbers).then_with(|| {
            // A pre-release comes before its release.
            match (self.pre.is_empty(), other.pre.is_empty()) {
                (true, true) => Ordering::Equal,
                (true, false) => Ordering::Greater,
                (false, true) => Ordering::Less,
                (false, false) => {
                    for (a, b) in self.pre.iter().zip(&other.pre) {
                        let order = match (a.parse::<u64>(), b.parse::<u64>()) {
                            (Ok(a), Ok(b)) => a.cmp(&b),
                            (Ok(_), Err(_)) => Ordering::Less,
                            (Err(_), Ok(_)) => Ordering::Greater,
                            (Err(_), Err(_)) => a.cmp(b),
                        };
                        if order != Ordering::Equal {
                            return order;
                        }
                    }
                    self.pre.len().cmp(&other.pre.len())
                }
            }
        })
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let [major, minor, patch] = self.numbers;
        write!(f, "{major}.{minor}.{patch}")?;
        if !self.pre.is_empty() {
            write!(f, "-{}", self.pre.join("."))?;
        }
        Ok(())
    }
}

/// A file attached to a release.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Asset {
    pub name: String,
    /// Where to download it.
    pub url: String,
    pub size: u64,
}

/// A published release.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    pub version: Version,
    /// The release's page, with what is new.
    pub page: String,
    pub assets: Vec<Asset>,
}

/// The release in GitHub's answer about the latest one; `None` for a draft, a
/// pre-release or a tag that is not a version.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub fn parse_release(json: &str) -> Result<Option<Release>, String> {
    let value: serde_json::Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
    fn text<'a>(v: &'a serde_json::Value, key: &str) -> Option<&'a str> {
        v.get(key).and_then(|t| t.as_str())
    }
    let flag = |key: &str| value.get(key).and_then(|f| f.as_bool()).unwrap_or(false);
    if flag("draft") || flag("prerelease") {
        return Ok(None);
    }
    let Some(tag) = text(&value, "tag_name") else {
        return Err("the answer has no tag_name".into());
    };
    let Some(version) = Version::parse(tag) else {
        return Ok(None);
    };
    let page = text(&value, "html_url")
        .map(str::to_owned)
        .unwrap_or_else(|| format!("https://github.com/mllwrl/PaintBoard/releases/tag/{tag}"));
    let assets = value
        .get("assets")
        .and_then(|a| a.as_array())
        .map(|assets| {
            assets
                .iter()
                .filter_map(|a| {
                    Some(Asset {
                        name: text(a, "name")?.to_owned(),
                        url: text(a, "browser_download_url")?.to_owned(),
                        size: a.get("size").and_then(|s| s.as_u64()).unwrap_or(0),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(Some(Release {
        version,
        page,
        assets,
    }))
}

/// The systems releases have files for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Os {
    Windows,
    Linux,
    Mac,
}

impl Os {
    /// The system this app was built for, if releases have files for it.
    pub fn this() -> Option<Self> {
        if cfg!(target_os = "windows") {
            Some(Self::Windows)
        } else if cfg!(target_os = "macos") {
            Some(Self::Mac)
        } else if cfg!(target_os = "linux") {
            Some(Self::Linux)
        } else {
            None
        }
    }
}

/// The file of `assets` that installs PaintBoard on `os`: the Windows installer, the
/// AppImage, the disk image.
pub fn asset_for(assets: &[Asset], os: Os) -> Option<&Asset> {
    assets.iter().find(|a| {
        let name = a.name.to_ascii_lowercase();
        match os {
            Os::Windows => name.ends_with("-setup.exe"),
            Os::Linux => name.ends_with(".appimage"),
            Os::Mac => name.ends_with(".dmg"),
        }
    })
}

/// What is remembered about checking between runs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Prefs {
    /// Automatic checks are turned off.
    pub off: bool,
    /// When the last check started (ms since the Unix epoch).
    pub checked: Option<i64>,
    /// The version put off with "Later", and when.
    pub later: Option<(String, i64)>,
}

impl Prefs {
    /// From the settings file: lines `off`, `checked <ms>`, `later <version> <ms>`.
    pub fn parse(text: &str) -> Self {
        let mut prefs = Self::default();
        for line in text.lines() {
            let words: Vec<&str> = line.split_whitespace().collect();
            match words[..] {
                ["off"] => prefs.off = true,
                ["checked", ms] => prefs.checked = ms.parse().ok(),
                ["later", version, ms] => {
                    prefs.later = ms.parse().ok().map(|ms| (version.to_owned(), ms));
                }
                _ => {}
            }
        }
        prefs
    }

    pub fn to_text(&self) -> String {
        let mut text = String::new();
        if self.off {
            text += "off\n";
        }
        if let Some(ms) = self.checked {
            text += &format!("checked {ms}\n");
        }
        if let Some((version, ms)) = &self.later {
            text += &format!("later {version} {ms}\n");
        }
        text
    }

    /// An automatic check is due at `now`: on, and none for a day (or the clock went back).
    pub fn check_due(&self, now: i64) -> bool {
        !self.off && self.checked.is_none_or(|at| now < at || now - at >= DAY_MS)
    }

    /// `version` was put off with "Later" less than a day before `now`.
    pub fn put_off(&self, version: &Version, now: i64) -> bool {
        self.later.as_ref().is_some_and(|(later, at)| {
            Version::parse(later).as_ref() == Some(version) && (0..DAY_MS).contains(&(now - at))
        })
    }
}

/// Where updating stands, for the app.
#[derive(Default)]
pub struct Updates {
    pub prefs: Prefs,
    /// A newer release, found by the last check.
    pub found: Option<Release>,
    /// The notice about it shows.
    pub notice: bool,
    /// A check is running: `Some(true)` if asked for by hand.
    pub checking: Option<bool>,
    pub download: Option<Download>,
}

/// The new version's installer being downloaded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Download {
    /// Bytes so far, of how many.
    Running(u64, u64),
    Failed(String),
    /// Downloaded here; the app closes for it to run.
    Ready(std::path::PathBuf),
}

/// Automatic checks are turned off through [`NO_CHECK_VAR`].
pub fn off_by_env() -> bool {
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::env::var_os(NO_CHECK_VAR).is_some_and(|v| !v.is_empty() && v != "0")
    }
    #[cfg(target_arch = "wasm32")]
    true
}

/// Asks GitHub for the latest release (blocking: call it off the UI thread). `None` if
/// there is none, or it is a draft or a pre-release.
#[cfg(not(target_arch = "wasm32"))]
pub fn fetch_latest() -> Result<Option<Release>, String> {
    let agent = format!("PaintBoard/{}", env!("CARGO_PKG_VERSION"));
    let headers = [
        ("User-Agent", agent.as_str()),
        ("Accept", "application/vnd.github+json"),
        ("X-GitHub-Api-Version", "2022-11-28"),
    ];
    let (status, body) = pb_platform::web::get(LATEST_URL, &headers)?;
    match status {
        200 => parse_release(&body),
        // No release published yet.
        404 => Ok(None),
        _ => Err(format!(
            "{status}: {}",
            body.chars().take(200).collect::<String>()
        )),
    }
}

/// Downloads `asset` into the temporary folder (blocking), telling `progress` the bytes
/// so far; checks its size. Returns where it is.
#[cfg(not(target_arch = "wasm32"))]
pub fn download(
    asset: &Asset,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<std::path::PathBuf, String> {
    // Only the file's own name, whatever the answer said.
    let name = std::path::Path::new(&asset.name)
        .file_name()
        .ok_or("the file has no name")?;
    let dest = pb_platform::fs::temp_dir()
        .map_err(|e| e.to_string())?
        .join(name);
    let agent = format!("PaintBoard/{}", env!("CARGO_PKG_VERSION"));
    let headers = [
        ("User-Agent", agent.as_str()),
        ("Accept", "application/octet-stream"),
    ];
    let mut tell = |done: u64, total: Option<u64>| progress(done, total.unwrap_or(asset.size));
    let size = pb_platform::web::download(&asset.url, &headers, &dest, &mut tell)?;
    if size != asset.size {
        let _ = pb_platform::fs::remove(&dest);
        return Err(format!(
            "the download is {size} bytes instead of {}",
            asset.size
        ));
    }
    Ok(dest)
}

/// Starts the Windows installer at `path` to update PaintBoard without asking: it closes
/// this app if still running and starts the new version when done (`/RELAUNCH=1`, see
/// `packaging/windows/PaintBoard.iss`).
#[cfg(not(target_arch = "wasm32"))]
pub fn run_installer(path: &std::path::Path) -> Result<(), String> {
    std::process::Command::new(path)
        .args([
            "/SILENT",
            "/SUPPRESSMSGBOXES",
            "/NORESTART",
            "/CLOSEAPPLICATIONS",
            "/RELAUNCH=1",
        ])
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(text: &str) -> Version {
        Version::parse(text).unwrap()
    }

    #[test]
    fn versions_are_read_and_compared_as_semver() {
        assert_eq!(v("v0.2.0"), v("0.2.0"));
        assert_eq!(v("1.2"), v("1.2.0"));
        assert_eq!(v("1.0.0+build.5"), v("1.0.0"));
        assert_eq!(v("V1.2.3-rc.1").to_string(), "1.2.3-rc.1");
        assert!(v("0.10.0") > v("0.9.9"));
        assert!(v("1.0.0") > v("0.99.99"));
        assert!(v("0.2.1") > v("0.2.0"));
        assert!(v("1.0.0") > v("1.0.0-rc.1"));
        assert!(v("1.0.0-rc.2") > v("1.0.0-rc.1"));
        assert!(v("1.0.0-rc.10") > v("1.0.0-rc.9"));
        assert!(v("1.0.0-beta") > v("1.0.0-alpha.5"));
        assert!(v("1.0.0-alpha.1") > v("1.0.0-alpha"));
        assert!(v("1.0.0-alpha") > v("1.0.0-1"));
        for bad in [
            "",
            "v",
            "x1.0",
            "1..2",
            "1.2.3.4",
            "1.a.0",
            "1.0.0-",
            "1.0.0-a..b",
        ] {
            assert_eq!(Version::parse(bad), None, "{bad}");
        }
        assert!(Version::current() >= v("0.0.1"));
    }

    const ANSWER: &str = r#"{
        "tag_name": "v0.3.0",
        "html_url": "https://github.com/mllwrl/PaintBoard/releases/tag/v0.3.0",
        "draft": false,
        "prerelease": false,
        "assets": [
            {"name": "PaintBoard-0.3.0-x86_64.AppImage", "size": 900,
             "browser_download_url": "https://example.com/a.AppImage"},
            {"name": "PaintBoard-0.3.0-setup.exe", "size": 1234,
             "browser_download_url": "https://example.com/setup.exe"},
            {"name": "PaintBoard-0.3.0-linux-x86_64.tar.gz", "size": 800,
             "browser_download_url": "https://example.com/a.tar.gz"},
            {"name": "PaintBoard-0.3.0-macos.dmg", "size": 700,
             "browser_download_url": "https://example.com/a.dmg"},
            {"name": "broken"}
        ]
    }"#;

    #[test]
    fn the_latest_release_is_read_from_githubs_answer() {
        let release = parse_release(ANSWER).unwrap().unwrap();
        assert_eq!(release.version, v("0.3.0"));
        assert!(release.page.ends_with("/tag/v0.3.0"));
        assert_eq!(release.assets.len(), 4);
        let setup = asset_for(&release.assets, Os::Windows).unwrap();
        assert_eq!(setup.size, 1234);
        assert_eq!(setup.url, "https://example.com/setup.exe");
        let linux = asset_for(&release.assets, Os::Linux).unwrap();
        assert!(linux.name.ends_with(".AppImage"));
        assert!(
            asset_for(&release.assets, Os::Mac)
                .unwrap()
                .name
                .ends_with(".dmg")
        );
        assert_eq!(asset_for(&[], Os::Windows), None);

        let draft = ANSWER.replace(r#""draft": false"#, r#""draft": true"#);
        assert_eq!(parse_release(&draft), Ok(None));
        let pre = ANSWER.replace(r#""prerelease": false"#, r#""prerelease": true"#);
        assert_eq!(parse_release(&pre), Ok(None));
        let odd_tag = ANSWER.replace("\"v0.3.0\"", "\"nightly\"");
        assert_eq!(parse_release(&odd_tag), Ok(None));
        assert!(parse_release("not json").is_err());
        assert!(parse_release("{}").is_err());
    }

    #[test]
    fn checks_happen_once_a_day_unless_turned_off() {
        let now = 1_000 * DAY_MS;
        let mut prefs = Prefs::default();
        assert!(prefs.check_due(now));
        prefs.checked = Some(now - DAY_MS + 1);
        assert!(!prefs.check_due(now));
        prefs.checked = Some(now - DAY_MS);
        assert!(prefs.check_due(now));
        // The clock went back: check rather than wait.
        prefs.checked = Some(now + 5);
        assert!(prefs.check_due(now));
        prefs.off = true;
        assert!(!prefs.check_due(now));
    }

    #[test]
    fn later_hides_that_version_for_a_day() {
        let now = 1_000 * DAY_MS;
        let prefs = Prefs {
            later: Some(("0.3.0".into(), now - 1000)),
            ..Prefs::default()
        };
        assert!(prefs.put_off(&v("0.3.0"), now));
        assert!(!prefs.put_off(&v("0.3.1"), now));
        assert!(!prefs.put_off(&v("0.3.0"), now - 1000 + DAY_MS));
        assert!(!Prefs::default().put_off(&v("0.3.0"), now));
    }

    #[test]
    fn prefs_survive_the_settings_file() {
        let prefs = Prefs {
            off: true,
            checked: Some(123),
            later: Some(("0.4.0-rc.1".into(), 456)),
        };
        assert_eq!(Prefs::parse(&prefs.to_text()), prefs);
        assert_eq!(Prefs::parse(""), Prefs::default());
        assert_eq!(
            Prefs::parse("checked x\nlater 1.0\nnonsense"),
            Prefs::default()
        );
    }
}
