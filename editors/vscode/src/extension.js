// Mylis VS Code 扩展：零 npm 依赖的极简 LSP 客户端 + TextMate 语法。
// 语义能力（诊断 / 悬停 / 跳转 / 补全 / 大纲）全部来自 myls 语言服务器。
const vscode = require('vscode');
const cp = require('child_process');
const fs = require('fs');
const path = require('path');

let proc = null;
let nextId = 1;
const pending = new Map();
let diagnostics = null;
let output = null;
let status = null;
let started = false;

function log(msg) {
  if (output) output.appendLine(msg);
}

function exeNames() {
  return process.platform === 'win32' ? ['myls.exe', 'myls'] : ['myls', 'myls.exe'];
}

/** 服务器查找：配置 → 环境变量 → 工作区 ./bin → 工作区上级 ./bin → 扩展 ../../bin → PATH */
function findServer(context) {
  const configured = (vscode.workspace.getConfiguration('mylis').get('serverPath') || '').trim();
  if (configured) return configured;

  const env = (process.env.MYLIS_SERVER || '').trim();
  if (env) return env;

  const dirs = [];
  for (const f of vscode.workspace.workspaceFolders || []) {
    dirs.push(f.uri.fsPath, path.join(f.uri.fsPath, 'bin'), path.join(f.uri.fsPath, '..', 'bin'));
  }
  dirs.push(context.extensionPath, path.join(context.extensionPath, '..', '..'),
           path.join(context.extensionPath, '..', '..', 'bin'));
  for (const d of dirs) {
    for (const n of exeNames()) {
      const p = path.join(d, n);
      if (fs.existsSync(p)) return p;
    }
  }
  return exeNames()[0]; // 交给 PATH
}

function start(context) {
  if (proc) return;
  const serverPath = findServer(context);
  log(`[myls] 启动 ${serverPath}`);
  proc = cp.spawn(serverPath, ['--stdio'], { stdio: ['pipe', 'pipe', 'pipe'] });
  proc.stderr.on('data', (d) => log('[myls] ' + d.toString().trim()));
  proc.on('error', (e) => {
    log('[myls] 无法启动: ' + e.message);
    setStatus('myls: 启动失败');
  });
  proc.on('exit', (code) => {
    log('[myls] 退出 ' + code);
    proc = null;
    started = false;
    setStatus('myls: 已停止');
  });
  proc.stdout.on('data', (chunk) => onData(chunk));
  started = true;

  const folders = vscode.workspace.workspaceFolders || [];
  sendRequest('initialize', {
    processId: process.pid,
    rootUri: folders.length ? folders[0].uri.toString() : null,
    workspaceFolders: folders.map((f) => ({ uri: f.uri.toString(), name: f.name })),
    capabilities: {
      textDocument: {
        synchronization: { dynamicRegistration: false },
        hover: { contentFormat: ['markdown', 'plaintext'] },
        completion: { completionItem: { snippetSupport: false } },
        definition: { linkSupport: false },
        references: {},
        rename: { prepareSupport: true },
        signatureHelp: { signatureInformation: { documentationFormat: ['markdown', 'plaintext'] } },
        documentSymbol: { hierarchicalDocumentSymbolSupport: true }
      },
      workspace: { workspaceFolders: true, symbol: {} }
    }
  }).then(() => {
    sendNotification('initialized', {});
    setStatus('myls: 就绪');
    openAll();
  }).catch((e) => {
    log('[myls] initialize 失败: ' + e.message);
    setStatus('myls: 初始化失败');
  });
}

function setStatus(text) {
  if (status) status.text = '$(symbol-method) ' + text;
}

function openAll() {
  vscode.workspace.textDocuments.forEach((doc) => {
    if (doc.languageId === 'mylis') doOpen(doc);
  });
}

function doOpen(doc) {
  sendNotification('textDocument/didOpen', {
    textDocument: { uri: doc.uri.toString(), languageId: 'mylis', version: doc.version, text: doc.getText() }
  });
}

// ---- JSON-RPC over stdio（Content-Length 分帧） ----
let buffer = Buffer.from([]);
function onData(chunk) {
  buffer = Buffer.concat([buffer, chunk]);
  while (true) {
    const sep = buffer.indexOf('\r\n\r\n');
    if (sep < 0) return;
    const header = buffer.slice(0, sep).toString();
    const m = /Content-Length:\s*(\d+)/i.exec(header);
    if (!m) { buffer = buffer.slice(sep + 4); continue; }
    const len = parseInt(m[1], 10);
    if (buffer.length < sep + 4 + len) return;
    const body = buffer.slice(sep + 4, sep + 4 + len).toString();
    buffer = buffer.slice(sep + 4 + len);
    let msg;
    try { msg = JSON.parse(body); } catch (e) { continue; }
    dispatch(msg);
  }
}

