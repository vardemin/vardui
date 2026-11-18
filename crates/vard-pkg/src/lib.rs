//! Vard packages: the data and protocol layer under `vard add/install/publish`.
//!
//! - [`manifest`] `vard.toml` (apps and packages), [`lock`] `vard.lock`
//! - [`semver`] versions and requirements, [`resolve`] minimal version selection
//! - [`vpk`] deterministic archives, [`registry`] static, content-addressed registries
//!   (builtin / directory / HTTP) with a signed append-only [`log`]
//! - [`project`] loading an app with its dependencies for the compiler
//!
//! Everything a person or an agent sees is a [`Problem`] with a stable `P0xx` code
//! (`vard explain P008`), the file line when there is one, and a fix when it is knowable.

pub mod builtin;
pub mod log;
pub mod lock;
pub mod manifest;
pub mod net;
pub mod project;
pub mod registry;
pub mod resolve;
pub mod semver;
pub mod toml;
pub mod vpk;

use sha2::{Digest, Sha256};

pub const VARD_VERSION: &str = env!("CARGO_PKG_VERSION");

/// The public registry: a GitHub repository (vardemin/vard-registry) served by GitHub
/// Pages. Its signing key is compiled in, so the very first contact is already verified.
pub const PUBLIC_REGISTRY_NAME: &str = "vard";
pub const PUBLIC_REGISTRY_URL: &str = "https://vardemin.github.io/vard-registry";
pub const PUBLIC_REGISTRY_KEY: &str = "0f614a9c2b3b110841ebfbb449142b11b3596bc165ecf42023dc749c5cab4286";

/// AGENT.md budget: what an agent reads to use a package (~1500 tokens).
pub const AGENT_DOC_BUDGET: usize = 6144;
/// Upper bound for a package archive.
pub const MAX_PACKAGE_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq)]
pub struct Problem {
    pub code: &'static str,
    /// line in vard.toml (0 = not about a line)
    pub line: usize,
    pub msg: String,
    /// replacement for that line, or a command to run
    pub fix: Option<String>,
}

impl Problem {
    pub fn new(code: &'static str, msg: impl Into<String>) -> Problem {
        Problem { code, line: 0, msg: msg.into(), fix: None }
    }
    pub fn fix(mut self, f: impl Into<String>) -> Problem {
        self.fix = Some(f.into());
        self
    }
    /// `error P008: ...` (+ `vard.toml:LINE` and `fix:` lines).
    pub fn render(&self, file: &str) -> String {
        let mut s = if self.line > 0 { format!("{}:{} error {}: {}\n", file, self.line, self.code, self.msg) } else { format!("error {}: {}\n", self.code, self.msg) };
        if let Some(f) = &self.fix {
            s.push_str(&format!("  fix: {}\n", f));
        }
        s
    }
}

pub fn render_all(ps: &[Problem], file: &str) -> String {
    ps.iter().map(|p| p.render(file)).collect()
}

pub fn sha256_hex(data: &[u8]) -> String {
    let d = Sha256::digest(data);
    d.iter().map(|b| format!("{:02x}", b)).collect()
}

/// `vard explain P0xx`.
pub const EXPLAIN: &[(&str, &str)] = &[
    ("P001", "No package with this name in the registry. Package managers never guess: check the `did you mean` list or search (`vard search WORDS`). A name an LLM invents is the classic way to install an attacker's package (slopsquatting), so `vard add` only installs exact names."),
    ("P002", "The registry could not be reached or answered something that is not a Vard registry. Check the URL in vard.toml [registries] / ~/.vard/config.toml; offline, only builtin, path and cached packages work."),
    ("P003", "No published version matches the requirement. `\"1.4\"` means at least 1.4.0 and below 2.0.0. `vard info NAME` lists the versions."),
    ("P004", "Integrity failure: the archive's SHA-256 differs from the index/lockfile, or the version is not in the registry's signed log. Do not work around it; the content is not what was published."),
    ("P005", "The registry's signing key differs from the one pinned on first use (~/.vard/known_registries), or its log was rewritten. Either the registry rotated its key (confirm out of band, then remove the old line) or someone is impersonating it."),
    ("P006", "Two packages need incompatible versions of one package (e.g. 1.x and 2.x). A Vard app links one version of each package, so the requirements must meet on one line: upgrade the older dependent, or pin the shared package to the common line."),
    ("P007", "The package needs a newer (or older) Vard core than this toolchain. Upgrade vard, or require an older version of the package."),
    ("P008", "The dependency needs a sensitive capability (secure, permissions, push, native, network) that the app's [policy] does not allow. This is a human decision: add it to `[policy] allow` in vard.toml (it shows up in code review), or pass `--allow CAP` on the command line."),
    ("P009", "The version was yanked by its author (a serious bug or a leaked secret). Existing lockfiles keep working; new resolutions skip it. Upgrade to a newer version."),
    ("P010", "Package names are lowercase identifiers ([a-z][a-z0-9_]*, at most 32 characters) so they work as qualifiers in code (`charts.LineChart`). Reserved words and names one edit away from an existing package of another owner are refused to prevent typosquatting."),
    ("P011", "Every package ships AGENT.md: what an agent reads to use it (`vard doc NAME`). It must exist and fit the budget (6 KB ≈ 1500 tokens): purpose, the main views/functions with one example each, headless testing notes. The API list is generated; do not repeat it."),
    ("P012", "The package does not compile (with its declared dependencies). Run `vard check` in the package directory."),
    ("P013", "The capabilities declared in [package] capabilities must match what the compiler computes from the code (`vard api --caps`). Missing ones would hide what the package can do; unused ones would ask consumers for permissions it does not need."),
    ("P014", "The package's tests failed. Publishing runs every .vtest of the package (and of example/) headless; fix them first (`vard test`)."),
    ("P015", "The version number does not match the API change. The compiler diffs the public API against the previous version: removing or changing something is a major change, adding is minor, anything else is a patch. `vard publish --dry-run` prints the diff and the minimal version."),
    ("P016", "This version already exists, or is not newer than the latest version on its line. Published versions are immutable; publish a new version."),
    ("P017", "[sim] answers refer to externs the package does not declare."),
    ("P018", "Publishing needs a token of the package's owner (`--token`, VARD_TOKEN or ~/.vard/credentials.toml). The first publisher of a name owns it."),
    ("P019", "The package archive is too large (8 MB limit). Ship code, docs and small assets; fetch big media at runtime."),
    ("P020", "vard.toml is not valid TOML (the subset Vard uses: [tables], key = \"string\" | 123 | true | [..] | { .. })."),
    ("P021", "A field of vard.toml has a wrong value or is unknown. The message names the field and the expected form."),
    ("P022", "The project's packages are missing or out of date. `vard install` resolves vard.toml, fetches and verifies packages and rewrites vard.lock."),
];

pub fn explain(code: &str) -> Option<&'static str> {
    EXPLAIN.iter().find(|(c, _)| c.eq_ignore_ascii_case(code)).map(|(_, t)| *t)
}
