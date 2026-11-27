use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};

use peedle_dom::SharedOriginStorage;
use peedle_engine::{DownloadPayload, DownloadViewEntry, SearchEngine};
use peedle_net::CookieJar;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SavedBookmark {
    pub title: String,
    pub url: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GlobalHistoryEntry {
    pub url: String,
    pub title: String,
    pub visits: u32,
    pub last_visit: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SavedDownload {
    pub filename: String,
    pub url: String,
    pub path: String,
    pub bytes: usize,
    pub timestamp: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BrowserSettings {
    pub dark_mode: bool,
    pub search_engine: String,
    pub restore_session: bool,
    pub bookmarks_visible: bool,
    pub default_zoom: f32,
}

impl Default for BrowserSettings {
    fn default() -> Self {
        Self {
            dark_mode: true,
            search_engine: "duckduckgo".to_string(),
            restore_session: true,
            bookmarks_visible: true,
            default_zoom: 1.0,
        }
    }
}

impl BrowserSettings {
    pub fn engine(&self) -> SearchEngine {
        SearchEngine::from_id(&self.search_engine)
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SavedSession {
    pub tabs: Vec<String>,
    pub active_tab: usize,
}

pub struct BrowserProfile {
    pub dir: PathBuf,
    pub settings: BrowserSettings,
    pub bookmarks: Vec<SavedBookmark>,
    pub history: Vec<GlobalHistoryEntry>,
    pub downloads: Vec<SavedDownload>,
    pub session: SavedSession,
    pub cookies: Arc<Mutex<CookieJar>>,
    pub origin_storage: SharedOriginStorage,
}

impl BrowserProfile {
    pub fn load() -> Self {
        let dir = profile_dir();
        let _ = std::fs::create_dir_all(&dir);
        let settings: BrowserSettings = load_json(&dir.join("settings.json")).unwrap_or_default();
        let bookmarks: Vec<SavedBookmark> = load_json(&dir.join("bookmarks.json"))
            .unwrap_or_else(default_bookmarks);
        let history: Vec<GlobalHistoryEntry> =
            load_json(&dir.join("history.json")).unwrap_or_default();
        let downloads: Vec<SavedDownload> =
            load_json(&dir.join("downloads.json")).unwrap_or_default();
        let session: SavedSession = load_json(&dir.join("session.json")).unwrap_or_default();
        let jar = CookieJar::load_from_file(&dir.join("cookies.json"));
        let storage_map: HashMap<String, HashMap<String, String>> =
            load_json(&dir.join("local_storage.json")).unwrap_or_default();

        Self {
            dir,
            settings,
            bookmarks,
            history,
            downloads,
            session,
            cookies: Arc::new(Mutex::new(jar)),
            origin_storage: Arc::new(Mutex::new(storage_map)),
        }
    }

    pub fn save_settings(&self) {
        save_json(&self.dir.join("settings.json"), &self.settings);
    }

    pub fn save_bookmarks(&self) {
        save_json(&self.dir.join("bookmarks.json"), &self.bookmarks);
    }

    pub fn save_history(&self) {
        save_json(&self.dir.join("history.json"), &self.history);
    }

    pub fn save_downloads(&self) {
        save_json(&self.dir.join("downloads.json"), &self.downloads);
    }

    pub fn save_cookies(&self) {
        self.cookies
            .lock()
            .unwrap()
            .save_to_file(&self.dir.join("cookies.json"));
    }

    pub fn save_origin_storage(&self) {
        let map = self.origin_storage.lock().unwrap();
        save_json(&self.dir.join("local_storage.json"), &*map);
    }

    pub fn save_session(&mut self, tabs: Vec<String>, active_tab: usize) {
        self.session = SavedSession { tabs, active_tab };
        save_json(&self.dir.join("session.json"), &self.session);
    }

    pub fn save_all(&self) {
        self.save_settings();
        self.save_bookmarks();
        self.save_history();
        self.save_downloads();
        self.save_cookies();
        self.save_origin_storage();
    }

    pub fn record_visit(&mut self, url: &str, title: &str) {
        if url.is_empty() || url.starts_with("peedle://") || url.starts_with("about:") {
            return;
        }
        let now = chrono::Local::now().format("%Y-%m-%d %H:%M").to_string();
        if let Some(pos) = self.history.iter().position(|h| h.url == url) {
            let mut entry = self.history.remove(pos);
            entry.visits = entry.visits.saturating_add(1);
            entry.last_visit = now;
            if !title.is_empty() && title != url {
                entry.title = title.to_string();
            }
            self.history.push(entry);
        } else {
            self.history.push(GlobalHistoryEntry {
                url: url.to_string(),
                title: if title.is_empty() {
                    url.to_string()
                } else {
                    title.to_string()
                },
                visits: 1,
                last_visit: now,
            });
            if self.history.len() > 2000 {
                let drop_n = self.history.len() - 2000;
                self.history.drain(..drop_n);
            }
        }
        self.save_history();
    }

    pub fn save_download(&mut self, payload: &DownloadPayload) -> Result<SavedDownload, String> {
        let dl_dir = downloads_dir();
        let _ = std::fs::create_dir_all(&dl_dir);
        let dest = unique_dest_path(&dl_dir, &payload.filename);
        std::fs::write(&dest, &payload.bytes).map_err(|e| e.to_string())?;
        let final_name = dest
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or(&payload.filename)
            .to_string();
        let entry = SavedDownload {
            filename: final_name,
            url: payload.url.clone(),
            path: dest.to_string_lossy().into_owned(),
            bytes: payload.bytes.len(),
            timestamp: chrono::Local::now().format("%Y-%m-%d %H:%M").to_string(),
        };
        self.downloads.push(entry.clone());
        self.save_downloads();
        Ok(entry)
    }

    pub fn save_page_html(&mut self, title: &str, url: &str, html: &str) -> Result<SavedDownload, String> {
        let base: String = title
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == ' ' || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        let trimmed = base.trim();
        let fname = if trimmed.is_empty() {
            "page.html".to_string()
        } else {
            format!("{}.html", trimmed.chars().take(60).collect::<String>())
        };
        let payload = DownloadPayload {
            filename: fname,
            url: url.to_string(),
            mime: "text/html".to_string(),
            bytes: html.as_bytes().to_vec(),
        };
        self.save_download(&payload)
    }

    pub fn download_views(&self) -> Vec<DownloadViewEntry> {
        self.downloads
            .iter()
            .map(|d| DownloadViewEntry {
                filename: d.filename.clone(),
                url: d.url.clone(),
                path: d.path.clone(),
                bytes: d.bytes,
                timestamp: d.timestamp.clone(),
            })
            .collect()
    }
}

pub fn reveal_in_file_manager(path: &str) {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        let _ = Command::new("explorer.exe")
            .arg(format!("/select,{}", path))
            .creation_flags(0x08000000)
            .spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = Command::new("open").args(["-R", path]).spawn();
    }
    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    {
        let target = Path::new(path)
            .parent()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string());
        let _ = Command::new("xdg-open").arg(target).spawn();
    }
}

fn default_bookmarks() -> Vec<SavedBookmark> {
    vec![
        SavedBookmark {
            title: "Hacker News".to_string(),
            url: "https://news.ycombinator.com".to_string(),
        },
        SavedBookmark {
            title: "Wikipedia".to_string(),
            url: "https://en.wikipedia.org/wiki/Rust_(programming_language)".to_string(),
        },
        SavedBookmark {
            title: "GitHub".to_string(),
            url: "https://github.com/coah80/peedle".to_string(),
        },
        SavedBookmark {
            title: "MDN Docs".to_string(),
            url: "https://developer.mozilla.org/en-US/".to_string(),
        },
        SavedBookmark {
            title: "Demo".to_string(),
            url: "peedle://demo".to_string(),
        },
        SavedBookmark {
            title: "Settings".to_string(),
            url: "peedle://settings".to_string(),
        },
    ]
}

fn profile_dir() -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        if let Ok(local) = std::env::var("LOCALAPPDATA") {
            return PathBuf::from(local).join("Peedle").join("profile");
        }
    }
    #[cfg(target_os = "macos")]
    {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home)
                .join("Library")
                .join("Application Support")
                .join("Peedle");
        }
    }
    if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
        if !xdg.is_empty() {
            return PathBuf::from(xdg).join("peedle");
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home)
            .join(".local")
            .join("share")
            .join("peedle");
    }
    PathBuf::from(".peedle-profile")
}

pub fn downloads_dir() -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        if let Ok(profile) = std::env::var("USERPROFILE") {
            return PathBuf::from(profile).join("Downloads");
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home).join("Downloads");
    }
    PathBuf::from(".")
}

fn unique_dest_path(dir: &Path, filename: &str) -> PathBuf {
    let candidate = dir.join(filename);
    if !candidate.exists() {
        return candidate;
    }
    let p = Path::new(filename);
    let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or(filename);
    let ext = p.extension().and_then(|s| s.to_str());
    for n in 1..1000 {
        let name = match ext {
            Some(e) => format!("{} ({}).{}", stem, n, e),
            None => format!("{} ({})", stem, n),
        };
        let next = dir.join(name);
        if !next.exists() {
            return next;
        }
    }
    dir.join(filename)
}

fn load_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Option<T> {
    let data = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&data).ok()
}

fn save_json<T: Serialize>(path: &Path, value: &T) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(text) = serde_json::to_string_pretty(value) {
        let _ = std::fs::write(path, text);
    }
}
