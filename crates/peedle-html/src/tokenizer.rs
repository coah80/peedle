use crate::dom::Attribute;
use crate::entities::lookup_entity_prefix;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DoctypeToken {
    pub name: Option<String>,
    pub public_id: Option<String>,
    pub system_id: Option<String>,
    pub force_quirks: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TagToken {
    pub is_end: bool,
    pub name: String,
    pub self_closing: bool,
    pub attributes: Vec<Attribute>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Token {
    Doctype(DoctypeToken),
    StartTag(TagToken),
    EndTag(TagToken),
    Comment(String),
    Character(char),
    Eof,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenizerState {
    Data,
    Rcdata,
    Rawtext,
    ScriptData,
    Plaintext,
    TagOpen,
    EndTagOpen,
    TagName,
    RcdataLessThanSign,
    RcdataEndTagOpen,
    RcdataEndTagName,
    RawtextLessThanSign,
    RawtextEndTagOpen,
    RawtextEndTagName,
    ScriptDataLessThanSign,
    ScriptDataEndTagOpen,
    ScriptDataEndTagName,
    ScriptDataEscapeStart,
    ScriptDataEscapeStartDash,
    ScriptDataEscaped,
    ScriptDataEscapedDash,
    ScriptDataEscapedDashDash,
    ScriptDataEscapedLessThanSign,
    ScriptDataEscapedEndTagOpen,
    ScriptDataEscapedEndTagName,
    ScriptDataDoubleEscapeStart,
    ScriptDataDoubleEscaped,
    ScriptDataDoubleEscapedDash,
    ScriptDataDoubleEscapedDashDash,
    ScriptDataDoubleEscapedLessThanSign,
    ScriptDataDoubleEscapeEnd,
    BeforeAttributeName,
    AttributeName,
    AfterAttributeName,
    BeforeAttributeValue,
    AttributeValueDoubleQuoted,
    AttributeValueSingleQuoted,
    AttributeValueUnquoted,
    AfterAttributeValueQuoted,
    SelfClosingStartTag,
    BogusComment,
    MarkupDeclarationOpen,
    CommentStart,
    CommentStartDash,
    Comment,
    CommentLessThanSign,
    CommentLessThanSignBang,
    CommentLessThanSignBangDash,
    CommentLessThanSignBangDashDash,
    CommentEndDash,
    CommentEnd,
    CommentEndBang,
    Doctype,
    BeforeDoctypeName,
    DoctypeName,
    AfterDoctypeName,
    AfterDoctypePublicKeyword,
    BeforeDoctypePublicIdentifier,
    DoctypePublicIdentifierDoubleQuoted,
    DoctypePublicIdentifierSingleQuoted,
    AfterDoctypePublicIdentifier,
    BetweenDoctypePublicAndSystemIdentifiers,
    AfterDoctypeSystemKeyword,
    BeforeDoctypeSystemIdentifier,
    DoctypeSystemIdentifierDoubleQuoted,
    DoctypeSystemIdentifierSingleQuoted,
    AfterDoctypeSystemIdentifier,
    BogusDoctype,
    CdataSection,
    CdataSectionBracket,
    CdataSectionEnd,
    CharacterReference,
    NamedCharacterReference,
    NumericCharacterReference,
    HexadecimalCharacterReferenceStart,
    DecimalCharacterReferenceStart,
    HexadecimalCharacterReference,
    DecimalCharacterReference,
    NumericCharacterReferenceEnd,
}

pub struct Tokenizer {
    chars: Vec<char>,
    pos: usize,
    pub state: TokenizerState,
    return_state: TokenizerState,
    temp_buffer: String,
    current_tag: Option<TagToken>,
    current_attr_name: String,
    current_attr_value: String,
    has_current_attr: bool,
    current_comment: String,
    current_doctype: Option<DoctypeToken>,
    last_start_tag_name: String,
    char_ref_code: u32,
    pub allow_cdata: bool,
    pending_tokens: Vec<Token>,
}

impl Tokenizer {
    pub fn new(input: &str) -> Self {
        let normalized = preprocess_input(input);
        Self {
            chars: normalized.chars().collect(),
            pos: 0,
            state: TokenizerState::Data,
            return_state: TokenizerState::Data,
            temp_buffer: String::new(),
            current_tag: None,
            current_attr_name: String::new(),
            current_attr_value: String::new(),
            has_current_attr: false,
            current_comment: String::new(),
            current_doctype: None,
            last_start_tag_name: String::new(),
            char_ref_code: 0,
            allow_cdata: false,
            pending_tokens: Vec::new(),
        }
    }

    pub fn set_last_start_tag_name(&mut self, name: &str) {
        self.last_start_tag_name = name.to_string();
    }

    pub fn next_token(&mut self) -> Token {
        while self.pending_tokens.is_empty() {
            self.step();
        }
        self.pending_tokens.remove(0)
    }

    fn emit(&mut self, token: Token) {
        self.pending_tokens.push(token);
    }

    fn emit_str(&mut self, s: &str) {
        for c in s.chars() {
            self.emit(Token::Character(c));
        }
    }

    fn consume(&mut self) -> Option<char> {
        if self.pos < self.chars.len() {
            let c = self.chars[self.pos];
            self.pos += 1;
            Some(c)
        } else {
            None
        }
    }

    fn reconsume(&mut self) {
        if self.pos > 0 {
            self.pos -= 1;
        }
    }

    fn matches_ascii_ci(&self, s: &str) -> bool {
        let mut idx = self.pos;
        for expected in s.chars() {
            match self.chars.get(idx) {
                Some(&c) if c.to_ascii_lowercase() == expected.to_ascii_lowercase() => idx += 1,
                _ => return false,
            }
        }
        true
    }

    fn matches_exact(&self, s: &str) -> bool {
        let mut idx = self.pos;
        for expected in s.chars() {
            match self.chars.get(idx) {
                Some(&c) if c == expected => idx += 1,
                _ => return false,
            }
        }
        true
    }

    fn advance_by(&mut self, n: usize) {
        self.pos = (self.pos + n).min(self.chars.len());
    }

    fn finish_attr(&mut self) {
        if !self.has_current_attr {
            return;
        }
        self.has_current_attr = false;
        let name = std::mem::take(&mut self.current_attr_name);
        let value = std::mem::take(&mut self.current_attr_value);
        if let Some(ref mut tag) = self.current_tag {
            if !tag.attributes.iter().any(|a| a.name == name) {
                tag.attributes.push(Attribute::new(name, value));
            }
        }
    }

    fn emit_current_tag(&mut self) {
        self.finish_attr();
        if let Some(tag) = self.current_tag.take() {
            if !tag.is_end {
                self.last_start_tag_name = tag.name.clone();
                self.emit(Token::StartTag(tag));
            } else {
                let mut end_tag = tag;
                end_tag.attributes.clear();
                end_tag.self_closing = false;
                self.emit(Token::EndTag(end_tag));
            }
        }
    }

    fn emit_current_doctype(&mut self) {
        if let Some(dt) = self.current_doctype.take() {
            self.emit(Token::Doctype(dt));
        }
    }

    fn is_appropriate_end_tag(&self) -> bool {
        if self.last_start_tag_name.is_empty() {
            return false;
        }
        self.current_tag
            .as_ref()
            .map_or(false, |t| t.name == self.last_start_tag_name)
    }

    fn consumed_as_part_of_attribute(&self) -> bool {
        matches!(
            self.return_state,
            TokenizerState::AttributeValueDoubleQuoted
                | TokenizerState::AttributeValueSingleQuoted
                | TokenizerState::AttributeValueUnquoted
        )
    }

    fn flush_code_points_as_char_ref(&mut self) {
        let buf = std::mem::take(&mut self.temp_buffer);
        if self.consumed_as_part_of_attribute() {
            self.current_attr_value.push_str(&buf);
        } else {
            self.emit_str(&buf);
        }
    }

    fn step(&mut self) {
        match self.state {
            TokenizerState::Data => match self.consume() {
                Some('&') => {
                    self.return_state = TokenizerState::Data;
                    self.state = TokenizerState::CharacterReference;
                }
                Some('<') => self.state = TokenizerState::TagOpen,
                Some('\0') => self.emit(Token::Character('\0')),
                Some(c) => self.emit(Token::Character(c)),
                None => self.emit(Token::Eof),
            },
            TokenizerState::Rcdata => match self.consume() {
                Some('&') => {
                    self.return_state = TokenizerState::Rcdata;
                    self.state = TokenizerState::CharacterReference;
                }
                Some('<') => self.state = TokenizerState::RcdataLessThanSign,
                Some('\0') => self.emit(Token::Character('\u{FFFD}')),
                Some(c) => self.emit(Token::Character(c)),
                None => self.emit(Token::Eof),
            },
            TokenizerState::Rawtext => match self.consume() {
                Some('<') => self.state = TokenizerState::RawtextLessThanSign,
                Some('\0') => self.emit(Token::Character('\u{FFFD}')),
                Some(c) => self.emit(Token::Character(c)),
                None => self.emit(Token::Eof),
            },
            TokenizerState::ScriptData => match self.consume() {
                Some('<') => self.state = TokenizerState::ScriptDataLessThanSign,
                Some('\0') => self.emit(Token::Character('\u{FFFD}')),
                Some(c) => self.emit(Token::Character(c)),
                None => self.emit(Token::Eof),
            },
            TokenizerState::Plaintext => match self.consume() {
                Some('\0') => self.emit(Token::Character('\u{FFFD}')),
                Some(c) => self.emit(Token::Character(c)),
                None => self.emit(Token::Eof),
            },
            TokenizerState::TagOpen => match self.consume() {
                Some('!') => self.state = TokenizerState::MarkupDeclarationOpen,
                Some('/') => self.state = TokenizerState::EndTagOpen,
                Some(c) if c.is_ascii_alphabetic() => {
                    self.current_tag = Some(TagToken {
                        is_end: false,
                        name: String::new(),
                        self_closing: false,
                        attributes: Vec::new(),
                    });
                    self.reconsume();
                    self.state = TokenizerState::TagName;
                }
                Some('?') => {
                    self.current_comment.clear();
                    self.reconsume();
                    self.state = TokenizerState::BogusComment;
                }
                Some(_) => {
                    self.emit(Token::Character('<'));
                    self.reconsume();
                    self.state = TokenizerState::Data;
                }
                None => {
                    self.emit(Token::Character('<'));
                    self.emit(Token::Eof);
                }
            },
            TokenizerState::EndTagOpen => match self.consume() {
                Some(c) if c.is_ascii_alphabetic() => {
                    self.current_tag = Some(TagToken {
                        is_end: true,
                        name: String::new(),
                        self_closing: false,
                        attributes: Vec::new(),
                    });
                    self.reconsume();
                    self.state = TokenizerState::TagName;
                }
                Some('>') => self.state = TokenizerState::Data,
                Some(_) => {
                    self.current_comment.clear();
                    self.reconsume();
                    self.state = TokenizerState::BogusComment;
                }
                None => {
                    self.emit(Token::Character('<'));
                    self.emit(Token::Character('/'));
                    self.emit(Token::Eof);
                }
            },
            TokenizerState::TagName => match self.consume() {
                Some('\t' | '\n' | '\x0C' | ' ') => self.state = TokenizerState::BeforeAttributeName,
                Some('/') => self.state = TokenizerState::SelfClosingStartTag,
                Some('>') => {
                    self.state = TokenizerState::Data;
                    self.emit_current_tag();
                }
                Some(c) if c.is_ascii_uppercase() => {
                    if let Some(ref mut t) = self.current_tag {
                        t.name.push(c.to_ascii_lowercase());
                    }
                }
                Some('\0') => {
                    if let Some(ref mut t) = self.current_tag {
                        t.name.push('\u{FFFD}');
                    }
                }
                Some(c) => {
                    if let Some(ref mut t) = self.current_tag {
                        t.name.push(c);
                    }
                }
                None => self.emit(Token::Eof),
            },
            TokenizerState::RcdataLessThanSign => match self.consume() {
                Some('/') => {
                    self.temp_buffer.clear();
                    self.state = TokenizerState::RcdataEndTagOpen;
                }
                Some(_) => {
                    self.emit(Token::Character('<'));
                    self.reconsume();
                    self.state = TokenizerState::Rcdata;
                }
                None => {
                    self.emit(Token::Character('<'));
                    self.state = TokenizerState::Rcdata;
                }
            },
            TokenizerState::RcdataEndTagOpen => match self.consume() {
                Some(c) if c.is_ascii_alphabetic() => {
                    self.current_tag = Some(TagToken {
                        is_end: true,
                        name: String::new(),
                        self_closing: false,
                        attributes: Vec::new(),
                    });
                    self.reconsume();
                    self.state = TokenizerState::RcdataEndTagName;
                }
                Some(_) => {
                    self.emit_str("</");
                    self.reconsume();
                    self.state = TokenizerState::Rcdata;
                }
                None => {
                    self.emit_str("</");
                    self.state = TokenizerState::Rcdata;
                }
            },
            TokenizerState::RcdataEndTagName => self.step_end_tag_name(TokenizerState::Rcdata),
            TokenizerState::RawtextLessThanSign => match self.consume() {
                Some('/') => {
                    self.temp_buffer.clear();
                    self.state = TokenizerState::RawtextEndTagOpen;
                }
                Some(_) => {
                    self.emit(Token::Character('<'));
                    self.reconsume();
                    self.state = TokenizerState::Rawtext;
                }
                None => {
                    self.emit(Token::Character('<'));
                    self.state = TokenizerState::Rawtext;
                }
            },
            TokenizerState::RawtextEndTagOpen => match self.consume() {
                Some(c) if c.is_ascii_alphabetic() => {
                    self.current_tag = Some(TagToken {
                        is_end: true,
                        name: String::new(),
                        self_closing: false,
                        attributes: Vec::new(),
                    });
                    self.reconsume();
                    self.state = TokenizerState::RawtextEndTagName;
                }
                Some(_) => {
                    self.emit_str("</");
                    self.reconsume();
                    self.state = TokenizerState::Rawtext;
                }
                None => {
                    self.emit_str("</");
                    self.state = TokenizerState::Rawtext;
                }
            },
            TokenizerState::RawtextEndTagName => self.step_end_tag_name(TokenizerState::Rawtext),
            TokenizerState::ScriptDataLessThanSign => match self.consume() {
                Some('/') => {
                    self.temp_buffer.clear();
                    self.state = TokenizerState::ScriptDataEndTagOpen;
                }
                Some('!') => {
                    self.emit_str("<!");
                    self.state = TokenizerState::ScriptDataEscapeStart;
                }
                Some(_) => {
                    self.emit(Token::Character('<'));
                    self.reconsume();
                    self.state = TokenizerState::ScriptData;
                }
                None => {
                    self.emit(Token::Character('<'));
                    self.state = TokenizerState::ScriptData;
                }
            },
            TokenizerState::ScriptDataEndTagOpen => match self.consume() {
                Some(c) if c.is_ascii_alphabetic() => {
                    self.current_tag = Some(TagToken {
                        is_end: true,
                        name: String::new(),
                        self_closing: false,
                        attributes: Vec::new(),
                    });
                    self.reconsume();
                    self.state = TokenizerState::ScriptDataEndTagName;
                }
                Some(_) => {
                    self.emit_str("</");
                    self.reconsume();
                    self.state = TokenizerState::ScriptData;
                }
                None => {
                    self.emit_str("</");
                    self.state = TokenizerState::ScriptData;
                }
            },
            TokenizerState::ScriptDataEndTagName => {
                self.step_end_tag_name(TokenizerState::ScriptData);
            }
            TokenizerState::ScriptDataEscapeStart => match self.consume() {
                Some('-') => {
                    self.emit(Token::Character('-'));
                    self.state = TokenizerState::ScriptDataEscapeStartDash;
                }
                Some(_) => {
                    self.reconsume();
                    self.state = TokenizerState::ScriptData;
                }
                None => self.state = TokenizerState::ScriptData,
            },
            TokenizerState::ScriptDataEscapeStartDash => match self.consume() {
                Some('-') => {
                    self.emit(Token::Character('-'));
                    self.state = TokenizerState::ScriptDataEscapedDashDash;
                }
                Some(_) => {
                    self.reconsume();
                    self.state = TokenizerState::ScriptData;
                }
                None => self.state = TokenizerState::ScriptData,
            },
            TokenizerState::ScriptDataEscaped => match self.consume() {
                Some('-') => {
                    self.emit(Token::Character('-'));
                    self.state = TokenizerState::ScriptDataEscapedDash;
                }
                Some('<') => self.state = TokenizerState::ScriptDataEscapedLessThanSign,
                Some('\0') => self.emit(Token::Character('\u{FFFD}')),
                Some(c) => self.emit(Token::Character(c)),
                None => self.emit(Token::Eof),
            },
            TokenizerState::ScriptDataEscapedDash => match self.consume() {
                Some('-') => {
                    self.emit(Token::Character('-'));
                    self.state = TokenizerState::ScriptDataEscapedDashDash;
                }
                Some('<') => self.state = TokenizerState::ScriptDataEscapedLessThanSign,
                Some('\0') => {
                    self.emit(Token::Character('\u{FFFD}'));
                    self.state = TokenizerState::ScriptDataEscaped;
                }
                Some(c) => {
                    self.emit(Token::Character(c));
                    self.state = TokenizerState::ScriptDataEscaped;
                }
                None => self.emit(Token::Eof),
            },
            TokenizerState::ScriptDataEscapedDashDash => match self.consume() {
                Some('-') => self.emit(Token::Character('-')),
                Some('<') => self.state = TokenizerState::ScriptDataEscapedLessThanSign,
                Some('>') => {
                    self.emit(Token::Character('>'));
                    self.state = TokenizerState::ScriptData;
                }
                Some('\0') => {
                    self.emit(Token::Character('\u{FFFD}'));
                    self.state = TokenizerState::ScriptDataEscaped;
                }
                Some(c) => {
                    self.emit(Token::Character(c));
                    self.state = TokenizerState::ScriptDataEscaped;
                }
                None => self.emit(Token::Eof),
            },
            TokenizerState::ScriptDataEscapedLessThanSign => match self.consume() {
                Some('/') => {
                    self.temp_buffer.clear();
                    self.state = TokenizerState::ScriptDataEscapedEndTagOpen;
                }
                Some(c) if c.is_ascii_alphabetic() => {
                    self.temp_buffer.clear();
                    self.emit(Token::Character('<'));
                    self.reconsume();
                    self.state = TokenizerState::ScriptDataDoubleEscapeStart;
                }
                Some(_) => {
                    self.emit(Token::Character('<'));
                    self.reconsume();
                    self.state = TokenizerState::ScriptDataEscaped;
                }
                None => {
                    self.emit(Token::Character('<'));
                    self.state = TokenizerState::ScriptDataEscaped;
                }
            },
            TokenizerState::ScriptDataEscapedEndTagOpen => match self.consume() {
                Some(c) if c.is_ascii_alphabetic() => {
                    self.current_tag = Some(TagToken {
                        is_end: true,
                        name: String::new(),
                        self_closing: false,
                        attributes: Vec::new(),
                    });
                    self.reconsume();
                    self.state = TokenizerState::ScriptDataEscapedEndTagName;
                }
                Some(_) => {
                    self.emit_str("</");
                    self.reconsume();
                    self.state = TokenizerState::ScriptDataEscaped;
                }
                None => {
                    self.emit_str("</");
                    self.state = TokenizerState::ScriptDataEscaped;
                }
            },
            TokenizerState::ScriptDataEscapedEndTagName => {
                self.step_end_tag_name(TokenizerState::ScriptDataEscaped);
            }
            TokenizerState::ScriptDataDoubleEscapeStart => match self.consume() {
                Some(c @ ('\t' | '\n' | '\x0C' | ' ' | '/' | '>')) => {
                    self.emit(Token::Character(c));
                    if self.temp_buffer == "script" {
                        self.state = TokenizerState::ScriptDataDoubleEscaped;
                    } else {
                        self.state = TokenizerState::ScriptDataEscaped;
                    }
                }
                Some(c) if c.is_ascii_alphabetic() => {
                    self.temp_buffer.push(c.to_ascii_lowercase());
                    self.emit(Token::Character(c));
                }
                Some(_) => {
                    self.reconsume();
                    self.state = TokenizerState::ScriptDataEscaped;
                }
                None => self.state = TokenizerState::ScriptDataEscaped,
            },
            TokenizerState::ScriptDataDoubleEscaped => match self.consume() {
                Some('-') => {
                    self.emit(Token::Character('-'));
                    self.state = TokenizerState::ScriptDataDoubleEscapedDash;
                }
                Some('<') => {
                    self.emit(Token::Character('<'));
                    self.state = TokenizerState::ScriptDataDoubleEscapedLessThanSign;
                }
                Some('\0') => self.emit(Token::Character('\u{FFFD}')),
                Some(c) => self.emit(Token::Character(c)),
                None => self.emit(Token::Eof),
            },
            TokenizerState::ScriptDataDoubleEscapedDash => match self.consume() {
                Some('-') => {
                    self.emit(Token::Character('-'));
                    self.state = TokenizerState::ScriptDataDoubleEscapedDashDash;
                }
                Some('<') => {
                    self.emit(Token::Character('<'));
                    self.state = TokenizerState::ScriptDataDoubleEscapedLessThanSign;
                }
                Some('\0') => {
                    self.emit(Token::Character('\u{FFFD}'));
                    self.state = TokenizerState::ScriptDataDoubleEscaped;
                }
                Some(c) => {
                    self.emit(Token::Character(c));
                    self.state = TokenizerState::ScriptDataDoubleEscaped;
                }
                None => self.emit(Token::Eof),
            },
            TokenizerState::ScriptDataDoubleEscapedDashDash => match self.consume() {
                Some('-') => self.emit(Token::Character('-')),
                Some('<') => {
                    self.emit(Token::Character('<'));
                    self.state = TokenizerState::ScriptDataDoubleEscapedLessThanSign;
                }
                Some('>') => {
                    self.emit(Token::Character('>'));
                    self.state = TokenizerState::ScriptData;
                }
                Some('\0') => {
                    self.emit(Token::Character('\u{FFFD}'));
                    self.state = TokenizerState::ScriptDataDoubleEscaped;
                }
                Some(c) => {
                    self.emit(Token::Character(c));
                    self.state = TokenizerState::ScriptDataDoubleEscaped;
                }
                None => self.emit(Token::Eof),
            },
            TokenizerState::ScriptDataDoubleEscapedLessThanSign => match self.consume() {
                Some('/') => {
                    self.temp_buffer.clear();
                    self.emit(Token::Character('/'));
                    self.state = TokenizerState::ScriptDataDoubleEscapeEnd;
                }
                Some(_) => {
                    self.reconsume();
                    self.state = TokenizerState::ScriptDataDoubleEscaped;
                }
                None => self.state = TokenizerState::ScriptDataDoubleEscaped,
            },
            TokenizerState::ScriptDataDoubleEscapeEnd => match self.consume() {
                Some(c @ ('\t' | '\n' | '\x0C' | ' ' | '/' | '>')) => {
                    self.emit(Token::Character(c));
                    if self.temp_buffer == "script" {
                        self.state = TokenizerState::ScriptDataEscaped;
                    } else {
                        self.state = TokenizerState::ScriptDataDoubleEscaped;
                    }
                }
                Some(c) if c.is_ascii_alphabetic() => {
                    self.temp_buffer.push(c.to_ascii_lowercase());
                    self.emit(Token::Character(c));
                }
                Some(_) => {
                    self.reconsume();
                    self.state = TokenizerState::ScriptDataDoubleEscaped;
                }
                None => self.state = TokenizerState::ScriptDataDoubleEscaped,
            },
            TokenizerState::BeforeAttributeName => match self.consume() {
                Some('\t' | '\n' | '\x0C' | ' ') => {}
                Some('/' | '>') => {
                    self.reconsume();
                    self.state = TokenizerState::AfterAttributeName;
                }
                None => self.state = TokenizerState::AfterAttributeName,
                Some('=') => {
                    self.finish_attr();
                    self.has_current_attr = true;
                    self.current_attr_name = "=".to_string();
                    self.current_attr_value.clear();
                    self.state = TokenizerState::AttributeName;
                }
                Some(_) => {
                    self.finish_attr();
                    self.has_current_attr = true;
                    self.current_attr_name.clear();
                    self.current_attr_value.clear();
                    self.reconsume();
                    self.state = TokenizerState::AttributeName;
                }
            },
            TokenizerState::AttributeName => match self.consume() {
                Some('\t' | '\n' | '\x0C' | ' ' | '/' | '>') => {
                    self.reconsume();
                    self.state = TokenizerState::AfterAttributeName;
                }
                None => self.state = TokenizerState::AfterAttributeName,
                Some('=') => self.state = TokenizerState::BeforeAttributeValue,
                Some(c) if c.is_ascii_uppercase() => {
                    self.current_attr_name.push(c.to_ascii_lowercase());
                }
                Some('\0') => self.current_attr_name.push('\u{FFFD}'),
                Some(c) => self.current_attr_name.push(c),
            },
            TokenizerState::AfterAttributeName => match self.consume() {
                Some('\t' | '\n' | '\x0C' | ' ') => {}
                Some('/') => self.state = TokenizerState::SelfClosingStartTag,
                Some('=') => self.state = TokenizerState::BeforeAttributeValue,
                Some('>') => {
                    self.state = TokenizerState::Data;
                    self.emit_current_tag();
                }
                None => self.emit(Token::Eof),
                Some(_) => {
                    self.finish_attr();
                    self.has_current_attr = true;
                    self.current_attr_name.clear();
                    self.current_attr_value.clear();
                    self.reconsume();
                    self.state = TokenizerState::AttributeName;
                }
            },
            TokenizerState::BeforeAttributeValue => match self.consume() {
                Some('\t' | '\n' | '\x0C' | ' ') => {}
                Some('"') => self.state = TokenizerState::AttributeValueDoubleQuoted,
                Some('\'') => self.state = TokenizerState::AttributeValueSingleQuoted,
                Some('>') => {
                    self.state = TokenizerState::Data;
                    self.emit_current_tag();
                }
                Some(_) => {
                    self.reconsume();
                    self.state = TokenizerState::AttributeValueUnquoted;
                }
                None => self.state = TokenizerState::AttributeValueUnquoted,
            },
            TokenizerState::AttributeValueDoubleQuoted => match self.consume() {
                Some('"') => self.state = TokenizerState::AfterAttributeValueQuoted,
                Some('&') => {
                    self.return_state = TokenizerState::AttributeValueDoubleQuoted;
                    self.state = TokenizerState::CharacterReference;
                }
                Some('\0') => self.current_attr_value.push('\u{FFFD}'),
                Some(c) => self.current_attr_value.push(c),
                None => self.emit(Token::Eof),
            },
            TokenizerState::AttributeValueSingleQuoted => match self.consume() {
                Some('\'') => self.state = TokenizerState::AfterAttributeValueQuoted,
                Some('&') => {
                    self.return_state = TokenizerState::AttributeValueSingleQuoted;
                    self.state = TokenizerState::CharacterReference;
                }
                Some('\0') => self.current_attr_value.push('\u{FFFD}'),
                Some(c) => self.current_attr_value.push(c),
                None => self.emit(Token::Eof),
            },
            TokenizerState::AttributeValueUnquoted => match self.consume() {
                Some('\t' | '\n' | '\x0C' | ' ') => self.state = TokenizerState::BeforeAttributeName,
                Some('&') => {
                    self.return_state = TokenizerState::AttributeValueUnquoted;
                    self.state = TokenizerState::CharacterReference;
                }
                Some('>') => {
                    self.state = TokenizerState::Data;
                    self.emit_current_tag();
                }
                Some('\0') => self.current_attr_value.push('\u{FFFD}'),
                Some(c) => self.current_attr_value.push(c),
                None => self.emit(Token::Eof),
            },
            TokenizerState::AfterAttributeValueQuoted => match self.consume() {
                Some('\t' | '\n' | '\x0C' | ' ') => self.state = TokenizerState::BeforeAttributeName,
                Some('/') => self.state = TokenizerState::SelfClosingStartTag,
                Some('>') => {
                    self.state = TokenizerState::Data;
                    self.emit_current_tag();
                }
                None => self.emit(Token::Eof),
                Some(_) => {
                    self.reconsume();
                    self.state = TokenizerState::BeforeAttributeName;
                }
            },
            TokenizerState::SelfClosingStartTag => match self.consume() {
                Some('>') => {
                    if let Some(ref mut t) = self.current_tag {
                        t.self_closing = true;
                    }
                    self.state = TokenizerState::Data;
                    self.emit_current_tag();
                }
                None => self.emit(Token::Eof),
                Some(_) => {
                    self.reconsume();
                    self.state = TokenizerState::BeforeAttributeName;
                }
            },
            TokenizerState::BogusComment => match self.consume() {
                Some('>') => {
                    let c = std::mem::take(&mut self.current_comment);
                    self.emit(Token::Comment(c));
                    self.state = TokenizerState::Data;
                }
                Some('\0') => self.current_comment.push('\u{FFFD}'),
                Some(c) => self.current_comment.push(c),
                None => {
                    let c = std::mem::take(&mut self.current_comment);
                    self.emit(Token::Comment(c));
                    self.emit(Token::Eof);
                }
            },
            TokenizerState::MarkupDeclarationOpen => {
                if self.matches_exact("--") {
                    self.advance_by(2);
                    self.current_comment.clear();
                    self.state = TokenizerState::CommentStart;
                } else if self.matches_ascii_ci("DOCTYPE") {
                    self.advance_by(7);
                    self.state = TokenizerState::Doctype;
                } else if self.matches_exact("[CDATA[") {
                    self.advance_by(7);
                    if self.allow_cdata {
                        self.state = TokenizerState::CdataSection;
                    } else {
                        self.current_comment = "[CDATA[".to_string();
                        self.state = TokenizerState::BogusComment;
                    }
                } else {
                    self.current_comment.clear();
                    self.state = TokenizerState::BogusComment;
                }
            }
            TokenizerState::CommentStart => match self.consume() {
                Some('-') => self.state = TokenizerState::CommentStartDash,
                Some('>') => {
                    let c = std::mem::take(&mut self.current_comment);
                    self.emit(Token::Comment(c));
                    self.state = TokenizerState::Data;
                }
                Some(_) => {
                    self.reconsume();
                    self.state = TokenizerState::Comment;
                }
                None => self.state = TokenizerState::Comment,
            },
            TokenizerState::CommentStartDash => match self.consume() {
                Some('-') => self.state = TokenizerState::CommentEnd,
                Some('>') => {
                    let c = std::mem::take(&mut self.current_comment);
                    self.emit(Token::Comment(c));
                    self.state = TokenizerState::Data;
                }
                Some(_) => {
                    self.current_comment.push('-');
                    self.reconsume();
                    self.state = TokenizerState::Comment;
                }
                None => {
                    let c = std::mem::take(&mut self.current_comment);
                    self.emit(Token::Comment(c));
                    self.emit(Token::Eof);
                }
            },
            TokenizerState::Comment => match self.consume() {
                Some('<') => {
                    self.current_comment.push('<');
                    self.state = TokenizerState::CommentLessThanSign;
                }
                Some('-') => self.state = TokenizerState::CommentEndDash,
                Some('\0') => self.current_comment.push('\u{FFFD}'),
                Some(c) => self.current_comment.push(c),
                None => {
                    let c = std::mem::take(&mut self.current_comment);
                    self.emit(Token::Comment(c));
                    self.emit(Token::Eof);
                }
            },
            TokenizerState::CommentLessThanSign => match self.consume() {
                Some('!') => {
                    self.current_comment.push('!');
                    self.state = TokenizerState::CommentLessThanSignBang;
                }
                Some('<') => self.current_comment.push('<'),
                Some(_) => {
                    self.reconsume();
                    self.state = TokenizerState::Comment;
                }
                None => self.state = TokenizerState::Comment,
            },
            TokenizerState::CommentLessThanSignBang => match self.consume() {
                Some('-') => self.state = TokenizerState::CommentLessThanSignBangDash,
                Some(_) => {
                    self.reconsume();
                    self.state = TokenizerState::Comment;
                }
                None => self.state = TokenizerState::Comment,
            },
            TokenizerState::CommentLessThanSignBangDash => match self.consume() {
                Some('-') => self.state = TokenizerState::CommentLessThanSignBangDashDash,
                Some(_) => {
                    self.reconsume();
                    self.state = TokenizerState::CommentEndDash;
                }
                None => self.state = TokenizerState::CommentEndDash,
            },
            TokenizerState::CommentLessThanSignBangDashDash => match self.consume() {
                Some('>') => {
                    self.reconsume();
                    self.state = TokenizerState::CommentEnd;
                }
                Some(_) => {
                    self.reconsume();
                    self.state = TokenizerState::CommentEnd;
                }
                None => self.state = TokenizerState::CommentEnd,
            },
            TokenizerState::CommentEndDash => match self.consume() {
                Some('-') => self.state = TokenizerState::CommentEnd,
                Some(_) => {
                    self.current_comment.push('-');
                    self.reconsume();
                    self.state = TokenizerState::Comment;
                }
                None => {
                    let c = std::mem::take(&mut self.current_comment);
                    self.emit(Token::Comment(c));
                    self.emit(Token::Eof);
                }
            },
            TokenizerState::CommentEnd => match self.consume() {
                Some('>') => {
                    let c = std::mem::take(&mut self.current_comment);
                    self.emit(Token::Comment(c));
                    self.state = TokenizerState::Data;
                }
                Some('!') => self.state = TokenizerState::CommentEndBang,
                Some('-') => self.current_comment.push('-'),
                Some(_) => {
                    self.current_comment.push_str("--");
                    self.reconsume();
                    self.state = TokenizerState::Comment;
                }
                None => {
                    let c = std::mem::take(&mut self.current_comment);
                    self.emit(Token::Comment(c));
                    self.emit(Token::Eof);
                }
            },
            TokenizerState::CommentEndBang => match self.consume() {
                Some('-') => {
                    self.current_comment.push_str("--!");
                    self.state = TokenizerState::CommentEndDash;
                }
                Some('>') => {
                    let c = std::mem::take(&mut self.current_comment);
                    self.emit(Token::Comment(c));
                    self.state = TokenizerState::Data;
                }
                Some(_) => {
                    self.current_comment.push_str("--!");
                    self.reconsume();
                    self.state = TokenizerState::Comment;
                }
                None => {
                    let c = std::mem::take(&mut self.current_comment);
                    self.emit(Token::Comment(c));
                    self.emit(Token::Eof);
                }
            },
            TokenizerState::Doctype => match self.consume() {
                Some('\t' | '\n' | '\x0C' | ' ') => self.state = TokenizerState::BeforeDoctypeName,
                Some('>') => {
                    self.reconsume();
                    self.state = TokenizerState::BeforeDoctypeName;
                }
                Some(_) => {
                    self.reconsume();
                    self.state = TokenizerState::BeforeDoctypeName;
                }
                None => {
                    self.emit(Token::Doctype(DoctypeToken {
                        name: None,
                        public_id: None,
                        system_id: None,
                        force_quirks: true,
                    }));
                    self.emit(Token::Eof);
                }
            },
            TokenizerState::BeforeDoctypeName => match self.consume() {
                Some('\t' | '\n' | '\x0C' | ' ') => {}
                Some(c) if c.is_ascii_uppercase() => {
                    self.current_doctype = Some(DoctypeToken {
                        name: Some(c.to_ascii_lowercase().to_string()),
                        public_id: None,
                        system_id: None,
                        force_quirks: false,
                    });
                    self.state = TokenizerState::DoctypeName;
                }
                Some('\0') => {
                    self.current_doctype = Some(DoctypeToken {
                        name: Some("\u{FFFD}".to_string()),
                        public_id: None,
                        system_id: None,
                        force_quirks: false,
                    });
                    self.state = TokenizerState::DoctypeName;
                }
                Some('>') => {
                    self.emit(Token::Doctype(DoctypeToken {
                        name: None,
                        public_id: None,
                        system_id: None,
                        force_quirks: true,
                    }));
                    self.state = TokenizerState::Data;
                }
                Some(c) => {
                    self.current_doctype = Some(DoctypeToken {
                        name: Some(c.to_string()),
                        public_id: None,
                        system_id: None,
                        force_quirks: false,
                    });
                    self.state = TokenizerState::DoctypeName;
                }
                None => {
                    self.emit(Token::Doctype(DoctypeToken {
                        name: None,
                        public_id: None,
                        system_id: None,
                        force_quirks: true,
                    }));
                    self.emit(Token::Eof);
                }
            },
            TokenizerState::DoctypeName => match self.consume() {
                Some('\t' | '\n' | '\x0C' | ' ') => self.state = TokenizerState::AfterDoctypeName,
                Some('>') => {
                    self.state = TokenizerState::Data;
                    self.emit_current_doctype();
                }
                Some(c) if c.is_ascii_uppercase() => {
                    if let Some(ref mut dt) = self.current_doctype {
                        dt.name.get_or_insert_with(String::new).push(c.to_ascii_lowercase());
                    }
                }
                Some('\0') => {
                    if let Some(ref mut dt) = self.current_doctype {
                        dt.name.get_or_insert_with(String::new).push('\u{FFFD}');
                    }
                }
                Some(c) => {
                    if let Some(ref mut dt) = self.current_doctype {
                        dt.name.get_or_insert_with(String::new).push(c);
                    }
                }
                None => {
                    if let Some(ref mut dt) = self.current_doctype {
                        dt.force_quirks = true;
                    }
                    self.emit_current_doctype();
                    self.emit(Token::Eof);
                }
            },
            TokenizerState::AfterDoctypeName => match self.consume() {
                Some('\t' | '\n' | '\x0C' | ' ') => {}
                Some('>') => {
                    self.state = TokenizerState::Data;
                    self.emit_current_doctype();
                }
                None => {
                    if let Some(ref mut dt) = self.current_doctype {
                        dt.force_quirks = true;
                    }
                    self.emit_current_doctype();
                    self.emit(Token::Eof);
                }
                Some(_) => {
                    self.reconsume();
                    if self.matches_ascii_ci("PUBLIC") {
                        self.advance_by(6);
                        self.state = TokenizerState::AfterDoctypePublicKeyword;
                    } else if self.matches_ascii_ci("SYSTEM") {
                        self.advance_by(6);
                        self.state = TokenizerState::AfterDoctypeSystemKeyword;
                    } else {
                        if let Some(ref mut dt) = self.current_doctype {
                            dt.force_quirks = true;
                        }
                        self.state = TokenizerState::BogusDoctype;
                    }
                }
            },
            TokenizerState::AfterDoctypePublicKeyword => match self.consume() {
                Some('\t' | '\n' | '\x0C' | ' ') => {
                    self.state = TokenizerState::BeforeDoctypePublicIdentifier;
                }
                Some('"') => {
                    if let Some(ref mut dt) = self.current_doctype {
                        dt.public_id = Some(String::new());
                    }
                    self.state = TokenizerState::DoctypePublicIdentifierDoubleQuoted;
                }
                Some('\'') => {
                    if let Some(ref mut dt) = self.current_doctype {
                        dt.public_id = Some(String::new());
                    }
                    self.state = TokenizerState::DoctypePublicIdentifierSingleQuoted;
                }
                Some('>') => {
                    if let Some(ref mut dt) = self.current_doctype {
                        dt.force_quirks = true;
                    }
                    self.state = TokenizerState::Data;
                    self.emit_current_doctype();
                }
                Some(_) => {
                    if let Some(ref mut dt) = self.current_doctype {
                        dt.force_quirks = true;
                    }
                    self.reconsume();
                    self.state = TokenizerState::BogusDoctype;
                }
                None => {
                    if let Some(ref mut dt) = self.current_doctype {
                        dt.force_quirks = true;
                    }
                    self.emit_current_doctype();
                    self.emit(Token::Eof);
                }
            },
            TokenizerState::BeforeDoctypePublicIdentifier => match self.consume() {
                Some('\t' | '\n' | '\x0C' | ' ') => {}
                Some('"') => {
                    if let Some(ref mut dt) = self.current_doctype {
                        dt.public_id = Some(String::new());
                    }
                    self.state = TokenizerState::DoctypePublicIdentifierDoubleQuoted;
                }
                Some('\'') => {
                    if let Some(ref mut dt) = self.current_doctype {
                        dt.public_id = Some(String::new());
                    }
                    self.state = TokenizerState::DoctypePublicIdentifierSingleQuoted;
                }
                Some('>') => {
                    if let Some(ref mut dt) = self.current_doctype {
                        dt.force_quirks = true;
                    }
                    self.state = TokenizerState::Data;
                    self.emit_current_doctype();
                }
                Some(_) => {
                    if let Some(ref mut dt) = self.current_doctype {
                        dt.force_quirks = true;
                    }
                    self.reconsume();
                    self.state = TokenizerState::BogusDoctype;
                }
                None => {
                    if let Some(ref mut dt) = self.current_doctype {
                        dt.force_quirks = true;
                    }
                    self.emit_current_doctype();
                    self.emit(Token::Eof);
                }
            },
            TokenizerState::DoctypePublicIdentifierDoubleQuoted => match self.consume() {
                Some('"') => self.state = TokenizerState::AfterDoctypePublicIdentifier,
                Some('\0') => {
                    if let Some(ref mut dt) = self.current_doctype {
                        dt.public_id.get_or_insert_with(String::new).push('\u{FFFD}');
                    }
                }
                Some('>') => {
                    if let Some(ref mut dt) = self.current_doctype {
                        dt.force_quirks = true;
                    }
                    self.state = TokenizerState::Data;
                    self.emit_current_doctype();
                }
                Some(c) => {
                    if let Some(ref mut dt) = self.current_doctype {
                        dt.public_id.get_or_insert_with(String::new).push(c);
                    }
                }
                None => {
                    if let Some(ref mut dt) = self.current_doctype {
                        dt.force_quirks = true;
                    }
                    self.emit_current_doctype();
                    self.emit(Token::Eof);
                }
            },
            TokenizerState::DoctypePublicIdentifierSingleQuoted => match self.consume() {
                Some('\'') => self.state = TokenizerState::AfterDoctypePublicIdentifier,
                Some('\0') => {
                    if let Some(ref mut dt) = self.current_doctype {
                        dt.public_id.get_or_insert_with(String::new).push('\u{FFFD}');
                    }
                }
                Some('>') => {
                    if let Some(ref mut dt) = self.current_doctype {
                        dt.force_quirks = true;
                    }
                    self.state = TokenizerState::Data;
                    self.emit_current_doctype();
                }
                Some(c) => {
                    if let Some(ref mut dt) = self.current_doctype {
                        dt.public_id.get_or_insert_with(String::new).push(c);
                    }
                }
                None => {
                    if let Some(ref mut dt) = self.current_doctype {
                        dt.force_quirks = true;
                    }
                    self.emit_current_doctype();
                    self.emit(Token::Eof);
                }
            },
            TokenizerState::AfterDoctypePublicIdentifier => match self.consume() {
                Some('\t' | '\n' | '\x0C' | ' ') => {
                    self.state = TokenizerState::BetweenDoctypePublicAndSystemIdentifiers;
                }
                Some('>') => {
                    self.state = TokenizerState::Data;
                    self.emit_current_doctype();
                }
                Some('"') => {
                    if let Some(ref mut dt) = self.current_doctype {
                        dt.system_id = Some(String::new());
                    }
                    self.state = TokenizerState::DoctypeSystemIdentifierDoubleQuoted;
                }
                Some('\'') => {
                    if let Some(ref mut dt) = self.current_doctype {
                        dt.system_id = Some(String::new());
                    }
                    self.state = TokenizerState::DoctypeSystemIdentifierSingleQuoted;
                }
                Some(_) => {
                    if let Some(ref mut dt) = self.current_doctype {
                        dt.force_quirks = true;
                    }
                    self.reconsume();
                    self.state = TokenizerState::BogusDoctype;
                }
                None => {
                    if let Some(ref mut dt) = self.current_doctype {
                        dt.force_quirks = true;
                    }
                    self.emit_current_doctype();
                    self.emit(Token::Eof);
                }
            },
            TokenizerState::BetweenDoctypePublicAndSystemIdentifiers => match self.consume() {
                Some('\t' | '\n' | '\x0C' | ' ') => {}
                Some('>') => {
                    self.state = TokenizerState::Data;
                    self.emit_current_doctype();
                }
                Some('"') => {
                    if let Some(ref mut dt) = self.current_doctype {
                        dt.system_id = Some(String::new());
                    }
                    self.state = TokenizerState::DoctypeSystemIdentifierDoubleQuoted;
                }
                Some('\'') => {
                    if let Some(ref mut dt) = self.current_doctype {
                        dt.system_id = Some(String::new());
                    }
                    self.state = TokenizerState::DoctypeSystemIdentifierSingleQuoted;
                }
                Some(_) => {
                    if let Some(ref mut dt) = self.current_doctype {
                        dt.force_quirks = true;
                    }
                    self.reconsume();
                    self.state = TokenizerState::BogusDoctype;
                }
                None => {
                    if let Some(ref mut dt) = self.current_doctype {
                        dt.force_quirks = true;
                    }
                    self.emit_current_doctype();
                    self.emit(Token::Eof);
                }
            },
            TokenizerState::AfterDoctypeSystemKeyword => match self.consume() {
                Some('\t' | '\n' | '\x0C' | ' ') => {
                    self.state = TokenizerState::BeforeDoctypeSystemIdentifier;
                }
                Some('"') => {
                    if let Some(ref mut dt) = self.current_doctype {
                        dt.system_id = Some(String::new());
                    }
                    self.state = TokenizerState::DoctypeSystemIdentifierDoubleQuoted;
                }
                Some('\'') => {
                    if let Some(ref mut dt) = self.current_doctype {
                        dt.system_id = Some(String::new());
                    }
                    self.state = TokenizerState::DoctypeSystemIdentifierSingleQuoted;
                }
                Some('>') => {
                    if let Some(ref mut dt) = self.current_doctype {
                        dt.force_quirks = true;
                    }
                    self.state = TokenizerState::Data;
                    self.emit_current_doctype();
                }
                Some(_) => {
                    if let Some(ref mut dt) = self.current_doctype {
                        dt.force_quirks = true;
                    }
                    self.reconsume();
                    self.state = TokenizerState::BogusDoctype;
                }
                None => {
                    if let Some(ref mut dt) = self.current_doctype {
                        dt.force_quirks = true;
                    }
                    self.emit_current_doctype();
                    self.emit(Token::Eof);
                }
            },
            TokenizerState::BeforeDoctypeSystemIdentifier => match self.consume() {
                Some('\t' | '\n' | '\x0C' | ' ') => {}
                Some('"') => {
                    if let Some(ref mut dt) = self.current_doctype {
                        dt.system_id = Some(String::new());
                    }
                    self.state = TokenizerState::DoctypeSystemIdentifierDoubleQuoted;
                }
                Some('\'') => {
                    if let Some(ref mut dt) = self.current_doctype {
                        dt.system_id = Some(String::new());
                    }
                    self.state = TokenizerState::DoctypeSystemIdentifierSingleQuoted;
                }
                Some('>') => {
                    if let Some(ref mut dt) = self.current_doctype {
                        dt.force_quirks = true;
                    }
                    self.state = TokenizerState::Data;
                    self.emit_current_doctype();
                }
                Some(_) => {
                    if let Some(ref mut dt) = self.current_doctype {
                        dt.force_quirks = true;
                    }
                    self.reconsume();
                    self.state = TokenizerState::BogusDoctype;
                }
                None => {
                    if let Some(ref mut dt) = self.current_doctype {
                        dt.force_quirks = true;
                    }
                    self.emit_current_doctype();
                    self.emit(Token::Eof);
                }
            },
            TokenizerState::DoctypeSystemIdentifierDoubleQuoted => match self.consume() {
                Some('"') => self.state = TokenizerState::AfterDoctypeSystemIdentifier,
                Some('\0') => {
                    if let Some(ref mut dt) = self.current_doctype {
                        dt.system_id.get_or_insert_with(String::new).push('\u{FFFD}');
                    }
                }
                Some('>') => {
                    if let Some(ref mut dt) = self.current_doctype {
                        dt.force_quirks = true;
                    }
                    self.state = TokenizerState::Data;
                    self.emit_current_doctype();
                }
                Some(c) => {
                    if let Some(ref mut dt) = self.current_doctype {
                        dt.system_id.get_or_insert_with(String::new).push(c);
                    }
                }
                None => {
                    if let Some(ref mut dt) = self.current_doctype {
                        dt.force_quirks = true;
                    }
                    self.emit_current_doctype();
                    self.emit(Token::Eof);
                }
            },
            TokenizerState::DoctypeSystemIdentifierSingleQuoted => match self.consume() {
                Some('\'') => self.state = TokenizerState::AfterDoctypeSystemIdentifier,
                Some('\0') => {
                    if let Some(ref mut dt) = self.current_doctype {
                        dt.system_id.get_or_insert_with(String::new).push('\u{FFFD}');
                    }
                }
                Some('>') => {
                    if let Some(ref mut dt) = self.current_doctype {
                        dt.force_quirks = true;
                    }
                    self.state = TokenizerState::Data;
                    self.emit_current_doctype();
                }
                Some(c) => {
                    if let Some(ref mut dt) = self.current_doctype {
                        dt.system_id.get_or_insert_with(String::new).push(c);
                    }
                }
                None => {
                    if let Some(ref mut dt) = self.current_doctype {
                        dt.force_quirks = true;
                    }
                    self.emit_current_doctype();
                    self.emit(Token::Eof);
                }
            },
            TokenizerState::AfterDoctypeSystemIdentifier => match self.consume() {
                Some('\t' | '\n' | '\x0C' | ' ') => {}
                Some('>') => {
                    self.state = TokenizerState::Data;
                    self.emit_current_doctype();
                }
                Some(_) => {
                    self.reconsume();
                    self.state = TokenizerState::BogusDoctype;
                }
                None => {
                    if let Some(ref mut dt) = self.current_doctype {
                        dt.force_quirks = true;
                    }
                    self.emit_current_doctype();
                    self.emit(Token::Eof);
                }
            },
            TokenizerState::BogusDoctype => match self.consume() {
                Some('>') => {
                    self.state = TokenizerState::Data;
                    self.emit_current_doctype();
                }
                Some(_) => {}
                None => {
                    self.emit_current_doctype();
                    self.emit(Token::Eof);
                }
            },
            TokenizerState::CdataSection => match self.consume() {
                Some(']') => self.state = TokenizerState::CdataSectionBracket,
                Some(c) => self.emit(Token::Character(c)),
                None => self.emit(Token::Eof),
            },
            TokenizerState::CdataSectionBracket => match self.consume() {
                Some(']') => self.state = TokenizerState::CdataSectionEnd,
                Some(_) => {
                    self.emit(Token::Character(']'));
                    self.reconsume();
                    self.state = TokenizerState::CdataSection;
                }
                None => {
                    self.emit(Token::Character(']'));
                    self.state = TokenizerState::CdataSection;
                }
            },
            TokenizerState::CdataSectionEnd => match self.consume() {
                Some(']') => self.emit(Token::Character(']')),
                Some('>') => self.state = TokenizerState::Data,
                Some(_) => {
                    self.emit_str("]]");
                    self.reconsume();
                    self.state = TokenizerState::CdataSection;
                }
                None => {
                    self.emit_str("]]");
                    self.state = TokenizerState::CdataSection;
                }
            },
            TokenizerState::CharacterReference => {
                self.temp_buffer.clear();
                self.temp_buffer.push('&');
                match self.consume() {
                    Some(c) if c.is_ascii_alphanumeric() => {
                        self.reconsume();
                        self.state = TokenizerState::NamedCharacterReference;
                    }
                    Some('#') => {
                        self.temp_buffer.push('#');
                        self.state = TokenizerState::NumericCharacterReference;
                    }
                    Some(_) => {
                        self.flush_code_points_as_char_ref();
                        self.reconsume();
                        self.state = self.return_state;
                    }
                    None => {
                        self.flush_code_points_as_char_ref();
                        self.state = self.return_state;
                    }
                }
            }
            TokenizerState::NamedCharacterReference => {
                let mut slice = String::new();
                for &ch in self.chars[self.pos..].iter().take(32) {
                    slice.push(ch);
                }
                if let Some((matched_name, replacement)) = lookup_entity_prefix(&slice) {
                    let count = matched_name.chars().count();
                    let next_char = self.chars.get(self.pos + count).copied();
                    let ends_with_semi = matched_name.ends_with(';');
                    if self.consumed_as_part_of_attribute()
                        && !ends_with_semi
                        && matches!(next_char, Some('=') | Some('0'..='9') | Some('a'..='z') | Some('A'..='Z'))
                    {
                        self.advance_by(count);
                        self.temp_buffer.push_str(matched_name);
                        self.flush_code_points_as_char_ref();
                    } else {
                        self.advance_by(count);
                        self.temp_buffer.clear();
                        self.temp_buffer.push_str(replacement);
                        self.flush_code_points_as_char_ref();
                    }
                } else {
                    self.flush_code_points_as_char_ref();
                }
                self.state = self.return_state;
            }
            TokenizerState::NumericCharacterReference => {
                self.char_ref_code = 0;
                match self.consume() {
                    Some(c @ ('x' | 'X')) => {
                        self.temp_buffer.push(c);
                        self.state = TokenizerState::HexadecimalCharacterReferenceStart;
                    }
                    Some(_) => {
                        self.reconsume();
                        self.state = TokenizerState::DecimalCharacterReferenceStart;
                    }
                    None => self.state = TokenizerState::DecimalCharacterReferenceStart,
                }
            }
            TokenizerState::HexadecimalCharacterReferenceStart => match self.consume() {
                Some(c) if c.is_ascii_hexdigit() => {
                    self.reconsume();
                    self.state = TokenizerState::HexadecimalCharacterReference;
                }
                Some(_) => {
                    self.flush_code_points_as_char_ref();
                    self.reconsume();
                    self.state = self.return_state;
                }
                None => {
                    self.flush_code_points_as_char_ref();
                    self.state = self.return_state;
                }
            },
            TokenizerState::DecimalCharacterReferenceStart => match self.consume() {
                Some(c) if c.is_ascii_digit() => {
                    self.reconsume();
                    self.state = TokenizerState::DecimalCharacterReference;
                }
                Some(_) => {
                    self.flush_code_points_as_char_ref();
                    self.reconsume();
                    self.state = self.return_state;
                }
                None => {
                    self.flush_code_points_as_char_ref();
                    self.state = self.return_state;
                }
            },
            TokenizerState::HexadecimalCharacterReference => match self.consume() {
                Some(c) if c.is_ascii_hexdigit() => {
                    let digit = c.to_digit(16).unwrap();
                    self.char_ref_code = self.char_ref_code.saturating_mul(16).saturating_add(digit);
                    if self.char_ref_code > 0x10FFFF {
                        self.char_ref_code = 0x110000;
                    }
                }
                Some(';') => self.state = TokenizerState::NumericCharacterReferenceEnd,
                Some(_) => {
                    self.reconsume();
                    self.state = TokenizerState::NumericCharacterReferenceEnd;
                }
                None => self.state = TokenizerState::NumericCharacterReferenceEnd,
            },
            TokenizerState::DecimalCharacterReference => match self.consume() {
                Some(c) if c.is_ascii_digit() => {
                    let digit = c.to_digit(10).unwrap();
                    self.char_ref_code = self.char_ref_code.saturating_mul(10).saturating_add(digit);
                    if self.char_ref_code > 0x10FFFF {
                        self.char_ref_code = 0x110000;
                    }
                }
                Some(';') => self.state = TokenizerState::NumericCharacterReferenceEnd,
                Some(_) => {
                    self.reconsume();
                    self.state = TokenizerState::NumericCharacterReferenceEnd;
                }
                None => self.state = TokenizerState::NumericCharacterReferenceEnd,
            },
            TokenizerState::NumericCharacterReferenceEnd => {
                let ch = decode_numeric_char_ref(self.char_ref_code);
                self.temp_buffer.clear();
                self.temp_buffer.push(ch);
                self.flush_code_points_as_char_ref();
                self.state = self.return_state;
            }
        }
    }

    fn step_end_tag_name(&mut self, fallback: TokenizerState) {
        match self.consume() {
            Some('\t' | '\n' | '\x0C' | ' ') if self.is_appropriate_end_tag() => {
                self.state = TokenizerState::BeforeAttributeName;
            }
            Some('/') if self.is_appropriate_end_tag() => {
                self.state = TokenizerState::SelfClosingStartTag;
            }
            Some('>') if self.is_appropriate_end_tag() => {
                self.state = TokenizerState::Data;
                self.emit_current_tag();
            }
            Some(c) if c.is_ascii_uppercase() => {
                if let Some(ref mut t) = self.current_tag {
                    t.name.push(c.to_ascii_lowercase());
                }
                self.temp_buffer.push(c);
            }
            Some(c) if c.is_ascii_lowercase() => {
                if let Some(ref mut t) = self.current_tag {
                    t.name.push(c);
                }
                self.temp_buffer.push(c);
            }
            Some(_) => {
                self.current_tag = None;
                self.emit_str("</");
                let buf = std::mem::take(&mut self.temp_buffer);
                self.emit_str(&buf);
                self.reconsume();
                self.state = fallback;
            }
            None => {
                self.current_tag = None;
                self.emit_str("</");
                let buf = std::mem::take(&mut self.temp_buffer);
                self.emit_str(&buf);
                self.state = fallback;
            }
        }
    }
}

fn preprocess_input(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\r' {
            if chars.peek() == Some(&'\n') {
                chars.next();
            }
            out.push('\n');
        } else {
            out.push(c);
        }
    }
    out
}

fn decode_numeric_char_ref(code: u32) -> char {
    match code {
        0x00 => '\u{FFFD}',
        0x80 => '\u{20AC}',
        0x82 => '\u{201A}',
        0x83 => '\u{0192}',
        0x84 => '\u{201E}',
        0x85 => '\u{2026}',
        0x86 => '\u{2020}',
        0x87 => '\u{2021}',
        0x88 => '\u{02C6}',
        0x89 => '\u{2030}',
        0x8A => '\u{0160}',
        0x8B => '\u{2039}',
        0x8C => '\u{0152}',
        0x8E => '\u{017D}',
        0x91 => '\u{2018}',
        0x92 => '\u{2019}',
        0x93 => '\u{201C}',
        0x94 => '\u{201D}',
        0x95 => '\u{2022}',
        0x96 => '\u{2013}',
        0x97 => '\u{2014}',
        0x98 => '\u{02DC}',
        0x99 => '\u{2122}',
        0x9A => '\u{0161}',
        0x9B => '\u{203A}',
        0x9C => '\u{0153}',
        0x9E => '\u{017E}',
        0x9F => '\u{0178}',
        c if c > 0x10FFFF || (0xD800..=0xDFFF).contains(&c) => '\u{FFFD}',
        c => char::from_u32(c).unwrap_or('\u{FFFD}'),
    }
}
