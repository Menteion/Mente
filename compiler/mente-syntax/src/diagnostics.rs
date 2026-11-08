//! Structured diagnostics (`spec/diagnostics.md`).
//!
//! Every problem the compiler reports is a [`Diagnostic`]: a stable code, a severity, a message,
//! a primary span, any related spans and any fixes. It renders as text for people and as
//! versioned JSON for tools, and a fix is a set of text edits a tool can apply.

use std::fmt;

use crate::Span;

/// The version of the JSON form. It rises when a field changes meaning or is removed; adding a
/// field leaves it unchanged.
pub const JSON_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
    Note,
}

impl Severity {
    pub fn as_str(self) -> &'static str {
        match self {
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Note => "note",
        }
    }
}

/// A span that explains the primary one: where a clashing declaration is, say.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Label {
    pub span: Span,
    pub message: String,
}

/// How far a fix can be trusted, as rustc grades its suggestions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Applicability {
    /// Certain: a tool may apply it without a person reading it.
    MachineApplicable,
    /// Plausible: it needs judgement before it is applied.
    Suggestion,
}

impl Applicability {
    pub fn as_str(self) -> &'static str {
        match self {
            Applicability::MachineApplicable => "machine-applicable",
            Applicability::Suggestion => "suggestion",
        }
    }
}

/// Replace the bytes of `span` with `replacement`; an empty span inserts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edit {
    pub span: Span,
    pub replacement: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fix {
    pub message: String,
    pub applicability: Applicability,
    pub edits: Vec<Edit>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    /// A code from the catalogue `mn explain` reads, such as `E0413`. Never reused.
    pub code: &'static str,
    pub severity: Severity,
    pub message: String,
    pub primary: Span,
    pub related: Vec<Label>,
    pub fixes: Vec<Fix>,
}

impl Diagnostic {
    pub fn error(code: &'static str, primary: Span, message: impl Into<String>) -> Self {
        Diagnostic {
            code,
            severity: Severity::Error,
            message: message.into(),
            primary,
            related: Vec::new(),
            fixes: Vec::new(),
        }
    }

    pub fn with_related(mut self, span: Span, message: impl Into<String>) -> Self {
        self.related.push(Label {
            span,
            message: message.into(),
        });
        self
    }

    /// A fix of one edit.
    pub fn with_fix(
        mut self,
        applicability: Applicability,
        message: impl Into<String>,
        span: Span,
        replacement: impl Into<String>,
    ) -> Self {
        self.fixes.push(Fix {
            message: message.into(),
            applicability,
            edits: vec![Edit {
                span,
                replacement: replacement.into(),
            }],
        });
        self
    }
}

/// One line: `error[E0413]: message`.
impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}[{}]: {}",
            self.severity.as_str(),
            self.code,
            self.message
        )
    }
}

/// A span that covers no text, where an edit inserts.
pub fn insertion_point(at: Span) -> Span {
    Span {
        end: at.start,
        ..at
    }
}

/// The 1-based line and column of a byte offset. The column counts characters, not bytes.
pub fn line_col(src: &str, byte: usize) -> (usize, usize) {
    let mut at = byte.min(src.len());
    while !src.is_char_boundary(at) {
        at -= 1;
    }
    let before = &src[..at];
    let line = before.matches('\n').count() + 1;
    let line_start = before.rfind('\n').map_or(0, |i| i + 1);
    (line, src[line_start..at].chars().count() + 1)
}

/// A diagnostic as people read it: the code and message, the place, the source line with the
/// span underlined, then related spans and fixes.
pub fn render_text(d: &Diagnostic, file: &str, src: &str) -> String {
    let (line, col) = line_col(src, d.primary.start);
    let mut out = format!(
        "{}[{}]: {}\n  --> {}:{}:{}\n",
        d.severity.as_str(),
        d.code,
        d.message,
        file,
        line,
        col
    );
    if let Some(text) = src.lines().nth(line - 1) {
        let width = src[d.primary.start.min(src.len())..d.primary.end.min(src.len())]
            .lines()
            .next()
            .map_or(0, |s| s.chars().count())
            .max(1);
        let gutter = line.to_string().len();
        out.push_str(&format!(
            "{:g$} |\n{} | {}\n{:g$} | {}{}\n",
            "",
            line,
            text,
            "",
            " ".repeat(col - 1),
            "^".repeat(width),
            g = gutter
        ));
    }
    for r in &d.related {
        let (l, c) = line_col(src, r.span.start);
        out.push_str(&format!("  note: {} ({}:{}:{})\n", r.message, file, l, c));
    }
    for f in &d.fixes {
        out.push_str(&format!(
            "  fix ({}): {}\n",
            f.applicability.as_str(),
            f.message
        ));
    }
    out
}

