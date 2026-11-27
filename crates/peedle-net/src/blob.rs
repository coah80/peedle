use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Clone, Debug)]
pub struct BlobEntry {
    pub content_type: String,
    pub data: Vec<u8>,
}

#[derive(Default)]
pub struct BlobStore {
    entries: HashMap<String, BlobEntry>,
    next_id: AtomicU64,
}

impl BlobStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn create(&mut self, origin: &str, content_type: String, data: Vec<u8>) -> String {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed) + 1;
        let base = if origin.is_empty() || origin == "null" {
            "null"
        } else {
            origin
        };
        let url = format!("blob:{}/peedle-blob-{}", base, id);
        self.entries.insert(
            url.clone(),
            BlobEntry { content_type, data },
        );
        url
    }

    pub fn revoke(&mut self, url: &str) {
        self.entries.remove(url);
    }

    pub fn get(&self, url: &str) -> Option<&BlobEntry> {
        self.entries.get(url)
    }
}
