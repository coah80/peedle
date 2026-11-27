use crate::error::JsError;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RegExpFlags {
    pub global: bool,
    pub ignore_case: bool,
    pub multiline: bool,
    pub dot_all: bool,
    pub unicode: bool,
    pub unicode_sets: bool,
    pub sticky: bool,
    pub has_indices: bool,
}

impl RegExpFlags {
    pub fn is_unicode(&self) -> bool {
        self.unicode || self.unicode_sets
    }

    pub fn parse(s: &str) -> Result<Self, JsError> {
        let mut flags = RegExpFlags::default();
        for ch in s.chars() {
            match ch {
                'g' => {
                    if flags.global {
                        return Err(JsError::syntax("Duplicate regex flag 'g'"));
                    }
                    flags.global = true;
                }
                'i' => {
                    if flags.ignore_case {
                        return Err(JsError::syntax("Duplicate regex flag 'i'"));
                    }
                    flags.ignore_case = true;
                }
                'm' => {
                    if flags.multiline {
                        return Err(JsError::syntax("Duplicate regex flag 'm'"));
                    }
                    flags.multiline = true;
                }
                's' => {
                    if flags.dot_all {
                        return Err(JsError::syntax("Duplicate regex flag 's'"));
                    }
                    flags.dot_all = true;
                }
                'u' => {
                    if flags.unicode || flags.unicode_sets {
                        return Err(JsError::syntax("Duplicate or incompatible regex flag 'u'"));
                    }
                    flags.unicode = true;
                }
                'y' => {
                    if flags.sticky {
                        return Err(JsError::syntax("Duplicate regex flag 'y'"));
                    }
                    flags.sticky = true;
                }
                'd' => {
                    if flags.has_indices {
                        return Err(JsError::syntax("Duplicate regex flag 'd'"));
                    }
                    flags.has_indices = true;
                }
                'v' => {
                    if flags.unicode_sets || flags.unicode {
                        return Err(JsError::syntax("Duplicate or incompatible regex flag 'v'"));
                    }
                    flags.unicode_sets = true;
                }
                other => {
                    return Err(JsError::syntax(format!("Invalid regex flag '{}'", other)));
                }
            }
        }
        Ok(flags)
    }

    pub fn canonical_string(&self) -> String {
        let mut s = String::new();
        if self.has_indices {
            s.push('d');
        }
        if self.global {
            s.push('g');
        }
        if self.ignore_case {
            s.push('i');
        }
        if self.multiline {
            s.push('m');
        }
        if self.dot_all {
            s.push('s');
        }
        if self.unicode {
            s.push('u');
        }
        if self.unicode_sets {
            s.push('v');
        }
        if self.sticky {
            s.push('y');
        }
        s
    }
}

#[derive(Debug, Clone)]
pub struct RegExpMatch {
    pub start: usize,
    pub end: usize,
    pub captures: Vec<Option<(usize, usize)>>,
    pub named_groups: HashMap<String, Option<(usize, usize)>>,
}

#[derive(Debug, Clone)]
enum ClassAtom {
    Char(u32),
    Range(u32, u32),
    Digit(bool),
    Word(bool),
    Space(bool),
    UnicodeProp(String, bool),
}

#[derive(Debug, Clone)]
enum ClassSetExpr {
    Union(Vec<ClassSetExpr>),
    Intersection(Vec<ClassSetExpr>),
    Difference(Vec<ClassSetExpr>),
    Negated(Box<ClassSetExpr>),
    Atom(ClassAtom),
    StringLit(Vec<u32>),
    StringProp(String),
}

#[derive(Debug, Clone)]
enum Node {
    Seq(Vec<Node>),
    Alt(Vec<Node>),
    Literal(u32),
    Dot,
    CharClass {
        negated: bool,
        atoms: Vec<ClassAtom>,
    },
    ClassSet(ClassSetExpr),
    Digit(bool),
    Word(bool),
    Space(bool),
    UnicodeProp(String, bool),
    AnchorStart,
    AnchorEnd,
    WordBoundary(bool),
    Group {
        index: Option<usize>,
        body: Box<Node>,
    },
    ModifiedGroup {
        add_i: bool,
        add_m: bool,
        add_s: bool,
        rem_i: bool,
        rem_m: bool,
        rem_s: bool,
        body: Box<Node>,
    },
    Backref(usize),
    NamedBackref(String),
    Quantifier {
        body: Box<Node>,
        min: usize,
        max: Option<usize>,
        greedy: bool,
        cap_start: usize,
        cap_end: usize,
    },
    Lookaround {
        body: Box<Node>,
        positive: bool,
        behind: bool,
    },
}

#[derive(Debug, Clone)]
pub struct CompiledRegExp {
    pub source: String,
    pub flags: RegExpFlags,
    root: Node,
    capture_count: usize,
    named_groups: Vec<(String, usize)>,
}

impl CompiledRegExp {
    pub fn compile(pattern: &str, flag_str: &str) -> Result<Self, JsError> {
        let flags = RegExpFlags::parse(flag_str)?;
        let chars: Vec<char> = pattern.chars().collect();
        let mut parser = RegexParser {
            chars: &chars,
            pos: 0,
            flags,
            capture_count: 0,
            named_groups: Vec::new(),
        };
        let (root, _) = parser.parse_alt()?;
        if parser.pos < chars.len() {
            return Err(JsError::syntax("Unexpected ')' in regular expression"));
        }
        validate_regexp_refs(
            &root,
            parser.capture_count,
            &parser.named_groups,
            flags.is_unicode(),
        )?;
        Ok(CompiledRegExp {
            source: if pattern.is_empty() {
                "(?:)".to_string()
            } else {
                pattern.to_string()
            },
            flags,
            root,
            capture_count: parser.capture_count,
            named_groups: parser.named_groups,
        })
    }

    pub fn capture_count(&self) -> usize {
        self.capture_count
    }

    pub fn named_groups(&self) -> &[(String, usize)] {
        &self.named_groups
    }

    pub fn exec(&self, input: &str, start_utf16: usize) -> Option<RegExpMatch> {
        let units: Vec<u16> = input.encode_utf16().collect();
        if start_utf16 > units.len() {
            return None;
        }
        let max_start = if self.flags.sticky {
            start_utf16
        } else {
            units.len()
        };
        for pos in start_utf16..=max_start {
            let mut state = MatchState {
                units: &units,
                flags: self.flags,
                captures: vec![None; self.capture_count + 1],
                named_groups: &self.named_groups,
                steps: 0,
            };
            if let Some(end) = state.match_node(&self.root, pos, false) {
                state.captures[0] = Some((pos, end));
                let mut named = HashMap::new();
                for (name, idx) in &self.named_groups {
                    named.insert(name.clone(), state.captures[*idx]);
                }
                return Some(RegExpMatch {
                    start: pos,
                    end,
                    captures: state.captures,
                    named_groups: named,
                });
            }
            if self.flags.sticky {
                break;
            }
        }
        None
    }

    pub fn exec_utf16(&self, units: &[u16], start_utf16: usize) -> Option<RegExpMatch> {
        if start_utf16 > units.len() {
            return None;
        }
        let max_start = if self.flags.sticky {
            start_utf16
        } else {
            units.len()
        };
        for pos in start_utf16..=max_start {
            let mut state = MatchState {
                units,
                flags: self.flags,
                captures: vec![None; self.capture_count + 1],
                named_groups: &self.named_groups,
                steps: 0,
            };
            if let Some(end) = state.match_node(&self.root, pos, false) {
                state.captures[0] = Some((pos, end));
                let mut named = HashMap::new();
                for (name, idx) in &self.named_groups {
                    named.insert(name.clone(), state.captures[*idx]);
                }
                return Some(RegExpMatch {
                    start: pos,
                    end,
                    captures: state.captures,
                    named_groups: named,
                });
            }
            if self.flags.sticky {
                break;
            }
        }
        None
    }
}

