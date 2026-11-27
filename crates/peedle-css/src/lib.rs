pub mod cascade;
pub mod parser;
pub mod selector;
pub mod style;
pub mod tokenizer;
pub mod values;

pub use cascade::{
    compute_styles, compute_styles_for_root, compute_styles_with_ua, compute_styles_with_ua_root,
    StyleMap,
};
pub use parser::{
    is_supported_declaration, parse_inline_declarations, parse_media_query_list,
    parse_stylesheet, parse_supports_condition, Declaration, FontFaceRule, ImportRule,
    KeyframeStop, KeyframesRule, LayerBlockRule, LayerStatementRule, MediaCondition, MediaFeature,
    MediaQuery, MediaQueryList, MediaType, RangeOp, Rule, StyleRule, Stylesheet, SupportsCondition,
    Viewport,
};
pub use selector::{
    desugar_nested_selectors, matches_complex_selector, matches_selector, matches_selector_list,
    parse_an_plus_b, parse_forgiving_selector_list, parse_relative_selector_list,
    parse_selector_list, parse_selector_list_from_tokens, query_selector, query_selector_all,
    AnPlusB, AttrOperator, AttrSelector, CaseSensitivity, Combinator, ComplexSelector,
    CompoundSelector, PseudoClass, PseudoElement, RelativeSelector, SelectorList, SimpleSelector,
    Specificity,
};
pub use style::{
    get_computed_property, is_inherited_property, ua_stylesheet, AlignContent, AlignItems,
    AlignSelf, BackgroundClip, BackgroundRepeat, BackgroundSize, BorderCollapse, BorderStyle,
    BoxSizing, Clear, ComputedStyle, Cursor, Display, FlexDirection, FlexWrap, Float, FontStyle,
    GridAutoFlow, JustifyContent, LineHeight, ListStylePosition, ListStyleType, ObjectFit,
    Overflow, OverflowWrap, PointerEvents, Position, TableLayout, TextAlign, TextDecorationLine,
    TextDecorationStyle, TextOverflow, TextTransform, UserSelect, VerticalAlign, Visibility,
    WhiteSpace, WordBreak,
};
pub use tokenizer::{serialize_tokens, tokenize, HashKind, NumericKind, Token, Tokenizer};
pub use values::{
    parse_angle_deg, parse_background_image, parse_box_shadows, parse_calc_expr,
    parse_calc_function, parse_color, parse_color_from_tokens, parse_color_value, parse_filters,
    parse_grid_placement, parse_grid_tracks, parse_length_percentage_auto, parse_length_unit,
    parse_text_shadows, parse_transforms, BackgroundImage, BoxShadow, CalcContext, CalcExpr, Color,
    ColorValue, FilterOp, GradientStop, GridPlacement, GridRepeatCount, GridTrackSize, Length,
    LengthPercentageAuto, LengthUnit, TextShadow, TransformOp,
};

pub fn version() -> &'static str {
    "0.1.0"
}
