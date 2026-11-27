use peedle_url::Url;

#[derive(Clone, Debug)]
pub struct Cookie {
    pub name: String,
    pub value: String,
    pub domain: String,
    pub path: String,
    pub host_only: bool,
    pub secure: bool,
    pub http_only: bool,
}

#[derive(Clone, Debug, Default)]
pub struct CookieJar {
    cookies: Vec<Cookie>,
}

impl CookieJar {
    pub fn new() -> Self {
        Self {
            cookies: Vec::new(),
        }
    }

    pub fn set_from_header(&mut self, url: &Url, header_val: &str) {
        let mut parts = header_val.split(';');
        let Some(first) = parts.next() else {
            return;
        };
        let Some(eq) = first.find('=') else {
            return;
        };
        let name = first[..eq].trim().to_string();
        let value = first[eq + 1..].trim().to_string();
        if name.is_empty() {
            return;
        }
        let mut domain = url.hostname().to_ascii_lowercase();
        let mut host_only = true;
        let mut path = default_cookie_path(&url.pathname());
        let mut secure = false;
        let mut http_only = false;
        let mut expired = false;

        for attr in parts {
            let attr = attr.trim();
            let (k, v) = match attr.find('=') {
                Some(i) => (attr[..i].trim(), Some(attr[i + 1..].trim())),
                None => (attr, None),
            };
            if k.eq_ignore_ascii_case("domain") {
                if let Some(d) = v {
                    let clean = d.trim_start_matches('.').to_ascii_lowercase();
                    if !clean.is_empty() && domain_matches(&url.hostname().to_ascii_lowercase(), &clean) {
                        domain = clean;
                        host_only = false;
                    }
                }
            } else if k.eq_ignore_ascii_case("path") {
                if let Some(p) = v {
                    if p.starts_with('/') {
                        path = p.to_string();
                    }
                }
            } else if k.eq_ignore_ascii_case("secure") {
                secure = true;
            } else if k.eq_ignore_ascii_case("httponly") {
                http_only = true;
            } else if k.eq_ignore_ascii_case("max-age") {
                if let Some(ma) = v {
                    if let Ok(secs) = ma.parse::<i64>() {
                        if secs <= 0 {
                            expired = true;
                        }
                    }
                }
            }
        }

        self.cookies.retain(|c| {
            !(c.name == name && c.domain == domain && c.path == path)
        });
        if !expired {
            self.cookies.push(Cookie {
                name,
                value,
                domain,
                path,
                host_only,
                secure,
                http_only,
            });
        }
    }

    pub fn cookie_header_for(&self, url: &Url, include_http_only: bool) -> String {
        let host = url.hostname().to_ascii_lowercase();
        let path = url.pathname();
        let is_secure = url.scheme == "https" || url.scheme == "wss";
        let mut matched: Vec<&Cookie> = self
            .cookies
            .iter()
            .filter(|c| {
                if c.secure && !is_secure {
                    return false;
                }
                if c.http_only && !include_http_only {
                    return false;
                }
                let host_ok = if c.host_only {
                    host == c.domain
                } else {
                    domain_matches(&host, &c.domain)
                };
                host_ok && path_matches(&path, &c.path)
            })
            .collect();
        matched.sort_by(|a, b| b.path.len().cmp(&a.path.len()));
        matched
            .into_iter()
            .map(|c| format!("{}={}", c.name, c.value))
            .collect::<Vec<_>>()
            .join("; ")
    }
}

fn default_cookie_path(pathname: &str) -> String {
    if !pathname.starts_with('/') {
        return "/".to_string();
    }
    match pathname.rfind('/') {
        Some(0) | None => "/".to_string(),
        Some(idx) => pathname[..idx].to_string(),
    }
}

fn domain_matches(host: &str, domain: &str) -> bool {
    if host == domain {
        return true;
    }
    host.ends_with(domain)
        && host.len() > domain.len()
        && host.as_bytes()[host.len() - domain.len() - 1] == b'.'
}

fn path_matches(req_path: &str, cookie_path: &str) -> bool {
    if req_path == cookie_path {
        return true;
    }
    if req_path.starts_with(cookie_path) {
        if cookie_path.ends_with('/') {
            return true;
        }
        if req_path.as_bytes().get(cookie_path.len()) == Some(&b'/') {
            return true;
        }
    }
    false
}
