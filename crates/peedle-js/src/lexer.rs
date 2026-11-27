use crate::error::JsError;

#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    Ident(String, bool),
    PrivateIdent(String),
    Number(f64),
    BigInt(String),
    String(String, bool),
    RegExp(String, String),
    NoSubTemplate { raw: String, cooked: Option<String> },
    TemplateHead { raw: String, cooked: Option<String> },
    TemplateMiddle { raw: String, cooked: Option<String> },
    TemplateTail { raw: String, cooked: Option<String> },

    KwBreak,
    KwCase,
    KwCatch,
    KwClass,
    KwConst,
    KwContinue,
    KwDebugger,
    KwDefault,
    KwDelete,
    KwDo,
    KwElse,
    KwExport,
    KwExtends,
    KwFalse,
    KwFinally,
    KwFor,
    KwFunction,
    KwIf,
    KwImport,
    KwIn,
    KwInstanceof,
    KwNew,
    KwNull,
    KwReturn,
    KwSuper,
    KwSwitch,
    KwThis,
    KwThrow,
    KwTrue,
    KwTry,
    KwTypeof,
    KwVar,
    KwVoid,
    KwWhile,
    KwWith,
    KwYield,
    KwAwait,

    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    Dot,
    Ellipsis,
    Semi,
    Comma,
    Lt,
    Gt,
    LtEq,
    GtEq,
    EqEq,
    NotEq,
    EqEqEq,
    NotEqEq,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    StarStar,
    PlusPlus,
    MinusMinus,
    LtLt,
    GtGt,
    GtGtGt,
    Amp,
    Pipe,
    Caret,
    Bang,
    Tilde,
    AmpAmp,
    PipePipe,
    QuestionQuestion,
    Question,
    QuestionDot,
    Colon,
    Eq,
    PlusEq,
    MinusEq,
    StarEq,
    SlashEq,
    PercentEq,
    StarStarEq,
    LtLtEq,
    GtGtEq,
    GtGtGtEq,
    AmpEq,
    PipeEq,
    CaretEq,
    AmpAmpEq,
    PipePipeEq,
    QuestionQuestionEq,
    Arrow,

    Eof,
}

#[derive(Debug, Clone)]
pub struct Token {
    pub kind: TokenKind,
    pub start: usize,
    pub end: usize,
    pub line: usize,
    pub col: usize,
    pub preceded_by_newline: bool,
    pub strict_error: Option<&'static str>,
    pub has_lone_surrogate: bool,
}

#[derive(Clone)]
pub struct Lexer<'a> {
    src: &'a str,
    bytes: &'a [u8],
    pos: usize,
    line: usize,
    col: usize,
    at_line_start: bool,
    is_module: bool,
    pending_strict_error: Option<&'static str>,
    pending_lone_surrogate: bool,
}

impl<'a> Lexer<'a> {
    pub fn new(src: &'a str) -> Self {
        Self::new_with_mode(src, false)
    }

    pub fn new_with_mode(src: &'a str, is_module: bool) -> Self {
        let mut lex = Lexer {
            src,
            bytes: src.as_bytes(),
            pos: 0,
            line: 1,
            col: 1,
            at_line_start: true,
            is_module,
            pending_strict_error: None,
            pending_lone_surrogate: false,
        };
        if lex.src.starts_with("\u{FEFF}") {
            lex.pos = 3;
        }
        if lex.bytes.get(lex.pos..lex.pos + 2) == Some(b"#!") {
            lex.skip_line_comment();
        }
        lex
    }

