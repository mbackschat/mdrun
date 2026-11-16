import assert from 'node:assert/strict';
import { createHash, randomBytes } from 'node:crypto';
import { once } from 'node:events';
import { existsSync } from 'node:fs';
import { mkdtemp, readFile, writeFile, rm } from 'node:fs/promises';
import { createConnection, createServer } from 'node:net';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import test from 'node:test';
import { setTimeout as delay } from 'node:timers/promises';

// The fake host deliberately implements framing independently of the SDK.
class Peer {
    constructor(socket) {
        this.socket = socket;
        this.bytes = Buffer.alloc(0);
        this.frames = [];
        this.waiter = null;
        this.failure = null;
        socket.on('data', bytes => {
            this.bytes = Buffer.concat([this.bytes, bytes]);
            while (this.bytes.length >= 4) {
                const size = this.bytes.readUInt32BE(0);
                assert.ok(size > 0 && size <= 1048576);
                if (this.bytes.length < size + 4) break;
                this.frames.push(JSON.parse(this.bytes.subarray(4, size + 4).toString('utf8')));
                this.bytes = this.bytes.subarray(size + 4);
            }
            this.pump();
        });
        socket.on('error', error => { this.failure = error; this.pump(); });
        socket.on('close', () => { this.failure = new Error('peer closed'); this.pump(); });
    }
    pump() {
        if (!this.waiter) return;
        if (this.frames.length) this.waiter.resolve(this.frames.shift());
        else if (this.failure) this.waiter.reject(this.failure);
    }
    async next() {
        assert.equal(this.waiter, null);
        return await new Promise((resolve, reject) => {
            const timer = setTimeout(() => finish(reject, new Error('frame deadline')), 3000);
            const finish = (callback, value) => {
                clearTimeout(timer);
                this.waiter = null;
                callback(value);
            };
            this.waiter = { resolve: frame => finish(resolve, frame), reject: error => finish(reject, error) };
            this.pump();
        });
    }
    send(frame) {
        const body = Buffer.from(JSON.stringify(frame));
        const size = Buffer.alloc(4);
        size.writeUInt32BE(body.length);
        this.socket.write(Buffer.concat([size, body]));
    }
}

async function host(t, fixture = new URL('./fixture-worker.mjs', import.meta.url)) {
    const directory = await mkdtemp(join(tmpdir(), 'mdrun-node-'));
    t.after(() => rm(directory, { recursive: true, force: true }));
    const server = createServer();
    const peers = [];
    const identity = { protocol: 1, session_id: 'node-test', artifact_id: 'fixture-v1', token: randomBytes(24).toString('hex') };
    const rolePromises = new Map();
    for (const role of ['worker_data', 'worker_control']) {
        let resolve;
        let reject;
        const promise = new Promise((done, fail) => { resolve = done; reject = fail; });
        promise.catch(() => {});
        rolePromises.set(role, { promise, resolve, reject });
    }
    server.on('connection', socket => {
        const peer = new Peer(socket);
        peers.push(peer);
        peer.next().then(hello => {
            assert.deepEqual(hello, { kind: 'hello', ...identity, role: hello.role });
            assert.ok(rolePromises.has(hello.role));
            rolePromises.get(hello.role).resolve(peer);
        }).catch(() => {});
    });
    server.listen(0, '127.0.0.1');
    await once(server, 'listening');
    const configPath = join(directory, 'config.json');
    await writeFile(configPath, JSON.stringify({ ...identity, connect: { host: '127.0.0.1', port: server.address().port } }), { mode: 0o600 });
    const worker = spawn(process.execPath, [fileURLToPath(fixture), configPath], { stdio: ['ignore', 'pipe', 'pipe'] });
    let diagnostics = '';
    worker.stderr.on('data', bytes => { diagnostics += bytes; });
    const exited = new Promise(resolve => worker.once('exit', (code, signal) => resolve({ code, signal })));
    t.after(async () => {
        if (worker.exitCode === null && worker.signalCode === null) worker.kill('SIGKILL');
        await exited;
        for (const peer of peers) peer.socket.destroy();
        await new Promise(resolve => server.close(resolve));
    });
    worker.once('exit', () => {
        for (const role of rolePromises.values()) role.reject(new Error(`worker exited before authentication: ${diagnostics}`));
    });
    const deadline = setTimeout(() => worker.kill('SIGKILL'), 12000);
    t.after(() => clearTimeout(deadline));
    const data = await rolePromises.get('worker_data').promise;
    const control = await rolePromises.get('worker_control').promise;
    const ready = await data.next();
    assert.deepEqual(ready, { kind: 'ready', protocol: 1, session_id: identity.session_id, artifact_id: identity.artifact_id, capabilities: ['pull_stdin', 'sequenced_output', 'release_ack'] });
    return {
        data, control, worker, exited, directory,
        async classify(command, requestId = 'one') {
            data.send({ kind: 'classify', request_id: requestId, argv: [command] });
            return await data.next();
        },
        async invoke(command, requestId = 'one') {
            assert.deepEqual(await this.classify(command, requestId), { kind: 'eligible', request_id: requestId });
            data.send({ kind: 'invoke', request_id: requestId });
            assert.deepEqual(await data.next(), { kind: 'accepted', request_id: requestId });
        },
        async release(requestId = 'one') {
            data.send({ kind: 'release', request_id: requestId });
            assert.deepEqual(await data.next(), { kind: 'released', request_id: requestId });
        },
        async shutdown() {
            control.send({ kind: 'shutdown' });
            assert.deepEqual(await exited, { code: 0, signal: null });
        },
    };
}