struct RegexParser<'a> {
    chars: &'a [char],
    pos: usize,
    flags: RegExpFlags,
    capture_count: usize,
    named_groups: Vec<(String, usize)>,
}

impl<'a> RegexParser<'a> {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn next(&mut self) -> Option<char> {
        let c = self.chars.get(self.pos).copied()?;
        self.pos += 1;
        Some(c)
    }

    fn parse_alt(&mut self) -> Result<(Node, std::collections::HashSet<String>), JsError> {
        let (first, mut all_names) = self.parse_seq()?;
        let mut branches = vec![first];
        while self.peek() == Some('|') {
            self.pos += 1;
            let (b, b_names) = self.parse_seq()?;
            all_names.extend(b_names);
            branches.push(b);
        }
        let node = if branches.len() == 1 {
            branches.pop().unwrap()
        } else {
            Node::Alt(branches)
        };
        Ok((node, all_names))
    }

    fn parse_seq(&mut self) -> Result<(Node, std::collections::HashSet<String>), JsError> {
        let mut items = Vec::new();
        let mut seq_names = std::collections::HashSet::new();
        while let Some(c) = self.peek() {
            if c == '|' || c == ')' {
                break;
            }
            let cap_before = self.capture_count;
            let (atom, atom_names) = self.parse_atom()?;
            for n in atom_names {
                if !seq_names.insert(n.clone()) {
                    return Err(JsError::syntax(format!(
                        "Duplicate capture group name '{}'",
                        n
                    )));
                }
            }
            let cap_after = self.capture_count;
            let node = self.parse_quantifier(atom, cap_before + 1, cap_after)?;
            items.push(node);
        }
        let node = if items.len() == 1 {
            items.pop().unwrap()
        } else {
            Node::Seq(items)
        };
        Ok((node, seq_names))
    }

    fn parse_atom(&mut self) -> Result<(Node, std::collections::HashSet<String>), JsError> {
        let Some(c) = self.next() else {
            return Err(JsError::syntax("Unexpected end of regular expression"));
        };
        let empty = std::collections::HashSet::new();
        match c {
            '^' => Ok((Node::AnchorStart, empty)),
            '$' => Ok((Node::AnchorEnd, empty)),
            '.' => Ok((Node::Dot, empty)),
            '[' => Ok((self.parse_char_class()?, empty)),
            '(' => self.parse_group(),
            '\\' => Ok((self.parse_escape(false)?, empty)),
            '*' | '+' | '?' => Err(JsError::syntax("Nothing to repeat in regular expression")),
            '{' => {
                self.pos -= 1;
                if self.try_parse_brace_quantifier()?.is_some() {
                    return Err(JsError::syntax("Nothing to repeat in regular expression"));
                }
                if self.flags.is_unicode() {
                    return Err(JsError::syntax("Lone quantifier bracket in Unicode regex"));
                }
                self.pos += 1;
                Ok((Node::Literal('{' as u32), empty))
            }
            '}' | ']' if self.flags.is_unicode() => {
                Err(JsError::syntax("Lone bracket in Unicode regex"))
            }
            other => Ok((Node::Literal(other as u32), empty)),
        }
    }

