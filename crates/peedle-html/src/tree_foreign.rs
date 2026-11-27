use crate::dom::{Attribute, Document, Namespace, NodeId};

pub fn adjust_svg_tag_name(name: &str) -> String {
    match name {
        "altglyph" => "altGlyph",
        "altglyphdef" => "altGlyphDef",
        "altglyphitem" => "altGlyphItem",
        "animatecolor" => "animateColor",
        "animatemotion" => "animateMotion",
        "animatetransform" => "animateTransform",
        "clippath" => "clipPath",
        "feblend" => "feBlend",
        "fecolormatrix" => "feColorMatrix",
        "fecomponenttransfer" => "feComponentTransfer",
        "fecomposite" => "feComposite",
        "feconvolvematrix" => "feConvolveMatrix",
        "fediffuselighting" => "feDiffuseLighting",
        "fedisplacementmap" => "feDisplacementMap",
        "fedistantlight" => "feDistantLight",
        "fedropshadow" => "feDropShadow",
        "feflood" => "feFlood",
        "fefunca" => "feFuncA",
        "fefuncb" => "feFuncB",
        "fefuncg" => "feFuncG",
        "fefuncr" => "feFuncR",
        "fegaussianblur" => "feGaussianBlur",
        "feimage" => "feImage",
        "femerge" => "feMerge",
        "femergenode" => "feMergeNode",
        "femorphology" => "feMorphology",
        "feoffset" => "feOffset",
        "fepointlight" => "fePointLight",
        "fespecularlighting" => "feSpecularLighting",
        "fespotlight" => "feSpotLight",
        "fetile" => "feTile",
        "feturbulence" => "feTurbulence",
        "foreignobject" => "foreignObject",
        "glyphref" => "glyphRef",
        "lineargradient" => "linearGradient",
        "radialgradient" => "radialGradient",
        "textpath" => "textPath",
        other => other,
    }
    .to_string()
}

pub fn adjust_svg_attributes(attrs: &mut [Attribute]) {
    for attr in attrs.iter_mut() {
        let fixed = match attr.name.as_str() {
            "attributename" => "attributeName",
            "attributetype" => "attributeType",
            "basefrequency" => "baseFrequency",
            "baseprofile" => "baseProfile",
            "calcmode" => "calcMode",
            "clippathunits" => "clipPathUnits",
            "diffuseconstant" => "diffuseConstant",
            "edgemode" => "edgeMode",
            "filterunits" => "filterUnits",
            "glyphref" => "glyphRef",
            "gradienttransform" => "gradientTransform",
            "gradientunits" => "gradientUnits",
            "kernelmatrix" => "kernelMatrix",
            "kernelunitlength" => "kernelUnitLength",
            "keypoints" => "keyPoints",
            "keysplines" => "keySplines",
            "keytimes" => "keyTimes",
            "lengthadjust" => "lengthAdjust",
            "limitingconeangle" => "limitingConeAngle",
            "markerheight" => "markerHeight",
            "markerunits" => "markerUnits",
            "markerwidth" => "markerWidth",
            "maskcontentunits" => "maskContentUnits",
            "maskunits" => "maskUnits",
            "numoctaves" => "numOctaves",
            "pathlength" => "pathLength",
            "patterncontentunits" => "patternContentUnits",
            "patterntransform" => "patternTransform",
            "patternunits" => "patternUnits",
            "pointsatx" => "pointsAtX",
            "pointsaty" => "pointsAtY",
            "pointsatz" => "pointsAtZ",
            "preservealpha" => "preserveAlpha",
            "preserveaspectratio" => "preserveAspectRatio",
            "primitiveunits" => "primitiveUnits",
            "refx" => "refX",
            "refy" => "refY",
            "repeatcount" => "repeatCount",
            "repeatdur" => "repeatDur",
            "requiredextensions" => "requiredExtensions",
            "requiredfeatures" => "requiredFeatures",
            "specularconstant" => "specularConstant",
            "specularexponent" => "specularExponent",
            "spreadmethod" => "spreadMethod",
            "startoffset" => "startOffset",
            "stddeviation" => "stdDeviation",
            "stitchtiles" => "stitchTiles",
            "surfacescale" => "surfaceScale",
            "systemlanguage" => "systemLanguage",
            "tablevalues" => "tableValues",
            "targetx" => "targetX",
            "targety" => "targetY",
            "textlength" => "textLength",
            "viewbox" => "viewBox",
            "viewtarget" => "viewTarget",
            "xchannelselector" => "xChannelSelector",
            "ychannelselector" => "yChannelSelector",
            "zoomandpan" => "zoomAndPan",
            _ => continue,
        };
        attr.name = fixed.to_string();
    }
}

