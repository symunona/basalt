use std::{borrow::Cow, fmt, ops::Range, vec::IntoIter};

use pulldown_cmark::CowStr;

/// A style that can be applied to [`TextSegment`] (code, emphasis, strikethrough, strong).
#[derive(Clone, Debug, PartialEq)]
pub enum Style {
    Code,
    Emphasis,
    Strong,
    Strikethrough,
    Link,
}

impl fmt::Display for Style {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Style::Code => write!(f, "Code"),
            Style::Emphasis => write!(f, "Emphasis"),
            Style::Strong => write!(f, "Strong"),
            Style::Strikethrough => write!(f, "Strikethrough"),
            Style::Link => write!(f, "Link"),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TextSegment {
    pub content: String,
    pub style: Option<Style>,
    pub link: Option<Range<usize>>,
}

impl TextSegment {
    pub fn new(content: &str, style: Option<Style>) -> Self {
        Self {
            content: content.to_string(),
            style,
            link: None,
        }
    }

    pub fn add_style(&mut self, style: &Style) {
        self.style = Some(style.clone());
    }

    pub fn empty_line() -> Self {
        Self {
            content: '\n'.to_string(),
            style: None,
            link: None,
        }
    }

    pub fn plain(content: &str) -> Self {
        Self {
            content: content.to_string(),
            style: None,
            link: None,
        }
    }

    pub fn styled(content: &str, style: Style) -> Self {
        Self {
            content: content.to_string(),
            style: Some(style),
            link: None,
        }
    }

    pub fn link(content: &str, source: Range<usize>) -> Self {
        Self {
            content: content.to_string(),
            style: Some(Style::Link),
            link: Some(source),
        }
    }
}

impl fmt::Display for TextSegment {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let content = &self.content;
        write!(f, "{content}")
    }
}

impl From<CowStr<'_>> for TextSegment {
    fn from(content: CowStr<'_>) -> Self {
        content.to_string().into()
    }
}

impl From<String> for TextSegment {
    fn from(content: String) -> Self {
        content.as_str().into()
    }
}

impl From<&str> for TextSegment {
    fn from(content: &str) -> Self {
        // Replace tab character with spaces.
        // Tab character can break the rendered TUI.
        // FIXME: Control tab length with config value.
        Self::plain(&content.replace("\t", "  "))
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct RichText {
    segments: Vec<TextSegment>,
}

impl IntoIterator for RichText {
    type Item = TextSegment;
    type IntoIter = IntoIter<Self::Item>;
    fn into_iter(self) -> Self::IntoIter {
        self.segments.into_iter()
    }
}

impl fmt::Display for RichText {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.segments
            .iter()
            .try_for_each(|segment| write!(f, "{segment}"))
    }
}

impl RichText {
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.segments.is_empty()
    }

    pub fn segments(&self) -> &[TextSegment] {
        &self.segments
    }

    /// The text with all styling dropped. Borrows when the text is exactly one
    /// segment, and joins the segments otherwise.
    pub fn plain_text(&self) -> Cow<'_, str> {
        match self.segments.as_slice() {
            [segment] => Cow::Borrowed(&segment.content),
            _ => Cow::Owned(self.to_string()),
        }
    }

    pub fn links(&self) -> Vec<(Range<usize>, Range<usize>)> {
        let mut links: Vec<(Range<usize>, Range<usize>)> = Vec::new();
        let mut start = 0;
        for segment in &self.segments {
            let range = start..start + segment.content.len();
            start = range.end;
            let Some(source) = &segment.link else {
                continue;
            };
            match links.last_mut() {
                Some((last, last_source)) if last.end == range.start && last_source == source => {
                    last.end = range.end
                }
                _ => links.push((range, source.clone())),
            }
        }
        links
    }
}

impl From<Vec<TextSegment>> for RichText {
    fn from(segments: Vec<TextSegment>) -> Self {
        Self { segments }
    }
}

impl<const N: usize> From<[TextSegment; N]> for RichText {
    fn from(segments: [TextSegment; N]) -> Self {
        Self {
            segments: segments.to_vec(),
        }
    }
}
