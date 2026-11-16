import { readFile } from 'node:fs/promises';
import { createConnection } from 'node:net';

const FRAME_LIMIT = 1048576;
const CHUNK_LIMIT = 65536;
const UTF8 = new TextDecoder('utf-8', { fatal: true });
let workerActive = false;

function requireValue(condition, reason = 'invalid_frame') {
    if (!condition) throw new Error(reason);
}

function objectFields(value, fields, reason = 'invalid_frame') {
    requireValue(value !== null && typeof value === 'object' && !Array.isArray(value), reason);
    requireValue(Object.keys(value).length === fields.length && fields.every(field => Object.hasOwn(value, field)), reason);
}

function nonempty(value) {
    return typeof value === 'string' && value.length > 0;
}

function encode(frame) {
    const body = Buffer.from(JSON.stringify(frame));
    requireValue(body.length > 0 && body.length <= FRAME_LIMIT, 'frame_limit');
    const prefix = Buffer.alloc(4);
    prefix.writeUInt32BE(body.length);
    return Buffer.concat([prefix, body]);
}

class Channel {
    constructor(socket) {
        this.socket = socket;
        this.iterator = socket[Symbol.asyncIterator]();
        this.remaining = Buffer.alloc(0);
        socket.on('error', () => {});
    }

    async readBytes(count) {
        const bytes = Buffer.allocUnsafe(count);
        let offset = 0;
        while (offset < count) {
            if (this.remaining.length === 0) {
                const next = await this.iterator.next();
                requireValue(!next.done, 'connection_lost');
                this.remaining = next.value;
            }
            const length = Math.min(count - offset, this.remaining.length);
            this.remaining.copy(bytes, offset, 0, length);
            this.remaining = this.remaining.subarray(length);
            offset += length;
        }
        return bytes;
    }

    async read() {
        const size = (await this.readBytes(4)).readUInt32BE();
        requireValue(size > 0 && size <= FRAME_LIMIT, 'frame_limit');
        let frame;
        try {
            frame = JSON.parse(UTF8.decode(await this.readBytes(size)));
        } catch {
            throw new Error('invalid_json');
        }
        requireValue(frame !== null && typeof frame === 'object' && !Array.isArray(frame) && typeof frame.kind === 'string');
        return frame;
    }

    async send(frame) {
        const bytes = encode(frame);
        await new Promise((resolve, reject) => this.socket.write(bytes, error => error ? reject(new Error('connection_lost')) : resolve()));
    }

    async fault(frame) {
        if (this.socket.destroyed) return;
        await new Promise(resolve => {
            const done = () => {
                clearTimeout(timer);
                this.socket.off('close', done);
                this.socket.destroy();
                resolve();
            };
            const timer = setTimeout(done, 50);
            this.socket.once('close', done);
            this.socket.end(encode(frame), done);
        });
    }
}

async function connect(address) {
    const socket = createConnection({ host: address.host, port: address.port });
    await new Promise((resolve, reject) => {
        const error = () => {
            socket.destroy();
            reject(new Error('connection_lost'));
        };
        socket.once('error', error);
        socket.once('connect', () => {
            socket.off('error', error);
            resolve();
        });
    });
    return new Channel(socket);
}

function inputBytes(base64, limit) {
    requireValue(typeof base64 === 'string' && base64.length > 0 && base64.length <= 87384, 'invalid_stdin');
    const bytes = Buffer.from(base64, 'base64');
    requireValue(bytes.length > 0 && bytes.length <= limit && bytes.toString('base64') === base64, 'invalid_stdin');
    return bytes;
}

