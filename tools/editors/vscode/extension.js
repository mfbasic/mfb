// MFBASIC editor features that don't need a language server.
//
// Registers a DocumentSymbolProvider (Outline, breadcrumbs, sticky scroll,
// Go to Symbol) and a FoldingRangeProvider (precise block folding). The actual
// parsing lives in mfbasic-parse.js, which is plain, vscode-free, and unit
// tested; this file only maps its plain-data results onto the VS Code APIs.

'use strict';

const vscode = require('vscode');
const { parseSymbols, parseFoldingRanges } = require('./mfbasic-parse');

const SYMBOL_KIND = {
  function: vscode.SymbolKind.Function,
  method: vscode.SymbolKind.Method,
  struct: vscode.SymbolKind.Struct,
  enum: vscode.SymbolKind.Enum,
  enummember: vscode.SymbolKind.EnumMember,
  field: vscode.SymbolKind.Field,
  module: vscode.SymbolKind.Module,
};

function toDocumentSymbol(doc, node) {
  const lastLine = Math.min(node.endLine, doc.lineCount - 1);
  const fullRange = new vscode.Range(node.startLine, 0, lastLine, doc.lineAt(lastLine).text.length);
  const selRange = new vscode.Range(node.nameLine, node.nameStart, node.nameLine, node.nameEnd);
  const sym = new vscode.DocumentSymbol(
    node.name,
    node.detail || '',
    SYMBOL_KIND[node.kind] || vscode.SymbolKind.Variable,
    fullRange,
    // Keep the selection range inside the full range (VS Code requires it).
    fullRange.contains(selRange) ? selRange : fullRange,
  );
  sym.children = (node.children || []).map((c) => toDocumentSymbol(doc, c));
  return sym;
}

const symbolProvider = {
  provideDocumentSymbols(doc) {
    return parseSymbols(doc.getText()).map((n) => toDocumentSymbol(doc, n));
  },
};

const foldingProvider = {
  provideFoldingRanges(doc) {
    return parseFoldingRanges(doc.getText()).map((r) => {
      const kind = r.kind === 'comment' ? vscode.FoldingRangeKind.Comment
        : r.kind === 'region' ? vscode.FoldingRangeKind.Region
          : undefined;
      return new vscode.FoldingRange(r.start, r.end, kind);
    });
  },
};

function activate(context) {
  const selector = { language: 'mfbasic' };
  context.subscriptions.push(
    vscode.languages.registerDocumentSymbolProvider(selector, symbolProvider),
    vscode.languages.registerFoldingRangeProvider(selector, foldingProvider),
  );
}

function deactivate() {}

module.exports = { activate, deactivate };
