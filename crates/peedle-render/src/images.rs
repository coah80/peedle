use std::collections::HashMap;

#[derive(Clone, Debug)]
pub struct DecodedImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl DecodedImage {
    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        let img = image::load_from_memory(bytes).map_err(|e| e.to_string())?;
        let rgba = img.to_rgba8();
        Ok(Self {
            width: rgba.width(),
            height: rgba.height(),
            rgba: rgba.into_raw(),
        })
    }
}

#[derive(Clone, Default)]
pub struct ImageCache {
    entries: HashMap<String, DecodedImage>,
}

impl ImageCache {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    pub fn insert(&mut self, url: String, img: DecodedImage) {
        self.entries.insert(url, img);
    }

    pub fn get(&self, url: &str) -> Option<&DecodedImage> {
        self.entries.get(url)
    }

    pub fn contains(&self, url: &str) -> bool {
        self.entries.contains_key(url)
    }
}
