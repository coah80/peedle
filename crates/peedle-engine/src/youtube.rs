use peedle_net::{HttpClient, HttpMethod, HttpRequest};
use peedle_url::Url;
use serde_json::Value;

#[derive(Clone, Debug)]
pub struct MediaQualityOption {
    pub label: String,
    pub video_url: String,
    pub audio_url: Option<String>,
}

#[derive(Clone, Debug)]
pub struct CaptionCue {
    pub start_sec: f64,
    pub end_sec: f64,
    pub text: String,
}

#[derive(Clone, Debug)]
struct YtVideoCard {
    video_id: String,
    title: String,
    channel: String,
    meta: String,
    duration: String,
    thumbnail: String,
    snippet: String,
}

pub enum YouTubeRoute {
    Watch(String),
    Search(String),
    Home,
}

pub fn classify_youtube_url(url: &Url) -> Option<YouTubeRoute> {
    let host = url.host_str().to_ascii_lowercase();
    let is_yt = host == "youtube.com"
        || host == "www.youtube.com"
        || host == "m.youtube.com"
        || host == "music.youtube.com";
    let is_short = host == "youtu.be" || host == "www.youtu.be";
    if !is_yt && !is_short {
        return None;
    }
    let path = url.pathname();
    if is_short {
        let id = path.trim_start_matches('/').split('/').next().unwrap_or("");
        if id.len() >= 6 {
            return Some(YouTubeRoute::Watch(id.to_string()));
        }
        return Some(YouTubeRoute::Home);
    }
    if path == "/watch" || path.starts_with("/watch/") {
        for (k, v) in parse_query_pairs(&url.search()) {
            if k == "v" && !v.is_empty() {
                return Some(YouTubeRoute::Watch(v));
            }
        }
    }
    if let Some(rest) = path.strip_prefix("/shorts/") {
        let id = rest.split('/').next().unwrap_or("");
        if id.len() >= 6 {
            return Some(YouTubeRoute::Watch(id.to_string()));
        }
    }
    if let Some(rest) = path.strip_prefix("/embed/") {
        let id = rest.split('/').next().unwrap_or("");
        if id.len() >= 6 {
            return Some(YouTubeRoute::Watch(id.to_string()));
        }
    }
    if path == "/results" || path.starts_with("/results/") {
        for (k, v) in parse_query_pairs(&url.search()) {
            if k == "search_query" {
                return Some(YouTubeRoute::Search(v));
            }
        }
        return Some(YouTubeRoute::Search(String::new()));
    }
    if path == "/" || path.is_empty() || path.starts_with("/feed/") {
        return Some(YouTubeRoute::Home);
    }
    None
}

fn parse_query_pairs(search: &str) -> Vec<(String, String)> {
    let q = search.trim_start_matches('?');
    let mut out = Vec::new();
    for pair in q.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (k, v) = match pair.split_once('=') {
            Some((a, b)) => (a, b),
            None => (pair, ""),
        };
        out.push((decode_query_component(k), decode_query_component(v)));
    }
    out
}

