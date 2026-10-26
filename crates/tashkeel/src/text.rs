//! Text preservation and model-independent handling of Arabic diacritics.
//!
//! Only the eight common tashkeel marks U+064B–U+0652 are replaced. Other
//! combining marks, base characters, punctuation and whitespace are retained.

pub(crate) mod tokenizer;

/// Whether a character is one of the eight common Arabic tashkeel marks.
pub fn is_tashkeel(character: char) -> bool {
    matches!(character, '\u{064b}'..='\u{0652}')
}

/// Remove exactly the eight common tashkeel marks, preserving everything else.
pub fn strip_tashkeel(text: &str) -> String {
    text.chars().filter(|&c| !is_tashkeel(c)).collect()
}

pub(crate) fn is_arabic_letter(character: char) -> bool {
    matches!(character, '\u{0621}'..='\u{063a}' | '\u{0641}'..='\u{064a}')
}

/// Split on sentence/paragraph boundaries, then words, then Unicode scalars.
/// Every source scalar (including old tashkeel) consumes the scalar budget.
pub(crate) fn chunks<'a>(
    text: &'a str,
    scalar_limit: usize,
    tokenizer: &tokenizer::Tokenizer,
) -> Vec<&'a str> {
    let mut result = Vec::new();
    for sentence in text.split_inclusive(|c| {
        matches!(
            c,
            '\n' | '\r' | '\u{2028}' | '\u{2029}' | '.' | '!' | '?' | '؟' | '؛'
        )
    }) {
        let mut remaining = sentence;
        while !remaining.is_empty() {
            let mut end = 0;
            let mut word_boundary = 0;
            let mut counter = tokenizer::TokenCounter::default();
            for (scalars, (offset, c)) in remaining.char_indices().enumerate() {
                let next = tokenizer.count_after(counter, c);
                if scalars == scalar_limit || next.positions > tokenizer.position_limit() {
                    break;
                }
                counter = next;
                end = offset + c.len_utf8();
                if c.is_whitespace() {
                    word_boundary = end;
                }
            }
            // Validated limits and the bounded per-character normalization ensure
            // at least one scalar always fits. Never split a UTF-8 code point.
            debug_assert!(end > 0);
            if end < remaining.len() && word_boundary > 0 {
                end = word_boundary;
            }
            result.push(&remaining[..end]);
            remaining = &remaining[end..];
        }
    }
    result
}

#[cfg(test)]
mod tests;
