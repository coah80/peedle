#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HashKind {
    Id,
    Unrestricted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NumericKind {
    Integer,
    Number,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Token {
    Ident(String),
    Function(String),
    AtKeyword(String),
    Hash {
        value: String,
        kind: HashKind,
    },
    String(String),
    BadString,
    Url(String),
    BadUrl,
    Delim(char),
    Number {
        value: f32,
        repr: String,
        kind: NumericKind,
    },
    Percentage {
        value: f32,
        repr: String,
    },
    Dimension {
        value: f32,
        repr: String,
        kind: NumericKind,
        unit: String,
    },
    UnicodeRange {
        start: u32,
        end: u32,
    },
    Whitespace,
    Cdo,
    Cdc,
    Colon,
    Semicolon,
    Comma,
    OpenSquare,
    CloseSquare,
    OpenParen,
    CloseParen,
    OpenCurly,
    CloseCurly,
    Eof,
}

pub struct Tokenizer {
    chars: Vec<char>,
    pos: usize,
}

impl Tokenizer {
    pub fn new(input: &str) -> Self {
        let mut chars = Vec::with_capacity(input.len());
        let mut iter = input.chars().peekable();
        while let Some(c) = iter.next() {
            match c {
                '\r' => {
                    if iter.peek() == Some(&'\n') {
                        iter.next();
                    }
                    chars.push('\n');
                }
                '\u{000C}' => chars.push('\n'),
                '\0' => chars.push('\u{FFFD}'),
                other => chars.push(other),
            }
        }
        Self { chars, pos: 0 }
    }

    fn peek(&self, offset: usize) -> Option<char> {
        self.chars.get(self.pos + offset).copied()
    }

    fn consume(&mut self) -> Option<char> {
        let ch = self.chars.get(self.pos).copied()?;
        self.pos += 1;
        Some(ch)
    }

    fn skip_comments(&mut self) {
        while self.peek(0) == Some('/') && self.peek(1) == Some('*') {
            self.pos += 2;
            while let Some(c) = self.consume() {
                if c == '*' && self.peek(0) == Some('/') {
                    self.pos += 1;
                    break;
                }
            }
        }
    }

    fn is_whitespace(c: char) -> bool {
        matches!(c, ' ' | '\t' | '\n')
    }

    fn is_ident_start(c: char) -> bool {
        c.is_ascii_alphabetic() || c == '_' || (c as u32) >= 0x80
    }

    fn is_ident_char(c: char) -> bool {
        Self::is_ident_start(c) || c.is_ascii_digit() || c == '-'
    }

    fn is_valid_escape_at(&self, offset: usize) -> bool {
        self.peek(offset) == Some('\\') && self.peek(offset + 1) != Some('\n')
    }

    fn would_start_ident_at(&self, offset: usize) -> bool {
        match self.peek(offset) {
            Some('-') => match self.peek(offset + 1) {
                Some(c) if Self::is_ident_start(c) || c == '-' => true,
                Some('\\') => self.is_valid_escape_at(offset + 1),
                _ => false,
            },
            Some(c) if Self::is_ident_start(c) => true,
            Some('\\') => self.is_valid_escape_at(offset),
            _ => false,
        }
    }

    fn would_start_number_at(&self, offset: usize) -> bool {
        match self.peek(offset) {
            Some('+') | Some('-') => match self.peek(offset + 1) {
                Some(c) if c.is_ascii_digit() => true,
                Some('.') => self.peek(offset + 2).map_or(false, |c| c.is_ascii_digit()),
                _ => false,
            },
            Some('.') => self.peek(offset + 1).map_or(false, |c| c.is_ascii_digit()),
            Some(c) if c.is_ascii_digit() => true,
            _ => false,
        }
    }

    fn consume_escaped_code_point(&mut self) -> char {
        let Some(first) = self.consume() else {
            return '\u{FFFD}';
        };
        if first.is_ascii_hexdigit() {
            let mut value: u32 = first.to_digit(16).unwrap_or(0);
            for _ in 0..5 {
                if let Some(d) = self.peek(0).and_then(|c| c.to_digit(16)) {
                    self.pos += 1;
                    value = (value << 4) | d;
                } else {
                    break;
                }
            }
            if let Some(ws) = self.peek(0) {
                if Self::is_whitespace(ws) {
                    self.pos += 1;
                }
            }
            if value == 0 || (0xD800..=0xDFFF).contains(&value) || value > 0x10FFFF {
                '\u{FFFD}'
            } else {
                char::from_u32(value).unwrap_or('\u{FFFD}')
            }
        } else {
            first
        }
    }

    fn consume_ident_sequence(&mut self) -> String {
        let mut out = String::new();
        while let Some(c) = self.peek(0) {
            if Self::is_ident_char(c) {
                self.pos += 1;
                out.push(c);
            } else if self.is_valid_escape_at(0) {
                self.pos += 1;
                out.push(self.consume_escaped_code_point());
            } else {
                break;
            }
        }
        out
    }

    fn consume_string_token(&mut self, quote: char) -> Token {
        let mut out = String::new();
        while let Some(c) = self.consume() {
            if c == quote {
                return Token::String(out);
            } else if c == '\n' {
                self.pos -= 1;
                return Token::BadString;
            } else if c == '\\' {
                match self.peek(0) {
                    None => return Token::String(out),
                    Some('\n') => {
                        self.pos += 1;
                    }
                    Some(_) => {
                        out.push(self.consume_escaped_code_point());
                    }
                }
            } else {
                out.push(c);
            }
        }
        Token::String(out)
    }

    fn consume_number(&mut self) -> (f32, String, NumericKind) {
        let mut repr = String::new();
        let mut kind = NumericKind::Integer;
        if let Some(sign @ ('+' | '-')) = self.peek(0) {
            self.pos += 1;
            repr.push(sign);
        }
        while let Some(d) = self.peek(0) {
            if d.is_ascii_digit() {
                self.pos += 1;
                repr.push(d);
            } else {
                break;
            }
        }
        if self.peek(0) == Some('.') && self.peek(1).map_or(false, |c| c.is_ascii_digit()) {
            self.pos += 1;
            repr.push('.');
            kind = NumericKind::Number;
            while let Some(d) = self.peek(0) {
                if d.is_ascii_digit() {
                    self.pos += 1;
                    repr.push(d);
                } else {
                    break;
                }
            }
        }
        if matches!(self.peek(0), Some('e' | 'E')) {
            let has_exp = match self.peek(1) {
                Some(c) if c.is_ascii_digit() => true,
                Some('+' | '-') => self.peek(2).map_or(false, |c| c.is_ascii_digit()),
                _ => false,
            };
            if has_exp {
                kind = NumericKind::Number;
                repr.push(self.consume().unwrap());
                if matches!(self.peek(0), Some('+' | '-')) {
                    repr.push(self.consume().unwrap());
                }
                while let Some(d) = self.peek(0) {
                    if d.is_ascii_digit() {
                        self.pos += 1;
                        repr.push(d);
                    } else {
                        break;
                    }
                }
            }
        }
        let value = repr.parse::<f32>().unwrap_or(0.0);
        (value, repr, kind)
    }

    fn consume_numeric_token(&mut self) -> Token {
        let (value, repr, kind) = self.consume_number();
        if self.would_start_ident_at(0) {
            let unit = self.consume_ident_sequence();
            Token::Dimension {
                value,
                repr,
                kind,
                unit,
            }
        } else if self.peek(0) == Some('%') {
            self.pos += 1;
            Token::Percentage { value, repr }
        } else {
            Token::Number { value, repr, kind }
        }
    }

    fn consume_bad_url_remnants(&mut self) {
        while let Some(c) = self.consume() {
            if c == ')' {
                break;
            } else if c == '\\' && self.peek(0) != Some('\n') && self.peek(0).is_some() {
                let _ = self.consume_escaped_code_point();
            }
        }
    }

    fn consume_url_token(&mut self) -> Token {
        while self.peek(0).map_or(false, Self::is_whitespace) {
            self.pos += 1;
        }
        let mut out = String::new();
        while let Some(c) = self.consume() {
            match c {
                ')' => return Token::Url(out),
                ch if Self::is_whitespace(ch) => {
                    while self.peek(0).map_or(false, Self::is_whitespace) {
                        self.pos += 1;
                    }
                    if self.peek(0) == Some(')') {
                        self.pos += 1;
                        return Token::Url(out);
                    } else if self.peek(0).is_none() {
                        return Token::Url(out);
                    } else {
                        self.consume_bad_url_remnants();
                        return Token::BadUrl;
                    }
                }
                '"' | '\'' | '(' => {
                    self.consume_bad_url_remnants();
                    return Token::BadUrl;
                }
                ch if is_non_printable(ch) => {
                    self.consume_bad_url_remnants();
                    return Token::BadUrl;
                }
                '\\' => {
                    if self.peek(0) == Some('\n') || self.peek(0).is_none() {
                        self.consume_bad_url_remnants();
                        return Token::BadUrl;
                    }
                    out.push(self.consume_escaped_code_point());
                }
                other => out.push(other),
            }
        }
        Token::Url(out)
    }

    fn consume_unicode_range(&mut self) -> Token {
        let mut start_hex = String::new();
        let mut wildcards = 0usize;
        while start_hex.len() + wildcards < 6 {
            match self.peek(0) {
                Some(c) if c.is_ascii_hexdigit() && wildcards == 0 => {
                    self.pos += 1;
                    start_hex.push(c);
                }
                Some('?') => {
                    self.pos += 1;
                    wildcards += 1;
                }
                _ => break,
            }
        }
        if wildcards > 0 {
            let mut s_str = start_hex.clone();
            let mut e_str = start_hex;
            for _ in 0..wildcards {
                s_str.push('0');
                e_str.push('F');
            }
            let start = u32::from_str_radix(&s_str, 16).unwrap_or(0);
            let end = u32::from_str_radix(&e_str, 16).unwrap_or(start);
            return Token::UnicodeRange { start, end };
        }
        let start = u32::from_str_radix(&start_hex, 16).unwrap_or(0);
        if self.peek(0) == Some('-') && self.peek(1).map_or(false, |c| c.is_ascii_hexdigit()) {
            self.pos += 1;
            let mut end_hex = String::new();
            while end_hex.len() < 6 {
                if let Some(c) = self.peek(0).filter(|c| c.is_ascii_hexdigit()) {
                    self.pos += 1;
                    end_hex.push(c);
                } else {
                    break;
                }
            }
            let end = u32::from_str_radix(&end_hex, 16).unwrap_or(start);
            return Token::UnicodeRange { start, end };
        }
        Token::UnicodeRange { start, end: start }
    }

    fn consume_ident_like_token(&mut self) -> Token {
        let name = self.consume_ident_sequence();
        if name.eq_ignore_ascii_case("url") && self.peek(0) == Some('(') {
            self.pos += 1;
            let mut look = 0;
            while self.peek(look).map_or(false, Self::is_whitespace) {
                look += 1;
            }
            if matches!(self.peek(look), Some('"' | '\'')) {
                return Token::Function(name);
            }
            return self.consume_url_token();
        }
        if self.peek(0) == Some('(') {
            self.pos += 1;
            return Token::Function(name);
        }
        Token::Ident(name)
    }

    pub fn next_token(&mut self) -> Token {
        self.skip_comments();
        let Some(c) = self.peek(0) else {
            return Token::Eof;
        };

        if Self::is_whitespace(c) {
            while self.peek(0).map_or(false, Self::is_whitespace) {
                self.pos += 1;
            }
            return Token::Whitespace;
        }

        if matches!(c, 'u' | 'U')
            && self.peek(1) == Some('+')
            && self
                .peek(2)
                .map_or(false, |ch| ch.is_ascii_hexdigit() || ch == '?')
        {
            self.pos += 2;
            return self.consume_unicode_range();
        }

        match c {
            '"' | '\'' => {
                self.pos += 1;
                self.consume_string_token(c)
            }
            '#' => {
                self.pos += 1;
                let starts_ident_char = self.peek(0).map_or(false, Self::is_ident_char)
                    || self.is_valid_escape_at(0);
                if starts_ident_char {
                    let kind = if self.would_start_ident_at(0) {
                        HashKind::Id
                    } else {
                        HashKind::Unrestricted
                    };
                    let value = self.consume_ident_sequence();
                    Token::Hash { value, kind }
                } else {
                    Token::Delim('#')
                }
            }
            '(' => {
                self.pos += 1;
                Token::OpenParen
            }
            ')' => {
                self.pos += 1;
                Token::CloseParen
            }
            '[' => {
                self.pos += 1;
                Token::OpenSquare
            }
            ']' => {
                self.pos += 1;
                Token::CloseSquare
            }
            '{' => {
                self.pos += 1;
                Token::OpenCurly
            }
            '}' => {
                self.pos += 1;
                Token::CloseCurly
            }
            ',' => {
                self.pos += 1;
                Token::Comma
            }
            ':' => {
                self.pos += 1;
                Token::Colon
            }
            ';' => {
                self.pos += 1;
                Token::Semicolon
            }
            '+' => {
                if self.would_start_number_at(0) {
                    self.consume_numeric_token()
                } else {
                    self.pos += 1;
                    Token::Delim('+')
                }
            }
            '-' => {
                if self.would_start_number_at(0) {
                    self.consume_numeric_token()
                } else if self.peek(1) == Some('-') && self.peek(2) == Some('>') {
                    self.pos += 3;
                    Token::Cdc
                } else if self.would_start_ident_at(0) {
                    self.consume_ident_like_token()
                } else {
                    self.pos += 1;
                    Token::Delim('-')
                }
            }
            '.' => {
                if self.would_start_number_at(0) {
                    self.consume_numeric_token()
                } else {
                    self.pos += 1;
                    Token::Delim('.')
                }
            }
            '<' => {
                if self.peek(1) == Some('!')
                    && self.peek(2) == Some('-')
                    && self.peek(3) == Some('-')
                {
                    self.pos += 4;
                    Token::Cdo
                } else {
                    self.pos += 1;
                    Token::Delim('<')
                }
            }
            '@' => {
                self.pos += 1;
                if self.would_start_ident_at(0) {
                    let name = self.consume_ident_sequence();
                    Token::AtKeyword(name)
                } else {
                    Token::Delim('@')
                }
            }
            '\\' => {
                if self.is_valid_escape_at(0) {
                    self.consume_ident_like_token()
                } else {
                    self.pos += 1;
                    Token::Delim('\\')
                }
            }
            ch if ch.is_ascii_digit() => self.consume_numeric_token(),
            ch if Self::is_ident_start(ch) => self.consume_ident_like_token(),
            other => {
                self.pos += 1;
                Token::Delim(other)
            }
        }
    }
}

fn is_non_printable(c: char) -> bool {
    let code = c as u32;
    code <= 0x08 || code == 0x0B || (0x0E..=0x1F).contains(&code) || code == 0x7F
}

pub fn tokenize(input: &str) -> Vec<Token> {
    let mut tok = Tokenizer::new(input);
    let mut out = Vec::new();
    loop {
        let t = tok.next_token();
        if t == Token::Eof {
            break;
        }
        out.push(t);
    }
    out
}

pub fn serialize_tokens(tokens: &[Token]) -> String {
    let mut out = String::new();
    for t in tokens {
        match t {
            Token::Ident(s) => out.push_str(s),
            Token::Function(s) => {
                out.push_str(s);
                out.push('(');
            }
            Token::AtKeyword(s) => {
                out.push('@');
                out.push_str(s);
            }
            Token::Hash { value, .. } => {
                out.push('#');
                out.push_str(value);
            }
            Token::String(s) => {
                out.push('"');
                for ch in s.chars() {
                    if ch == '"' || ch == '\\' {
                        out.push('\\');
                    }
                    out.push(ch);
                }
                out.push('"');
            }
            Token::BadString => {}
            Token::Url(u) => {
                out.push_str("url(");
                out.push_str(u);
                out.push(')');
            }
            Token::BadUrl => {}
            Token::Delim(c) => out.push(*c),
            Token::Number { repr, .. } => out.push_str(repr),
            Token::Percentage { repr, .. } => {
                out.push_str(repr);
                out.push('%');
            }
            Token::Dimension { repr, unit, .. } => {
                out.push_str(repr);
                out.push_str(unit);
            }
            Token::UnicodeRange { start, end } => {
                if start == end {
                    out.push_str(&format!("U+{:X}", start));
                } else {
                    out.push_str(&format!("U+{:X}-{:X}", start, end));
                }
            }
            Token::Whitespace => out.push(' '),
            Token::Cdo => out.push_str("<!--"),
            Token::Cdc => out.push_str("-->"),
            Token::Colon => out.push(':'),
            Token::Semicolon => out.push(';'),
            Token::Comma => out.push(','),
            Token::OpenSquare => out.push('['),
            Token::CloseSquare => out.push(']'),
            Token::OpenParen => out.push('('),
            Token::CloseParen => out.push(')'),
            Token::OpenCurly => out.push('{'),
            Token::CloseCurly => out.push('}'),
            Token::Eof => {}
        }
    }
    out
}
