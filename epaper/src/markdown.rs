//! Turns a message written in Markdown into the tree that text.typ draws.
//!
//! The tree holds only strings and style names, so nothing in a message is
//! ever evaluated as Typst code.

use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use serde::Serialize;

/// A run of text, a line break, or styled children.
#[derive(Debug, PartialEq, Serialize)]
#[serde(untagged)]
pub enum Node {
    Text(String),
    Styled { k: &'static str, c: Vec<Node> },
}

fn line_break() -> Node {
    Node::Styled { k: "br", c: Vec::new() }
}

/// Every line of the message stays a line on the panel; blocks such as
/// paragraphs, headings and list items each start on a new one.
pub fn parse(markdown: &str) -> Vec<Node> {
    let mut tree = Tree::default();
    for event in Parser::new_ext(markdown, Options::ENABLE_STRIKETHROUGH) {
        match event {
            Event::Start(tag) => tree.start(tag),
            Event::End(end) => tree.end(end),
            Event::Text(text) | Event::Html(text) | Event::InlineHtml(text) => tree.text(&text),
            Event::Code(code) => {
                tree.lead();
                tree.frames.push(("code", Vec::new()));
                tree.text(&code);
                tree.close();
            }
            Event::SoftBreak | Event::HardBreak => tree.push(line_break()),
            Event::Rule => tree.new_line = true,
            Event::TaskListMarker(done) => tree.text(if done { "☑ " } else { "☐ " }),
            Event::FootnoteReference(name) => tree.text(&format!("[^{name}]")),
            Event::InlineMath(math) | Event::DisplayMath(math) => tree.text(&math),
        }
    }
    while tree.frames.len() > 1 {
        tree.close();
    }
    tree.frames.pop().map(|(_, nodes)| nodes).unwrap_or_default()
}

struct Tree {
    frames: Vec<(&'static str, Vec<Node>)>,
    lists: Vec<Option<u64>>,
    bullet: Option<String>,
    new_line: bool,
    started: bool,
}

impl Default for Tree {
    fn default() -> Self {
        Tree {
            frames: vec![("span", Vec::new())],
            lists: Vec::new(),
            bullet: None,
            new_line: false,
            started: false,
        }
    }
}

impl Tree {
    fn start(&mut self, tag: Tag) {
        if matches!(
            tag,
            Tag::Emphasis | Tag::Strong | Tag::Strikethrough | Tag::Link { .. } | Tag::Image { .. }
        ) {
            self.lead();
        }
        let style = match tag {
            Tag::Emphasis => "emph",
            Tag::Strong => "strong",
            Tag::Strikethrough => "strike",
            Tag::CodeBlock(_) => "code",
            Tag::Heading {
                level: HeadingLevel::H1,
                ..
            } => "h1",
            Tag::Heading { .. } => "h2",
            Tag::List(first) => {
                self.lists.push(first);
                "span"
            }
            Tag::Item => {
                self.bullet = Some(match self.lists.last_mut() {
                    Some(Some(number)) => {
                        *number += 1;
                        format!("{}. ", *number - 1)
                    }
                    _ => "• ".to_owned(),
                });
                "span"
            }
            _ => "span",
        };
        self.frames.push((style, Vec::new()));
    }

    fn end(&mut self, end: TagEnd) {
        self.close();
        match end {
            TagEnd::Emphasis
            | TagEnd::Strong
            | TagEnd::Strikethrough
            | TagEnd::Superscript
            | TagEnd::Subscript
            | TagEnd::Link
            | TagEnd::Image => {}
            TagEnd::List(_) => {
                self.lists.pop();
                self.new_line = true;
            }
            _ => self.new_line = true,
        }
    }

    fn close(&mut self) {
        let Some((style, nodes)) = self.frames.pop() else {
            return;
        };
        if nodes.is_empty() {
            return;
        }
        if style == "span" {
            for node in nodes {
                self.push(node);
            }
        } else {
            self.push(Node::Styled { k: style, c: nodes });
        }
    }

    fn push(&mut self, node: Node) {
        if let Some((_, nodes)) = self.frames.last_mut() {
            nodes.push(node);
        }
    }

    fn text(&mut self, text: &str) {
        for (i, line) in text.split('\n').enumerate() {
            if i > 0 {
                self.push(line_break());
            }
            if line.is_empty() {
                continue;
            }
            self.lead();
            self.started = true;
            self.push(Node::Text(line.to_owned()));
        }
    }

    /// Puts down what goes before a block's first content: the line break after
    /// the previous block, and the bullet or number of a list item.
    fn lead(&mut self) {
        if self.new_line && self.started {
            self.push(line_break());
        }
        self.new_line = false;
        if let Some(bullet) = self.bullet.take() {
            self.started = true;
            self.push(Node::Text(bullet));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn json(markdown: &str) -> String {
        serde_json::to_string(&parse(markdown)).unwrap()
    }

    #[test]
    fn plain_text_stays_plain() {
        assert_eq!(json("Hoi zäme 👋"), r#"["Hoi zäme 👋"]"#);
    }

    #[test]
    fn emphasis_strong_strike_and_code_are_styles() {
        assert_eq!(
            json("*so* **fescht** ~~nöd~~ `code`"),
            r#"[{"k":"emph","c":["so"]}," ",{"k":"strong","c":["fescht"]}," ",{"k":"strike","c":["nöd"]}," ",{"k":"code","c":["code"]}]"#
        );
    }

    #[test]
    fn every_line_and_block_starts_a_new_line() {
        assert_eq!(
            json("# Titel\neis\nzwei\n\n- a\n- b"),
            r#"[{"k":"h1","c":["Titel"]},{"k":"br","c":[]},"eis",{"k":"br","c":[]},"zwei",{"k":"br","c":[]},"• ","a",{"k":"br","c":[]},"• ","b"]"#
        );
    }

    #[test]
    fn a_styled_list_item_keeps_its_bullet_plain() {
        assert_eq!(
            json("eis\n\n- ~~Salat~~"),
            r#"["eis",{"k":"br","c":[]},"• ",{"k":"strike","c":["Salat"]}]"#
        );
    }

    #[test]
    fn ordered_lists_keep_their_numbers() {
        assert_eq!(
            json("3. drü\n4. vier"),
            r#"["3. ","drü",{"k":"br","c":[]},"4. ","vier"]"#
        );
    }

    #[test]
    fn typst_code_is_just_text() {
        assert_eq!(json(r#"#panic("boom")"#), r##"["#panic(\"boom\")"]"##);
    }
}