    fn parse_group_name(&mut self) -> Result<String, JsError> {
        let mut name = String::new();
        let mut closed = false;
        let mut first = true;
        while let Some(ch) = self.next() {
            if ch == '>' {
                closed = true;
                break;
            }
            let decoded = if ch == '\\' {
                if self.next() != Some('u') {
                    return Err(JsError::syntax("Invalid escape in regex group name"));
                }
                let cp = if self.peek() == Some('{') {
                    self.pos += 1;
                    let mut val = 0u32;
                    let mut count = 0usize;
                    let mut end_brace = false;
                    while let Some(hc) = self.next() {
                        if hc == '}' {
                            end_brace = true;
                            break;
                        }
                        let Some(d) = hc.to_digit(16) else {
                            return Err(JsError::syntax("Invalid hex in regex group name"));
                        };
                        val = val.saturating_mul(16).saturating_add(d);
                        count += 1;
                        if val > 0x10FFFF {
                            return Err(JsError::syntax("Code point out of range in group name"));
                        }
                    }
                    if !end_brace || count == 0 {
                        return Err(JsError::syntax("Invalid unicode escape in group name"));
                    }
                    val
                } else {
                    let Some(hi) = self.try_read_hex(4) else {
                        return Err(JsError::syntax("Invalid unicode escape in group name"));
                    };
                    if (0xD800..=0xDBFF).contains(&hi)
                        && self.chars.get(self.pos) == Some(&'\\')
                        && self.chars.get(self.pos + 1) == Some(&'u')
                    {
                        let saved = self.pos;
                        self.pos += 2;
                        if let Some(lo) = self.try_read_hex(4) {
                            if (0xDC00..=0xDFFF).contains(&lo) {
                                0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00)
                            } else {
                                self.pos = saved;
                                return Err(JsError::syntax("Lone surrogate in group name"));
                            }
                        } else {
                            self.pos = saved;
                            return Err(JsError::syntax("Lone surrogate in group name"));
                        }
                    } else {
                        hi
                    }
                };
                if (0xD800..=0xDFFF).contains(&cp) {
                    return Err(JsError::syntax("Lone surrogate in group name"));
                }
                char::from_u32(cp)
                    .ok_or_else(|| JsError::syntax("Invalid code point in group name"))?
            } else {
                ch
            };
            if (first && !crate::lexer::is_ident_start(decoded))
                || (!first && !crate::lexer::is_ident_continue(decoded))
            {
                return Err(JsError::syntax("Invalid character in regex group name"));
            }
            name.push(decoded);
            first = false;
        }
        if !closed || name.is_empty() {
            return Err(JsError::syntax("Invalid or unterminated group name in regex"));
        }
        Ok(name)
    }

    fn parse_group(&mut self) -> Result<(Node, std::collections::HashSet<String>), JsError> {
        if self.peek() == Some('?') {
            self.pos += 1;
            match self.peek() {
                Some(':') => {
                    self.pos += 1;
                    let (body, names) = self.parse_alt()?;
                    self.expect_char(')')?;
                    Ok((
                        Node::Group {
                            index: None,
                            body: Box::new(body),
                        },
                        names,
                    ))
                }
                Some('=') => {
                    self.pos += 1;
                    let (body, names) = self.parse_alt()?;
                    self.expect_char(')')?;
                    Ok((
                        Node::Lookaround {
                            body: Box::new(body),
                            positive: true,
                            behind: false,
                        },
                        names,
                    ))
                }
                Some('!') => {
                    self.pos += 1;
                    let (body, names) = self.parse_alt()?;
                    self.expect_char(')')?;
                    Ok((
                        Node::Lookaround {
                            body: Box::new(body),
                            positive: false,
                            behind: false,
                        },
                        names,
                    ))
                }
                Some('<') => {
                    self.pos += 1;
                    match self.peek() {
                        Some('=') => {
                            self.pos += 1;
                            let (body, names) = self.parse_alt()?;
                            self.expect_char(')')?;
                            Ok((
                                Node::Lookaround {
                                    body: Box::new(body),
                                    positive: true,
                                    behind: true,
                                },
                                names,
                            ))
                        }
                        Some('!') => {
                            self.pos += 1;
                            let (body, names) = self.parse_alt()?;
                            self.expect_char(')')?;
                            Ok((
                                Node::Lookaround {
                                    body: Box::new(body),
                                    positive: false,
                                    behind: true,
                                },
                                names,
                            ))
                        }
                        _ => {
                            let name = self.parse_group_name()?;
                            self.capture_count += 1;
                            let idx = self.capture_count;
                            self.named_groups.push((name.clone(), idx));
                            let (body, mut names) = self.parse_alt()?;
                            self.expect_char(')')?;
                            if !names.insert(name.clone()) {
                                return Err(JsError::syntax(format!(
                                    "Duplicate capture group name '{}'",
                                    name
                                )));
                            }
                            Ok((
                                Node::Group {
                                    index: Some(idx),
                                    body: Box::new(body),
                                },
                                names,
                            ))
                        }
                    }
                }
                Some('i' | 'm' | 's' | '-') => {
                    let mut add_i = false;
                    let mut add_m = false;
                    let mut add_s = false;
                    let mut rem_i = false;
                    let mut rem_m = false;
                    let mut rem_s = false;
                    let mut in_rem = false;
                    let mut any_flag = false;
                    loop {
                        match self.next() {
                            Some(':') => break,
                            Some('-') if !in_rem => {
                                in_rem = true;
                            }
                            Some('i') => {
                                any_flag = true;
                                if in_rem {
                                    if rem_i || add_i {
                                        return Err(JsError::syntax("Invalid regex modifier"));
                                    }
                                    rem_i = true;
                                } else {
                                    if add_i {
                                        return Err(JsError::syntax("Duplicate regex modifier"));
                                    }
                                    add_i = true;
                                }
                            }
                            Some('m') => {
                                any_flag = true;
                                if in_rem {
                                    if rem_m || add_m {
                                        return Err(JsError::syntax("Invalid regex modifier"));
                                    }
                                    rem_m = true;
                                } else {
                                    if add_m {
                                        return Err(JsError::syntax("Duplicate regex modifier"));
                                    }
                                    add_m = true;
                                }
                            }
                            Some('s') => {
                                any_flag = true;
                                if in_rem {
                                    if rem_s || add_s {
                                        return Err(JsError::syntax("Invalid regex modifier"));
                                    }
                                    rem_s = true;
                                } else {
                                    if add_s {
                                        return Err(JsError::syntax("Duplicate regex modifier"));
                                    }
                                    add_s = true;
                                }
                            }
                            _ => {
                                return Err(JsError::syntax("Invalid group modifier in regex"));
                            }
                        }
                    }
                    if !any_flag {
                        return Err(JsError::syntax("Empty regex modifier group"));
                    }
                    let (body, names) = self.parse_alt()?;
                    self.expect_char(')')?;
                    Ok((
                        Node::ModifiedGroup {
                            add_i,
                            add_m,
                            add_s,
                            rem_i,
                            rem_m,
                            rem_s,
                            body: Box::new(body),
                        },
                        names,
                    ))
                }
                _ => Err(JsError::syntax("Invalid group specifier in regex")),
            }
        } else {
            self.capture_count += 1;
            let idx = self.capture_count;
            let (body, names) = self.parse_alt()?;
            self.expect_char(')')?;
            Ok((
                Node::Group {
                    index: Some(idx),
                    body: Box::new(body),
                },
                names,
            ))
        }
    }

    fn expect_char(&mut self, expected: char) -> Result<(), JsError> {
        if self.next() == Some(expected) {
            Ok(())
        } else {
            Err(JsError::syntax(format!(
                "Expected '{}' in regular expression",
                expected
            )))
        }
    }

    fn parse_escape(&mut self, in_class: bool) -> Result<Node, JsError> {
        let Some(c) = self.next() else {
            return Err(JsError::syntax("Trailing backslash in regular expression"));
        };
        match c {
            'd' => Ok(Node::Digit(false)),
            'D' => Ok(Node::Digit(true)),
            'w' => Ok(Node::Word(false)),
            'W' => Ok(Node::Word(true)),
            's' => Ok(Node::Space(false)),
            'S' => Ok(Node::Space(true)),
            'b' if !in_class => Ok(Node::WordBoundary(false)),
            'B' if !in_class => Ok(Node::WordBoundary(true)),
            'b' => Ok(Node::Literal(0x08)),
            'n' => Ok(Node::Literal('\n' as u32)),
            'r' => Ok(Node::Literal('\r' as u32)),
            't' => Ok(Node::Literal('\t' as u32)),
            'f' => Ok(Node::Literal(0x0C)),
            'v' => Ok(Node::Literal(0x0B)),
            '0' => {
                if self.peek().is_some_and(|ch| ch.is_ascii_digit()) {
                    if self.flags.is_unicode() {
                        return Err(JsError::syntax("Invalid octal escape in Unicode regex"));
                    }
                    if !in_class && self.peek().is_some_and(|ch| ('0'..='7').contains(&ch)) {
                        let mut val = 0u32;
                        for _ in 0..2 {
                            if let Some(ch) = self.peek() {
                                if ('0'..='7').contains(&ch) {
                                    self.pos += 1;
                                    val = val * 8 + (ch as u32 - '0' as u32);
                                } else {
                                    break;
                                }
                            }
                        }
                        return Ok(Node::Literal(val));
                    }
                }
                Ok(Node::Literal(0))
            }
            '1'..='9' if !in_class => {
                let mut num = (c as usize) - ('0' as usize);
                while let Some(d) = self.peek() {
                    if d.is_ascii_digit() {
                        self.pos += 1;
                        num = num * 10 + (d as usize - '0' as usize);
                    } else {
                        break;
                    }
                }
                Ok(Node::Backref(num))
            }
            '1'..='9' if in_class && self.flags.is_unicode() => {
                Err(JsError::syntax("Invalid decimal escape in Unicode character class"))
            }
            'k' if !in_class => {
                if self.peek() == Some('<') {
                    self.pos += 1;
                    let name = self.parse_group_name()?;
                    Ok(Node::NamedBackref(name))
                } else if self.flags.is_unicode() {
                    Err(JsError::syntax("Invalid named backreference in Unicode regex"))
                } else {
                    Ok(Node::Literal('k' as u32))
                }
            }
            'x' => {
                if let Some(val) = self.try_read_hex(2) {
                    Ok(Node::Literal(val))
                } else if self.flags.is_unicode() {
                    Err(JsError::syntax("Invalid hex escape in Unicode regex"))
                } else {
                    Ok(Node::Literal('x' as u32))
                }
            }
            'u' => {
                if self.peek() == Some('{') {
                    let saved = self.pos;
                    self.pos += 1;
                    let mut val = 0u32;
                    let mut ok = false;
                    let mut digits = 0usize;
                    while let Some(ch) = self.next() {
                        if ch == '}' {
                            ok = digits > 0;
                            break;
                        }
                        if let Some(d) = ch.to_digit(16) {
                            digits += 1;
                            val = val.saturating_mul(16).saturating_add(d);
                        } else {
                            ok = false;
                            break;
                        }
                    }
                    if ok && val <= 0x10FFFF {
                        Ok(Node::Literal(val))
                    } else if self.flags.is_unicode() {
                        Err(JsError::syntax("Invalid Unicode escape in regex"))
                    } else {
                        self.pos = saved;
                        Ok(Node::Literal('u' as u32))
                    }
                } else if let Some(val) = self.try_read_hex(4) {
                    Ok(Node::Literal(val))
                } else if self.flags.is_unicode() {
                    Err(JsError::syntax("Invalid Unicode escape in regex"))
                } else {
                    Ok(Node::Literal('u' as u32))
                }
            }
            'c' => {
                if let Some(ch) = self.peek() {
                    if ch.is_ascii_alphabetic() {
                        self.pos += 1;
                        return Ok(Node::Literal((ch as u32) % 32));
                    }
                }
                if self.flags.is_unicode() {
                    return Err(JsError::syntax("Invalid control escape in Unicode regex"));
                }
                Ok(Node::Literal('c' as u32))
            }
            'p' | 'P' if self.peek() == Some('{') => {
                let negated = c == 'P';
                self.pos += 1;
                let mut prop = String::new();
                while let Some(ch) = self.next() {
                    if ch == '}' {
                        break;
                    }
                    prop.push(ch);
                }
                Ok(Node::UnicodeProp(prop, negated))
            }
            other => {
                if self.flags.is_unicode()
                    && !matches!(
                        other,
                        '^' | '$'
                            | '\\'
                            | '.'
                            | '*'
                            | '+'
                            | '?'
                            | '('
                            | ')'
                            | '['
                            | ']'
                            | '{'
                            | '}'
                            | '|'
                            | '/'
                    )
                    && !(in_class && other == '-')
                    && !(self.flags.unicode_sets
                        && in_class
                        && matches!(
                            other,
                            '&' | '!' | '#' | '%' | ',' | ':' | ';' | '<' | '=' | '>' | '@' | '`' | '~'
                        ))
                {
                    return Err(JsError::syntax(format!(
                        "Invalid escape '\\{}' in Unicode regex",
                        other
                    )));
                }
                Ok(Node::Literal(other as u32))
            }
        }
    }

    fn try_read_hex(&mut self, count: usize) -> Option<u32> {
        if self.pos + count > self.chars.len() {
            return None;
        }
        let mut val = 0u32;
        for i in 0..count {
            let d = self.chars[self.pos + i].to_digit(16)?;
            val = (val << 4) | d;
        }
        self.pos += count;
        Some(val)
    }

    fn parse_char_class(&mut self) -> Result<Node, JsError> {
        if self.flags.unicode_sets {
            let expr = self.parse_unicode_set_class()?;
            return Ok(Node::ClassSet(expr));
        }
        let negated = if self.peek() == Some('^') {
            self.pos += 1;
            true
        } else {
            false
        };
        let mut atoms = Vec::new();
        while let Some(c) = self.peek() {
            if c == ']' {
                self.pos += 1;
                return Ok(Node::CharClass { negated, atoms });
            }
            let first = self.parse_class_atom()?;
            if self.peek() == Some('-') && self.chars.get(self.pos + 1) != Some(&']') {
                if let ClassAtom::Char(start) = first {
                    self.pos += 1;
                    let second = self.parse_class_atom()?;
                    if let ClassAtom::Char(end) = second {
                        if start > end {
                            return Err(JsError::syntax("Range out of order in character class"));
                        }
                        atoms.push(ClassAtom::Range(start, end));
                    } else if self.flags.is_unicode() {
                        return Err(JsError::syntax("Invalid character class range in Unicode regex"));
                    } else {
                        atoms.push(ClassAtom::Char(start));
                        atoms.push(ClassAtom::Char('-' as u32));
                        atoms.push(second);
                    }
                    continue;
                } else if self.flags.is_unicode() {
                    return Err(JsError::syntax("Invalid character class range in Unicode regex"));
                }
            }
            atoms.push(first);
        }
        Err(JsError::syntax("Unterminated character class in regex"))
    }

    fn parse_unicode_set_class(&mut self) -> Result<ClassSetExpr, JsError> {
        let negated = if self.peek() == Some('^') {
            self.pos += 1;
            true
        } else {
            false
        };
        if self.peek() == Some(']') {
            self.pos += 1;
            let empty = ClassSetExpr::Union(Vec::new());
            return Ok(if negated {
                ClassSetExpr::Negated(Box::new(empty))
            } else {
                empty
            });
        }

        let first = self.parse_v_class_element(true)?;
        let inner = if self.peek() == Some('-') && self.chars.get(self.pos + 1) == Some(&'-') {
            let mut items = vec![first];
            while self.peek() == Some('-') && self.chars.get(self.pos + 1) == Some(&'-') {
                self.pos += 2;
                items.push(self.parse_v_class_element(false)?);
            }
            if self.peek() != Some(']') {
                return Err(JsError::syntax(
                    "Invalid character in unicodeSets difference expression",
                ));
            }
            self.pos += 1;
            ClassSetExpr::Difference(items)
        } else if self.peek() == Some('&') && self.chars.get(self.pos + 1) == Some(&'&') {
            let mut items = vec![first];
            while self.peek() == Some('&') && self.chars.get(self.pos + 1) == Some(&'&') {
                self.pos += 2;
                items.push(self.parse_v_class_element(false)?);
            }
            if self.peek() != Some(']') {
                return Err(JsError::syntax(
                    "Invalid character in unicodeSets intersection expression",
                ));
            }
            self.pos += 1;
            ClassSetExpr::Intersection(items)
        } else {
            let mut items = vec![first];
            while let Some(c) = self.peek() {
                if c == ']' {
                    self.pos += 1;
                    break;
                }
                if (c == '-' && self.chars.get(self.pos + 1) == Some(&'-'))
                    || (c == '&' && self.chars.get(self.pos + 1) == Some(&'&'))
                {
                    return Err(JsError::syntax(
                        "Cannot mix union and set operators in unicodeSets character class",
                    ));
                }
                items.push(self.parse_v_class_element(true)?);
            }
            if self.pos == 0 || self.chars.get(self.pos - 1) != Some(&']') {
                return Err(JsError::syntax("Unterminated character class in regex"));
            }
            if items.len() == 1 {
                items.into_iter().next().unwrap()
            } else {
                ClassSetExpr::Union(items)
            }
        };

        if negated {
            if class_set_has_strings(&inner) {
                return Err(JsError::syntax(
                    "Cannot negate a unicodeSets character class that may contain strings",
                ));
            }
            Ok(ClassSetExpr::Negated(Box::new(inner)))
        } else {
            Ok(inner)
        }
    }

    fn parse_v_class_element(&mut self, allow_range: bool) -> Result<ClassSetExpr, JsError> {
        let Some(c) = self.peek() else {
            return Err(JsError::syntax("Unterminated character class in regex"));
        };
        if c == '[' {
            self.pos += 1;
            return self.parse_unicode_set_class();
        }
        if c == '\\' && self.chars.get(self.pos + 1) == Some(&'q') {
            self.pos += 2;
            if self.next() != Some('{') {
                return Err(JsError::syntax("Expected '{' after \\q in unicodeSets"));
            }
            let mut alts = Vec::new();
            let mut cur = Vec::new();
            loop {
                match self.peek() {
                    None => return Err(JsError::syntax("Unterminated \\q{...} in unicodeSets")),
                    Some('}') => {
                        self.pos += 1;
                        alts.push(ClassSetExpr::StringLit(cur));
                        break;
                    }
                    Some('|') => {
                        self.pos += 1;
                        alts.push(ClassSetExpr::StringLit(cur));
                        cur = Vec::new();
                    }
                    Some('\\') => {
                        self.pos += 1;
                        match self.parse_escape(true)? {
                            Node::Literal(ch) => cur.push(ch),
                            _ => {
                                return Err(JsError::syntax(
                                    "Invalid escape inside \\q{...} in unicodeSets",
                                ));
                            }
                        }
                    }
                    Some(ch) => {
                        if matches!(ch, '(' | ')' | '[' | ']' | '{' | '/' | '-') {
                            return Err(JsError::syntax(
                                "Invalid character inside \\q{...} in unicodeSets",
                            ));
                        }
                        self.pos += 1;
                        cur.push(ch as u32);
                    }
                }
            }
            return Ok(if alts.len() == 1 {
                alts.into_iter().next().unwrap()
            } else {
                ClassSetExpr::Union(alts)
            });
        }
        if c != '\\' {
            if matches!(c, '(' | ')' | '{' | '}' | '/' | '-' | '|' | ']') {
                return Err(JsError::syntax(
                    "Invalid character in unicodeSets character class",
                ));
            }
            if matches!(
                c,
                '&' | '!'
                    | '#'
                    | '$'
                    | '%'
                    | '*'
                    | '+'
                    | ','
                    | '.'
                    | ':'
                    | ';'
                    | '<'
                    | '='
                    | '>'
                    | '?'
                    | '@'
                    | '^'
                    | '`'
                    | '~'
            ) && self.chars.get(self.pos + 1) == Some(&c)
            {
                return Err(JsError::syntax(
                    "Invalid double punctuator in unicodeSets character class",
                ));
            }
        }
        let atom = self.parse_class_atom()?;
        if let ClassAtom::UnicodeProp(ref prop, neg) = atom {
            if is_string_unicode_prop(prop) {
                if neg {
                    return Err(JsError::syntax(
                        "Cannot negate property of strings with \\P in unicodeSets",
                    ));
                }
                return Ok(ClassSetExpr::StringProp(prop.clone()));
            }
        }
        if allow_range
            && self.peek() == Some('-')
            && self.chars.get(self.pos + 1) != Some(&'-')
            && self.chars.get(self.pos + 1) != Some(&']')
        {
            if let ClassAtom::Char(start) = atom {
                self.pos += 1;
                let second = self.parse_v_class_char()?;
                if start > second {
                    return Err(JsError::syntax("Range out of order in character class"));
                }
                return Ok(ClassSetExpr::Atom(ClassAtom::Range(start, second)));
            }
            return Err(JsError::syntax(
                "Invalid character class range in unicodeSets regex",
            ));
        }
        Ok(ClassSetExpr::Atom(atom))
    }

    fn parse_v_class_char(&mut self) -> Result<u32, JsError> {
        let Some(c) = self.peek() else {
            return Err(JsError::syntax("Unterminated character class"));
        };
        if c != '\\' && matches!(c, '(' | ')' | '[' | ']' | '{' | '}' | '/' | '-' | '|') {
            return Err(JsError::syntax(
                "Invalid character in unicodeSets character class range",
            ));
        }
        match self.parse_class_atom()? {
            ClassAtom::Char(ch) => Ok(ch),
            _ => Err(JsError::syntax(
                "Invalid character class range in unicodeSets regex",
            )),
        }
    }

    fn parse_class_atom(&mut self) -> Result<ClassAtom, JsError> {
        let Some(c) = self.next() else {
            return Err(JsError::syntax("Unterminated character class"));
        };
        if c == '\\' {
            match self.parse_escape(true)? {
                Node::Literal(ch) => Ok(ClassAtom::Char(ch)),
                Node::Digit(neg) => Ok(ClassAtom::Digit(neg)),
                Node::Word(neg) => Ok(ClassAtom::Word(neg)),
                Node::Space(neg) => Ok(ClassAtom::Space(neg)),
                Node::UnicodeProp(p, neg) => Ok(ClassAtom::UnicodeProp(p, neg)),
                _ => Ok(ClassAtom::Char('\\' as u32)),
            }
        } else {
            Ok(ClassAtom::Char(c as u32))
        }
    }

    fn parse_quantifier(
        &mut self,
        atom: Node,
        cap_start: usize,
        cap_end: usize,
    ) -> Result<Node, JsError> {
        let (min, max) = match self.peek() {
            Some('*') => {
                self.pos += 1;
                (0, None)
            }
            Some('+') => {
                self.pos += 1;
                (1, None)
            }
            Some('?') => {
                self.pos += 1;
                (0, Some(1))
            }
            Some('{') => {
                if let Some(bounds) = self.try_parse_brace_quantifier()? {
                    bounds
                } else {
                    return Ok(atom);
                }
            }
            _ => return Ok(atom),
        };
        let greedy = if self.peek() == Some('?') {
            self.pos += 1;
            false
        } else {
            true
        };
        if matches!(
            atom,
            Node::Lookaround { behind: true, .. }
                | Node::AnchorStart
                | Node::AnchorEnd
                | Node::WordBoundary(_)
        ) || (self.flags.is_unicode() && matches!(atom, Node::Lookaround { .. }))
        {
            return Err(JsError::syntax("Nothing to repeat in regular expression"));
        }
        if matches!(self.peek(), Some('*' | '+' | '?')) {
            return Err(JsError::syntax("Nothing to repeat in regular expression"));
        }
        if self.peek() == Some('{') {
            let saved = self.pos;
            let stacked = self.try_parse_brace_quantifier()?.is_some();
            self.pos = saved;
            if stacked {
                return Err(JsError::syntax("Nothing to repeat in regular expression"));
            }
        }
        Ok(Node::Quantifier {
            body: Box::new(atom),
            min,
            max,
            greedy,
            cap_start,
            cap_end,
        })
    }

    fn try_parse_brace_quantifier(&mut self) -> Result<Option<(usize, Option<usize>)>, JsError> {
        let saved = self.pos;
        self.pos += 1;
        let Some(min) = self.read_uint() else {
            self.pos = saved;
            return Ok(None);
        };
        match self.peek() {
            Some('}') => {
                self.pos += 1;
                Ok(Some((min, Some(min))))
            }
            Some(',') => {
                self.pos += 1;
                if self.peek() == Some('}') {
                    self.pos += 1;
                    Ok(Some((min, None)))
                } else {
                    let Some(max) = self.read_uint() else {
                        self.pos = saved;
                        return Ok(None);
                    };
                    if self.peek() == Some('}') {
                        self.pos += 1;
                        if max < min {
                            return Err(JsError::syntax("Numbers out of order in {} quantifier"));
                        }
                        Ok(Some((min, Some(max))))
                    } else {
                        self.pos = saved;
                        Ok(None)
                    }
                }
            }
            _ => {
                self.pos = saved;
                Ok(None)
            }
        }
    }

    fn read_uint(&mut self) -> Option<usize> {
        let mut val = 0usize;
        let mut count = 0usize;
        while let Some(c) = self.peek() {
            if c.is_ascii_digit() {
                self.pos += 1;
                val = val.saturating_mul(10).saturating_add(c as usize - '0' as usize);
                count += 1;
            } else {
                break;
            }
        }
        if count > 0 { Some(val) } else { None }
    }
}

