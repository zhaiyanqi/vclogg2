use super::*;

/// Counts and representative references describe the stored search; no source text is read.
pub(super) fn summarize_search(search: &SearchSnapshot, offset: usize) -> Result<Value> {
    let total = search
        .groups
        .iter()
        .map(|(_, rows)| rows.len())
        .sum::<usize>();
    let mut files = Vec::new();
    for (doc, rows) in search.groups.iter().skip(offset).take(50) {
        doc.verify()?;
        files.push(json!({"document_id":doc.id,"version":doc.version,"file":doc.document.file_name(),"matches":rows.len(),"first_line":rows.get(0).map(|r|r+1),"last_line":rows.get(rows.len().saturating_sub(1)).map(|r|r+1)}));
    }
    // Select representative references from compressed row indexes without decoding log text.
    let mut samples = Vec::new();
    let stride = total.div_ceil(12).max(1);
    let mut next_sample = 0;
    let mut group_start = 0;
    for (doc, rows) in &search.groups {
        let group_end = group_start + rows.len();
        if next_sample < group_end {
            doc.verify()?;
        }
        while next_sample < group_end && samples.len() < 12 {
            let row = rows
                .get(next_sample - group_start)
                .context("Search sample unavailable")?;
            let reference = doc.reference(row);
            samples.push(json!({"reference":reference,"url":reference.url(),"file":doc.document.file_name(),"result_index":next_sample+1}));
            next_sample += stride;
        }
        group_start = group_end;
    }
    Ok(json!({
        "total_matches":total,"truncated":search.truncated,"total_files":search.groups.len(),
        "files":files,"next_offset":(offset.saturating_add(50)<search.groups.len()).then_some(offset.saturating_add(50)),
        "representation":"references","content_included":false,
        "sampling":"evenly spaced matching row references; read selected references before analyzing content",
        "samples":samples
    }))
}

pub(super) fn read_context(
    doc: &DocumentSnapshot,
    row: usize,
    before: usize,
    after: usize,
    cancellation: &SearchCancellation,
) -> Result<Value> {
    doc.verify()?;
    // Always include the requested evidence before spending the remaining budget on neighbors.
    let focus = log_row(doc, row)?;
    let mut rows = Vec::new();
    let start = row.saturating_sub(before);
    let end = row
        .saturating_add(after)
        .saturating_add(1)
        .min(doc.document.source_line_count());
    let mut bytes = serde_json::to_vec(&focus)?.len();
    for distance in 1..=before.max(after) {
        for candidate in [
            row.checked_sub(distance).filter(|r| *r >= start),
            row.checked_add(distance).filter(|r| *r < end),
        ]
        .into_iter()
        .flatten()
        {
            if cancellation.is_cancelled() {
                bail!("Read cancelled");
            }
            let excerpt = log_excerpt(doc, candidate, 512, None)?;
            let size = serde_json::to_vec(&excerpt)?.len();
            if bytes + size > 15 * 1024 {
                continue;
            }
            bytes += size;
            rows.push(excerpt);
        }
    }
    rows.sort_by_key(|value| value["reference"]["line"].as_u64().unwrap_or_default());
    let truncated = rows.len() + 1 < end - start;
    Ok(
        json!({"focus":focus,"context":rows,"context_truncated":truncated,"requested_start_line":start+1,"requested_end_line":end}),
    )
}
