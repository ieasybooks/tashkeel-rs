use crate::Model;

use super::{chunks, is_tashkeel, strip_tashkeel, tokenizer::Tokenizer};

#[test]
fn removes_exactly_eight_marks_without_other_normalization() {
    assert_eq!(
        (0x0640..=0x067f)
            .filter_map(char::from_u32)
            .filter(|&c| is_tashkeel(c))
            .count(),
        8
    );
    let input = "\tٱلْعَرَبِيَّةُ ﷲ ﻻ ـٰۖٔ ١٢3 Latin\r\n🙂";
    assert_eq!(strip_tashkeel(input), "\tٱلعربية ﷲ ﻻ ـٰۖٔ ١٢3 Latin\r\n🙂");
}

#[test]
fn catt_maps_normalized_positions_back_to_original_text() {
    let tokenizer = Tokenizer::new(Model::Catt).unwrap();
    let source = "\tٱلـعَرَبِيَّةُ، 42 API\nبٰۖ\r\n";
    let encoded = tokenizer.encode(source);
    let plain = tokenizer.encode("العربية ب");
    assert_eq!(encoded.ids, plain.ids);
    let output = tokenizer
        .render(source, &encoded, &vec![13; encoded.ids.len()])
        .unwrap();
    assert_eq!(output, "\tٱَلَـعَرَبَيَةَ، 42 API\nبَٰۖ\r\n");
    assert_eq!(strip_tashkeel(&output), strip_tashkeel(source));
}

#[test]
fn presentation_expansions_are_context_without_ambiguous_marks() {
    let tokenizer = Tokenizer::new(Model::Catt).unwrap();
    for (source, normalized) in [
        ("ﻻ", "لا"),
        ("ﷲ", "الله"),
        ("ﷺ", "صلى الله عليه وسلم"),
        ("ﻷ", "لأ"),
    ] {
        let encoded = tokenizer.encode(source);
        assert_eq!(encoded.ids, tokenizer.encode(normalized).ids, "{source}");
        assert!(!encoded.has_targets());
        assert_eq!(
            tokenizer
                .render(source, &encoded, &vec![13; encoded.ids.len()])
                .unwrap(),
            source
        );
    }
    let source = "ﺏ ﻻ ب";
    let encoded = tokenizer.encode(source);
    assert_eq!(
        tokenizer
            .render(source, &encoded, &vec![13; encoded.ids.len()])
            .unwrap(),
        "ﺏَ ﻻ بَ"
    );
}

#[test]
fn shakkelha_retains_unknown_positions_without_diacritizing_them() {
    let tokenizer = Tokenizer::new(Model::Shakkelha).unwrap();
    let source = "بَ،\tٱـﻻA\r\n";
    let encoded = tokenizer.encode(source);
    assert_eq!(encoded.ids, [1, 50, 40, 3, 3, 3, 3, 3, 3, 4, 2]);
    let output = tokenizer
        .render(source, &encoded, &vec![1; encoded.ids.len()])
        .unwrap();
    assert_eq!(output, source);
}

#[test]
fn special_classes_never_delete_base_characters_or_shift_labels() {
    for model in [Model::Shakkelha, Model::Catt] {
        let tokenizer = Tokenizer::new(model).unwrap();
        let source = "ب ت ث";
        let encoded = tokenizer.encode(source);
        let label = if model == Model::Catt { 0 } else { 17 };
        assert_eq!(
            tokenizer
                .render(source, &encoded, &vec![label; encoded.ids.len()])
                .unwrap(),
            source
        );
        assert!(tokenizer.render(source, &encoded, &[]).is_err());
        assert!(
            tokenizer
                .render(source, &encoded, &vec![99; encoded.ids.len()])
                .is_err()
        );
    }
}

#[test]
fn chunks_preserve_source_and_prefer_sentences_then_words() {
    let tokenizer = Tokenizer::new(Model::Catt).unwrap();
    let source = "أول ثان.\r\nنص عربي طويل؟\tآخر";
    let parts = chunks(source, 10, &tokenizer);
    assert_eq!(parts.concat(), source);
    assert!(parts.iter().all(|s| s.chars().count() <= 10));
    assert_eq!(
        parts,
        ["أول ثان.", "\r", "\n", "نص عربي ", "طويل؟", "\tآخر"]
    );
    assert_eq!(chunks("", 500, &tokenizer), [] as [&str; 0]);
    assert_eq!(chunks("بَتُ🙂 abc", 2, &tokenizer).concat(), "بَتُ🙂 abc");
    assert!(
        chunks("بَتُ🙂 abc", 2, &tokenizer)
            .iter()
            .all(|s| s.chars().count() <= 2)
    );
}

#[test]
fn catt_position_limit_counts_expansions_and_never_splits_source_scalars() {
    let tokenizer = Tokenizer::new(Model::Catt).unwrap();
    for source in [
        "ب".repeat(1024),
        "ب".repeat(1025),
        "ﻻ".repeat(513),
        "ﷺ".repeat(100),
        format!("{}ـ\tب", "ب".repeat(1024)),
    ] {
        let parts = chunks(&source, 10_000, &tokenizer);
        assert_eq!(parts.concat(), source);
        assert!(
            parts
                .iter()
                .all(|part| tokenizer.encode(part).ids.len() <= 1024)
        );
    }
    assert_eq!(chunks(&"ب".repeat(1024), 10_000, &tokenizer).len(), 1);
    assert_eq!(chunks(&"ب".repeat(1025), 10_000, &tokenizer).len(), 2);
    assert_eq!(chunks(&"ﻻ".repeat(512), 10_000, &tokenizer).len(), 1);
    let shakkelha = Tokenizer::new(Model::Shakkelha).unwrap();
    assert_eq!(chunks(&"ب".repeat(2000), 3000, &shakkelha).len(), 1);
}

#[test]
fn token_counter_matches_encoder_across_unicode_edge_cases() {
    let tokenizer = Tokenizer::new(Model::Catt).unwrap();
    for source in [
        "ب🙂ت\tﻻ",
        "\nۖـ\u{0670}\u{064e}",
        "ٱﷺ\rب",
        "ﺏ\u{feff}ت",
        "a b ١٢3",
        "\u{fe70}ب",
    ] {
        let count = source
            .chars()
            .fold(super::tokenizer::TokenCounter::default(), |state, c| {
                tokenizer.count_after(state, c)
            });
        assert_eq!(
            count.positions,
            tokenizer.encode(source).ids.len(),
            "{source}"
        );
    }
}