pub fn adjust_mathml_attributes(attrs: &mut [Attribute]) {
    for attr in attrs.iter_mut() {
        if attr.name == "definitionurl" {
            attr.name = "definitionURL".to_string();
        }
    }
}

pub fn adjust_foreign_attributes(attrs: &mut [Attribute]) {
    for attr in attrs.iter_mut() {
        let info = match attr.name.as_str() {
            "xlink:actuate" | "xlink:arcrole" | "xlink:href" | "xlink:role" | "xlink:show"
            | "xlink:title" | "xlink:type" => {
                Some(("xlink", "http://www.w3.org/1999/xlink"))
            }
            "xml:lang" | "xml:space" => {
                Some(("xml", "http://www.w3.org/XML/1998/namespace"))
            }
            "xmlns:xlink" => Some(("xmlns", "http://www.w3.org/2000/xmlns/")),
            "xmlns" => Some(("", "http://www.w3.org/2000/xmlns/")),
            _ => None,
        };
        if let Some((prefix, uri)) = info {
            if !prefix.is_empty() {
                attr.prefix = Some(prefix.to_string());
            }
            attr.namespace = Some(uri.to_string());
        }
    }
}

pub fn is_mathml_text_integration_point(doc: &Document, id: NodeId) -> bool {
    let Some(el) = doc.element(id) else {
        return false;
    };
    el.namespace == Namespace::MathMl
        && matches!(el.tag_name.as_str(), "mi" | "mo" | "mn" | "ms" | "mtext")
}

pub fn is_html_integration_point(doc: &Document, id: NodeId) -> bool {
    let Some(el) = doc.element(id) else {
        return false;
    };
    match el.namespace {
        Namespace::Svg => matches!(el.tag_name.as_str(), "foreignObject" | "desc" | "title"),
        Namespace::MathMl => {
            if el.tag_name == "annotation-xml" {
                el.get_attr("encoding").map_or(false, |v| {
                    v.eq_ignore_ascii_case("text/html")
                        || v.eq_ignore_ascii_case("application/xhtml+xml")
                })
            } else {
                false
            }
        }
        Namespace::Html => false,
    }
}

pub fn breaks_out_of_foreign(tag: &str, attrs: &[Attribute]) -> bool {
    if tag == "font" {
        return attrs
            .iter()
            .any(|a| matches!(a.name.as_str(), "color" | "face" | "size"));
    }
    matches!(
        tag,
        "b" | "big"
            | "blockquote"
            | "body"
            | "br"
            | "center"
            | "code"
            | "dd"
            | "div"
            | "dl"
            | "dt"
            | "em"
            | "embed"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "head"
            | "hr"
            | "i"
            | "img"
            | "li"
            | "listing"
            | "menu"
            | "meta"
            | "nobr"
            | "ol"
            | "p"
            | "pre"
            | "ruby"
            | "s"
            | "small"
            | "span"
            | "strong"
            | "strike"
            | "sub"
            | "sup"
            | "table"
            | "tt"
            | "u"
            | "ul"
            | "var"
    )
}
