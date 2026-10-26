//! Measure the complete persistent-session text path on the current CPU.

use std::path::Path;
use std::time::Instant;

use tashkeel::{Diacritizer, InferenceOptions, Model};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    if arguments.len() != 2 {
        return Err("usage: benchmark shakkelha|catt MODEL_DIRECTORY (set ORT_DYLIB_PATH)".into());
    }
    let model: Model = arguments[0].parse()?;
    let start = Instant::now();
    let mut engine =
        Diacritizer::load(model, Path::new(&arguments[1]), InferenceOptions::default())?;
    let load_ms = start.elapsed().as_secs_f64() * 1000.0;
    let prose = "اللغة العربية جميلة ونحن نقرأ الكتب ونكتب الكلمات ونتعلم كل يوم شيئا جديدا ";
    let source = prose.repeat(200);
    let mut measurements = Vec::new();
    for length in [100, 500, 1000, 10_000] {
        let text: String = source.chars().take(length).collect();
        assert_eq!(text.chars().count(), length);
        engine.diacritize(&text)?;
        let mut times = Vec::new();
        for _ in 0..3 {
            let start = Instant::now();
            let output = engine.diacritize(&text)?;
            times.push(start.elapsed().as_secs_f64() * 1000.0);
            assert_eq!(tashkeel::text::strip_tashkeel(&output), text);
        }
        times.sort_by(f64::total_cmp);
        measurements.push(serde_json::json!({
            "characters": length, "warm_ms": times, "median_ms": times[1],
        }));
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "model": model.as_str(), "threads": 1, "chunk_size": 500,
            "load_ms": load_ms, "measurements": measurements,
            "scope": "tokenization, chunking, native inference and source reconstruction; excludes model download and queueing",
        }))?
    );
    Ok(())
}
