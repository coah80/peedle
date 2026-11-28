pub mod internal_pages;
pub mod page;
pub mod youtube;

pub use internal_pages::{
    render_bookmarks_html, render_downloads_html, render_history_html, render_history_html_themed,
    render_newtab_html, render_settings_html, render_view_source_html, DownloadViewEntry,
    SettingsView,
};
pub use page::{
    fetch_navigation_background, normalize_navigation_input, normalize_navigation_input_with_engine,
    ClickAction, DownloadPayload, HistoryEntry, NetworkEntry, Page, PreparedNavigation,
    SearchEngine,
};
pub use youtube::{CaptionCue, MediaQualityOption};
