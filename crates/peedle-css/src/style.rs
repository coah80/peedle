use crate::parser::{parse_stylesheet, Stylesheet};
use crate::tokenizer::Token;
use crate::values::{
    format_px, trim_float, BackgroundImage, BoxShadow, Color, FilterOp, GridPlacement,
    GridTrackSize, LengthPercentageAuto, TextShadow, TransformOp,
};
use std::collections::HashMap;
use std::rc::Rc;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Display {
    #[default]
    Inline,
    Block,
    InlineBlock,
    Flex,
    InlineFlex,
    Grid,
    InlineGrid,
    Table,
    InlineTable,
    TableRow,
    TableCell,
    TableRowGroup,
    TableHeaderGroup,
    TableFooterGroup,
    TableCaption,
    ListItem,
    Contents,
    FlowRoot,
    None,
}

impl Display {
    pub fn as_css_str(self) -> &'static str {
        match self {
            Display::Inline => "inline",
            Display::Block => "block",
            Display::InlineBlock => "inline-block",
            Display::Flex => "flex",
            Display::InlineFlex => "inline-flex",
            Display::Grid => "grid",
            Display::InlineGrid => "inline-grid",
            Display::Table => "table",
            Display::InlineTable => "inline-table",
            Display::TableRow => "table-row",
            Display::TableCell => "table-cell",
            Display::TableRowGroup => "table-row-group",
            Display::TableHeaderGroup => "table-header-group",
            Display::TableFooterGroup => "table-footer-group",
            Display::TableCaption => "table-caption",
            Display::ListItem => "list-item",
            Display::Contents => "contents",
            Display::FlowRoot => "flow-root",
            Display::None => "none",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Position {
    #[default]
    Static,
    Relative,
    Absolute,
    Fixed,
    Sticky,
}

impl Position {
    pub fn as_css_str(self) -> &'static str {
        match self {
            Position::Static => "static",
            Position::Relative => "relative",
            Position::Absolute => "absolute",
            Position::Fixed => "fixed",
            Position::Sticky => "sticky",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum BoxSizing {
    #[default]
    ContentBox,
    BorderBox,
}

impl BoxSizing {
    pub fn as_css_str(self) -> &'static str {
        match self {
            BoxSizing::ContentBox => "content-box",
            BoxSizing::BorderBox => "border-box",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Float {
    #[default]
    None,
    Left,
    Right,
}

impl Float {
    pub fn as_css_str(self) -> &'static str {
        match self {
            Float::None => "none",
            Float::Left => "left",
            Float::Right => "right",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Clear {
    #[default]
    None,
    Left,
    Right,
    Both,
}

impl Clear {
    pub fn as_css_str(self) -> &'static str {
        match self {
            Clear::None => "none",
            Clear::Left => "left",
            Clear::Right => "right",
            Clear::Both => "both",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Overflow {
    #[default]
    Visible,
    Hidden,
    Scroll,
    Auto,
    Clip,
}

impl Overflow {
    pub fn as_css_str(self) -> &'static str {
        match self {
            Overflow::Visible => "visible",
            Overflow::Hidden => "hidden",
            Overflow::Scroll => "scroll",
            Overflow::Auto => "auto",
            Overflow::Clip => "clip",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Visibility {
    #[default]
    Visible,
    Hidden,
    Collapse,
}

impl Visibility {
    pub fn as_css_str(self) -> &'static str {
        match self {
            Visibility::Visible => "visible",
            Visibility::Hidden => "hidden",
            Visibility::Collapse => "collapse",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum FontStyle {
    #[default]
    Normal,
    Italic,
    Oblique,
}

impl FontStyle {
    pub fn as_css_str(self) -> &'static str {
        match self {
            FontStyle::Normal => "normal",
            FontStyle::Italic => "italic",
            FontStyle::Oblique => "oblique",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TextAlign {
    #[default]
    Start,
    End,
    Left,
    Right,
    Center,
    Justify,
}

impl TextAlign {
    pub fn as_css_str(self) -> &'static str {
        match self {
            TextAlign::Start => "start",
            TextAlign::End => "end",
            TextAlign::Left => "left",
            TextAlign::Right => "right",
            TextAlign::Center => "center",
            TextAlign::Justify => "justify",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TextDecorationLine {
    #[default]
    None,
    Underline,
    Overline,
    LineThrough,
}

impl TextDecorationLine {
    pub fn as_css_str(self) -> &'static str {
        match self {
            TextDecorationLine::None => "none",
            TextDecorationLine::Underline => "underline",
            TextDecorationLine::Overline => "overline",
            TextDecorationLine::LineThrough => "line-through",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TextDecorationStyle {
    #[default]
    Solid,
    Double,
    Dotted,
    Dashed,
    Wavy,
}

impl TextDecorationStyle {
    pub fn as_css_str(self) -> &'static str {
        match self {
            TextDecorationStyle::Solid => "solid",
            TextDecorationStyle::Double => "double",
            TextDecorationStyle::Dotted => "dotted",
            TextDecorationStyle::Dashed => "dashed",
            TextDecorationStyle::Wavy => "wavy",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TextTransform {
    #[default]
    None,
    Uppercase,
    Lowercase,
    Capitalize,
}

impl TextTransform {
    pub fn as_css_str(self) -> &'static str {
        match self {
            TextTransform::None => "none",
            TextTransform::Uppercase => "uppercase",
            TextTransform::Lowercase => "lowercase",
            TextTransform::Capitalize => "capitalize",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TextOverflow {
    #[default]
    Clip,
    Ellipsis,
}

impl TextOverflow {
    pub fn as_css_str(self) -> &'static str {
        match self {
            TextOverflow::Clip => "clip",
            TextOverflow::Ellipsis => "ellipsis",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum WhiteSpace {
    #[default]
    Normal,
    Nowrap,
    Pre,
    PreWrap,
    PreLine,
    BreakSpaces,
}

impl WhiteSpace {
    pub fn as_css_str(self) -> &'static str {
        match self {
            WhiteSpace::Normal => "normal",
            WhiteSpace::Nowrap => "nowrap",
            WhiteSpace::Pre => "pre",
            WhiteSpace::PreWrap => "pre-wrap",
            WhiteSpace::PreLine => "pre-line",
            WhiteSpace::BreakSpaces => "break-spaces",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum WordBreak {
    #[default]
    Normal,
    BreakAll,
    KeepAll,
    BreakWord,
}

impl WordBreak {
    pub fn as_css_str(self) -> &'static str {
        match self {
            WordBreak::Normal => "normal",
            WordBreak::BreakAll => "break-all",
            WordBreak::KeepAll => "keep-all",
            WordBreak::BreakWord => "break-word",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum OverflowWrap {
    #[default]
    Normal,
    BreakWord,
    Anywhere,
}

impl OverflowWrap {
    pub fn as_css_str(self) -> &'static str {
        match self {
            OverflowWrap::Normal => "normal",
            OverflowWrap::BreakWord => "break-word",
            OverflowWrap::Anywhere => "anywhere",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum VerticalAlign {
    #[default]
    Baseline,
    Top,
    Middle,
    Bottom,
    TextTop,
    TextBottom,
    Sub,
    Super,
    Length(f32),
}

impl VerticalAlign {
    pub fn as_css_str(self) -> &'static str {
        match self {
            VerticalAlign::Baseline => "baseline",
            VerticalAlign::Top => "top",
            VerticalAlign::Middle => "middle",
            VerticalAlign::Bottom => "bottom",
            VerticalAlign::TextTop => "text-top",
            VerticalAlign::TextBottom => "text-bottom",
            VerticalAlign::Sub => "sub",
            VerticalAlign::Super => "super",
            VerticalAlign::Length(_) => "0px",
        }
    }

    pub fn to_css_string(self) -> String {
        match self {
            VerticalAlign::Length(px) => format_px(px),
            other => other.as_css_str().to_string(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum BorderStyle {
    #[default]
    None,
    Hidden,
    Solid,
    Dashed,
    Dotted,
    Double,
    Groove,
    Ridge,
    Inset,
    Outset,
}

impl BorderStyle {
    pub fn as_css_str(self) -> &'static str {
        match self {
            BorderStyle::None => "none",
            BorderStyle::Hidden => "hidden",
            BorderStyle::Solid => "solid",
            BorderStyle::Dashed => "dashed",
            BorderStyle::Dotted => "dotted",
            BorderStyle::Double => "double",
            BorderStyle::Groove => "groove",
            BorderStyle::Ridge => "ridge",
            BorderStyle::Inset => "inset",
            BorderStyle::Outset => "outset",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum BorderCollapse {
    #[default]
    Separate,
    Collapse,
}

impl BorderCollapse {
    pub fn as_css_str(self) -> &'static str {
        match self {
            BorderCollapse::Separate => "separate",
            BorderCollapse::Collapse => "collapse",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TableLayout {
    #[default]
    Auto,
    Fixed,
}

impl TableLayout {
    pub fn as_css_str(self) -> &'static str {
        match self {
            TableLayout::Auto => "auto",
            TableLayout::Fixed => "fixed",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum FlexDirection {
    #[default]
    Row,
    RowReverse,
    Column,
    ColumnReverse,
}

impl FlexDirection {
    pub fn as_css_str(self) -> &'static str {
        match self {
            FlexDirection::Row => "row",
            FlexDirection::RowReverse => "row-reverse",
            FlexDirection::Column => "column",
            FlexDirection::ColumnReverse => "column-reverse",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum FlexWrap {
    #[default]
    Nowrap,
    Wrap,
    WrapReverse,
}

impl FlexWrap {
    pub fn as_css_str(self) -> &'static str {
        match self {
            FlexWrap::Nowrap => "nowrap",
            FlexWrap::Wrap => "wrap",
            FlexWrap::WrapReverse => "wrap-reverse",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum JustifyContent {
    #[default]
    FlexStart,
    FlexEnd,
    Center,
    SpaceBetween,
    SpaceAround,
    SpaceEvenly,
    Start,
    End,
    Stretch,
}

impl JustifyContent {
    pub fn as_css_str(self) -> &'static str {
        match self {
            JustifyContent::FlexStart => "flex-start",
            JustifyContent::FlexEnd => "flex-end",
            JustifyContent::Center => "center",
            JustifyContent::SpaceBetween => "space-between",
            JustifyContent::SpaceAround => "space-around",
            JustifyContent::SpaceEvenly => "space-evenly",
            JustifyContent::Start => "start",
            JustifyContent::End => "end",
            JustifyContent::Stretch => "stretch",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum AlignItems {
    #[default]
    Stretch,
    FlexStart,
    FlexEnd,
    Center,
    Baseline,
    Start,
    End,
}

impl AlignItems {
    pub fn as_css_str(self) -> &'static str {
        match self {
            AlignItems::Stretch => "stretch",
            AlignItems::FlexStart => "flex-start",
            AlignItems::FlexEnd => "flex-end",
            AlignItems::Center => "center",
            AlignItems::Baseline => "baseline",
            AlignItems::Start => "start",
            AlignItems::End => "end",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum AlignSelf {
    #[default]
    Auto,
    Stretch,
    FlexStart,
    FlexEnd,
    Center,
    Baseline,
    Start,
    End,
}

impl AlignSelf {
    pub fn as_css_str(self) -> &'static str {
        match self {
            AlignSelf::Auto => "auto",
            AlignSelf::Stretch => "stretch",
            AlignSelf::FlexStart => "flex-start",
            AlignSelf::FlexEnd => "flex-end",
            AlignSelf::Center => "center",
            AlignSelf::Baseline => "baseline",
            AlignSelf::Start => "start",
            AlignSelf::End => "end",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum AlignContent {
    #[default]
    Stretch,
    FlexStart,
    FlexEnd,
    Center,
    SpaceBetween,
    SpaceAround,
    SpaceEvenly,
    Start,
    End,
}

impl AlignContent {
    pub fn as_css_str(self) -> &'static str {
        match self {
            AlignContent::Stretch => "stretch",
            AlignContent::FlexStart => "flex-start",
            AlignContent::FlexEnd => "flex-end",
            AlignContent::Center => "center",
            AlignContent::SpaceBetween => "space-between",
            AlignContent::SpaceAround => "space-around",
            AlignContent::SpaceEvenly => "space-evenly",
            AlignContent::Start => "start",
            AlignContent::End => "end",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum GridAutoFlow {
    #[default]
    Row,
    Column,
    RowDense,
    ColumnDense,
}

impl GridAutoFlow {
    pub fn as_css_str(self) -> &'static str {
        match self {
            GridAutoFlow::Row => "row",
            GridAutoFlow::Column => "column",
            GridAutoFlow::RowDense => "row dense",
            GridAutoFlow::ColumnDense => "column dense",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum BackgroundRepeat {
    #[default]
    Repeat,
    RepeatX,
    RepeatY,
    NoRepeat,
    Space,
    Round,
}

impl BackgroundRepeat {
    pub fn as_css_str(self) -> &'static str {
        match self {
            BackgroundRepeat::Repeat => "repeat",
            BackgroundRepeat::RepeatX => "repeat-x",
            BackgroundRepeat::RepeatY => "repeat-y",
            BackgroundRepeat::NoRepeat => "no-repeat",
            BackgroundRepeat::Space => "space",
            BackgroundRepeat::Round => "round",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub enum BackgroundSize {
    #[default]
    Auto,
    Cover,
    Contain,
    Explicit(LengthPercentageAuto, LengthPercentageAuto),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum BackgroundClip {
    #[default]
    BorderBox,
    PaddingBox,
    ContentBox,
    Text,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ListStyleType {
    #[default]
    Disc,
    Circle,
    Square,
    Decimal,
    LowerAlpha,
    UpperAlpha,
    LowerRoman,
    UpperRoman,
    None,
}

impl ListStyleType {
    pub fn as_css_str(self) -> &'static str {
        match self {
            ListStyleType::Disc => "disc",
            ListStyleType::Circle => "circle",
            ListStyleType::Square => "square",
            ListStyleType::Decimal => "decimal",
            ListStyleType::LowerAlpha => "lower-alpha",
            ListStyleType::UpperAlpha => "upper-alpha",
            ListStyleType::LowerRoman => "lower-roman",
            ListStyleType::UpperRoman => "upper-roman",
            ListStyleType::None => "none",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ListStylePosition {
    #[default]
    Outside,
    Inside,
}

impl ListStylePosition {
    pub fn as_css_str(self) -> &'static str {
        match self {
            ListStylePosition::Outside => "outside",
            ListStylePosition::Inside => "inside",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Cursor {
    #[default]
    Auto,
    Default,
    None,
    ContextMenu,
    Help,
    Pointer,
    Progress,
    Wait,
    Cell,
    Crosshair,
    Text,
    VerticalText,
    Alias,
    Copy,
    Move,
    NoDrop,
    NotAllowed,
    Grab,
    Grabbing,
    EResize,
    NResize,
    NeResize,
    NwResize,
    SResize,
    SeResize,
    SwResize,
    WResize,
    EwResize,
    NsResize,
    NeswResize,
    NwseResize,
    ColResize,
    RowResize,
    AllScroll,
    ZoomIn,
    ZoomOut,
}

impl Cursor {
    pub fn as_css_str(self) -> &'static str {
        match self {
            Cursor::Auto => "auto",
            Cursor::Default => "default",
            Cursor::None => "none",
            Cursor::ContextMenu => "context-menu",
            Cursor::Help => "help",
            Cursor::Pointer => "pointer",
            Cursor::Progress => "progress",
            Cursor::Wait => "wait",
            Cursor::Cell => "cell",
            Cursor::Crosshair => "crosshair",
            Cursor::Text => "text",
            Cursor::VerticalText => "vertical-text",
            Cursor::Alias => "alias",
            Cursor::Copy => "copy",
            Cursor::Move => "move",
            Cursor::NoDrop => "no-drop",
            Cursor::NotAllowed => "not-allowed",
            Cursor::Grab => "grab",
            Cursor::Grabbing => "grabbing",
            Cursor::EResize => "e-resize",
            Cursor::NResize => "n-resize",
            Cursor::NeResize => "ne-resize",
            Cursor::NwResize => "nw-resize",
            Cursor::SResize => "s-resize",
            Cursor::SeResize => "se-resize",
            Cursor::SwResize => "sw-resize",
            Cursor::WResize => "w-resize",
            Cursor::EwResize => "ew-resize",
            Cursor::NsResize => "ns-resize",
            Cursor::NeswResize => "nesw-resize",
            Cursor::NwseResize => "nwse-resize",
            Cursor::ColResize => "col-resize",
            Cursor::RowResize => "row-resize",
            Cursor::AllScroll => "all-scroll",
            Cursor::ZoomIn => "zoom-in",
            Cursor::ZoomOut => "zoom-out",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum PointerEvents {
    #[default]
    Auto,
    None,
}

impl PointerEvents {
    pub fn as_css_str(self) -> &'static str {
        match self {
            PointerEvents::Auto => "auto",
            PointerEvents::None => "none",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum UserSelect {
    #[default]
    Auto,
    None,
    Text,
    All,
}

impl UserSelect {
    pub fn as_css_str(self) -> &'static str {
        match self {
            UserSelect::Auto => "auto",
            UserSelect::None => "none",
            UserSelect::Text => "text",
            UserSelect::All => "all",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ObjectFit {
    #[default]
    Fill,
    Contain,
    Cover,
    None,
    ScaleDown,
}

impl ObjectFit {
    pub fn as_css_str(self) -> &'static str {
        match self {
            ObjectFit::Fill => "fill",
            ObjectFit::Contain => "contain",
            ObjectFit::Cover => "cover",
            ObjectFit::None => "none",
            ObjectFit::ScaleDown => "scale-down",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum LineHeight {
    #[default]
    Normal,
    Number(f32),
    Px(f32),
}

impl LineHeight {
    pub fn resolve(self, font_size: f32) -> f32 {
        match self {
            LineHeight::Normal => font_size * 1.2,
            LineHeight::Number(n) => font_size * n,
            LineHeight::Px(px) => px,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ComputedStyle {
    pub display: Display,
    pub position: Position,
    pub box_sizing: BoxSizing,
    pub float: Float,
    pub clear: Clear,
    pub z_index: Option<i32>,

    pub width: LengthPercentageAuto,
    pub height: LengthPercentageAuto,
    pub min_width: LengthPercentageAuto,
    pub max_width: LengthPercentageAuto,
    pub min_height: LengthPercentageAuto,
    pub max_height: LengthPercentageAuto,
    pub aspect_ratio: Option<f32>,

    pub top: LengthPercentageAuto,
    pub right: LengthPercentageAuto,
    pub bottom: LengthPercentageAuto,
    pub left: LengthPercentageAuto,

    pub margin_top: LengthPercentageAuto,
    pub margin_right: LengthPercentageAuto,
    pub margin_bottom: LengthPercentageAuto,
    pub margin_left: LengthPercentageAuto,

    pub padding_top: LengthPercentageAuto,
    pub padding_right: LengthPercentageAuto,
    pub padding_bottom: LengthPercentageAuto,
    pub padding_left: LengthPercentageAuto,

    pub border_top_width: f32,
    pub border_right_width: f32,
    pub border_bottom_width: f32,
    pub border_left_width: f32,
    pub border_top_style: BorderStyle,
    pub border_right_style: BorderStyle,
    pub border_bottom_style: BorderStyle,
    pub border_left_style: BorderStyle,
    pub border_top_color: Color,
    pub border_right_color: Color,
    pub border_bottom_color: Color,
    pub border_left_color: Color,

    pub border_top_left_radius: LengthPercentageAuto,
    pub border_top_right_radius: LengthPercentageAuto,
    pub border_bottom_right_radius: LengthPercentageAuto,
    pub border_bottom_left_radius: LengthPercentageAuto,

    pub border_collapse: BorderCollapse,
    pub border_spacing_x: f32,
    pub border_spacing_y: f32,
    pub table_layout: TableLayout,

    pub outline_width: f32,
    pub outline_style: BorderStyle,
    pub outline_color: Color,
    pub outline_offset: f32,

    pub color: Color,
    pub font_family: Vec<String>,
    pub font_size: f32,
    pub font_weight: u16,
    pub font_style: FontStyle,
    pub line_height: LineHeight,
    pub letter_spacing: f32,
    pub word_spacing: f32,
    pub text_align: TextAlign,
    pub text_decoration_line: TextDecorationLine,
    pub text_decoration_color: Option<Color>,
    pub text_decoration_style: TextDecorationStyle,
    pub text_transform: TextTransform,
    pub text_indent: f32,
    pub text_overflow: TextOverflow,
    pub white_space: WhiteSpace,
    pub word_break: WordBreak,
    pub overflow_wrap: OverflowWrap,
    pub vertical_align: VerticalAlign,

    pub background_color: Color,
    pub background_image: BackgroundImage,
    pub mask_image: Option<String>,
    pub background_position_x: LengthPercentageAuto,
    pub background_position_y: LengthPercentageAuto,
    pub background_attachment_fixed: bool,
    pub background_size: BackgroundSize,
    pub background_repeat: BackgroundRepeat,
    pub background_clip: BackgroundClip,

    pub opacity: f32,
    pub visibility: Visibility,
    pub overflow_x: Overflow,
    pub overflow_y: Overflow,
    pub clip_hidden: bool,
    pub box_shadow: Vec<BoxShadow>,
    pub text_shadow: Vec<TextShadow>,
    pub transform: Vec<TransformOp>,
    pub transform_origin_x: LengthPercentageAuto,
    pub transform_origin_y: LengthPercentageAuto,
    pub filter: Vec<FilterOp>,
    pub backdrop_filter: Vec<FilterOp>,
    pub object_fit: ObjectFit,

    pub flex_direction: FlexDirection,
    pub flex_wrap: FlexWrap,
    pub flex_grow: f32,
    pub flex_shrink: f32,
    pub flex_basis: LengthPercentageAuto,
    pub justify_content: JustifyContent,
    pub align_items: AlignItems,
    pub align_self: AlignSelf,
    pub align_content: AlignContent,
    pub justify_items: AlignItems,
    pub justify_self: AlignSelf,
    pub order: i32,
    pub row_gap: LengthPercentageAuto,
    pub column_gap: LengthPercentageAuto,

    pub grid_template_columns: Vec<GridTrackSize>,
    pub grid_template_rows: Vec<GridTrackSize>,
    pub grid_template_areas: Vec<Vec<String>>,
    pub grid_auto_columns: Vec<GridTrackSize>,
    pub grid_auto_rows: Vec<GridTrackSize>,
    pub grid_auto_flow: GridAutoFlow,
    pub grid_area: Option<String>,
    pub grid_column_start: GridPlacement,
    pub grid_column_end: GridPlacement,
    pub grid_row_start: GridPlacement,
    pub grid_row_end: GridPlacement,

    pub list_style_type: ListStyleType,
    pub list_style_position: ListStylePosition,
    pub content: Option<String>,
    pub cursor: Cursor,
    pub pointer_events: PointerEvents,
    pub user_select: UserSelect,
    pub custom_properties: Rc<HashMap<String, Vec<Token>>>,
}

impl Default for ComputedStyle {
    fn default() -> Self {
        Self {
            display: Display::Inline,
            position: Position::Static,
            box_sizing: BoxSizing::ContentBox,
            float: Float::None,
            clear: Clear::None,
            z_index: None,

            width: LengthPercentageAuto::Auto,
            height: LengthPercentageAuto::Auto,
            min_width: LengthPercentageAuto::Auto,
            max_width: LengthPercentageAuto::Auto,
            min_height: LengthPercentageAuto::Auto,
            max_height: LengthPercentageAuto::Auto,
            aspect_ratio: None,

            top: LengthPercentageAuto::Auto,
            right: LengthPercentageAuto::Auto,
            bottom: LengthPercentageAuto::Auto,
            left: LengthPercentageAuto::Auto,

            margin_top: LengthPercentageAuto::px(0.0),
            margin_right: LengthPercentageAuto::px(0.0),
            margin_bottom: LengthPercentageAuto::px(0.0),
            margin_left: LengthPercentageAuto::px(0.0),

            padding_top: LengthPercentageAuto::px(0.0),
            padding_right: LengthPercentageAuto::px(0.0),
            padding_bottom: LengthPercentageAuto::px(0.0),
            padding_left: LengthPercentageAuto::px(0.0),

            border_top_width: 0.0,
            border_right_width: 0.0,
            border_bottom_width: 0.0,
            border_left_width: 0.0,
            border_top_style: BorderStyle::None,
            border_right_style: BorderStyle::None,
            border_bottom_style: BorderStyle::None,
            border_left_style: BorderStyle::None,
            border_top_color: Color::BLACK,
            border_right_color: Color::BLACK,
            border_bottom_color: Color::BLACK,
            border_left_color: Color::BLACK,

            border_top_left_radius: LengthPercentageAuto::px(0.0),
            border_top_right_radius: LengthPercentageAuto::px(0.0),
            border_bottom_right_radius: LengthPercentageAuto::px(0.0),
            border_bottom_left_radius: LengthPercentageAuto::px(0.0),

            border_collapse: BorderCollapse::Separate,
            border_spacing_x: 0.0,
            border_spacing_y: 0.0,
            table_layout: TableLayout::Auto,

            outline_width: 0.0,
            outline_style: BorderStyle::None,
            outline_color: Color::BLACK,
            outline_offset: 0.0,

            color: Color::BLACK,
            font_family: vec!["sans-serif".to_string()],
            font_size: 16.0,
            font_weight: 400,
            font_style: FontStyle::Normal,
            line_height: LineHeight::Normal,
            letter_spacing: 0.0,
            word_spacing: 0.0,
            text_align: TextAlign::Start,
            text_decoration_line: TextDecorationLine::None,
            text_decoration_color: None,
            text_decoration_style: TextDecorationStyle::Solid,
            text_transform: TextTransform::None,
            text_indent: 0.0,
            text_overflow: TextOverflow::Clip,
            white_space: WhiteSpace::Normal,
            word_break: WordBreak::Normal,
            overflow_wrap: OverflowWrap::Normal,
            vertical_align: VerticalAlign::Baseline,

            background_color: Color::TRANSPARENT,
            background_image: BackgroundImage::None,
            mask_image: None,
            background_position_x: LengthPercentageAuto::Percent(0.0),
            background_position_y: LengthPercentageAuto::Percent(0.0),
            background_attachment_fixed: false,
            background_size: BackgroundSize::Auto,
            background_repeat: BackgroundRepeat::Repeat,
            background_clip: BackgroundClip::BorderBox,

            opacity: 1.0,
            visibility: Visibility::Visible,
            overflow_x: Overflow::Visible,
            overflow_y: Overflow::Visible,
            clip_hidden: false,
            box_shadow: Vec::new(),
            text_shadow: Vec::new(),
            transform: Vec::new(),
            transform_origin_x: LengthPercentageAuto::Percent(50.0),
            transform_origin_y: LengthPercentageAuto::Percent(50.0),
            filter: Vec::new(),
            backdrop_filter: Vec::new(),
            object_fit: ObjectFit::Fill,

            flex_direction: FlexDirection::Row,
            flex_wrap: FlexWrap::Nowrap,
            flex_grow: 0.0,
            flex_shrink: 1.0,
            flex_basis: LengthPercentageAuto::Auto,
            justify_content: JustifyContent::FlexStart,
            align_items: AlignItems::Stretch,
            align_self: AlignSelf::Auto,
            align_content: AlignContent::Stretch,
            justify_items: AlignItems::Stretch,
            justify_self: AlignSelf::Auto,
            order: 0,
            row_gap: LengthPercentageAuto::px(0.0),
            column_gap: LengthPercentageAuto::px(0.0),

            grid_template_columns: Vec::new(),
            grid_template_rows: Vec::new(),
            grid_template_areas: Vec::new(),
            grid_auto_columns: Vec::new(),
            grid_auto_rows: Vec::new(),
            grid_auto_flow: GridAutoFlow::Row,
            grid_area: None,
            grid_column_start: GridPlacement::Auto,
            grid_column_end: GridPlacement::Auto,
            grid_row_start: GridPlacement::Auto,
            grid_row_end: GridPlacement::Auto,

            list_style_type: ListStyleType::Disc,
            list_style_position: ListStylePosition::Outside,
            content: None,
            cursor: Cursor::Auto,
            pointer_events: PointerEvents::Auto,
            user_select: UserSelect::Auto,
            custom_properties: Rc::new(HashMap::new()),
        }
    }
}

impl ComputedStyle {
    pub fn inherit_from(parent: &ComputedStyle) -> Self {
        let mut s = Self::default();
        s.color = parent.color;
        s.border_top_color = parent.color;
        s.border_right_color = parent.color;
        s.border_bottom_color = parent.color;
        s.border_left_color = parent.color;
        s.outline_color = parent.color;
        s.font_family = parent.font_family.clone();
        s.font_size = parent.font_size;
        s.font_weight = parent.font_weight;
        s.font_style = parent.font_style;
        s.line_height = parent.line_height;
        s.letter_spacing = parent.letter_spacing;
        s.word_spacing = parent.word_spacing;
        s.text_align = parent.text_align;
        s.text_transform = parent.text_transform;
        s.text_indent = parent.text_indent;
        s.white_space = parent.white_space;
        s.word_break = parent.word_break;
        s.overflow_wrap = parent.overflow_wrap;
        s.visibility = parent.visibility;
        s.text_shadow = parent.text_shadow.clone();
        s.border_collapse = parent.border_collapse;
        s.border_spacing_x = parent.border_spacing_x;
        s.border_spacing_y = parent.border_spacing_y;
        s.list_style_type = parent.list_style_type;
        s.list_style_position = parent.list_style_position;
        s.cursor = parent.cursor;
        s.pointer_events = parent.pointer_events;
        s.user_select = parent.user_select;
        s.custom_properties = Rc::clone(&parent.custom_properties);
        s
    }

    pub fn effective_border_top_width(&self) -> f32 {
        if matches!(self.border_top_style, BorderStyle::None | BorderStyle::Hidden) {
            0.0
        } else {
            self.border_top_width
        }
    }

    pub fn effective_border_right_width(&self) -> f32 {
        if matches!(
            self.border_right_style,
            BorderStyle::None | BorderStyle::Hidden
        ) {
            0.0
        } else {
            self.border_right_width
        }
    }

    pub fn effective_border_bottom_width(&self) -> f32 {
        if matches!(
            self.border_bottom_style,
            BorderStyle::None | BorderStyle::Hidden
        ) {
            0.0
        } else {
            self.border_bottom_width
        }
    }

    pub fn effective_border_left_width(&self) -> f32 {
        if matches!(
            self.border_left_style,
            BorderStyle::None | BorderStyle::Hidden
        ) {
            0.0
        } else {
            self.border_left_width
        }
    }
}

pub fn is_inherited_property(prop: &str) -> bool {
    if prop.starts_with("--") {
        return true;
    }
    matches!(
        prop,
        "color"
            | "font-family"
            | "font-size"
            | "font-weight"
            | "font-style"
            | "line-height"
            | "letter-spacing"
            | "word-spacing"
            | "text-align"
            | "text-transform"
            | "text-indent"
            | "text-shadow"
            | "white-space"
            | "word-break"
            | "overflow-wrap"
            | "visibility"
            | "border-collapse"
            | "border-spacing"
            | "list-style-type"
            | "list-style-position"
            | "cursor"
            | "pointer-events"
            | "user-select"
    )
}

pub fn get_computed_property(style: &ComputedStyle, prop: &str) -> String {
    if prop.starts_with("--") {
        return style
            .custom_properties
            .get(prop)
            .map(|t| crate::tokenizer::serialize_tokens(t))
            .unwrap_or_default();
    }
    match prop.to_ascii_lowercase().as_str() {
        "display" => style.display.as_css_str().to_string(),
        "position" => style.position.as_css_str().to_string(),
        "box-sizing" => style.box_sizing.as_css_str().to_string(),
        "float" => style.float.as_css_str().to_string(),
        "clear" => style.clear.as_css_str().to_string(),
        "z-index" => style
            .z_index
            .map(|z| z.to_string())
            .unwrap_or_else(|| "auto".to_string()),
        "width" => style.width.to_css_string(),
        "height" => style.height.to_css_string(),
        "min-width" => style.min_width.to_css_string(),
        "max-width" => style.max_width.to_css_string(),
        "min-height" => style.min_height.to_css_string(),
        "max-height" => style.max_height.to_css_string(),
        "aspect-ratio" => style
            .aspect_ratio
            .map(trim_float)
            .unwrap_or_else(|| "auto".to_string()),
        "top" => style.top.to_css_string(),
        "right" => style.right.to_css_string(),
        "bottom" => style.bottom.to_css_string(),
        "left" => style.left.to_css_string(),
        "margin-top" => style.margin_top.to_css_string(),
        "margin-right" => style.margin_right.to_css_string(),
        "margin-bottom" => style.margin_bottom.to_css_string(),
        "margin-left" => style.margin_left.to_css_string(),
        "padding-top" => style.padding_top.to_css_string(),
        "padding-right" => style.padding_right.to_css_string(),
        "padding-bottom" => style.padding_bottom.to_css_string(),
        "padding-left" => style.padding_left.to_css_string(),
        "border-top-width" => format_px(style.effective_border_top_width()),
        "border-right-width" => format_px(style.effective_border_right_width()),
        "border-bottom-width" => format_px(style.effective_border_bottom_width()),
        "border-left-width" => format_px(style.effective_border_left_width()),
        "border-top-style" => style.border_top_style.as_css_str().to_string(),
        "border-right-style" => style.border_right_style.as_css_str().to_string(),
        "border-bottom-style" => style.border_bottom_style.as_css_str().to_string(),
        "border-left-style" => style.border_left_style.as_css_str().to_string(),
        "border-top-color" => style.border_top_color.to_css_string(),
        "border-right-color" => style.border_right_color.to_css_string(),
        "border-bottom-color" => style.border_bottom_color.to_css_string(),
        "border-left-color" => style.border_left_color.to_css_string(),
        "border-top-left-radius" => style.border_top_left_radius.to_css_string(),
        "border-top-right-radius" => style.border_top_right_radius.to_css_string(),
        "border-bottom-right-radius" => style.border_bottom_right_radius.to_css_string(),
        "border-bottom-left-radius" => style.border_bottom_left_radius.to_css_string(),
        "border-collapse" => style.border_collapse.as_css_str().to_string(),
        "table-layout" => style.table_layout.as_css_str().to_string(),
        "outline-width" => format_px(style.outline_width),
        "outline-style" => style.outline_style.as_css_str().to_string(),
        "outline-color" => style.outline_color.to_css_string(),
        "outline-offset" => format_px(style.outline_offset),
        "color" => style.color.to_css_string(),
        "background-color" => style.background_color.to_css_string(),
        "background-repeat" => style.background_repeat.as_css_str().to_string(),
        "font-family" => style.font_family.join(", "),
        "font-size" => format_px(style.font_size),
        "font-weight" => style.font_weight.to_string(),
        "font-style" => style.font_style.as_css_str().to_string(),
        "line-height" => match style.line_height {
            LineHeight::Normal => "normal".to_string(),
            LineHeight::Number(n) => format_px(style.font_size * n),
            LineHeight::Px(px) => format_px(px),
        },
        "letter-spacing" => {
            if style.letter_spacing == 0.0 {
                "normal".to_string()
            } else {
                format_px(style.letter_spacing)
            }
        }
        "word-spacing" => format_px(style.word_spacing),
        "text-align" => style.text_align.as_css_str().to_string(),
        "text-decoration-line" | "text-decoration" => {
            style.text_decoration_line.as_css_str().to_string()
        }
        "text-decoration-style" => style.text_decoration_style.as_css_str().to_string(),
        "text-decoration-color" => style
            .text_decoration_color
            .unwrap_or(style.color)
            .to_css_string(),
        "text-transform" => style.text_transform.as_css_str().to_string(),
        "text-indent" => format_px(style.text_indent),
        "text-overflow" => style.text_overflow.as_css_str().to_string(),
        "white-space" => style.white_space.as_css_str().to_string(),
        "word-break" => style.word_break.as_css_str().to_string(),
        "overflow-wrap" => style.overflow_wrap.as_css_str().to_string(),
        "vertical-align" => style.vertical_align.as_css_str().to_string(),
        "opacity" => trim_float(style.opacity),
        "visibility" => style.visibility.as_css_str().to_string(),
        "overflow-x" => style.overflow_x.as_css_str().to_string(),
        "overflow-y" => style.overflow_y.as_css_str().to_string(),
        "overflow" => {
            if style.overflow_x == style.overflow_y {
                style.overflow_x.as_css_str().to_string()
            } else {
                format!(
                    "{} {}",
                    style.overflow_x.as_css_str(),
                    style.overflow_y.as_css_str()
                )
            }
        }
        "flex-direction" => style.flex_direction.as_css_str().to_string(),
        "flex-wrap" => style.flex_wrap.as_css_str().to_string(),
        "flex-grow" => trim_float(style.flex_grow),
        "flex-shrink" => trim_float(style.flex_shrink),
        "flex-basis" => style.flex_basis.to_css_string(),
        "justify-content" => style.justify_content.as_css_str().to_string(),
        "align-items" => style.align_items.as_css_str().to_string(),
        "align-self" => style.align_self.as_css_str().to_string(),
        "align-content" => style.align_content.as_css_str().to_string(),
        "justify-items" => style.justify_items.as_css_str().to_string(),
        "justify-self" => style.justify_self.as_css_str().to_string(),
        "order" => style.order.to_string(),
        "row-gap" => style.row_gap.to_css_string(),
        "column-gap" => style.column_gap.to_css_string(),
        "grid-auto-flow" => style.grid_auto_flow.as_css_str().to_string(),
        "list-style-type" => style.list_style_type.as_css_str().to_string(),
        "list-style-position" => style.list_style_position.as_css_str().to_string(),
        "content" => style
            .content
            .as_ref()
            .map(|c| format!("\"{}\"", c))
            .unwrap_or_else(|| "normal".to_string()),
        "cursor" => style.cursor.as_css_str().to_string(),
        "pointer-events" => style.pointer_events.as_css_str().to_string(),
        "user-select" => style.user_select.as_css_str().to_string(),
        "object-fit" => style.object_fit.as_css_str().to_string(),
        _ => String::new(),
    }
}

pub fn ua_stylesheet() -> Stylesheet {
    parse_stylesheet(UA_CSS)
}

const UA_CSS: &str = "
head, title, meta, link, style, script, noscript, template, datalist, dialog:not([open]), [hidden] {
    display: none;
}
html, body, div, section, article, aside, header, footer, main, nav, address, figure, figcaption,
details, summary, hgroup, search, p, blockquote, pre, hr, ul, ol, dl, dt, dd, form, fieldset,
legend, center {
    display: block;
}
html {
    font-size: 16px;
    color: #000000;
}
body {
    margin: 8px;
}
h1 { display: block; font-size: 2em; font-weight: bold; margin: 0.67em 0; }
h2 { display: block; font-size: 1.5em; font-weight: bold; margin: 0.83em 0; }
h3 { display: block; font-size: 1.17em; font-weight: bold; margin: 1em 0; }
h4 { display: block; font-size: 1em; font-weight: bold; margin: 1.33em 0; }
h5 { display: block; font-size: 0.83em; font-weight: bold; margin: 1.67em 0; }
h6 { display: block; font-size: 0.67em; font-weight: bold; margin: 2.33em 0; }
p { margin: 1em 0; }
blockquote { margin: 1em 40px; }
pre, code, kbd, samp, tt { font-family: monospace; }
pre { white-space: pre; margin: 1em 0; }
b, strong { font-weight: bold; }
i, em, cite, dfn, var { font-style: italic; }
u, ins { text-decoration: underline; }
s, strike, del { text-decoration: line-through; }
small { font-size: 0.833em; }
big { font-size: 1.17em; }
sub { font-size: 0.833em; vertical-align: sub; }
sup { font-size: 0.833em; vertical-align: super; }
mark { background-color: yellow; color: black; }
hr { border-top: 1px solid #808080; margin: 0.5em 0; }
ul, ol { margin: 1em 0; padding-left: 40px; }
ul { list-style-type: disc; }
ol { list-style-type: decimal; }
li { display: list-item; }
dd { margin-left: 40px; }
a:any-link { color: #0000ee; text-decoration: underline; cursor: pointer; }
a:visited { color: #551a8b; }
a:active { color: #ff0000; }
table { display: table; border-collapse: separate; border-spacing: 2px; box-sizing: border-box; text-align: start; }
thead { display: table-header-group; vertical-align: middle; }
tbody { display: table-row-group; vertical-align: middle; }
tfoot { display: table-footer-group; vertical-align: middle; }
tr { display: table-row; vertical-align: inherit; }
td, th { display: table-cell; vertical-align: inherit; padding: 1px; }
th { font-weight: bold; text-align: center; }
caption { display: table-caption; text-align: center; }
img, svg, canvas, video, iframe { display: inline-block; }
button, input, select, textarea {
    display: inline-block;
    font-family: sans-serif;
    font-size: 13.333px;
    box-sizing: border-box;
}
button, input[type=button], input[type=submit], input[type=reset] {
    padding: 4px 10px;
    border: 1px solid #94a3b8;
    border-radius: 4px;
    background-color: #f1f5f9;
    color: #0f172a;
    cursor: pointer;
    text-align: center;
}
input, textarea, select {
    padding: 4px 8px;
    border: 1px solid #b4bcc6;
    border-radius: 4px;
    background-color: #ffffff;
    color: #0f172a;
}
input[type=checkbox i], input[type=radio i] {
    padding: 0;
    border: 0;
    background-color: transparent;
}
input[type=hidden i] { display: none; }
textarea { white-space: pre-wrap; }
fieldset { border: 2px groove #c0c0c0; padding: 0.35em 0.75em 0.625em; margin: 0 2px; }
details:not([open]) > :not(summary) { display: none; }
summary { cursor: pointer; font-weight: bold; }
center { text-align: center; }
center > table, center > div { margin-left: auto; margin-right: auto; }
";