fn validate_regexp_refs(
    node: &Node,
    capture_count: usize,
    named_groups: &[(String, usize)],
    unicode: bool,
) -> Result<(), JsError> {
    match node {
        Node::Seq(items) | Node::Alt(items) => {
            for it in items {
                validate_regexp_refs(it, capture_count, named_groups, unicode)?;
            }
        }
        Node::Group { body, .. }
        | Node::ModifiedGroup { body, .. }
        | Node::Quantifier { body, .. }
        | Node::Lookaround { body, .. } => {
            validate_regexp_refs(body, capture_count, named_groups, unicode)?;
        }
        Node::Backref(n) => {
            if unicode && *n > capture_count {
                return Err(JsError::syntax("Invalid decimal escape in Unicode regex"));
            }
        }
        Node::NamedBackref(name) => {
            if !named_groups.iter().any(|(gn, _)| gn == name) {
                return Err(JsError::syntax(format!(
                    "Invalid named backreference '{}'",
                    name
                )));
            }
        }
        _ => {}
    }
    Ok(())
}

struct MatchState<'a> {
    units: &'a [u16],
    flags: RegExpFlags,
    captures: Vec<Option<(usize, usize)>>,
    named_groups: &'a [(String, usize)],
    steps: usize,
}

const MAX_REGEX_STEPS: usize = 2_000_000;