/// The diagnostics of one source file, for [`render_json`].
pub struct FileDiagnostics<'a> {
    pub file: &'a str,
    pub src: &'a str,
    pub diagnostics: &'a [Diagnostic],
}

/// The JSON form: `{"version": 1, "diagnostics": [...]}`, every diagnostic of every file.
pub fn render_json(files: &[FileDiagnostics]) -> String {
    let mut items = Vec::new();
    for f in files {
        for d in f.diagnostics {
            items.push(diagnostic_json(d, f.file, f.src));
        }
    }
    format!(
        "{{\"version\":{},\"diagnostics\":[{}]}}",
        JSON_VERSION,
        items.join(",")
    )
}

fn diagnostic_json(d: &Diagnostic, file: &str, src: &str) -> String {
    let related: Vec<String> = d
        .related
        .iter()
        .map(|r| {
            format!(
                "{{\"message\":{},\"span\":{}}}",
                json_string(&r.message),
                span_json(r.span, file, src)
            )
        })
        .collect();
    let fixes: Vec<String> = d
        .fixes
        .iter()
        .map(|f| {
            let edits: Vec<String> = f
                .edits
                .iter()
                .map(|e| {
                    format!(
                        "{{\"span\":{},\"replacement\":{}}}",
                        span_json(e.span, file, src),
                        json_string(&e.replacement)
                    )
                })
                .collect();
            format!(
                "{{\"message\":{},\"applicability\":\"{}\",\"edits\":[{}]}}",
                json_string(&f.message),
                f.applicability.as_str(),
                edits.join(",")
            )
        })
        .collect();
    format!(
        "{{\"code\":\"{}\",\"severity\":\"{}\",\"message\":{},\"primary\":{},\"related\":[{}],\"fixes\":[{}]}}",
        d.code,
        d.severity.as_str(),
        json_string(&d.message),
        span_json(d.primary, file, src),
        related.join(","),
        fixes.join(",")
    )
}

fn span_json(span: Span, file: &str, src: &str) -> String {
    let (line, column) = line_col(src, span.start);
    format!(
        "{{\"file\":{},\"start\":{},\"end\":{},\"line\":{},\"column\":{}}}",
        json_string(file),
        span.start,
        span.end,
        line,
        column
    )
}

