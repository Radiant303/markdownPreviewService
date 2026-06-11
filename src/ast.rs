use pulldown_cmark::{Alignment as MdAlignment, CodeBlockKind, Event, HeadingLevel, Tag, TagEnd};

// ═══════════════════════════════════════════════════════════════════════════════
//  AST types
// ═══════════════════════════════════════════════════════════════════════════════

#[derive(Clone, Debug)]
pub(crate) enum Inline {
    Text(String),
    Bold(String),
    Italic(String),
    Code(String),
    Math(String),
}

#[derive(Clone, Debug)]
pub(crate) enum Node {
    Heading {
        level: u8,
        content: Vec<Inline>,
    },
    Paragraph(Vec<Inline>),
    Quote {
        children: Vec<Node>,
    },
    List {
        ordered: bool,
        start: Option<u64>,
        items: Vec<Node>,
    },
    ListItem {
        children: Vec<Node>,
    },
    CodeBlock {
        language: String,
        content: String,
    },
    MathBlock {
        latex: String,
    },
    Table {
        alignments: Vec<TableAlignment>,
        header: Vec<Vec<Inline>>,
        rows: Vec<Vec<Vec<Inline>>>,
    },
    Rule,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TableAlignment {
    None,
    Left,
    Center,
    Right,
}

// ═══════════════════════════════════════════════════════════════════════════════
//  AST Builder
// ═══════════════════════════════════════════════════════════════════════════════

#[derive(Clone, Copy)]
enum InlineStyle {
    Normal,
    Bold,
    Italic,
}

enum Container {
    Root(Vec<Node>),
    BlockQuote(Vec<Node>),
    List {
        ordered: bool,
        start: Option<u64>,
        items: Vec<Node>,
    },
    ListItem(Vec<Node>),
}

struct TableBuilder {
    alignments: Vec<TableAlignment>,
    header: Vec<Vec<Inline>>,
    rows: Vec<Vec<Vec<Inline>>>,
    current_row: Vec<Vec<Inline>>,
    in_head: bool,
    in_cell: bool,
}

pub(crate) struct AstBuilder {
    stack: Vec<Container>,
    current_text: String,
    style_stack: Vec<InlineStyle>,
    inlines: Vec<Inline>,
    // Code block state
    in_code: bool,
    code_lang: String,
    code_buf: String,
    // Heading state
    heading_level: u8,
    // Table state
    table: Option<TableBuilder>,
}

impl AstBuilder {
    pub(crate) fn build<'a>(events: impl Iterator<Item = Event<'a>>) -> Vec<Node> {
        let mut b = AstBuilder {
            stack: vec![Container::Root(Vec::new())],
            current_text: String::new(),
            style_stack: vec![InlineStyle::Normal],
            inlines: Vec::new(),
            in_code: false,
            code_lang: String::new(),
            code_buf: String::new(),
            heading_level: 0,
            table: None,
        };

        for event in events {
            b.handle_event(event);
        }

        // Flush any remaining text
        b.flush_text();
        b.flush_inlines_as_paragraph();

        // Unwrap root
        match b.stack.pop() {
            Some(Container::Root(nodes)) => nodes,
            _ => Vec::new(),
        }
    }

    fn push_node(&mut self, node: Node) {
        match self.stack.last_mut() {
            Some(Container::Root(children))
            | Some(Container::BlockQuote(children))
            | Some(Container::ListItem(children)) => children.push(node),
            Some(Container::List { items, .. }) => items.push(node),
            None => {}
        }
    }

    fn push_container(&mut self, c: Container) {
        self.stack.push(c);
    }

    fn current_style(&self) -> InlineStyle {
        *self.style_stack.last().unwrap_or(&InlineStyle::Normal)
    }

    fn flush_text(&mut self) {
        let text = std::mem::take(&mut self.current_text);
        if text.is_empty() {
            return;
        }
        let inline = match self.current_style() {
            InlineStyle::Normal => Inline::Text(text),
            InlineStyle::Bold => Inline::Bold(text),
            InlineStyle::Italic => Inline::Italic(text),
        };
        self.inlines.push(inline);
    }

