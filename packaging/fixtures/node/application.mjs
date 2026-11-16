import { open } from 'node:fs/promises';

export async function readFile(argv, write) {
    if (argv.length !== 2 || argv[0] !== 'read_file') return 2;
    const source = await open(argv[1], 'r');
    try {
        const chunk = Buffer.alloc(32768);
        for (;;) {
            const { bytesRead } = await source.read(chunk, 0, chunk.length, null);
            if (bytesRead === 0) break;
            await write(chunk.subarray(0, bytesRead));
        }
    } finally {
        await source.close();
    }
    return 0;
}
