import { runWorker } from '../../../sdk/node/index.mjs';
import { rename, writeFile } from 'node:fs/promises';

let afterShutdown;

await runWorker({
    configPath: process.argv[2],
    classify(argv) {
        return argv[0] === 'refuse' ? 'unsupported_command' : true;
    },
    async invoke(request) {
        switch (request.argv[0]) {
            case 'ignore':
                await request.stdout.write(Buffer.from('ok'));
                return 0;
            case 'echo':
                for (;;) {
                    const bytes = await request.stdin.read(32768);
                    if (bytes === null) break;
                    await request.stdout.write(bytes);
                }
                return 0;
            case 'binary':
                await request.stdout.write(Buffer.from([0, 255, 1]));
                await request.stderr.write(Buffer.from([254, 0]));
                await request.stdout.write(Buffer.from([2]));
                return 0;
            case 'fail':
                return 17;
            case 'throw':
                throw new Error('application failure');
            case 'late':
                setTimeout(() => request.stdout.write(Buffer.from('late')).catch(() => {}), 50);
                return 0;
            case 'late_shutdown_stdout':
            case 'late_shutdown_stderr':
            case 'late_shutdown_input':
            case 'late_shutdown_nonzero':
                afterShutdown = () => {
                    if (request.argv[0] === 'late_shutdown_nonzero') process.exitCode = 17;
                    const operation = request.argv[0] === 'late_shutdown_input'
                        ? request.stdin.read(1)
                        : request[request.argv[0] === 'late_shutdown_stderr' ? 'stderr' : 'stdout'].write(Buffer.from('late'));
                    operation.catch(() => {});
                };
                return 0;
            case 'blocked_input':
                await request.stdin.read(1);
                return 0;
            case 'blocked_output':
                for (;;) await request.stdout.write(Buffer.alloc(65536));
            case 'streaming': {
                const progress = request.argv[1];
                const pattern = Buffer.alloc(65536);
                for (let index = 0; index < pattern.length; index++) pattern[index] = index % 251;
                const publish = async completed => {
                    await writeFile(`${progress}.pending`, JSON.stringify({ pid: process.pid, completed_bytes: completed }));
                    await rename(`${progress}.pending`, progress);
                };
                await publish(0);
                for (let completed = 0; completed < 128 * 1024 * 1024;) {
                    await request.stdout.write(pattern);
                    completed += pattern.length;
                    await publish(completed);
                }
                return 0;
            }
            case 'unawaited_output':
                request.stdout.write(Buffer.from('queued')).catch(() => {});
                return 0;
            case 'concurrent_output':
                request.stdout.write(Buffer.alloc(65536)).catch(() => {});
                request.stderr.write(Buffer.alloc(65536)).catch(() => {});
                return 0;
            case 'unawaited_input':
                request.stdin.read(1).catch(() => {});
                return 0;
            case 'uncaught':
                setTimeout(() => { throw new Error('escaped timer failure'); }, 0);
                await request.stdin.read(1);
                return 0;
            case 'unhandled':
                void Promise.reject(new Error('escaped promise failure'));
                await request.stdin.read(1);
                return 0;
            default:
                return 0;
        }
    },
    async reset(request) {
        if (request.argv[0] === 'resetfail') throw new Error('resource remained live');
    },
}).then(() => {
    if (afterShutdown) setTimeout(afterShutdown, 50);
}).catch(() => {
    process.exitCode = 70;
});