test('module exposes worker without import-time process hooks', async () => {
    const before = ['uncaughtException', 'unhandledRejection'].map(name => process.listenerCount(name));
    const sdk = await import('../../../sdk/node/index.mjs');
    assert.equal(typeof sdk.runWorker, 'function');
    assert.deepEqual(['uncaughtException', 'unhandledRejection'].map(name => process.listenerCount(name)), before);
});

test('ignoring input emits no credit and completes before release', async t => {
    const h = await host(t);
    await h.invoke('ignore');
    assert.deepEqual(await h.data.next(), { kind: 'output', request_id: 'one', sequence: 0, stream: 'stdout', base64: 'b2s=' });
    assert.deepEqual(await h.data.next(), { kind: 'complete', request_id: 'one', exit_code: 0 });
    await h.release();
    await h.shutdown();
});

test('streams more than one frame in total with binary bytes and fresh EOF', async t => {
    const h = await host(t);
    const input = randomBytes(2500000);
    const output = createHash('sha256');
    let offset = 0;
    let sequence = 0;
    await h.invoke('echo');
    for (;;) {
        const frame = await h.data.next();
        if (frame.kind === 'stdin_read') {
            assert.ok(frame.max_bytes > 0 && frame.max_bytes <= 65536);
            const bytes = input.subarray(offset, offset + frame.max_bytes);
            offset += bytes.length;
            h.data.send(bytes.length ? { kind: 'stdin', request_id: 'one', base64: bytes.toString('base64') } : { kind: 'stdin_eof', request_id: 'one' });
        } else if (frame.kind === 'output') {
            assert.equal(frame.sequence, sequence++);
            const bytes = Buffer.from(frame.base64, 'base64');
            assert.ok(bytes.length <= 65536);
            output.update(bytes);
        } else {
            assert.deepEqual(frame, { kind: 'complete', request_id: 'one', exit_code: 0 });
            break;
        }
    }
    assert.equal(offset, input.length);
    assert.equal(output.digest('hex'), createHash('sha256').update(input).digest('hex'));
    await h.release();
    await h.invoke('echo', 'two');
    assert.deepEqual(await h.data.next(), { kind: 'stdin_read', request_id: 'two', max_bytes: 32768 });
    h.data.send({ kind: 'stdin_eof', request_id: 'two' });
    assert.deepEqual(await h.data.next(), { kind: 'complete', request_id: 'two', exit_code: 0 });
    await h.release('two');
    await h.shutdown();
});