fn decode_query_component(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'+' {
            out.push(b' ');
            i += 1;
        } else if bytes[i] == b'%' && i + 2 < bytes.len() {
            let h1 = (bytes[i + 1] as char).to_digit(16);
            let h2 = (bytes[i + 2] as char).to_digit(16);
            if let (Some(a), Some(b)) = (h1, h2) {
                out.push(((a << 4) | b) as u8);
                i += 3;
                continue;
            }
            out.push(bytes[i]);
            i += 1;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub fn build_youtube_html(
    client: &HttpClient,
    url: &Url,
    dark_mode: bool,
) -> Option<String> {
    let route = classify_youtube_url(url)?;
    match route {
        YouTubeRoute::Watch(video_id) => Some(build_watch_page(client, &video_id, dark_mode)),
        YouTubeRoute::Search(query) => Some(build_search_page(client, &query, dark_mode)),
        YouTubeRoute::Home => Some(build_home_page(client, dark_mode)),
    }
}

fn post_innertube(client: &HttpClient, endpoint: &str, body_json: &str, android: bool) -> Option<Value> {
    let url = Url::parse(&format!(
        "https://www.youtube.com/youtubei/v1/{}?prettyPrint=false",
        endpoint
    ))
    .ok()?;
    let ua = if android {
        "com.google.android.youtube/20.10.38 (Linux; U; Android 14) gzip"
    } else {
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/134.0.0.0 Safari/537.36"
    };
    let req = HttpRequest {
        method: HttpMethod::Post,
        url,
        headers: vec![
            ("Content-Type".to_string(), "application/json".to_string()),
            ("User-Agent".to_string(), ua.to_string()),
        ],
        body: Some(body_json.as_bytes().to_vec()),
        max_redirects: 5,
    };
    let resp = client.send(req).ok()?;
    if resp.status >= 400 {
        return None;
    }
    serde_json::from_slice(&resp.body).ok()
}

fn fetch_player_json(client: &HttpClient, video_id: &str) -> Option<Value> {
    let body = serde_json::json!({
        "context": {
            "client": {
                "clientName": "ANDROID",
                "clientVersion": "20.10.38",
                "androidSdkVersion": 34,
                "hl": "en",
                "gl": "US"
            }
        },
        "videoId": video_id,
        "contentCheckOk": true,
        "racyCheckOk": true
    })
    .to_string();
    post_innertube(client, "player", &body, true)
}

fn fetch_next_json(client: &HttpClient, video_id: &str) -> Option<Value> {
    let body = serde_json::json!({
        "context": {
            "client": {
                "clientName": "WEB",
                "clientVersion": "2.20250310.01.00",
                "hl": "en",
                "gl": "US"
            }
        },
        "videoId": video_id
    })
    .to_string();
    post_innertube(client, "next", &body, false)
}

fn fetch_search_json(client: &HttpClient, query: &str) -> Option<Value> {
    let body = serde_json::json!({
        "context": {
            "client": {
                "clientName": "WEB",
                "clientVersion": "2.20250310.01.00",
                "hl": "en",
                "gl": "US"
            }
        },
        "query": query
    })
    .to_string();
    post_innertube(client, "search", &body, false)
}

fn extract_qualities(player: &Value) -> Vec<MediaQualityOption> {
    let mut out = Vec::new();
    let sd = &player["streamingData"];
    let audio_url = sd["adaptiveFormats"]
        .as_array()
        .and_then(|arr| {
            arr.iter()
                .find(|f| {
                    f["mimeType"]
                        .as_str()
                        .map_or(false, |m| m.contains("audio/mp4") && m.contains("mp4a.40.2"))
                        && f["url"].is_string()
                })
                .or_else(|| {
                    arr.iter().find(|f| {
                        f["mimeType"]
                            .as_str()
                            .map_or(false, |m| m.contains("audio/webm") && m.contains("opus"))
                            && f["url"].is_string()
                    })
                })
        })
        .and_then(|f| f["url"].as_str().map(|s| s.to_string()));

    let mut seen_labels = std::collections::HashSet::new();
    if let Some(afmts) = sd["adaptiveFormats"].as_array() {
        let mut h264: Vec<(u64, String, String)> = Vec::new();
        for f in afmts {
            let mime = f["mimeType"].as_str().unwrap_or("");
            let Some(u) = f["url"].as_str() else {
                continue;
            };
            if mime.contains("video/mp4") && mime.contains("avc1") {
                let h = f["height"].as_u64().unwrap_or(0);
                let label = f["qualityLabel"]
                    .as_str()
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| format!("{}p", h));
                if h > 0 && h <= 1080 {
                    h264.push((h, label, u.to_string()));
                }
            }
        }
        h264.sort_by(|a, b| b.0.cmp(&a.0));
        for (_, label, u) in h264 {
            if seen_labels.insert(label.clone()) {
                out.push(MediaQualityOption {
                    label,
                    video_url: u,
                    audio_url: audio_url.clone(),
                });
            }
        }
    }

    if let Some(fmts) = sd["formats"].as_array() {
        for f in fmts {
            if let Some(u) = f["url"].as_str() {
                let label = f["qualityLabel"]
                    .as_str()
                    .unwrap_or("360p")
                    .to_string();
                let key = format!("{} mux", label);
                if seen_labels.insert(key) {
                    out.push(MediaQualityOption {
                        label,
                        video_url: u.to_string(),
                        audio_url: None,
                    });
                }
            }
        }
    }

    if let Some(pos) = out.iter().position(|q| q.audio_url.is_none()) {
        let preferred = out.remove(pos);
        out.insert(0, preferred);
    } else if let Some(pos) = out.iter().position(|q| q.label.starts_with("720p")) {
        let preferred = out.remove(pos);
        out.insert(0, preferred);
    } else if let Some(pos) = out.iter().position(|q| q.label.starts_with("480p") || q.label.starts_with("360p")) {
        let preferred = out.remove(pos);
        out.insert(0, preferred);
    }

    out
}

fn extract_caption_url(player: &Value) -> Option<String> {
    let tracks = player["captions"]["playerCaptionsTracklistRenderer"]["captionTracks"]
        .as_array()?;
    let chosen = tracks
        .iter()
        .find(|t| t["languageCode"].as_str().map_or(false, |c| c.starts_with("en")))
        .or_else(|| tracks.first())?;
    chosen["baseUrl"].as_str().map(|s| s.to_string())
}

pub fn parse_timedtext_xml(xml: &str) -> Vec<CaptionCue> {
    let mut cues = Vec::new();
    let mut rest = xml;
    while let Some(idx) = rest.find("<text ") {
        rest = &rest[idx + 6..];
        let Some(gt) = rest.find('>') else {
            break;
        };
        let attrs = &rest[..gt];
        rest = &rest[gt + 1..];
        let Some(end_tag) = rest.find("</text>") else {
            break;
        };
        let raw_body = &rest[..end_tag];
        rest = &rest[end_tag + 7..];
        let start = extract_xml_attr(attrs, "start")
            .and_then(|s| s.parse::<f64>().ok())
            .unwrap_or(0.0);
        let dur = extract_xml_attr(attrs, "dur")
            .and_then(|s| s.parse::<f64>().ok())
            .unwrap_or(2.5);
        let text = decode_xml_entities(raw_body)
            .replace('\n', " ")
            .trim()
            .to_string();
        if !text.is_empty() {
            cues.push(CaptionCue {
                start_sec: start,
                end_sec: start + dur.max(0.5),
                text,
            });
        }
    }
    cues
}

fn extract_xml_attr<'a>(attrs: &'a str, name: &str) -> Option<&'a str> {
    let needle = format!("{}=\"", name);
    let idx = attrs.find(&needle)?;
    let rest = &attrs[idx + needle.len()..];
    let end = rest.find('"')?;
    Some(&rest[..end])
}

