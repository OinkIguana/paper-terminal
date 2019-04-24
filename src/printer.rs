use std::process::{Command, Stdio};
use std::io::{Read as _, Write as _};
use ansi_term::Style;
use pulldown_cmark::{Alignment, Event, Tag};
use image::{self, GenericImageView as _};
use console::measure_text_width;
use syncat_stylesheet::Stylesheet;
use termpix;
use crate::words::Words;
use crate::table::Table;

#[derive(Debug, PartialEq)]
enum Scope {
    Paper,
    Indent,
    Italic,
    Bold,
    Strikethrough,
    Link,
    Caption,
    Rule,
    FootnoteDefinition,
    FootnoteReference,
    FootnoteContent,
    List(Option<usize>),
    ListItem(Option<usize>, bool),
    Code,
    CodeBlock(String),
    BlockQuote,
    Table(Vec<Alignment>),
    TableHead,
    TableRow,
    TableCell,
    Heading(i32),
}

impl Scope {
    fn prefix_len(&self) -> usize {
        match self {
            Scope::Indent => 4,
            Scope::FootnoteContent => 4,
            Scope::ListItem(..) => 4,
            Scope::CodeBlock(..) => 2,
            Scope::BlockQuote => 4,
            Scope::Heading(2) => 5,
            Scope::Heading(..) => 4,
            _ => 0
        }
    }

    fn prefix(&mut self) -> String {
        match self {
            Scope::Indent => "    ".to_string(),
            Scope::FootnoteContent => "    ".to_string(),
            Scope::ListItem(Some(index), ref mut handled) => {
                if *handled {
                    "    ".to_string()
                } else {
                    *handled = true;
                    format!("{: <4}", format!("{}.", index))
                }
            }
            Scope::ListItem(None, ref mut handled) => {
                if *handled {
                    "    ".to_string()
                } else {
                    *handled = true;
                    "•   ".to_string()
                }
            }
            Scope::CodeBlock(..) => "  ".to_string(),
            Scope::BlockQuote => "│   ".to_string(),
            Scope::Heading(1) => "    ".to_string(),
            Scope::Heading(2) => "├─── ".to_string(),
            Scope::Heading(3) => "    ".to_string(),
            Scope::Heading(4) => "    ".to_string(),
            Scope::Heading(5) => "    ".to_string(),
            Scope::Heading(6) => "    ".to_string(),
            _ => String::new(),
        }
    }

    fn suffix_len(&self) -> usize {
        match self {
            Scope::CodeBlock(..) => 2,
            Scope::Heading(2) => 5,
            _ => 0
        }
    }

    fn suffix(&mut self) -> String {
        match self {
            Scope::CodeBlock(..) => "  ".to_string(),
            Scope::Heading(2) => " ───┤".to_string(),
            _ => String::new(),
        }
    }

    fn name(&self) -> &'static str {
        use Scope::*;
        match self {
            Paper => "paper",
            Indent => "indent",
            Italic => "emphasis",
            Bold => "strong",
            Strikethrough => "strikethrough",
            Link => "link",
            Caption => "caption",
            Rule => "hr",
            FootnoteDefinition => "footnote-def",
            FootnoteReference => "footnote-ref",
            FootnoteContent => "footnote",
            List(Some(..)) => "ol",
            List(None) => "ul",
            ListItem(..) => "li",
            Code => "code",
            CodeBlock(..) => "codeblock",
            BlockQuote => "blockquote",
            Table(..) => "table",
            TableHead => "th",
            TableRow => "tr",
            TableCell => "td",
            Heading(1) => "h1",
            Heading(2) => "h2",
            Heading(3) => "h3",
            Heading(4) => "h4",
            Heading(5) => "h5",
            Heading(6) => "h6",
            _ => "",
        }
    }
}

pub struct Printer<'a> {
    centering: &'a str,
    margin: &'a str,
    stylesheet: &'a Stylesheet,
    opts: &'a crate::Opts,
    width: usize,
    buffer: String,
    table: (Vec<String>, Vec<Vec<String>>),
    content: String,
    scope: Vec<Scope>,
    empty_queued: bool,
}

