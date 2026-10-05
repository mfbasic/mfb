//! `os::getConfigOr` — descriptor entry + authored docs (plan-167-B).
//!
//! The three overloads are [`Body::Intrinsic`]: they exist for type checking and
//! docs only. `ir::shape` requires both arguments to be literals, and
//! `target::shared::config_fold::fold_config_calls` replaces every call with the
//! selected config's value (or the default) as packages merge, so no call
//! reaches codegen.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str =
    r#"Read a build-time setting from project.json, or a default when it is not declared"#;
const DESC: &str = r#"`os::getConfigOr` returns the value the selected config declares for `key`, or
`default` when that config does not declare `key`.

Configs are named sets of settings in the `config` section of `project.json`.
`release` is required; add as many others as you need:

```json
"config": {
  "release": { "apiUrl": "https://api.example.com", "retries": 3, "timeout": 2.5 },
  "dev":     { "apiUrl": "http://localhost:8080", "retries": 0, "timeout": 30.0 }
}
```

`mfb build` reads from `release`. `mfb build --config dev` (or `--config=dev`)
reads from `dev` instead, and `mfb test` takes the same flag. A name that is not
declared stops the build and lists the names that are.

Settings that belong to one machine, such as API keys, passwords, or a local
server address, go in an optional `project-local.json` beside `project.json`.
It holds only a `config` section. Its values replace the same keys of the same
config, add new keys, or add whole new configs that only that machine can
select. A replaced value must keep its type. Keep `project-local.json` out of
source control; `mfb init` writes a `.gitignore` that excludes it:

```json
{ "config": { "dev": { "apiKey": "my-secret-key" } } }
```

The value is fixed when the program is built, not read when it runs. Changing
either file changes the program only after the next build, and the built
program needs neither file beside it.

There are three forms, one per value type. The type of `default` picks the form
and the result type: a String default returns a String, an Integer default an
Integer, and a Float default a Float. A number written with a `.` or an exponent
(`2.5`, `1e3`) is a Float in the config; a number without one (`3`) is an
Integer. A key has the same type in every config that declares it.

- **Both arguments must be literals.** `key` is a String literal and `default`
  is a literal of the form's type; a negative number such as `-1` or `-0.5`
  counts. A variable, an expression or a function call in either position stops
  the build.
- **A missing key is not an error.** It yields `default`. Keys match exactly, so
  `"Port"` does not find a declared `port`.
- **A type mismatch stops the build.** If the config declares `retries` as an
  Integer, `os::getConfigOr("retries", "3")` does not build.
- **In a package**, a call reads the settings of the program being built, not
  the package's own `project.json` or `project-local.json`.

Every value a program reads this way, including one from `project-local.json`,
is stored as plain text in the built program, so anyone holding the program file
can read it. Values the program never reads are left out."#;
const EX: &str = r#"Read three settings with defaults. With the `config` section above, this
prints `https://api.example.com`, `3`, `2.50` and `false`; with no `config`
section it prints `http://localhost:8080`, `1`, `10.00` and `false`:

```
IMPORT os
IMPORT io

FUNC main() AS Nothing
  LET url AS String = os::getConfigOr("apiUrl", "http://localhost:8080")
  LET retries AS Integer = os::getConfigOr("retries", 1)
  LET timeout AS Float = os::getConfigOr("timeout", 10.0)
  io::print(url)
  io::print(toString(retries))
  io::print(toString(timeout))
  io::print(os::getConfigOr("verbose", "false"))
END FUNC
```"#;

fn overload(ty: ParameterType, default_desc: &'static str) -> Implementation {
    Implementation {
        params: vec![
            Parameter {
                name: "key",
                desc: "The setting's name, as a String literal. Matched exactly against the keys of the selected config.",
                aliases: &[],
                ty: ParameterType::String,
                default: DefaultValue::None,
            },
            Parameter {
                name: "default",
                desc: default_desc,
                aliases: &[],
                ty: ty.clone(),
                default: DefaultValue::None,
            },
        ],
        return_type: ty,
        errors: vec![],
        body: Body::Intrinsic,
    }
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "getConfigOr",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![
            overload(
                ParameterType::String,
                "The String literal returned when the config does not declare `key`. A declared value that is not a String stops the build.",
            ),
            overload(
                ParameterType::Integer,
                "The Integer literal returned when the config does not declare `key`. A declared value that is not an Integer stops the build.",
            ),
            overload(
                ParameterType::Float,
                "The Float literal returned when the config does not declare `key`. A declared value that is not a Float stops the build.",
            ),
        ],
    });
}
