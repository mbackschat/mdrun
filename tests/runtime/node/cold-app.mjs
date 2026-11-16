import { createReadStream } from 'node:fs';
import { parseArgs } from 'node:util';

const { positionals } = parseArgs({ allowPositionals: true });

async function write(stream, bytes) {
    await new Promise((resolve, reject) => stream.write(bytes, error => error ? reject(error) : resolve()));
}

switch (positionals[0]) {
    case 'ignore':
        await write(process.stdout, Buffer.from('ok'));
        break;
    case 'fail':
        process.exitCode = 17;
        break;
    case 'binary':
        await write(process.stdout, Buffer.from([0, 255, 1]));
        await write(process.stderr, Buffer.from([254, 0]));
        await write(process.stdout, Buffer.from([2]));
        break;
    case 'echo':
        for await (const bytes of process.stdin) await write(process.stdout, bytes);
        break;
    case 'read_file':
        for await (const bytes of createReadStream(positionals[1], { highWaterMark: 32768 })) {
            await write(process.stdout, bytes);
        }
        break;
    default:
        throw new Error('unsupported fixture command');
}
