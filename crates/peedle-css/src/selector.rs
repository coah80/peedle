use crate::tokenizer::{serialize_tokens, tokenize, HashKind, Token};
use peedle_html::{Document, ElementData, NodeData, NodeId};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Specificity(pub u32, pub u32, pub u32);

impl Specificity {
    pub const ZERO: Self = Self(0, 0, 0);

    pub fn add(self, other: Self) -> Self {
        Self(self.0 + other.0, self.1 + other.1, self.2 + other.2)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PseudoElement {
    Before,
    After,
    FirstLine,
    FirstLetter,
    Placeholder,
    Selection,
    Marker,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttrOperator {
    Exists,
    Equals,
    Includes,
    DashMatch,
    Prefix,
    Suffix,
    Substring,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaseSensitivity {
    Default,
    AsciiCaseInsensitive,
    CaseSensitive,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttrSelector {
    pub name: String,
    pub op: AttrOperator,
    pub value: String,
    pub case: CaseSensitivity,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AnPlusB {
    pub a: i32,
    pub b: i32,
}

impl AnPlusB {
    pub fn matches(self, index_one_based: usize) -> bool {
        let idx = index_one_based as i32;
        if self.a == 0 {
            return idx == self.b;
        }
        let diff = idx - self.b;
        if diff % self.a != 0 {
            return false;
        }
        let n = diff / self.a;
        n >= 0
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum PseudoClass {
    Root,
    Empty,
    FirstChild,
    LastChild,
    OnlyChild,
    NthChild {
        formula: AnPlusB,
        of: Option<SelectorList>,
    },
    NthLastChild {
        formula: AnPlusB,
        of: Option<SelectorList>,
    },
    FirstOfType,
    LastOfType,
    OnlyOfType,
    NthOfType(AnPlusB),
    NthLastOfType(AnPlusB),
    Not(SelectorList),
    Is(SelectorList),
    Where(SelectorList),
    Has(Vec<RelativeSelector>),
    Hover,
    Active,
    Focus,
    FocusVisible,
    FocusWithin,
    Link,
    AnyLink,
    Visited,
    Checked,
    Disabled,
    Enabled,
    Required,
    Optional,
    ReadOnly,
    ReadWrite,
    PlaceholderShown,
    Target,
    Lang(Vec<String>),
    Scope,
    Defined,
    Nesting,
}

#[derive(Clone, Debug, PartialEq)]
pub enum SimpleSelector {
    Universal,
    Type(String),
    Id(String),
    Class(String),
    Attr(AttrSelector),
    PseudoClass(PseudoClass),
    PseudoElement(PseudoElement),
}

impl SimpleSelector {
    pub fn specificity(&self) -> Specificity {
        match self {
            SimpleSelector::Universal => Specificity::ZERO,
            SimpleSelector::Type(_) | SimpleSelector::PseudoElement(_) => Specificity(0, 0, 1),
            SimpleSelector::Id(_) => Specificity(1, 0, 0),
            SimpleSelector::Class(_) | SimpleSelector::Attr(_) => Specificity(0, 1, 0),
            SimpleSelector::PseudoClass(pc) => pc.specificity(),
        }
    }
}

impl PseudoClass {
    pub fn specificity(&self) -> Specificity {
        match self {
            PseudoClass::Where(_) => Specificity::ZERO,
            PseudoClass::Is(list) | PseudoClass::Not(list) => list
                .selectors
                .iter()
                .map(|s| s.specificity())
                .max()
                .unwrap_or(Specificity::ZERO),
            PseudoClass::Has(rels) => rels
                .iter()
                .map(|r| r.selector.specificity())
                .max()
                .unwrap_or(Specificity::ZERO),
            PseudoClass::NthChild { of, .. } | PseudoClass::NthLastChild { of, .. } => {
                let extra = of
                    .as_ref()
                    .and_then(|l| l.selectors.iter().map(|s| s.specificity()).max())
                    .unwrap_or(Specificity::ZERO);
                Specificity(0, 1, 0).add(extra)
            }
            PseudoClass::Nesting => Specificity::ZERO,
            _ => Specificity(0, 1, 0),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct CompoundSelector {
    pub simple: Vec<SimpleSelector>,
}

impl CompoundSelector {
    pub fn specificity(&self) -> Specificity {
        self.simple
            .iter()
            .fold(Specificity::ZERO, |acc, s| acc.add(s.specificity()))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Combinator {
    Descendant,
    Child,
    NextSibling,
    SubsequentSibling,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ComplexSelector {
    pub compounds: Vec<CompoundSelector>,
    pub combinators: Vec<Combinator>,
    pub pseudo_element: Option<PseudoElement>,
}

impl ComplexSelector {
    pub fn specificity(&self) -> Specificity {
        let mut total = self
            .compounds
            .iter()
            .fold(Specificity::ZERO, |acc, c| acc.add(c.specificity()));
        if self.pseudo_element.is_some() {
            total = total.add(Specificity(0, 0, 1));
        }
        total
    }

    pub fn contains_nesting(&self) -> bool {
        self.compounds.iter().any(|c| {
            c.simple.iter().any(|s| match s {
                SimpleSelector::PseudoClass(PseudoClass::Nesting) => true,
                SimpleSelector::PseudoClass(
                    PseudoClass::Is(l) | PseudoClass::Not(l) | PseudoClass::Where(l),
                ) => l.selectors.iter().any(|x| x.contains_nesting()),
                SimpleSelector::PseudoClass(PseudoClass::Has(rels)) => {
                    rels.iter().any(|r| r.selector.contains_nesting())
                }
                _ => false,
            })
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct RelativeSelector {
    pub combinator: Combinator,
    pub selector: ComplexSelector,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SelectorList {
    pub selectors: Vec<ComplexSelector>,
}

pub fn parse_selector_list(input: &str) -> Result<SelectorList, String> {
    let tokens = tokenize(input);
    parse_selector_list_from_tokens(&tokens)
}

pub fn parse_selector_list_from_tokens(tokens: &[Token]) -> Result<SelectorList, String> {
    let groups = split_top_level_commas(tokens);
    let mut selectors = Vec::new();
    for group in groups {
        let sel = parse_complex_selector(group)?;
        selectors.push(sel);
    }
    if selectors.is_empty() {
        return Err("empty selector list".to_string());
    }
    Ok(SelectorList { selectors })
}

pub fn parse_forgiving_selector_list(tokens: &[Token]) -> SelectorList {
    let groups = split_top_level_commas(tokens);
    let mut selectors = Vec::new();
    for group in groups {
        if let Ok(sel) = parse_complex_selector(group) {
            selectors.push(sel);
        }
    }
    SelectorList { selectors }
}

pub fn parse_relative_selector_list(tokens: &[Token]) -> Result<Vec<RelativeSelector>, String> {
    let groups = split_top_level_commas(tokens);
    let mut out = Vec::new();
    for group in groups {
        let mut p = SelParser::new(group);
        p.skip_ws();
        let combinator = match p.peek() {
            Some(Token::Delim('>')) => {
                p.pos += 1;
                Combinator::Child
            }
            Some(Token::Delim('+')) => {
                p.pos += 1;
                Combinator::NextSibling
            }
            Some(Token::Delim('~')) => {
                p.pos += 1;
                Combinator::SubsequentSibling
            }
            _ => Combinator::Descendant,
        };
        let selector = p.parse_complex()?;
        out.push(RelativeSelector {
            combinator,
            selector,
        });
    }
    if out.is_empty() {
        return Err("empty relative selector list".to_string());
    }
    Ok(out)
}

pub fn desugar_nested_selectors(parent: &SelectorList, child_tokens: &[Token]) -> SelectorList {
    let groups = split_top_level_commas(child_tokens);
    let mut out = Vec::new();
    for group in groups {
        let trimmed = trim_ws(group);
        if trimmed.is_empty() {
            continue;
        }
        let leading_comb = match trimmed.first() {
            Some(Token::Delim('>')) => Some(Combinator::Child),
            Some(Token::Delim('+')) => Some(Combinator::NextSibling),
            Some(Token::Delim('~')) => Some(Combinator::SubsequentSibling),
            _ => None,
        };
        if let Some(comb) = leading_comb {
            if let Ok(child_sel) = parse_complex_selector(&trimmed[1..]) {
                for p in &parent.selectors {
                    let mut compounds = p.compounds.clone();
                    let mut combinators = p.combinators.clone();
                    combinators.push(comb);
                    compounds.extend(child_sel.compounds.clone());
                    combinators.extend(child_sel.combinators.clone());
                    out.push(ComplexSelector {
                        compounds,
                        combinators,
                        pseudo_element: child_sel.pseudo_element.or(p.pseudo_element),
                    });
                }
            }
            continue;
        }
        let Ok(child_sel) = parse_complex_selector(trimmed) else {
            continue;
        };
        if child_sel.contains_nesting() {
            let expanded = replace_nesting_in_complex(&child_sel, parent);
            out.extend(expanded);
        } else {
            for p in &parent.selectors {
                let mut compounds = p.compounds.clone();
                let mut combinators = p.combinators.clone();
                combinators.push(Combinator::Descendant);
                compounds.extend(child_sel.compounds.clone());
                combinators.extend(child_sel.combinators.clone());
                out.push(ComplexSelector {
                    compounds,
                    combinators,
                    pseudo_element: child_sel.pseudo_element.or(p.pseudo_element),
                });
            }
        }
    }
    SelectorList { selectors: out }
}

fn replace_nesting_in_complex(
    child: &ComplexSelector,
    parent: &SelectorList,
) -> Vec<ComplexSelector> {
    let parent_is_single_compound =
        parent.selectors.len() == 1 && parent.selectors[0].compounds.len() == 1;
    let mut results = vec![ComplexSelector {
        compounds: Vec::new(),
        combinators: Vec::new(),
        pseudo_element: child.pseudo_element,
    }];

    for (idx, comp) in child.compounds.iter().enumerate() {
        let has_direct_nesting = comp
            .simple
            .iter()
            .any(|s| matches!(s, SimpleSelector::PseudoClass(PseudoClass::Nesting)));

        if idx > 0 {
            let comb = child.combinators[idx - 1];
            for r in &mut results {
                r.combinators.push(comb);
            }
        }

        if has_direct_nesting {
            let rest: Vec<SimpleSelector> = comp
                .simple
                .iter()
                .filter(|s| !matches!(s, SimpleSelector::PseudoClass(PseudoClass::Nesting)))
                .map(|s| replace_nesting_in_simple(s, parent))
                .collect();

            if parent_is_single_compound {
                let p_comp = &parent.selectors[0].compounds[0];
                for r in &mut results {
                    let mut merged = p_comp.simple.clone();
                    merged.extend(rest.clone());
                    r.compounds.push(CompoundSelector { simple: merged });
                }
            } else if parent.selectors.len() == 1 {
                let p_sel = &parent.selectors[0];
                for r in &mut results {
                    let last_idx = p_sel.compounds.len() - 1;
                    for (pi, pc) in p_sel.compounds.iter().enumerate() {
                        if pi > 0 {
                            r.combinators.push(p_sel.combinators[pi - 1]);
                        }
                        if pi == last_idx {
                            let mut merged = pc.simple.clone();
                            merged.extend(rest.clone());
                            r.compounds.push(CompoundSelector { simple: merged });
                        } else {
                            r.compounds.push(pc.clone());
                        }
                    }
                }
            } else {
                let mut merged = vec![SimpleSelector::PseudoClass(PseudoClass::Is(parent.clone()))];
                merged.extend(rest);
                for r in &mut results {
                    r.compounds.push(CompoundSelector {
                        simple: merged.clone(),
                    });
                }
            }
        } else {
            let mapped = CompoundSelector {
                simple: comp
                    .simple
                    .iter()
                    .map(|s| replace_nesting_in_simple(s, parent))
                    .collect(),
            };
            for r in &mut results {
                r.compounds.push(mapped.clone());
            }
        }
    }

    results
}

fn replace_nesting_in_simple(s: &SimpleSelector, parent: &SelectorList) -> SimpleSelector {
    match s {
        SimpleSelector::PseudoClass(PseudoClass::Is(l)) => {
            let mut out = Vec::new();
            for sel in &l.selectors {
                out.extend(replace_nesting_in_complex(sel, parent));
            }
            SimpleSelector::PseudoClass(PseudoClass::Is(SelectorList { selectors: out }))
        }
        SimpleSelector::PseudoClass(PseudoClass::Not(l)) => {
            let mut out = Vec::new();
            for sel in &l.selectors {
                out.extend(replace_nesting_in_complex(sel, parent));
            }
            SimpleSelector::PseudoClass(PseudoClass::Not(SelectorList { selectors: out }))
        }
        SimpleSelector::PseudoClass(PseudoClass::Where(l)) => {
            let mut out = Vec::new();
            for sel in &l.selectors {
                out.extend(replace_nesting_in_complex(sel, parent));
            }
            SimpleSelector::PseudoClass(PseudoClass::Where(SelectorList { selectors: out }))
        }
        SimpleSelector::PseudoClass(PseudoClass::Has(rels)) => {
            let mut out = Vec::new();
            for rel in rels {
                for sel in replace_nesting_in_complex(&rel.selector, parent) {
                    out.push(RelativeSelector {
                        combinator: rel.combinator,
                        selector: sel,
                    });
                }
            }
            SimpleSelector::PseudoClass(PseudoClass::Has(out))
        }
        other => other.clone(),
    }
}

fn trim_ws(tokens: &[Token]) -> &[Token] {
    let mut start = 0;
    let mut end = tokens.len();
    while start < end && tokens[start] == Token::Whitespace {
        start += 1;
    }
    while end > start && tokens[end - 1] == Token::Whitespace {
        end -= 1;
    }
    &tokens[start..end]
}

fn split_top_level_commas(tokens: &[Token]) -> Vec<&[Token]> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut start = 0;
    for (i, t) in tokens.iter().enumerate() {
        match t {
            Token::OpenParen | Token::Function(_) | Token::OpenSquare | Token::OpenCurly => {
                depth += 1;
            }
            Token::CloseParen | Token::CloseSquare | Token::CloseCurly => {
                depth = depth.saturating_sub(1);
            }
            Token::Comma if depth == 0 => {
                out.push(&tokens[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(&tokens[start..]);
    out
}

fn parse_complex_selector(tokens: &[Token]) -> Result<ComplexSelector, String> {
    let mut p = SelParser::new(tokens);
    p.parse_complex()
}

struct SelParser<'a> {
    tokens: &'a [Token],
    pos: usize,
}

impl<'a> SelParser<'a> {
    fn new(tokens: &'a [Token]) -> Self {
        Self { tokens, pos: 0 }
    }

    fn peek(&self) -> Option<&'a Token> {
        self.tokens.get(self.pos)
    }

    fn next(&mut self) -> Option<&'a Token> {
        let t = self.tokens.get(self.pos)?;
        self.pos += 1;
        Some(t)
    }

    fn skip_ws(&mut self) -> bool {
        let mut skipped = false;
        while self.peek() == Some(&Token::Whitespace) {
            self.pos += 1;
            skipped = true;
        }
        skipped
    }

    fn parse_complex(&mut self) -> Result<ComplexSelector, String> {
        self.skip_ws();
        let mut compounds = Vec::new();
        let mut combinators = Vec::new();
        let mut pseudo_element: Option<PseudoElement> = None;

        let (first, pe) = self.parse_compound()?;
        if let Some(p) = pe {
            pseudo_element = Some(p);
        }
        compounds.push(first);

        loop {
            let had_ws = self.skip_ws();
            let Some(tok) = self.peek() else {
                break;
            };
            let comb = match tok {
                Token::Delim('>') => {
                    self.pos += 1;
                    self.skip_ws();
                    Combinator::Child
                }
                Token::Delim('+') => {
                    self.pos += 1;
                    self.skip_ws();
                    Combinator::NextSibling
                }
                Token::Delim('~') => {
                    self.pos += 1;
                    self.skip_ws();
                    Combinator::SubsequentSibling
                }
                _ if had_ws => Combinator::Descendant,
                _ => return Err(format!("unexpected token in selector: {:?}", tok)),
            };
            if self.peek().is_none() {
                return Err("trailing combinator in selector".to_string());
            }
            let (comp, pe) = self.parse_compound()?;
            if let Some(p) = pe {
                pseudo_element = Some(p);
            }
            combinators.push(comb);
            compounds.push(comp);
        }

        Ok(ComplexSelector {
            compounds,
            combinators,
            pseudo_element,
        })
    }

    fn parse_compound(&mut self) -> Result<(CompoundSelector, Option<PseudoElement>), String> {
        let mut simple = Vec::new();
        let mut pseudo_element = None;

        match self.peek() {
            Some(Token::Ident(name)) => {
                let n = name.clone();
                self.pos += 1;
                if self.peek() == Some(&Token::Delim('|')) {
                    self.pos += 1;
                    match self.peek() {
                        Some(Token::Ident(local)) => {
                            let l = local.clone();
                            self.pos += 1;
                            simple.push(SimpleSelector::Type(l));
                        }
                        Some(Token::Delim('*')) => {
                            self.pos += 1;
                            simple.push(SimpleSelector::Universal);
                        }
                        _ => return Err("expected local name after namespace prefix".to_string()),
                    }
                } else {
                    simple.push(SimpleSelector::Type(n));
                }
            }
            Some(Token::Delim('*')) => {
                self.pos += 1;
                if self.peek() == Some(&Token::Delim('|')) {
                    self.pos += 1;
                    match self.peek() {
                        Some(Token::Ident(local)) => {
                            let l = local.clone();
                            self.pos += 1;
                            simple.push(SimpleSelector::Type(l));
                        }
                        Some(Token::Delim('*')) => {
                            self.pos += 1;
                            simple.push(SimpleSelector::Universal);
                        }
                        _ => return Err("expected local name after *|".to_string()),
                    }
                } else {
                    simple.push(SimpleSelector::Universal);
                }
            }
            Some(Token::Delim('|')) => {
                self.pos += 1;
                match self.peek() {
                    Some(Token::Ident(local)) => {
                        let l = local.clone();
                        self.pos += 1;
                        simple.push(SimpleSelector::Type(l));
                    }
                    Some(Token::Delim('*')) => {
                        self.pos += 1;
                        simple.push(SimpleSelector::Universal);
                    }
                    _ => return Err("expected local name after |".to_string()),
                }
            }
            _ => {}
        }

        while let Some(tok) = self.peek() {
            match tok {
                Token::Hash {
                    value,
                    kind: HashKind::Id,
                } => {
                    simple.push(SimpleSelector::Id(value.clone()));
                    self.pos += 1;
                }
                Token::Delim('.') => {
                    self.pos += 1;
                    match self.next() {
                        Some(Token::Ident(cls)) => {
                            simple.push(SimpleSelector::Class(cls.clone()));
                        }
                        _ => return Err("expected class name after '.'".to_string()),
                    }
                }
                Token::Delim('&') => {
                    self.pos += 1;
                    simple.push(SimpleSelector::PseudoClass(PseudoClass::Nesting));
                }
                Token::OpenSquare => {
                    let attr = self.parse_attr_selector()?;
                    simple.push(SimpleSelector::Attr(attr));
                }
                Token::Colon => {
                    self.pos += 1;
                    if self.peek() == Some(&Token::Colon) {
                        self.pos += 1;
                        let pe = self.parse_pseudo_element()?;
                        pseudo_element = Some(pe);
                    } else if let Some(pe) = self.try_legacy_pseudo_element() {
                        pseudo_element = Some(pe);
                    } else {
                        let pc = self.parse_pseudo_class()?;
                        simple.push(SimpleSelector::PseudoClass(pc));
                    }
                }
                _ => break,
            }
        }

        if simple.is_empty() && pseudo_element.is_none() {
            return Err("empty compound selector".to_string());
        }

        Ok((CompoundSelector { simple }, pseudo_element))
    }

    fn try_legacy_pseudo_element(&mut self) -> Option<PseudoElement> {
        let Some(Token::Ident(name)) = self.peek() else {
            return None;
        };
        let pe = match name.to_ascii_lowercase().as_str() {
            "before" => PseudoElement::Before,
            "after" => PseudoElement::After,
            "first-line" => PseudoElement::FirstLine,
            "first-letter" => PseudoElement::FirstLetter,
            _ => return None,
        };
        self.pos += 1;
        Some(pe)
    }

    fn parse_pseudo_element(&mut self) -> Result<PseudoElement, String> {
        match self.next() {
            Some(Token::Ident(name)) => match name.to_ascii_lowercase().as_str() {
                "before" => Ok(PseudoElement::Before),
                "after" => Ok(PseudoElement::After),
                "first-line" => Ok(PseudoElement::FirstLine),
                "first-letter" => Ok(PseudoElement::FirstLetter),
                "placeholder" => Ok(PseudoElement::Placeholder),
                "selection" => Ok(PseudoElement::Selection),
                "marker" => Ok(PseudoElement::Marker),
                other => Err(format!("unknown pseudo-element ::{}", other)),
            },
            _ => Err("expected pseudo-element name".to_string()),
        }
    }

    fn parse_attr_selector(&mut self) -> Result<AttrSelector, String> {
        self.pos += 1;
        self.skip_ws();
        let name = match self.next() {
            Some(Token::Ident(s)) => s.clone(),
            _ => return Err("expected attribute name".to_string()),
        };
        self.skip_ws();
        if self.peek() == Some(&Token::CloseSquare) {
            self.pos += 1;
            return Ok(AttrSelector {
                name,
                op: AttrOperator::Exists,
                value: String::new(),
                case: CaseSensitivity::Default,
            });
        }
        let op = match self.next() {
            Some(Token::Delim('=')) => AttrOperator::Equals,
            Some(Token::Delim('~')) if self.peek() == Some(&Token::Delim('=')) => {
                self.pos += 1;
                AttrOperator::Includes
            }
            Some(Token::Delim('|')) if self.peek() == Some(&Token::Delim('=')) => {
                self.pos += 1;
                AttrOperator::DashMatch
            }
            Some(Token::Delim('^')) if self.peek() == Some(&Token::Delim('=')) => {
                self.pos += 1;
                AttrOperator::Prefix
            }
            Some(Token::Delim('$')) if self.peek() == Some(&Token::Delim('=')) => {
                self.pos += 1;
                AttrOperator::Suffix
            }
            Some(Token::Delim('*')) if self.peek() == Some(&Token::Delim('=')) => {
                self.pos += 1;
                AttrOperator::Substring
            }
            _ => return Err("expected attribute operator".to_string()),
        };
        self.skip_ws();
        let value = match self.next() {
            Some(Token::Ident(s)) | Some(Token::String(s)) => s.clone(),
            _ => return Err("expected attribute value".to_string()),
        };
        self.skip_ws();
        let case = match self.peek() {
            Some(Token::Ident(flag)) if flag.eq_ignore_ascii_case("i") => {
                self.pos += 1;
                self.skip_ws();
                CaseSensitivity::AsciiCaseInsensitive
            }
            Some(Token::Ident(flag)) if flag.eq_ignore_ascii_case("s") => {
                self.pos += 1;
                self.skip_ws();
                CaseSensitivity::CaseSensitive
            }
            _ => CaseSensitivity::Default,
        };
        if self.next() != Some(&Token::CloseSquare) {
            return Err("expected ']' closing attribute selector".to_string());
        }
        Ok(AttrSelector {
            name,
            op,
            value,
            case,
        })
    }

    fn consume_balanced_paren_inner(&mut self) -> Result<&'a [Token], String> {
        let start = self.pos;
        let mut depth = 1usize;
        while let Some(t) = self.peek() {
            match t {
                Token::OpenParen | Token::Function(_) => depth += 1,
                Token::CloseParen => {
                    depth -= 1;
                    if depth == 0 {
                        let slice = &self.tokens[start..self.pos];
                        self.pos += 1;
                        return Ok(slice);
                    }
                }
                _ => {}
            }
            self.pos += 1;
        }
        Err("unclosed '(' in pseudo-class".to_string())
    }

    fn parse_pseudo_class(&mut self) -> Result<PseudoClass, String> {
        match self.next() {
            Some(Token::Ident(name)) => match name.to_ascii_lowercase().as_str() {
                "root" => Ok(PseudoClass::Root),
                "empty" => Ok(PseudoClass::Empty),
                "first-child" => Ok(PseudoClass::FirstChild),
                "last-child" => Ok(PseudoClass::LastChild),
                "only-child" => Ok(PseudoClass::OnlyChild),
                "first-of-type" => Ok(PseudoClass::FirstOfType),
                "last-of-type" => Ok(PseudoClass::LastOfType),
                "only-of-type" => Ok(PseudoClass::OnlyOfType),
                "hover" => Ok(PseudoClass::Hover),
                "active" => Ok(PseudoClass::Active),
                "focus" => Ok(PseudoClass::Focus),
                "focus-visible" => Ok(PseudoClass::FocusVisible),
                "focus-within" => Ok(PseudoClass::FocusWithin),
                "link" => Ok(PseudoClass::Link),
                "any-link" => Ok(PseudoClass::AnyLink),
                "visited" => Ok(PseudoClass::Visited),
                "checked" => Ok(PseudoClass::Checked),
                "disabled" => Ok(PseudoClass::Disabled),
                "enabled" => Ok(PseudoClass::Enabled),
                "required" => Ok(PseudoClass::Required),
                "optional" => Ok(PseudoClass::Optional),
                "read-only" => Ok(PseudoClass::ReadOnly),
                "read-write" => Ok(PseudoClass::ReadWrite),
                "placeholder-shown" => Ok(PseudoClass::PlaceholderShown),
                "target" => Ok(PseudoClass::Target),
                "scope" => Ok(PseudoClass::Scope),
                "defined" => Ok(PseudoClass::Defined),
                other => Err(format!("unknown pseudo-class :{}", other)),
            },
            Some(Token::Function(name)) => {
                let lower = name.to_ascii_lowercase();
                let inner = self.consume_balanced_paren_inner()?;
                match lower.as_str() {
                    "not" => {
                        let list = parse_selector_list_from_tokens(inner)?;
                        Ok(PseudoClass::Not(list))
                    }
                    "is" => Ok(PseudoClass::Is(parse_forgiving_selector_list(inner))),
                    "where" => Ok(PseudoClass::Where(parse_forgiving_selector_list(inner))),
                    "has" => {
                        let rels = parse_relative_selector_list(inner)?;
                        Ok(PseudoClass::Has(rels))
                    }
                    "nth-child" => {
                        let (formula, of) = parse_nth_with_of(inner)?;
                        Ok(PseudoClass::NthChild { formula, of })
                    }
                    "nth-last-child" => {
                        let (formula, of) = parse_nth_with_of(inner)?;
                        Ok(PseudoClass::NthLastChild { formula, of })
                    }
                    "nth-of-type" => {
                        let formula = parse_an_plus_b(inner)?;
                        Ok(PseudoClass::NthOfType(formula))
                    }
                    "nth-last-of-type" => {
                        let formula = parse_an_plus_b(inner)?;
                        Ok(PseudoClass::NthLastOfType(formula))
                    }
                    "lang" => {
                        let mut langs = Vec::new();
                        for part in split_top_level_commas(inner) {
                            let s = serialize_tokens(trim_ws(part)).trim().to_string();
                            let unquoted = s.trim_matches('"').trim_matches('\'').to_string();
                            if !unquoted.is_empty() {
                                langs.push(unquoted);
                            }
                        }
                        if langs.is_empty() {
                            return Err("empty :lang()".to_string());
                        }
                        Ok(PseudoClass::Lang(langs))
                    }
                    other => Err(format!("unknown functional pseudo-class :{}()", other)),
                }
            }
            _ => Err("expected pseudo-class name".to_string()),
        }
    }
}

fn parse_nth_with_of(tokens: &[Token]) -> Result<(AnPlusB, Option<SelectorList>), String> {
    let mut depth = 0usize;
    let mut of_pos = None;
    for (i, t) in tokens.iter().enumerate() {
        match t {
            Token::OpenParen | Token::Function(_) | Token::OpenSquare => depth += 1,
            Token::CloseParen | Token::CloseSquare => depth = depth.saturating_sub(1),
            Token::Ident(s) if depth == 0 && s.eq_ignore_ascii_case("of") => {
                of_pos = Some(i);
                break;
            }
            _ => {}
        }
    }
    if let Some(idx) = of_pos {
        let formula = parse_an_plus_b(&tokens[..idx])?;
        let sel_list = parse_selector_list_from_tokens(&tokens[idx + 1..])?;
        Ok((formula, Some(sel_list)))
    } else {
        Ok((parse_an_plus_b(tokens)?, None))
    }
}

pub fn parse_an_plus_b(tokens: &[Token]) -> Result<AnPlusB, String> {
    let raw = serialize_tokens(trim_ws(tokens))
        .chars()
        .filter(|c| !c.is_ascii_whitespace())
        .collect::<String>()
        .to_ascii_lowercase();
    if raw.is_empty() {
        return Err("empty An+B".to_string());
    }
    if raw == "odd" {
        return Ok(AnPlusB { a: 2, b: 1 });
    }
    if raw == "even" {
        return Ok(AnPlusB { a: 2, b: 0 });
    }
    if let Some(n_pos) = raw.find('n') {
        let a_str = &raw[..n_pos];
        let b_str = &raw[n_pos + 1..];
        let a = match a_str {
            "" | "+" => 1,
            "-" => -1,
            other => other
                .parse::<i32>()
                .map_err(|_| format!("invalid An+B: {}", raw))?,
        };
        let b = if b_str.is_empty() {
            0
        } else {
            b_str
                .parse::<i32>()
                .map_err(|_| format!("invalid An+B: {}", raw))?
        };
        Ok(AnPlusB { a, b })
    } else {
        let b = raw
            .parse::<i32>()
            .map_err(|_| format!("invalid An+B: {}", raw))?;
        Ok(AnPlusB { a: 0, b })
    }
}

pub fn matches_selector(doc: &Document, node: NodeId, sel: &str) -> bool {
    let Ok(list) = parse_selector_list(sel) else {
        return false;
    };
    matches_selector_list(doc, node, &list, None)
}

pub fn matches_selector_list(
    doc: &Document,
    node: NodeId,
    list: &SelectorList,
    scope: Option<NodeId>,
) -> bool {
    list.selectors
        .iter()
        .any(|s| matches_complex_selector(doc, node, s, scope))
}

pub fn matches_complex_selector(
    doc: &Document,
    node: NodeId,
    sel: &ComplexSelector,
    scope: Option<NodeId>,
) -> bool {
    if doc.element(node).is_none() || sel.compounds.is_empty() {
        return false;
    }
    matches_complex_at(doc, node, sel, sel.compounds.len() - 1, scope)
}

fn matches_complex_at(
    doc: &Document,
    node: NodeId,
    sel: &ComplexSelector,
    idx: usize,
    scope: Option<NodeId>,
) -> bool {
    if !matches_compound(doc, node, &sel.compounds[idx], scope) {
        return false;
    }
    if idx == 0 {
        return true;
    }
    let comb = sel.combinators[idx - 1];
    match comb {
        Combinator::Child => {
            let Some(parent) = doc.node(node).parent else {
                return false;
            };
            if doc.element(parent).is_none() {
                return false;
            }
            matches_complex_at(doc, parent, sel, idx - 1, scope)
        }
        Combinator::Descendant => {
            let mut cur = doc.node(node).parent;
            while let Some(p) = cur {
                if doc.element(p).is_some() && matches_complex_at(doc, p, sel, idx - 1, scope) {
                    return true;
                }
                cur = doc.node(p).parent;
            }
            false
        }
        Combinator::NextSibling => {
            let Some(prev) = prev_element_sibling(doc, node) else {
                return false;
            };
            matches_complex_at(doc, prev, sel, idx - 1, scope)
        }
        Combinator::SubsequentSibling => {
            let mut cur = prev_element_sibling(doc, node);
            while let Some(prev) = cur {
                if matches_complex_at(doc, prev, sel, idx - 1, scope) {
                    return true;
                }
                cur = prev_element_sibling(doc, prev);
            }
            false
        }
    }
}

fn prev_element_sibling(doc: &Document, node: NodeId) -> Option<NodeId> {
    let mut cur = doc.node(node).prev_sibling;
    while let Some(id) = cur {
        if doc.element(id).is_some() {
            return Some(id);
        }
        cur = doc.node(id).prev_sibling;
    }
    None
}

fn next_element_sibling(doc: &Document, node: NodeId) -> Option<NodeId> {
    let mut cur = doc.node(node).next_sibling;
    while let Some(id) = cur {
        if doc.element(id).is_some() {
            return Some(id);
        }
        cur = doc.node(id).next_sibling;
    }
    None
}

fn matches_compound(
    doc: &Document,
    node: NodeId,
    comp: &CompoundSelector,
    scope: Option<NodeId>,
) -> bool {
    let Some(el) = doc.element(node) else {
        return false;
    };
    comp.simple
        .iter()
        .all(|s| matches_simple(doc, node, el, s, scope))
}

fn matches_simple(
    doc: &Document,
    node: NodeId,
    el: &ElementData,
    simple: &SimpleSelector,
    scope: Option<NodeId>,
) -> bool {
    match simple {
        SimpleSelector::Universal => true,
        SimpleSelector::Type(tag) => el.tag_is(tag),
        SimpleSelector::Id(id) => el.id() == Some(id.as_str()),
        SimpleSelector::Class(cls) => el.has_class(cls),
        SimpleSelector::Attr(attr) => matches_attr(el, attr),
        SimpleSelector::PseudoElement(_) => true,
        SimpleSelector::PseudoClass(pc) => matches_pseudo_class(doc, node, el, pc, scope),
    }
}

fn is_html_case_insensitive_attr(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "accept"
            | "accept-charset"
            | "align"
            | "alink"
            | "axis"
            | "bgcolor"
            | "charset"
            | "checked"
            | "clear"
            | "codetype"
            | "color"
            | "compact"
            | "declare"
            | "defer"
            | "dir"
            | "direction"
            | "disabled"
            | "enctype"
            | "face"
            | "frame"
            | "hreflang"
            | "http-equiv"
            | "lang"
            | "language"
            | "link"
            | "media"
            | "method"
            | "multiple"
            | "nohref"
            | "noresize"
            | "noshade"
            | "nowrap"
            | "readonly"
            | "rel"
            | "rev"
            | "rules"
            | "scope"
            | "scrolling"
            | "selected"
            | "shape"
            | "target"
            | "text"
            | "type"
            | "valign"
            | "valuetype"
            | "vlink"
    )
}

fn matches_attr(el: &ElementData, attr: &AttrSelector) -> bool {
    let Some(actual) = el.get_attr(&attr.name) else {
        return false;
    };
    if attr.op == AttrOperator::Exists {
        return true;
    }
    let ci = match attr.case {
        CaseSensitivity::AsciiCaseInsensitive => true,
        CaseSensitivity::CaseSensitive => false,
        CaseSensitivity::Default => el.is_html() && is_html_case_insensitive_attr(&attr.name),
    };
    let eq = |a: &str, b: &str| {
        if ci {
            a.eq_ignore_ascii_case(b)
        } else {
            a == b
        }
    };
    match attr.op {
        AttrOperator::Exists => true,
        AttrOperator::Equals => eq(actual, &attr.value),
        AttrOperator::Includes => {
            if attr.value.is_empty() || attr.value.chars().any(|c| c.is_ascii_whitespace()) {
                return false;
            }
            actual
                .split_ascii_whitespace()
                .any(|part| eq(part, &attr.value))
        }
        AttrOperator::DashMatch => {
            if eq(actual, &attr.value) {
                return true;
            }
            if actual.len() > attr.value.len() && actual.is_char_boundary(attr.value.len()) {
                let (head, tail) = actual.split_at(attr.value.len());
                eq(head, &attr.value) && tail.starts_with('-')
            } else {
                false
            }
        }
        AttrOperator::Prefix => {
            if attr.value.is_empty() {
                return false;
            }
            if ci {
                actual
                    .to_ascii_lowercase()
                    .starts_with(&attr.value.to_ascii_lowercase())
            } else {
                actual.starts_with(&attr.value)
            }
        }
        AttrOperator::Suffix => {
            if attr.value.is_empty() {
                return false;
            }
            if ci {
                actual
                    .to_ascii_lowercase()
                    .ends_with(&attr.value.to_ascii_lowercase())
            } else {
                actual.ends_with(&attr.value)
            }
        }
        AttrOperator::Substring => {
            if attr.value.is_empty() {
                return false;
            }
            if ci {
                actual
                    .to_ascii_lowercase()
                    .contains(&attr.value.to_ascii_lowercase())
            } else {
                actual.contains(&attr.value)
            }
        }
    }
}

fn matches_pseudo_class(
    doc: &Document,
    node: NodeId,
    el: &ElementData,
    pc: &PseudoClass,
    scope: Option<NodeId>,
) -> bool {
    match pc {
        PseudoClass::Root => doc.document_element() == Some(node),
        PseudoClass::Empty => {
            for child in doc.children(node) {
                match &doc.node(child).data {
                    NodeData::Element(_) => return false,
                    NodeData::Text(t) if !t.is_empty() => return false,
                    _ => {}
                }
            }
            true
        }
        PseudoClass::FirstChild => {
            doc.node(node).parent.is_some() && prev_element_sibling(doc, node).is_none()
        }
        PseudoClass::LastChild => {
            doc.node(node).parent.is_some() && next_element_sibling(doc, node).is_none()
        }
        PseudoClass::OnlyChild => {
            doc.node(node).parent.is_some()
                && prev_element_sibling(doc, node).is_none()
                && next_element_sibling(doc, node).is_none()
        }
        PseudoClass::NthChild { formula, of } => {
            let Some(parent) = doc.node(node).parent else {
                return false;
            };
            if let Some(ref filter) = of {
                if !matches_selector_list(doc, node, filter, scope) {
                    return false;
                }
            }
            let mut idx = 0usize;
            for sib in doc.element_children(parent) {
                let counted = match of {
                    Some(ref filter) => matches_selector_list(doc, sib, filter, scope),
                    None => true,
                };
                if counted {
                    idx += 1;
                    if sib == node {
                        return formula.matches(idx);
                    }
                }
            }
            false
        }
        PseudoClass::NthLastChild { formula, of } => {
            let Some(parent) = doc.node(node).parent else {
                return false;
            };
            if let Some(ref filter) = of {
                if !matches_selector_list(doc, node, filter, scope) {
                    return false;
                }
            }
            let mut idx = 0usize;
            for sib in doc.element_children(parent).into_iter().rev() {
                let counted = match of {
                    Some(ref filter) => matches_selector_list(doc, sib, filter, scope),
                    None => true,
                };
                if counted {
                    idx += 1;
                    if sib == node {
                        return formula.matches(idx);
                    }
                }
            }
            false
        }
        PseudoClass::FirstOfType => {
            let Some(parent) = doc.node(node).parent else {
                return false;
            };
            doc.element_children(parent)
                .into_iter()
                .find(|&id| {
                    doc.element(id)
                        .map_or(false, |e| e.namespace == el.namespace && e.tag_is(&el.tag_name))
                })
                == Some(node)
        }
        PseudoClass::LastOfType => {
            let Some(parent) = doc.node(node).parent else {
                return false;
            };
            doc.element_children(parent)
                .into_iter()
                .rev()
                .find(|&id| {
                    doc.element(id)
                        .map_or(false, |e| e.namespace == el.namespace && e.tag_is(&el.tag_name))
                })
                == Some(node)
        }
        PseudoClass::OnlyOfType => {
            let Some(parent) = doc.node(node).parent else {
                return false;
            };
            let count = doc
                .element_children(parent)
                .into_iter()
                .filter(|&id| {
                    doc.element(id)
                        .map_or(false, |e| e.namespace == el.namespace && e.tag_is(&el.tag_name))
                })
                .count();
            count == 1
        }
        PseudoClass::NthOfType(formula) => {
            let Some(parent) = doc.node(node).parent else {
                return false;
            };
            let mut idx = 0usize;
            for sib in doc.element_children(parent) {
                let same = doc
                    .element(sib)
                    .map_or(false, |e| e.namespace == el.namespace && e.tag_is(&el.tag_name));
                if same {
                    idx += 1;
                    if sib == node {
                        return formula.matches(idx);
                    }
                }
            }
            false
        }
        PseudoClass::NthLastOfType(formula) => {
            let Some(parent) = doc.node(node).parent else {
                return false;
            };
            let mut idx = 0usize;
            for sib in doc.element_children(parent).into_iter().rev() {
                let same = doc
                    .element(sib)
                    .map_or(false, |e| e.namespace == el.namespace && e.tag_is(&el.tag_name));
                if same {
                    idx += 1;
                    if sib == node {
                        return formula.matches(idx);
                    }
                }
            }
            false
        }
        PseudoClass::Not(list) => !matches_selector_list(doc, node, list, scope),
        PseudoClass::Is(list) | PseudoClass::Where(list) => {
            matches_selector_list(doc, node, list, scope)
        }
        PseudoClass::Has(rels) => rels.iter().any(|r| matches_relative(doc, node, r)),
        PseudoClass::Hover => el.hovered,
        PseudoClass::Active => el.active,
        PseudoClass::Focus | PseudoClass::FocusVisible => el.focused,
        PseudoClass::FocusWithin => {
            if el.focused {
                return true;
            }
            doc.descendants(node)
                .into_iter()
                .any(|d| doc.element(d).map_or(false, |e| e.focused))
        }
        PseudoClass::AnyLink => is_link_element(el),
        PseudoClass::Link => is_link_element(el) && !el.visited,
        PseudoClass::Visited => is_link_element(el) && el.visited,
        PseudoClass::Checked => {
            if el.tag_is("option") {
                el.has_attr("selected")
            } else {
                el.is_checked()
            }
        }
        PseudoClass::Disabled => is_form_control(el) && is_effectively_disabled(doc, node, el),
        PseudoClass::Enabled => is_form_control(el) && !is_effectively_disabled(doc, node, el),
        PseudoClass::Required => is_required_capable(el) && el.has_attr("required"),
        PseudoClass::Optional => is_required_capable(el) && !el.has_attr("required"),
        PseudoClass::ReadWrite => is_editable(el),
        PseudoClass::ReadOnly => !is_editable(el),
        PseudoClass::PlaceholderShown => {
            (el.tag_is("input") || el.tag_is("textarea"))
                && el.has_attr("placeholder")
                && el.current_value().is_empty()
        }
        PseudoClass::Target => {
            let frag = doc.url.split_once('#').map(|(_, f)| f).unwrap_or("");
            !frag.is_empty() && el.id() == Some(frag)
        }
        PseudoClass::Lang(langs) => {
            let Some(effective) = effective_lang(doc, node) else {
                return false;
            };
            langs.iter().any(|wanted| lang_matches(&effective, wanted))
        }
        PseudoClass::Scope | PseudoClass::Nesting => match scope {
            Some(s) => s == node,
            None => doc.document_element() == Some(node),
        },
        PseudoClass::Defined => true,
    }
}

fn matches_relative(doc: &Document, anchor: NodeId, rel: &RelativeSelector) -> bool {
    let candidates: Vec<NodeId> = match rel.combinator {
        Combinator::Descendant => doc
            .descendants(anchor)
            .into_iter()
            .filter(|&id| doc.element(id).is_some())
            .collect(),
        Combinator::Child => doc.element_children(anchor),
        Combinator::NextSibling => next_element_sibling(doc, anchor).into_iter().collect(),
        Combinator::SubsequentSibling => {
            let mut out = Vec::new();
            let mut cur = next_element_sibling(doc, anchor);
            while let Some(id) = cur {
                out.push(id);
                cur = next_element_sibling(doc, id);
            }
            out
        }
    };
    for cand in candidates {
        if matches_relative_tail(doc, cand, anchor, rel) {
            return true;
        }
    }
    false
}

fn matches_relative_tail(
    doc: &Document,
    candidate: NodeId,
    anchor: NodeId,
    rel: &RelativeSelector,
) -> bool {
    let sel = &rel.selector;
    if sel.compounds.is_empty() {
        return false;
    }
    matches_relative_at(doc, candidate, anchor, rel, sel.compounds.len() - 1)
}

fn matches_relative_at(
    doc: &Document,
    node: NodeId,
    anchor: NodeId,
    rel: &RelativeSelector,
    idx: usize,
) -> bool {
    if !matches_compound(doc, node, &rel.selector.compounds[idx], Some(anchor)) {
        return false;
    }
    if idx == 0 {
        return match rel.combinator {
            Combinator::Descendant => doc.is_ancestor(anchor, node),
            Combinator::Child => doc.node(node).parent == Some(anchor),
            Combinator::NextSibling => prev_element_sibling(doc, node) == Some(anchor),
            Combinator::SubsequentSibling => {
                let mut cur = prev_element_sibling(doc, node);
                while let Some(p) = cur {
                    if p == anchor {
                        return true;
                    }
                    cur = prev_element_sibling(doc, p);
                }
                false
            }
        };
    }
    let comb = rel.selector.combinators[idx - 1];
    match comb {
        Combinator::Child => {
            let Some(p) = doc.node(node).parent else {
                return false;
            };
            doc.element(p).is_some() && matches_relative_at(doc, p, anchor, rel, idx - 1)
        }
        Combinator::Descendant => {
            let mut cur = doc.node(node).parent;
            while let Some(p) = cur {
                if doc.element(p).is_some() && matches_relative_at(doc, p, anchor, rel, idx - 1) {
                    return true;
                }
                cur = doc.node(p).parent;
            }
            false
        }
        Combinator::NextSibling => {
            let Some(p) = prev_element_sibling(doc, node) else {
                return false;
            };
            matches_relative_at(doc, p, anchor, rel, idx - 1)
        }
        Combinator::SubsequentSibling => {
            let mut cur = prev_element_sibling(doc, node);
            while let Some(p) = cur {
                if matches_relative_at(doc, p, anchor, rel, idx - 1) {
                    return true;
                }
                cur = prev_element_sibling(doc, p);
            }
            false
        }
    }
}

fn is_link_element(el: &ElementData) -> bool {
    (el.tag_is("a") || el.tag_is("area")) && el.has_attr("href")
}

fn is_form_control(el: &ElementData) -> bool {
    matches!(
        el.tag_name.to_ascii_lowercase().as_str(),
        "button" | "input" | "select" | "textarea" | "optgroup" | "option" | "fieldset"
    )
}

fn is_effectively_disabled(doc: &Document, node: NodeId, el: &ElementData) -> bool {
    if el.is_disabled() {
        return true;
    }
    let mut cur = doc.node(node).parent;
    let mut prev = node;
    while let Some(p) = cur {
        if let Some(pel) = doc.element(p) {
            if pel.tag_is("fieldset") && pel.is_disabled() {
                let first_legend = doc
                    .element_children(p)
                    .into_iter()
                    .find(|&c| doc.element(c).map_or(false, |ce| ce.tag_is("legend")));
                if first_legend != Some(prev) {
                    return true;
                }
            }
        }
        prev = p;
        cur = doc.node(p).parent;
    }
    false
}

fn is_required_capable(el: &ElementData) -> bool {
    el.tag_is("input") || el.tag_is("select") || el.tag_is("textarea")
}

fn is_editable(el: &ElementData) -> bool {
    if let Some(ce) = el.get_attr("contenteditable") {
        if ce.is_empty() || ce.eq_ignore_ascii_case("true") {
            return true;
        }
    }
    if (el.tag_is("input") || el.tag_is("textarea"))
        && !el.has_attr("readonly")
        && !el.is_disabled()
    {
        return true;
    }
    false
}

fn effective_lang(doc: &Document, node: NodeId) -> Option<String> {
    let mut cur = Some(node);
    while let Some(id) = cur {
        if let Some(el) = doc.element(id) {
            if let Some(l) = el.get_attr("lang").or_else(|| el.get_attr("xml:lang")) {
                if !l.is_empty() {
                    return Some(l.to_string());
                }
            }
        }
        cur = doc.node(id).parent;
    }
    None
}

fn lang_matches(actual: &str, wanted: &str) -> bool {
    if wanted == "*" {
        return !actual.is_empty();
    }
    if actual.eq_ignore_ascii_case(wanted) {
        return true;
    }
    if actual.len() > wanted.len() && actual.is_char_boundary(wanted.len()) {
        let (head, tail) = actual.split_at(wanted.len());
        head.eq_ignore_ascii_case(wanted) && tail.starts_with('-')
    } else {
        false
    }
}

pub fn query_selector(doc: &Document, root: NodeId, sel: &str) -> Option<NodeId> {
    let list = parse_selector_list(sel).ok()?;
    let scope = doc.element(root).map(|_| root);
    for id in doc.descendants(root) {
        if doc.element(id).is_some() && matches_selector_list(doc, id, &list, scope) {
            return Some(id);
        }
    }
    None
}

pub fn query_selector_all(doc: &Document, root: NodeId, sel: &str) -> Vec<NodeId> {
    let Ok(list) = parse_selector_list(sel) else {
        return Vec::new();
    };
    let scope = doc.element(root).map(|_| root);
    let mut out = Vec::new();
    for id in doc.descendants(root) {
        if doc.element(id).is_some() && matches_selector_list(doc, id, &list, scope) {
            out.push(id);
        }
    }
    out
}
