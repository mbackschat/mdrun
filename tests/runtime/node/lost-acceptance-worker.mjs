import { appendFileSync, closeSync, fsyncSync, openSync, readFileSync } from 'node:fs';
import { connect } from 'node:net';

const config = JSON.parse(readFileSync(process.argv[2], 'utf8'));
const { protocol, session_id, artifact_id, token } = config;
const data = connect(config.connect);
const control = connect(config.connect);

function send(socket, frame) {
    const bytes = Buffer.from(JSON.stringify(frame));
    const header = Buffer.alloc(4);
    header.writeUInt32BE(bytes.length);
    socket.write(Buffer.concat([header, bytes]));
}

data.once('connect', () => {
    send(data, { kind: 'hello', protocol, session_id, artifact_id, token, role: 'worker_data' });
    send(data, { kind: 'ready', protocol, session_id, artifact_id,
        capabilities: ['pull_stdin', 'sequenced_output', 'release_ack'] });
});
control.once('connect', () => {
    send(control, { kind: 'hello', protocol, session_id, artifact_id, token, role: 'worker_control' });
});

let pending = Buffer.alloc(0);
let effect;
data.on('data', bytes => {
    pending = Buffer.concat([pending, bytes]);
    while (pending.length >= 4 && pending.length >= 4 + pending.readUInt32BE()) {
        const length = pending.readUInt32BE();
        const frame = JSON.parse(pending.subarray(4, 4 + length));
        pending = pending.subarray(4 + length);
        if (frame.kind === 'classify') {
            effect = frame.argv[0];
            send(data, { kind: 'eligible', request_id: frame.request_id });
        } else if (frame.kind === 'invoke') {
            const descriptor = openSync(effect, 'a');
            appendFileSync(descriptor, 'warm\n');
            fsyncSync(descriptor);
            closeSync(descriptor);
            // Execution happened, but its acceptance response is lost: retry would duplicate effects.
            data.destroy();
            control.destroy();
        }
    }
});
