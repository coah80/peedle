use std::collections::HashMap;

use peedle_css::ComputedStyle;
use peedle_html::NodeId;

use crate::geometry::{Dimensions, Rect};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LayoutBoxId(pub usize);

#[derive(Clone, Debug, PartialEq)]
pub enum ReplacedKind {
    Image {
        url: String,
        intrinsic_w: f32,
        intrinsic_h: f32,
    },
    Canvas {
        width: f32,
        height: f32,
    },
    Svg {
        width: f32,
        height: f32,
    },
    Input {
        input_type: String,
        value: String,
        placeholder: String,
        checked: bool,
    },
    Textarea {
        value: String,
        rows: usize,
        cols: usize,
    },
    Select {
        label: String,
    },
    Hr,
    Br,
}

#[derive(Clone, Debug, PartialEq)]
pub enum BoxKind {
    Block,
    Inline,
    InlineBlock,
    Flex,
    Grid,
    Table,
    TableRowGroup,
    TableRow,
    TableCell { colspan: usize, rowspan: usize },
    Text(String),
    Replaced(ReplacedKind),
    AnonymousBlock,
}

#[derive(Clone, Debug)]
pub struct TextFragment {
    pub text: String,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub baseline: f32,
}

#[derive(Clone, Debug)]
pub struct LayoutBox {
    pub id: LayoutBoxId,
    pub node: Option<NodeId>,
    pub kind: BoxKind,
    pub style: ComputedStyle,
    pub dims: Dimensions,
    pub children: Vec<LayoutBoxId>,
    pub text_fragments: Vec<TextFragment>,
    pub list_marker: Option<String>,
}

impl LayoutBox {
    pub fn border_rect(&self) -> Rect {
        self.dims.border_box()
    }

    pub fn content_rect(&self) -> Rect {
        self.dims.content
    }
}

#[derive(Clone, Debug, Default)]
pub struct LayoutTree {
    pub boxes: Vec<LayoutBox>,
    pub root: Option<LayoutBoxId>,
    pub node_to_box: HashMap<NodeId, LayoutBoxId>,
    pub viewport_width: f32,
    pub viewport_height: f32,
    pub content_width: f32,
    pub content_height: f32,
}

impl LayoutTree {
    pub fn get(&self, id: LayoutBoxId) -> &LayoutBox {
        &self.boxes[id.0]
    }

    pub fn get_mut(&mut self, id: LayoutBoxId) -> &mut LayoutBox {
        &mut self.boxes[id.0]
    }

    pub fn alloc(&mut self, node: Option<NodeId>, kind: BoxKind, style: ComputedStyle) -> LayoutBoxId {
        let id = LayoutBoxId(self.boxes.len());
        if let Some(nid) = node {
            self.node_to_box.entry(nid).or_insert(id);
        }
        self.boxes.push(LayoutBox {
            id,
            node,
            kind,
            style,
            dims: Dimensions::default(),
            children: Vec::new(),
            text_fragments: Vec::new(),
            list_marker: None,
        });
        id
    }

    pub fn box_of(&self, node: NodeId) -> Option<&LayoutBox> {
        let bid = *self.node_to_box.get(&node)?;
        self.boxes.get(bid.0)
    }

    pub fn bounding_rect(&self, node: NodeId) -> Option<Rect> {
        Some(self.box_of(node)?.border_rect())
    }

    pub fn hit_test(&self, x: f32, y: f32) -> Option<NodeId> {
        let root = self.root?;
        self.hit_test_box(root, x, y)
    }

    fn hit_test_box(&self, id: LayoutBoxId, x: f32, y: f32) -> Option<NodeId> {
        let b = self.get(id);
        if b.style.visibility == peedle_css::Visibility::Hidden
            || b.style.pointer_events == peedle_css::PointerEvents::None
        {
            return None;
        }
        for &child in b.children.iter().rev() {
            if let Some(hit) = self.hit_test_box(child, x, y) {
                return Some(hit);
            }
        }
        if b.border_rect().contains(x, y) {
            return b.node;
        }
        for frag in &b.text_fragments {
            let r = Rect::new(frag.x, frag.y, frag.width, frag.height);
            if r.contains(x, y) {
                return b.node;
            }
        }
        None
    }
}