    pub fn source(&self) -> &'a str {
        self.src
    }

    pub fn pos(&self) -> usize {
        self.pos
    }

    fn peek_byte(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn peek_byte_at(&self, offset: usize) -> Option<u8> {
        self.bytes.get(self.pos + offset).copied()
    }

    fn peek_char(&self) -> Option<char> {
        self.src[self.pos..].chars().next()
    }

    fn advance_byte(&mut self) -> Option<u8> {
        let b = *self.bytes.get(self.pos)?;
        self.pos += 1;
        if b == b'\n' {
            self.line += 1;
            self.col = 1;
            self.at_line_start = true;
        } else {
            self.col += 1;
        }
        Some(b)
    }

    fn advance_char(&mut self) -> Option<char> {
        let c = self.src[self.pos..].chars().next()?;
        self.pos += c.len_utf8();
        if c == '\n' || c == '\r' || c == '\u{2028}' || c == '\u{2029}' {
            if c == '\r' && self.peek_byte() == Some(b'\n') {
                self.pos += 1;
            }
            self.line += 1;
            self.col = 1;
            self.at_line_start = true;
        } else {
            self.col += 1;
        }
        Some(c)
    }

    fn skip_line_comment(&mut self) {
        while let Some(c) = self.peek_char() {
            if is_line_terminator(c) {
                break;
            }
            self.advance_char();
        }
    }

    fn skip_whitespace_and_comments(&mut self) -> Result<bool, JsError> {
        let mut saw_newline = false;
        loop {
            let Some(c) = self.peek_char() else {
                break;
            };
            if is_line_terminator(c) {
                saw_newline = true;
                self.advance_char();
                continue;
            }
            if is_horizontal_whitespace(c) {
                self.advance_char();
                continue;
            }
            if c == '/' {
                if self.peek_byte_at(1) == Some(b'/') {
                    self.pos += 2;
                    self.col += 2;
                    self.skip_line_comment();
                    continue;
                }
                if self.peek_byte_at(1) == Some(b'*') {
                    self.pos += 2;
                    self.col += 2;
                    let mut closed = false;
                    while let Some(ch) = self.peek_char() {
                        if ch == '*' && self.peek_byte_at(1) == Some(b'/') {
                            self.pos += 2;
                            self.col += 2;
                            closed = true;
                            break;
                        }
                        if is_line_terminator(ch) {
                            saw_newline = true;
                        }
                        self.advance_char();
                    }
                    if !closed {
                        return Err(JsError::syntax("Unterminated block comment"));
                    }
                    continue;
                }
            }
            if !self.is_module
                && c == '<'
                && self.bytes.get(self.pos..self.pos + 4) == Some(b"<!--")
            {
                self.pos += 4;
                self.col += 4;
                self.skip_line_comment();
                continue;
            }
            if !self.is_module
                && (saw_newline || self.at_line_start)
                && c == '-'
                && self.bytes.get(self.pos..self.pos + 3) == Some(b"-->")
            {
                self.pos += 3;
                self.col += 3;
                self.skip_line_comment();
                continue;
            }
            break;
        }
        Ok(saw_newline)
    }

    pub fn next_token(&mut self, allow_regexp: bool) -> Result<Token, JsError> {
        self.pending_strict_error = None;
        self.pending_lone_surrogate = false;
        let preceded_by_newline = self.skip_whitespace_and_comments()?;
        self.at_line_start = false;
        let start = self.pos;
        let line = self.line;
        let col = self.col;

        let Some(c) = self.peek_char() else {
            return Ok(Token {
                kind: TokenKind::Eof,
                start,
                end: start,
                line,
                col,
                preceded_by_newline,
                strict_error: None,
                has_lone_surrogate: false,
            });
        };

        let kind = match c {
            '(' => {
                self.advance_byte();
                TokenKind::LParen
            }
            ')' => {
                self.advance_byte();
                TokenKind::RParen
            }
            '{' => {
                self.advance_byte();
                TokenKind::LBrace
            }
            '}' => {
                self.advance_byte();
                TokenKind::RBrace
            }
            '[' => {
                self.advance_byte();
                TokenKind::LBracket
            }
            ']' => {
                self.advance_byte();
                TokenKind::RBracket
            }
            ';' => {
                self.advance_byte();
                TokenKind::Semi
            }
            ',' => {
                self.advance_byte();
                TokenKind::Comma
            }
            ':' => {
                self.advance_byte();
                TokenKind::Colon
            }
            '~' => {
                self.advance_byte();
                TokenKind::Tilde
            }
            '.' => {
                if self.peek_byte_at(1) == Some(b'.') && self.peek_byte_at(2) == Some(b'.') {
                    self.pos += 3;
                    self.col += 3;
                    TokenKind::Ellipsis
                } else if self
                    .peek_byte_at(1)
                    .is_some_and(|b| b.is_ascii_digit())
                {
                    self.lex_number()?
                } else {
                    self.advance_byte();
                    TokenKind::Dot
                }
            }
            '+' => {
                self.advance_byte();
                match self.peek_byte() {
                    Some(b'+') => {
                        self.advance_byte();
                        TokenKind::PlusPlus
                    }
                    Some(b'=') => {
                        self.advance_byte();
                        TokenKind::PlusEq
                    }
                    _ => TokenKind::Plus,
                }
            }
            '-' => {
                self.advance_byte();
                match self.peek_byte() {
                    Some(b'-') => {
                        self.advance_byte();
                        TokenKind::MinusMinus
                    }
                    Some(b'=') => {
                        self.advance_byte();
                        TokenKind::MinusEq
                    }
                    _ => TokenKind::Minus,
                }
            }
            '*' => {
                self.advance_byte();
                match self.peek_byte() {
                    Some(b'*') => {
                        self.advance_byte();
                        if self.peek_byte() == Some(b'=') {
                            self.advance_byte();
                            TokenKind::StarStarEq
                        } else {
                            TokenKind::StarStar
                        }
                    }
                    Some(b'=') => {
                        self.advance_byte();
                        TokenKind::StarEq
                    }
                    _ => TokenKind::Star,
                }
            }
            '/' => {
                if allow_regexp {
                    self.lex_regexp()?
                } else {
                    self.advance_byte();
                    if self.peek_byte() == Some(b'=') {
                        self.advance_byte();
                        TokenKind::SlashEq
                    } else {
                        TokenKind::Slash
                    }
                }
            }
            '%' => {
                self.advance_byte();
                if self.peek_byte() == Some(b'=') {
                    self.advance_byte();
                    TokenKind::PercentEq
                } else {
                    TokenKind::Percent
                }
            }
            '<' => {
                self.advance_byte();
                match self.peek_byte() {
                    Some(b'<') => {
                        self.advance_byte();
                        if self.peek_byte() == Some(b'=') {
                            self.advance_byte();
                            TokenKind::LtLtEq
                        } else {
                            TokenKind::LtLt
                        }
                    }
                    Some(b'=') => {
                        self.advance_byte();
                        TokenKind::LtEq
                    }
                    _ => TokenKind::Lt,
                }
            }
            '>' => {
                self.advance_byte();
                match self.peek_byte() {
                    Some(b'>') => {
                        self.advance_byte();
                        match self.peek_byte() {
                            Some(b'>') => {
                                self.advance_byte();
                                if self.peek_byte() == Some(b'=') {
                                    self.advance_byte();
                                    TokenKind::GtGtGtEq
                                } else {
                                    TokenKind::GtGtGt
                                }
                            }
                            Some(b'=') => {
                                self.advance_byte();
                                TokenKind::GtGtEq
                            }
                            _ => TokenKind::GtGt,
                        }
                    }
                    Some(b'=') => {
                        self.advance_byte();
                        TokenKind::GtEq
                    }
                    _ => TokenKind::Gt,
                }
            }
            '=' => {
                self.advance_byte();
                match self.peek_byte() {
                    Some(b'=') => {
                        self.advance_byte();
                        if self.peek_byte() == Some(b'=') {
                            self.advance_byte();
                            TokenKind::EqEqEq
                        } else {
                            TokenKind::EqEq
                        }
                    }
                    Some(b'>') => {
                        self.advance_byte();
                        TokenKind::Arrow
                    }
                    _ => TokenKind::Eq,
                }
            }
            '!' => {
                self.advance_byte();
                if self.peek_byte() == Some(b'=') {
                    self.advance_byte();
                    if self.peek_byte() == Some(b'=') {
                        self.advance_byte();
                        TokenKind::NotEqEq
                    } else {
                        TokenKind::NotEq
                    }
                } else {
                    TokenKind::Bang
                }
            }
            '&' => {
                self.advance_byte();
                match self.peek_byte() {
                    Some(b'&') => {
                        self.advance_byte();
                        if self.peek_byte() == Some(b'=') {
                            self.advance_byte();
                            TokenKind::AmpAmpEq
                        } else {
                            TokenKind::AmpAmp
                        }
                    }
                    Some(b'=') => {
                        self.advance_byte();
                        TokenKind::AmpEq
                    }
                    _ => TokenKind::Amp,
                }
            }
            '|' => {
                self.advance_byte();
                match self.peek_byte() {
                    Some(b'|') => {
                        self.advance_byte();
                        if self.peek_byte() == Some(b'=') {
                            self.advance_byte();
                            TokenKind::PipePipeEq
                        } else {
                            TokenKind::PipePipe
                        }
                    }
                    Some(b'=') => {
                        self.advance_byte();
                        TokenKind::PipeEq
                    }
                    _ => TokenKind::Pipe,
                }
            }
            '^' => {
                self.advance_byte();
                if self.peek_byte() == Some(b'=') {
                    self.advance_byte();
                    TokenKind::CaretEq
                } else {
                    TokenKind::Caret
                }
            }
            '?' => {
                self.advance_byte();
                match self.peek_byte() {
                    Some(b'?') => {
                        self.advance_byte();
                        if self.peek_byte() == Some(b'=') {
                            self.advance_byte();
                            TokenKind::QuestionQuestionEq
                        } else {
                            TokenKind::QuestionQuestion
                        }
                    }
                    Some(b'.') => {
                        if self
                            .peek_byte_at(1)
                            .is_some_and(|b| b.is_ascii_digit())
                        {
                            TokenKind::Question
                        } else {
                            self.advance_byte();
                            TokenKind::QuestionDot
                        }
                    }
                    _ => TokenKind::Question,
                }
            }
            '\'' | '"' => self.lex_string(c)?,
            '`' => self.lex_template(true)?,
            '#' => {
                self.advance_byte();
                let (name, _) = self.lex_ident_raw()?;
                if name.is_empty() {
                    return Err(JsError::syntax("Expected private identifier after '#'"));
                }
                TokenKind::PrivateIdent(name)
            }
            '0'..='9' => self.lex_number()?,
            c if is_ident_start(c) || c == '\\' => self.lex_ident_or_keyword()?,
            other => {
                return Err(JsError::syntax(format!(
                    "Unexpected character '{}' at {}:{}",
                    other, line, col
                )));
            }
        };

        Ok(Token {
            kind,
            start,
            end: self.pos,
            line,
            col,
            preceded_by_newline,
            strict_error: self.pending_strict_error.take(),
            has_lone_surrogate: self.pending_lone_surrogate,
        })
    }

    pub fn next_template_continuation(&mut self) -> Result<Token, JsError> {
        self.pending_strict_error = None;
        self.pending_lone_surrogate = false;
        let start = self.pos;
        let line = self.line;
        let col = self.col;
        let kind = self.lex_template(false)?;
        Ok(Token {
            kind,
            start,
            end: self.pos,
            line,
            col,
            preceded_by_newline: false,
            strict_error: None,
            has_lone_surrogate: self.pending_lone_surrogate,
        })
    }

    fn lex_template(&mut self, start_backtick: bool) -> Result<TokenKind, JsError> {
        if start_backtick {
            self.advance_byte();
        }
        let mut raw = String::new();
        let mut cooked = Some(String::new());
        while let Some(c) = self.peek_char() {
            match c {
                '`' => {
                    self.advance_byte();
                    return Ok(if start_backtick {
                        TokenKind::NoSubTemplate { raw, cooked }
                    } else {
                        TokenKind::TemplateTail { raw, cooked }
                    });
                }
                '$' if self.peek_byte_at(1) == Some(b'{') => {
                    self.pos += 2;
                    self.col += 2;
                    return Ok(if start_backtick {
                        TokenKind::TemplateHead { raw, cooked }
                    } else {
                        TokenKind::TemplateMiddle { raw, cooked }
                    });
                }
                '\\' => {
                    self.advance_byte();
                    raw.push('\\');
                    let Some(esc) = self.peek_char() else {
                        return Err(JsError::syntax("Unterminated template literal"));
                    };
                    if esc == '\r' {
                        self.advance_char();
                        raw.push('\n');
                        continue;
                    }
                    if is_line_terminator(esc) {
                        self.advance_char();
                        raw.push(esc);
                        continue;
                    }
                    let esc_start = self.pos;
                    match self.lex_escape_seq(true) {
                        Ok(Some(ch)) => {
                            raw.push_str(&self.src[esc_start..self.pos]);
                            if let Some(ref mut s) = cooked {
                                s.push(ch);
                            }
                        }
                        Ok(None) => {
                            raw.push_str(&self.src[esc_start..self.pos]);
                        }
                        Err(_) => {
                            if self.pos == esc_start {
                                if let Some(ch) = self.advance_char() {
                                    raw.push(ch);
                                }
                            } else {
                                raw.push_str(&self.src[esc_start..self.pos]);
                            }
                            cooked = None;
                        }
                    }
                }
                '\r' => {
                    self.advance_char();
                    raw.push('\n');
                    if let Some(ref mut s) = cooked {
                        s.push('\n');
                    }
                }
                _ => {
                    self.advance_char();
                    raw.push(c);
                    if let Some(ref mut s) = cooked {
                        s.push(c);
                    }
                }
            }
        }
        Err(JsError::syntax("Unterminated template literal"))
    }

    fn lex_string(&mut self, quote: char) -> Result<TokenKind, JsError> {
        self.advance_byte();
        let mut out = String::new();
        let mut has_escape = false;
        while let Some(c) = self.peek_char() {
            if c == quote {
                self.advance_byte();
                return Ok(TokenKind::String(out, has_escape));
            }
            if c == '\n' || c == '\r' {
                return Err(JsError::syntax("Unterminated string literal"));
            }
            if c == '\\' {
                has_escape = true;
                self.advance_byte();
                let Some(esc) = self.peek_char() else {
                    return Err(JsError::syntax("Unterminated string escape"));
                };
                if is_line_terminator(esc) {
                    self.advance_char();
                    continue;
                }
                if let Some(ch) = self.lex_escape_seq(false)? {
                    out.push(ch);
                }
            } else {
                self.advance_char();
                out.push(c);
            }
        }
        Err(JsError::syntax("Unterminated string literal"))
    }

    fn lex_escape_seq(&mut self, in_template: bool) -> Result<Option<char>, JsError> {
        let Some(c) = self.advance_char() else {
            return Err(JsError::syntax("Unexpected end of escape sequence"));
        };
        let ch = match c {
            'n' => '\n',
            'r' => '\r',
            't' => '\t',
            'b' => '\u{0008}',
            'f' => '\u{000C}',
            'v' => '\u{000B}',
            '\\' => '\\',
            '\'' => '\'',
            '"' => '"',
            '`' => '`',
            '$' => '$',
            '0' => {
                if self.peek_byte().is_some_and(|b| (b'0'..=b'7').contains(&b)) {
                    if in_template {
                        return Err(JsError::syntax("Octal escape in template"));
                    }
                    self.pending_strict_error = Some("Octal escape sequences are not allowed in strict mode");
                    let mut val = 0u32;
                    for _ in 0..2 {
                        if let Some(b) = self.peek_byte() {
                            if (b'0'..=b'7').contains(&b) {
                                self.advance_byte();
                                val = val * 8 + (b - b'0') as u32;
                            } else {
                                break;
                            }
                        }
                    }
                    char::from_u32(val).unwrap_or('\0')
                } else if self.peek_byte().is_some_and(|b| b == b'8' || b == b'9') {
                    if in_template {
                        return Err(JsError::syntax("Invalid escape in template"));
                    }
                    self.pending_strict_error = Some("Octal escape sequences are not allowed in strict mode");
                    '\0'
                } else {
                    '\0'
                }
            }
            '1'..='7' => {
                if in_template {
                    return Err(JsError::syntax("Octal escape in template"));
                }
                self.pending_strict_error = Some("Octal escape sequences are not allowed in strict mode");
                let mut val = (c as u32) - ('0' as u32);
                let max_extra = if c <= '3' { 2 } else { 1 };
                for _ in 0..max_extra {
                    if let Some(b) = self.peek_byte() {
                        if (b'0'..=b'7').contains(&b) {
                            self.advance_byte();
                            val = val * 8 + (b - b'0') as u32;
                        } else {
                            break;
                        }
                    }
                }
                char::from_u32(val).unwrap_or('\u{FFFD}')
            }
            '8' | '9' => {
                if in_template {
                    return Err(JsError::syntax("Non-octal decimal escape in template"));
                }
                self.pending_strict_error = Some("Non-octal decimal escape sequences are not allowed in strict mode");
                c
            }
            'x' => {
                let code = self.read_hex_digits(2)?;
                if (0xD800..=0xDFFF).contains(&code) {
                    self.pending_lone_surrogate = true;
                }
                char::from_u32(code).unwrap_or('\u{FFFD}')
            }
            'u' => {
                let code = self.read_unicode_escape()?;
                if (0xD800..=0xDBFF).contains(&code)
                    && self.bytes.get(self.pos..self.pos + 2) == Some(b"\\u")
                {
                    let saved_pos = self.pos;
                    let saved_line = self.line;
                    let saved_col = self.col;
                    self.pos += 2;
                    self.col += 2;
                    if let Ok(low) = self.read_unicode_escape() {
                        if (0xDC00..=0xDFFF).contains(&low) {
                            let combined =
                                0x10000 + ((code - 0xD800) << 10) + (low - 0xDC00);
                            return Ok(char::from_u32(combined));
                        }
                    }
                    self.pos = saved_pos;
                    self.line = saved_line;
                    self.col = saved_col;
                }
                if (0xD800..=0xDFFF).contains(&code) {
                    self.pending_lone_surrogate = true;
                }
                char::from_u32(code).unwrap_or('\u{FFFD}')
            }
            other => other,
        };
        Ok(Some(ch))
    }

    fn read_hex_digits(&mut self, count: usize) -> Result<u32, JsError> {
        let mut val = 0u32;
        for _ in 0..count {
            let Some(b) = self.peek_byte() else {
                return Err(JsError::syntax("Invalid hex escape"));
            };
            let d = hex_val(b).ok_or_else(|| JsError::syntax("Invalid hex escape"))?;
            self.advance_byte();
            val = (val << 4) | d;
        }
        Ok(val)
    }

    fn read_unicode_escape(&mut self) -> Result<u32, JsError> {
        if self.peek_byte() == Some(b'{') {
            self.advance_byte();
            let mut val = 0u32;
            let mut count = 0usize;
            while let Some(b) = self.peek_byte() {
                if b == b'}' {
                    self.advance_byte();
                    if count == 0 || val > 0x10FFFF {
                        return Err(JsError::syntax("Invalid Unicode code point escape"));
                    }
                    return Ok(val);
                }
                let d = hex_val(b)
                    .ok_or_else(|| JsError::syntax("Invalid Unicode escape digit"))?;
                self.advance_byte();
                val = val.saturating_mul(16).saturating_add(d);
                count += 1;
                if val > 0x10FFFF {
                    return Err(JsError::syntax("Unicode escape out of range"));
                }
            }
            Err(JsError::syntax("Unterminated Unicode escape"))
        } else {
            self.read_hex_digits(4)
        }
    }

    fn lex_regexp(&mut self) -> Result<TokenKind, JsError> {
        self.advance_byte();
        let mut pattern = String::new();
        let mut in_class = false;
        let mut closed = false;
        while let Some(c) = self.peek_char() {
            if is_line_terminator(c) {
                return Err(JsError::syntax("Unterminated regular expression literal"));
            }
            if c == '\\' {
                self.advance_char();
                pattern.push('\\');
                let Some(next) = self.peek_char() else {
                    return Err(JsError::syntax("Unterminated regular expression escape"));
                };
                if is_line_terminator(next) {
                    return Err(JsError::syntax("Unterminated regular expression literal"));
                }
                self.advance_char();
                pattern.push(next);
                continue;
            }
            if c == '[' {
                in_class = true;
            } else if c == ']' {
                in_class = false;
            } else if c == '/' && !in_class {
                self.advance_byte();
                closed = true;
                break;
            }
            self.advance_char();
            pattern.push(c);
        }
        if !closed {
            return Err(JsError::syntax("Unterminated regular expression literal"));
        }
        let mut flags = String::new();
        while let Some(c) = self.peek_char() {
            if is_ident_continue(c) {
                self.advance_char();
                flags.push(c);
            } else {
                break;
            }
        }
        Ok(TokenKind::RegExp(pattern, flags))
    }

    fn check_after_numeric(&self) -> Result<(), JsError> {
        if let Some(c) = self.peek_char() {
            if is_ident_start(c) || c.is_ascii_digit() {
                return Err(JsError::syntax(
                    "Identifier or digit directly after numeric literal",
                ));
            }
        }
        Ok(())
    }

    fn lex_number(&mut self) -> Result<TokenKind, JsError> {
        if self.peek_byte() == Some(b'0') {
            match self.peek_byte_at(1) {
                Some(b'x' | b'X') => {
                    self.pos += 2;
                    self.col += 2;
                    return self.lex_radix_number(16, |b| b.is_ascii_hexdigit());
                }
                Some(b'b' | b'B') => {
                    self.pos += 2;
                    self.col += 2;
                    return self.lex_radix_number(2, |b| b == b'0' || b == b'1');
                }
                Some(b'o' | b'O') => {
                    self.pos += 2;
                    self.col += 2;
                    return self.lex_radix_number(8, |b| (b'0'..=b'7').contains(&b));
                }
                Some(b'_') => {
                    return Err(JsError::syntax(
                        "Numeric separator cannot appear after leading 0",
                    ));
                }
                Some(b'0'..=b'9') => {
                    self.advance_byte();
                    let mut is_octal = true;
                    let mut digits = String::new();
                    while let Some(b) = self.peek_byte() {
                        if b.is_ascii_digit() {
                            if b >= b'8' {
                                is_octal = false;
                            }
                            self.advance_byte();
                            digits.push(b as char);
                        } else if b == b'_' {
                            return Err(JsError::syntax(
                                "Numeric separator not allowed in legacy octal or non-octal decimal literal",
                            ));
                        } else {
                            break;
                        }
                    }
                    if self.peek_byte() == Some(b'n') {
                        return Err(JsError::syntax(
                            "BigInt literal cannot have leading zero",
                        ));
                    }
                    self.pending_strict_error = Some(if is_octal {
                        "Legacy octal literals are not allowed in strict mode"
                    } else {
                        "Decimals with leading zeros are not allowed in strict mode"
                    });
                    if is_octal && !matches!(self.peek_byte(), Some(b'.' | b'e' | b'E')) {
                        let val = parse_radix_digits_f64(&digits, 8);
                        self.check_after_numeric()?;
                        return Ok(TokenKind::Number(val));
                    }
                    let mut buf = format!("0{}", digits);
                    if self.peek_byte() == Some(b'.') {
                        self.advance_byte();
                        buf.push('.');
                        if self.peek_byte().is_some_and(|b| b.is_ascii_digit()) {
                            self.read_decimal_digits(&mut buf)?;
                        }
                    }
                    if matches!(self.peek_byte(), Some(b'e' | b'E')) {
                        self.advance_byte();
                        buf.push('e');
                        if matches!(self.peek_byte(), Some(b'+' | b'-')) {
                            buf.push(self.advance_byte().unwrap() as char);
                        }
                        if !self.peek_byte().is_some_and(|b| b.is_ascii_digit()) {
                            return Err(JsError::syntax(
                                "Missing exponent digits in number literal",
                            ));
                        }
                        self.read_decimal_digits(&mut buf)?;
                    }
                    let val: f64 = buf
                        .parse()
                        .map_err(|_| JsError::syntax(format!("Invalid numeric literal: {}", buf)))?;
                    self.check_after_numeric()?;
                    return Ok(TokenKind::Number(val));
                }
                _ => {}
            }
        }

        let mut buf = String::new();
        let mut is_float = false;
        self.read_decimal_digits(&mut buf)?;

        if self.peek_byte() == Some(b'.') {
            is_float = true;
            self.advance_byte();
            buf.push('.');
            if self.peek_byte().is_some_and(|b| b.is_ascii_digit()) {
                self.read_decimal_digits(&mut buf)?;
            }
        }

        if matches!(self.peek_byte(), Some(b'e' | b'E')) {
            is_float = true;
            self.advance_byte();
            buf.push('e');
            if matches!(self.peek_byte(), Some(b'+' | b'-')) {
                buf.push(self.advance_byte().unwrap() as char);
            }
            if !self.peek_byte().is_some_and(|b| b.is_ascii_digit()) {
                return Err(JsError::syntax("Missing exponent digits in number literal"));
            }
            self.read_decimal_digits(&mut buf)?;
        }

        if self.peek_byte() == Some(b'n') {
            if is_float {
                return Err(JsError::syntax("BigInt literal cannot have decimal or exponent"));
            }
            self.advance_byte();
            self.check_after_numeric()?;
            return Ok(TokenKind::BigInt(buf));
        }

        let val: f64 = buf
            .parse()
            .map_err(|_| JsError::syntax(format!("Invalid numeric literal: {}", buf)))?;
        self.check_after_numeric()?;
        Ok(TokenKind::Number(val))
    }

    fn lex_radix_number<F>(&mut self, radix: u32, is_digit: F) -> Result<TokenKind, JsError>
    where
        F: Fn(u8) -> bool,
    {
        let mut digits = String::new();
        let mut prev_sep = false;
        while let Some(b) = self.peek_byte() {
            if is_digit(b) {
                prev_sep = false;
                self.advance_byte();
                digits.push(b as char);
            } else if b == b'_' {
                if digits.is_empty() || prev_sep {
                    return Err(JsError::syntax("Invalid numeric separator"));
                }
                prev_sep = true;
                self.advance_byte();
            } else {
                break;
            }
        }
        if digits.is_empty() || prev_sep {
            return Err(JsError::syntax("Invalid numeric literal"));
        }
        if self.peek_byte() == Some(b'n') {
            self.advance_byte();
            self.check_after_numeric()?;
            let prefix = match radix {
                16 => "0x",
                8 => "0o",
                2 => "0b",
                _ => "",
            };
            return Ok(TokenKind::BigInt(format!("{}{}", prefix, digits)));
        }
        self.check_after_numeric()?;
        Ok(TokenKind::Number(parse_radix_digits_f64(&digits, radix)))
    }

    fn read_decimal_digits(&mut self, buf: &mut String) -> Result<(), JsError> {
        let mut prev_sep = false;
        let mut count = 0usize;
        while let Some(b) = self.peek_byte() {
            if b.is_ascii_digit() {
                prev_sep = false;
                count += 1;
                self.advance_byte();
                buf.push(b as char);
            } else if b == b'_' {
                if count == 0 || prev_sep {
                    return Err(JsError::syntax("Invalid numeric separator"));
                }
                prev_sep = true;
                self.advance_byte();
            } else {
                break;
            }
        }
        if prev_sep {
            return Err(JsError::syntax("Trailing numeric separator"));
        }
        Ok(())
    }

    fn lex_ident_raw(&mut self) -> Result<(String, bool), JsError> {
        let mut name = String::new();
        let mut escaped = false;
        let mut first = true;
        while let Some(c) = self.peek_char() {
            if c == '\\' {
                if self.peek_byte_at(1) != Some(b'u') {
                    break;
                }
                escaped = true;
                self.pos += 2;
                self.col += 2;
                let code = self.read_unicode_escape()?;
                let ch = char::from_u32(code)
                    .ok_or_else(|| JsError::syntax("Invalid Unicode identifier escape"))?;
                if (first && !is_ident_start(ch)) || (!first && !is_ident_continue(ch)) {
                    return Err(JsError::syntax("Invalid identifier character in escape"));
                }
                name.push(ch);
                first = false;
            } else if (first && is_ident_start(c)) || (!first && is_ident_continue(c)) {
                self.advance_char();
                name.push(c);
                first = false;
            } else {
                break;
            }
        }
        Ok((name, escaped))
    }

    fn lex_ident_or_keyword(&mut self) -> Result<TokenKind, JsError> {
        let (name, escaped) = self.lex_ident_raw()?;
        if name.is_empty() {
            return Err(JsError::syntax("Expected identifier"));
        }
        if !escaped {
            if let Some(kw) = match_keyword(&name) {
                return Ok(kw);
            }
        }
        Ok(TokenKind::Ident(name, escaped))
    }
}