impl<'a> MatchState<'a> {
    fn match_node(&mut self, node: &Node, pos: usize, rev: bool) -> Option<usize> {
        self.steps += 1;
        if self.steps > MAX_REGEX_STEPS {
            return None;
        }
        self.match_with_cont(node, pos, rev, &mut |_, end| Some(end))
    }

    fn match_with_cont(
        &mut self,
        node: &Node,
        pos: usize,
        rev: bool,
        cont: &mut dyn FnMut(&mut Self, usize) -> Option<usize>,
    ) -> Option<usize> {
        self.steps += 1;
        if self.steps > MAX_REGEX_STEPS {
            return None;
        }
        match node {
            Node::Seq(items) => {
                if rev {
                    let reversed: Vec<&Node> = items.iter().rev().collect();
                    self.match_seq_slice(&reversed, pos, true, cont)
                } else {
                    let refs: Vec<&Node> = items.iter().collect();
                    self.match_seq_slice(&refs, pos, false, cont)
                }
            }
            Node::Alt(branches) => {
                for b in branches {
                    let saved = self.captures.clone();
                    if let Some(res) = self.match_with_cont(b, pos, rev, cont) {
                        return Some(res);
                    }
                    self.captures = saved;
                }
                None
            }
            Node::Literal(ch) => {
                let (next_pos, unit) = self.read_unit(pos, rev)?;
                if chars_equal(unit, *ch, self.flags.ignore_case) {
                    cont(self, next_pos)
                } else {
                    None
                }
            }
            Node::Dot => {
                let (next_pos, unit) = self.read_unit(pos, rev)?;
                if self.flags.dot_all || !is_line_term_u32(unit) {
                    cont(self, next_pos)
                } else {
                    None
                }
            }
            Node::Digit(neg) => {
                let (next_pos, unit) = self.read_unit(pos, rev)?;
                let m = (unit >= '0' as u32 && unit <= '9' as u32) ^ *neg;
                if m { cont(self, next_pos) } else { None }
            }
            Node::Word(neg) => {
                let (next_pos, unit) = self.read_unit(pos, rev)?;
                let m = is_word_char(unit) ^ *neg;
                if m { cont(self, next_pos) } else { None }
            }
            Node::Space(neg) => {
                let (next_pos, unit) = self.read_unit(pos, rev)?;
                let m = is_space_u32(unit) ^ *neg;
                if m { cont(self, next_pos) } else { None }
            }
            Node::UnicodeProp(prop, neg) => {
                if is_string_unicode_prop(prop) && !*neg {
                    let expr = ClassSetExpr::StringProp(prop.clone());
                    return self.match_class_set(&expr, pos, rev, cont);
                }
                let (next_pos, unit) = self.read_unit(pos, rev)?;
                let m = match_unicode_prop(unit, prop) ^ *neg;
                if m { cont(self, next_pos) } else { None }
            }
            Node::ClassSet(expr) => self.match_class_set(expr, pos, rev, cont),
            Node::CharClass { negated, atoms } => {
                let (next_pos, unit) = self.read_unit(pos, rev)?;
                let mut matched = false;
                for atom in atoms {
                    if match_class_atom(unit, atom, self.flags.ignore_case) {
                        matched = true;
                        break;
                    }
                }
                if matched ^ *negated {
                    cont(self, next_pos)
                } else {
                    None
                }
            }
            Node::AnchorStart => {
                let ok = pos == 0
                    || (self.flags.multiline
                        && pos > 0
                        && is_line_term_u32(self.units[pos - 1] as u32));
                if ok { cont(self, pos) } else { None }
            }
            Node::AnchorEnd => {
                let ok = pos == self.units.len()
                    || (self.flags.multiline
                        && pos < self.units.len()
                        && is_line_term_u32(self.units[pos] as u32));
                if ok { cont(self, pos) } else { None }
            }
            Node::WordBoundary(neg) => {
                let before = pos > 0 && is_word_char(self.units[pos - 1] as u32);
                let after = pos < self.units.len() && is_word_char(self.units[pos] as u32);
                if (before != after) ^ *neg {
                    cont(self, pos)
                } else {
                    None
                }
            }
            Node::Group { index, body } => {
                if let Some(idx) = *index {
                    let saved = self.captures[idx];
                    let start = pos;
                    let res = self.match_with_cont(body, pos, rev, &mut |st, end| {
                        let (a, b) = if rev { (end, start) } else { (start, end) };
                        st.captures[idx] = Some((a, b));
                        cont(st, end)
                    });
                    if res.is_none() {
                        self.captures[idx] = saved;
                    }
                    res
                } else {
                    self.match_with_cont(body, pos, rev, cont)
                }
            }
            Node::ModifiedGroup {
                add_i,
                add_m,
                add_s,
                rem_i,
                rem_m,
                rem_s,
                body,
            } => {
                let saved_flags = self.flags;
                if *add_i {
                    self.flags.ignore_case = true;
                }
                if *rem_i {
                    self.flags.ignore_case = false;
                }
                if *add_m {
                    self.flags.multiline = true;
                }
                if *rem_m {
                    self.flags.multiline = false;
                }
                if *add_s {
                    self.flags.dot_all = true;
                }
                if *rem_s {
                    self.flags.dot_all = false;
                }
                let res = self.match_with_cont(body, pos, rev, &mut |st, end| {
                    let inner_flags = st.flags;
                    st.flags = saved_flags;
                    let r = cont(st, end);
                    st.flags = inner_flags;
                    r
                });
                self.flags = saved_flags;
                res
            }
            Node::Backref(idx) => {
                let cap = self.captures.get(*idx).copied().flatten();
                self.match_backref(cap, pos, rev, cont)
            }
            Node::NamedBackref(name) => {
                let idx = self
                    .named_groups
                    .iter()
                    .find(|(n, _)| n == name)
                    .map(|(_, i)| *i);
                let cap = idx.and_then(|i| self.captures.get(i).copied().flatten());
                self.match_backref(cap, pos, rev, cont)
            }
            Node::Quantifier {
                body,
                min,
                max,
                greedy,
                cap_start,
                cap_end,
            } => self.match_quant(body, *min, *max, *greedy, *cap_start, *cap_end, 0, pos, rev, cont),
            Node::Lookaround {
                body,
                positive,
                behind,
            } => {
                let saved = self.captures.clone();
                let inner = self.match_node(body, pos, *behind);
                let ok = inner.is_some() == *positive;
                if !*positive || !ok {
                    self.captures = saved;
                }
                if ok {
                    cont(self, pos)
                } else {
                    None
                }
            }
        }
    }

