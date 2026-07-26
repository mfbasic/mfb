//! Help- and usage-screen text for the `mfb` CLI. Each constant is the exact
//! body printed for `--help`/`-h` on the corresponding command (or the
//! top-level `USAGE` for a bare `mfb`). Text is output-neutral: do not reword.

pub(crate) const USAGE: &str = "\
Usage: mfb <command> [arguments]

Project Setup:
  init <path>             Create a new MFBASIC executable project
  init-pkg <path>         Create a new MFBASIC package project

Package Management:
  pkg add <target>        Add a package: file:// URL or <owner>#<pkg>[@ver] ident
  pkg update              Resolve dependencies and write mfb.lock
  pkg install             Install dependencies from mfb.lock (by hash)
  pkg verify              Verify packages declared in project.json
  Run 'mfb pkg --help' for all package commands.

Repository, Auth & Publishing:
  repo register <owner>   Register a repository owner
  repo auth <owner>       Authenticate as a repository owner
  repo publish <owner>    Sign and publish a package project to a repository
  Run 'mfb repo --help' for all repository, auth & publishing commands.

Build & Development:
  build [options] [path]  Validate and build an MFBASIC project
  test [options] [path]   Build and run the project's TESTING blocks
  fmt [options] [path]    Format project source (indentation/capitalization)
  audit [options] [path]  Report security and code audit findings

Documentation & Reference:
  doc [options] [path]    Render HTML docs from package or file source
  pkg doc <pkg> [options] Render HTML docs from a compiled package
  man [pkg] [func]        Show built-in package and function help
  spec [topic] [sub]      Show the MFBASIC language specification
  help                    Show this message
  --version               Show the compiler version and build provenance

Run 'mfb <command> --help' for more information on a specific command.";

pub(crate) const INIT_HELP: &str = "\
Usage: mfb init <path>

Create a new MFBASIC executable project at the specified path.

Arguments:
  <path>      The directory where the project will be initialized.";

pub(crate) const INIT_PKG_HELP: &str = "\
Usage: mfb init-pkg <path>

Create a new MFBASIC package project (library) at the specified path.

Arguments:
  <path>      The directory where the project will be initialized.";

pub(crate) const PKG_HELP: &str = "\
Usage: mfb pkg <command> [arguments]

Commands:
  add <target> [--pin|--no-pin]
                          Add a package: file:// URL or <owner>#<pkg>[@ver] ident
  info <pkg>              Show metadata and dependencies of a compiled package
  doc <pkg> [--out <f>]   Render HTML documentation from a compiled package
  verify [--proof]        Verify packages declared in project.json
  validate <pkg>          Check an existing package's signatures and structure
  install [path]          Install dependencies from mfb.lock (by hash)
  remove <owner>#<pkg>    Remove a package and everything that imports it
  update                  Re-resolve all dependencies and write mfb.lock
  update <owner>#<pkg>[@ver] [--pin|--no-pin] [--yes]
                          Update one declared dependency

Publishing a package? Those commands live under 'mfb repo' —
see 'mfb repo --help'.

Options:
  --proof                 (verify) Also print each dependency's inclusion proof
  --out <file>            (doc) Path to the generated HTML file (default: index.html)
  --pin                   (add, update) Record the exact version; never float
  --no-pin                (add, update) Float above this version, which becomes the ABI floor
  --yes                   (update, remove) Skip the confirmation prompt";

pub(crate) const REPO_HELP: &str = "\
Usage: mfb repo <command> [arguments]
       mfb machine|key|org|token <command> [arguments]

Repository:
  repo register <owner>   Register a new repository owner identity
  repo auth <owner>       Log in to an existing owner account
  repo link --start <owner>
                          (old machine) display a one-time pairing code
  repo link <owner>       (new machine) enter the pairing code to become an equal
  repo trust <registry-id> <root-fingerprint>
                          Pin and verify a registry's signed-metadata root