test('output sequence spans stdout and stderr preserving binary order', async t => {
    const h = await host(t);
    await h.invoke('binary');
    const frames = [await h.data.next(), await h.data.next(), await h.data.next()];
    assert.deepEqual(frames.map(frame => [frame.sequence, frame.stream, [...Buffer.from(frame.base64, 'base64')]]), [[0, 'stdout', [0, 255, 1]], [1, 'stderr', [254, 0]], [2, 'stdout', [2]]]);
    assert.equal((await h.data.next()).kind, 'complete');
    await h.release();
    await h.shutdown();
});

test('ordinary failure can be released and followed by success', async t => {
    const h = await host(t);
    await h.invoke('fail');
    assert.deepEqual(await h.data.next(), { kind: 'complete', request_id: 'one', exit_code: 17 });
    await h.release();
    await h.invoke('ignore', 'two');
    assert.equal((await h.data.next()).kind, 'output');
    assert.equal((await h.data.next()).kind, 'complete');
    await h.release('two');
    await h.shutdown();
});

test('refusal clears classification without invoking application', async t => {
    const h = await host(t);
    assert.deepEqual(await h.classify('refuse'), { kind: 'refused', request_id: 'one', reason: 'unsupported_command' });
    await h.invoke('fail', 'two');
    assert.deepEqual(await h.data.next(), { kind: 'complete', request_id: 'two', exit_code: 17 });
    await h.release('two');
    await h.shutdown();
});

for (const command of ['throw', 'resetfail']) {
    test(`${command} poisons instead of reporting application success`, async t => {
        const h = await host(t);
        await h.invoke(command);
        const fault = await h.data.next();
        assert.equal(fault.kind, 'fault');
        assert.equal(fault.request_id, 'one');
        assert.equal((await h.exited).code, 70);
    });
}

test('late output poisons even when caller catches rejected write', async t => {
    const h = await host(t);
    await h.invoke('late');
    assert.equal((await h.data.next()).kind, 'complete');
    await h.release();
    const fault = await h.data.next();
    assert.equal(fault.kind, 'fault');
    assert.equal(fault.reason, 'late_output');
    assert.equal((await h.exited).code, 70);
});

for (const stream of ['stdout', 'stderr', 'input']) {
    test(`caught late ${stream} after orderly shutdown fails the worker process`, async t => {
        const h = await host(t);
        await h.invoke(`late_shutdown_${stream}`);
        assert.deepEqual(await h.data.next(), { kind: 'complete', request_id: 'one', exit_code: 0 });
        await h.release();
        h.control.send({ kind: 'shutdown' });
        assert.deepEqual(await h.exited, { code: 70, signal: null });
    });
}

test('caught late output after orderly shutdown preserves an existing failure status', async t => {
    const h = await host(t);
    await h.invoke('late_shutdown_nonzero');
    assert.deepEqual(await h.data.next(), { kind: 'complete', request_id: 'one', exit_code: 0 });
    await h.release();
    h.control.send({ kind: 'shutdown' });
    assert.deepEqual(await h.exited, { code: 17, signal: null });
});

for (const command of ['blocked_input', 'blocked_output']) {
    test(`control loss interrupts ${command}`, async t => {
        const h = await host(t);
        await h.invoke(command);
        if (command === 'blocked_input') assert.equal((await h.data.next()).kind, 'stdin_read');
        else h.data.socket.pause();
        h.control.socket.destroy();
        assert.equal((await h.exited).code, 70);
    });
}

test('complete retains reservation until release', async t => {
    const h = await host(t);
    await h.invoke('fail');
    assert.equal((await h.data.next()).kind, 'complete');
    h.data.send({ kind: 'classify', request_id: 'two', argv: ['ignore'] });
    assert.equal((await h.data.next()).kind, 'fault');
    assert.equal((await h.exited).code, 70);
});

