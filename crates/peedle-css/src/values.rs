use crate::tokenizer::{serialize_tokens, tokenize, Token};
use std::cell::Cell;

thread_local! {
    static ACTIVE_DARK_MODE: Cell<bool> = const { Cell::new(false) };
}

pub fn set_active_dark_mode(dark: bool) -> bool {
    ACTIVE_DARK_MODE.with(|c| {
        let prev = c.get();
        c.set(dark);
        prev
    })
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const TRANSPARENT: Self = Self {
        r: 0,
        g: 0,
        b: 0,
        a: 0,
    };
    pub const BLACK: Self = Self {
        r: 0,
        g: 0,
        b: 0,
        a: 255,
    };
    pub const WHITE: Self = Self {
        r: 255,
        g: 255,
        b: 255,
        a: 255,
    };

    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    pub fn to_css_string(self) -> String {
        if self.a == 255 {
            format!("rgb({}, {}, {})", self.r, self.g, self.b)
        } else {
            let alpha = (self.a as f32 / 255.0 * 1000.0).round() / 1000.0;
            format!("rgba({}, {}, {}, {})", self.r, self.g, self.b, alpha)
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorValue {
    Color(Color),
    CurrentColor,
}

impl ColorValue {
    pub fn resolve(self, current: Color) -> Color {
        match self {
            ColorValue::Color(c) => c,
            ColorValue::CurrentColor => current,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LengthUnit {
    Px,
    Em,
    Rem,
    Vw,
    Vh,
    Vmin,
    Vmax,
    Ch,
    Ex,
    Pt,
    Pc,
    Cm,
    Mm,
    In,
    Percent,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Length {
    pub value: f32,
    pub unit: LengthUnit,
}

impl Length {
    pub const ZERO: Self = Self {
        value: 0.0,
        unit: LengthUnit::Px,
    };

    pub const fn px(value: f32) -> Self {
        Self {
            value,
            unit: LengthUnit::Px,
        }
    }

    pub fn to_px(self, ctx: &CalcContext) -> f32 {
        match self.unit {
            LengthUnit::Px => self.value,
            LengthUnit::Em => self.value * ctx.font_size,
            LengthUnit::Rem => self.value * ctx.root_font_size,
            LengthUnit::Vw => self.value * ctx.viewport_width / 100.0,
            LengthUnit::Vh => self.value * ctx.viewport_height / 100.0,
            LengthUnit::Vmin => self.value * ctx.viewport_width.min(ctx.viewport_height) / 100.0,
            LengthUnit::Vmax => self.value * ctx.viewport_width.max(ctx.viewport_height) / 100.0,
            LengthUnit::Ch => self.value * ctx.font_size * 0.5,
            LengthUnit::Ex => self.value * ctx.font_size * 0.5,
            LengthUnit::Pt => self.value * (96.0 / 72.0),
            LengthUnit::Pc => self.value * 16.0,
            LengthUnit::Cm => self.value * (96.0 / 2.54),
            LengthUnit::Mm => self.value * (96.0 / 25.4),
            LengthUnit::In => self.value * 96.0,
            LengthUnit::Percent => self.value * ctx.percentage_base / 100.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CalcContext {
    pub font_size: f32,
    pub root_font_size: f32,
    pub viewport_width: f32,
    pub viewport_height: f32,
    pub percentage_base: f32,
}

impl Default for CalcContext {
    fn default() -> Self {
        Self {
            font_size: 16.0,
            root_font_size: 16.0,
            viewport_width: 1280.0,
            viewport_height: 720.0,
            percentage_base: 0.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum CalcExpr {
    Number(f32),
    Length(Length),
    Add(Box<CalcExpr>, Box<CalcExpr>),
    Sub(Box<CalcExpr>, Box<CalcExpr>),
    Mul(Box<CalcExpr>, Box<CalcExpr>),
    Div(Box<CalcExpr>, Box<CalcExpr>),
    Min(Vec<CalcExpr>),
    Max(Vec<CalcExpr>),
    Clamp(Box<CalcExpr>, Box<CalcExpr>, Box<CalcExpr>),
}

impl CalcExpr {
    pub fn evaluate(&self, ctx: &CalcContext) -> f32 {
        match self {
            CalcExpr::Number(n) => *n,
            CalcExpr::Length(l) => l.to_px(ctx),
            CalcExpr::Add(a, b) => a.evaluate(ctx) + b.evaluate(ctx),
            CalcExpr::Sub(a, b) => a.evaluate(ctx) - b.evaluate(ctx),
            CalcExpr::Mul(a, b) => a.evaluate(ctx) * b.evaluate(ctx),
            CalcExpr::Div(a, b) => {
                let d = b.evaluate(ctx);
                if d.abs() < 1e-12 {
                    0.0
                } else {
                    a.evaluate(ctx) / d
                }
            }
            CalcExpr::Min(items) => items
                .iter()
                .map(|x| x.evaluate(ctx))
                .fold(f32::INFINITY, f32::min),
            CalcExpr::Max(items) => items
                .iter()
                .map(|x| x.evaluate(ctx))
                .fold(f32::NEG_INFINITY, f32::max),
            CalcExpr::Clamp(min_e, val_e, max_e) => {
                let lo = min_e.evaluate(ctx);
                let mid = val_e.evaluate(ctx);
                let hi = max_e.evaluate(ctx);
                lo.max(mid.min(hi))
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum LengthPercentageAuto {
    Auto,
    Length(Length),
    Percent(f32),
    Calc(CalcExpr),
}

impl Default for LengthPercentageAuto {
    fn default() -> Self {
        Self::Auto
    }
}

impl LengthPercentageAuto {
    pub fn px(v: f32) -> Self {
        Self::Length(Length::px(v))
    }

    pub fn resolve(&self, ctx: &CalcContext) -> Option<f32> {
        match self {
            LengthPercentageAuto::Auto => None,
            LengthPercentageAuto::Length(l) => Some(l.to_px(ctx)),
            LengthPercentageAuto::Percent(p) => Some(p * ctx.percentage_base / 100.0),
            LengthPercentageAuto::Calc(expr) => Some(expr.evaluate(ctx)),
        }
    }

    pub fn resolve_or(&self, ctx: &CalcContext, default: f32) -> f32 {
        self.resolve(ctx).unwrap_or(default)
    }

    pub fn to_css_string(&self) -> String {
        match self {
            LengthPercentageAuto::Auto => "auto".to_string(),
            LengthPercentageAuto::Length(l) => format_px(l.value),
            LengthPercentageAuto::Percent(p) => format!("{}%", trim_float(*p)),
            LengthPercentageAuto::Calc(expr) => {
                format_px(expr.evaluate(&CalcContext::default()))
            }
        }
    }
}

pub fn format_px(v: f32) -> String {
    format!("{}px", trim_float(v))
}

pub fn trim_float(v: f32) -> String {
    if (v - v.round()).abs() < 1e-4 {
        format!("{}", v.round() as i64)
    } else {
        let s = format!("{:.3}", v);
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct GradientStop {
    pub color: Color,
    pub position: Option<f32>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum BackgroundImage {
    None,
    Url(String),
    LinearGradient {
        angle_deg: f32,
        repeating: bool,
        stops: Vec<GradientStop>,
    },
    RadialGradient {
        shape: String,
        repeating: bool,
        stops: Vec<GradientStop>,
    },
    ConicGradient {
        from_deg: f32,
        repeating: bool,
        stops: Vec<GradientStop>,
    },
}

impl Default for BackgroundImage {
    fn default() -> Self {
        Self::None
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct BoxShadow {
    pub offset_x: f32,
    pub offset_y: f32,
    pub blur_radius: f32,
    pub spread_radius: f32,
    pub color: Color,
    pub inset: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TextShadow {
    pub offset_x: f32,
    pub offset_y: f32,
    pub blur_radius: f32,
    pub color: Color,
}

#[derive(Clone, Debug, PartialEq)]
pub enum TransformOp {
    Translate(LengthPercentageAuto, LengthPercentageAuto),
    TranslateX(LengthPercentageAuto),
    TranslateY(LengthPercentageAuto),
    Scale(f32, f32),
    ScaleX(f32),
    ScaleY(f32),
    Rotate(f32),
    Skew(f32, f32),
    SkewX(f32),
    SkewY(f32),
    Matrix(f32, f32, f32, f32, f32, f32),
}

#[derive(Clone, Debug, PartialEq)]
pub enum FilterOp {
    Blur(f32),
    Brightness(f32),
    Contrast(f32),
    Grayscale(f32),
    HueRotate(f32),
    Invert(f32),
    Opacity(f32),
    Saturate(f32),
    Sepia(f32),
    DropShadow(BoxShadow),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GridRepeatCount {
    Count(u32),
    AutoFill,
    AutoFit,
}

#[derive(Clone, Debug, PartialEq)]
pub enum GridTrackSize {
    Px(f32),
    Percent(f32),
    Fr(f32),
    Auto,
    MinContent,
    MaxContent,
    MinMax(Box<GridTrackSize>, Box<GridTrackSize>),
    FitContent(f32),
    Repeat(GridRepeatCount, Vec<GridTrackSize>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GridPlacement {
    Auto,
    Line(i32),
    Span(u32),
}

impl Default for GridPlacement {
    fn default() -> Self {
        Self::Auto
    }
}

pub fn parse_length_unit(unit: &str) -> Option<LengthUnit> {
    match unit.to_ascii_lowercase().as_str() {
        "px" => Some(LengthUnit::Px),
        "em" => Some(LengthUnit::Em),
        "rem" => Some(LengthUnit::Rem),
        "vw" => Some(LengthUnit::Vw),
        "vh" => Some(LengthUnit::Vh),
        "vmin" => Some(LengthUnit::Vmin),
        "vmax" => Some(LengthUnit::Vmax),
        "ch" => Some(LengthUnit::Ch),
        "ex" => Some(LengthUnit::Ex),
        "pt" => Some(LengthUnit::Pt),
        "pc" => Some(LengthUnit::Pc),
        "cm" => Some(LengthUnit::Cm),
        "mm" => Some(LengthUnit::Mm),
        "in" => Some(LengthUnit::In),
        _ => None,
    }
}

pub fn parse_angle_deg(tok: &Token) -> Option<f32> {
    match tok {
        Token::Dimension { value, unit, .. } => match unit.to_ascii_lowercase().as_str() {
            "deg" => Some(*value),
            "rad" => Some(value.to_degrees()),
            "grad" => Some(*value * 0.9),
            "turn" => Some(*value * 360.0),
            _ => None,
        },
        Token::Number { value, .. } => Some(*value),
        _ => None,
    }
}

pub fn parse_length_percentage_auto(tokens: &[Token]) -> Option<LengthPercentageAuto> {
    let trimmed = trim_ws(tokens);
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.len() == 1 {
        return parse_single_lpa(&trimmed[0]);
    }
    if let Some(Token::Function(name)) = trimmed.first() {
        let lower = name.to_ascii_lowercase();
        if matches!(lower.as_str(), "calc" | "min" | "max" | "clamp") {
            let expr = parse_calc_function(trimmed)?;
            return Some(LengthPercentageAuto::Calc(expr));
        }
    }
    None
}

fn parse_single_lpa(tok: &Token) -> Option<LengthPercentageAuto> {
    match tok {
        Token::Ident(s) if s.eq_ignore_ascii_case("auto") => Some(LengthPercentageAuto::Auto),
        Token::Dimension { value, unit, .. } => {
            let u = parse_length_unit(unit)?;
            Some(LengthPercentageAuto::Length(Length {
                value: *value,
                unit: u,
            }))
        }
        Token::Percentage { value, .. } => Some(LengthPercentageAuto::Percent(*value)),
        Token::Number { value, .. } if *value == 0.0 => {
            Some(LengthPercentageAuto::Length(Length::ZERO))
        }
        _ => None,
    }
}

pub fn parse_calc_expr(input: &str) -> Option<CalcExpr> {
    let tokens = tokenize(input);
    parse_calc_function(trim_ws(&tokens))
}

pub fn parse_calc_function(tokens: &[Token]) -> Option<CalcExpr> {
    let mut p = CalcParser::new(tokens);
    let expr = p.parse_expr()?;
    p.skip_ws();
    if p.pos == tokens.len() {
        Some(expr)
    } else {
        None
    }
}

struct CalcParser<'a> {
    tokens: &'a [Token],
    pos: usize,
}

impl<'a> CalcParser<'a> {
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

    fn consume_paren_slice(&mut self) -> Option<&'a [Token]> {
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

    fn parse_expr(&mut self) -> Option<CalcExpr> {
        self.parse_sum()
    }

    fn parse_sum(&mut self) -> Option<CalcExpr> {
        let mut lhs = self.parse_product()?;
        loop {
            self.skip_ws();
            match self.peek() {
                Some(Token::Delim('+')) => {
                    self.pos += 1;
                    let rhs = self.parse_product()?;
                    lhs = CalcExpr::Add(Box::new(lhs), Box::new(rhs));
                }
                Some(Token::Delim('-')) => {
                    self.pos += 1;
                    let rhs = self.parse_product()?;
                    lhs = CalcExpr::Sub(Box::new(lhs), Box::new(rhs));
                }
                _ => break,
            }
        }
        Some(lhs)
    }

    fn parse_product(&mut self) -> Option<CalcExpr> {
        let mut lhs = self.parse_primary()?;
        loop {
            self.skip_ws();
            match self.peek() {
                Some(Token::Delim('*')) => {
                    self.pos += 1;
                    let rhs = self.parse_primary()?;
                    lhs = CalcExpr::Mul(Box::new(lhs), Box::new(rhs));
                }
                Some(Token::Delim('/')) => {
                    self.pos += 1;
                    let rhs = self.parse_primary()?;
                    lhs = CalcExpr::Div(Box::new(lhs), Box::new(rhs));
                }
                _ => break,
            }
        }
        Some(lhs)
    }

    fn parse_primary(&mut self) -> Option<CalcExpr> {
        self.skip_ws();
        match self.peek()? {
            Token::Number { value, .. } => {
                let v = *value;
                self.pos += 1;
                Some(CalcExpr::Number(v))
            }
            Token::Percentage { value, .. } => {
                let v = *value;
                self.pos += 1;
                Some(CalcExpr::Length(Length {
                    value: v,
                    unit: LengthUnit::Percent,
                }))
            }
            Token::Dimension { value, unit, .. } => {
                let v = *value;
                let u = parse_length_unit(unit)?;
                self.pos += 1;
                Some(CalcExpr::Length(Length { value: v, unit: u }))
            }
            Token::OpenParen => {
                self.pos += 1;
                let inner = self.consume_paren_slice()?;
                let mut sub = CalcParser::new(inner);
                sub.parse_expr()
            }
            Token::Function(name) => {
                let lower = name.to_ascii_lowercase();
                self.pos += 1;
                let inner = self.consume_paren_slice()?;
                match lower.as_str() {
                    "calc" => {
                        let mut sub = CalcParser::new(inner);
                        sub.parse_expr()
                    }
                    "min" => {
                        let args = parse_calc_args(inner)?;
                        if args.is_empty() {
                            None
                        } else {
                            Some(CalcExpr::Min(args))
                        }
                    }
                    "max" => {
                        let args = parse_calc_args(inner)?;
                        if args.is_empty() {
                            None
                        } else {
                            Some(CalcExpr::Max(args))
                        }
                    }
                    "clamp" => {
                        let mut args = parse_calc_args(inner)?;
                        if args.len() != 3 {
                            return None;
                        }
                        let hi = args.pop()?;
                        let mid = args.pop()?;
                        let lo = args.pop()?;
                        Some(CalcExpr::Clamp(Box::new(lo), Box::new(mid), Box::new(hi)))
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    }
}

fn parse_calc_args(tokens: &[Token]) -> Option<Vec<CalcExpr>> {
    let mut out = Vec::new();
    for part in split_commas(tokens) {
        let trimmed = trim_ws(part);
        if trimmed.is_empty() {
            return None;
        }
        let mut p = CalcParser::new(trimmed);
        out.push(p.parse_expr()?);
    }
    Some(out)
}

pub fn parse_color(input: &str) -> Option<Color> {
    let tokens = tokenize(input);
    match parse_color_value(&tokens)? {
        ColorValue::Color(c) => Some(c),
        ColorValue::CurrentColor => None,
    }
}

pub fn parse_color_value(tokens: &[Token]) -> Option<ColorValue> {
    let trimmed = trim_ws(tokens);
    if trimmed.is_empty() {
        return None;
    }
    match &trimmed[0] {
        Token::Ident(name) if trimmed.len() == 1 => {
            let lower = name.to_ascii_lowercase();
            if lower == "transparent" {
                return Some(ColorValue::Color(Color::TRANSPARENT));
            }
            if lower == "currentcolor" {
                return Some(ColorValue::CurrentColor);
            }
            lookup_named_color(&lower).map(ColorValue::Color)
        }
        Token::Hash { value, .. } if trimmed.len() == 1 => {
            parse_hex_color(value).map(ColorValue::Color)
        }
        Token::Function(name) => {
            let lower = name.to_ascii_lowercase();
            let inner = extract_single_fn_body(trimmed)?;
            let c = match lower.as_str() {
                "rgb" | "rgba" => parse_rgb_fn(inner)?,
                "hsl" | "hsla" => parse_hsl_fn(inner)?,
                "hwb" => parse_hwb_fn(inner)?,
                "oklab" => parse_oklab_fn(inner)?,
                "oklch" => parse_oklch_fn(inner)?,
                "lab" => parse_lab_fn(inner)?,
                "lch" => parse_lch_fn(inner)?,
                "color-mix" => parse_color_mix_fn(inner)?,
                "light-dark" => {
                    let parts = split_commas(inner);
                    if parts.len() >= 2 {
                        let pick = if ACTIVE_DARK_MODE.with(|c| c.get()) {
                            parts[1]
                        } else {
                            parts[0]
                        };
                        parse_color_from_tokens(pick, Color::BLACK)?
                    } else {
                        return None;
                    }
                }
                _ => return None,
            };
            Some(ColorValue::Color(c))
        }
        _ => None,
    }
}

pub fn parse_color_from_tokens(tokens: &[Token], current: Color) -> Option<Color> {
    parse_color_value(tokens).map(|cv| cv.resolve(current))
}

fn extract_single_fn_body(tokens: &[Token]) -> Option<&[Token]> {
    if !matches!(tokens.first(), Some(Token::Function(_))) {
        return None;
    }
    let mut depth = 1usize;
    for (i, t) in tokens.iter().enumerate().skip(1) {
        match t {
            Token::OpenParen | Token::Function(_) => depth += 1,
            Token::CloseParen => {
                depth -= 1;
                if depth == 0 {
                    if i + 1 == tokens.len() {
                        return Some(&tokens[1..i]);
                    }
                    return None;
                }
            }
            _ => {}
        }
    }
    None
}

fn parse_hex_color(hex: &str) -> Option<Color> {
    let bytes = hex.as_bytes();
    let hex_digit = |b: u8| -> Option<u8> {
        match b {
            b'0'..=b'9' => Some(b - b'0'),
            b'a'..=b'f' => Some(b - b'a' + 10),
            b'A'..=b'F' => Some(b - b'A' + 10),
            _ => None,
        }
    };
    let pair = |a: u8, b: u8| -> Option<u8> { Some((hex_digit(a)? << 4) | hex_digit(b)?) };
    match bytes.len() {
        3 => {
            let r = hex_digit(bytes[0])? * 17;
            let g = hex_digit(bytes[1])? * 17;
            let b = hex_digit(bytes[2])? * 17;
            Some(Color::rgb(r, g, b))
        }
        4 => {
            let r = hex_digit(bytes[0])? * 17;
            let g = hex_digit(bytes[1])? * 17;
            let b = hex_digit(bytes[2])? * 17;
            let a = hex_digit(bytes[3])? * 17;
            Some(Color::rgba(r, g, b, a))
        }
        6 => Some(Color::rgb(
            pair(bytes[0], bytes[1])?,
            pair(bytes[2], bytes[3])?,
            pair(bytes[4], bytes[5])?,
        )),
        8 => Some(Color::rgba(
            pair(bytes[0], bytes[1])?,
            pair(bytes[2], bytes[3])?,
            pair(bytes[4], bytes[5])?,
            pair(bytes[6], bytes[7])?,
        )),
        _ => None,
    }
}

fn split_color_args(tokens: &[Token]) -> (Vec<&Token>, Option<&Token>) {
    let non_ws: Vec<&Token> = tokens
        .iter()
        .filter(|t| !matches!(t, Token::Whitespace | Token::Comma))
        .collect();
    if let Some(slash) = non_ws.iter().position(|t| **t == Token::Delim('/')) {
        let channels = non_ws[..slash].to_vec();
        let alpha = non_ws.get(slash + 1).copied();
        (channels, alpha)
    } else if non_ws.len() == 4 {
        (non_ws[..3].to_vec(), Some(non_ws[3]))
    } else {
        (non_ws, None)
    }
}

fn parse_alpha_token(tok: Option<&Token>) -> u8 {
    let Some(t) = tok else {
        return 255;
    };
    let f = match t {
        Token::Number { value, .. } => *value,
        Token::Percentage { value, .. } => *value / 100.0,
        _ => 1.0,
    };
    (f.clamp(0.0, 1.0) * 255.0).round() as u8
}

fn parse_rgb_channel(tok: &Token) -> Option<u8> {
    let v = match tok {
        Token::Number { value, .. } => *value,
        Token::Percentage { value, .. } => *value * 2.55,
        Token::Ident(s) if s.eq_ignore_ascii_case("none") => 0.0,
        _ => return None,
    };
    Some(v.clamp(0.0, 255.0).round() as u8)
}

fn parse_rgb_fn(inner: &[Token]) -> Option<Color> {
    let (ch, alpha) = split_color_args(inner);
    if ch.len() != 3 {
        return None;
    }
    let r = parse_rgb_channel(ch[0])?;
    let g = parse_rgb_channel(ch[1])?;
    let b = parse_rgb_channel(ch[2])?;
    let a = parse_alpha_token(alpha);
    Some(Color::rgba(r, g, b, a))
}

fn parse_percent_or_number(tok: &Token, max_val: f32) -> Option<f32> {
    match tok {
        Token::Percentage { value, .. } => Some((*value / 100.0) * max_val),
        Token::Number { value, .. } => Some(*value),
        Token::Ident(s) if s.eq_ignore_ascii_case("none") => Some(0.0),
        _ => None,
    }
}

fn parse_hsl_fn(inner: &[Token]) -> Option<Color> {
    let legacy_commas = inner.iter().any(|t| matches!(t, Token::Comma));
    let (ch, alpha) = split_color_args(inner);
    if ch.len() != 3 {
        return None;
    }
    if legacy_commas
        && (!matches!(ch[1], Token::Percentage { .. })
            || !matches!(ch[2], Token::Percentage { .. }))
    {
        return None;
    }
    let h = parse_angle_deg(ch[0])?.rem_euclid(360.0);
    let s = parse_percent_or_number(ch[1], 100.0)?.clamp(0.0, 100.0) / 100.0;
    let l = parse_percent_or_number(ch[2], 100.0)?.clamp(0.0, 100.0) / 100.0;
    let (r, g, b) = hsl_to_rgb(h, s, l);
    let a = parse_alpha_token(alpha);
    Some(Color::rgba(r, g, b, a))
}

fn hsl_to_rgb(h: f32, s: f32, l: f32) -> (u8, u8, u8) {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let hp = h / 60.0;
    let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
    let (r1, g1, b1) = match hp as i32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    (
        ((r1 + m).clamp(0.0, 1.0) * 255.0).round() as u8,
        ((g1 + m).clamp(0.0, 1.0) * 255.0).round() as u8,
        ((b1 + m).clamp(0.0, 1.0) * 255.0).round() as u8,
    )
}

fn parse_hwb_fn(inner: &[Token]) -> Option<Color> {
    let (ch, alpha) = split_color_args(inner);
    if ch.len() != 3 {
        return None;
    }
    let h = parse_angle_deg(ch[0])?.rem_euclid(360.0);
    let w = parse_percent_or_number(ch[1], 100.0)?.max(0.0) / 100.0;
    let bl = parse_percent_or_number(ch[2], 100.0)?.max(0.0) / 100.0;
    if w + bl >= 1.0 {
        let gray = (w / (w + bl) * 255.0).round() as u8;
        return Some(Color::rgba(gray, gray, gray, parse_alpha_token(alpha)));
    }
    let (r0, g0, b0) = hsl_to_rgb(h, 1.0, 0.5);
    let scale = 1.0 - w - bl;
    let mix = |c: u8| (((c as f32 / 255.0) * scale + w).clamp(0.0, 1.0) * 255.0).round() as u8;
    Some(Color::rgba(mix(r0), mix(g0), mix(b0), parse_alpha_token(alpha)))
}

fn linear_to_srgb_byte(x: f32) -> u8 {
    let clamped = x.clamp(0.0, 1.0);
    let s = if clamped <= 0.003_130_8 {
        12.92 * clamped
    } else {
        1.055 * clamped.powf(1.0 / 2.4) - 0.055
    };
    (s.clamp(0.0, 1.0) * 255.0).round() as u8
}

fn srgb_byte_to_linear(c: u8) -> f32 {
    let v = c as f32 / 255.0;
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

fn oklab_to_srgb(l: f32, a: f32, b: f32) -> (u8, u8, u8) {
    let l_ = l + 0.396_337_78 * a + 0.215_803_76 * b;
    let m_ = l - 0.105_561_35 * a - 0.063_854_17 * b;
    let s_ = l - 0.089_484_18 * a - 1.291_485_5 * b;

    let l3 = l_ * l_ * l_;
    let m3 = m_ * m_ * m_;
    let s3 = s_ * s_ * s_;

    let r = 4.076_741_7 * l3 - 3.307_711_6 * m3 + 0.230_969_94 * s3;
    let g = -1.268_438 * l3 + 2.609_757_4 * m3 - 0.341_319_38 * s3;
    let bl = -0.004_196_086 * l3 - 0.703_418_6 * m3 + 1.707_614_7 * s3;
    (
        linear_to_srgb_byte(r),
        linear_to_srgb_byte(g),
        linear_to_srgb_byte(bl),
    )
}

fn srgb_to_oklab(c: Color) -> (f32, f32, f32) {
    let r = srgb_byte_to_linear(c.r);
    let g = srgb_byte_to_linear(c.g);
    let b = srgb_byte_to_linear(c.b);

    let l = 0.412_221_47 * r + 0.536_332_55 * g + 0.051_445_995 * b;
    let m = 0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b;
    let s = 0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b;

    let l_ = l.cbrt();
    let m_ = m.cbrt();
    let s_ = s.cbrt();

    let ll = 0.210_454_26 * l_ + 0.793_617_8 * m_ - 0.004_072_047 * s_;
    let aa = 1.977_998_5 * l_ - 2.428_592_2 * m_ + 0.450_593_7 * s_;
    let bb = 0.025_904_037 * l_ + 0.782_771_77 * m_ - 0.808_675_77 * s_;
    (ll, aa, bb)
}

fn parse_oklab_fn(inner: &[Token]) -> Option<Color> {
    let (ch, alpha) = split_color_args(inner);
    if ch.len() != 3 {
        return None;
    }
    let l = parse_percent_or_number(ch[0], 1.0)?;
    let a = parse_percent_or_number(ch[1], 0.4)?;
    let b = parse_percent_or_number(ch[2], 0.4)?;
    let (r, g, bl) = oklab_to_srgb(l, a, b);
    Some(Color::rgba(r, g, bl, parse_alpha_token(alpha)))
}

fn parse_oklch_fn(inner: &[Token]) -> Option<Color> {
    let (ch, alpha) = split_color_args(inner);
    if ch.len() != 3 {
        return None;
    }
    let l = parse_percent_or_number(ch[0], 1.0)?;
    let c = parse_percent_or_number(ch[1], 0.4)?.max(0.0);
    let h_deg = parse_angle_deg(ch[2])?;
    let h_rad = h_deg.to_radians();
    let (r, g, bl) = oklab_to_srgb(l, c * h_rad.cos(), c * h_rad.sin());
    Some(Color::rgba(r, g, bl, parse_alpha_token(alpha)))
}

fn lab_to_srgb(l: f32, a: f32, b: f32) -> (u8, u8, u8) {
    let fy = (l + 16.0) / 116.0;
    let fx = a / 500.0 + fy;
    let fz = fy - b / 200.0;
    let eps = 216.0 / 24389.0;
    let kappa = 24389.0 / 27.0;
    let xr = if fx * fx * fx > eps {
        fx * fx * fx
    } else {
        (116.0 * fx - 16.0) / kappa
    };
    let yr = if l > kappa * eps {
        fy * fy * fy
    } else {
        l / kappa
    };
    let zr = if fz * fz * fz > eps {
        fz * fz * fz
    } else {
        (116.0 * fz - 16.0) / kappa
    };
    let x50 = xr * 0.964_22;
    let y50 = yr * 1.0;
    let z50 = zr * 0.825_21;

    let x65 = 0.955_576_6 * x50 - 0.023_039_3 * y50 + 0.063_163_6 * z50;
    let y65 = -0.028_289_5 * x50 + 1.009_941_6 * y50 + 0.021_007_7 * z50;
    let z65 = 0.012_298_2 * x50 - 0.020_483_0 * y50 + 1.329_909_8 * z50;

    let r = 3.240_454_2 * x65 - 1.537_138_5 * y65 - 0.498_531_4 * z65;
    let g = -0.969_266_0 * x65 + 1.876_010_8 * y65 + 0.041_556_0 * z65;
    let bl = 0.055_643_4 * x65 - 0.204_025_9 * y65 + 1.057_225_2 * z65;

    (
        linear_to_srgb_byte(r),
        linear_to_srgb_byte(g),
        linear_to_srgb_byte(bl),
    )
}

fn parse_lab_fn(inner: &[Token]) -> Option<Color> {
    let (ch, alpha) = split_color_args(inner);
    if ch.len() != 3 {
        return None;
    }
    let l = parse_percent_or_number(ch[0], 100.0)?;
    let a = parse_percent_or_number(ch[1], 125.0)?;
    let b = parse_percent_or_number(ch[2], 125.0)?;
    let (r, g, bl) = lab_to_srgb(l, a, b);
    Some(Color::rgba(r, g, bl, parse_alpha_token(alpha)))
}

fn parse_lch_fn(inner: &[Token]) -> Option<Color> {
    let (ch, alpha) = split_color_args(inner);
    if ch.len() != 3 {
        return None;
    }
    let l = parse_percent_or_number(ch[0], 100.0)?;
    let c = parse_percent_or_number(ch[1], 150.0)?.max(0.0);
    let h_rad = parse_angle_deg(ch[2])?.to_radians();
    let (r, g, bl) = lab_to_srgb(l, c * h_rad.cos(), c * h_rad.sin());
    Some(Color::rgba(r, g, bl, parse_alpha_token(alpha)))
}

fn parse_color_mix_fn(inner: &[Token]) -> Option<Color> {
    let parts = split_commas(inner);
    if parts.len() != 3 {
        return None;
    }
    let space_str = serialize_tokens(trim_ws(parts[0])).to_ascii_lowercase();
    let use_oklch = space_str.contains("oklch");
    let use_oklab = space_str.contains("oklab");

    let (c1, p1_opt) = parse_mix_component(trim_ws(parts[1]))?;
    let (c2, p2_opt) = parse_mix_component(trim_ws(parts[2]))?;

    let (mut p1, mut p2) = match (p1_opt, p2_opt) {
        (None, None) => (0.5, 0.5),
        (Some(a), None) => (a, 1.0 - a),
        (None, Some(b)) => (1.0 - b, b),
        (Some(a), Some(b)) => (a, b),
    };
    let sum = p1 + p2;
    if sum <= 0.0 {
        return None;
    }
    let alpha_multiplier = if sum < 1.0 { sum } else { 1.0 };
    if sum != 1.0 {
        p1 /= sum;
        p2 /= sum;
    }

    let a1 = c1.a as f32 / 255.0;
    let a2 = c2.a as f32 / 255.0;
    let out_a = ((a1 * p1 + a2 * p2) * alpha_multiplier).clamp(0.0, 1.0);

    if use_oklch {
        let (l1, a1o, b1o) = srgb_to_oklab(c1);
        let (l2, a2o, b2o) = srgb_to_oklab(c2);
        let c1_ch = (a1o * a1o + b1o * b1o).sqrt();
        let c2_ch = (a2o * a2o + b2o * b2o).sqrt();
        let h1 = b1o.atan2(a1o).to_degrees().rem_euclid(360.0);
        let h2 = b2o.atan2(a2o).to_degrees().rem_euclid(360.0);
        let mut dh = h2 - h1;
        if dh > 180.0 {
            dh -= 360.0;
        } else if dh < -180.0 {
            dh += 360.0;
        }
        let h = if c1_ch < 1e-4 {
            h2
        } else if c2_ch < 1e-4 {
            h1
        } else {
            (h1 + dh * p2).rem_euclid(360.0)
        };
        let l = l1 * p1 + l2 * p2;
        let c = c1_ch * p1 + c2_ch * p2;
        let hr = h.to_radians();
        let (r, g, b) = oklab_to_srgb(l, c * hr.cos(), c * hr.sin());
        return Some(Color::rgba(r, g, b, (out_a * 255.0).round() as u8));
    }

    if use_oklab {
        let (l1, a1o, b1o) = srgb_to_oklab(c1);
        let (l2, a2o, b2o) = srgb_to_oklab(c2);
        let (r, g, b) = oklab_to_srgb(
            l1 * p1 + l2 * p2,
            a1o * p1 + a2o * p2,
            b1o * p1 + b2o * p2,
        );
        return Some(Color::rgba(r, g, b, (out_a * 255.0).round() as u8));
    }

    let mix_ch = |v1: u8, v2: u8| -> u8 {
        let denom = a1 * p1 + a2 * p2;
        if denom <= 1e-6 {
            0
        } else {
            (((v1 as f32 * a1 * p1 + v2 as f32 * a2 * p2) / denom).clamp(0.0, 255.0)).round()
                as u8
        }
    };
    Some(Color::rgba(
        mix_ch(c1.r, c2.r),
        mix_ch(c1.g, c2.g),
        mix_ch(c1.b, c2.b),
        (out_a * 255.0).round() as u8,
    ))
}

fn parse_mix_component(tokens: &[Token]) -> Option<(Color, Option<f32>)> {
    let mut pct = None;
    let mut color_tokens = Vec::new();
    let mut depth = 0usize;
    for t in tokens {
        match t {
            Token::OpenParen | Token::Function(_) => {
                depth += 1;
                color_tokens.push(t.clone());
            }
            Token::CloseParen => {
                depth = depth.saturating_sub(1);
                color_tokens.push(t.clone());
            }
            Token::Percentage { value, .. } if depth == 0 => {
                pct = Some((*value / 100.0).clamp(0.0, 1.0));
            }
            _ => color_tokens.push(t.clone()),
        }
    }
    let color = match parse_color_value(&color_tokens)? {
        ColorValue::Color(c) => c,
        ColorValue::CurrentColor => Color::BLACK,
    };
    Some((color, pct))
}

fn lookup_named_color(name: &str) -> Option<Color> {
    if let Some((r, g, b)) = named_whites_and_grays(name) {
        return Some(Color::rgb(r, g, b));
    }
    if let Some((r, g, b)) = named_reds_oranges_yellows(name) {
        return Some(Color::rgb(r, g, b));
    }
    if let Some((r, g, b)) = named_browns_and_greens(name) {
        return Some(Color::rgb(r, g, b));
    }
    if let Some((r, g, b)) = named_blues_and_purples(name) {
        return Some(Color::rgb(r, g, b));
    }
    None
}

fn named_whites_and_grays(name: &str) -> Option<(u8, u8, u8)> {
    Some(match name {
        "white" => (255, 255, 255),
        "snow" => (255, 250, 250),
        "honeydew" => (240, 255, 240),
        "mintcream" => (245, 255, 250),
        "azure" => (240, 255, 255),
        "aliceblue" => (240, 248, 255),
        "ghostwhite" => (248, 248, 255),
        "whitesmoke" => (245, 245, 245),
        "seashell" => (255, 245, 238),
        "beige" => (245, 245, 220),
        "oldlace" => (253, 245, 230),
        "floralwhite" => (255, 250, 240),
        "ivory" => (255, 255, 240),
        "antiquewhite" => (250, 235, 215),
        "linen" => (250, 240, 230),
        "lavenderblush" => (255, 240, 245),
        "mistyrose" => (255, 228, 225),
        "black" => (0, 0, 0),
        "dimgray" | "dimgrey" => (105, 105, 105),
        "gray" | "grey" => (128, 128, 128),
        "darkgray" | "darkgrey" => (169, 169, 169),
        "silver" => (192, 192, 192),
        "lightgray" | "lightgrey" => (211, 211, 211),
        "gainsboro" => (220, 220, 220),
        "lightslategray" | "lightslategrey" => (119, 136, 153),
        "slategray" | "slategrey" => (112, 128, 144),
        "darkslategray" | "darkslategrey" => (47, 79, 79),
        _ => return None,
    })
}

fn named_reds_oranges_yellows(name: &str) -> Option<(u8, u8, u8)> {
    Some(match name {
        "red" => (255, 0, 0),
        "darkred" => (139, 0, 0),
        "firebrick" => (178, 34, 34),
        "crimson" => (220, 20, 60),
        "indianred" => (205, 92, 92),
        "lightcoral" => (240, 128, 128),
        "salmon" => (250, 128, 114),
        "darksalmon" => (233, 150, 122),
        "lightsalmon" => (255, 160, 122),
        "pink" => (255, 192, 203),
        "lightpink" => (255, 182, 193),
        "hotpink" => (255, 105, 180),
        "deeppink" => (255, 20, 147),
        "palevioletred" => (219, 112, 147),
        "mediumvioletred" => (199, 21, 133),
        "orange" => (255, 165, 0),
        "darkorange" => (255, 140, 0),
        "coral" => (255, 127, 80),
        "tomato" => (255, 99, 71),
        "orangered" => (255, 69, 0),
        "gold" => (255, 215, 0),
        "yellow" => (255, 255, 0),
        "lightyellow" => (255, 255, 224),
        "lemonchiffon" => (255, 250, 205),
        "lightgoldenrodyellow" => (250, 250, 210),
        "papayawhip" => (255, 239, 213),
        "moccasin" => (255, 228, 181),
        "peachpuff" => (255, 218, 185),
        "palegoldenrod" => (238, 232, 170),
        "khaki" => (240, 230, 140),
        "darkkhaki" => (189, 183, 107),
        "goldenrod" => (218, 165, 32),
        _ => return None,
    })
}

fn named_browns_and_greens(name: &str) -> Option<(u8, u8, u8)> {
    Some(match name {
        "cornsilk" => (255, 248, 220),
        "blanchedalmond" => (255, 235, 205),
        "bisque" => (255, 228, 196),
        "navajowhite" => (255, 222, 173),
        "wheat" => (245, 222, 179),
        "burlywood" => (222, 184, 135),
        "tan" => (210, 180, 140),
        "rosybrown" => (188, 143, 143),
        "sandybrown" => (244, 164, 96),
        "darkgoldenrod" => (184, 134, 11),
        "peru" => (205, 133, 63),
        "chocolate" => (210, 105, 30),
        "saddlebrown" => (139, 69, 19),
        "sienna" => (160, 82, 45),
        "brown" => (165, 42, 42),
        "maroon" => (128, 0, 0),
        "greenyellow" => (173, 255, 47),
        "chartreuse" => (127, 255, 0),
        "lawngreen" => (124, 252, 0),
        "lime" => (0, 255, 0),
        "limegreen" => (50, 205, 50),
        "palegreen" => (152, 251, 152),
        "lightgreen" => (144, 238, 144),
        "mediumspringgreen" => (0, 250, 154),
        "springgreen" => (0, 255, 127),
        "mediumseagreen" => (60, 179, 113),
        "seagreen" => (46, 139, 87),
        "forestgreen" => (34, 139, 34),
        "green" => (0, 128, 0),
        "darkgreen" => (0, 100, 0),
        "yellowgreen" => (154, 205, 50),
        "olivedrab" => (107, 142, 35),
        "olive" => (128, 128, 0),
        "darkolivegreen" => (85, 107, 47),
        "mediumaquamarine" => (102, 205, 170),
        "darkseagreen" => (143, 188, 143),
        "lightseagreen" => (32, 178, 170),
        "darkcyan" => (0, 139, 139),
        "teal" => (0, 128, 128),
        _ => return None,
    })
}

fn named_blues_and_purples(name: &str) -> Option<(u8, u8, u8)> {
    Some(match name {
        "aqua" | "cyan" => (0, 255, 255),
        "lightcyan" => (224, 255, 255),
        "paleturquoise" => (175, 238, 238),
        "aquamarine" => (127, 255, 212),
        "turquoise" => (64, 224, 208),
        "mediumturquoise" => (72, 209, 204),
        "darkturquoise" => (0, 206, 209),
        "cadetblue" => (95, 158, 160),
        "steelblue" => (70, 130, 180),
        "lightsteelblue" => (176, 196, 222),
        "powderblue" => (176, 224, 230),
        "lightblue" => (173, 216, 230),
        "skyblue" => (135, 206, 235),
        "lightskyblue" => (135, 206, 250),
        "deepskyblue" => (0, 191, 255),
        "dodgerblue" => (30, 144, 255),
        "cornflowerblue" => (100, 149, 237),
        "mediumslateblue" => (123, 104, 238),
        "royalblue" => (65, 105, 225),
        "blue" => (0, 0, 255),
        "mediumblue" => (0, 0, 205),
        "darkblue" => (0, 0, 139),
        "navy" => (0, 0, 128),
        "midnightblue" => (25, 25, 112),
        "lavender" => (230, 230, 250),
        "thistle" => (216, 191, 216),
        "plum" => (221, 160, 221),
        "violet" => (238, 130, 238),
        "orchid" => (218, 112, 214),
        "fuchsia" | "magenta" => (255, 0, 255),
        "mediumorchid" => (186, 85, 211),
        "mediumpurple" => (147, 112, 219),
        "rebeccapurple" => (102, 51, 153),
        "blueviolet" => (138, 43, 226),
        "darkviolet" => (148, 0, 211),
        "darkorchid" => (153, 50, 204),
        "darkmagenta" => (139, 0, 139),
        "purple" => (128, 0, 128),
        "indigo" => (75, 0, 130),
        "slateblue" => (106, 90, 205),
        "darkslateblue" => (72, 61, 139),
        _ => return None,
    })
}

pub fn parse_background_image(tokens: &[Token], current: Color) -> Option<BackgroundImage> {
    let trimmed = trim_ws(tokens);
    if trimmed.is_empty() {
        return None;
    }
    match &trimmed[0] {
        Token::Ident(s) if s.eq_ignore_ascii_case("none") => Some(BackgroundImage::None),
        Token::Url(u) => Some(BackgroundImage::Url(u.clone())),
        Token::Function(name) => {
            let lower = name.to_ascii_lowercase();
            let inner = extract_single_fn_body(trimmed)?;
            match lower.as_str() {
                "url" => {
                    for t in inner {
                        if let Token::String(s) = t {
                            return Some(BackgroundImage::Url(s.clone()));
                        }
                    }
                    None
                }
                "linear-gradient" | "repeating-linear-gradient" => {
                    parse_linear_gradient(inner, lower.starts_with("repeating-"), current)
                }
                "radial-gradient" | "repeating-radial-gradient" => {
                    parse_radial_gradient(inner, lower.starts_with("repeating-"), current)
                }
                "conic-gradient" | "repeating-conic-gradient" => {
                    parse_conic_gradient(inner, lower.starts_with("repeating-"), current)
                }
                _ => None,
            }
        }
        _ => None,
    }
}

fn parse_linear_gradient(
    inner: &[Token],
    repeating: bool,
    current: Color,
) -> Option<BackgroundImage> {
    let parts = split_commas(inner);
    if parts.is_empty() {
        return None;
    }
    let mut angle_deg = 180.0;
    let mut start_idx = 0;
    let first = trim_ws(parts[0]);
    if let Some(a) = parse_gradient_direction(first) {
        angle_deg = a;
        start_idx = 1;
    }
    let stops = parse_gradient_stops(&parts[start_idx..], current);
    if stops.is_empty() {
        return None;
    }
    Some(BackgroundImage::LinearGradient {
        angle_deg,
        repeating,
        stops,
    })
}

fn parse_gradient_direction(tokens: &[Token]) -> Option<f32> {
    if tokens.len() == 1 {
        if matches!(tokens[0], Token::Dimension { .. }) {
            return parse_angle_deg(&tokens[0]);
        }
        return None;
    }
    let idents: Vec<String> = tokens
        .iter()
        .filter_map(|t| match t {
            Token::Ident(s) => Some(s.to_ascii_lowercase()),
            _ => None,
        })
        .collect();
    if idents.first().map(|s| s.as_str()) != Some("to") {
        return None;
    }
    let dir: Vec<&str> = idents[1..].iter().map(|s| s.as_str()).collect();
    match dir.as_slice() {
        ["top"] => Some(0.0),
        ["right"] => Some(90.0),
        ["bottom"] => Some(180.0),
        ["left"] => Some(270.0),
        ["top", "right"] | ["right", "top"] => Some(45.0),
        ["bottom", "right"] | ["right", "bottom"] => Some(135.0),
        ["bottom", "left"] | ["left", "bottom"] => Some(225.0),
        ["top", "left"] | ["left", "top"] => Some(315.0),
        _ => None,
    }
}

fn parse_radial_gradient(
    inner: &[Token],
    repeating: bool,
    current: Color,
) -> Option<BackgroundImage> {
    let parts = split_commas(inner);
    if parts.is_empty() {
        return None;
    }
    let mut shape = "ellipse".to_string();
    let mut start_idx = 0;
    let first = trim_ws(parts[0]);
    if parse_gradient_stop(first, current).is_none() {
        shape = serialize_tokens(first).trim().to_string();
        start_idx = 1;
    }
    let stops = parse_gradient_stops(&parts[start_idx..], current);
    if stops.is_empty() {
        return None;
    }
    Some(BackgroundImage::RadialGradient {
        shape,
        repeating,
        stops,
    })
}

fn parse_conic_gradient(
    inner: &[Token],
    repeating: bool,
    current: Color,
) -> Option<BackgroundImage> {
    let parts = split_commas(inner);
    if parts.is_empty() {
        return None;
    }
    let mut from_deg = 0.0;
    let mut start_idx = 0;
    let first = trim_ws(parts[0]);
    if parse_gradient_stop(first, current).is_none() {
        for t in first {
            if let Some(a) = parse_angle_deg(t) {
                from_deg = a;
                break;
            }
        }
        start_idx = 1;
    }
    let stops = parse_gradient_stops(&parts[start_idx..], current);
    if stops.is_empty() {
        return None;
    }
    Some(BackgroundImage::ConicGradient {
        from_deg,
        repeating,
        stops,
    })
}

fn parse_gradient_stops(parts: &[&[Token]], current: Color) -> Vec<GradientStop> {
    let mut stops = Vec::new();
    for p in parts {
        if let Some(stop) = parse_gradient_stop(trim_ws(p), current) {
            stops.push(stop);
        }
    }
    stops
}

fn parse_gradient_stop(tokens: &[Token], current: Color) -> Option<GradientStop> {
    let groups = split_whitespace_groups(tokens);
    if groups.is_empty() {
        return None;
    }
    let color = parse_color_from_tokens(groups[0], current)?;
    let position = groups.get(1).and_then(|g| match g.first() {
        Some(Token::Percentage { value, .. }) => Some(*value / 100.0),
        Some(Token::Dimension { value, unit, .. }) if unit.eq_ignore_ascii_case("deg") => {
            Some(*value / 360.0)
        }
        Some(Token::Dimension { value, unit, .. }) if unit.eq_ignore_ascii_case("turn") => {
            Some(*value)
        }
        _ => None,
    });
    Some(GradientStop { color, position })
}

pub fn parse_box_shadows(tokens: &[Token], ctx: &CalcContext, current: Color) -> Vec<BoxShadow> {
    let trimmed = trim_ws(tokens);
    if trimmed.len() == 1 {
        if let Token::Ident(s) = &trimmed[0] {
            if s.eq_ignore_ascii_case("none") {
                return Vec::new();
            }
        }
    }
    let mut out = Vec::new();
    for part in split_commas(trimmed) {
        if let Some(sh) = parse_single_box_shadow(trim_ws(part), ctx, current) {
            out.push(sh);
        }
    }
    out
}

fn parse_single_box_shadow(
    tokens: &[Token],
    ctx: &CalcContext,
    current: Color,
) -> Option<BoxShadow> {
    let groups = split_whitespace_groups(tokens);
    let mut inset = false;
    let mut color = None;
    let mut lengths = Vec::new();

    for g in groups {
        if g.len() == 1 {
            if let Token::Ident(s) = &g[0] {
                if s.eq_ignore_ascii_case("inset") {
                    inset = true;
                    continue;
                }
            }
        }
        if let Some(lpa) = parse_length_percentage_auto(g) {
            if let Some(px) = lpa.resolve(ctx) {
                lengths.push(px);
                continue;
            }
        }
        if let Some(c) = parse_color_from_tokens(g, current) {
            color = Some(c);
        }
    }

    if lengths.len() < 2 {
        return None;
    }
    Some(BoxShadow {
        offset_x: lengths[0],
        offset_y: lengths[1],
        blur_radius: lengths.get(2).copied().unwrap_or(0.0).max(0.0),
        spread_radius: lengths.get(3).copied().unwrap_or(0.0),
        color: color.unwrap_or(current),
        inset,
    })
}

pub fn parse_text_shadows(tokens: &[Token], ctx: &CalcContext, current: Color) -> Vec<TextShadow> {
    let trimmed = trim_ws(tokens);
    if trimmed.len() == 1 {
        if let Token::Ident(s) = &trimmed[0] {
            if s.eq_ignore_ascii_case("none") {
                return Vec::new();
            }
        }
    }
    let mut out = Vec::new();
    for part in split_commas(trimmed) {
        let groups = split_whitespace_groups(trim_ws(part));
        let mut color = None;
        let mut lengths = Vec::new();
        for g in groups {
            if let Some(lpa) = parse_length_percentage_auto(g) {
                if let Some(px) = lpa.resolve(ctx) {
                    lengths.push(px);
                    continue;
                }
            }
            if let Some(c) = parse_color_from_tokens(g, current) {
                color = Some(c);
            }
        }
        if lengths.len() >= 2 {
            out.push(TextShadow {
                offset_x: lengths[0],
                offset_y: lengths[1],
                blur_radius: lengths.get(2).copied().unwrap_or(0.0).max(0.0),
                color: color.unwrap_or(current),
            });
        }
    }
    out
}

pub fn parse_transforms(tokens: &[Token]) -> Vec<TransformOp> {
    let trimmed = trim_ws(tokens);
    if trimmed.len() == 1 {
        if let Token::Ident(s) = &trimmed[0] {
            if s.eq_ignore_ascii_case("none") {
                return Vec::new();
            }
        }
    }
    let mut ops = Vec::new();
    let mut i = 0;
    while i < trimmed.len() {
        let Token::Function(name) = &trimmed[i] else {
            i += 1;
            continue;
        };
        let lower = name.to_ascii_lowercase();
        let start = i + 1;
        let mut depth = 1usize;
        i += 1;
        while i < trimmed.len() && depth > 0 {
            match trimmed[i] {
                Token::OpenParen | Token::Function(_) => depth += 1,
                Token::CloseParen => depth -= 1,
                _ => {}
            }
            i += 1;
        }
        let end = if depth == 0 { i - 1 } else { i };
        let args = split_commas(&trimmed[start..end]);
        match lower.as_str() {
            "translate" => {
                let x = parse_length_percentage_auto(args.first().copied().unwrap_or(&[]))
                    .unwrap_or_else(|| LengthPercentageAuto::px(0.0));
                let y = args
                    .get(1)
                    .and_then(|a| parse_length_percentage_auto(a))
                    .unwrap_or_else(|| LengthPercentageAuto::px(0.0));
                ops.push(TransformOp::Translate(x, y));
            }
            "translatex" => {
                if let Some(x) = parse_length_percentage_auto(args.first().copied().unwrap_or(&[]))
                {
                    ops.push(TransformOp::TranslateX(x));
                }
            }
            "translatey" => {
                if let Some(y) = parse_length_percentage_auto(args.first().copied().unwrap_or(&[]))
                {
                    ops.push(TransformOp::TranslateY(y));
                }
            }
            "scale" => {
                if let Some(sx) = first_number(args.first().copied().unwrap_or(&[])) {
                    let sy = args.get(1).and_then(|a| first_number(a)).unwrap_or(sx);
                    ops.push(TransformOp::Scale(sx, sy));
                }
            }
            "scalex" => {
                if let Some(sx) = first_number(args.first().copied().unwrap_or(&[])) {
                    ops.push(TransformOp::ScaleX(sx));
                }
            }
            "scaley" => {
                if let Some(sy) = first_number(args.first().copied().unwrap_or(&[])) {
                    ops.push(TransformOp::ScaleY(sy));
                }
            }
            "rotate" => {
                if let Some(deg) = trim_ws(args.first().copied().unwrap_or(&[]))
                    .first()
                    .and_then(parse_angle_deg)
                {
                    ops.push(TransformOp::Rotate(deg));
                }
            }
            "skew" => {
                if let Some(ax) = trim_ws(args.first().copied().unwrap_or(&[]))
                    .first()
                    .and_then(parse_angle_deg)
                {
                    let ay = args
                        .get(1)
                        .and_then(|a| trim_ws(a).first())
                        .and_then(parse_angle_deg)
                        .unwrap_or(0.0);
                    ops.push(TransformOp::Skew(ax, ay));
                }
            }
            "skewx" => {
                if let Some(ax) = trim_ws(args.first().copied().unwrap_or(&[]))
                    .first()
                    .and_then(parse_angle_deg)
                {
                    ops.push(TransformOp::SkewX(ax));
                }
            }
            "skewy" => {
                if let Some(ay) = trim_ws(args.first().copied().unwrap_or(&[]))
                    .first()
                    .and_then(parse_angle_deg)
                {
                    ops.push(TransformOp::SkewY(ay));
                }
            }
            "matrix" if args.len() == 6 => {
                let nums: Vec<f32> = args.iter().filter_map(|a| first_number(a)).collect();
                if nums.len() == 6 {
                    ops.push(TransformOp::Matrix(
                        nums[0], nums[1], nums[2], nums[3], nums[4], nums[5],
                    ));
                }
            }
            _ => {}
        }
    }
    ops
}

pub fn parse_filters(tokens: &[Token], ctx: &CalcContext, current: Color) -> Vec<FilterOp> {
    let trimmed = trim_ws(tokens);
    if trimmed.len() == 1 {
        if let Token::Ident(s) = &trimmed[0] {
            if s.eq_ignore_ascii_case("none") {
                return Vec::new();
            }
        }
    }
    let mut ops = Vec::new();
    let mut i = 0;
    while i < trimmed.len() {
        let Token::Function(name) = &trimmed[i] else {
            i += 1;
            continue;
        };
        let lower = name.to_ascii_lowercase();
        let start = i + 1;
        let mut depth = 1usize;
        i += 1;
        while i < trimmed.len() && depth > 0 {
            match trimmed[i] {
                Token::OpenParen | Token::Function(_) => depth += 1,
                Token::CloseParen => depth -= 1,
                _ => {}
            }
            i += 1;
        }
        let end = if depth == 0 { i - 1 } else { i };
        let inner = trim_ws(&trimmed[start..end]);
        match lower.as_str() {
            "blur" => {
                let px = parse_length_percentage_auto(inner)
                    .and_then(|l| l.resolve(ctx))
                    .unwrap_or(0.0);
                ops.push(FilterOp::Blur(px.max(0.0)));
            }
            "brightness" => ops.push(FilterOp::Brightness(first_factor(inner).unwrap_or(1.0))),
            "contrast" => ops.push(FilterOp::Contrast(first_factor(inner).unwrap_or(1.0))),
            "grayscale" => ops.push(FilterOp::Grayscale(first_factor(inner).unwrap_or(1.0))),
            "invert" => ops.push(FilterOp::Invert(first_factor(inner).unwrap_or(1.0))),
            "opacity" => ops.push(FilterOp::Opacity(first_factor(inner).unwrap_or(1.0))),
            "saturate" => ops.push(FilterOp::Saturate(first_factor(inner).unwrap_or(1.0))),
            "sepia" => ops.push(FilterOp::Sepia(first_factor(inner).unwrap_or(1.0))),
            "hue-rotate" => {
                let deg = inner.first().and_then(parse_angle_deg).unwrap_or(0.0);
                ops.push(FilterOp::HueRotate(deg));
            }
            "drop-shadow" => {
                if let Some(sh) = parse_single_box_shadow(inner, ctx, current) {
                    ops.push(FilterOp::DropShadow(sh));
                }
            }
            _ => {}
        }
    }
    ops
}

pub fn parse_grid_tracks(tokens: &[Token], ctx: &CalcContext) -> Vec<GridTrackSize> {
    let trimmed = trim_ws(tokens);
    if trimmed.len() == 1 {
        if let Token::Ident(s) = &trimmed[0] {
            if s.eq_ignore_ascii_case("none") {
                return Vec::new();
            }
        }
    }
    let mut tracks = Vec::new();
    let mut i = 0;
    while i < trimmed.len() {
        match &trimmed[i] {
            Token::Whitespace => {
                i += 1;
            }
            Token::OpenSquare => {
                i += 1;
                while i < trimmed.len() && trimmed[i] != Token::CloseSquare {
                    i += 1;
                }
                i += 1;
            }
            Token::Function(name) => {
                let lower = name.to_ascii_lowercase();
                let start = i + 1;
                let mut depth = 1usize;
                i += 1;
                while i < trimmed.len() && depth > 0 {
                    match trimmed[i] {
                        Token::OpenParen | Token::Function(_) => depth += 1,
                        Token::CloseParen => depth -= 1,
                        _ => {}
                    }
                    i += 1;
                }
                let end = if depth == 0 { i - 1 } else { i };
                let inner = &trimmed[start..end];
                match lower.as_str() {
                    "minmax" => {
                        let args = split_commas(inner);
                        if args.len() == 2 {
                            let min_t = parse_single_track(trim_ws(args[0]), ctx)
                                .unwrap_or(GridTrackSize::Auto);
                            let max_t = parse_single_track(trim_ws(args[1]), ctx)
                                .unwrap_or(GridTrackSize::Auto);
                            tracks.push(GridTrackSize::MinMax(Box::new(min_t), Box::new(max_t)));
                        }
                    }
                    "fit-content" => {
                        let px = parse_length_percentage_auto(inner)
                            .and_then(|l| l.resolve(ctx))
                            .unwrap_or(0.0);
                        tracks.push(GridTrackSize::FitContent(px));
                    }
                    "repeat" => {
                        let args = split_commas(inner);
                        if args.len() >= 2 {
                            let count = match trim_ws(args[0]).first() {
                                Some(Token::Number { value, .. }) if *value >= 1.0 => {
                                    Some(GridRepeatCount::Count(value.round() as u32))
                                }
                                Some(Token::Ident(s)) if s.eq_ignore_ascii_case("auto-fill") => {
                                    Some(GridRepeatCount::AutoFill)
                                }
                                Some(Token::Ident(s)) if s.eq_ignore_ascii_case("auto-fit") => {
                                    Some(GridRepeatCount::AutoFit)
                                }
                                _ => None,
                            };
                            if let Some(c) = count {
                                let inner_tracks = parse_grid_tracks(args[1], ctx);
                                if !inner_tracks.is_empty() {
                                    tracks.push(GridTrackSize::Repeat(c, inner_tracks));
                                }
                            }
                        }
                    }
                    "calc" | "min" | "max" | "clamp" => {
                        if let Some(expr) = parse_calc_function(&trimmed[start - 1..i]) {
                            tracks.push(GridTrackSize::Px(expr.evaluate(ctx)));
                        }
                    }
                    _ => {}
                }
            }
            tok => {
                if let Some(tr) = parse_single_track_token(tok, ctx) {
                    tracks.push(tr);
                }
                i += 1;
            }
        }
    }
    tracks
}

fn parse_single_track(tokens: &[Token], ctx: &CalcContext) -> Option<GridTrackSize> {
    if tokens.len() == 1 {
        return parse_single_track_token(&tokens[0], ctx);
    }
    if let Some(lpa) = parse_length_percentage_auto(tokens) {
        return match lpa {
            LengthPercentageAuto::Auto => Some(GridTrackSize::Auto),
            LengthPercentageAuto::Percent(p) => Some(GridTrackSize::Percent(p)),
            other => Some(GridTrackSize::Px(other.resolve_or(ctx, 0.0))),
        };
    }
    None
}

fn parse_single_track_token(tok: &Token, ctx: &CalcContext) -> Option<GridTrackSize> {
    match tok {
        Token::Ident(s) => match s.to_ascii_lowercase().as_str() {
            "auto" => Some(GridTrackSize::Auto),
            "min-content" => Some(GridTrackSize::MinContent),
            "max-content" => Some(GridTrackSize::MaxContent),
            _ => None,
        },
        Token::Dimension { value, unit, .. } => {
            if unit.eq_ignore_ascii_case("fr") {
                Some(GridTrackSize::Fr(*value))
            } else {
                let u = parse_length_unit(unit)?;
                Some(GridTrackSize::Px(Length { value: *value, unit: u }.to_px(ctx)))
            }
        }
        Token::Percentage { value, .. } => Some(GridTrackSize::Percent(*value)),
        Token::Number { value, .. } if *value == 0.0 => Some(GridTrackSize::Px(0.0)),
        _ => None,
    }
}

pub fn parse_grid_placement(tokens: &[Token]) -> Option<GridPlacement> {
    let non_ws: Vec<&Token> = tokens
        .iter()
        .filter(|t| **t != Token::Whitespace)
        .collect();
    if non_ws.is_empty() {
        return None;
    }
    if non_ws.len() == 1 {
        return match non_ws[0] {
            Token::Ident(s) if s.eq_ignore_ascii_case("auto") => Some(GridPlacement::Auto),
            Token::Number { value, .. } => Some(GridPlacement::Line(value.round() as i32)),
            _ => None,
        };
    }
    if let Token::Ident(s) = non_ws[0] {
        if s.eq_ignore_ascii_case("span") {
            if let Some(Token::Number { value, .. }) = non_ws.get(1) {
                return Some(GridPlacement::Span((*value).max(1.0).round() as u32));
            }
            return Some(GridPlacement::Span(1));
        }
    }
    None
}

fn first_number(tokens: &[Token]) -> Option<f32> {
    for t in tokens {
        match t {
            Token::Number { value, .. } => return Some(*value),
            Token::Percentage { value, .. } => return Some(*value / 100.0),
            _ => {}
        }
    }
    None
}

fn first_factor(tokens: &[Token]) -> Option<f32> {
    for t in tokens {
        match t {
            Token::Number { value, .. } => return Some(*value),
            Token::Percentage { value, .. } => return Some(*value / 100.0),
            _ => {}
        }
    }
    None
}

pub(crate) fn trim_ws(tokens: &[Token]) -> &[Token] {
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

pub(crate) fn split_commas(tokens: &[Token]) -> Vec<&[Token]> {
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

pub(crate) fn split_whitespace_groups(tokens: &[Token]) -> Vec<&[Token]> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut start: Option<usize> = None;
    for (i, t) in tokens.iter().enumerate() {
        match t {
            Token::OpenParen | Token::Function(_) | Token::OpenSquare | Token::OpenCurly => {
                if start.is_none() {
                    start = Some(i);
                }
                depth += 1;
            }
            Token::CloseParen | Token::CloseSquare | Token::CloseCurly => {
                depth = depth.saturating_sub(1);
            }
            Token::Whitespace if depth == 0 => {
                if let Some(s) = start.take() {
                    out.push(&tokens[s..i]);
                }
            }
            _ => {
                if start.is_none() {
                    start = Some(i);
                }
            }
        }
    }
    if let Some(s) = start {
        out.push(&tokens[s..]);
    }
    out
}
