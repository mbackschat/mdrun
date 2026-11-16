import { readFile } from './application.mjs';

process.exitCode = await readFile(process.argv.slice(2), bytes => new Promise((resolve, reject) => {
    process.stdout.write(bytes, error => error ? reject(error) : resolve());
}));
