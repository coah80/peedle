use crate::error::JsError;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RegExpFlags {
    pub global: bool,
    pub ignore_case: bool,
    pub multiline: bool,
    pub dot_all: bool,
    pub unicode: bool,
    pub sticky: bool,
    pub has_indices: bool,
}

impl RegExpFlags {
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
                    if flags.unicode {
                        return Err(JsError::syntax("Duplicate regex flag 'u'"));
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
                    if flags.unicode {
                        return Err(JsError::syntax("Duplicate regex flag 'v'"));
                    }
                    flags.unicode = true;
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
enum Node {
    Seq(Vec<Node>),
    Alt(Vec<Node>),
    Literal(u32),
    Dot,
    CharClass {
        negated: bool,
        atoms: Vec<ClassAtom>,
    },
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
            capture_count: 0,
            named_groups: Vec::new(),
        };
        let root = parser.parse_alt()?;
        if parser.pos < chars.len() {
            return Err(JsError::syntax("Unexpected ')' in regular expression"));
        }
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

    fn parse_alt(&mut self) -> Result<Node, JsError> {
        let mut branches = vec![self.parse_seq()?];
        while self.peek() == Some('|') {
            self.pos += 1;
            branches.push(self.parse_seq()?);
        }
        if branches.len() == 1 {
            Ok(branches.pop().unwrap())
        } else {
            Ok(Node::Alt(branches))
        }
    }

    fn parse_seq(&mut self) -> Result<Node, JsError> {
        let mut items = Vec::new();
        while let Some(c) = self.peek() {
            if c == '|' || c == ')' {
                break;
            }
            let cap_before = self.capture_count;
            let atom = self.parse_atom()?;
            let cap_after = self.capture_count;
            let node = self.parse_quantifier(atom, cap_before + 1, cap_after)?;
            items.push(node);
        }
        if items.len() == 1 {
            Ok(items.pop().unwrap())
        } else {
            Ok(Node::Seq(items))
        }
    }

    fn parse_atom(&mut self) -> Result<Node, JsError> {
        let Some(c) = self.next() else {
            return Err(JsError::syntax("Unexpected end of regular expression"));
        };
        match c {
            '^' => Ok(Node::AnchorStart),
            '$' => Ok(Node::AnchorEnd),
            '.' => Ok(Node::Dot),
            '[' => self.parse_char_class(),
            '(' => self.parse_group(),
            '\\' => self.parse_escape(false),
            '*' | '+' | '?' => Err(JsError::syntax("Nothing to repeat in regular expression")),
            other => Ok(Node::Literal(other as u32)),
        }
    }

    fn parse_group(&mut self) -> Result<Node, JsError> {
        if self.peek() == Some('?') {
            self.pos += 1;
            match self.peek() {
                Some(':') => {
                    self.pos += 1;
                    let body = self.parse_alt()?;
                    self.expect_char(')')?;
                    Ok(Node::Group {
                        index: None,
                        body: Box::new(body),
                    })
                }
                Some('=') => {
                    self.pos += 1;
                    let body = self.parse_alt()?;
                    self.expect_char(')')?;
                    Ok(Node::Lookaround {
                        body: Box::new(body),
                        positive: true,
                        behind: false,
                    })
                }
                Some('!') => {
                    self.pos += 1;
                    let body = self.parse_alt()?;
                    self.expect_char(')')?;
                    Ok(Node::Lookaround {
                        body: Box::new(body),
                        positive: false,
                        behind: false,
                    })
                }
                Some('<') => {
                    self.pos += 1;
                    match self.peek() {
                        Some('=') => {
                            self.pos += 1;
                            let body = self.parse_alt()?;
                            self.expect_char(')')?;
                            Ok(Node::Lookaround {
                                body: Box::new(body),
                                positive: true,
                                behind: true,
                            })
                        }
                        Some('!') => {
                            self.pos += 1;
                            let body = self.parse_alt()?;
                            self.expect_char(')')?;
                            Ok(Node::Lookaround {
                                body: Box::new(body),
                                positive: false,
                                behind: true,
                            })
                        }
                        _ => {
                            let mut name = String::new();
                            while let Some(ch) = self.next() {
                                if ch == '>' {
                                    break;
                                }
                                name.push(ch);
                            }
                            if name.is_empty() {
                                return Err(JsError::syntax("Empty group name in regex"));
                            }
                            self.capture_count += 1;
                            let idx = self.capture_count;
                            self.named_groups.push((name, idx));
                            let body = self.parse_alt()?;
                            self.expect_char(')')?;
                            Ok(Node::Group {
                                index: Some(idx),
                                body: Box::new(body),
                            })
                        }
                    }
                }
                _ => Err(JsError::syntax("Invalid group specifier in regex")),
            }
        } else {
            self.capture_count += 1;
            let idx = self.capture_count;
            let body = self.parse_alt()?;
            self.expect_char(')')?;
            Ok(Node::Group {
                index: Some(idx),
                body: Box::new(body),
            })
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
                if !in_class
                    && self
                        .peek()
                        .is_some_and(|ch| ('0'..='7').contains(&ch))
                {
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
                    Ok(Node::Literal(val))
                } else {
                    Ok(Node::Literal(0))
                }
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
            'k' if !in_class && self.peek() == Some('<') => {
                self.pos += 1;
                let mut name = String::new();
                while let Some(ch) = self.next() {
                    if ch == '>' {
                        break;
                    }
                    name.push(ch);
                }
                Ok(Node::NamedBackref(name))
            }
            'x' => {
                if let Some(val) = self.try_read_hex(2) {
                    Ok(Node::Literal(val))
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
                    while let Some(ch) = self.next() {
                        if ch == '}' {
                            ok = true;
                            break;
                        }
                        if let Some(d) = ch.to_digit(16) {
                            val = val.saturating_mul(16).saturating_add(d);
                        } else {
                            ok = false;
                            break;
                        }
                    }
                    if ok && val <= 0x10FFFF {
                        Ok(Node::Literal(val))
                    } else {
                        self.pos = saved;
                        Ok(Node::Literal('u' as u32))
                    }
                } else if let Some(val) = self.try_read_hex(4) {
                    Ok(Node::Literal(val))
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
            other => Ok(Node::Literal(other as u32)),
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
                    } else {
                        atoms.push(ClassAtom::Char(start));
                        atoms.push(ClassAtom::Char('-' as u32));
                        atoms.push(second);
                    }
                    continue;
                }
            }
            atoms.push(first);
        }
        Err(JsError::syntax("Unterminated character class in regex"))
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
                if let Some(bounds) = self.try_parse_brace_quantifier() {
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
        Ok(Node::Quantifier {
            body: Box::new(atom),
            min,
            max,
            greedy,
            cap_start,
            cap_end,
        })
    }

    fn try_parse_brace_quantifier(&mut self) -> Option<(usize, Option<usize>)> {
        let saved = self.pos;
        self.pos += 1;
        let min = self.read_uint()?;
        match self.peek() {
            Some('}') => {
                self.pos += 1;
                Some((min, Some(min)))
            }
            Some(',') => {
                self.pos += 1;
                if self.peek() == Some('}') {
                    self.pos += 1;
                    Some((min, None))
                } else {
                    let max = self.read_uint()?;
                    if self.peek() == Some('}') && max >= min {
                        self.pos += 1;
                        Some((min, Some(max)))
                    } else {
                        self.pos = saved;
                        None
                    }
                }
            }
            _ => {
                self.pos = saved;
                None
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
                let (next_pos, unit) = self.read_unit(pos, rev)?;
                let m = match_unicode_prop(unit, prop) ^ *neg;
                if m { cont(self, next_pos) } else { None }
            }
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

    fn read_unit(&self, pos: usize, rev: bool) -> Option<(usize, u32)> {
        if rev {
            if pos == 0 {
                return None;
            }
            let u = self.units[pos - 1];
            if self.flags.unicode && (0xDC00..=0xDFFF).contains(&u) && pos >= 2 {
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
            if self.flags.unicode && (0xD800..=0xDBFF).contains(&u) && pos + 1 < self.units.len() {
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