impl<'a> Printer<'a> {
    pub fn new(centering: &'a str, margin: &'a str, width: usize, stylesheet: &'a Stylesheet, opts: &'a crate::Opts) -> Printer<'a> {
        Printer {
            centering,
            margin,
            width,
            stylesheet,
            opts,
            buffer: String::new(),
            table: (vec![], vec![]),
            content: String::new(),
            scope: vec![Scope::Paper],
            empty_queued: false,
        }
    }

    fn prefix_len(&self) -> usize {
        self.scope
            .iter()
            .fold(0, |len, scope| len + scope.prefix_len())
    }

    fn suffix_len(&self) -> usize {
        self.scope
            .iter()
            .fold(0, |len, scope| len + scope.suffix_len())
    }

    fn prefix(&mut self) -> (String, usize) {
        let stylesheet = self.stylesheet;
        self.scope
            .iter_mut()
            .scan(vec![], |scopes, scope| {
                scopes.push(scope.name());
                let prefix = scope.prefix();
                let style = stylesheet.resolve_basic(&scopes[..], Some("prefix")).build();
                Some((format!("{}", style.paint(&prefix)), prefix.chars().count()))
            })
            .fold((String::new(), 0), |(s, c), (s2, c2)| {
                (s + &s2, c + c2)
            })
    }

    fn suffix(&mut self) -> (String, usize) {
        let stylesheet = self.stylesheet;
        self.scope
            .iter_mut()
            .scan(vec![], |scopes, scope| {
                scopes.push(scope.name());
                let suffix = scope.suffix();
                let style = stylesheet.resolve_basic(&scopes[..], Some("suffix")).build();
                Some((format!("{}", style.paint(&suffix)), suffix.chars().count()))
            })
            .fold((String::new(), 0), |(s, c), (s2, c2)| {
                (s2 + &s, c + c2)
            })
    }

    fn style2(&self, token: Option<&str>) -> Style {
        let scope_names: Vec<_> = self.scope.iter().map(Scope::name).collect();
        self.stylesheet.resolve_basic(&scope_names, token).build()
    }

    fn style(&self) -> Style {
        self.style2(None)
    }

    fn shadow(&self) -> String {
        format!("{}", self.stylesheet.resolve_basic(&["shadow"], None).build().paint(" "))
    }

    fn paper_style(&self) -> Style {
        self.stylesheet.resolve_basic(&["paper"], None).build()
    }

    fn queue_empty(&mut self) {
        self.empty_queued = true;
    }

    fn empty(&mut self) {
        let (prefix, prefix_len) = self.prefix();
        let (suffix, suffix_len) = self.suffix();
        println!(
            "{}{}{}{}{}{}{}",
            self.centering,
            self.margin,
            prefix,
            self.paper_style().paint(" ".repeat(self.width - prefix_len - suffix_len)),
            suffix,
            self.margin,
            self.shadow(),
        );
        self.empty_queued = false;
    }

    fn print_rule(&mut self) {
        let (prefix, prefix_len) = self.prefix();
        let (suffix, suffix_len) = self.suffix();
        println!(
            "{}{}{}{}{}{}{}",
            self.centering,
            self.margin,
            prefix,
            self.style().paint("─".repeat(self.width - prefix_len - suffix_len)),
            suffix,
            self.margin,
            self.shadow(),
        );
    }

    fn print_table(&mut self) {
        let alignments = if let Some(Scope::Table(alignments)) = self.scope.last() {
            alignments
        } else { return };
        let (heading, rows) = std::mem::replace(&mut self.table, (vec![], vec![]));
        let available_width = self.width - self.prefix_len() - self.suffix_len();
        let table_str = Table::new(heading, rows, available_width)
            .print(self.paper_style(), alignments);
        for line in table_str.lines() {
            let (prefix, _) = self.prefix();
            let (suffix, _) = self.suffix();
            println!(
                "{}{}{}{}{}{}{}{}",
                self.centering,
                self.margin,
                line,
                prefix,
                self.paper_style().paint(" ".repeat(available_width - measure_text_width(line))),
                suffix,
                self.margin,
                self.shadow(),
            );
        }
    }

    fn flush_buffer(&mut self) {
        match self.scope.last() {
            Some(Scope::CodeBlock(lang)) => {
                let style = if lang.is_empty() || !self.opts.syncat {
                    self.style2(Some("txt"))
                } else {
                    self.style2(Some(lang))
                };
                let lang = lang.to_string();
                let mut first_prefix = Some(self.prefix());
                let mut first_suffix = Some(self.suffix());

                let available_width = self.width
                    - first_prefix.as_ref().unwrap().1
                    - first_suffix.as_ref().unwrap().1;
                let buffer = std::mem::replace(&mut self.buffer, String::new());
                let buffer = if self.opts.syncat {
                    let syncat = Command::new("syncat")
                        .args(&["-l", &lang, "-w", &available_width.to_string()])
                        .stdin(Stdio::piped())
                        .stdout(Stdio::piped())
                        .spawn();
                    match syncat {
                        Ok(syncat) => {
                            {
                                let mut stdin = syncat.stdin.unwrap();
                                write!(stdin, "{}", buffer).unwrap();
                            }
                            let mut output = String::new();
                            syncat.stdout.unwrap().read_to_string(&mut output).unwrap();
                            output
                        }
                        Err(error) => {
                            eprintln!("{}", error);
                            buffer.to_string()
                        }
                    }
                } else {
                    buffer
                        .lines()
                        .map(|mut line| {
                            let mut output = String::new();
                            while line.chars().count() > available_width {
                                let prefix = line.chars().take(available_width).collect::<String>();
                                output = format!("{}{}\n", output, prefix);
                                line = &line[prefix.len()..];
                            }
                            format!("{}{}{}\n", output, line, " ".repeat(available_width - line.chars().count()))
                        })
                        .collect()
                };

                let (prefix, _) = first_prefix.take().unwrap_or_else(|| self.prefix());
                let (suffix, _) = first_suffix.take().unwrap_or_else(|| self.suffix());
                println!(
                    "{}{}{}{}{}{}{}",
                    self.centering,
                    self.margin,
                    prefix,
                    style.paint(" ".repeat(available_width)),
                    suffix,
                    self.margin,
                    self.shadow(),
                );

                for line in buffer.lines() {
                    let (prefix, _) = self.prefix();
                    let (suffix, _) = self.suffix();
                    println!(
                        "{}{}{}{}{}{}{}",
                        self.centering,
                        self.margin,
                        prefix,
                        style.paint(line),
                        suffix,
                        self.margin,
                        self.shadow(),
                    );
                }

                let (prefix, _) = first_prefix.take().unwrap_or_else(|| self.prefix());
                let (suffix, _) = first_suffix.take().unwrap_or_else(|| self.suffix());
                println!(
                    "{}{}{}{}{}{}{}",
                    self.centering,
                    self.margin,
                    prefix,
                    format!("{}{}", style.paint(" ".repeat(available_width - lang.chars().count())), self.style2(Some("lang-tag")).paint(lang)),
                    suffix,
                    self.margin,
                    self.shadow(),
                );

            }
            _ => {}
        }
    }

    fn flush(&mut self) {
        if !self.buffer.is_empty() {
            return;
        }
        if self.scope.iter().find(|scope| if let Scope::Table(..) = scope { true } else { false }).is_some() {
            return;
        }
        if self.content.is_empty() { return }
        let (prefix, prefix_len) = self.prefix();
        let (suffix, suffix_len) = self.suffix();
        println!(
            "{}{}{}{}{}{}{}{}",
            self.centering,
            self.margin,
            prefix,
            self.content,
            suffix,
            self.paper_style().paint(" ".repeat(self.width - measure_text_width(&self.content) - prefix_len - suffix_len)),
            self.margin,
            self.shadow(),
        );
        self.content.clear();
    }

    fn target(&mut self) -> &mut String {
        if self.scope.iter().find(|scope| *scope == &Scope::TableHead).is_some() {
            self.table.0.last_mut().unwrap()
        } else if self.scope.iter().find(|scope| *scope == &Scope::TableRow).is_some() {
            self.table.1.last_mut().unwrap().last_mut().unwrap()
        } else {
            &mut self.content
        }
    }

    fn handle_text<S>(&mut self, text: S) where S: AsRef<str> {
        let s = text.as_ref();
        if let Some(Scope::CodeBlock(..)) = self.scope.last() {
            self.buffer += s;
            return;
        }
        let style = self.style();
        for word in Words::new(s) {
            if measure_text_width(&self.content) + word.len() + self.prefix_len() + self.suffix_len() > self.width {
                self.flush();
            }
            let mut word = if self.target().is_empty() {
                word.trim()
            } else {
                &word
            };
            let available_len = self.width - self.prefix_len() - self.suffix_len();
            while measure_text_width(&self.content) + word.len() > available_len {
                let part = word.chars().take(available_len).collect::<String>();
                self.target().push_str(&format!("{}", style.paint(&part)));
                word = &word[part.len()..];
                self.flush();
            }
            self.target().push_str(&format!("{}", style.paint(word)));
        }
    }

    pub fn handle(&mut self, event: Event) {
        match event {
            Event::Start(tag) => {
                if self.empty_queued {
                    // TODO: queue an empty after an item's initial text when there's a block
                    self.empty();
                }
                match tag {
                    Tag::Paragraph => { self.flush(); }
                    Tag::Rule => {
                        self.flush();
                        self.scope.push(Scope::Rule);
                    }
                    Tag::Header(level) => {
                        self.flush();
                        if level == 1 {
                            self.print_rule();
                        }
                        self.scope.push(Scope::Heading(level));
                    }
                    Tag::BlockQuote => {
                        self.flush();
                        self.scope.push(Scope::BlockQuote);
                    }
                    Tag::CodeBlock(language) => {
                        self.flush();
                        self.scope.push(Scope::CodeBlock(language.to_string()));
                    }
                    Tag::List(start_index) => {
                        self.flush();
                        self.scope.push(Scope::List(start_index));
                    }
                    Tag::Item => {
                        self.flush();
                        if let Some(Scope::List(index)) = self.scope.last() {
                            self.scope.push(Scope::ListItem(*index, false));
                        } else {
                            self.scope.push(Scope::ListItem(None, false));
                        }
                    }
                    Tag::FootnoteDefinition(text) => {
                        self.flush();
                        self.scope.push(Scope::FootnoteDefinition);
                        self.handle_text(&format!("{}:", text));
                        self.scope.pop();
                        self.flush();
                        self.scope.push(Scope::FootnoteContent);
                    }
                    Tag::HtmlBlock => { /* unknown */ }
                    Tag::Table(columns) => { self.scope.push(Scope::Table(columns)) }
                    Tag::TableHead => {
                        self.scope.push(Scope::TableHead);
                    }
                    Tag::TableRow => {
                        self.scope.push(Scope::TableRow);
                        self.table.1.push(vec![]);
                    }
                    Tag::TableCell => {
                        self.scope.push(Scope::TableCell);
                        if self.scope.iter().find(|scope| *scope == &Scope::TableHead).is_some() {
                            self.table.0.push(String::new());
                        } else {
                            self.table.1.last_mut().unwrap().push(String::new());
                        }
                    }
                    Tag::Emphasis => { self.scope.push(Scope::Italic); }
                    Tag::Strong => { self.scope.push(Scope::Bold); }
                    Tag::Strikethrough => { self.scope.push(Scope::Strikethrough); }
                    Tag::Code => { self.scope.push(Scope::Code); }
                    Tag::Link(_link_type, _destination, _title) => {
                        self.scope.push(Scope::Link);
                    }
                    Tag::Image(_link_type, destination, title) => {
                        self.flush();

                        if !self.opts.no_images {
                            let available_width = self.width - self.prefix_len() - self.suffix_len();
                            match image::open(destination.as_ref()) {
                                Ok(image) => {
                                    let (mut width, mut height) = image.dimensions();
                                    if width > available_width as u32 {
                                        let scale = available_width as f64 / width as f64;
                                        width = (width as f64 * scale) as u32;
                                        height = (height as f64 * scale) as u32;
                                    }
                                    let mut vec = vec![];
                                    termpix::print_image(image, true, width, height, &mut vec);
                                    let string = String::from_utf8(vec).unwrap();

                                    for line in string.lines() {
                                        let (prefix, _) = self.prefix();
                                        let (suffix, _) = self.suffix();
                                        println!(
                                            "{}{}{}{}{}{}{}",
                                            self.centering,
                                            self.margin,
                                            prefix,
                                            line,
                                            suffix,
                                            self.margin,
                                            self.shadow(),
                                        );
                                    }

                                    self.scope.push(Scope::Indent);
                                    self.scope.push(Scope::Caption);
                                    self.handle_text(title);
                                }
                                Err(error) => {
                                    self.handle_text("Cannot open image ");
                                    self.scope.push(Scope::Indent);
                                    self.scope.push(Scope::Link);
                                    self.handle_text(destination);
                                    self.scope.pop();
                                    self.handle_text(&format!(": {}", error));
                                    self.scope.push(Scope::Caption);
                                    self.flush();
                                }
                            }
                        } else {
                            self.scope.push(Scope::Indent);
                            self.handle_text("[Image");
                            if !title.is_empty() {
                                self.handle_text(": ");
                                self.scope.push(Scope::Caption);
                                self.handle_text(title);
                                self.scope.pop();
                            }
                            if !destination.is_empty() && !self.opts.hide_urls {
                                self.handle_text(" <");
                                self.scope.push(Scope::Link);
                                self.handle_text(destination);
                                self.scope.pop();
                                self.handle_text(">");
                            }
                            self.handle_text("]");
                            self.scope.push(Scope::Caption);
                            self.flush();
                        }
                    }
                }
            }

            Event::End(tag) => {
                match tag {
                    Tag::Paragraph => {
                        self.flush();
                        self.queue_empty();
                    }
                    Tag::Header(level) => {
                        self.flush();
                        self.scope.pop();
                        if level == 1 {
                            self.print_rule();
                        }
                        self.queue_empty();
                    }
                    Tag::Rule => {
                        self.flush();
                        self.print_rule();
                        self.scope.pop();
                    }
                    Tag::List(..) => {
                        self.flush();
                        self.scope.pop();
                        self.queue_empty();
                    }
                    Tag::Item => {
                        self.flush();
                        self.scope.pop();
                        if let Some(Scope::List(index)) = self.scope.last_mut() {
                            *index = index.map(|x| x + 1);
                        }
                    },
                    Tag::BlockQuote => {
                        self.flush();
                        self.scope.pop();
                        self.queue_empty();
                    }
                    Tag::Table(..) => {
                        self.print_table();
                        self.scope.pop();
                        self.queue_empty();
                    }
                    Tag::CodeBlock(..) => {
                        self.flush_buffer();
                        self.scope.pop();
                        self.queue_empty();
                    }
                    Tag::Link(_link_type, destination, title) => {
                        if !title.is_empty() && !destination.is_empty() && !self.opts.hide_urls {
                            self.handle_text(format!(" <{}: {}>", title, destination));
                        } else if !destination.is_empty() && !self.opts.hide_urls {
                            self.handle_text(format!(" <{}>", destination));
                        } else if !title.is_empty() {
                            self.handle_text(format!(" <{}>", title));
                        }
                        self.scope.pop();
                    }
                    Tag::Image(_link_type, _destination, _title) => {
                        self.flush();
                        self.scope.pop();
                        self.scope.pop();
                        self.queue_empty();
                    }
                    Tag::FootnoteDefinition(..) => {
                        self.flush();
                        self.scope.pop();
                        self.queue_empty();
                    }
                    _ => { self.scope.pop(); }
                }
            }
            Event::Text(text) => { self.handle_text(text); }
            Event::Html(_text) => { /* unimplemented */ }
            Event::InlineHtml(_text) => { /* unimplemented */ }
            Event::FootnoteReference(text) => { 
                self.scope.push(Scope::FootnoteReference);
                self.handle_text(&format!("[{}]", text)); 
                self.scope.pop();
            }
            Event::SoftBreak => { self.handle_text(" "); }
            Event::HardBreak => { self.flush(); }
            Event::TaskListMarker(checked) => {
                self.handle_text(if checked { "[ ] " } else { "[✓] " });
            }
        }
    }
}
