use crate::parser::{State, Url, UrlPath};
use crate::percent::{utf8_percent_encode, EncodeSet};

impl Url {
    pub fn set_href(&mut self, input: &str) -> Result<(), crate::UrlError> {
        *self = Url::parse(input)?;
        Ok(())
    }

    pub fn set_protocol(&mut self, input: &str) {
        let with_colon = format!("{}:", input);
        let _ = self.basic_parse_into(&with_colon, None, Some(State::SchemeStart));
    }

    pub fn set_username(&mut self, input: &str) {
        if self.cannot_have_username_password_or_port() {
            return;
        }
        self.username = utf8_percent_encode(input, EncodeSet::UserInfo);
    }

    pub fn set_password(&mut self, input: &str) {
        if self.cannot_have_username_password_or_port() {
            return;
        }
        self.password = utf8_percent_encode(input, EncodeSet::UserInfo);
    }

    pub fn set_host(&mut self, input: &str) {
        if matches!(self.path, UrlPath::Opaque(_)) {
            return;
        }
        let _ = self.basic_parse_into(input, None, Some(State::Host));
    }

    pub fn set_hostname(&mut self, input: &str) {
        if matches!(self.path, UrlPath::Opaque(_)) {
            return;
        }
        let _ = self.basic_parse_into(input, None, Some(State::Hostname));
    }

    pub fn set_port(&mut self, input: &str) {
        if self.cannot_have_username_password_or_port() {
            return;
        }
        if input.is_empty() {
            self.port = None;
            return;
        }
        let _ = self.basic_parse_into(input, None, Some(State::Port));
    }

    pub fn set_pathname(&mut self, input: &str) {
        if matches!(self.path, UrlPath::Opaque(_)) {
            return;
        }
        self.path = UrlPath::Segments(Vec::new());
        let _ = self.basic_parse_into(input, None, Some(State::PathStart));
    }

    pub fn set_search(&mut self, input: &str) {
        if input.is_empty() {
            self.query = None;
            self.potentially_strip_trailing_spaces_from_opaque_path();
            return;
        }
        let trimmed = input.strip_prefix('?').unwrap_or(input);
        self.query = Some(String::new());
        let _ = self.basic_parse_into(trimmed, None, Some(State::Query));
    }

    pub fn set_hash(&mut self, input: &str) {
        if input.is_empty() {
            self.fragment = None;
            self.potentially_strip_trailing_spaces_from_opaque_path();
            return;
        }
        let trimmed = input.strip_prefix('#').unwrap_or(input);
        self.fragment = Some(String::new());
        let _ = self.basic_parse_into(trimmed, None, Some(State::Fragment));
    }

    fn potentially_strip_trailing_spaces_from_opaque_path(&mut self) {
        if self.fragment.is_some() || self.query.is_some() {
            return;
        }
        if let UrlPath::Opaque(ref mut s) = self.path {
            while s.ends_with(' ') {
                s.pop();
            }
        }
    }
}
