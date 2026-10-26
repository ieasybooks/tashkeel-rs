//! Command-line declarations, shared by help, completions and generated documentation.
#![allow(
    clippy::doc_markdown,
    reason = "doc comments are --help text, where backticks would show literally"
)]

use std::path::PathBuf;

use tashkeel::Model;

const HELP_FOOTER: &str = "\
Choose a model explicitly. Models are downloaded and hash-verified on first use unless --offline or --model-dir is supplied.
Inference requires the CPU ONNX Runtime shared library; pass its path with --ort-library. See the README for installation instructions.
Ctrl-C cancels input/setup or stops between chunks, discarding the unfinished result. Press again to exit immediately.

Use @FILE for shell-style argument files; @@ escapes a literal @.
Text, including leading/trailing whitespace and line endings, is preserved.
DOCX formatting is preserved; legacy .doc files must first be converted to .docx.";

/// Add Arabic diacritics to UTF-8 text or a DOCX document on the CPU.
///
/// Diacritize is the default command. Select shakkelha or catt explicitly.
/// Use models to prepare verified weights, completions to set up your shell,
/// or spec to export the CLI definition for documentation and other Usage tools.
#[derive(Debug, Clone, usage_rs::Cli)]
#[usage(
    bin = "tashkeel",
    version,
    repository = env!("CARGO_PKG_REPOSITORY"),
    license = env!("CARGO_PKG_LICENSE"),
    completion,
    default_subcommand = "diacritize",
    default_subcommand_flags,
    default_subcommand_help,
    arg_required_else_help,
    unknown_flags = "error",
    max_term_width = 100,
    exit_code(0, "Success"),
    exit_code(1, "Setup, filesystem, document or inference error"),
    exit_code(2, "Invalid arguments"),
    exit_code(130, "Interrupted with Ctrl-C"),
    example("tashkeel book.txt --model shakkelha -o book.diacritized.txt", help = "Diacritize a text file"),
    example("tashkeel completions zsh --install", help = "Install shell completions")
)]
pub(crate) struct Cli {
    #[usage(subcommand)]
    pub(crate) command: Command,
}

/// Operations available from the command line.
#[derive(Debug, Clone, usage_rs::Subcommands)]
pub(crate) enum Command {
    /// Diacritize one text input or DOCX document.
    Diacritize(Box<Args>),
    /// List models or download verified model files without loading ONNX Runtime.
    Models(Models),
    /// Generate or install shell completions.
    Completions(Completions),
    /// Print the Usage specification as KDL for docs, man pages and tooling.
    Spec,
}

/// Diacritize one UTF-8 text input or DOCX document, preserving its content and layout.
///
/// Supply INPUT or --text. A single dash reads UTF-8 text from stdin.
/// Text goes to stdout by default, with no added newline. DOCX requires an explicit
/// .docx output path. Existing files are replaced only with --force.
#[derive(Debug, Clone, usage_rs::Args)]
#[usage(
    unknown_flags = "error",
    max_term_width = 100,
    after_help = HELP_FOOTER,
    group("input", required),
    example("tashkeel --text 'السلام عليكم' --model shakkelha", help = "Diacritize direct text"),
    example("tashkeel - --model catt --offline", help = "Read text from stdin with a cached model"),
    example("tashkeel book.docx --model shakkelha -o book.diacritized.docx", help = "Preserve a Word document's formatting")
)]
pub(crate) struct Args {
    /// One .txt or .docx file, or - for UTF-8 text from stdin.
    #[usage(arg, name = "INPUT", group = "input", value_hint = usage_rs::ValueHint::FilePath)]
    pub(crate) input: Option<PathBuf>,

    /// Direct text instead of INPUT; whitespace is kept exactly.
    #[usage(long, value_name = "TEXT", group = "input")]
    pub(crate) text: Option<String>,

    /// Required model choice; no automatic selection or fallback.
    #[usage(long, choices("shakkelha", "catt"), value_name = "MODEL")]
    pub(crate) model: Model,