Publishing:
  repo publish <owner> [path]
                          Sign and publish a package project to a repository
  repo check-abi [path]   Diff this package's ABI against its published version
  repo release-state <state> [version]
                          Set a published version's state (available/deprecated/yanked)
  repo transfer <owner>#<pkg> <to-owner>
                          Offer a package to another owner
  repo transfer-accept <owner>#<pkg>@<to-owner>
                          Accept a pending package transfer

Machines & Keys:
  machine revoke <owner> <auth-fingerprint>
                          Revoke a lost machine's auth key (needs the ident key)
  key rotate <owner>      Rotate the account ident; consumers follow the chain

Organizations:
  org grant <org> <member> <role>
                          Grant a member an org role (owner/admin/publisher)
  org remove <org> <member>
                          Remove a member from an org

Publish Tokens:
  token issue <owner> <scope> <ttl-seconds>
                          Issue a scoped, short-lived publish token
  token revoke <owner> <token-fingerprint>
                          Revoke a publish token

Arguments:
  <owner>                 The unique handle for the repository owner";

pub(crate) const BUILD_HELP: &str = "\
Usage: mfb build [options] [path]

Validate and compile an MFBASIC project.

Arguments:
  [path]              Path to the project (default: current directory)

Options:
  --sign <owner>      Sign the resulting binary with the specified owner
  --target <os-arch>  Cross-compile to a specific target (e.g., linux-x86_64)
  --regalloc <name>   Select the register-allocation strategy
  --app               Build as a standalone application instead of a library
  --app-debug         Like --app, but keep the intermediate build/<name>.AppDir
                      beside the AppImage (Linux; inert on macOS)
  --unsigned          Allow unsigned dependencies from a non-local source
  -q, --quiet         Print only the artifact line and any diagnostics
  -v, --verbose       Also print a per-phase timing line for each build stage

Debug/Inspection (Emits intermediate output):
  --ast               Outputs Abstract Syntax Tree
  --ir                Outputs Intermediate Representation
  --br                Outputs MFPC binary representation
  --mir               Outputs Mid-level IR
  --nir               Outputs native IR
  --nplan             Outputs the execution plan
  --nobj              Outputs the object plan
  --ncode             Outputs native code output";

pub(crate) const TEST_HELP: &str = "\
Usage: mfb test [options] [path]

Build and run the project's TESTING blocks, streaming a pass/fail tree and a
summary line. Exits non-zero iff any case failed.

Arguments:
  [path]              Path to the project (default: current directory)

Options:
  --coverage          Emit coverage.html for the exercised source lines
  --target <os-arch>  Build for a specific target (only host targets are run)
  --regalloc <name>   Select the register-allocation strategy";

pub(crate) const FMT_HELP: &str = "\
Usage: mfb fmt [options] [path]

Format MFBASIC source files for consistent indentation and capitalization.

Options:
  --check             Check if files are formatted without writing changes
  --indent <N>        Set the number of spaces for indentation (default: 2)

Arguments:
  [path]              File or directory to format (default: current directory)";

pub(crate) const AUDIT_HELP: &str = "\
Usage: mfb audit [options] [path]

Scan the project for security vulnerabilities and code smells.

Options:
  --format <type>     Output format: text, json (default: text)
  --locked            Only audit packages defined in project.lock";

pub(crate) const DOC_HELP: &str = "\
Usage: mfb doc [options] [path]

Render HTML documentation from source files or a project directory.

Options:
  --out <file>        Path to the generated HTML file (default: index.html)

Arguments:
  [path]              Source file or project folder to document";

pub(crate) const MAN_HELP: &str = "\
Usage: mfb man [package] [function] [options]

Display the built-in manual for packages and specific functions.

Options:
  --all               Print the whole manual, or one package in full

Examples:
  mfb man standard print
  mfb man io --all
  mfb man --all";

pub(crate) const SPEC_HELP: &str = "\
Usage: mfb spec [topic] [subtopic] [options]

Display the formal MFBASIC language specification.

Options:
  --all               Print the entire specification to the console

Example:
  mfb spec types integer";