fn decode_xml_entities(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&#x27;", "'")
}

fn extract_text_val(v: &Value) -> String {
    if let Some(s) = v.as_str() {
        return s.to_string();
    }
    if let Some(s) = v["simpleText"].as_str() {
        return s.to_string();
    }
    if let Some(s) = v["content"].as_str() {
        return s.to_string();
    }
    if let Some(runs) = v["runs"].as_array() {
        let mut out = String::new();
        for r in runs {
            if let Some(t) = r["text"].as_str() {
                out.push_str(t);
            }
        }
        return out;
    }
    String::new()
}

fn collect_video_cards(v: &Value, out: &mut Vec<YtVideoCard>, limit: usize) {
    if out.len() >= limit {
        return;
    }
    match v {
        Value::Object(map) => {
            if let Some(vr) = map.get("videoRenderer") {
                if let Some(card) = parse_video_renderer(vr) {
                    if !out.iter().any(|c| c.video_id == card.video_id) {
                        out.push(card);
                    }
                }
                return;
            }
            if let Some(cvr) = map.get("compactVideoRenderer") {
                if let Some(card) = parse_video_renderer(cvr) {
                    if !out.iter().any(|c| c.video_id == card.video_id) {
                        out.push(card);
                    }
                }
                return;
            }
            if let Some(lvm) = map.get("lockupViewModel") {
                if let Some(card) = parse_lockup_view_model(lvm) {
                    if !out.iter().any(|c| c.video_id == card.video_id) {
                        out.push(card);
                    }
                }
                return;
            }
            for child in map.values() {
                collect_video_cards(child, out, limit);
                if out.len() >= limit {
                    break;
                }
            }
        }
        Value::Array(arr) => {
            for item in arr {
                collect_video_cards(item, out, limit);
                if out.len() >= limit {
                    break;
                }
            }
        }
        _ => {}
    }
}

fn parse_video_renderer(vr: &Value) -> Option<YtVideoCard> {
    let video_id = vr["videoId"].as_str()?.to_string();
    let title = extract_text_val(&vr["title"]);
    if title.is_empty() {
        return None;
    }
    let channel = extract_text_val(&vr["ownerText"])
        .if_empty_or(|| extract_text_val(&vr["longBylineText"]))
        .if_empty_or(|| extract_text_val(&vr["shortBylineText"]));
    let views = extract_text_val(&vr["shortViewCountText"])
        .if_empty_or(|| extract_text_val(&vr["viewCountText"]));
    let published = extract_text_val(&vr["publishedTimeText"]);
    let meta = match (views.is_empty(), published.is_empty()) {
        (false, false) => format!("{} \u{2022} {}", views, published),
        (false, true) => views,
        (true, false) => published,
        (true, true) => String::new(),
    };
    let duration = extract_text_val(&vr["lengthText"]);
    let thumbnail = format!("https://i.ytimg.com/vi/{}/mqdefault.jpg", video_id);
    let snippet = vr["detailedMetadataSnippets"]
        .as_array()
        .and_then(|a| a.first())
        .map(|s| extract_text_val(&s["snippetText"]))
        .unwrap_or_default();
    Some(YtVideoCard {
        video_id,
        title,
        channel,
        meta,
        duration,
        thumbnail,
        snippet,
    })
}