function dispatch(msg) {
  if (msg.id !== undefined && (msg.result !== undefined || msg.error !== undefined)) {
    const waiter = pending.get(msg.id);
    if (waiter) {
      pending.delete(msg.id);
      if (msg.error) waiter.reject(new Error(msg.error.message));
      else waiter.resolve(msg.result);
    }
    return;
  }
  if (msg.method === 'textDocument/publishDiagnostics') publish(msg.params);
}

function writeRaw(obj) {
  if (!proc || !proc.stdin.writable) return;
  const data = Buffer.from(JSON.stringify(obj), 'utf8');
  proc.stdin.write('Content-Length: ' + data.length + '\r\n\r\n');
  proc.stdin.write(data);
}

function sendNotification(method, params) { writeRaw({ jsonrpc: '2.0', method, params }); }

function sendRequest(method, params) {
  const id = nextId++;
  return new Promise((resolve, reject) => {
    if (!proc) { reject(new Error('myls 未运行')); return; }
    pending.set(id, { resolve, reject });
    writeRaw({ jsonrpc: '2.0', id, method, params });
  });
}

function publish(params) {
  if (!diagnostics) diagnostics = vscode.languages.createDiagnosticCollection('mylis');
  const uri = vscode.Uri.parse(params.uri);
  const items = (params.diagnostics || []).map((d) => {
    const r = new vscode.Range(d.range.start.line, d.range.start.character, d.range.end.line, d.range.end.character);
    const sevMap = {
      1: vscode.DiagnosticSeverity.Error,
      2: vscode.DiagnosticSeverity.Warning,
      3: vscode.DiagnosticSeverity.Information,
      4: vscode.DiagnosticSeverity.Hint
    };
    const sev = sevMap[d.severity] || vscode.DiagnosticSeverity.Error;
    const diag = new vscode.Diagnostic(r, d.message, sev);
    diag.source = 'mylis';
    if (d.code) diag.code = d.code;
    return diag;
  });
  diagnostics.set(uri, items);
}

function at(doc, pos) {
  return { textDocument: { uri: doc.uri.toString() }, position: { line: pos.line, character: pos.character } };
}

function toRange(r) {
  return new vscode.Range(r.start.line, r.start.character, r.end.line, r.end.character);
}

function toLocation(loc) {
  if (!loc || !loc.uri || !loc.range) return null;
  return new vscode.Location(vscode.Uri.parse(loc.uri), toRange(loc.range));
}

// LSP SymbolKind 是 1 起始，VS Code SymbolKind 是 0 起始。
function toSymbolKind(kind) {
  return kind > 0 ? kind - 1 : kind;
}

