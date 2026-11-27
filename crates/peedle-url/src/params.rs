use crate::percent::{form_urlencoded_decode, utf8_percent_encode, EncodeSet};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UrlSearchParams {
    entries: Vec<(String, String)>,
}

impl UrlSearchParams {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    pub fn parse(input: &str) -> Self {
        let trimmed = input.strip_prefix('?').unwrap_or(input);
        let mut entries = Vec::new();
        if !trimmed.is_empty() {
            for part in trimmed.split('&') {
                if part.is_empty() {
                    continue;
                }
                let (name, value) = match part.find('=') {
                    Some(idx) => (&part[..idx], &part[idx + 1..]),
                    None => (part, ""),
                };
                entries.push((
                    form_urlencoded_decode(name),
                    form_urlencoded_decode(value),
                ));
            }
        }
        Self { entries }
    }

    pub fn append(&mut self, name: impl Into<String>, value: impl Into<String>) {
        self.entries.push((name.into(), value.into()));
    }

    pub fn delete(&mut self, name: &str, value: Option<&str>) {
        self.entries.retain(|(k, v)| {
            if k != name {
                true
            } else if let Some(target) = value {
                v != target
            } else {
                false
            }
        });
    }

    pub fn get(&self, name: &str) -> Option<&str> {
        self.entries
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }

    pub fn get_all(&self, name: &str) -> Vec<&str> {
        self.entries
            .iter()
            .filter(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
            .collect()
    }

    pub fn has(&self, name: &str, value: Option<&str>) -> bool {
        self.entries.iter().any(|(k, v)| {
            k == name && value.map_or(true, |target| v == target)
        })
    }

    pub fn set(&mut self, name: impl Into<String>, value: impl Into<String>) {
        let name = name.into();
        let value = value.into();
        let mut found = false;
        let mut next = Vec::with_capacity(self.entries.len() + 1);
        for (k, v) in self.entries.drain(..) {
            if k == name {
                if !found {
                    next.push((name.clone(), value.clone()));
                    found = true;
                }
            } else {
                next.push((k, v));
            }
        }
        if !found {
            next.push((name, value));
        }
        self.entries = next;
    }

    pub fn sort(&mut self) {
        self.entries.sort_by(|a, b| {
            let a_u16: Vec<u16> = a.0.encode_utf16().collect();
            let b_u16: Vec<u16> = b.0.encode_utf16().collect();
            a_u16.cmp(&b_u16)
        });
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn entries(&self) -> &[(String, String)] {
        &self.entries
    }

    pub fn serialize(&self) -> String {
        let mut out = String::new();
        for (i, (k, v)) in self.entries.iter().enumerate() {
            if i > 0 {
                out.push('&');
            }
            out.push_str(&utf8_percent_encode(k, EncodeSet::FormUrlEncoded));
            out.push('=');
            out.push_str(&utf8_percent_encode(v, EncodeSet::FormUrlEncoded));
        }
        out
    }
}

impl std::fmt::Display for UrlSearchParams {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.serialize())
    }
}
