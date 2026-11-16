import { runWorker } from '../../../sdk/node/index.mjs';
import { existsSync, readFileSync, writeFileSync } from 'node:fs';
import { rename, writeFile } from 'node:fs/promises';
import { createServer } from 'node:net';
import { dirname, join } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import { parseArgs } from 'node:util';

const { positionals } = parseArgs({ allowPositionals: true });
if (positionals.length !== 1) throw new Error('one SDK configuration path is required');
const [configPath] = positionals;
const { session_id: sessionId } = JSON.parse(readFileSync(configPath, 'utf8'));
const directory = process.cwd();
const calls = [];
const sockets = new Set();
let id = null;
let ticks = 0;
let stopping = false;

async function publish(name, value) {
    const path = join(directory, name);
    await writeFile(path + '.pending', JSON.stringify(value));
    await rename(path + '.pending', path);
}

async function waitGate(name) {
    const deadline = performance.now() + 12000;
    while (!existsSync(join(directory, name))) {
        if (performance.now() >= deadline) throw new Error(`fixture gate expired: ${name}`);
        await delay(5);
    }
}

const expiry = setTimeout(() => {
    try { writeFileSync(join(directory, `${sessionId}.expired`), 'fixture expiry is failure\n'); } catch {}
    process.exit(72);
}, 18000);
const heartbeat = setInterval(() => ticks++, 10);
const server = createServer(socket => {
    sockets.add(socket);
    socket.once('close', () => sockets.delete(socket));
    socket.on('error', () => {});
    socket.end(JSON.stringify({ session_id: sessionId, ticks, stopping }) + '\n');
});
await new Promise((resolve, reject) => {
    server.once('error', reject);
    server.listen(0, '127.0.0.1', resolve);
});
const state = {
    session_id: sessionId,
    pid: process.pid,
    port: server.address().port,
    private_directory: dirname(configPath),
};

try {
    await publish(`${sessionId}.worker.json`, state);
    await runWorker({
        configPath,
        classify(argv) {
            return argv.length === 2 && ['a', 'b', 'c'].includes(argv[0])
                && ['first', 'second'].includes(argv[1]) ? true : 'unsupported_fixture_call';
        },
        async invoke(request) {
            const [document, phase] = request.argv;
            id ??= document;
            if (document !== id || phase !== ['first', 'second'][calls.length]) {
                throw new Error('unexpected document invocation sequence');
            }
            calls.push(phase);
            await publish(`${id}.${phase}.json`, state);
            if (phase === 'second') await waitGate(`${id}.invoke-go`);
            await request.stdout.write(Buffer.from(`${id}:${phase}\n`));
            return 0;
        },
        async reset() {},
    });
    if (calls.length !== 2) throw new Error('worker stopped without both document calls');
    stopping = true;
    await publish(`${id}.shutdown.json`, { ...state, calls });
    await waitGate(`${id}.close-go`);
} catch (error) {
    process.exitCode = 70;
    await publish(`${sessionId}.failed`, { error: error.message });
} finally {
    clearInterval(heartbeat);
    await new Promise((resolve, reject) => {
        server.close(error => error ? reject(error) : resolve());
        for (const socket of sockets) socket.destroy();
    });
    await publish(`${id ?? sessionId}.closed.json`, state);
    clearTimeout(expiry);
}
