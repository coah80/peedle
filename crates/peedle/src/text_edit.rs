use peedle_render::{FontFaceKey, FontSystem};

#[derive(Clone, Debug, Default)]
pub struct TextEdit {
    pub text: String,
    pub cursor: usize,
    pub anchor: Option<usize>,
}

impl TextEdit {
    pub fn new(initial: &str) -> Self {
        let len = initial.chars().count();
        Self {
            text: initial.to_string(),
            cursor: len,
            anchor: None,
        }
    }

    pub fn char_len(&self) -> usize {
        self.text.chars().count()
    }

    pub fn set_text(&mut self, value: &str) {
        self.text = value.to_string();
        self.cursor = self.char_len();
        self.anchor = None;
    }

    pub fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
        self.anchor = None;
    }

    pub fn select_all(&mut self) {
        let len = self.char_len();
        if len > 0 {
            self.anchor = Some(0);
            self.cursor = len;
        } else {
            self.anchor = None;
            self.cursor = 0;
        }
    }

    pub fn selected_range(&self) -> Option<(usize, usize)> {
        let a = self.anchor?;
        let lo = a.min(self.cursor);
        let hi = a.max(self.cursor);
        if lo < hi {
            Some((lo, hi))
        } else {
            None
        }
    }

    pub fn selected_text(&self) -> String {
        if let Some((lo, hi)) = self.selected_range() {
            let b0 = char_to_byte(&self.text, lo);
            let b1 = char_to_byte(&self.text, hi);
            self.text[b0..b1].to_string()
        } else {
            self.text.clone()
        }
    }

    pub fn delete_selection(&mut self) -> bool {
        let Some((lo, hi)) = self.selected_range() else {
            self.anchor = None;
            return false;
        };
        let b0 = char_to_byte(&self.text, lo);
        let b1 = char_to_byte(&self.text, hi);
        self.text.replace_range(b0..b1, "");
        self.cursor = lo;
        self.anchor = None;
        true
    }

    pub fn insert_str(&mut self, s: &str) {
        let cleaned: String = s.chars().filter(|c| !c.is_control()).collect();
        if cleaned.is_empty() {
            return;
        }
        self.delete_selection();
        let b = char_to_byte(&self.text, self.cursor);
        self.text.insert_str(b, &cleaned);
        self.cursor += cleaned.chars().count();
        self.anchor = None;
    }

    pub fn backspace(&mut self, word: bool) {
        if self.delete_selection() {
            return;
        }
        if self.cursor == 0 {
            return;
        }
        let target = if word {
            prev_word_boundary(&self.text, self.cursor)
        } else {
            self.cursor - 1
        };
        let b0 = char_to_byte(&self.text, target);
        let b1 = char_to_byte(&self.text, self.cursor);
        self.text.replace_range(b0..b1, "");
        self.cursor = target;
        self.anchor = None;
    }

    pub fn delete(&mut self, word: bool) {
        if self.delete_selection() {
            return;
        }
        let len = self.char_len();
        if self.cursor >= len {
            return;
        }
        let target = if word {
            next_word_boundary(&self.text, self.cursor)
        } else {
            self.cursor + 1
        };
        let b0 = char_to_byte(&self.text, self.cursor);
        let b1 = char_to_byte(&self.text, target);
        self.text.replace_range(b0..b1, "");
        self.anchor = None;
    }

    pub fn move_left(&mut self, shift: bool, word: bool) {
        if !shift {
            if let Some((lo, _)) = self.selected_range() {
                self.cursor = lo;
                self.anchor = None;
                return;
            }
            self.anchor = None;
        } else if self.anchor.is_none() {
            self.anchor = Some(self.cursor);
        }
        if self.cursor > 0 {
            self.cursor = if word {
                prev_word_boundary(&self.text, self.cursor)
            } else {
                self.cursor - 1
            };
        }
    }

    pub fn move_right(&mut self, shift: bool, word: bool) {
        let len = self.char_len();
        if !shift {
            if let Some((_, hi)) = self.selected_range() {
                self.cursor = hi;
                self.anchor = None;
                return;
            }
            self.anchor = None;
        } else if self.anchor.is_none() {
            self.anchor = Some(self.cursor);
        }
        if self.cursor < len {
            self.cursor = if word {
                next_word_boundary(&self.text, self.cursor)
            } else {
                self.cursor + 1
            };
        }
    }

    pub fn move_home(&mut self, shift: bool) {
        if shift {
            if self.anchor.is_none() {
                self.anchor = Some(self.cursor);
            }
        } else {
            self.anchor = None;
        }
        self.cursor = 0;
    }

    pub fn move_end(&mut self, shift: bool) {
        if shift {
            if self.anchor.is_none() {
                self.anchor = Some(self.cursor);
            }
        } else {
            self.anchor = None;
        }
        self.cursor = self.char_len();
    }

    pub fn set_cursor_from_x(&mut self, rel_x: f32, fonts: &FontSystem, face: FontFaceKey, px: f32) {
        let mut best_idx = 0usize;
        let mut best_dist = rel_x.abs();
        let mut acc = 0.0f32;
        for (i, c) in self.text.chars().enumerate() {
            let mut buf = [0u8; 4];
            let s = c.encode_utf8(&mut buf);
            acc += fonts.measure_text(s, face, px);
            let d = (rel_x - acc).abs();
            if d < best_dist {
                best_dist = d;
                best_idx = i + 1;
            }
        }
        self.cursor = best_idx;
        self.anchor = None;
    }

    pub fn prefix_width(&self, char_idx: usize, fonts: &FontSystem, face: FontFaceKey, px: f32) -> f32 {
        let b = char_to_byte(&self.text, char_idx.min(self.char_len()));
        fonts.measure_text(&self.text[..b], face, px)
    }
}

fn char_to_byte(s: &str, char_idx: usize) -> usize {
    if char_idx == 0 {
        return 0;
    }
    s.char_indices()
        .nth(char_idx)
        .map(|(b, _)| b)
        .unwrap_or(s.len())
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn prev_word_boundary(s: &str, pos: usize) -> usize {
    let chars: Vec<char> = s.chars().collect();
    let mut i = pos.min(chars.len());
    while i > 0 && !is_word_char(chars[i - 1]) {
        i -= 1;
    }
    while i > 0 && is_word_char(chars[i - 1]) {
        i -= 1;
    }
    i
}

fn next_word_boundary(s: &str, pos: usize) -> usize {
    let chars: Vec<char> = s.chars().collect();
    let mut i = pos.min(chars.len());
    while i < chars.len() && is_word_char(chars[i]) {
        i += 1;
    }
    while i < chars.len() && !is_word_char(chars[i]) {
        i += 1;
    }
    i
}
