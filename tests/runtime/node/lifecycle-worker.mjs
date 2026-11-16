import { runWorker } from '../../../sdk/node/index.mjs';
import { spawn } from 'node:child_process';
import { existsSync, renameSync, writeFileSync } from 'node:fs';
import { rename, writeFile } from 'node:fs/promises';
import { createServer } from 'node:net';
import { dirname, join } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import { fileURLToPath } from 'node:url';
import { parseArgs } from 'node:util';

const { positionals } = parseArgs({ allowPositionals: true });
const configPath = positionals[0];
const directory = dirname(configPath);

async function publish(name, value) {
    const path = join(directory, name);
    await writeFile(path + '.pending', JSON.stringify(value));
    await rename(path + '.pending', path);
}

function expire(name) {
    try { writeFileSync(join(directory, name), 'fixture expiry is failure\n'); } catch {}
    process.exit(72);
}

async function listener(message) {
    const sockets = new Set();
    const server = createServer(socket => {
        sockets.add(socket);
        socket.once('close', () => sockets.delete(socket));
        socket.on('error', () => {});
        socket.end(message);
    });
    await new Promise((resolve, reject) => {
        server.once('error', reject);
        server.listen(0, '127.0.0.1', resolve);
    });
    return {
        port: server.address().port,
        async close() {
            const closed = new Promise((resolve, reject) => server.close(error => error ? reject(error) : resolve()));
            for (const socket of sockets) socket.destroy();
            await closed;
        },
    };
}

if (process.env.MDRUN_NODE_LIFECYCLE_ROLE === 'child') {
    const deadline = setTimeout(() => expire('child.expired'), 6500);
    const server = await listener('child-live\n');
    let stopping = false;
    const stop = async () => {
        if (stopping) return;
        stopping = true;
        await server.close();
        await publish('child.closed.json', { port: server.port });
        clearTimeout(deadline);
        if (process.connected) process.disconnect();
    };
    process.on('message', message => {
        if (message?.kind === 'stop') stop().catch(() => expire('child.cleanup.failed'));
    });
    process.once('disconnect', () => stop().catch(() => expire('child.cleanup.failed')));
    process.send({ kind: 'ready', port: server.port });
} else {
    const deadline = setTimeout(() => expire('worker.expired'), 8000);
    const owned = { timer: null, server: null, child: null, childExit: null };
    let ticks = 0;
    let cleanupFailure = null;
    let stopping = null;

    async function closeOwned() {
        if (owned.timer !== null) {
            clearInterval(owned.timer);
            owned.timer = null;
        }
        if (owned.server !== null) {
            await owned.server.close();
            owned.server = null;
        }
        if (owned.child !== null) {
            const child = owned.child;
            if (child.connected) child.send({ kind: 'stop' }, () => {});
            const fallback = setTimeout(() => {
                cleanupFailure = 'owned_child_stop_timeout';
                child.kill('SIGKILL');
            }, 1000);
            let status;
            try { status = await owned.childExit; } finally { clearTimeout(fallback); }
            owned.child = null;
            if (status.code !== 0 || status.signal !== null) cleanupFailure ??= 'owned_child_exit_failed';
            await publish('child.observed.json', status);
        }
        if (cleanupFailure !== null) throw new Error(cleanupFailure);
    }

    function stopOwned() {
        stopping ??= closeOwned();
        return stopping;
    }

    async function waitGate(name) {
        const until = performance.now() + 2000;
        while (!existsSync(join(directory, name))) {
            if (performance.now() >= until) throw new Error('fixture gate expired');
            await delay(5);
        }
    }

    async function startOwned(kind) {
        stopping = null;
        const state = { kinds: kind === 'clean' ? ['timer', 'listener', 'child'] : [kind] };
        if (state.kinds.includes('timer')) {
            owned.timer = setInterval(() => {
                const path = join(directory, 'timer.progress');
                writeFileSync(path + '.pending', String(++ticks));
                renameSync(path + '.pending', path);
            }, 10);
        }
        if (state.kinds.includes('listener')) {
            owned.server = await listener('listener-live\n');
            state.listenerPort = owned.server.port;
        }
        if (state.kinds.includes('child')) {
            const child = spawn(process.execPath, [fileURLToPath(import.meta.url), configPath], {
                env: { ...process.env, MDRUN_NODE_LIFECYCLE_ROLE: 'child' },
                stdio: ['ignore', 'ignore', 'ignore', 'ipc'],
            });
            owned.child = child;
            owned.childExit = new Promise(resolve => {
                child.once('exit', (code, signal) => resolve({ code, signal }));
                child.once('error', () => resolve({ code: null, signal: null, error: 'spawn_failed' }));
            });
            state.childPort = await new Promise((resolve, reject) => {
                child.once('message', message => {
                    if (message?.kind !== 'ready' || !Number.isInteger(message.port)) reject(new Error('invalid child readiness'));
                    else resolve(message.port);
                });
                child.once('error', reject);
                child.once('exit', () => reject(new Error('child exited before readiness')));
            });
        }
        await publish('owned.ready.json', state);
    }

    try {
        await runWorker({
            configPath,
            classify(argv) { return ['clean', 'timer', 'listener', 'child', 'ignore'].includes(argv[0]) ? true : 'unsupported_command'; },
            async invoke(request) {
                if (request.argv[0] === 'ignore') {
                    await request.stdout.write(Buffer.from('ok'));
                    return 0;
                }
                await startOwned(request.argv[0]);
                await waitGate('invoke.continue');
                if (request.signal.aborted) return 0;
                if (request.argv[0] === 'clean') {
                    await stopOwned();
                    await publish('invocation.closed.json', { clean: true });
                }
                return 0;
            },
            async reset(request) {
                if (owned.timer !== null || owned.server !== null || owned.child !== null) {
                    await publish('reset.failed.json', { actor: request.argv[0] });
                    throw new Error('application-owned work remained live');
                }
            },
        });
    } catch {
        process.exitCode = 70;
    } finally {
        try {
            if (owned.timer !== null || owned.server !== null || owned.child !== null) {
                await publish('cleanup.pending.json', {});
                try { await waitGate('cleanup.continue'); } catch {
                    cleanupFailure = 'cleanup_gate_expired';
                    await publish('cleanup.expired.json', {});
                }
            }
            await publish('cleanup.started.json', {});
            await stopOwned();
            await publish('cleanup.closed.json', { clean: true });
        } catch {
            await publish('cleanup.failed.json', { reason: cleanupFailure ?? 'cleanup_failed' });
            process.exitCode = 71;
        } finally {
            clearTimeout(deadline);
        }
    }
}