    fn match_seq_slice(
        &mut self,
        nodes: &[&Node],
        pos: usize,
        rev: bool,
        cont: &mut dyn FnMut(&mut Self, usize) -> Option<usize>,
    ) -> Option<usize> {
        let Some((first, rest)) = nodes.split_first() else {
            return cont(self, pos);
        };
        self.match_with_cont(first, pos, rev, &mut |st, next_pos| {
            st.match_seq_slice(rest, next_pos, rev, cont)
        })
    }

    fn match_backref(
        &mut self,
        cap: Option<(usize, usize)>,
        pos: usize,
        rev: bool,
        cont: &mut dyn FnMut(&mut Self, usize) -> Option<usize>,
    ) -> Option<usize> {
        let Some((cs, ce)) = cap else {
            return cont(self, pos);
        };
        let len = ce - cs;
        if rev {
            if pos < len {
                return None;
            }
            let start = pos - len;
            for i in 0..len {
                if !chars_equal(
                    self.units[start + i] as u32,
                    self.units[cs + i] as u32,
                    self.flags.ignore_case,
                ) {
                    return None;
                }
            }
            cont(self, start)
        } else {
            if pos + len > self.units.len() {
                return None;
            }
            for i in 0..len {
                if !chars_equal(
                    self.units[pos + i] as u32,
                    self.units[cs + i] as u32,
                    self.flags.ignore_case,
                ) {
                    return None;
                }
            }
            cont(self, pos + len)
        }
    }

    fn is_simple_atom(node: &Node) -> bool {
        matches!(
            node,
            Node::Literal(_)
                | Node::Dot
                | Node::Digit(_)
                | Node::Word(_)
                | Node::Space(_)
                | Node::UnicodeProp(_, _)
                | Node::CharClass { .. }
        )
    }

