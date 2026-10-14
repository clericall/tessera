//! Tessera diagnostics: source line/column mapping and readable error rendering.
//!
//! Deliberately dependency-free for the first milestone. Later this can swap the
//! renderer for `codespan-reporting` while keeping the same [`SourceFile`] API.

pub struct SourceFile {
    pub name: String,
    pub src: String,
    /// Byte offset of the first byte of each line. Always starts with 0.
    line_starts: Vec<usize>,
}

impl SourceFile {
    pub fn new(name: impl Into<String>, src: impl Into<String>) -> Self {
        let src = src.into();
        let mut line_starts = vec![0usize];
        for (idx, ch) in src.char_indices() {
            if ch == '\n' {
                line_starts.push(idx + 1);
            }
        }
        SourceFile { name: name.into(), src, line_starts }
    }

    /// 1-based (line, column) for a 0-based byte offset.
    pub fn line_col(&self, offset: usize) -> (usize, usize) {
        let offset = offset.min(self.src.len());
        let line_idx = match self.line_starts.binary_search(&offset) {
            Ok(l) => l,
            Err(l) => l - 1,
        };
        let col = offset - self.line_starts[line_idx];
        (line_idx + 1, col + 1)
    }

    fn line_text(&self, line_1based: usize) -> &str {
        let start = self.line_starts[line_1based - 1];
        let end = self
            .line_starts
            .get(line_1based)
            .copied()
            .unwrap_or(self.src.len());
        self.src[start..end].trim_end_matches(['\n', '\r'])
    }

    /// Render a single diagnostic with a caret underline spanning `span`.
    ///
    /// `label` points at the offending span (may be empty); `help` is an optional
    /// trailing hint line.
    pub fn render(
        &self,
        code: &str,
        message: &str,
        span: (usize, usize),
        label: &str,
        help: &str,
    ) -> String {
        let (line, col) = self.line_col(span.0);
        let gutter = " ".repeat(line.to_string().len());
        let src_line = self.line_text(line);

        let start_col = self.line_col(span.0).1 - 1;
        let end_col = self
            .line_col(span.1.saturating_sub(1).max(span.0))
            .1;
        let width = end_col.saturating_sub(start_col).max(1);

        let mut out = String::new();
        out.push_str(&format!("error[{code}]: {message}\n"));
        out.push_str(&format!("{gutter}--> {}:{line}:{col}\n", self.name));
        out.push_str(&format!("{gutter} |\n"));
        out.push_str(&format!("{line} | {src_line}\n"));
        out.push_str(&format!(
            "{gutter} | {}{} {}\n",
            " ".repeat(start_col),
            "^".repeat(width),
            label
        ));
        if !help.is_empty() {
            out.push_str(&format!("{gutter} = help: {help}\n"));
        }
        out
    }
}