fn match_keyword(s: &str) -> Option<TokenKind> {
    Some(match s {
        "break" => TokenKind::KwBreak,
        "case" => TokenKind::KwCase,
        "catch" => TokenKind::KwCatch,
        "class" => TokenKind::KwClass,
        "const" => TokenKind::KwConst,
        "continue" => TokenKind::KwContinue,
        "debugger" => TokenKind::KwDebugger,
        "default" => TokenKind::KwDefault,
        "delete" => TokenKind::KwDelete,
        "do" => TokenKind::KwDo,
        "else" => TokenKind::KwElse,
        "export" => TokenKind::KwExport,
        "extends" => TokenKind::KwExtends,
        "false" => TokenKind::KwFalse,
        "finally" => TokenKind::KwFinally,
        "for" => TokenKind::KwFor,
        "function" => TokenKind::KwFunction,
        "if" => TokenKind::KwIf,
        "import" => TokenKind::KwImport,
        "in" => TokenKind::KwIn,
        "instanceof" => TokenKind::KwInstanceof,
        "new" => TokenKind::KwNew,
        "null" => TokenKind::KwNull,
        "return" => TokenKind::KwReturn,
        "super" => TokenKind::KwSuper,
        "switch" => TokenKind::KwSwitch,
        "this" => TokenKind::KwThis,
        "throw" => TokenKind::KwThrow,
        "true" => TokenKind::KwTrue,
        "try" => TokenKind::KwTry,
        "typeof" => TokenKind::KwTypeof,
        "var" => TokenKind::KwVar,
        "void" => TokenKind::KwVoid,
        "while" => TokenKind::KwWhile,
        "with" => TokenKind::KwWith,
        "yield" => TokenKind::KwYield,
        "await" => TokenKind::KwAwait,
        _ => return None,
    })
}

