use std::{
    path::{Path, PathBuf},
    sync::Mutex,
};

use ort::{
    environment::Environment,
    session::{Session, builder::GraphOptimizationLevel},
    value::{Outlet, Tensor, TensorElementType, ValueType},
};

use crate::{Error, InferenceOptions, Model, Result};

// ort rc.15 permits one Environment creation for the lifetime of the process,
// even after its last session is dropped. Keep it alive and share clones.
static RUNTIME: Mutex<Option<(PathBuf, Environment)>> = Mutex::new(None);

pub(super) enum Backend {
    Shakkelha(Box<Session>),
    Catt {
        encoder: Box<Session>,
        decoder: Box<Session>,
    },
}

fn failure(error: impl std::fmt::Display) -> Error {
    Error::Inference(error.to_string())
}

fn environment(options: &InferenceOptions) -> Result<Environment> {
    let library_name = format!(
        "{}onnxruntime{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    );
    let requested = options
        .runtime_path
        .clone()
        .or_else(|| {
            std::env::var_os("ORT_DYLIB_PATH")
                .filter(|p| !p.is_empty())
                .map(PathBuf::from)
        })
        .or_else(|| {
            let executable = std::env::current_exe().ok()?;
            let adjacent = executable.parent()?.join(&library_name);
            adjacent.is_file().then_some(adjacent)
        });
    let requested = resolve_runtime_path(requested, &library_name)?;
    let mut loaded = RUNTIME
        .lock()
        .map_err(|_| failure("runtime initialization lock poisoned"))?;
    if let Some((previous, environment)) = loaded.as_ref() {
        if previous != &requested {
            return Err(failure(format!(
                "ONNX Runtime is already initialized from {}; cannot switch to {} in the same process",
                previous.display(),
                requested.display()
            )));
        }
        return Ok(environment.clone());
    }
    // Explicit initialization returns load errors instead of ort's implicit
    // initialization panic when the native library is missing or too old.
    let environment = ort::init_from(&requested)
        .map_err(failure)?
        .build()
        .map_err(failure)?;
    *loaded = Some((requested, environment.clone()));
    Ok(environment)
}

fn resolve_runtime_path(requested: Option<PathBuf>, library_name: &str) -> Result<PathBuf> {
    if let Some(path) = requested {
        // Configured paths must identify a real file. Passing a missing bare
        // name onward would unexpectedly fall back to the loader's search.
        return path.canonicalize().map_err(|error| {
            failure(format!(
                "cannot resolve ONNX Runtime library {}: {error}",
                path.display()
            ))
        });
    }
    if cfg!(windows) {
        // LoadLibrary's default search includes the current directory, which
        // may contain an uploaded document and an unrelated DLL beside it.
        return Err(failure(
            "Windows requires onnxruntime.dll beside the executable, or an explicit library path through runtime_path / --ort-library / ORT_DYLIB_PATH",
        ));
    }
    Ok(library_name.into())
}

fn session(environment: &Environment, path: &Path, threads: usize) -> Result<Session> {
    Session::builder(environment)
        .map_err(failure)?
        .with_intra_threads(threads)
        .map_err(failure)?
        .with_inter_threads(1)
        .map_err(failure)?
        .with_parallel_execution(false)
        .map_err(failure)?
        .with_intra_op_spinning(false)
        .map_err(failure)?
        .with_inter_op_spinning(false)
        .map_err(failure)?
        .with_optimization_level(GraphOptimizationLevel::Level3)
        .map_err(failure)?
        .commit_from_file(path)
        .map_err(failure)
}

/// A negative expected dimension requires a dynamic axis. Batch may be fixed
/// at one; feature dimensions must be explicit so wrong models fail at load.
fn outlet(outlets: &[Outlet], name: &str, ty: TensorElementType, expected: &[i64]) -> Result<()> {
    let Some(outlet) = outlets.iter().find(|outlet| outlet.name() == name) else {
        return Err(failure(format!("model lacks tensor {name:?}")));
    };
    let ValueType::Tensor {
        ty: actual_ty,
        shape,
        ..
    } = outlet.dtype()
    else {
        return Err(failure(format!("{name} must be a tensor")));
    };
    let valid = *actual_ty == ty
        && shape.len() == expected.len()
        && shape
            .iter()
            .zip(expected)
            .enumerate()
            .all(|(axis, (&actual, &expected))| {
                actual == expected || (axis == 0 && expected == 1 && actual == -1)
            });
    if !valid {
        return Err(failure(format!(
            "{name} has incompatible type or shape: {:?}; expected {ty:?} {expected:?}",
            outlet.dtype()
        )));
    }
    Ok(())
}

fn single_output(session: &Session, width: i64) -> Result<()> {
    if session.outputs().len() != 1 {
        return Err(failure("model must expose exactly one output"));
    }
    outlet(
        session.outputs(),
        session.outputs()[0].name(),
        TensorElementType::Float32,
        &[1, -1, width],
    )
}

impl Backend {
    pub(super) fn load(model: Model, directory: &Path, options: &InferenceOptions) -> Result<Self> {
        let names: &[&str] = match model {
            Model::Shakkelha => &["model.onnx"],
            Model::Catt => &["encoder.onnx", "decoder.onnx"],
        };
        for name in names {
            let path = directory.join(name);
            if !path.is_file() {
                return Err(Error::Model(format!(
                    "missing model file {}",
                    path.display()
                )));
            }
        }
        let environment = environment(options)?;
        match model {
            Model::Shakkelha => {
                let session =
                    session(&environment, &directory.join("model.onnx"), options.threads)?;
                if session.inputs().len() != 1 {
                    return Err(failure("Shakkelha must have exactly one input"));
                }
                outlet(
                    session.inputs(),
                    "input_ids",
                    TensorElementType::Int64,
                    &[1, -1],
                )?;
                single_output(&session, 19)?;
                Ok(Self::Shakkelha(Box::new(session)))
            }
            Model::Catt => {
                let encoder = session(
                    &environment,
                    &directory.join("encoder.onnx"),
                    options.threads,
                )?;
                let decoder = session(
                    &environment,
                    &directory.join("decoder.onnx"),
                    options.threads,
                )?;
                if encoder.inputs().len() != 2 || decoder.inputs().len() != 1 {
                    return Err(failure(
                        "CATT encoder-only requires two encoder inputs and one classifier input",
                    ));
                }
                outlet(encoder.inputs(), "src", TensorElementType::Int64, &[1, -1])?;
                outlet(
                    encoder.inputs(),
                    "src_mask",
                    TensorElementType::Bool,
                    &[1, 1, -1, -1],
                )?;
                single_output(&encoder, 512)?;
                outlet(
                    decoder.inputs(),
                    "enc_src",
                    TensorElementType::Float32,
                    &[1, -1, 512],
                )?;
                single_output(&decoder, 18)?;
                Ok(Self::Catt {
                    encoder: Box::new(encoder),
                    decoder: Box::new(decoder),
                })
            }
        }
    }

    pub(super) fn predict(&mut self, ids: &[i64], classes: usize) -> Result<Vec<usize>> {
        if ids.is_empty() {
            return Err(failure("cannot infer an empty token sequence"));
        }
        let input = Tensor::from_array(([1, ids.len()], ids.to_vec())).map_err(failure)?;
        match self {
            Self::Shakkelha(session) => {
                let output = session
                    .run(ort::inputs!["input_ids" => &input])
                    .map_err(failure)?;
                let (shape, scores) = output[0].try_extract_tensor::<f32>().map_err(failure)?;
                predictions(shape, scores, ids.len(), classes)
            }
            Self::Catt { encoder, decoder } => {
                if ids.len() > 1024 {
                    return Err(failure(
                        "CATT supports at most 1024 normalized positions per chunk",
                    ));
                }
                // There is no padding at batch one, so the full attention mask
                // is true. Do not impose a causal mask on this encoder.
                let mask = Tensor::from_array((
                    [1, 1, ids.len(), ids.len()],
                    vec![true; ids.len() * ids.len()],
                ))
                .map_err(failure)?;
                let encoded = encoder
                    .run(ort::inputs!["src" => &input, "src_mask" => &mask])
                    .map_err(failure)?;
                let (shape, _) = encoded[0].try_extract_tensor::<f32>().map_err(failure)?;
                check_shape(shape, ids.len(), 512)?;
                let output = decoder
                    .run(ort::inputs!["enc_src" => &encoded[0]])
                    .map_err(failure)?;
                let (shape, scores) = output[0].try_extract_tensor::<f32>().map_err(failure)?;
                predictions(shape, scores, ids.len(), classes)
            }
        }
    }
}

fn check_shape(shape: &[i64], positions: usize, classes: usize) -> Result<()> {
    let positions = i64::try_from(positions).map_err(failure)?;
    let classes = i64::try_from(classes).map_err(failure)?;
    if shape != [1, positions, classes] {
        return Err(failure(format!(
            "unexpected output shape {shape:?}; expected [1, {positions}, {classes}]"
        )));
    }
    Ok(())
}

fn predictions(
    shape: &[i64],
    scores: &[f32],
    positions: usize,
    classes: usize,
) -> Result<Vec<usize>> {
    check_shape(shape, positions, classes)?;
    if classes == 0
        || positions.checked_mul(classes) != Some(scores.len())
        || scores.iter().any(|x| !x.is_finite())
    {
        return Err(failure("model returned invalid or non-finite scores"));
    }
    Ok(scores
        .chunks_exact(classes)
        .map(|row| {
            // NumPy argmax chooses the first index on ties.
            row.iter().enumerate().skip(1).fold(
                0,
                |best, (index, score)| {
                    if *score > row[best] { index } else { best }
                },
            )
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::{predictions, resolve_runtime_path};

    #[test]
    fn configured_runtime_paths_cannot_fall_back_to_library_search() {
        let directory = tempfile::tempdir().unwrap();
        let missing = directory.path().join("missing-runtime");
        assert!(resolve_runtime_path(Some(missing), "unused").is_err());
        let file = tempfile::NamedTempFile::new_in(directory.path()).unwrap();
        assert_eq!(
            resolve_runtime_path(Some(file.path().to_owned()), "unused").unwrap(),
            file.path().canonicalize().unwrap()
        );
    }

    #[test]
    fn platform_loader_fallback_is_disabled_on_windows() {
        let resolved = resolve_runtime_path(None, "onnxruntime-test");
        if cfg!(windows) {
            assert!(
                resolved
                    .unwrap_err()
                    .to_string()
                    .contains("Windows requires")
            );
        } else {
            assert_eq!(resolved.unwrap(), std::path::Path::new("onnxruntime-test"));
        }
    }

    #[test]
    fn scores_validate_shapes_classes_and_finiteness() {
        assert_eq!(
            predictions(&[1, 2, 3], &[1., 1., 0., -2., 0., 1.], 2, 3).unwrap(),
            [0, 2]
        );
        assert!(predictions(&[2, 1, 3], &[0.; 6], 2, 3).is_err());
        assert!(predictions(&[1, 2, 3], &[0.; 5], 2, 3).is_err());
        assert!(predictions(&[1, 1, 2], &[f32::NAN, 1.], 1, 2).is_err());
        assert!(predictions(&[1, 1, 2], &[0., f32::INFINITY], 1, 2).is_err());
    }
}