test('completion flushes an output operation even when handler omitted await', async t => {
    const h = await host(t);
    await h.invoke('unawaited_output');
    assert.deepEqual(await h.data.next(), { kind: 'output', request_id: 'one', sequence: 0, stream: 'stdout', base64: 'cXVldWVk' });
    assert.equal((await h.data.next()).kind, 'complete');
    await h.release();
    await h.shutdown();
});

for (const command of ['concurrent_output', 'unawaited_input', 'uncaught', 'unhandled']) {
    test(`${command} cannot turn incomplete or escaped work into success`, async t => {
        const h = await host(t);
        await h.invoke(command);
        let frame = await h.data.next();
        while (frame.kind === 'output' || frame.kind === 'stdin_read') frame = await h.data.next();
        assert.equal(frame.kind, 'fault');
        assert.equal((await h.exited).code, 70);
    });
}

test('input without application credit is rejected', async t => {
    const h = await host(t);
    h.data.send({ kind: 'stdin_eof', request_id: 'one' });
    assert.equal((await h.data.next()).kind, 'fault');
    assert.equal((await h.exited).code, 70);
});

test('input cannot exceed the application credit', async t => {
    const h = await host(t);
    await h.invoke('blocked_input');
    assert.equal((await h.data.next()).kind, 'stdin_read');
    h.data.send({ kind: 'stdin', request_id: 'one', base64: 'YWI=' });
    assert.equal((await h.data.next()).kind, 'fault');
    assert.equal((await h.exited).code, 70);
});

for (const bytes of [Buffer.from([0, 16, 0, 1]), Buffer.from([0, 0, 0, 1, 0xff]), Buffer.from([0, 0, 0, 1, 0x7b])]) {
    test(`rejects invalid framing ${bytes.toString('hex')}`, async t => {
        const h = await host(t);
        h.data.socket.write(bytes);
        assert.equal((await h.data.next()).kind, 'fault');
        assert.equal((await h.exited).code, 70);
    });
}

for (const frame of [
    { kind: 'invoke', request_id: 'one' },
    { kind: 'classify', request_id: 'one', argv: [], extra: 1 },
    { kind: 'unknown', request_id: 'one' },
]) {
    test(`rejects illegal frame ${JSON.stringify(frame)}`, async t => {
        const h = await host(t);
        h.data.send(frame);
        assert.equal((await h.data.next()).kind, 'fault');
        assert.equal((await h.exited).code, 70);
    });
}

for (const base64 of ['YQ', 'YR==', '', 'YQ===']) {
    test(`rejects noncanonical input ${JSON.stringify(base64)}`, async t => {
        const h = await host(t);
        await h.invoke('blocked_input');
        assert.equal((await h.data.next()).kind, 'stdin_read');
        h.data.send({ kind: 'stdin', request_id: 'one', base64 });
        assert.equal((await h.data.next()).kind, 'fault');
        assert.equal((await h.exited).code, 70);
    });
}

async function lifecycleFile(h, name) {
    const deadline = performance.now() + 1000;
    for (;;) {
        try { return JSON.parse(await readFile(join(h.directory, name), 'utf8')); } catch (error) {
            if (error.code !== 'ENOENT') throw error;
            assert.ok(performance.now() < deadline, `missing lifecycle observation ${name}`);
            await delay(5);
        }
    }
}

async function resourceReply(port) {
    return await new Promise((resolve, reject) => {
        const socket = createConnection({ host: '127.0.0.1', port });
        let bytes = '';
        socket.setTimeout(500, () => socket.destroy(new Error('resource observation deadline')));
        socket.on('data', chunk => {
            bytes += chunk.toString('utf8');
            if (bytes.length > 64) socket.destroy(new Error('unexpected resource response'));
        });
        socket.on('end', () => { socket.destroy(); resolve(bytes); });
        socket.on('error', error => {
            socket.destroy();
            if (error.code === 'ECONNREFUSED') resolve(null);
            else reject(error);
        });
    });
}

