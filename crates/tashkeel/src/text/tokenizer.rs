use std::collections::HashMap;

use serde::Deserialize;
use unicode_normalization::UnicodeNormalization;

use crate::{Error, Model, Result};

use super::{is_arabic_letter, is_tashkeel};

#[derive(Deserialize)]
struct Vocabulary {
    characters: HashMap<String, i64>,
    classes: HashMap<usize, String>,
}

pub(crate) struct Tokenizer {
    model: Model,
    characters: HashMap<char, i64>,
    classes: Vec<String>,
}

pub(crate) struct Encoded {
    pub(crate) ids: Vec<i64>,
    /// Byte offsets into the original input. None means no safe mark target.
    pub(crate) sources: Vec<Option<usize>>,
}

#[derive(Clone, Copy, Default)]
pub(crate) struct TokenCounter {
    pub(crate) positions: usize,
    pending_space: bool,
}

impl Tokenizer {
    pub(crate) fn new(model: Model) -> Result<Self> {
        let (json, class_count) = match model {
            Model::Shakkelha => (include_str!("../../data/shakkelha.json"), 19),
            Model::Catt => (include_str!("../../data/catt.json"), 18),
        };
        let vocabulary: Vocabulary = serde_json::from_str(json)?;
        let classes = (0..class_count)
            .map(|index| {
                vocabulary.classes.get(&index).cloned().ok_or_else(|| {
                    Error::Model(format!("embedded {model} vocabulary lacks class {index}"))
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let characters = vocabulary
            .characters
            .into_iter()
            .filter_map(|(key, value)| {
                let mut chars = key.chars();
                let first = chars.next()?;
                chars.next().is_none().then_some((first, value))
            })
            .collect();
        Ok(Self {
            model,
            characters,
            classes,
        })
    }

    pub(crate) fn class_count(&self) -> usize {
        self.classes.len()
    }

    pub(crate) fn position_limit(&self) -> usize {
        match self.model {
            Model::Shakkelha => usize::MAX - 2,
            // Encoder-only CATT does not pass BOS or EOS to the encoder.
            Model::Catt => 1024,
        }
    }

    pub(crate) fn count_after(&self, mut counter: TokenCounter, c: char) -> TokenCounter {
        if is_tashkeel(c) {
            return counter;
        }
        if self.model == Model::Shakkelha {
            counter.positions += 1;
        } else {
            for normalized in normalize_catt(c).chars() {
                if normalized == ' ' {
                    counter.pending_space = counter.positions > 0;
                } else {
                    counter.positions += 1 + usize::from(counter.pending_space);
                    counter.pending_space = false;
                }
            }
        }
        counter
    }

    pub(crate) fn encode(&self, text: &str) -> Encoded {
        let mut encoded = Encoded {
            ids: Vec::new(),
            sources: Vec::new(),
        };
        if self.model == Model::Shakkelha {
            encoded.push(1, None);
            for (offset, c) in text.char_indices().filter(|&(_, c)| !is_tashkeel(c)) {
                encoded.push(
                    self.characters.get(&c).copied().unwrap_or(3),
                    is_arabic_letter(c).then_some(offset),
                );
            }
            encoded.push(2, None);
        } else {
            let mut pending_space = false;
            for (offset, c) in text.char_indices() {
                let normalized = normalize_catt(c);
                // A ligature with multiple letters has no lossless place for
                // internal marks. Keep its tokens as context, with no targets.
                let safe = normalized.chars().count() == 1;
                for letter in normalized.chars() {
                    if letter == ' ' {
                        pending_space = !encoded.ids.is_empty();
                    } else {
                        if pending_space {
                            encoded.push(self.characters[&' '], None);
                            pending_space = false;
                        }
                        encoded.push(self.characters[&letter], safe.then_some(offset));
                    }
                }
            }
        }
        encoded
    }

    pub(crate) fn render(
        &self,
        original: &str,
        encoded: &Encoded,
        labels: &[usize],
    ) -> Result<String> {
        if labels.len() != encoded.ids.len() || labels.iter().any(|&id| id >= self.classes.len()) {
            return Err(Error::Inference(
                "model returned invalid label count or class".into(),
            ));
        }
        let mut predictions = encoded
            .sources
            .iter()
            .zip(labels)
            .filter_map(|(&offset, &id)| offset.map(|offset| (offset, self.classes[id].as_str())))
            .peekable();
        let mut output = String::with_capacity(original.len());
        for (offset, c) in original.char_indices() {
            if is_tashkeel(c) {
                continue;
            }
            output.push(c);
            if predictions
                .peek()
                .is_some_and(|&(position, _)| position == offset)
                && let Some((_, marks)) = predictions.next()
            {
                // Special output classes carry no printable diacritics.
                output.extend(marks.chars().filter(|&mark| is_tashkeel(mark)));
            }
        }
        Ok(output)
    }
}

impl Encoded {
    fn push(&mut self, id: i64, source: Option<usize>) {
        self.ids.push(id);
        self.sources.push(source);
    }

    pub(crate) fn has_targets(&self) -> bool {
        self.sources.iter().any(Option::is_some)
    }
}

/// Normalization is used for model input only. Unknown characters separate
/// words; tatweel, dagger alif and replaced tashkeel consume no positions.
fn normalize_catt(c: char) -> String {
    if is_tashkeel(c) || matches!(c, '\u{0640}' | '\u{0670}') {
        return String::new();
    }
    if c == '\u{0671}' {
        return "ا".into();
    }
    if is_arabic_letter(c) {
        return c.to_string();
    }
    if matches!(c, '\u{fb50}'..='\u{fdff}' | '\u{fe70}'..='\u{fefc}') {
        return c
            .to_string()
            .nfkc()
            .filter_map(|letter| {
                if is_tashkeel(letter) || matches!(letter, '\u{0640}' | '\u{0670}') {
                    None
                } else if is_arabic_letter(letter) {
                    Some(letter)
                } else if letter == '\u{0671}' {
                    Some('ا')
                } else {
                    Some(' ')
                }
            })
            .collect();
    }
    " ".into()
}
