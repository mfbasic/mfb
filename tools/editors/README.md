# Editor integrations

Editor support for MFBASIC, versioned in-tree so it stays in sync with the
compiler.

- [`vscode/`](vscode/) — Visual Studio Code extension: a TextMate grammar that
  highlights `.mfb` source. Also consumable by Sublime Text, Zed, and GitHub
  Linguist.

Planned (not yet built):

- A Tree-sitter grammar (Neovim, Helix, Zed-native, GitHub semantic highlighting).
- A language server reusing the Rust lexer/parser/type-checker for diagnostics,
  hover, and go-to-definition.