function registerProviders() {
  const hover = vscode.languages.registerHoverProvider('mylis', {
    provideHover(doc, pos) {
      return sendRequest('textDocument/hover', at(doc, pos)).then((res) => {
        if (!res || !res.contents) return null;
        const value = typeof res.contents === 'string' ? res.contents : res.contents.value;
        const range = res.range
          ? new vscode.Range(res.range.start.line, res.range.start.character, res.range.end.line, res.range.end.character)
          : undefined;
        return new vscode.Hover(new vscode.MarkdownString(value), range);
      }).catch(() => null);
    }
  });

  const definition = vscode.languages.registerDefinitionProvider('mylis', {
    provideDefinition(doc, pos) {
      return sendRequest('textDocument/definition', at(doc, pos)).then((res) => {
        const list = Array.isArray(res) ? res : [res];
        return list.map(toLocation).filter(Boolean);
      }).catch(() => null);
    }
  });

  const completion = vscode.languages.registerCompletionItemProvider('mylis', {
    provideCompletionItems(doc, pos) {
      return sendRequest('textDocument/completion', at(doc, pos)).then((res) => {
        const items = (res && res.items) || [];
        return items.map((it) => {
          const c = new vscode.CompletionItem(it.label, it.kind || vscode.CompletionItemKind.Text);
          if (it.detail) c.detail = it.detail;
          return c;
        });
      }).catch(() => []);
    }
  }, '.', ':');

  const symbols = vscode.languages.registerDocumentSymbolProvider('mylis', {
    provideDocumentSymbols(doc) {
      return sendRequest('textDocument/documentSymbol', {
        textDocument: { uri: doc.uri.toString() }
      }).then((res) => (res || []).map((s) => new vscode.DocumentSymbol(
        s.name,
        s.detail || '',
        toSymbolKind(s.kind),
        new vscode.Range(s.range.start.line, s.range.start.character, s.range.end.line, s.range.end.character),
        new vscode.Range(s.selectionRange.start.line, s.selectionRange.start.character,
                         s.selectionRange.end.line, s.selectionRange.end.character)
      ))).catch(() => []);
    }
  });

  const references = vscode.languages.registerReferenceProvider('mylis', {
    provideReferences(doc, pos, context) {
      const params = at(doc, pos);
      params.context = { includeDeclaration: context.includeDeclaration };
      return sendRequest('textDocument/references', params)
        .then((res) => (res || []).map(toLocation).filter(Boolean))
        .catch(() => []);
    }
  });

  const rename = vscode.languages.registerRenameProvider('mylis', {
    prepareRename(doc, pos) {
      return sendRequest('textDocument/prepareRename', at(doc, pos)).then((res) => {
        if (!res || !res.range) return null;
        return { range: toRange(res.range), placeholder: res.placeholder || '' };
      }).catch(() => null);
    },
    provideRenameEdits(doc, pos, newName) {
      return sendRequest('textDocument/rename', Object.assign(at(doc, pos), { newName })).then((res) => {
        if (!res || !res.changes) return null;
        const edit = new vscode.WorkspaceEdit();
        for (const [uri, edits] of Object.entries(res.changes)) {
          for (const e of edits) edit.replace(vscode.Uri.parse(uri), toRange(e.range), e.newText);
        }
        return edit;
      }).catch(() => null);
    }
  });

  const signature = vscode.languages.registerSignatureHelpProvider('mylis', {
    provideSignatureHelp(doc, pos) {
      return sendRequest('textDocument/signatureHelp', at(doc, pos)).then((res) => {
        if (!res || !res.signatures) return null;
        const help = new vscode.SignatureHelp();
        help.signatures = res.signatures.map((s) => {
          const info = new vscode.SignatureInformation(s.label);
          info.parameters = (s.parameters || []).map((p) => new vscode.ParameterInformation(p.label));
          if (s.documentation) info.documentation = s.documentation;
          return info;
        });
        help.activeSignature = res.activeSignature || 0;
        help.activeParameter = res.activeParameter || 0;
        return help;
      }).catch(() => null);
    }
  }, '(', ',');

  const workspaceSymbols = vscode.languages.registerWorkspaceSymbolProvider({
    provideWorkspaceSymbols(query) {
      return sendRequest('workspace/symbol', { query }).then((res) => (res || []).map((s) => {
        const loc = toLocation(s.location);
        if (!loc) return null;
        return new vscode.SymbolInformation(s.name, toSymbolKind(s.kind), s.containerName || '', loc);
      }).filter(Boolean)).catch(() => []);
    }
  });

  return [hover, definition, completion, symbols, references, rename, signature, workspaceSymbols];
}

function activate(context) {
  output = vscode.window.createOutputChannel('Mylis LSP');
  status = vscode.window.createStatusBarItem(vscode.StatusBarAlignment.Left, 0);
  status.command = 'mylis.restartServer';
  status.tooltip = '点击重启 myls 语言服务器';
  status.show();

  context.subscriptions.push(output, status, ...registerProviders());
  start(context);

  context.subscriptions.push(
    vscode.workspace.onDidOpenTextDocument((doc) => { if (doc.languageId === 'mylis') doOpen(doc); }),
    vscode.workspace.onDidChangeTextDocument((e) => {
      if (e.document.languageId !== 'mylis') return;
      sendNotification('textDocument/didChange', {
        textDocument: { uri: e.document.uri.toString(), version: e.document.version },
        contentChanges: [{ text: e.document.getText() }]
      });
    }),
    vscode.workspace.onDidSaveTextDocument((doc) => {
      if (doc.languageId !== 'mylis') return;
      sendNotification('textDocument/didSave', { textDocument: { uri: doc.uri.toString() } });
    }),
    vscode.workspace.onDidCloseTextDocument((doc) => {
      if (doc.languageId !== 'mylis') return;
      sendNotification('textDocument/didClose', { textDocument: { uri: doc.uri.toString() } });
      if (diagnostics) diagnostics.delete(doc.uri);
    }),
    vscode.commands.registerCommand('mylis.restartServer', () => {
      if (proc) { try { proc.kill(); } catch (e) {} proc = null; }
      pending.clear();
      setStatus('myls: 重启中');
      start(context);
    }),
    vscode.commands.registerCommand('mylis.showOutput', () => output.show()),
    new vscode.Disposable(() => {
      if (proc) {
        sendNotification('shutdown', {});
        sendNotification('exit', {});
        proc = null;
      }
    })
  );
}

function deactivate() { if (proc) proc.kill(); }

module.exports = { activate, deactivate };
