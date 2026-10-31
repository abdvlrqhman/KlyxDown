use std::{fs, io, path::PathBuf};

use klyxdown_engine::SpeedMode;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

pub const THEMES: &[&str] = &["system", "midnight", "graphite", "fern", "aluminum", "sand"];

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub theme: String,
    /// "system" or a language code the UI has (src/locales).
    pub language: String,
    pub run_on_startup: bool,
    /// When launched at login, stay in the tray instead of opening the window.
    pub start_hidden: bool,
    pub minimize_to_tray: bool,
    /// The close button hides to the tray; downloads keep running.
    pub close_to_tray: bool,
    pub download_dir: PathBuf,
    pub max_active: usize,
    pub connections: u8,
    pub mode: SpeedMode,
    /// Bytes per second; `None` means no limit.
    pub limit: Option<u64>,
    /// Show downloads caught from the browser before starting them.
    pub ask_on_capture: bool,
    /// Save into a subfolder per file type (Video, Programs, Archives...).
    pub sort_by_type: bool,
    /// Short animated intro when the window opens.
    pub show_intro: bool,
    /// The first-run guide has been completed or skipped.
    pub onboarded: bool,
    /// "system", "none" or "custom".
    pub proxy_mode: String,
    /// Used when `proxy_mode` is "custom": http://, https:// or socks5:// address.
    pub proxy_url: String,
    /// Hold off sleep while something is downloading.
    pub keep_awake: bool,
    /// Offer links to files that get copied to the clipboard.
    pub watch_clipboard: bool,
    /// Look for a new version on GitHub shortly after launch.
    pub auto_update: bool,
    /// Hold downloads while the connection is metered (phone hotspot, capped plans).
    pub pause_on_metered: bool,
    /// Download no more than this many megabytes every `quota_hours`; `None` is no quota.
    pub quota_mb: Option<u64>,
    pub quota_hours: u32,
    /// Keep sharing torrents once they finish, up to `seed_ratio` (0: until stopped).
    pub seed: bool,
    pub seed_ratio: f64,
    /// Open magnet links from the browser and other apps in KlyxDown.
    pub handle_magnets: bool,
    /// Check finished files with the antivirus.
    pub scan_after: bool,
    /// A scanner command (`{file}` is the path); empty uses the system's.
    pub scanner: String,
    /// Sites with a saved sign-in. Passwords are in the OS keychain, never here.
    pub sites: Vec<Site>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Site {
    pub host: String,
    pub user: String,
}

/// The keychain entry for a site's password.
pub const KEYCHAIN_SERVICE: &str = "net.spacie.klyxdown";

pub fn keychain_entry(site: &Site) -> keyring::Result<keyring::Entry> {
    keyring::Entry::new(KEYCHAIN_SERVICE, &format!("{}@{}", site.user, site.host))
}

/// Just the host of what someone types as a site: "https://Files.Example.com/x" → "files.example.com".
pub fn site_host(text: &str) -> Option<String> {
    let text = text.trim();
    let with_scheme = if text.contains("://") {
        text.to_string()
    } else {
        format!("https://{text}")
    };
    let host = reqwest_free_host(&with_scheme)?;
    (!host.is_empty()
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':' | '[' | ']')))
    .then_some(host)
}

/// The host part of a URL, lowercased, without user info or port.
fn reqwest_free_host(url: &str) -> Option<String> {
    let rest = url.split_once("://")?.1;
    let authority = rest.split(['/', '?', '#']).next()?;
    let host = authority.rsplit('@').next()?;
    let host = if host.starts_with('[') {
        host.split(']').next().map(|h| format!("{h}]"))?
    } else {
        host.split(':').next()?.to_string()
    };
    Some(host.to_ascii_lowercase())
}