fn parse_lockup_view_model(lvm: &Value) -> Option<YtVideoCard> {
    let video_id = lvm["contentId"].as_str()?.to_string();
    if video_id.len() != 11 {
        return None;
    }
    let title = extract_text_val(&lvm["metadata"]["lockupMetadataViewModel"]["title"]);
    if title.is_empty() {
        return None;
    }
    let rows = lvm["metadata"]["lockupMetadataViewModel"]["metadata"]["contentMetadataViewModel"]
        ["metadataRows"]
        .as_array();
    let mut channel = String::new();
    let mut meta_parts = Vec::new();
    if let Some(rows) = rows {
        for (idx, row) in rows.iter().enumerate() {
            if let Some(parts) = row["metadataParts"].as_array() {
                for p in parts {
                    let t = extract_text_val(&p["text"]);
                    if t.is_empty() {
                        continue;
                    }
                    if idx == 0 && channel.is_empty() {
                        channel = t;
                    } else {
                        meta_parts.push(t);
                    }
                }
            }
        }
    }
    let meta = meta_parts.join(" \u{2022} ");
    let duration = find_duration_badge(lvm).unwrap_or_default();
    let thumbnail = format!("https://i.ytimg.com/vi/{}/mqdefault.jpg", video_id);
    Some(YtVideoCard {
        video_id,
        title,
        channel,
        meta,
        duration,
        thumbnail,
        snippet: String::new(),
    })
}

fn find_duration_badge(v: &Value) -> Option<String> {
    match v {
        Value::Object(map) => {
            if let Some(t) = map.get("thumbnailBadgeViewModel") {
                if let Some(s) = t["text"].as_str() {
                    if !s.is_empty() {
                        return Some(s.to_string());
                    }
                }
            }
            for child in map.values() {
                if let Some(found) = find_duration_badge(child) {
                    return Some(found);
                }
            }
            None
        }
        Value::Array(arr) => {
            for item in arr {
                if let Some(found) = find_duration_badge(item) {
                    return Some(found);
                }
            }
            None
        }
        _ => None,
    }
}

trait StrFallback {
    fn if_empty_or<F: FnOnce() -> String>(self, f: F) -> String;
}

impl StrFallback for String {
    fn if_empty_or<F: FnOnce() -> String>(self, f: F) -> String {
        if self.is_empty() { f() } else { self }
    }
}