/// A JSON string literal: quotes, backslashes and control characters escaped.
pub fn json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Applies a fix to the source its spans were computed on. Its edits may not overlap.
pub fn apply_fix(src: &str, fix: &Fix) -> Result<String, String> {
    let mut edits: Vec<&Edit> = fix.edits.iter().collect();
    edits.sort_by_key(|e| (e.span.start, e.span.end));
    let mut out = String::with_capacity(src.len());
    let mut at = 0;
    for e in edits {
        let (start, end) = (e.span.start, e.span.end);
        if start < at {
            return Err(format!("edits overlap at byte {}", start));
        }
        // `is_char_boundary` is false past the end, so it bounds `end` too.
        if start > end || !src.is_char_boundary(start) || !src.is_char_boundary(end) {
            return Err(format!("edit {}..{} lies outside the source", start, end));
        }
        out.push_str(&src[at..start]);
        out.push_str(&e.replacement);
        at = end;
    }
    out.push_str(&src[at..]);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span(start: usize, end: usize) -> Span {
        Span {
            start,
            end,
            line: 0,
            col: 0,
        }
    }

    #[test]
    fn test_line_col_counts_lines_and_characters_from_one() {
        let src = "ab\ncdé f\n";
        assert_eq!(line_col(src, 0), (1, 1));
        assert_eq!(line_col(src, 3), (2, 1));
        // `é` is two bytes and one column.
        assert_eq!(line_col(src, src.find('f').unwrap()), (2, 5));
        assert_eq!(line_col(src, src.len() + 10), (3, 1));
        // An offset inside a character counts as the character's start.
        let e = src.find('é').unwrap();
        assert_eq!(line_col(src, e + 1), line_col(src, e));
        assert_eq!(insertion_point(span(3, 9)), span(3, 3));
    }

    #[test]
    fn test_json_form_of_a_diagnostic() {
        let src = "module m\nfield: u12\n";
        let d = Diagnostic::error("E0417", span(16, 19), "type `u12` has no \"byte\" size")
            .with_related(span(0, 6), "in this module")
            .with_fix(
                Applicability::Suggestion,
                "use a bitstruct",
                span(16, 19),
                "u16",
            );
        let json = render_json(&[FileDiagnostics {
            file: "a\\b.mn",
            src,
            diagnostics: &[d],
        }]);
        assert_eq!(
            json,
            concat!(
                "{\"version\":1,\"diagnostics\":[{\"code\":\"E0417\",\"severity\":\"error\",",
                "\"message\":\"type `u12` has no \\\"byte\\\" size\",",
                "\"primary\":{\"file\":\"a\\\\b.mn\",\"start\":16,\"end\":19,\"line\":2,\"column\":8},",
                "\"related\":[{\"message\":\"in this module\",\"span\":{\"file\":\"a\\\\b.mn\",\"start\":0,\"end\":6,\"line\":1,\"column\":1}}],",
                "\"fixes\":[{\"message\":\"use a bitstruct\",\"applicability\":\"suggestion\",",
                "\"edits\":[{\"span\":{\"file\":\"a\\\\b.mn\",\"start\":16,\"end\":19,\"line\":2,\"column\":8},\"replacement\":\"u16\"}]}]}]}"
            )
        );
        assert_eq!(render_json(&[]), "{\"version\":1,\"diagnostics\":[]}");
        assert_eq!(json_string("a\tb\u{1}"), "\"a\\tb\\u0001\"");
    }

    #[test]
    fn test_text_form_underlines_the_span() {
        let src = "module m\n  @bytes(12..16) pub rest: u32\n";
        let start = src.find('@').unwrap();
        let d = Diagnostic::error("E0413", span(start, start + 14), "a gap before `rest`")
            .with_related(span(0, 6), "the module")
            .with_fix(
                Applicability::Suggestion,
                "start it at byte 8",
                span(start, start + 14),
                "@bytes(8..12)",
            );
        assert_eq!(
            render_text(&d, "m.mn", src),
            concat!(
                "error[E0413]: a gap before `rest`\n",
                "  --> m.mn:2:3\n",
                "  |\n",
                "2 |   @bytes(12..16) pub rest: u32\n",
                "  |   ^^^^^^^^^^^^^^\n",
                "  note: the module (m.mn:1:1)\n",
                "  fix (suggestion): start it at byte 8\n",
            )
        );
    }

    #[test]
    fn test_apply_fix_edits_in_order_and_rejects_overlap() {
        let fix = |edits: Vec<(usize, usize, &str)>| Fix {
            message: String::new(),
            applicability: Applicability::MachineApplicable,
            edits: edits
                .into_iter()
                .map(|(s, e, r)| Edit {
                    span: span(s, e),
                    replacement: r.to_string(),
                })
                .collect(),
        };
        // Given out of order: an insertion at 0 and a replacement of "cd".
        assert_eq!(
            apply_fix("abcdef", &fix(vec![(2, 4, "XY"), (0, 0, ">")])).unwrap(),
            ">abXYef"
        );
        assert!(apply_fix("abcdef", &fix(vec![(1, 4, "x"), (3, 5, "y")])).is_err());
        // Each way an edit can fall outside the source, on its own.
        assert!(apply_fix("abcdef", &fix(vec![(3, 2, "x")])).is_err());
        assert!(apply_fix("abc", &fix(vec![(2, 9, "x")])).is_err());
        assert!(apply_fix("é", &fix(vec![(1, 2, "x")])).is_err());
        assert!(apply_fix("é", &fix(vec![(0, 1, "x")])).is_err());
        assert_eq!(apply_fix("é", &fix(vec![(0, 2, "e")])).unwrap(), "e");
    }
}