/// The saved sign-in for a host: the site itself or a parent domain ("cdn.files.test" uses
/// "files.test").
pub fn site_for<'a>(sites: &'a [Site], host: &str) -> Option<&'a Site> {
    let host = host.to_ascii_lowercase();
    sites
        .iter()
        .filter(|s| host == s.host || host.ends_with(&format!(".{}", s.host)))
        .max_by_key(|s| s.host.len())
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: "midnight".into(),
            language: "system".into(),
            run_on_startup: false,
            start_hidden: true,
            minimize_to_tray: true,
            close_to_tray: true,
            download_dir: PathBuf::new(),
            max_active: 4,
            connections: 8,
            mode: SpeedMode::Balanced,
            limit: None,
            ask_on_capture: true,
            sort_by_type: true,
            show_intro: true,
            onboarded: false,
            proxy_mode: "system".into(),
            proxy_url: String::new(),
            keep_awake: true,
            watch_clipboard: false,
            auto_update: true,
            pause_on_metered: false,
            quota_mb: None,
            quota_hours: 1,
            seed: true,
            seed_ratio: 1.0,
            handle_magnets: true,
            scan_after: true,
            scanner: String::new(),
            sites: Vec::new(),
        }
    }
}

impl Settings {
    /// Folder for a file: the download folder, plus a type subfolder when sorting is on.
    pub fn folder_for(&self, file_name: &str) -> PathBuf {
        match category_folder(file_name).filter(|_| self.sort_by_type) {
            Some(sub) => self.download_dir.join(sub),
            None => self.download_dir.clone(),
        }
    }

    /// Settings arrive from the webview, so clamp everything before trusting it.
    pub fn sanitized(mut self, app: &AppHandle) -> Self {
        if !THEMES.contains(&self.theme.as_str()) {
            self.theme = "midnight".into();
        }
        let tag = |s: &str| {
            s.len() <= 12
                && !s.is_empty()
                && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        };
        if !tag(&self.language) {
            self.language = "system".into();
        }
        if self.download_dir.as_os_str().is_empty() || !self.download_dir.is_absolute() {
            self.download_dir = app.path().download_dir().unwrap_or_default();
        }
        self.max_active = self.max_active.clamp(1, 16);
        self.connections = self.connections.clamp(1, 32);
        self.limit = self.limit.filter(|l| *l >= 16 * 1024);
        if !["system", "none", "custom"].contains(&self.proxy_mode.as_str()) {
            self.proxy_mode = "system".into();
        }
        self.proxy_url = self.proxy_url.trim().to_string();
        self.quota_mb = self.quota_mb.filter(|&mb| mb > 0);
        self.quota_hours = self.quota_hours.clamp(1, 24 * 7);
        self.scanner = self.scanner.trim().chars().take(1024).collect();
        self.sites
            .retain(|s| site_host(&s.host).as_deref() == Some(s.host.as_str()));
        self.seed_ratio = if self.seed_ratio.is_finite() {
            self.seed_ratio.clamp(0.0, 100.0)
        } else {
            1.0
        };
        self
    }

    pub fn scan(&self) -> Option<klyxdown_engine::scan::Scan> {
        use klyxdown_engine::scan::Scan;
        self.scan_after.then(|| match self.scanner.trim() {
            "" => Scan::System,
            cmd => Scan::Command(cmd.to_string()),
        })
    }

    pub fn quota(&self) -> Option<klyxdown_engine::Quota> {
        self.quota_mb.map(|mb| klyxdown_engine::Quota {
            bytes: mb.saturating_mul(1_000_000),
            hours: self.quota_hours,
        })
    }

    pub fn proxy(&self) -> klyxdown_engine::Proxy {
        match self.proxy_mode.as_str() {
            "none" => klyxdown_engine::Proxy::None,
            "custom" if !self.proxy_url.is_empty() => {
                klyxdown_engine::Proxy::Custom(self.proxy_url.clone())
            }
            _ => klyxdown_engine::Proxy::System,
        }
    }
}