    fn flush_inlines_as_paragraph(&mut self) {
        if self.table.as_ref().is_some_and(|table| table.in_cell) {
            return;
        }

        let inlines = std::mem::take(&mut self.inlines);
        if !inlines.is_empty() {
            self.push_node(Node::Paragraph(inlines));
        }
    }

    fn handle_event(&mut self, event: Event<'_>) {
        match event {
            // ── Code blocks ───────────────────────────────────────────
            Event::Start(Tag::CodeBlock(kind)) => {
                self.in_code = true;
                self.code_lang = match kind {
                    CodeBlockKind::Fenced(lang) => lang.trim().to_string(),
                    CodeBlockKind::Indented => String::new(),
                };
                self.code_buf.clear();
            }
            Event::End(TagEnd::CodeBlock) => {
                let language = std::mem::take(&mut self.code_lang);
                let content = std::mem::take(&mut self.code_buf);
                self.push_node(Node::CodeBlock { language, content });
                self.in_code = false;
            }

            // ── Tables ────────────────────────────────────────────────
            Event::Start(Tag::Table(alignments)) => {
                self.flush_text();
                self.flush_inlines_as_paragraph();
                self.table = Some(TableBuilder {
                    alignments: alignments.into_iter().map(TableAlignment::from).collect(),
                    header: Vec::new(),
                    rows: Vec::new(),
                    current_row: Vec::new(),
                    in_head: false,
                    in_cell: false,
                });
            }
            Event::End(TagEnd::Table) => {
                self.flush_text();
                if let Some(table) = self.table.take() {
                    if !table.header.is_empty() || !table.rows.is_empty() {
                        self.push_node(Node::Table {
                            alignments: table.alignments,
                            header: table.header,
                            rows: table.rows,
                        });
                    }
                }
            }
            Event::Start(Tag::TableHead) => {
                if let Some(table) = self.table.as_mut() {
                    table.in_head = true;
                }
            }
            Event::End(TagEnd::TableHead) => {
                if let Some(table) = self.table.as_mut() {
                    if !table.current_row.is_empty() {
                        table.header = std::mem::take(&mut table.current_row);
                    }
                    table.in_head = false;
                }
            }
            Event::Start(Tag::TableRow) => {
                if let Some(table) = self.table.as_mut() {
                    table.current_row.clear();
                }
            }
            Event::End(TagEnd::TableRow) => {
                self.flush_text();
                let in_head = self.table.as_ref().is_some_and(|table| table.in_head);
                let row = self
                    .table
                    .as_mut()
                    .map(|table| std::mem::take(&mut table.current_row))
                    .unwrap_or_default();

                if !row.is_empty() {
                    if let Some(table) = self.table.as_mut() {
                        if in_head {
                            table.header = row;
                        } else {
                            table.rows.push(row);
                        }
                    }
                }
            }
            Event::Start(Tag::TableCell) => {
                self.flush_text();
                self.inlines.clear();
                if let Some(table) = self.table.as_mut() {
                    table.in_cell = true;
                }
            }
            Event::End(TagEnd::TableCell) => {
                self.flush_text();
                let cell = std::mem::take(&mut self.inlines);
                if let Some(table) = self.table.as_mut() {
                    table.current_row.push(cell);
                    table.in_cell = false;
                }
            }

            // ── Headings ─────────────────────────────────────────────
            Event::Start(Tag::Heading { level, .. }) => {
                self.flush_text();
                self.flush_inlines_as_paragraph();
                self.heading_level = match level {
                    HeadingLevel::H1 => 1,
                    HeadingLevel::H2 => 2,
                    _ => 3,
                };
                self.inlines.clear();
            }
            Event::End(TagEnd::Heading(_)) => {
                self.flush_text();
                let content = std::mem::take(&mut self.inlines);
                if !content.is_empty() {
                    self.push_node(Node::Heading {
                        level: self.heading_level,
                        content,
                    });
                }
            }

            // ── Block quotes ────────────────────────────────────────────
            Event::Start(Tag::BlockQuote(_)) => {
                self.flush_text();
                self.flush_inlines_as_paragraph();
                self.push_container(Container::BlockQuote(Vec::new()));
            }
            Event::End(TagEnd::BlockQuote(_)) => {
                self.flush_text();
                self.flush_inlines_as_paragraph();
                if let Some(Container::BlockQuote(children)) = self.stack.pop() {
                    self.push_node(Node::Quote { children });
                }
            }

            // ── Lists ─────────────────────────────────────────────────
            Event::Start(Tag::List(start)) => {
                self.flush_text();
                self.flush_inlines_as_paragraph();
                self.push_container(Container::List {
                    ordered: start.is_some(),
                    start,
                    items: Vec::new(),
                });
            }
            Event::End(TagEnd::List(_)) => {
                self.flush_text();
                self.flush_inlines_as_paragraph();
                if let Some(Container::List {
                    ordered,
                    start,
                    items,
                }) = self.stack.pop()
                {
                    self.push_node(Node::List {
                        ordered,
                        start,
                        items,
                    });
                }
            }
            Event::Start(Tag::Item) => {
                self.flush_text();
                self.flush_inlines_as_paragraph();
                self.push_container(Container::ListItem(Vec::new()));
            }
            Event::End(TagEnd::Item) => {
                self.flush_text();
                self.flush_inlines_as_paragraph();
                if let Some(Container::ListItem(children)) = self.stack.pop() {
                    self.push_node(Node::ListItem { children });
                }
            }

            // ── Paragraphs ────────────────────────────────────────────
            Event::Start(Tag::Paragraph) => {
                self.flush_text();
                self.inlines.clear();
            }
            Event::End(TagEnd::Paragraph) => {
                self.flush_text();
                let inlines = std::mem::take(&mut self.inlines);
                if !inlines.is_empty() {
                    if self.table.as_ref().is_some_and(|table| table.in_cell) {
                        self.inlines = inlines;
                    } else {
                        self.push_node(Node::Paragraph(inlines));
                    }
                }
            }

            // ── Inline styles ─────────────────────────────────────────
            Event::Start(Tag::Emphasis) => {
                self.flush_text();
                self.style_stack.push(InlineStyle::Italic);
            }
            Event::End(TagEnd::Emphasis) => {
                self.flush_text();
                if self.style_stack.len() > 1 {
                    self.style_stack.pop();
                }
            }
            Event::Start(Tag::Strong) => {
                self.flush_text();
                self.style_stack.push(InlineStyle::Bold);
            }
            Event::End(TagEnd::Strong) => {
                self.flush_text();
                if self.style_stack.len() > 1 {
                    self.style_stack.pop();
                }
            }

            // ── Leaf events ───────────────────────────────────────────
            Event::Text(t) => {
                if self.in_code {
                    self.code_buf.push_str(&t);
                } else {
                    self.current_text.push_str(&t);
                }
            }
            Event::Code(inline) => {
                self.flush_text();
                self.inlines.push(Inline::Code(inline.to_string()));
            }
            Event::InlineMath(math) => {
                self.flush_text();
                self.inlines.push(Inline::Math(math.to_string()));
            }
            Event::DisplayMath(math) => {
                self.flush_text();
                if self.table.as_ref().is_some_and(|table| table.in_cell) {
                    self.inlines.push(Inline::Math(math.to_string()));
                } else {
                    self.flush_inlines_as_paragraph();
                    self.push_node(Node::MathBlock {
                        latex: math.to_string(),
                    });
                }
            }
            Event::SoftBreak | Event::HardBreak => {
                if self.in_code {
                    self.code_buf.push('\n');
                } else {
                    self.current_text.push('\n');
                }
            }
            Event::TaskListMarker(checked) => {
                let marker = if checked { "☑ " } else { "☐ " };
                self.current_text.push_str(marker);
            }
            Event::Rule => {
                self.flush_text();
                self.flush_inlines_as_paragraph();
                self.push_node(Node::Rule);
            }
            _ => {}
        }
    }
}

impl From<MdAlignment> for TableAlignment {
    fn from(value: MdAlignment) -> Self {
        match value {
            MdAlignment::None => Self::None,
            MdAlignment::Left => Self::Left,
            MdAlignment::Center => Self::Center,
            MdAlignment::Right => Self::Right,
        }
    }
}
