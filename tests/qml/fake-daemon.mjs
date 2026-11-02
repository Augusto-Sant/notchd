// A fake notchd daemon for the QML client tests (tests/qml/client.qml): no
// Rust. `node fake-daemon.mjs <socket-path>` listens on that Unix socket and
// answers `hello` with the `welcome` fixture of tests/protocol/to-ui/, then
// sends the `config`, `schema`, `state`, `module` and `view` fixtures. Every
// request (`set`, `edit`, `ipc`) gets an `ok` (an `ipc` of method "fail" gets
// an `err`). Input messages are accepted.
//
// The test scripts it over a control socket, <socket-path>.ctl, one JSON
// object per line:
//   -> {"cmd":"send","msg":{...}}          send a message to every UI
//   -> {"cmd":"raw","chunks":["..",".."],"gapMs":20}   send raw text in pieces
//   -> {"cmd":"drop"}                      close every UI connection
//   -> {"cmd":"restart","downMs":300}      close everything, listen again later
//   -> {"cmd":"refuse","on":true}          answer hello with err proto (want 1)
//   -> {"cmd":"proto","value":2}           the proto this daemon speaks
//   -> {"cmd":"reset"}                    refuse off, proto 1, close every UI connection
//   -> {"cmd":"quit"}
//   <- {"ev":"listening"} / {"ev":"connect","n":1} / {"ev":"close","n":1}
//   <- {"rx":{...},"n":1}                  every message a UI sent, n = connection
// A control client that connects gets {"ev":"ready"} first.
import fs from 'node:fs';
import net from 'node:net';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const sockPath = process.argv[2];
if (!sockPath) { console.error('usage: fake-daemon.mjs <socket-path>'); process.exit(2); }
const ctlPath = sockPath + '.ctl';
const fixtures = path.join(path.dirname(fileURLToPath(import.meta.url)), '..', 'protocol', 'to-ui');
const fixture = (name, i = 0) => JSON.parse(fs.readFileSync(path.join(fixtures, name + '.json'), 'utf8')).valid[i];

let refuse = false;
let proto = 1;
let connections = 0;
const uis = new Set();
const controls = new Set();
let server = null;

const line = (msg) => JSON.stringify(msg) + '\n';
const tell = (msg) => { for (const c of controls) c.write(line(msg)); };

function greet(socket) {
  const welcome = fixture('welcome', 1);
  for (const msg of [welcome, fixture('config'), fixture('schema'), fixture('state'), fixture('module'),
                     fixture('view', 0), fixture('view', 1)]) socket.write(line(msg));
}

function onUi(socket) {
  const n = ++connections;
  uis.add(socket);
  tell({ ev: 'connect', n });
  let buffer = '';
  socket.on('data', (chunk) => {
    buffer += chunk.toString('utf8');
    let at;
    while ((at = buffer.indexOf('\n')) >= 0) {
      const text = buffer.slice(0, at);
      buffer = buffer.slice(at + 1);
      let msg;
      try { msg = JSON.parse(text); } catch { continue; }
      tell({ rx: msg, n });
      if (msg.t === 'hello') {
        if (refuse || msg.proto !== proto) { socket.write(line({ t: 'err', code: 'proto', want: proto })); socket.end(); }
        else greet(socket);
      } else if (typeof msg.id === 'number') {
        if (msg.t === 'ipc' && msg.method === 'fail') socket.write(line({ t: 'err', id: msg.id, code: 'module', message: 'no such module' }));
        else if (msg.t === 'ipc') socket.write(line({ t: 'ok', id: msg.id, result: msg.method + ':' + msg.args.join(',') }));
        else socket.write(line({ t: 'ok', id: msg.id }));
      }
    }
  });
  socket.on('error', () => {});
  socket.on('close', () => { uis.delete(socket); tell({ ev: 'close', n }); });
}

function listen() {
  fs.rmSync(sockPath, { force: true });
  server = net.createServer(onUi);
  server.listen(sockPath, () => tell({ ev: 'listening' }));
}

function dropAll() { for (const s of uis) s.destroy(); }

function stopServer(then) {
  const s = server;
  server = null;
  dropAll();
  if (!s) { then(); return; }
  s.close(() => { fs.rmSync(sockPath, { force: true }); then(); });
}

function control(cmd) {
  switch (cmd.cmd) {
    case 'send': for (const s of uis) s.write(line(cmd.msg)); break;
    case 'raw': {
      let delay = 0;
      for (const chunk of cmd.chunks) {
        setTimeout(() => { for (const s of uis) s.write(chunk); }, delay);
        delay += cmd.gapMs ?? 20;
      }
      break;
    }
    case 'drop': dropAll(); break;
    case 'reset': refuse = false; proto = 1; dropAll(); break;
    case 'restart': stopServer(() => setTimeout(listen, cmd.downMs ?? 300)); break;
    case 'refuse': refuse = cmd.on !== false; break;
    case 'proto': proto = cmd.value; break;
    case 'quit': stopServer(() => { for (const c of controls) c.destroy(); ctl.close(() => process.exit(0)); }); break;
  }
}

fs.rmSync(ctlPath, { force: true });
const ctl = net.createServer((socket) => {
  controls.add(socket);
  socket.write(line({ ev: 'ready' }));
  let buffer = '';
  socket.on('data', (chunk) => {
    buffer += chunk.toString('utf8');
    let at;
    while ((at = buffer.indexOf('\n')) >= 0) {
      const text = buffer.slice(0, at);
      buffer = buffer.slice(at + 1);
      try { control(JSON.parse(text)); } catch (e) { console.error('fake-daemon: bad control line', text); }
    }
  });
  socket.on('error', () => {});
  socket.on('close', () => controls.delete(socket));
});
ctl.listen(ctlPath, listen);

// A test that dies must not leave this behind.
setTimeout(() => process.exit(0), 120000).unref();
process.on('SIGTERM', () => process.exit(0));