    /// Destination or - for stdout [default: stdout for text]. DOCX requires a .docx path.
    #[usage(short = 'o', long, value_name = "PATH", value_hint = usage_rs::ValueHint::FilePath)]
    pub(crate) output: Option<PathBuf>,

    /// Replace an existing output file atomically, including INPUT if explicitly selected.
    #[usage(long, negate = "no-force")]
    pub(crate) force: bool,

    /// Hide progress and setup notices; errors still appear on stderr.
    #[usage(short = 'q', long, negate = "no-quiet")]
    pub(crate) quiet: bool,

    /// Use verified model files from this directory without downloading anything.
    #[usage(long, value_name = "DIR", value_hint = usage_rs::ValueHint::DirPath)]
    pub(crate) model_dir: Option<PathBuf>,

    /// Model cache root [default: tashkeel in the operating system's user cache directory].
    #[usage(long, value_name = "DIR", value_hint = usage_rs::ValueHint::DirPath)]
    pub(crate) cache_dir: Option<PathBuf>,

    /// Require verified cached or local model files; never download.
    #[usage(long, negate = "no-offline")]
    pub(crate) offline: bool,

    /// CPU ONNX Runtime shared library (.so, .dylib or .dll).
    /// Otherwise use ORT_DYLIB_PATH, the bundled library beside tashkeel, then the system search path.
    #[usage(long, value_name = "PATH", value_hint = usage_rs::ValueHint::FilePath)]
    pub(crate) ort_library: Option<PathBuf>,

    /// Native inference threads; must be greater than zero.
    #[usage(long, value_name = "N", default = "1")]
    pub(crate) threads: usize,

    /// Maximum source Unicode characters per chunk; must be greater than zero.
    #[usage(long, value_name = "N", default = "500")]
    pub(crate) chunk_size: usize,
}

/// Model operations need no native runtime.
#[derive(Debug, Clone, usage_rs::Args)]
#[usage(unknown_flags = "error", arg_required_else_help)]
pub(crate) struct Models {
    #[usage(subcommand)]
    pub(crate) command: ModelCommand,
}

/// Explicit model management commands.
#[derive(Debug, Clone, usage_rs::Subcommands)]
pub(crate) enum ModelCommand {
    /// List the available backend IDs and their quality caveats.
    List,
    /// Download and hash-verify the selected model in the cache.
    Download(Download),
}

/// Prepare the selected model without loading ONNX Runtime or running inference.
#[derive(Debug, Clone, usage_rs::Args)]
#[usage(
    unknown_flags = "error",
    example(
        "tashkeel models download --model shakkelha",
        help = "Prepare the model for later offline use"
    )
)]
pub(crate) struct Download {
    /// Required model to download and verify.
    #[usage(long, choices("shakkelha", "catt"), value_name = "MODEL")]
    pub(crate) model: Model,

    /// Model cache root [default: tashkeel in the operating system's user cache directory].
    #[usage(long, value_name = "DIR", value_hint = usage_rs::ValueHint::DirPath)]
    pub(crate) cache_dir: Option<PathBuf>,

    /// Hide setup notices; the verified model directory is still printed to stdout.
    #[usage(short = 'q', long)]
    pub(crate) quiet: bool,
}

/// Generate a completion script, or install it in the shell's user directory.
///
/// Scripts call tashkeel directly; no separate usage executable is needed.
/// Installation prints any shell setup still required and preserves unrelated files.
#[derive(Debug, Clone, usage_rs::Args)]
#[usage(
    unknown_flags = "error",
    example("tashkeel completions bash", help = "Print a script to stdout"),
    example(
        "tashkeel completions zsh --install --dry-run",
        help = "Preview installation"
    )
)]
pub(crate) struct Completions {
    /// Shell to generate completions for.
    #[usage(arg, choices("bash", "elvish", "fish", "nu", "powershell", "zsh"))]
    pub(crate) shell: String,

    /// Install into this shell's user completion directory.
    #[usage(long)]
    pub(crate) install: bool,

    /// Show the destination and setup instructions without writing a file.
    #[usage(long, requires("--install"))]
    pub(crate) dry_run: bool,
}
