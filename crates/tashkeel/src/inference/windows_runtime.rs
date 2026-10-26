use std::path::Path;

use libloading::os::windows::{
    LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR, LOAD_LIBRARY_SEARCH_SYSTEM32, Library,
};

use crate::{Error, Result};

pub(super) fn preload(path: &Path) -> Result<Library> {
    if !path.is_absolute() {
        return Err(Error::Inference(
            "Windows runtime preload requires an absolute library path".into(),
        ));
    }
    // SAFETY: the caller selects and canonicalizes a trusted native runtime,
    // whose initialization/termination code is already part of ort's loading
    // contract. Its directory and System32 are the only dependency search
    // directories; neither CWD nor PATH is searched. No symbols are accessed
    // here. The caller holds this RAII handle through ort::init_from/build,
    // after which ort retains its own reference. Errors drop the handle.
    // ort rc.15 has no safe API for supplying these per-load search flags.
    #[allow(unsafe_code)]
    let library = unsafe {
        Library::load_with_flags(
            path,
            LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32,
        )
    };
    library.map_err(|error| {
        Error::Inference(format!(
            "cannot load ONNX Runtime library {} with restricted Windows dependency search: {error}",
            path.display()
        ))
    })
}