async function ownedResourcesLive(h, state) {
    if (state.kinds.includes('timer')) {
        const first = await lifecycleFile(h, 'timer.progress');
        assert.ok(Number.isSafeInteger(first) && first > 0);
        const deadline = performance.now() + 500;
        while (await lifecycleFile(h, 'timer.progress') <= first) {
            assert.ok(performance.now() < deadline, 'owned timer did not advance');
            await delay(5);
        }
    }
    if (state.kinds.includes('listener')) assert.equal(await resourceReply(state.listenerPort), 'listener-live\n');
    if (state.kinds.includes('child')) assert.equal(await resourceReply(state.childPort), 'child-live\n');
}

async function finishLifecycle(h, state) {
    await writeFile(join(h.directory, 'invoke.continue'), '');
    await writeFile(join(h.directory, 'cleanup.continue'), '');
    h.data.socket.destroy();
    h.control.socket.destroy();
    let forced = false;
    const deadline = setTimeout(() => { forced = true; h.worker.kill('SIGKILL'); }, 1500);
    let status;
    try { status = await h.exited; } finally { clearTimeout(deadline); }
    const until = performance.now() + 1000;
    for (const port of [state?.listenerPort, state?.childPort].filter(value => value !== undefined)) {
        while (await resourceReply(port) !== null) {
            assert.ok(performance.now() < until, 'owned resource remained live after worker exit');
            await delay(5);
        }
    }
    assert.equal(forced, false, 'owned worker required fixture teardown escalation');
    assert.deepEqual(await lifecycleFile(h, 'cleanup.closed.json'), { clean: true });
    for (const name of ['worker.expired', 'child.expired', 'child.cleanup.failed', 'cleanup.expired.json', 'cleanup.failed.json']) {
        assert.equal(existsSync(join(h.directory, name)), false, name);
    }
    if (state?.kinds.includes('child')) {
        assert.deepEqual(await lifecycleFile(h, 'child.observed.json'), { code: 0, signal: null });
        assert.deepEqual(await lifecycleFile(h, 'child.closed.json'), { port: state.childPort });
    }
    return status;
}

for (const kind of ['clean', 'timer', 'listener', 'child']) {
    test(`application-owned ${kind} resources are attested before bootstrap cleanup`, async t => {
        const h = await host(t, new URL('./lifecycle-worker.mjs', import.meta.url));
        let state;
        try {
            await h.invoke(kind);
            state = await lifecycleFile(h, 'owned.ready.json');
            assert.deepEqual(state.kinds, kind === 'clean' ? ['timer', 'listener', 'child'] : [kind]);
            await ownedResourcesLive(h, state);
            await writeFile(join(h.directory, 'invoke.continue'), '');
            if (kind === 'clean') {
                assert.deepEqual(await h.data.next(), { kind: 'complete', request_id: 'one', exit_code: 0 });
                assert.deepEqual(await lifecycleFile(h, 'invocation.closed.json'), { clean: true });
                await h.release();
                await h.invoke('ignore', 'two');
                assert.deepEqual(await h.data.next(), { kind: 'output', request_id: 'two', sequence: 0, stream: 'stdout', base64: 'b2s=' });
                assert.deepEqual(await h.data.next(), { kind: 'complete', request_id: 'two', exit_code: 0 });
                await h.release('two');
                await h.shutdown();
            } else {
                assert.deepEqual(await h.data.next(), { kind: 'fault', request_id: 'one', reason: 'reset_failed' });
                assert.deepEqual(await lifecycleFile(h, 'reset.failed.json'), { actor: kind });
                await lifecycleFile(h, 'cleanup.pending.json');
                assert.equal(existsSync(join(h.directory, 'cleanup.started.json')), false);
                assert.equal(h.worker.exitCode, null);
                assert.equal(h.worker.signalCode, null);
                await ownedResourcesLive(h, state);
                assert.ok(!h.data.frames.some(frame => frame.kind === 'complete'));
            }
        } finally {
            assert.deepEqual(await finishLifecycle(h, state), { code: kind === 'clean' ? 0 : 70, signal: null });
        }
    });
}
