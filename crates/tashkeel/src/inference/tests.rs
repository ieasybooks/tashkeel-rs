use std::{ops::ControlFlow, path::PathBuf};

use crate::{Error, Model, text::strip_tashkeel};

use super::{Diacritizer, InferenceOptions, Progress};

#[path = "fixtures.rs"]
mod fixtures;

#[test]
fn identifiers_and_options_are_validated_without_runtime() {
    for model in [Model::Shakkelha, Model::Catt] {
        assert_eq!(model.as_str().parse::<Model>().unwrap(), model);
        assert_eq!(model.to_string(), model.as_str());
    }
    assert!("unknown".parse::<Model>().is_err());
    let defaults = InferenceOptions::default();
    assert_eq!((defaults.threads, defaults.chunk_size), (1, 500));
    assert!(defaults.runtime_path.is_none());
    for options in [
        InferenceOptions {
            threads: 0,
            ..defaults.clone()
        },
        InferenceOptions {
            threads: usize::MAX,
            ..defaults.clone()
        },
        InferenceOptions {
            chunk_size: 0,
            ..defaults.clone()
        },
        InferenceOptions {
            runtime_path: Some(PathBuf::new()),
            ..defaults
        },
    ] {
        assert!(matches!(
            Diacritizer::load(Model::Catt, "/missing".as_ref(), options),
            Err(Error::InvalidInput(_))
        ));
    }
}

fn real_model(model: Model, chunk_size: usize) -> Diacritizer {
    let variable = match model {
        Model::Shakkelha => "TASHKEEL_TEST_SHAKKELHA_DIR",
        Model::Catt => "TASHKEEL_TEST_CATT_DIR",
    };
    let directory =
        std::env::var_os(variable).unwrap_or_else(|| panic!("set {variable} for this opt-in test"));
    Diacritizer::load(
        model,
        &PathBuf::from(directory),
        InferenceOptions {
            chunk_size,
            ..InferenceOptions::default()
        },
    )
    .unwrap()
}

fn check_fixtures(model: Model, fixtures: &[fixtures::Fixture]) {
    let mut diacritizer = real_model(model, 500);
    for fixture in fixtures {
        let encoded = diacritizer.tokenizer.encode(fixture.text);
        assert_eq!(encoded.ids, fixture.ids);
        let labels = diacritizer
            .backend
            .predict(&encoded.ids, diacritizer.tokenizer.class_count())
            .unwrap();
        assert_eq!(
            labels,
            fixture.labels,
            "{model}, {} positions",
            encoded.ids.len()
        );
        let output = diacritizer
            .tokenizer
            .render(fixture.text, &encoded, &labels)
            .unwrap();
        assert_eq!(strip_tashkeel(&output), strip_tashkeel(fixture.text));
        if let Some(expected) = fixture.output {
            assert_eq!(output, expected);
        }
    }
}

#[test]
#[ignore = "requires local Shakkelha model and ONNX Runtime API 31"]
fn real_shakkelha_matches_python_onnx_fixtures() {
    check_fixtures(Model::Shakkelha, fixtures::SHAKKELHA);
}

#[test]
#[ignore = "requires local CATT model and ONNX Runtime API 31"]
fn real_catt_matches_python_onnx_fixtures() {
    check_fixtures(Model::Catt, fixtures::CATT);
}

#[test]
#[ignore = "requires both local models and ONNX Runtime API 31"]
fn real_models_preserve_documents_and_cancel_between_chunks() {
    for model in [Model::Shakkelha, Model::Catt] {
        let mut diacritizer = real_model(model, 40);
        let input = "\tٱللُّغَةُ العربية جميلة.  API 42\r\n\nنقرأ كتابا ـــ مفيدا، في المكتبة!\nﷺ ﷲ ﻻ ﺏ بٰۖ\t123٤٥\n";
        let mut progress = Vec::new();
        let output = diacritizer
            .diacritize_with_progress(input, |p| {
                progress.push(p);
                ControlFlow::Continue(())
            })
            .unwrap();
        assert_eq!(strip_tashkeel(&output), strip_tashkeel(input));
        assert_ne!(output, strip_tashkeel(input));
        for (index, p) in progress.iter().enumerate() {
            assert_eq!(p.completed, index);
            assert_eq!(p.total, progress.len() - 1);
        }
        for cancel_at in [0, 1] {
            let mut calls = Vec::new();
            let result = diacritizer.diacritize_with_progress(input, |p| {
                calls.push(p);
                if p.completed == cancel_at {
                    ControlFlow::Break(())
                } else {
                    ControlFlow::Continue(())
                }
            });
            assert!(matches!(result, Err(Error::Cancelled)));
            assert_eq!(calls.len(), cancel_at + 1);
        }
        let mut empty = Vec::new();
        assert_eq!(
            diacritizer
                .diacritize_with_progress("", |p| {
                    empty.push(p);
                    ControlFlow::Continue(())
                })
                .unwrap(),
            ""
        );
        assert_eq!(
            empty,
            [Progress {
                completed: 0,
                total: 0
            }]
        );
        assert_eq!(
            diacritizer.diacritize("API 42\r\n🙂\t").unwrap(),
            "API 42\r\n🙂\t"
        );
        let long = "هذه لغة عربية جميلة، ونقرأ كتابا مفيدا.\n".repeat(35);
        assert!(long.chars().count() > 1000);
        let output = diacritizer.diacritize(&long).unwrap();
        assert_eq!(strip_tashkeel(&output), long);
    }
}

#[test]
#[ignore = "requires local CATT model and ONNX Runtime API 31"]
fn real_catt_handles_1024_positions_and_normalization_expansion() {
    let mut diacritizer = real_model(Model::Catt, 10_000);
    for source in [
        "ب".repeat(1024),
        "ب".repeat(1025),
        format!("{} ب", "ﻻ".repeat(513)),
    ] {
        let output = diacritizer.diacritize(&source).unwrap();
        assert_eq!(strip_tashkeel(&output), source);
    }
}
