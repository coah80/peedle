use crate::parser::{default_port, is_special_scheme, parse_host, Host, Url, UrlPath};
use crate::percent::{utf8_percent_encode, EncodeSet};

impl Url {
    pub fn set_href(&mut self, input: &str) -> Result<(), crate::UrlError> {
        *self = Url::parse(input)?;
        Ok(())
    }

    pub fn set_protocol(&mut self, input: &str) {
        let mut s = input.to_string();
        if !s.ends_with(':') {
            s.push(':');
        }
        let trimmed: String = s.chars().filter(|c| !matches!(c, '\t' | '\n' | '\r')).collect();
        let Some(colon) = trimmed.find(':') else {
            return;
        };
        let scheme = trimmed[..colon].to_ascii_lowercase();
        if scheme.is_empty()
            || !scheme.as_bytes()[0].is_ascii_alphabetic()
            || !scheme
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'-' | b'.'))
        {
            return;
        }
        let was_special = is_special_scheme(&self.scheme);
        let now_special = is_special_scheme(&scheme);
        if was_special != now_special {
            return;
        }
        if scheme == "file"
            && (!self.username.is_empty() || !self.password.is_empty() || self.port.is_some())
        {
            return;
        }
        if self.scheme == "file" && matches!(self.host, None | Some(Host::Empty)) {
            return;
        }
        self.scheme = scheme;
        if let Some(p) = self.port {
            if default_port(&self.scheme) == Some(p) {
                self.port = None;
            }
        }
    }

    pub fn set_username(&mut self, input: &str) {
        if matches!(self.host, None | Some(Host::Empty)) || self.scheme == "file" {
            return;
        }
        self.username = utf8_percent_encode(input, EncodeSet::UserInfo);
    }

    pub fn set_password(&mut self, input: &str) {
        if matches!(self.host, None | Some(Host::Empty)) || self.scheme == "file" {
            return;
        }
        self.password = utf8_percent_encode(input, EncodeSet::UserInfo);
    }

    pub fn set_host(&mut self, input: &str) {
        if matches!(self.path, UrlPath::Opaque(_)) {
            return;
        }
        let cleaned: String = input
            .chars()
            .filter(|c| !matches!(c, '\t' | '\n' | '\r'))
            .collect();
        let special = is_special_scheme(&self.scheme);
        let end = cleaned
            .find(|c| matches!(c, '/' | '?' | '#') || (special && c == '\\'))
            .unwrap_or(cleaned.len());
        let part = &cleaned[..end];
        if part.is_empty() {
            if self.scheme == "file" {
                self.host = Some(Host::Empty);
            } else if !special && self.username.is_empty() && self.password.is_empty() && self.port.is_none() {
                self.host = Some(Host::Opaque(String::new()));
            }
            return;
        }
        let (host_str, port_str) = split_host_port(part);
        if let Ok(h) = parse_host(host_str, !special) {
            if let Some(ps) = port_str {
                if ps.is_empty() {
                    self.host = Some(h);
                    self.port = None;
                } else if let Ok(p) = ps.parse::<u16>() {
                    self.host = Some(h);
                    self.port = if default_port(&self.scheme) == Some(p) {
                        None
                    } else {
                        Some(p)
                    };
                }
            } else {
                self.host = Some(h);
            }
        }
    }

    pub fn set_hostname(&mut self, input: &str) {
        if matches!(self.path, UrlPath::Opaque(_)) {
            return;
        }
        let cleaned: String = input
            .chars()
            .filter(|c| !matches!(c, '\t' | '\n' | '\r'))
            .collect();
        let special = is_special_scheme(&self.scheme);
        let end = cleaned
            .find(|c| matches!(c, '/' | '?' | '#') || (special && c == '\\'))
            .unwrap_or(cleaned.len());
        let part = &cleaned[..end];
        if part.is_empty() {
            if self.scheme == "file" {
                self.host = Some(Host::Empty);
            }
            return;
        }
        if let Ok(h) = parse_host(part, !special) {
            self.host = Some(h);
        }
    }

    pub fn set_port(&mut self, input: &str) {
        if matches!(self.host, None | Some(Host::Empty)) || self.scheme == "file" {
            return;
        }
        let cleaned: String = input
            .chars()
            .filter(|c| !matches!(c, '\t' | '\n' | '\r'))
            .collect();
        if cleaned.is_empty() {
            self.port = None;
            return;
        }
        let digits: String = cleaned
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect();
        if digits.is_empty() {
            return;
        }
        if let Ok(p) = digits.parse::<u16>() {
            self.port = if default_port(&self.scheme) == Some(p) {
                None
            } else {
                Some(p)
            };
        }
    }

    pub fn set_pathname(&mut self, input: &str) {
        if matches!(self.path, UrlPath::Opaque(_)) {
            return;
        }
        let base_only = format!(
            "{}://{}{}",
            self.scheme,
            if !self.username.is_empty() || !self.password.is_empty() {
                format!("{}:{}@", self.username, self.password)
            } else {
                String::new()
            },
            self.host_str()
        );
        let rel = if input.starts_with('/') {
            input.to_string()
        } else {
            format!("/{}", input)
        };
        if let Ok(base) = Url::parse(&base_only) {
            if let Ok(next) = base.join(&rel) {
                self.path = next.path;
            }
        }
    }

    pub fn set_search(&mut self, input: &str) {
        let trimmed = input.strip_prefix('?').unwrap_or(input);
        if trimmed.is_empty() {
            self.query = None;
            return;
        }
        let cleaned: String = trimmed
            .chars()
            .filter(|c| !matches!(c, '\t' | '\n' | '\r'))
            .collect();
        let set = if is_special_scheme(&self.scheme) {
            EncodeSet::SpecialQuery
        } else {
            EncodeSet::Query
        };
        self.query = Some(utf8_percent_encode(&cleaned, set));
    }

    pub fn set_hash(&mut self, input: &str) {
        let trimmed = input.strip_prefix('#').unwrap_or(input);
        if trimmed.is_empty() {
            self.fragment = None;
            return;
        }
        let cleaned: String = trimmed
            .chars()
            .filter(|c| !matches!(c, '\t' | '\n' | '\r'))
            .collect();
        self.fragment = Some(utf8_percent_encode(&cleaned, EncodeSet::Fragment));
    }
}

fn split_host_port(s: &str) -> (&str, Option<&str>) {
    if let Some(bracket_end) = s.find(']') {
        let rest = &s[bracket_end + 1..];
        if let Some(p) = rest.strip_prefix(':') {
            return (&s[..bracket_end + 1], Some(p));
        }
        return (s, None);
    }
    match s.find(':') {
        Some(idx) => (&s[..idx], Some(&s[idx + 1..])),
        None => (s, None),
    }
}
