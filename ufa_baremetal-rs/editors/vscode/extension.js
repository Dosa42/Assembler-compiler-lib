// VS Code client for ufa-baremetal-lsp.
//
// Deliberately hand-rolled instead of using the `vscode-languageclient`
// npm package: this file only ever calls into `vscode` (supplied by the
// extension host itself, not an npm install) and Node's built-in
// `child_process`. No package-lock.json, no node_modules, no build step
// -- install this folder as-is (see ../../bin/ufa-baremetal-lsp/README.md
// for the exact steps). Continues the same "stdlib only, no dependency
// you don't need" discipline the rest of this workspace holds itself to
// (see e.g. lib/ufa_baremetal_core/Cargo.toml's own comment on this) --
// the LSP base protocol is small and fully specified, same reasoning as
// why the server itself hand-rolls its JSON-RPC framing rather than
// pulling in a crate for it.
//
// Scope matches the server: diagnostics (via publishDiagnostics) and
// hover. No code actions / auto-apply-fix client-side either, for the
// same reason the server doesn't offer them -- see
// bin/ufa-baremetal-lsp/src/protocol.rs's own doc comment.
//
// Activates independently of any VS Code "language mode" (no `languages`
// contribution in package.json) so it never fights with -- or silently
// overrides -- syntax highlighting some other installed extension may
// already provide for `.s` files. Relevance is decided purely by file
// extension (see `isRelevant` below).

const vscode = require('vscode');
const cp = require('child_process');

let child = null;
let nextId = 1;
const pending = new Map(); // request id -> { resolve, reject }
const openDocs = new Set(); // uris we've successfully sent didOpen for
let diagnosticCollection;
let outputChannel;

function activate(context) {
  outputChannel = vscode.window.createOutputChannel('ufa-baremetal-lsp');
  diagnosticCollection = vscode.languages.createDiagnosticCollection('ufa-baremetal');
  context.subscriptions.push(outputChannel, diagnosticCollection);

  context.subscriptions.push(
    vscode.commands.registerCommand('ufaBaremetal.restartServer', () => {
      outputChannel.appendLine('[restarting by user request]');
      stopServer();
      startServer();
    })
  );

  context.subscriptions.push(
    vscode.workspace.onDidOpenTextDocument(onOpen),
    vscode.workspace.onDidChangeTextDocument(onChange),
    vscode.workspace.onDidSaveTextDocument(onSave),
    vscode.workspace.onDidCloseTextDocument(onClose),
    vscode.languages.registerHoverProvider({ pattern: '**/*.s' }, { provideHover })
  );

  startServer();
}

function deactivate() {
  stopServer();
}

function isRelevant(document) {
  return document.uri.scheme === 'file' && document.fileName.toLowerCase().endsWith('.s');
}

// ---------------------------------------------------------------------
// Process lifecycle
// ---------------------------------------------------------------------

function startServer() {
  const serverPath = vscode.workspace.getConfiguration('ufaBaremetal').get('serverPath') || 'ufa-baremetal-lsp';

  let proc;
  try {
    proc = cp.spawn(serverPath, [], { stdio: ['pipe', 'pipe', 'pipe'] });
  } catch (e) {
    vscode.window.showErrorMessage(`ufa-baremetal-lsp: failed to launch "${serverPath}": ${e.message}`);
    return;
  }
  child = proc;

  // Buffered locally to this process instance (not module-level) so a
  // quick restart can never let a stray chunk from the old process's
  // stdout corrupt the new one's parse state.
  let buf = Buffer.alloc(0);
  proc.stdout.on('data', (chunk) => {
    buf = Buffer.concat([buf, chunk]);
    for (;;) {
      const headerEnd = buf.indexOf('\r\n\r\n');
      if (headerEnd === -1) return;
      const header = buf.slice(0, headerEnd).toString('ascii');
      const match = /Content-Length:\s*(\d+)/i.exec(header);
      if (!match) {
        outputChannel.appendLine('ufa-baremetal-lsp: malformed frame header from server, dropping buffered data');
        buf = Buffer.alloc(0);
        return;
      }
      const length = parseInt(match[1], 10);
      const bodyStart = headerEnd + 4;
      if (buf.length < bodyStart + length) return; // wait for the rest of this frame
      const body = buf.slice(bodyStart, bodyStart + length).toString('utf8');
      buf = buf.slice(bodyStart + length);
      try {
        handleMessage(JSON.parse(body));
      } catch (e) {
        outputChannel.appendLine(`ufa-baremetal-lsp: could not parse message from server: ${e.message}`);
      }
    }
  });

  proc.on('error', (e) => {
    vscode.window.showErrorMessage(
      `ufa-baremetal-lsp: ${e.message}. Set "ufaBaremetal.serverPath" in Settings if the binary isn't on your PATH.`
    );
  });
  proc.stderr.on('data', (d) => outputChannel.append(d.toString()));
  proc.on('exit', (code, signal) => {
    outputChannel.appendLine(`[server exited: code=${code} signal=${signal}]`);
    if (child === proc) {
      child = null;
      openDocs.clear();
      diagnosticCollection.clear();
    }
  });

  sendRequest('initialize', { processId: process.pid, rootUri: null, capabilities: {} })
    .then(() => {
      sendNotification('initialized', {});
      vscode.workspace.textDocuments.forEach(onOpen);
    })
    .catch((e) => outputChannel.appendLine(`ufa-baremetal-lsp: initialize failed: ${e.message}`));
}

