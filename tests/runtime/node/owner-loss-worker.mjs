import { spawn } from 'node:child_process';
import { once } from 'node:events';
import { existsSync, readFileSync, renameSync, writeFileSync } from 'node:fs';
import { createServer } from 'node:net';
import { join } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import { fileURLToPath } from 'node:url';
import { runWorker } from '../../../sdk/node/index.mjs';

const directory = process.env.MDRUN_OWNER_LOSS_DIRECTORY;
const fixture = fileURLToPath(import.meta.url);

function publish(name, value) {
    const path = join(directory, name);
    writeFileSync(path + '.tmp', JSON.stringify(value));
    renameSync(path + '.tmp', path);
}

function markExpired(name) {
    try {
        writeFileSync(join(directory, name), String(process.pid));
    } catch {
        // Run::drop can remove scratch files after failure; expiry must still terminate the fixture.
    }
}

if (process.env.MDRUN_OWNER_LOSS_ROLE === 'descendant') {
    process.on('SIGTERM', () => {});
    publish('descendant.ready.json', { pid: process.pid });
    const stop = () => process.exit(0);
    setInterval(() => {
        if (existsSync(join(directory, 'finished'))) stop();
    }, 10);
    setTimeout(() => {
        markExpired('descendant.expired');
        stop();
    }, 15000);
} else {
    let descendant;
    setTimeout(() => {
        markExpired('owner.expired');
        descendant?.kill('SIGKILL');
        process.exit(71);
    }, 15000).unref();

    async function startDescendant() {
        descendant = spawn(process.execPath, [fixture], {
            stdio: 'ignore',
            env: { ...process.env, MDRUN_OWNER_LOSS_ROLE: 'descendant' },
        });
        await once(descendant, 'spawn');
        descendant.unref();
        const deadline = Date.now() + 3000;
        while (!existsSync(join(directory, 'descendant.ready.json'))) {
            if (Date.now() >= deadline) throw new Error('descendant readiness expired');
            await delay(5);
        }
        const ready = JSON.parse(readFileSync(join(directory, 'descendant.ready.json'), 'utf8'));
        if (ready.pid !== descendant.pid) throw new Error('descendant identity mismatch');
        publish('ready.json', { worker: process.pid, descendant: descendant.pid });
    }

    if (process.env.MDRUN_OWNER_LOSS_ROLE === 'direct_owner') {
        const address = JSON.parse(process.env.MDRUN_OWNER_LOSS_ADDRESS);
        const listener = createServer();
        listener.listen(address.port, address.host);
        await once(listener, 'listening');
        await startDescendant();
    } else {
        await runWorker({
            configPath: process.argv[2],
            classify(argv) {
                return argv.length === 1 && argv[0] === 'hold' ? true : 'unsupported_command';
            },
            async invoke(request) {
                await startDescendant();
                await new Promise((resolve, reject) => {
                    const trigger = setInterval(() => {
                        if (existsSync(join(directory, 'lose-worker'))) process.kill(process.pid, 'SIGKILL');
                    }, 5);
                    const abort = () => {
                        clearInterval(trigger);
                        reject(new Error('owner lost during accepted invocation'));
                    };
                    if (request.signal.aborted) abort();
                    else request.signal.addEventListener('abort', abort, { once: true });
                });
                return 0;
            },
            reset() {
                throw new Error('active descendant forbids reset');
            },
        }).catch(() => {
            process.exitCode = 70;
        });
    }
}
