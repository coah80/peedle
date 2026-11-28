use crate::selector::{
    desugar_nested_selectors, parse_selector_list_from_tokens, SelectorList,
};
use crate::tokenizer::{serialize_tokens, tokenize, Token};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Viewport {
    pub width: f32,
    pub height: f32,
    pub dark_mode: bool,
}

impl Default for Viewport {
    fn default() -> Self {
        Self {
            width: 1280.0,
            height: 720.0,
            dark_mode: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Declaration {
    pub name: String,
    pub value: Vec<Token>,
    pub important: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StyleRule {
    pub selectors: SelectorList,
    pub declarations: Vec<Declaration>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct KeyframeStop {
    pub offsets: Vec<f32>,
    pub declarations: Vec<Declaration>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct KeyframesRule {
    pub name: String,
    pub stops: Vec<KeyframeStop>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FontFaceRule {
    pub font_family: Option<String>,
    pub src: Vec<String>,
    pub font_weight: Option<String>,
    pub font_style: Option<String>,
    pub declarations: Vec<Declaration>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ImportRule {
    pub url: String,
    pub media: Option<MediaQueryList>,
    pub layer: Option<Option<String>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LayerBlockRule {
    pub name: Option<String>,
    pub rules: Vec<Rule>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LayerStatementRule {
    pub names: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Rule {
    Style(StyleRule),
    Media {
        query: MediaQueryList,
        rules: Vec<Rule>,
    },
    Supports {
        condition: SupportsCondition,
        rules: Vec<Rule>,
    },
    LayerBlock(LayerBlockRule),
    LayerStatement(LayerStatementRule),
    Keyframes(KeyframesRule),
    FontFace(FontFaceRule),
    Import(ImportRule),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Stylesheet {
    pub rules: Vec<Rule>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaType {
    All,
    Screen,
    Print,
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RangeOp {
    Lt,
    Le,
    Eq,
    Ge,
    Gt,
}

#[derive(Clone, Debug, PartialEq)]
pub enum MediaFeature {
    MinWidth(f32),
    MaxWidth(f32),
    Width(RangeOp, f32),
    MinHeight(f32),
    MaxHeight(f32),
    Height(RangeOp, f32),
    WidthRange(f32, RangeOp, RangeOp, f32),
    HeightRange(f32, RangeOp, RangeOp, f32),
    MinColor(i32),
    MaxColor(i32),
    Color(i32),
    MinMonochrome(i32),
    MaxMonochrome(i32),
    Monochrome(i32),
    PrefersColorScheme(bool),
    OrientationLandscape,
    OrientationPortrait,
    Hover(bool),
    Pointer(String),
    Unknown(bool),
    Invalid,
}

#[derive(Clone, Debug, PartialEq)]
pub enum MediaCondition {
    Feature(MediaFeature),
    Not(Box<MediaCondition>),
    And(Vec<MediaCondition>),
    Or(Vec<MediaCondition>),
}

impl MediaCondition {
    pub fn evaluate(&self, vp: &Viewport) -> bool {
        match self {
            MediaCondition::Feature(f) => f.evaluate(vp),
            MediaCondition::Not(inner) => !inner.evaluate(vp),
            MediaCondition::And(list) => list.iter().all(|c| c.evaluate(vp)),
            MediaCondition::Or(list) => list.iter().any(|c| c.evaluate(vp)),
        }
    }

    pub fn has_invalid(&self) -> bool {
        match self {
            MediaCondition::Feature(MediaFeature::Invalid) => true,
            MediaCondition::Feature(_) => false,
            MediaCondition::Not(inner) => inner.has_invalid(),
            MediaCondition::And(list) | MediaCondition::Or(list) => {
                list.iter().any(|c| c.has_invalid())
            }
        }
    }
}

impl MediaFeature {
    pub fn evaluate(&self, vp: &Viewport) -> bool {
        match self {
            MediaFeature::MinWidth(v) => vp.width >= *v,
            MediaFeature::MaxWidth(v) => vp.width <= *v,
            MediaFeature::Width(op, v) => eval_cmp(vp.width, *op, *v),
            MediaFeature::MinHeight(v) => vp.height >= *v,
            MediaFeature::MaxHeight(v) => vp.height <= *v,
            MediaFeature::Height(op, v) => eval_cmp(vp.height, *op, *v),
            MediaFeature::WidthRange(lo, op1, op2, hi) => {
                eval_cmp(*lo, *op1, vp.width) && eval_cmp(vp.width, *op2, *hi)
            }
            MediaFeature::HeightRange(lo, op1, op2, hi) => {
                eval_cmp(*lo, *op1, vp.height) && eval_cmp(vp.height, *op2, *hi)
            }
            MediaFeature::MinColor(v) => 8 >= *v,
            MediaFeature::MaxColor(v) => 8 <= *v,
            MediaFeature::Color(v) => 8 == *v,
            MediaFeature::MinMonochrome(v) => 0 >= *v,
            MediaFeature::MaxMonochrome(v) => 0 <= *v,
            MediaFeature::Monochrome(v) => 0 == *v,
            MediaFeature::PrefersColorScheme(dark) => vp.dark_mode == *dark,
            MediaFeature::OrientationLandscape => vp.width >= vp.height,
            MediaFeature::OrientationPortrait => vp.height > vp.width,
            MediaFeature::Hover(want_hover) => *want_hover,
            MediaFeature::Pointer(kind) => kind.eq_ignore_ascii_case("fine"),
            MediaFeature::Unknown(val) => *val,
            MediaFeature::Invalid => false,
        }
    }
}

fn eval_cmp(lhs: f32, op: RangeOp, rhs: f32) -> bool {
    match op {
        RangeOp::Lt => lhs < rhs,
        RangeOp::Le => lhs <= rhs,
        RangeOp::Eq => (lhs - rhs).abs() < 0.01,
        RangeOp::Ge => lhs >= rhs,
        RangeOp::Gt => lhs > rhs,
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct MediaQuery {
    pub negated: bool,
    pub media_type: MediaType,
    pub condition: Option<MediaCondition>,
}

impl MediaQuery {
    pub fn matches(&self, vp: &Viewport) -> bool {
        if self.condition.as_ref().map_or(false, |c| c.has_invalid()) {
            return false;
        }
        let type_ok = matches!(self.media_type, MediaType::All | MediaType::Screen);
        let cond_ok = self.condition.as_ref().map_or(true, |c| c.evaluate(vp));
        let raw = type_ok && cond_ok;
        if self.negated {
            !raw
        } else {
            raw
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct MediaQueryList {
    pub queries: Vec<MediaQuery>,
}

impl MediaQueryList {
    pub fn matches(&self, vp: &Viewport) -> bool {
        if self.queries.is_empty() {
            return true;
        }
        self.queries.iter().any(|q| q.matches(vp))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum SupportsCondition {
    Declaration(String, Vec<Token>),
    Selector(String),
    Not(Box<SupportsCondition>),
    And(Vec<SupportsCondition>),
    Or(Vec<SupportsCondition>),
}

impl SupportsCondition {
    pub fn evaluate(&self) -> bool {
        match self {
            SupportsCondition::Declaration(prop, vals) => is_supported_declaration(prop, vals),
            SupportsCondition::Selector(sel) => crate::selector::parse_selector_list(sel).is_ok(),
            SupportsCondition::Not(inner) => !inner.evaluate(),
            SupportsCondition::And(list) => list.iter().all(|c| c.evaluate()),
            SupportsCondition::Or(list) => list.iter().any(|c| c.evaluate()),
        }
    }
}

pub fn is_supported_declaration(prop: &str, value: &[Token]) -> bool {
    if prop.starts_with("--") {
        return !value.is_empty();
    }
    let trimmed = trim_ws(value);
    if trimmed.is_empty() {
        return false;
    }
    let p = prop.to_ascii_lowercase();
    let known = matches!(
        p.as_str(),
        "display"
            | "position"
            | "top"
            | "right"
            | "bottom"
            | "left"
            | "inset"
            | "z-index"
            | "float"
            | "clear"
            | "box-sizing"
            | "width"
            | "height"
            | "min-width"
            | "max-width"
            | "min-height"
            | "max-height"
            | "aspect-ratio"
            | "margin"
            | "margin-top"
            | "margin-right"
            | "margin-bottom"
            | "margin-left"
            | "padding"
            | "padding-top"
            | "padding-right"
            | "padding-bottom"
            | "padding-left"
            | "border"
            | "border-width"
            | "border-style"
            | "border-color"
            | "border-top"
            | "border-right"
            | "border-bottom"
            | "border-left"
            | "border-top-width"
            | "border-right-width"
            | "border-bottom-width"
            | "border-left-width"
            | "border-top-style"
            | "border-right-style"
            | "border-bottom-style"
            | "border-left-style"
            | "border-top-color"
            | "border-right-color"
            | "border-bottom-color"
            | "border-left-color"
            | "border-radius"
            | "border-top-left-radius"
            | "border-top-right-radius"
            | "border-bottom-right-radius"
            | "border-bottom-left-radius"
            | "border-collapse"
            | "border-spacing"
            | "outline"
            | "outline-width"
            | "outline-style"
            | "outline-color"
            | "outline-offset"
            | "color"
            | "background"
            | "background-color"
            | "background-image"
            | "background-position"
            | "background-size"
            | "background-repeat"
            | "background-clip"
            | "opacity"
            | "visibility"
            | "overflow"
            | "overflow-x"
            | "overflow-y"
            | "font"
            | "font-family"
            | "font-size"
            | "font-weight"
            | "font-style"
            | "line-height"
            | "letter-spacing"
            | "word-spacing"
            | "text-align"
            | "text-decoration"
            | "text-decoration-line"
            | "text-decoration-color"
            | "text-decoration-style"
            | "text-transform"
            | "text-indent"
            | "text-overflow"
            | "white-space"
            | "word-break"
            | "overflow-wrap"
            | "vertical-align"
            | "flex"
            | "flex-direction"
            | "flex-wrap"
            | "flex-flow"
            | "flex-grow"
            | "flex-shrink"
            | "flex-basis"
            | "justify-content"
            | "align-items"
            | "align-self"
            | "align-content"
            | "justify-items"
            | "justify-self"
            | "place-items"
            | "place-content"
            | "place-self"
            | "order"
            | "gap"
            | "row-gap"
            | "column-gap"
            | "grid-template-columns"
            | "grid-template-rows"
            | "grid-auto-columns"
            | "grid-auto-rows"
            | "grid-auto-flow"
            | "grid-column"
            | "grid-column-start"
            | "grid-column-end"
            | "grid-row"
            | "grid-row-start"
            | "grid-row-end"
            | "grid-area"
            | "box-shadow"
            | "text-shadow"
            | "transform"
            | "transform-origin"
            | "filter"
            | "backdrop-filter"
            | "list-style"
            | "list-style-type"
            | "list-style-position"
            | "content"
            | "cursor"
            | "pointer-events"
            | "user-select"
            | "table-layout"
            | "object-fit"
            | "object-position"
            | "color-scheme"
    );
    if !known {
        return false;
    }
    let val_str = serialize_tokens(trimmed).trim().to_ascii_lowercase();
    if p == "display" {
        return matches!(
            val_str.as_str(),
            "block"
                | "inline"
                | "inline-block"
                | "flex"
                | "inline-flex"
                | "grid"
                | "inline-grid"
                | "table"
                | "inline-table"
                | "table-row"
                | "table-cell"
                | "table-row-group"
                | "table-header-group"
                | "table-footer-group"
                | "table-caption"
                | "list-item"
                | "contents"
                | "flow-root"
                | "none"
                | "initial"
                | "inherit"
                | "unset"
                | "revert"
        );
    }
    true
}

pub fn parse_stylesheet(input: &str) -> Stylesheet {
    let tokens = tokenize(input);
    let mut parser = CssParser::new(&tokens);
    let rules = parser.parse_rule_list(true);
    Stylesheet { rules }
}

pub fn parse_inline_declarations(input: &str) -> Vec<Declaration> {
    let tokens = tokenize(input);
    let mut parser = CssParser::new(&tokens);
    parser.parse_declaration_list_only()
}

pub fn parse_media_query_list(input: &str) -> MediaQueryList {
    let tokens = tokenize(input);
    parse_media_query_list_tokens(&tokens)
}

pub fn parse_supports_condition(input: &str) -> Option<SupportsCondition> {
    let tokens = tokenize(input);
    parse_supports_condition_tokens(&tokens)
}

struct CssParser<'a> {
    tokens: &'a [Token],
    pos: usize,
}

impl<'a> CssParser<'a> {
    fn new(tokens: &'a [Token]) -> Self {
        Self { tokens, pos: 0 }
    }

    fn peek(&self) -> Option<&'a Token> {
        self.tokens.get(self.pos)
    }

    fn skip_ws(&mut self) {
        while matches!(
            self.peek(),
            Some(Token::Whitespace | Token::Cdo | Token::Cdc)
        ) {
            self.pos += 1;
        }
    }

    fn parse_rule_list(&mut self, top_level: bool) -> Vec<Rule> {
        let mut rules = Vec::new();
        while self.pos < self.tokens.len() {
            self.skip_ws();
            let Some(tok) = self.peek() else {
                break;
            };
            match tok {
                Token::CloseCurly if !top_level => {
                    self.pos += 1;
                    break;
                }
                Token::AtKeyword(_) => {
                    if let Some(r) = self.parse_at_rule() {
                        rules.push(r);
                    }
                }
                _ => {
                    let parsed = self.parse_qualified_rule(None);
                    rules.extend(parsed);
                }
            }
        }
        rules
    }

    fn parse_at_rule(&mut self) -> Option<Rule> {
        let name = match self.peek()? {
            Token::AtKeyword(n) => n.to_ascii_lowercase(),
            _ => return None,
        };
        self.pos += 1;
        let prelude_start = self.pos;
        let mut paren_depth = 0usize;
        let mut bracket_depth = 0usize;
        let mut has_block = false;
        while let Some(t) = self.peek() {
            match t {
                Token::OpenParen | Token::Function(_) => paren_depth += 1,
                Token::CloseParen => paren_depth = paren_depth.saturating_sub(1),
                Token::OpenSquare => bracket_depth += 1,
                Token::CloseSquare => bracket_depth = bracket_depth.saturating_sub(1),
                Token::Semicolon if paren_depth == 0 && bracket_depth == 0 => {
                    break;
                }
                Token::OpenCurly if paren_depth == 0 && bracket_depth == 0 => {
                    has_block = true;
                    break;
                }
                _ => {}
            }
            self.pos += 1;
        }
        let prelude = trim_ws(&self.tokens[prelude_start..self.pos]);
        if has_block {
            self.pos += 1;
            let block_tokens = self.consume_block_contents();
            match name.as_str() {
                "media" => {
                    let query = parse_media_query_list_tokens(prelude);
                    let mut sub = CssParser::new(block_tokens);
                    let rules = sub.parse_rule_list(false);
                    Some(Rule::Media { query, rules })
                }
                "supports" => {
                    let condition = parse_supports_condition_tokens(prelude)?;
                    let mut sub = CssParser::new(block_tokens);
                    let rules = sub.parse_rule_list(false);
                    Some(Rule::Supports { condition, rules })
                }
                "layer" => {
                    let layer_name = parse_single_layer_name(prelude);
                    let mut sub = CssParser::new(block_tokens);
                    let rules = sub.parse_rule_list(false);
                    Some(Rule::LayerBlock(LayerBlockRule {
                        name: layer_name,
                        rules,
                    }))
                }
                "keyframes" | "-webkit-keyframes" => {
                    let kf_name = parse_keyframes_name(prelude)?;
                    let stops = parse_keyframes_body(block_tokens);
                    Some(Rule::Keyframes(KeyframesRule {
                        name: kf_name,
                        stops,
                    }))
                }
                "font-face" => {
                    let mut sub = CssParser::new(block_tokens);
                    let decls = sub.parse_declaration_list_only();
                    Some(Rule::FontFace(build_font_face(decls)))
                }
                _ => None,
            }
        } else {
            if self.peek() == Some(&Token::Semicolon) {
                self.pos += 1;
            }
            match name.as_str() {
                "layer" => {
                    let names = parse_layer_statement_names(prelude);
                    if names.is_empty() {
                        None
                    } else {
                        Some(Rule::LayerStatement(LayerStatementRule { names }))
                    }
                }
                "import" => parse_import_prelude(prelude).map(Rule::Import),
                _ => None,
            }
        }
    }

    fn consume_block_contents(&mut self) -> &'a [Token] {
        let start = self.pos;
        let mut depth = 1usize;
        while let Some(t) = self.peek() {
            match t {
                Token::OpenCurly => depth += 1,
                Token::CloseCurly => {
                    depth -= 1;
                    if depth == 0 {
                        let slice = &self.tokens[start..self.pos];
                        self.pos += 1;
                        return slice;
                    }
                }
                _ => {}
            }
            self.pos += 1;
        }
        &self.tokens[start..self.pos]
    }

    fn parse_qualified_rule(&mut self, parent_selectors: Option<&SelectorList>) -> Vec<Rule> {
        let prelude_start = self.pos;
        let mut paren_depth = 0usize;
        let mut bracket_depth = 0usize;
        let mut found_block = false;
        while let Some(t) = self.peek() {
            match t {
                Token::OpenParen | Token::Function(_) => paren_depth += 1,
                Token::CloseParen => paren_depth = paren_depth.saturating_sub(1),
                Token::OpenSquare => bracket_depth += 1,
                Token::CloseSquare => bracket_depth = bracket_depth.saturating_sub(1),
                Token::OpenCurly if paren_depth == 0 && bracket_depth == 0 => {
                    found_block = true;
                    break;
                }
                Token::CloseCurly if paren_depth == 0 && bracket_depth == 0 => {
                    break;
                }
                _ => {}
            }
            self.pos += 1;
        }
        if !found_block {
            return Vec::new();
        }
        let prelude = trim_ws(&self.tokens[prelude_start..self.pos]);
        self.pos += 1;
        let block_tokens = self.consume_block_contents();

        let selectors = match parent_selectors {
            Some(parent) => desugar_nested_selectors(parent, prelude),
            None => match parse_selector_list_from_tokens(prelude) {
                Ok(s) => s,
                Err(_) => return Vec::new(),
            },
        };
        if selectors.selectors.is_empty() {
            return Vec::new();
        }

        let mut sub = CssParser::new(block_tokens);
        let (decls, nested_rules) = sub.parse_style_block(&selectors);

        let mut out = Vec::new();
        if !decls.is_empty() {
            out.push(Rule::Style(StyleRule {
                selectors,
                declarations: decls,
            }));
        }
        out.extend(nested_rules);
        out
    }

    fn parse_style_block(&mut self, parent: &SelectorList) -> (Vec<Declaration>, Vec<Rule>) {
        let mut decls = Vec::new();
        let mut nested = Vec::new();

        while self.pos < self.tokens.len() {
            self.skip_ws();
            let Some(tok) = self.peek() else {
                break;
            };
            match tok {
                Token::Semicolon => {
                    self.pos += 1;
                }
                Token::AtKeyword(_) => {
                    if let Some(r) = self.parse_nested_at_rule(parent) {
                        nested.push(r);
                    }
                }
                _ => {
                    if self.looks_like_nested_rule() {
                        let rules = self.parse_qualified_rule(Some(parent));
                        nested.extend(rules);
                    } else if let Some(d) = self.parse_declaration() {
                        decls.push(d);
                    }
                }
            }
        }

        (decls, nested)
    }

    fn parse_nested_at_rule(&mut self, parent: &SelectorList) -> Option<Rule> {
        let name = match self.peek()? {
            Token::AtKeyword(n) => n.to_ascii_lowercase(),
            _ => return None,
        };
        self.pos += 1;
        let prelude_start = self.pos;
        let mut paren_depth = 0usize;
        let mut has_block = false;
        while let Some(t) = self.peek() {
            match t {
                Token::OpenParen | Token::Function(_) => paren_depth += 1,
                Token::CloseParen => paren_depth = paren_depth.saturating_sub(1),
                Token::Semicolon if paren_depth == 0 => break,
                Token::OpenCurly if paren_depth == 0 => {
                    has_block = true;
                    break;
                }
                _ => {}
            }
            self.pos += 1;
        }
        let prelude = trim_ws(&self.tokens[prelude_start..self.pos]);
        if !has_block {
            if self.peek() == Some(&Token::Semicolon) {
                self.pos += 1;
            }
            return None;
        }
        self.pos += 1;
        let block_tokens = self.consume_block_contents();
        let mut sub = CssParser::new(block_tokens);
        let (decls, nested) = sub.parse_style_block(parent);
        let mut inner_rules = Vec::new();
        if !decls.is_empty() {
            inner_rules.push(Rule::Style(StyleRule {
                selectors: parent.clone(),
                declarations: decls,
            }));
        }
        inner_rules.extend(nested);
        match name.as_str() {
            "media" => Some(Rule::Media {
                query: parse_media_query_list_tokens(prelude),
                rules: inner_rules,
            }),
            "supports" => {
                let condition = parse_supports_condition_tokens(prelude)?;
                Some(Rule::Supports {
                    condition,
                    rules: inner_rules,
                })
            }
            "layer" => Some(Rule::LayerBlock(LayerBlockRule {
                name: parse_single_layer_name(prelude),
                rules: inner_rules,
            })),
            _ => None,
        }
    }

    fn looks_like_nested_rule(&self) -> bool {
        let mut i = self.pos;
        let mut paren_depth = 0usize;
        let mut bracket_depth = 0usize;
        let mut saw_colon = false;
        let mut is_custom_prop = false;
        if let Some(Token::Ident(name)) = self.tokens.get(i) {
            if name.starts_with("--") {
                is_custom_prop = true;
            }
        }
        while let Some(t) = self.tokens.get(i) {
            match t {
                Token::OpenParen | Token::Function(_) => paren_depth += 1,
                Token::CloseParen => paren_depth = paren_depth.saturating_sub(1),
                Token::OpenSquare => bracket_depth += 1,
                Token::CloseSquare => bracket_depth = bracket_depth.saturating_sub(1),
                Token::Colon if paren_depth == 0 && bracket_depth == 0 => {
                    saw_colon = true;
                }
                Token::Semicolon if paren_depth == 0 && bracket_depth == 0 => {
                    return false;
                }
                Token::OpenCurly if paren_depth == 0 && bracket_depth == 0 => {
                    if is_custom_prop && saw_colon {
                        return false;
                    }
                    return true;
                }
                Token::CloseCurly if paren_depth == 0 && bracket_depth == 0 => {
                    return false;
                }
                _ => {}
            }
            i += 1;
        }
        false
    }

    fn parse_declaration_list_only(&mut self) -> Vec<Declaration> {
        let mut decls = Vec::new();
        while self.pos < self.tokens.len() {
            self.skip_ws();
            if self.peek().is_none() {
                break;
            }
            if self.peek() == Some(&Token::Semicolon) {
                self.pos += 1;
                continue;
            }
            if let Some(d) = self.parse_declaration() {
                decls.push(d);
            }
        }
        decls
    }

    fn parse_declaration(&mut self) -> Option<Declaration> {
        self.skip_ws();
        let name = match self.peek() {
            Some(Token::Ident(n)) => {
                let name = if n.starts_with("--") {
                    n.clone()
                } else {
                    n.to_ascii_lowercase()
                };
                self.pos += 1;
                name
            }
            _ => {
                self.skip_to_semicolon();
                return None;
            }
        };
        self.skip_ws();
        if self.peek() != Some(&Token::Colon) {
            self.skip_to_semicolon();
            return None;
        }
        self.pos += 1;

        let val_start = self.pos;
        let mut paren_depth = 0usize;
        let mut bracket_depth = 0usize;
        let mut curly_depth = 0usize;
        while let Some(t) = self.peek() {
            match t {
                Token::OpenParen | Token::Function(_) => paren_depth += 1,
                Token::CloseParen => paren_depth = paren_depth.saturating_sub(1),
                Token::OpenSquare => bracket_depth += 1,
                Token::CloseSquare => bracket_depth = bracket_depth.saturating_sub(1),
                Token::OpenCurly => curly_depth += 1,
                Token::CloseCurly => {
                    if paren_depth == 0 && bracket_depth == 0 && curly_depth == 0 {
                        break;
                    }
                    curly_depth = curly_depth.saturating_sub(1);
                }
                Token::Semicolon if paren_depth == 0 && bracket_depth == 0 && curly_depth == 0 => {
                    break;
                }
                _ => {}
            }
            self.pos += 1;
        }
        let raw_val = &self.tokens[val_start..self.pos];
        if self.peek() == Some(&Token::Semicolon) {
            self.pos += 1;
        }

        let (value, important) = extract_important(raw_val);
        if value.is_empty() && !name.starts_with("--") {
            return None;
        }
        Some(Declaration {
            name,
            value,
            important,
        })
    }

    fn skip_to_semicolon(&mut self) {
        let mut depth = 0usize;
        while let Some(t) = self.peek() {
            match t {
                Token::OpenCurly | Token::OpenParen | Token::Function(_) | Token::OpenSquare => {
                    depth += 1;
                }
                Token::CloseCurly | Token::CloseParen | Token::CloseSquare => {
                    depth = depth.saturating_sub(1);
                }
                Token::Semicolon if depth == 0 => {
                    self.pos += 1;
                    break;
                }
                _ => {}
            }
            self.pos += 1;
        }
    }
}

fn extract_important(tokens: &[Token]) -> (Vec<Token>, bool) {
    let mut end = tokens.len();
    while end > 0 && tokens[end - 1] == Token::Whitespace {
        end -= 1;
    }
    let mut important = false;
    if end >= 2 {
        if let Token::Ident(ref kw) = tokens[end - 1] {
            if kw.eq_ignore_ascii_case("important") {
                let mut bang_idx = end - 2;
                while bang_idx > 0 && tokens[bang_idx] == Token::Whitespace {
                    bang_idx -= 1;
                }
                if tokens[bang_idx] == Token::Delim('!') {
                    important = true;
                    end = bang_idx;
                }
            }
        }
    }
    let trimmed = trim_ws(&tokens[..end]);
    (trimmed.to_vec(), important)
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

fn split_commas(tokens: &[Token]) -> Vec<&[Token]> {
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

fn parse_single_layer_name(tokens: &[Token]) -> Option<String> {
    let s = serialize_tokens(trim_ws(tokens)).trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

fn parse_layer_statement_names(tokens: &[Token]) -> Vec<String> {
    split_commas(tokens)
        .into_iter()
        .filter_map(|part| {
            let s = serialize_tokens(trim_ws(part)).trim().to_string();
            if s.is_empty() {
                None
            } else {
                Some(s)
            }
        })
        .collect()
}

fn parse_keyframes_name(tokens: &[Token]) -> Option<String> {
    for t in tokens {
        match t {
            Token::Ident(s) | Token::String(s) => return Some(s.clone()),
            _ => {}
        }
    }
    None
}

fn parse_keyframes_body(tokens: &[Token]) -> Vec<KeyframeStop> {
    let mut stops = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        while i < tokens.len() && tokens[i] == Token::Whitespace {
            i += 1;
        }
        let prelude_start = i;
        while i < tokens.len() && tokens[i] != Token::OpenCurly {
            i += 1;
        }
        if i >= tokens.len() {
            break;
        }
        let prelude = trim_ws(&tokens[prelude_start..i]);
        i += 1;
        let block_start = i;
        let mut depth = 1usize;
        while i < tokens.len() && depth > 0 {
            match tokens[i] {
                Token::OpenCurly => depth += 1,
                Token::CloseCurly => depth -= 1,
                _ => {}
            }
            i += 1;
        }
        let block_end = if depth == 0 { i - 1 } else { i };
        let mut offsets = Vec::new();
        for part in split_commas(prelude) {
            for t in trim_ws(part) {
                match t {
                    Token::Ident(s) if s.eq_ignore_ascii_case("from") => offsets.push(0.0),
                    Token::Ident(s) if s.eq_ignore_ascii_case("to") => offsets.push(1.0),
                    Token::Percentage { value, .. } => {
                        offsets.push((*value / 100.0).clamp(0.0, 1.0));
                    }
                    _ => {}
                }
            }
        }
        let mut sub = CssParser::new(&tokens[block_start..block_end]);
        let declarations = sub.parse_declaration_list_only();
        if !offsets.is_empty() {
            stops.push(KeyframeStop {
                offsets,
                declarations,
            });
        }
    }
    stops
}

fn build_font_face(declarations: Vec<Declaration>) -> FontFaceRule {
    let mut font_family = None;
    let mut src = Vec::new();
    let mut font_weight = None;
    let mut font_style = None;

    for decl in &declarations {
        match decl.name.as_str() {
            "font-family" => {
                let raw = serialize_tokens(&decl.value);
                font_family = Some(raw.trim().trim_matches('"').trim_matches('\'').to_string());
            }
            "src" => {
                let mut i = 0;
                while i < decl.value.len() {
                    match &decl.value[i] {
                        Token::Url(u) => src.push(u.clone()),
                        Token::Function(f) if f.eq_ignore_ascii_case("url") => {
                            if let Some(Token::String(u)) = decl.value.get(i + 1) {
                                src.push(u.clone());
                            } else if let Some(Token::Whitespace) = decl.value.get(i + 1) {
                                if let Some(Token::String(u)) = decl.value.get(i + 2) {
                                    src.push(u.clone());
                                }
                            }
                        }
                        _ => {}
                    }
                    i += 1;
                }
            }
            "font-weight" => {
                font_weight = Some(serialize_tokens(&decl.value).trim().to_string());
            }
            "font-style" => {
                font_style = Some(serialize_tokens(&decl.value).trim().to_string());
            }
            _ => {}
        }
    }

    FontFaceRule {
        font_family,
        src,
        font_weight,
        font_style,
        declarations,
    }
}

fn parse_import_prelude(tokens: &[Token]) -> Option<ImportRule> {
    let mut i = 0;
    while i < tokens.len() && tokens[i] == Token::Whitespace {
        i += 1;
    }
    let url = match tokens.get(i)? {
        Token::String(s) => {
            i += 1;
            s.clone()
        }
        Token::Url(u) => {
            i += 1;
            u.clone()
        }
        Token::Function(f) if f.eq_ignore_ascii_case("url") => {
            i += 1;
            while i < tokens.len() && tokens[i] == Token::Whitespace {
                i += 1;
            }
            let u = match tokens.get(i) {
                Some(Token::String(s)) => {
                    i += 1;
                    s.clone()
                }
                _ => return None,
            };
            while i < tokens.len() && tokens[i] != Token::CloseParen {
                i += 1;
            }
            if i < tokens.len() {
                i += 1;
            }
            u
        }
        _ => return None,
    };

    while i < tokens.len() && tokens[i] == Token::Whitespace {
        i += 1;
    }

    let mut layer = None;
    match tokens.get(i) {
        Some(Token::Ident(s)) if s.eq_ignore_ascii_case("layer") => {
            layer = Some(None);
            i += 1;
        }
        Some(Token::Function(s)) if s.eq_ignore_ascii_case("layer") => {
            i += 1;
            let start = i;
            let mut depth = 1usize;
            while i < tokens.len() && depth > 0 {
                match tokens[i] {
                    Token::OpenParen | Token::Function(_) => depth += 1,
                    Token::CloseParen => depth -= 1,
                    _ => {}
                }
                i += 1;
            }
            let end = if depth == 0 { i - 1 } else { i };
            let name = serialize_tokens(trim_ws(&tokens[start..end]))
                .trim()
                .to_string();
            layer = Some(if name.is_empty() { None } else { Some(name) });
        }
        _ => {}
    }

    let rest = trim_ws(&tokens[i..]);
    let media = if rest.is_empty() {
        None
    } else {
        Some(parse_media_query_list_tokens(rest))
    };

    Some(ImportRule { url, media, layer })
}

pub fn parse_media_query_list_tokens(tokens: &[Token]) -> MediaQueryList {
    let trimmed = trim_ws(tokens);
    if trimmed.is_empty() {
        return MediaQueryList {
            queries: Vec::new(),
        };
    }
    let mut queries = Vec::new();
    for part in split_commas(trimmed) {
        let p = trim_ws(part);
        if p.is_empty() {
            continue;
        }
        queries.push(parse_single_media_query(p));
    }
    MediaQueryList { queries }
}

fn parse_single_media_query(tokens: &[Token]) -> MediaQuery {
    let mut i = 0;
    let mut negated = false;
    let mut media_type = MediaType::All;

    if let Some(Token::Ident(s)) = tokens.get(i) {
        if s.eq_ignore_ascii_case("not") {
            let mut look = i + 1;
            while matches!(tokens.get(look), Some(Token::Whitespace)) {
                look += 1;
            }
            if !matches!(tokens.get(look), Some(Token::OpenParen)) {
                negated = true;
                i = look;
            }
        } else if s.eq_ignore_ascii_case("only") {
            i += 1;
            while matches!(tokens.get(i), Some(Token::Whitespace)) {
                i += 1;
            }
        }
    }

    if let Some(Token::Ident(s)) = tokens.get(i) {
        let lower = s.to_ascii_lowercase();
        if !matches!(lower.as_str(), "not" | "and" | "or") {
            media_type = match lower.as_str() {
                "all" => MediaType::All,
                "screen" => MediaType::Screen,
                "print" => MediaType::Print,
                _ => MediaType::Other,
            };
            i += 1;
            while matches!(tokens.get(i), Some(Token::Whitespace)) {
                i += 1;
            }
            if let Some(Token::Ident(and_kw)) = tokens.get(i) {
                if and_kw.eq_ignore_ascii_case("and") {
                    i += 1;
                    while matches!(tokens.get(i), Some(Token::Whitespace)) {
                        i += 1;
                    }
                }
            }
        }
    }

    let rest = trim_ws(&tokens[i..]);
    let condition = if rest.is_empty() {
        None
    } else {
        Some(
            parse_media_condition(rest)
                .unwrap_or(MediaCondition::Feature(MediaFeature::Invalid)),
        )
    };

    MediaQuery {
        negated,
        media_type,
        condition,
    }
}

fn parse_media_condition(tokens: &[Token]) -> Option<MediaCondition> {
    let mut p = CondParser::new(tokens);
    let cond = p.parse_media_expr()?;
    p.skip_ws();
    if p.peek().is_some() {
        return None;
    }
    Some(cond)
}

fn parse_supports_condition_tokens(tokens: &[Token]) -> Option<SupportsCondition> {
    let mut p = CondParser::new(tokens);
    p.parse_supports_expr()
}

struct CondParser<'a> {
    tokens: &'a [Token],
    pos: usize,
}

impl<'a> CondParser<'a> {
    fn new(tokens: &'a [Token]) -> Self {
        Self { tokens, pos: 0 }
    }

    fn peek(&self) -> Option<&'a Token> {
        self.tokens.get(self.pos)
    }

    fn skip_ws(&mut self) {
        while self.peek() == Some(&Token::Whitespace) {
            self.pos += 1;
        }
    }

    fn consume_paren_inner(&mut self) -> Option<&'a [Token]> {
        if self.peek() != Some(&Token::OpenParen) {
            return None;
        }
        self.pos += 1;
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
                        return Some(slice);
                    }
                }
                _ => {}
            }
            self.pos += 1;
        }
        None
    }

    fn parse_media_expr(&mut self) -> Option<MediaCondition> {
        let mut items = Vec::new();
        let mut op: Option<&'static str> = None;

        loop {
            self.skip_ws();
            let item = self.parse_media_atom()?;
            items.push(item);
            self.skip_ws();
            let Some(Token::Ident(kw)) = self.peek() else {
                break;
            };
            if kw.eq_ignore_ascii_case("and") {
                if op == Some("or") {
                    break;
                }
                op = Some("and");
                self.pos += 1;
            } else if kw.eq_ignore_ascii_case("or") {
                if op == Some("and") {
                    break;
                }
                op = Some("or");
                self.pos += 1;
            } else {
                break;
            }
        }

        if items.len() == 1 {
            items.into_iter().next()
        } else if op == Some("or") {
            Some(MediaCondition::Or(items))
        } else {
            Some(MediaCondition::And(items))
        }
    }

    fn parse_media_atom(&mut self) -> Option<MediaCondition> {
        self.skip_ws();
        if let Some(Token::Ident(kw)) = self.peek() {
            if kw.eq_ignore_ascii_case("not") {
                self.pos += 1;
                let inner = self.parse_media_atom()?;
                return Some(MediaCondition::Not(Box::new(inner)));
            }
        }
        let inner = trim_ws(self.consume_paren_inner()?);
        if is_nested_condition(inner) {
            let mut sub = CondParser::new(inner);
            sub.parse_media_expr()
        } else {
            Some(MediaCondition::Feature(parse_media_feature(inner)))
        }
    }

    fn parse_supports_expr(&mut self) -> Option<SupportsCondition> {
        let mut items = Vec::new();
        let mut op: Option<&'static str> = None;

        loop {
            self.skip_ws();
            let item = self.parse_supports_atom()?;
            items.push(item);
            self.skip_ws();
            let Some(Token::Ident(kw)) = self.peek() else {
                break;
            };
            if kw.eq_ignore_ascii_case("and") {
                if op == Some("or") {
                    break;
                }
                op = Some("and");
                self.pos += 1;
            } else if kw.eq_ignore_ascii_case("or") {
                if op == Some("and") {
                    break;
                }
                op = Some("or");
                self.pos += 1;
            } else {
                break;
            }
        }

        if items.len() == 1 {
            items.into_iter().next()
        } else if op == Some("or") {
            Some(SupportsCondition::Or(items))
        } else {
            Some(SupportsCondition::And(items))
        }
    }

    fn parse_supports_atom(&mut self) -> Option<SupportsCondition> {
        self.skip_ws();
        if let Some(Token::Ident(kw)) = self.peek() {
            if kw.eq_ignore_ascii_case("not") {
                self.pos += 1;
                let inner = self.parse_supports_atom()?;
                return Some(SupportsCondition::Not(Box::new(inner)));
            }
        }
        if let Some(Token::Function(fn_name)) = self.peek() {
            if fn_name.eq_ignore_ascii_case("selector") {
                self.pos += 1;
                let start = self.pos;
                let mut depth = 1usize;
                while let Some(t) = self.peek() {
                    match t {
                        Token::OpenParen | Token::Function(_) => depth += 1,
                        Token::CloseParen => {
                            depth -= 1;
                            if depth == 0 {
                                let inner = &self.tokens[start..self.pos];
                                self.pos += 1;
                                return Some(SupportsCondition::Selector(
                                    serialize_tokens(trim_ws(inner)),
                                ));
                            }
                        }
                        _ => {}
                    }
                    self.pos += 1;
                }
                return None;
            }
        }
        let inner = trim_ws(self.consume_paren_inner()?);
        if is_nested_condition(inner) {
            let mut sub = CondParser::new(inner);
            sub.parse_supports_expr()
        } else {
            parse_supports_declaration(inner)
        }
    }
}

fn is_nested_condition(inner: &[Token]) -> bool {
    match inner.first() {
        Some(Token::OpenParen) => true,
        Some(Token::Ident(s)) if s.eq_ignore_ascii_case("not") => true,
        Some(Token::Function(s)) if s.eq_ignore_ascii_case("selector") => true,
        _ => false,
    }
}

fn parse_supports_declaration(tokens: &[Token]) -> Option<SupportsCondition> {
    let mut i = 0;
    while matches!(tokens.get(i), Some(Token::Whitespace)) {
        i += 1;
    }
    let prop = match tokens.get(i) {
        Some(Token::Ident(n)) => n.clone(),
        _ => return None,
    };
    i += 1;
    while matches!(tokens.get(i), Some(Token::Whitespace)) {
        i += 1;
    }
    if tokens.get(i) != Some(&Token::Colon) {
        return None;
    }
    i += 1;
    let (val, _) = extract_important(&tokens[i..]);
    Some(SupportsCondition::Declaration(prop, val))
}

fn parse_media_length_px(tok: &Token) -> Option<f32> {
    match tok {
        Token::Dimension { value, unit, .. } => match unit.to_ascii_lowercase().as_str() {
            "px" => Some(*value),
            "em" | "rem" => Some(*value * 16.0),
            "pt" => Some(*value * 96.0 / 72.0),
            "cm" => Some(*value * 96.0 / 2.54),
            "mm" => Some(*value * 96.0 / 25.4),
            "in" => Some(*value * 96.0),
            _ => None,
        },
        Token::Number { value, .. } if *value == 0.0 => Some(0.0),
        _ => None,
    }
}

fn parse_media_feature(tokens: &[Token]) -> MediaFeature {
    let non_ws: Vec<&Token> = tokens
        .iter()
        .filter(|t| **t != Token::Whitespace)
        .collect();
    if non_ws.is_empty() {
        return MediaFeature::Unknown(false);
    }

    if let Some(colon_pos) = non_ws.iter().position(|t| **t == Token::Colon) {
        let name = match non_ws.first() {
            Some(Token::Ident(s)) => s.to_ascii_lowercase(),
            _ => return MediaFeature::Unknown(false),
        };
        let val = non_ws.get(colon_pos + 1);
        return match name.as_str() {
            "min-width" => val
                .and_then(|t| parse_media_length_px(t))
                .map(MediaFeature::MinWidth)
                .unwrap_or(MediaFeature::Unknown(false)),
            "max-width" => val
                .and_then(|t| parse_media_length_px(t))
                .map(MediaFeature::MaxWidth)
                .unwrap_or(MediaFeature::Unknown(false)),
            "width" => val
                .and_then(|t| parse_media_length_px(t))
                .map(|v| MediaFeature::Width(RangeOp::Eq, v))
                .unwrap_or(MediaFeature::Unknown(false)),
            "min-height" => val
                .and_then(|t| parse_media_length_px(t))
                .map(MediaFeature::MinHeight)
                .unwrap_or(MediaFeature::Unknown(false)),
            "max-height" => val
                .and_then(|t| parse_media_length_px(t))
                .map(MediaFeature::MaxHeight)
                .unwrap_or(MediaFeature::Unknown(false)),
            "height" => val
                .and_then(|t| parse_media_length_px(t))
                .map(|v| MediaFeature::Height(RangeOp::Eq, v))
                .unwrap_or(MediaFeature::Unknown(false)),
            "min-color" => match val {
                Some(Token::Number {
                    value,
                    kind: crate::tokenizer::NumericKind::Integer,
                    ..
                }) if *value >= 0.0 => MediaFeature::MinColor(*value as i32),
                _ => MediaFeature::Invalid,
            },
            "max-color" => match val {
                Some(Token::Number {
                    value,
                    kind: crate::tokenizer::NumericKind::Integer,
                    ..
                }) if *value >= 0.0 => MediaFeature::MaxColor(*value as i32),
                _ => MediaFeature::Invalid,
            },
            "color" => match val {
                Some(Token::Number {
                    value,
                    kind: crate::tokenizer::NumericKind::Integer,
                    ..
                }) if *value >= 0.0 => MediaFeature::Color(*value as i32),
                _ => MediaFeature::Invalid,
            },
            "min-monochrome" => match val {
                Some(Token::Number {
                    value,
                    kind: crate::tokenizer::NumericKind::Integer,
                    ..
                }) if *value >= 0.0 => MediaFeature::MinMonochrome(*value as i32),
                _ => MediaFeature::Invalid,
            },
            "max-monochrome" => match val {
                Some(Token::Number {
                    value,
                    kind: crate::tokenizer::NumericKind::Integer,
                    ..
                }) if *value >= 0.0 => MediaFeature::MaxMonochrome(*value as i32),
                _ => MediaFeature::Invalid,
            },
            "monochrome" => match val {
                Some(Token::Number {
                    value,
                    kind: crate::tokenizer::NumericKind::Integer,
                    ..
                }) if *value >= 0.0 => MediaFeature::Monochrome(*value as i32),
                _ => MediaFeature::Invalid,
            },
            "prefers-color-scheme" => match val {
                Some(Token::Ident(s)) if s.eq_ignore_ascii_case("dark") => {
                    MediaFeature::PrefersColorScheme(true)
                }
                Some(Token::Ident(s)) if s.eq_ignore_ascii_case("light") => {
                    MediaFeature::PrefersColorScheme(false)
                }
                _ => MediaFeature::Invalid,
            },
            "orientation" => match val {
                Some(Token::Ident(s)) if s.eq_ignore_ascii_case("landscape") => {
                    MediaFeature::OrientationLandscape
                }
                Some(Token::Ident(s)) if s.eq_ignore_ascii_case("portrait") => {
                    MediaFeature::OrientationPortrait
                }
                _ => MediaFeature::Invalid,
            },
            "hover" | "any-hover" => match val {
                Some(Token::Ident(s)) if s.eq_ignore_ascii_case("hover") => {
                    MediaFeature::Hover(true)
                }
                Some(Token::Ident(s)) if s.eq_ignore_ascii_case("none") => {
                    MediaFeature::Hover(false)
                }
                _ => MediaFeature::Invalid,
            },
            "pointer" | "any-pointer" => match val {
                Some(Token::Ident(s)) => MediaFeature::Pointer(s.clone()),
                _ => MediaFeature::Invalid,
            },
            "forced-colors" => match val {
                Some(Token::Ident(s)) if s.eq_ignore_ascii_case("none") => {
                    MediaFeature::Unknown(true)
                }
                Some(Token::Ident(s)) if s.eq_ignore_ascii_case("active") => {
                    MediaFeature::Unknown(false)
                }
                _ => MediaFeature::Invalid,
            },
            "prefers-reduced-motion" => match val {
                Some(Token::Ident(s)) if s.eq_ignore_ascii_case("no-preference") => {
                    MediaFeature::Unknown(true)
                }
                Some(Token::Ident(s)) if s.eq_ignore_ascii_case("reduce") => {
                    MediaFeature::Unknown(false)
                }
                _ => MediaFeature::Invalid,
            },
            _ => MediaFeature::Invalid,
        };
    }

    if let Some(range) = parse_media_range(&non_ws) {
        return range;
    }

    if non_ws.len() == 1 {
        if let Token::Ident(s) = non_ws[0] {
            return match s.to_ascii_lowercase().as_str() {
                "color" | "hover" | "pointer" => MediaFeature::Unknown(true),
                "monochrome" => MediaFeature::Unknown(false),
                _ => MediaFeature::Invalid,
            };
        }
    }

    MediaFeature::Invalid
}

fn read_range_op(tokens: &[&Token], idx: usize) -> Option<(RangeOp, usize)> {
    match tokens.get(idx) {
        Some(Token::Delim('<')) => {
            if matches!(tokens.get(idx + 1), Some(Token::Delim('='))) {
                Some((RangeOp::Le, idx + 2))
            } else {
                Some((RangeOp::Lt, idx + 1))
            }
        }
        Some(Token::Delim('>')) => {
            if matches!(tokens.get(idx + 1), Some(Token::Delim('='))) {
                Some((RangeOp::Ge, idx + 2))
            } else {
                Some((RangeOp::Gt, idx + 1))
            }
        }
        Some(Token::Delim('=')) => Some((RangeOp::Eq, idx + 1)),
        _ => None,
    }
}

fn flip_op(op: RangeOp) -> RangeOp {
    match op {
        RangeOp::Lt => RangeOp::Gt,
        RangeOp::Le => RangeOp::Ge,
        RangeOp::Eq => RangeOp::Eq,
        RangeOp::Ge => RangeOp::Le,
        RangeOp::Gt => RangeOp::Lt,
    }
}

fn parse_media_range(tokens: &[&Token]) -> Option<MediaFeature> {
    if let Some(Token::Ident(name)) = tokens.first() {
        let lower = name.to_ascii_lowercase();
        let (op, next) = read_range_op(tokens, 1)?;
        let val = parse_media_length_px(tokens.get(next)?)?;
        return match lower.as_str() {
            "width" => Some(MediaFeature::Width(op, val)),
            "height" => Some(MediaFeature::Height(op, val)),
            _ => None,
        };
    }

    let first_val = parse_media_length_px(tokens.first()?)?;
    let (op1, name_idx) = read_range_op(tokens, 1)?;
    let name = match tokens.get(name_idx) {
        Some(Token::Ident(s)) => s.to_ascii_lowercase(),
        _ => return None,
    };
    if let Some((op2, second_idx)) = read_range_op(tokens, name_idx + 1) {
        let second_val = parse_media_length_px(tokens.get(second_idx)?)?;
        return match name.as_str() {
            "width" => Some(MediaFeature::WidthRange(first_val, op1, op2, second_val)),
            "height" => Some(MediaFeature::HeightRange(first_val, op1, op2, second_val)),
            _ => None,
        };
    }
    let flipped = flip_op(op1);
    match name.as_str() {
        "width" => Some(MediaFeature::Width(flipped, first_val)),
        "height" => Some(MediaFeature::Height(flipped, first_val)),
        _ => None,
    }
}