    fn match_simple_atom(&self, node: &Node, pos: usize, rev: bool) -> Option<usize> {
        let (next_pos, unit) = self.read_unit(pos, rev)?;
        let ok = match node {
            Node::Literal(ch) => chars_equal(unit, *ch, self.flags.ignore_case),
            Node::Dot => self.flags.dot_all || !is_line_term_u32(unit),
            Node::Digit(neg) => (unit >= '0' as u32 && unit <= '9' as u32) ^ *neg,
            Node::Word(neg) => is_word_char(unit) ^ *neg,
            Node::Space(neg) => is_space_u32(unit) ^ *neg,
            Node::UnicodeProp(prop, neg) => match_unicode_prop(unit, prop) ^ *neg,
            Node::CharClass { negated, atoms } => {
                let mut matched = false;
                for atom in atoms {
                    if match_class_atom(unit, atom, self.flags.ignore_case) {
                        matched = true;
                        break;
                    }
                }
                matched ^ *negated
            }
            _ => false,
        };
        if ok { Some(next_pos) } else { None }
    }

    #[allow(clippy::too_many_arguments)]
    fn match_quant(
        &mut self,
        body: &Node,
        min: usize,
        max: Option<usize>,
        greedy: bool,
        cap_start: usize,
        cap_end: usize,
        count: usize,
        pos: usize,
        rev: bool,
        cont: &mut dyn FnMut(&mut Self, usize) -> Option<usize>,
    ) -> Option<usize> {
        if count == 0 && Self::is_simple_atom(body) {
            let mut positions = Vec::new();
            positions.push(pos);
            let mut cur = pos;
            let limit = max.unwrap_or(usize::MAX);
            while positions.len() - 1 < limit {
                if let Some(next_pos) = self.match_simple_atom(body, cur, rev) {
                    if next_pos == cur {
                        break;
                    }
                    positions.push(next_pos);
                    cur = next_pos;
                } else {
                    break;
                }
            }
            let max_matched = positions.len() - 1;
            if max_matched < min {
                return None;
            }
            if greedy {
                for k in (min..=max_matched).rev() {
                    let saved = self.captures.clone();
                    if let Some(res) = cont(self, positions[k]) {
                        return Some(res);
                    }
                    self.captures = saved;
                }
            } else {
                for k in min..=max_matched {
                    let saved = self.captures.clone();
                    if let Some(res) = cont(self, positions[k]) {
                        return Some(res);
                    }
                    self.captures = saved;
                }
            }
            return None;
        }
        if count > 4096 {
            return None;
        }
        self.steps += 1;
        if self.steps > MAX_REGEX_STEPS {
            return None;
        }
        if let Some(m) = max {
            if count == m {
                return cont(self, pos);
            }
        }
        if !greedy && count >= min {
            let saved = self.captures.clone();
            if let Some(res) = cont(self, pos) {
                return Some(res);
            }
            self.captures = saved;
        }

        let saved = self.captures.clone();
        if count >= min {
            for i in cap_start..=cap_end {
                if i < self.captures.len() {
                    self.captures[i] = None;
                }
            }
        }
        let body_res = self.match_with_cont(body, pos, rev, &mut |st, next_pos| {
            if count >= min && next_pos == pos {
                return None;
            }
            st.match_quant(
                body,
                min,
                max,
                greedy,
                cap_start,
                cap_end,
                count + 1,
                next_pos,
                rev,
                cont,
            )
        });
        if body_res.is_some() {
            return body_res;
        }
        self.captures = saved;

        if greedy && count >= min {
            cont(self, pos)
        } else {
            None
        }
    }

    fn match_class_set(
        &mut self,
        expr: &ClassSetExpr,
        pos: usize,
        rev: bool,
        cont: &mut dyn FnMut(&mut Self, usize) -> Option<usize>,
    ) -> Option<usize> {
        let ic = self.flags.ignore_case;
        let mut strings = collect_class_set_strings(expr, ic);
        strings.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
        strings.dedup();

        let mut has_empty = false;
        for s in &strings {
            if s.is_empty() {
                has_empty = true;
                continue;
            }
            if s.len() < 2 {
                continue;
            }
            if let Some(next_pos) = self.match_cp_seq(s, pos, rev, ic) {
                let saved = self.captures.clone();
                if let Some(res) = cont(self, next_pos) {
                    return Some(res);
                }
                self.captures = saved;
            }
        }
        if let Some((next_pos, unit)) = self.read_unit(pos, rev) {
            if match_class_set_char(unit, expr, ic) {
                let saved = self.captures.clone();
                if let Some(res) = cont(self, next_pos) {
                    return Some(res);
                }
                self.captures = saved;
            }
        }
        if has_empty {
            return cont(self, pos);
        }
        None
    }

    fn match_cp_seq(&self, seq: &[u32], pos: usize, rev: bool, ignore_case: bool) -> Option<usize> {
        let mut cur = pos;
        if rev {
            for &expected in seq.iter().rev() {
                let (next_pos, unit) = self.read_unit(cur, true)?;
                if !chars_equal(unit, expected, ignore_case) {
                    return None;
                }
                cur = next_pos;
            }
        } else {
            for &expected in seq {
                let (next_pos, unit) = self.read_unit(cur, false)?;
                if !chars_equal(unit, expected, ignore_case) {
                    return None;
                }
                cur = next_pos;
            }
        }
        Some(cur)
    }

    fn read_unit(&self, pos: usize, rev: bool) -> Option<(usize, u32)> {
        if rev {
            if pos == 0 {
                return None;
            }
            let u = self.units[pos - 1];
            if self.flags.is_unicode() && (0xDC00..=0xDFFF).contains(&u) && pos >= 2 {
                let hi = self.units[pos - 2];
                if (0xD800..=0xDBFF).contains(&hi) {
                    let cp = 0x10000 + (((hi as u32) - 0xD800) << 10) + ((u as u32) - 0xDC00);
                    return Some((pos - 2, cp));
                }
            }
            Some((pos - 1, u as u32))
        } else {
            if pos >= self.units.len() {
                return None;
            }
            let u = self.units[pos];
            if self.flags.is_unicode() && (0xD800..=0xDBFF).contains(&u) && pos + 1 < self.units.len() {
                let lo = self.units[pos + 1];
                if (0xDC00..=0xDFFF).contains(&lo) {
                    let cp = 0x10000 + (((u as u32) - 0xD800) << 10) + ((lo as u32) - 0xDC00);
                    return Some((pos + 2, cp));
                }
            }
            Some((pos + 1, u as u32))
        }
    }
}

fn chars_equal(a: u32, b: u32, ignore_case: bool) -> bool {
    if a == b {
        return true;
    }
    if !ignore_case {
        return false;
    }
    if a < 128 && b < 128 {
        return (a as u8).eq_ignore_ascii_case(&(b as u8));
    }
    match (char::from_u32(a), char::from_u32(b)) {
        (Some(ca), Some(cb)) => ca.to_lowercase().eq(cb.to_lowercase()),
        _ => false,
    }
}

fn match_class_atom(unit: u32, atom: &ClassAtom, ignore_case: bool) -> bool {
    match atom {
        ClassAtom::Char(c) => chars_equal(unit, *c, ignore_case),
        ClassAtom::Range(lo, hi) => {
            if unit >= *lo && unit <= *hi {
                true
            } else if ignore_case {
                if unit < 128 {
                    let lower = (unit as u8).to_ascii_lowercase() as u32;
                    let upper = (unit as u8).to_ascii_uppercase() as u32;
                    (lower >= *lo && lower <= *hi) || (upper >= *lo && upper <= *hi)
                } else if let Some(ch) = char::from_u32(unit) {
                    ch.to_lowercase()
                        .next()
                        .is_some_and(|c| (c as u32) >= *lo && (c as u32) <= *hi)
                        || ch
                            .to_uppercase()
                            .next()
                            .is_some_and(|c| (c as u32) >= *lo && (c as u32) <= *hi)
                } else {
                    false
                }
            } else {
                false
            }
        }
        ClassAtom::Digit(neg) => (unit >= '0' as u32 && unit <= '9' as u32) ^ *neg,
        ClassAtom::Word(neg) => is_word_char(unit) ^ *neg,
        ClassAtom::Space(neg) => is_space_u32(unit) ^ *neg,
        ClassAtom::UnicodeProp(prop, neg) => match_unicode_prop(unit, prop) ^ *neg,
    }
}