/** Run one cooperative, sequential worker; reset must attest application resource cleanup before reuse. */
export async function runWorker({ configPath, classify, invoke, reset }) {
    requireValue(!workerActive, 'worker_already_active');
    requireValue(typeof configPath === 'string' && [classify, invoke, reset].every(value => typeof value === 'function'), 'invalid_adapter');
    workerActive = true;
    let data;
    let control;
    let current = null;
    let terminated = false;
    let resolveTerminal;
    let rejectTerminal;
    const terminal = new Promise((resolve, reject) => { resolveTerminal = resolve; rejectTerminal = reject; });
    // Startup failures can occur before the main await attaches its rejection handler.
    terminal.catch(() => {});

    const poison = reason => {
        if (terminated) return;
        terminated = true;
        const error = new Error(reason);
        if (current) {
            current.phase = 'faulted';
            current.controller.abort(error);
            current.pendingRead?.reject(error);
            current.pendingRead = null;
        }
        control?.socket.destroy();
        const frame = { kind: 'fault', request_id: current?.requestId ?? null, reason };
        const flushed = data ? data.fault(frame) : Promise.resolve();
        flushed.catch(() => {}).finally(() => rejectTerminal(error));
    };
    const uncaught = () => poison('uncaught_exception');
    const unhandled = () => poison('unhandled_rejection');

    function fail(reason) {
        if (terminated && (!process.exitCode || process.exitCode === '0')) process.exitCode = 70;
        poison(reason);
        return Promise.reject(new Error(reason));
    }

    function makeInvocation(requestId, argv) {
        const context = {
            requestId, argv, phase: 'eligible', controller: new AbortController(),
            eof: false, pendingRead: null, pendingOutput: null, sequence: 0,
        };
        const write = (stream, bytes) => {
            if (context.phase !== 'running' || terminated) return fail('late_output');
            if (!(bytes instanceof Uint8Array)) return fail('invalid_output');
            if (context.pendingOutput) return fail('concurrent_output');
            const operation = (async () => {
                for (let offset = 0; offset < bytes.length; offset += CHUNK_LIMIT) {
                    requireValue(!terminated, 'worker_stopped');
                    requireValue(Number.isSafeInteger(context.sequence), 'sequence_limit');
                    const chunk = bytes.subarray(offset, offset + CHUNK_LIMIT);
                    await data.send({ kind: 'output', request_id: requestId, sequence: context.sequence++, stream, base64: Buffer.from(chunk).toString('base64') });
                }
            })();
            context.pendingOutput = operation;
            operation.then(() => {
                if (context.pendingOutput === operation) context.pendingOutput = null;
            }, () => poison('output_failed'));
            return operation;
        };
        const read = maxBytes => {
            if (context.phase !== 'running' || terminated) return fail('late_input');
            if (!Number.isInteger(maxBytes) || maxBytes < 1 || maxBytes > CHUNK_LIMIT) return fail('invalid_input_credit');
            if (context.pendingRead) return fail('concurrent_input');
            if (context.eof) return Promise.resolve(null);
            return new Promise((resolve, reject) => {
                context.pendingRead = { maxBytes, resolve, reject };
                data.send({ kind: 'stdin_read', request_id: requestId, max_bytes: maxBytes }).catch(() => poison('input_failed'));
            });
        };
        context.request = Object.freeze({
            requestId, argv, signal: context.controller.signal,
            stdin: Object.freeze({ read }),
            stdout: Object.freeze({ write: bytes => write('stdout', bytes) }),
            stderr: Object.freeze({ write: bytes => write('stderr', bytes) }),
        });
        return context;
    }

    async function execute(context) {
        try {
            const exitCode = await invoke(context.request);
            if (terminated) return;
            context.phase = 'resetting';
            requireValue(Number.isInteger(exitCode) && exitCode >= 0 && exitCode <= 255, 'invalid_exit_code');
            requireValue(context.pendingRead === null, 'outstanding_input');
            await context.pendingOutput;
            await reset(context.request);
            if (terminated) return;
            context.phase = 'complete';
            await data.send({ kind: 'complete', request_id: context.requestId, exit_code: exitCode });
        } catch (error) {
            poison(context.phase === 'resetting' ? 'reset_failed' : 'invocation_failed');
        }
    }

    async function receive(frame) {
        switch (frame.kind) {
            case 'classify': {
                objectFields(frame, ['kind', 'request_id', 'argv']);
                requireValue(current === null && nonempty(frame.request_id) && Array.isArray(frame.argv) && frame.argv.every(value => typeof value === 'string'), 'invalid_classification');
                const argv = Object.freeze([...frame.argv]);
                current = makeInvocation(frame.request_id, argv);
                current.phase = 'classifying';
                const result = await classify(argv);
                if (terminated) return;
                if (result === true) {
                    current.phase = 'eligible';
                    await data.send({ kind: 'eligible', request_id: frame.request_id });
                } else {
                    requireValue(nonempty(result), 'invalid_classification');
                    current = null;
                    await data.send({ kind: 'refused', request_id: frame.request_id, reason: result });
                }
                return;
            }
            case 'invoke':
                objectFields(frame, ['kind', 'request_id']);
                requireValue(current?.phase === 'eligible' && frame.request_id === current.requestId, 'invalid_invocation');
                current.phase = 'running';
                await data.send({ kind: 'accepted', request_id: current.requestId });
                if (!terminated) void execute(current);
                return;
            case 'stdin':
            case 'stdin_eof': {
                objectFields(frame, frame.kind === 'stdin' ? ['kind', 'request_id', 'base64'] : ['kind', 'request_id']);
                requireValue(current?.phase === 'running' && current.pendingRead !== null && frame.request_id === current.requestId, 'unsolicited_stdin');
                const bytes = frame.kind === 'stdin' ? inputBytes(frame.base64, current.pendingRead.maxBytes) : null;
                const pending = current.pendingRead;
                current.pendingRead = null;
                current.eof = bytes === null;
                pending.resolve(bytes);
                return;
            }
            case 'release': {
                objectFields(frame, ['kind', 'request_id']);
                requireValue(current?.phase === 'complete' && frame.request_id === current.requestId, 'invalid_release');
                const released = current;
                released.phase = 'released';
                current = null;
                await data.send({ kind: 'released', request_id: released.requestId });
                return;
            }
            default:
                throw new Error('unknown_frame');
        }
    }

    try {
        let config;
        try { config = JSON.parse(await readFile(configPath, 'utf8')); } catch { throw new Error('invalid_config'); }
        objectFields(config, ['protocol', 'session_id', 'artifact_id', 'token', 'connect'], 'invalid_config');
        objectFields(config.connect, ['host', 'port'], 'invalid_config');
        requireValue(config.protocol === 1 && [config.session_id, config.artifact_id, config.token].every(nonempty)
            && config.connect.host === '127.0.0.1' && Number.isInteger(config.connect.port) && config.connect.port >= 1 && config.connect.port <= 65535, 'invalid_config');
        process.on('uncaughtException', uncaught);
        process.on('unhandledRejection', unhandled);
        data = await connect(config.connect);
        await data.send({ kind: 'hello', protocol: 1, session_id: config.session_id, artifact_id: config.artifact_id, token: config.token, role: 'worker_data' });
        control = await connect(config.connect);
        await control.send({ kind: 'hello', protocol: 1, session_id: config.session_id, artifact_id: config.artifact_id, token: config.token, role: 'worker_control' });
        const controlLoop = async () => {
            try {
                const frame = await control.read();
                objectFields(frame, ['kind']);
                requireValue(frame.kind === 'shutdown', 'invalid_control');
                requireValue(current === null, 'shutdown_during_invocation');
                if (!terminated) {
                    terminated = true;
                    data.socket.destroy();
                    control.socket.destroy();
                    resolveTerminal();
                }
            } catch (error) {
                poison(error.message === 'connection_lost' ? 'owner_lost' : 'invalid_control');
            }
        };
        void controlLoop();
        await data.send({ kind: 'ready', protocol: 1, session_id: config.session_id, artifact_id: config.artifact_id, capabilities: ['pull_stdin', 'sequenced_output', 'release_ack'] });
        const dataLoop = async () => {
            try {
                while (!terminated) await receive(await data.read());
            } catch (error) {
                poison(error.message === 'connection_lost' ? 'data_lost' : 'protocol_error');
            }
        };
        void dataLoop();
        await terminal;
    } finally {
        terminated = true;
        data?.socket.destroy();
        control?.socket.destroy();
        process.off('uncaughtException', uncaught);
        process.off('unhandledRejection', unhandled);
        workerActive = false;
    }
}
