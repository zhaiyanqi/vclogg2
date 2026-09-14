use super::*;

const MAX_WINDOW_SOURCE_BYTES: usize = 16 * 1024 * 1024;

/// A bounded part of one source line; all offsets count Unicode scalar values.
#[derive(Clone, Debug)]
pub struct LineTextWindow {
    text: String,
    start_character: usize,
    next_character: Option<usize>,
    source_truncated: bool,
    match_characters: Option<std::ops::Range<usize>>,
}
impl LineTextWindow {
    pub fn text(&self) -> &str {
        &self.text
    }
    pub fn start_character(&self) -> usize {
        self.start_character
    }
    pub fn next_character(&self) -> Option<usize> {
        self.next_character
    }
    pub fn is_source_truncated(&self) -> bool {
        self.source_truncated
    }
    pub fn match_characters(&self) -> Option<std::ops::Range<usize>> {
        self.match_characters.clone()
    }
}

impl LogDocument {
    /// Read a source-line segment, or center on the first match of the original query.
    /// Matching uses a complete line to preserve regex anchors; lines exceeding the
    /// local scan cap return an explicit error. Offset mode only reads a bounded prefix.
    pub fn line_text_window(
        &self,
        source_row: usize,
        start_character: usize,
        max_characters: usize,
        matcher: Option<&SearchMatcher>,
        cancellation: &CancellationToken,
    ) -> Result<LineTextWindow> {
        anyhow::ensure!(
            (1..=4096).contains(&max_characters),
            "Window must contain 1 to 4096 characters"
        );
        anyhow::ensure!(!cancellation.is_cancelled(), "Line read cancelled");
        anyhow::ensure!(
            !self.source_changed()?,
            "Source changed; refresh the reference"
        );
        let row = self
            .local_row(source_row)
            .context("Source line unavailable")?;
        let range = self
            .line_byte_range_at_local_row(row)
            .context("Source line unavailable")?;
        // Include enough lookahead for newline trimming and multibyte boundaries.
        let budget = if matcher.is_some() {
            MAX_WINDOW_SOURCE_BYTES
        } else {
            start_character
                .saturating_add(max_characters)
                .saturating_add(1)
                .saturating_mul(4)
                .saturating_add(16)
                .min(MAX_WINDOW_SOURCE_BYTES)
        };
        if matcher.is_some() && range.len() > MAX_WINDOW_SOURCE_BYTES {
            anyhow::bail!(
                "This line exceeds the 16 MiB local match-scan limit; use a bounded character offset or report that the match could not be inspected"
            );
        }
        let preview = self
            .line_preview(source_row, budget)
            .context("Source changed or line unavailable")?;
        anyhow::ensure!(!cancellation.is_cancelled(), "Line read cancelled");
        let source = preview.text();
        let matched = if let Some(matcher) = matcher {
            anyhow::ensure!(
                !preview.is_truncated(),
                "Cannot match a truncated line; use character-offset reading"
            );
            let range = matcher
                .first_matching_range(source)
                .context("No non-empty match in this line; verify the query and reference")?;
            Some(source[..range.start].chars().count()..source[..range.end].chars().count())
        } else {
            None
        };
        let start = matched.as_ref().map_or(start_character, |r| {
            r.start.saturating_sub(max_characters / 3)
        });
        let total = source.chars().count();
        anyhow::ensure!(
            start <= total && !(start == total && preview.is_truncated()),
            "Character offset exceeds the inspected line prefix (local cap: 16 MiB)"
        );
        let text = source
            .chars()
            .skip(start)
            .take(max_characters)
            .collect::<String>();
        let next = start + text.chars().count();
        anyhow::ensure!(!cancellation.is_cancelled(), "Line read cancelled");
        anyhow::ensure!(
            !self.source_changed()?,
            "Source changed; refresh the reference"
        );
        Ok(LineTextWindow {
            text,
            start_character: start,
            next_character: (next < total || preview.is_truncated()).then_some(next),
            source_truncated: preview.is_truncated(),
            match_characters: matched,
        })
    }
}
