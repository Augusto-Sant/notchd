// The daemon connection: a Unix socket, one JSON object per line
// (docs/architecture.md, "Protocol: core to UI"). Framing, message building
// and the entity store are lib/Protocol.js, the same functions the QML client
// uses. Reconnects with backoff when the daemon goes away; the last store
// stays, so the notch freezes where it was.

import GLib from 'gi://GLib';
import Gio from 'gi://Gio';

import * as Protocol from './lib/Protocol.js';

const RETRY_MIN_MS = 250;
const RETRY_MAX_MS = 5000;

export function socketPath() {
    const explicit = GLib.getenv('NOTCHD_SOCKET');
    if (explicit)
        return explicit;
    const instance = GLib.getenv('NOTCHD_INSTANCE') || 'default';
    const runtime = GLib.getenv('XDG_RUNTIME_DIR') || GLib.get_tmp_dir();
    return GLib.build_filenamev([runtime, 'notchd', `${instance}.sock`]);
}

export class DaemonClient {
    // `hello`: the hello message to open each connection with.
    // `onConnected()`: a welcome arrived. `onStore(store, key)`: an entity
    // changed. `onDisconnected()`: the connection closed.
    constructor({hello, onConnected, onStore, onDisconnected, log}) {
        this._hello = hello;
        this._onConnected = onConnected ?? (() => {});
        this._onStore = onStore ?? (() => {});
        this._onDisconnected = onDisconnected ?? (() => {});
        this._log = log ?? (() => {});
        this.store = Protocol.emptyStore();
        this.ready = false;
        this._attempt = 0;
        this._retryId = 0;
        this._cancellable = null;
        this._conn = null;
        this._out = null;
        this._pending = new Uint8Array(0);
        this._decoder = new TextDecoder();
        this._stopped = true;
        this._loggedUnknown = false;
        this._queue = [];
    }

    start() {
        this._stopped = false;
        this._connect();
    }

    stop() {
        this._stopped = true;
        if (this._retryId) {
            GLib.source_remove(this._retryId);
            this._retryId = 0;
        }
        this._close();
    }

    // Sends one message; while disconnected it is dropped (the UI resends
    // what it reports, screens and the pointer, on the next welcome).
    send(msg) {
        if (!this._out)
            return false;
        const bytes = new TextEncoder().encode(Protocol.encode(msg));
        this._queue.push(bytes);
        if (this._queue.length === 1)
            this._flush();
        return true;
    }

    _flush() {
        if (!this._out || this._queue.length === 0)
            return;
        const bytes = this._queue[0];
        this._out.write_all_async(bytes, GLib.PRIORITY_DEFAULT, this._cancellable, (stream, res) => {
            try {
                stream.write_all_finish(res);
            } catch (e) {
                if (!e.matches?.(Gio.IOErrorEnum, Gio.IOErrorEnum.CANCELLED))
                    this._lost(`write: ${e.message}`);
                return;
            }
            this._queue.shift();
            this._flush();
        });
    }

    _connect() {
        if (this._stopped)
            return;
        this._cancellable = new Gio.Cancellable();
        const client = new Gio.SocketClient();
        const address = Gio.UnixSocketAddress.new(socketPath());
        client.connect_async(address, this._cancellable, (c, res) => {
            let conn;
            try {
                conn = c.connect_finish(res);
            } catch (e) {
                if (!e.matches?.(Gio.IOErrorEnum, Gio.IOErrorEnum.CANCELLED))
                    this._retry();
                return;
            }
            this._conn = conn;
            this._out = conn.get_output_stream();
            this._pending = new Uint8Array(0);
            this._queue = [];
            this._read(conn.get_input_stream());
            this.send(this._hello());
        });
    }

    _read(input) {
        input.read_bytes_async(65536, GLib.PRIORITY_DEFAULT, this._cancellable, (stream, res) => {
            let bytes;
            try {
                bytes = stream.read_bytes_finish(res);
            } catch (e) {
                if (!e.matches?.(Gio.IOErrorEnum, Gio.IOErrorEnum.CANCELLED))
                    this._lost(`read: ${e.message}`);
                return;
            }
            if (bytes.get_size() === 0) {
                this._lost('closed');
                return;
            }
            // GJS's TextDecoder has no `stream` mode, so a UTF-8 character
            // split across reads would break: decode only up to the last
            // newline and keep the bytes after it.
            const chunk = bytes.toArray();
            const joined = new Uint8Array(this._pending.length + chunk.length);
            joined.set(this._pending);
            joined.set(chunk, this._pending.length);
            const cut = joined.lastIndexOf(10) + 1;
            this._pending = joined.slice(cut);
            if (this._pending.length > Protocol.MAX_LINE) {
                this._log('dropped a line over the size limit');
                this._pending = new Uint8Array(0);
            }
            if (cut > 0) {
                const framed = Protocol.frame('', this._decoder.decode(joined.subarray(0, cut)));
                for (const line of framed.lines)
                    this._receive(line);
            }
            if (this._conn)
                this._read(stream);
        });
    }

    _receive(line) {
        const msg = Protocol.parse(line);
        if (!msg) {
            if (!this._loggedUnknown)
                this._log(`ignored a line that is not a message: ${line.slice(0, 120)}`);
            this._loggedUnknown = true;
            return;
        }
        if (Protocol.isRefusal(msg)) {
            this._log(`the daemon refused the handshake: wants proto ${msg.want}`);
            return;
        }
        if (msg.t === 'welcome') {
            this._attempt = 0;
            this.ready = true;
            this._onConnected(msg);
            return;
        }
        const result = Protocol.apply(this.store, msg);
        if (result.applied) {
            this.store = result.store;
            this._onStore(this.store, result.key);
        }
    }

    _lost(why) {
        this._log(`daemon connection lost (${why})`);
        this._close();
        this._onDisconnected();
        this._retry();
    }

    _close() {
        this.ready = false;
        this._cancellable?.cancel();
        this._cancellable = null;
        if (this._conn) {
            try {
                this._conn.close(null);
            } catch (e) {}
        }
        this._conn = null;
        this._out = null;
        this._queue = [];
        // A new connection gets every entity again: forget the revs.
        this.store = {...this.store, revs: {}};
    }

    _retry() {
        if (this._stopped || this._retryId)
            return;
        const delay = Protocol.backoff(this._attempt++, RETRY_MIN_MS, RETRY_MAX_MS);
        this._retryId = GLib.timeout_add(GLib.PRIORITY_DEFAULT, delay, () => {
            this._retryId = 0;
            this._connect();
            return GLib.SOURCE_REMOVE;
        });
    }
}