fn is_line_term_u32(u: u32) -> bool {
    matches!(u, 0x0A | 0x0D | 0x2028 | 0x2029)
}

fn is_word_char(u: u32) -> bool {
    (u >= 'a' as u32 && u <= 'z' as u32)
        || (u >= 'A' as u32 && u <= 'Z' as u32)
        || (u >= '0' as u32 && u <= '9' as u32)
        || u == '_' as u32
}

fn is_space_u32(u: u32) -> bool {
    matches!(
        u,
        0x09 | 0x0A
            | 0x0B
            | 0x0C
            | 0x0D
            | 0x20
            | 0xA0
            | 0x1680
            | 0x2000..=0x200A
            | 0x2028
            | 0x2029
            | 0x202F
            | 0x205F
            | 0x3000
            | 0xFEFF
    )
}

fn match_unicode_prop(unit: u32, prop: &str) -> bool {
    let Some(ch) = char::from_u32(unit) else {
        return false;
    };
    let val = prop.split('=').next_back().unwrap_or(prop);
    match val {
        "L" | "Letter" | "Alphabetic" => ch.is_alphabetic(),
        "Lu" | "Uppercase_Letter" | "Uppercase" => ch.is_uppercase(),
        "Ll" | "Lowercase_Letter" | "Lowercase" => ch.is_lowercase(),
        "N" | "Number" | "Nd" | "Decimal_Number" => ch.is_numeric(),
        "P" | "Punctuation" => ch.is_ascii_punctuation() || (!ch.is_alphanumeric() && !ch.is_whitespace() && !ch.is_control()),
        "S" | "Symbol" => matches!(ch, '$' | '+' | '<' | '=' | '>' | '^' | '`' | '|' | '~'),
        "Z" | "Separator" | "White_Space" => ch.is_whitespace(),
        "ASCII" => ch.is_ascii(),
        "ASCII_Hex_Digit" | "Hex_Digit" => ch.is_ascii_hexdigit(),
        _ => ch.is_alphabetic(),
    }
}

fn is_string_unicode_prop(prop: &str) -> bool {
    matches!(
        prop,
        "Emoji_Keycap_Sequence"
            | "Basic_Emoji"
            | "RGI_Emoji"
            | "RGI_Emoji_Flag_Sequence"
            | "RGI_Emoji_Modifier_Sequence"
            | "RGI_Emoji_Tag_Sequence"
            | "RGI_Emoji_ZWJ_Sequence"
    )
}

fn string_prop_entries(prop: &str) -> Vec<Vec<u32>> {
    match prop {
        "Emoji_Keycap_Sequence" => {
            let bases: [u32; 12] = [
                '#' as u32, '*' as u32, '0' as u32, '1' as u32, '2' as u32, '3' as u32,
                '4' as u32, '5' as u32, '6' as u32, '7' as u32, '8' as u32, '9' as u32,
            ];
            bases.iter().map(|&b| vec![b, 0xFE0F, 0x20E3]).collect()
        }
        _ => Vec::new(),
    }
}

fn class_set_has_strings(expr: &ClassSetExpr) -> bool {
    match expr {
        ClassSetExpr::Atom(_) | ClassSetExpr::Negated(_) => false,
        ClassSetExpr::StringLit(cps) => cps.len() != 1,
        ClassSetExpr::StringProp(_) => true,
        ClassSetExpr::Union(es) => es.iter().any(class_set_has_strings),
        ClassSetExpr::Intersection(es) => es.iter().all(class_set_has_strings),
        ClassSetExpr::Difference(es) => es.first().is_some_and(class_set_has_strings),
    }
}

fn match_class_set_char(unit: u32, expr: &ClassSetExpr, ignore_case: bool) -> bool {
    match expr {
        ClassSetExpr::Atom(a) => match_class_atom(unit, a, ignore_case),
        ClassSetExpr::StringLit(cps) => cps.len() == 1 && chars_equal(unit, cps[0], ignore_case),
        ClassSetExpr::StringProp(_) => false,
        ClassSetExpr::Union(es) => es
            .iter()
            .any(|e| match_class_set_char(unit, e, ignore_case)),
        ClassSetExpr::Intersection(es) => !es.is_empty()
            && es
                .iter()
                .all(|e| match_class_set_char(unit, e, ignore_case)),
        ClassSetExpr::Difference(es) => {
            if let Some((first, rest)) = es.split_first() {
                match_class_set_char(unit, first, ignore_case)
                    && !rest
                        .iter()
                        .any(|e| match_class_set_char(unit, e, ignore_case))
            } else {
                false
            }
        }
        ClassSetExpr::Negated(inner) => !match_class_set_char(unit, inner, ignore_case),
    }
}

fn match_class_set_str(s: &[u32], expr: &ClassSetExpr, ignore_case: bool) -> bool {
    if s.len() == 1 {
        return match_class_set_char(s[0], expr, ignore_case);
    }
    match expr {
        ClassSetExpr::Atom(_) | ClassSetExpr::Negated(_) => false,
        ClassSetExpr::StringLit(cps) => {
            cps.len() == s.len()
                && cps
                    .iter()
                    .zip(s.iter())
                    .all(|(&a, &b)| chars_equal(a, b, ignore_case))
        }
        ClassSetExpr::StringProp(prop) => string_prop_entries(prop).iter().any(|entry| {
            entry.len() == s.len()
                && entry
                    .iter()
                    .zip(s.iter())
                    .all(|(&a, &b)| chars_equal(a, b, ignore_case))
        }),
        ClassSetExpr::Union(es) => es.iter().any(|e| match_class_set_str(s, e, ignore_case)),
        ClassSetExpr::Intersection(es) => {
            !es.is_empty() && es.iter().all(|e| match_class_set_str(s, e, ignore_case))
        }
        ClassSetExpr::Difference(es) => {
            if let Some((first, rest)) = es.split_first() {
                match_class_set_str(s, first, ignore_case)
                    && !rest.iter().any(|e| match_class_set_str(s, e, ignore_case))
            } else {
                false
            }
        }
    }
}

fn collect_class_set_strings(expr: &ClassSetExpr, ignore_case: bool) -> Vec<Vec<u32>> {
    match expr {
        ClassSetExpr::Atom(_) | ClassSetExpr::Negated(_) => Vec::new(),
        ClassSetExpr::StringLit(cps) => {
            if cps.len() != 1 {
                vec![cps.clone()]
            } else {
                Vec::new()
            }
        }
        ClassSetExpr::StringProp(prop) => string_prop_entries(prop),
        ClassSetExpr::Union(es) => {
            let mut out = Vec::new();
            for e in es {
                out.extend(collect_class_set_strings(e, ignore_case));
            }
            out
        }
        ClassSetExpr::Intersection(es) => {
            let Some((first, rest)) = es.split_first() else {
                return Vec::new();
            };
            collect_class_set_strings(first, ignore_case)
                .into_iter()
                .filter(|s| rest.iter().all(|r| match_class_set_str(s, r, ignore_case)))
                .collect()
        }
        ClassSetExpr::Difference(es) => {
            let Some((first, rest)) = es.split_first() else {
                return Vec::new();
            };
            collect_class_set_strings(first, ignore_case)
                .into_iter()
                .filter(|s| !rest.iter().any(|r| match_class_set_str(s, r, ignore_case)))
                .collect()
        }
    }
}