/// Subfolder for a file type, like IDM and FDM sort downloads by default.
pub fn category_folder(file_name: &str) -> Option<&'static str> {
    let ext = file_name.rsplit_once('.')?.1.to_ascii_lowercase();
    Some(match ext.as_str() {
        "mp4" | "mkv" | "webm" | "mov" | "avi" | "m4v" => "Video",
        "mp3" | "flac" | "wav" | "ogg" | "m4a" | "aac" | "opus" => "Music",
        "exe" | "msi" | "msix" | "pkg" | "dmg" | "deb" | "rpm" | "appimage" | "apk" => "Programs",
        "zip" | "7z" | "rar" | "gz" | "xz" | "tar" | "bz2" | "zst" => "Archives",
        "pdf" | "epub" | "doc" | "docx" | "xls" | "xlsx" | "ppt" | "pptx" | "txt" | "csv"
        | "odt" => "Documents",
        "iso" | "img" | "vhd" | "vhdx" => "Disk images",
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "avif" | "svg" => "Images",
        "torrent" => "Torrents",
        _ => return None,
    })
}

/// Best guess of a file name from a link, before the server is asked.
pub fn name_from_url(url: &str) -> String {
    url.split(['?', '#'])
        .next()
        .unwrap_or(url)
        .rsplit('/')
        .next()
        .unwrap_or_default()
        .to_string()
}

/// Chassis color of each theme (keep in sync with src/app.css). "system" uses Midnight.
pub fn theme_background(theme: &str) -> tauri::window::Color {
    let (r, g, b) = match theme {
        "graphite" => (0x13, 0x14, 0x16),
        "fern" => (0x0d, 0x15, 0x11),
        "aluminum" => (0xdd, 0xe1, 0xe7),
        "sand" => (0xe6, 0xe1, 0xd9),
        _ => (0x0c, 0x13, 0x20),
    };
    tauri::window::Color(r, g, b, 255)
}

fn file(app: &AppHandle) -> io::Result<PathBuf> {
    crate::system::config_dir(app).map(|d| d.join("settings.json"))
}

pub fn load(app: &AppHandle) -> Settings {
    file(app)
        .and_then(fs::read)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Settings>(&bytes).ok())
        .unwrap_or_default()
        .sanitized(app)
}

pub fn save(app: &AppHandle, settings: &Settings) -> io::Result<()> {
    let path = file(app)?;
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    // Write then rename, so a crash never leaves half a settings file.
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_vec_pretty(settings)?)?;
    fs::rename(tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_sort_into_type_folders() {
        assert_eq!(category_folder("ubuntu-26.04.ISO"), Some("Disk images"));
        assert_eq!(category_folder("setup.exe"), Some("Programs"));
        assert_eq!(category_folder("notes"), None);
        assert_eq!(category_folder("archive.unknownext"), None);
    }

    #[test]
    fn sites_match_hosts_and_their_subdomains() {
        assert_eq!(
            site_host(" https://Files.Example.com:8443/x?y "),
            Some("files.example.com".into())
        );
        assert_eq!(site_host("ftp.example.org"), Some("ftp.example.org".into()));
        assert_eq!(site_host("bad host!"), None);
        let sites = vec![
            Site {
                host: "example.com".into(),
                user: "a".into(),
            },
            Site {
                host: "files.example.com".into(),
                user: "b".into(),
            },
        ];
        assert_eq!(
            site_for(&sites, "cdn.files.example.com").map(|s| s.user.as_str()),
            Some("b")
        );
        assert_eq!(
            site_for(&sites, "example.com").map(|s| s.user.as_str()),
            Some("a")
        );
        assert!(site_for(&sites, "notexample.com").is_none());
    }

    #[test]
    fn names_come_from_the_link_path() {
        assert_eq!(
            name_from_url("https://x.test/a/b/file.zip?token=1#top"),
            "file.zip"
        );
        assert_eq!(name_from_url("https://x.test/"), "");
    }
}