function stopServer() {
  const proc = child;
  if (!proc) return;
  // Fire shutdown then exit back-to-back rather than waiting for the
  // shutdown response: the server handles one message at a time on a
  // single thread, so it's guaranteed to see them in order, and the
  // kill() timer below is the actual safety net either way.
  writeToProcess(proc, { jsonrpc: '2.0', id: nextId++, method: 'shutdown', params: null });
  writeToProcess(proc, { jsonrpc: '2.0', method: 'exit', params: null });

  child = null;
  openDocs.clear();
  diagnosticCollection.clear();
  for (const p of pending.values()) p.reject(new Error('server restarting'));
  pending.clear();

  setTimeout(() => {
    if (proc && !proc.killed) proc.kill();
  }, 1000);
}

// ---------------------------------------------------------------------
// JSON-RPC framing (mirrors bin/ufa-baremetal-lsp/src/rpc.rs)
// ---------------------------------------------------------------------

function frameOf(message) {
  const body = Buffer.from(JSON.stringify(message), 'utf8');
  return Buffer.concat([Buffer.from(`Content-Length: ${body.length}\r\n\r\n`, 'ascii'), body]);
}

function writeToProcess(proc, message) {
  if (!proc || !proc.stdin || !proc.stdin.writable) return false;
  proc.stdin.write(frameOf(message));
  return true;
}

function sendNotification(method, params) {
  return writeToProcess(child, { jsonrpc: '2.0', method, params });
}

function sendRequest(method, params) {
  const id = nextId++;
  return new Promise((resolve, reject) => {
    if (!writeToProcess(child, { jsonrpc: '2.0', id, method, params })) {
      reject(new Error('ufa-baremetal-lsp server is not running'));
      return;
    }
    pending.set(id, { resolve, reject });
  });
}

function handleMessage(msg) {
  if (msg.id !== undefined && (msg.result !== undefined || msg.error !== undefined)) {
    const p = pending.get(msg.id);
    if (!p) return; // response to a request we're no longer tracking (e.g. post-restart) -- ignore
    pending.delete(msg.id);
    if (msg.error) p.reject(new Error(msg.error.message));
    else p.resolve(msg.result);
    return;
  }
  if (msg.method === 'textDocument/publishDiagnostics') {
    publishDiagnostics(msg.params);
  }
}

// ---------------------------------------------------------------------
// Diagnostics + hover
// ---------------------------------------------------------------------

const SEVERITY_MAP = {
  1: vscode.DiagnosticSeverity.Error,
  2: vscode.DiagnosticSeverity.Warning,
  3: vscode.DiagnosticSeverity.Information,
  4: vscode.DiagnosticSeverity.Hint,
};

function publishDiagnostics(params) {
  const uri = vscode.Uri.parse(params.uri);
  const diagnostics = (params.diagnostics || []).map((d) => {
    const range = new vscode.Range(d.range.start.line, d.range.start.character, d.range.end.line, d.range.end.character);
    const diag = new vscode.Diagnostic(range, d.message, SEVERITY_MAP[d.severity] || vscode.DiagnosticSeverity.Error);
    diag.source = d.source || 'ufa-baremetal';
    if (d.code) diag.code = d.code;
    return diag;
  });
  diagnosticCollection.set(uri, diagnostics);
}

function provideHover(document, position) {
  if (!isRelevant(document) || !child) return null;
  return sendRequest('textDocument/hover', {
    textDocument: { uri: document.uri.toString() },
    position: { line: position.line, character: position.character },
  })
    .then((result) => {
      if (!result || !result.contents) return null;
      return new vscode.Hover(new vscode.MarkdownString(result.contents.value));
    })
    .catch(() => null); // a hover that silently doesn't appear beats an error popup on every mouse-over
}

// ---------------------------------------------------------------------
// Document lifecycle -> textDocument/didOpen|didChange|didSave|didClose
// ---------------------------------------------------------------------

function onOpen(document) {
  if (!isRelevant(document)) return;
  const uri = document.uri.toString();
  const ok = sendNotification('textDocument/didOpen', {
    textDocument: { uri, languageId: document.languageId, version: document.version, text: document.getText() },
  });
  if (ok) openDocs.add(uri);
}

function onChange(event) {
  const document = event.document;
  if (!isRelevant(document)) return;
  const uri = document.uri.toString();
  if (!openDocs.has(uri)) {
    // Server (re)started after this buffer was already open, or its
    // earlier didOpen silently failed to send -- (re)send full state now
    // rather than only ever sending deltas it never got a base for.
    onOpen(document);
    return;
  }
  sendNotification('textDocument/didChange', {
    textDocument: { uri, version: document.version },
    // Full-document sync only, matching the server's advertised
    // textDocumentSync.change == 1 capability -- see initialize's
    // response and bin/ufa-baremetal-lsp/src/protocol.rs::on_did_change.
    contentChanges: [{ text: document.getText() }],
  });
}

function onSave(document) {
  if (!isRelevant(document)) return;
  sendNotification('textDocument/didSave', { textDocument: { uri: document.uri.toString() } });
}

function onClose(document) {
  if (!isRelevant(document)) return;
  const uri = document.uri.toString();
  diagnosticCollection.delete(document.uri);
  if (openDocs.has(uri)) {
    openDocs.delete(uri);
    sendNotification('textDocument/didClose', { textDocument: { uri } });
  }
}

module.exports = { activate, deactivate };