fn hex_val(b: u8) -> Option<u32> {
    match b {
        b'0'..=b'9' => Some((b - b'0') as u32),
        b'a'..=b'f' => Some((b - b'a' + 10) as u32),
        b'A'..=b'F' => Some((b - b'A' + 10) as u32),
        _ => None,
    }
}

fn parse_radix_digits_f64(digits: &str, radix: u32) -> f64 {
    let mut acc = 0.0_f64;
    let r = radix as f64;
    for b in digits.bytes() {
        let d = hex_val(b).unwrap_or(0);
        acc = acc * r + d as f64;
    }
    acc
}

fn is_line_terminator(c: char) -> bool {
    matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}')
}

fn is_horizontal_whitespace(c: char) -> bool {
    matches!(
        c,
        ' ' | '\t'
            | '\u{000B}'
            | '\u{000C}'
            | '\u{00A0}'
            | '\u{1680}'
            | '\u{2000}'..='\u{200A}'
            | '\u{202F}'
            | '\u{205F}'
            | '\u{3000}'
            | '\u{FEFF}'
    )
}

pub fn is_ident_start(c: char) -> bool {
    c == '_' || c == '$' || crate::unicode_id::is_unicode_id_start(c)
}

pub fn is_ident_continue(c: char) -> bool {
    c == '_'
        || c == '$'
        || c == '\u{200C}'
        || c == '\u{200D}'
        || crate::unicode_id::is_unicode_id_continue(c)
}
