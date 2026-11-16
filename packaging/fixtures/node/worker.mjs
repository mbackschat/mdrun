import { runWorker } from './index.mjs';
import { readFile } from './application.mjs';

try {
    await runWorker({
        configPath: process.argv[2],
        classify: argv => argv.length === 2 && argv[0] === 'read_file' ? true : 'unsupported_command',
        invoke: request => readFile(request.argv, bytes => request.stdout.write(bytes)),
        // readFile closes its one file handle before resolving and owns no detached work.
        reset() {},
    });
} catch {
    process.exitCode = 70;
}
