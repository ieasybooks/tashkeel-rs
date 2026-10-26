# Application integration

Create and retain a `Diacritizer` for each inference worker. Loading on every request adds latency and duplicates native allocations. Begin with one blocking worker, one inference thread, and batch one, then measure latency and memory before increasing concurrency. The library methods are synchronous; do not execute them directly on a Tokio executor thread.

Use `models::ensure` during setup or warmup, outside request-critical work, and call `Diacritizer::load` with the resulting directory. Persistent sessions do not spawn Python. The caller controls storage, input limits, model selection, and the native library location.

`diacritize_with_progress` invokes a callback between bounded chunks. Return `ControlFlow::Break(())` to cancel. Cancellation is cooperative: allow the current native call to finish and retain the worker/queue slot until it does. Do not pretend a detached blocking task has stopped when an async request is cancelled. Progress contains completed and total chunk counts for that text operation; document paragraphs may start separate operations.

`docx::transform` accepts a closure over paragraph text. Reuse the same engine inside it and propagate cancellation/errors. Keep the result in temporary storage until processing completes, then publish the output atomically. Review [DOCX limits](docx.md) before accepting document uploads.

The intended `arlang.tools` interface is a maximum of 1,000 characters for interactive text and one TXT or DOCX upload. Count characters as Unicode scalars rather than UTF-8 bytes, and apply separate file/decompression limits. The Rust library intentionally supports longer documents and has no 1,000-character total-input restriction. Model normalization, source reconstruction, and the CATT normalized-position limit are internal concerns; the application's user-facing output remains the original document with updated Arabic marks.