fn youtube_base_css(dark: bool) -> String {
    let (bg, surface, surface2, border, fg, muted, chip_bg, chip_active_bg, chip_active_fg) =
        if dark {
            (
                "#0f0f0f", "#272727", "#1f1f1f", "#303030", "#f1f1f1", "#aaaaaa", "#272727",
                "#f1f1f1", "#0f0f0f",
            )
        } else {
            (
                "#ffffff", "#f2f2f2", "#f8f8f8", "#d9d9d9", "#0f0f0f", "#606060", "#f2f2f2",
                "#0f0f0f", "#ffffff",
            )
        };
    format!(
        r#"*{{box-sizing:border-box;}}
body{{margin:0;background:{bg};color:{fg};font-family:Roboto,-apple-system,BlinkMacSystemFont,'Segoe UI',Arial,sans-serif;font-size:14px;line-height:1.4;}}
a{{color:inherit;text-decoration:none;}}
.yt-hdr{{position:sticky;top:0;z-index:20;height:56px;background:{bg};display:flex;align-items:center;justify-content:space-between;padding:0 16px;gap:16px;}}
.yt-brand{{display:flex;align-items:center;gap:6px;font-weight:700;font-size:19px;letter-spacing:-0.5px;white-space:nowrap;}}
.yt-logo-pill{{display:inline-flex;align-items:center;justify-content:center;width:30px;height:21px;border-radius:6px;background:#ff0033;color:#ffffff;font-size:11px;}}
.yt-search-form{{display:flex;align-items:center;flex:1;max-width:600px;margin:0 auto;}}
.yt-search-input{{flex:1;height:40px;padding:0 16px;border:1px solid {border};border-right:none;border-radius:20px 0 0 20px;background:{bg};color:{fg};font-size:15px;outline:none;}}
.yt-search-btn{{height:40px;width:64px;display:inline-flex;align-items:center;justify-content:center;border:1px solid {border};border-radius:0 20px 20px 0;background:{surface2};color:{fg};cursor:pointer;font-size:14px;font-weight:500;padding:0;}}
.yt-hdr-right{{display:flex;align-items:center;gap:10px;white-space:nowrap;}}
.yt-pill-btn{{display:inline-flex;align-items:center;gap:6px;height:36px;padding:0 14px;border-radius:18px;background:{surface};color:{fg};font-weight:500;font-size:13px;}}
.yt-sub-btn{{display:inline-flex;align-items:center;height:36px;padding:0 16px;border-radius:18px;background:{chip_active_bg};color:{chip_active_fg};font-weight:600;font-size:13px;}}
.yt-watch-wrap{{max-width:1660px;margin:0 auto;padding:12px 24px 40px 24px;display:grid;grid-template-columns:minmax(0,1fr) 402px;gap:24px;align-items:start;}}
.yt-player-box{{width:100%;aspect-ratio:16 / 9;background:#000000;border-radius:12px;overflow:hidden;}}
.yt-player-box video{{display:block;width:100%;height:100%;border-radius:12px;}}
.yt-title{{margin:14px 0 10px 0;font-size:20px;font-weight:700;line-height:1.3;}}
.yt-channel-row{{display:flex;align-items:center;justify-content:space-between;flex-wrap:wrap;gap:12px;margin-bottom:14px;}}
.yt-channel-left{{display:flex;align-items:center;gap:12px;}}
.yt-avatar{{width:40px;height:40px;border-radius:20px;background:#cc0000;color:#ffffff;display:flex;align-items:center;justify-content:center;font-weight:700;font-size:16px;overflow:hidden;flex-shrink:0;}}
.yt-avatar img{{width:40px;height:40px;object-fit:cover;}}
.yt-ch-name{{font-weight:600;font-size:15px;}}
.yt-ch-sub{{color:{muted};font-size:12px;}}
.yt-desc-box{{background:{surface};border-radius:12px;padding:14px 16px;font-size:13.5px;line-height:1.5;}}
.yt-desc-meta{{font-weight:600;margin-bottom:6px;}}
.yt-desc-body{{white-space:pre-wrap;color:{fg};}}
.yt-side-list{{display:flex;flex-direction:column;gap:10px;}}
.yt-rel-card{{display:flex;gap:10px;align-items:flex-start;}}
.yt-rel-thumb{{position:relative;width:168px;height:94px;border-radius:8px;overflow:hidden;background:#181818;flex-shrink:0;}}
.yt-rel-thumb img{{width:168px;height:94px;object-fit:cover;display:block;}}
.yt-dur{{position:absolute;right:4px;bottom:4px;background:rgba(0,0,0,0.82);color:#ffffff;font-size:11px;font-weight:600;padding:1px 5px;border-radius:4px;}}
.yt-rel-info{{flex:1;min-width:0;}}
.yt-rel-title{{font-weight:600;font-size:13.5px;line-height:1.3;margin-bottom:4px;}}
.yt-rel-meta{{color:{muted};font-size:12px;line-height:1.35;}}
.yt-shell{{display:grid;grid-template-columns:72px minmax(0,1fr);min-height:740px;}}
.yt-rail{{display:flex;flex-direction:column;align-items:center;gap:6px;padding:8px 4px;}}
.yt-rail-item{{display:flex;flex-direction:column;align-items:center;justify-content:center;width:64px;padding:12px 4px;border-radius:10px;font-size:10px;gap:4px;color:{fg};}}
.yt-main{{padding:8px 24px 36px 24px;}}
.yt-chips{{display:flex;flex-wrap:wrap;gap:10px;margin-bottom:20px;}}
.yt-chip{{display:inline-flex;align-items:center;height:32px;padding:0 12px;border-radius:8px;background:{chip_bg};color:{fg};font-size:13px;font-weight:500;}}
.yt-chip-active{{background:{chip_active_bg};color:{chip_active_fg};}}
.yt-grid{{display:grid;grid-template-columns:repeat(3,minmax(0,1fr));gap:20px 16px;}}
.yt-card{{display:flex;flex-direction:column;gap:10px;}}
.yt-card-thumb{{position:relative;width:100%;aspect-ratio:16 / 9;border-radius:12px;overflow:hidden;background:#181818;}}
.yt-card-thumb img{{width:100%;height:100%;object-fit:cover;display:block;}}
.yt-card-title{{font-weight:600;font-size:15px;line-height:1.35;margin-bottom:4px;}}
.yt-card-meta{{color:{muted};font-size:13px;}}
.yt-search-list{{max-width:1080px;display:flex;flex-direction:column;gap:16px;}}
.yt-search-row{{display:flex;gap:16px;align-items:flex-start;}}
.yt-search-thumb{{position:relative;width:360px;height:202px;border-radius:12px;overflow:hidden;background:#181818;flex-shrink:0;}}
.yt-search-thumb img{{width:360px;height:202px;object-fit:cover;display:block;}}
.yt-search-title{{font-size:18px;font-weight:600;line-height:1.35;margin-bottom:6px;}}
.yt-search-snippet{{color:{muted};font-size:13px;margin-top:8px;line-height:1.45;}}"#,
        bg = bg,
        surface = surface,
        surface2 = surface2,
        border = border,
        fg = fg,
        muted = muted,
        chip_bg = chip_bg,
        chip_active_bg = chip_active_bg,
        chip_active_fg = chip_active_fg
    )
}

fn youtube_header_html(search_value: &str) -> String {
    format!(
        r#"<header class="yt-hdr">
<a class="yt-brand" href="https://www.youtube.com/"><span class="yt-logo-pill">&#9654;</span><span>YouTube</span></a>
<form class="yt-search-form" action="https://www.youtube.com/results" method="get">
<input class="yt-search-input" type="text" name="search_query" value="{q}" placeholder="Search">
<button class="yt-search-btn" type="submit"><svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="2"><circle cx="11" cy="11" r="7"/><line x1="16.5" y1="16.5" x2="21" y2="21"/></svg></button>
</form>
<div class="yt-hdr-right"><a class="yt-pill-btn" href="https://www.youtube.com/">Home</a></div>
</header>"#,
        q = escape_html_attr(search_value)
    )
}

fn format_views(raw: &str) -> String {
    if let Ok(n) = raw.parse::<u64>() {
        if n >= 1_000_000_000 {
            return format!("{:.1}B views", n as f64 / 1_000_000_000.0);
        }
        if n >= 1_000_000 {
            return format!("{:.1}M views", n as f64 / 1_000_000.0);
        }
        if n >= 1_000 {
            return format!("{:.1}K views", n as f64 / 1_000.0);
        }
        return format!("{} views", n);
    }
    raw.to_string()
}

fn build_watch_page(client: &HttpClient, video_id: &str, dark_mode: bool) -> String {
    let c1 = client.clone();
    let c2 = client.clone();
    let vid1 = video_id.to_string();
    let vid2 = video_id.to_string();
    let (player_opt, next_opt) = std::thread::scope(|s| {
        let h1 = s.spawn(move || fetch_player_json(&c1, &vid1));
        let h2 = s.spawn(move || fetch_next_json(&c2, &vid2));
        (h1.join().ok().flatten(), h2.join().ok().flatten())
    });

    let player = player_opt.unwrap_or(Value::Null);
    let vd = &player["videoDetails"];
    let title = vd["title"].as_str().unwrap_or("YouTube Video").to_string();
    let author = vd["author"].as_str().unwrap_or("YouTube").to_string();
    let views = format_views(vd["viewCount"].as_str().unwrap_or(""));
    let desc = vd["shortDescription"].as_str().unwrap_or("").to_string();
    let poster = format!("https://i.ytimg.com/vi/{}/hqdefault.jpg", video_id);
    let qualities = extract_qualities(&player);
    let caption_url = extract_caption_url(&player);

    let (primary_video, primary_audio) = qualities
        .first()
        .map(|q| (q.video_url.clone(), q.audio_url.clone()))
        .unwrap_or_default();

    let qualities_json = serde_json::to_string(
        &qualities
            .iter()
            .map(|q| {
                serde_json::json!({
                    "label": q.label,
                    "video": q.video_url,
                    "audio": q.audio_url,
                })
            })
            .collect::<Vec<_>>(),
    )
    .unwrap_or_else(|_| "[]".to_string());

    let mut related = Vec::new();
    if let Some(ref nx) = next_opt {
        collect_video_cards(nx, &mut related, 18);
    }
    related.retain(|c| c.video_id != video_id);

    let mut side_html = String::new();
    for card in &related {
        let dur_badge = if card.duration.is_empty() {
            String::new()
        } else {
            format!("<span class=\"yt-dur\">{}</span>", escape_html(&card.duration))
        };
        side_html.push_str(&format!(
            r#"<a class="yt-rel-card" href="https://www.youtube.com/watch?v={id}">
<div class="yt-rel-thumb"><img src="{thumb}" alt="">{dur}</div>
<div class="yt-rel-info">
<div class="yt-rel-title">{title}</div>
<div class="yt-rel-meta">{ch}</div>
<div class="yt-rel-meta">{meta}</div>
</div>
</a>"#,
            id = escape_html_attr(&card.video_id),
            thumb = escape_html_attr(&card.thumbnail),
            dur = dur_badge,
            title = escape_html(&card.title),
            ch = escape_html(&card.channel),
            meta = escape_html(&card.meta),
        ));
    }

    let initial = author
        .chars()
        .next()
        .unwrap_or('Y')
        .to_uppercase()
        .to_string();
    let audio_attr = primary_audio
        .as_deref()
        .map(|u| format!(" data-peedle-audio-src=\"{}\"", escape_html_attr(u)))
        .unwrap_or_default();
    let cap_attr = caption_url
        .as_deref()
        .map(|u| format!(" data-peedle-captions=\"{}\"", escape_html_attr(u)))
        .unwrap_or_default();
    let short_desc: String = desc.chars().take(1200).collect();

    format!(
        r#"<!DOCTYPE html><html><head><meta charset="utf-8"><title>{title} - YouTube</title><style>{css}</style></head><body>
{hdr}
<main class="yt-watch-wrap">
<section>
<div class="yt-player-box">
<video controls autoplay playsinline src="{vsrc}" poster="{poster}"{audio_attr}{cap_attr} data-peedle-qualities="{qjson}"></video>
</div>
<h1 class="yt-title">{title}</h1>
<div class="yt-channel-row">
<div class="yt-channel-left">
<div class="yt-avatar">{init}</div>
<div><div class="yt-ch-name">{author}</div><div class="yt-ch-sub">{views}</div></div>
<span class="yt-sub-btn">Subscribe</span>
</div>
<div class="yt-hdr-right">
<span class="yt-pill-btn">Like</span>
<span class="yt-pill-btn">Share</span>
</div>
</div>
<div class="yt-desc-box">
<div class="yt-desc-meta">{views}</div>
<div class="yt-desc-body">{desc}</div>
</div>
</section>
<aside class="yt-side-list">{side}</aside>
</main>
</body></html>"#,
        title = escape_html(&title),
        css = youtube_base_css(dark_mode),
        hdr = youtube_header_html(""),
        vsrc = escape_html_attr(&primary_video),
        poster = escape_html_attr(&poster),
        audio_attr = audio_attr,
        cap_attr = cap_attr,
        qjson = escape_html_attr(&qualities_json),
        init = escape_html(&initial),
        author = escape_html(&author),
        views = escape_html(&views),
        desc = escape_html(&short_desc),
        side = side_html,
    )
}

fn build_search_page(client: &HttpClient, query: &str, dark_mode: bool) -> String {
    let q = if query.trim().is_empty() {
        "featured videos"
    } else {
        query.trim()
    };
    let mut cards = Vec::new();
    if let Some(json) = fetch_search_json(client, q) {
        collect_video_cards(&json, &mut cards, 20);
    }
    let mut rows_html = String::new();
    for card in &cards {
        let dur_badge = if card.duration.is_empty() {
            String::new()
        } else {
            format!("<span class=\"yt-dur\">{}</span>", escape_html(&card.duration))
        };
        let snip = if card.snippet.is_empty() {
            String::new()
        } else {
            format!(
                "<div class=\"yt-search-snippet\">{}</div>",
                escape_html(&card.snippet)
            )
        };
        rows_html.push_str(&format!(
            r#"<a class="yt-search-row" href="https://www.youtube.com/watch?v={id}">
<div class="yt-search-thumb"><img src="{thumb}" alt="">{dur}</div>
<div>
<div class="yt-search-title">{title}</div>
<div class="yt-card-meta">{meta}</div>
<div class="yt-card-meta" style="margin-top:6px;font-weight:500;">{ch}</div>
{snip}
</div>
</a>"#,
            id = escape_html_attr(&card.video_id),
            thumb = escape_html_attr(&card.thumbnail),
            dur = dur_badge,
            title = escape_html(&card.title),
            meta = escape_html(&card.meta),
            ch = escape_html(&card.channel),
            snip = snip,
        ));
    }

    format!(
        r#"<!DOCTYPE html><html><head><meta charset="utf-8"><title>{q} - YouTube</title><style>{css}</style></head><body>
{hdr}
<div class="yt-shell">
<nav class="yt-rail">
<a class="yt-rail-item" href="https://www.youtube.com/"><strong>Home</strong></a>
<a class="yt-rail-item" href="https://www.youtube.com/results?search_query=shorts"><span>Shorts</span></a>
<a class="yt-rail-item" href="https://www.youtube.com/results?search_query=trending"><span>Trending</span></a>
</nav>
<main class="yt-main"><div class="yt-search-list">{rows}</div></main>
</div>
</body></html>"#,
        q = escape_html(q),
        css = youtube_base_css(dark_mode),
        hdr = youtube_header_html(query),
        rows = rows_html,
    )
}

fn build_home_page(client: &HttpClient, dark_mode: bool) -> String {
    let c1 = client.clone();
    let c2 = client.clone();
    let c3 = client.clone();
    let (r1, r2, r3) = std::thread::scope(|s| {
        let h1 = s.spawn(move || fetch_search_json(&c1, "Veritasium OR Kurzgesagt OR 3Blue1Brown"));
        let h2 = s.spawn(move || fetch_search_json(&c2, "Fireship OR Marques Brownlee OR Linus Tech Tips"));
        let h3 = s.spawn(move || fetch_search_json(&c3, "Mark Rober OR SmarterEveryDay OR Computerphile"));
        (
            h1.join().ok().flatten(),
            h2.join().ok().flatten(),
            h3.join().ok().flatten(),
        )
    });
    let mut b1 = Vec::new();
    let mut b2 = Vec::new();
    let mut b3 = Vec::new();
    if let Some(ref j) = r1 {
        collect_video_cards(j, &mut b1, 8);
    }
    if let Some(ref j) = r2 {
        collect_video_cards(j, &mut b2, 8);
    }
    if let Some(ref j) = r3 {
        collect_video_cards(j, &mut b3, 8);
    }
    let mut cards = Vec::new();
    for i in 0..8 {
        for bucket in [&b1, &b2, &b3] {
            if let Some(card) = bucket.get(i) {
                if !cards.iter().any(|c: &YtVideoCard| c.video_id == card.video_id) {
                    cards.push(card.clone());
                }
            }
        }
        if cards.len() >= 18 {
            break;
        }
    }
    let chips = [
        ("All", "https://www.youtube.com/", true),
        ("Music", "https://www.youtube.com/results?search_query=music", false),
        ("Gaming", "https://www.youtube.com/results?search_query=gaming", false),
        ("Programming", "https://www.youtube.com/results?search_query=programming", false),
        ("Rust", "https://www.youtube.com/results?search_query=rust+programming", false),
        ("Science", "https://www.youtube.com/results?search_query=science", false),
        ("News", "https://www.youtube.com/results?search_query=news", false),
    ];
    let mut chips_html = String::new();
    for (label, href, active) in chips {
        let cls = if active { "yt-chip yt-chip-active" } else { "yt-chip" };
        chips_html.push_str(&format!(
            "<a class=\"{}\" href=\"{}\">{}</a>",
            cls, href, label
        ));
    }
    let mut grid_html = String::new();
    for card in &cards {
        let dur_badge = if card.duration.is_empty() {
            String::new()
        } else {
            format!("<span class=\"yt-dur\">{}</span>", escape_html(&card.duration))
        };
        grid_html.push_str(&format!(
            r#"<a class="yt-card" href="https://www.youtube.com/watch?v={id}">
<div class="yt-card-thumb"><img src="{thumb}" alt="">{dur}</div>
<div>
<div class="yt-card-title">{title}</div>
<div class="yt-card-meta">{ch}</div>
<div class="yt-card-meta">{meta}</div>
</div>
</a>"#,
            id = escape_html_attr(&card.video_id),
            thumb = escape_html_attr(&card.thumbnail),
            dur = dur_badge,
            title = escape_html(&card.title),
            ch = escape_html(&card.channel),
            meta = escape_html(&card.meta),
        ));
    }
    format!(
        r#"<!DOCTYPE html><html><head><meta charset="utf-8"><title>YouTube</title><style>{css}</style></head><body>
{hdr}
<div class="yt-shell">
<nav class="yt-rail">
<a class="yt-rail-item" href="https://www.youtube.com/"><strong>Home</strong></a>
<a class="yt-rail-item" href="https://www.youtube.com/results?search_query=shorts"><span>Shorts</span></a>
<a class="yt-rail-item" href="https://www.youtube.com/results?search_query=trending"><span>Trending</span></a>
</nav>
<main class="yt-main">
<div class="yt-chips">{chips}</div>
<div class="yt-grid">{grid}</div>
</main>
</div>
</body></html>"#,
        css = youtube_base_css(dark_mode),
        hdr = youtube_header_html(""),
        chips = chips_html,
        grid = grid_html,
    )
}

fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn escape_html_attr(s: &str) -> String {
    escape_html(s).replace('"', "&quot;").replace('\'', "&#39;")
}
