package io.github.mbackschat.mdrun.sdk;

import com.fasterxml.jackson.core.JsonFactory;
import com.fasterxml.jackson.core.JsonGenerator;
import com.fasterxml.jackson.core.JsonParser;
import com.fasterxml.jackson.core.JsonToken;
import com.fasterxml.jackson.core.StreamReadConstraints;
import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;
import java.math.BigInteger;
import java.net.InetSocketAddress;
import java.nio.ByteBuffer;
import java.nio.channels.ClosedChannelException;
import java.nio.channels.ClosedSelectorException;
import java.nio.channels.SelectionKey;
import java.nio.channels.Selector;
import java.nio.channels.SocketChannel;
import java.nio.charset.CodingErrorAction;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.Base64;
import java.util.HashSet;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Objects;
import java.util.Set;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.atomic.AtomicBoolean;
import java.util.concurrent.locks.ReentrantLock;

/** Runs the dedicated JVM process for one cooperative, sequential session.
 *
 * The factory and application callbacks run on one persistent execution thread.
 * All three SDK threads exist before factory initialization. The application owns
 * stopping and checking its threads, children, and other work; this SDK neither
 * virtualizes global JVM I/O nor restores arbitrary process state. The bootstrap
 * must terminate unsuccessfully when this method throws. A retained stream's
 * positive-length I/O after this method finishes halts the dedicated JVM with
 * status 70, because a caught exception cannot revoke a previously returned result.
 */
public final class Worker {
    private static final int FRAME_LIMIT = 1_048_576;
    private static final int CHUNK_LIMIT = 65_536;
    private static final long MAX_SEQUENCE = 9_007_199_254_740_991L;
    private static final AtomicBoolean STARTED = new AtomicBoolean();
    private static final JsonFactory JSON = new JsonFactory()
            .enable(JsonParser.Feature.STRICT_DUPLICATE_DETECTION)
            .setStreamReadConstraints(StreamReadConstraints.builder()
                    .maxNestingDepth(16)
                    .maxStringLength(FRAME_LIMIT)
                    .maxNumberLength(32)
                    .build());

    private Worker() {}

    /** Serves one worker lifetime using the private configuration path supplied by the host. */
    public static void run(Path configPath, ApplicationFactory factory) throws Exception {
        if (!STARTED.compareAndSet(false, true)) {
            throw new Failure("worker_already_started");
        }
        if (configPath == null || factory == null) {
            throw new Failure("invalid_adapter");
        }
        Config config;
        try (InputStream input = Files.newInputStream(configPath)) {
            byte[] bytes = input.readNBytes(FRAME_LIMIT + 1);
            require(bytes.length > 0 && bytes.length <= FRAME_LIMIT);
            config = Config.parse(parse(bytes));
        } catch (Throwable ignored) {
            throw new Failure("invalid_config");
        }
        new Session(config, factory).run();
    }

    private enum Phase { CLASSIFYING, ELIGIBLE, RUNNING, RESETTING, COMPLETE, RELEASED, FAULTED }
    private enum Operation { CLASSIFY, INVOKE }
    private enum InboundKind { CLASSIFY, INVOKE, STDIN, STDIN_EOF, RELEASE }

    private record Inbound(InboundKind kind, String id, List<String> argv, byte[] bytes) {
        static Inbound parse(Map<String, Object> message) throws WireFailure {
            String kind = string(message.get("kind"));
            List<String> argv = List.of();
            byte[] bytes = null;
            InboundKind decoded;
            switch (kind) {
                case "classify" -> {
                    fields(message, "kind", "request_id", "argv");
                    argv = strings(message.get("argv"));
                    decoded = InboundKind.CLASSIFY;
                }
                case "invoke" -> {
                    fields(message, "kind", "request_id");
                    decoded = InboundKind.INVOKE;
                }
                case "stdin" -> {
                    fields(message, "kind", "request_id", "base64");
                    String base64 = string(message.get("base64"));
                    require(!base64.isEmpty() && base64.length() <= 87_384);
                    try {
                        bytes = Base64.getDecoder().decode(base64);
                    } catch (IllegalArgumentException ignored) {
                        throw new WireFailure();
                    }
                    require(bytes.length > 0 && bytes.length <= CHUNK_LIMIT
                            && Base64.getEncoder().encodeToString(bytes).equals(base64));
                    decoded = InboundKind.STDIN;
                }
                case "stdin_eof" -> {
                    fields(message, "kind", "request_id");
                    decoded = InboundKind.STDIN_EOF;
                }
                case "release" -> {
                    fields(message, "kind", "request_id");
                    decoded = InboundKind.RELEASE;
                }
                default -> throw new WireFailure();
            }
            return new Inbound(decoded, nonempty(message.get("request_id")), argv, bytes);
        }
    }

    private record Config(String sessionId, String artifactId, String token, int port) {
        static Config parse(Map<String, Object> value) throws WireFailure {
            fields(value, "protocol", "session_id", "artifact_id", "token", "connect");
            require(integer(value.get("protocol"), 1, 1) == 1);
            Map<String, Object> connect = object(value.get("connect"));
            fields(connect, "host", "port");
            require(string(connect.get("host")).equals("127.0.0.1"));
            return new Config(nonempty(value.get("session_id")), nonempty(value.get("artifact_id")),
                    nonempty(value.get("token")), integer(connect.get("port"), 1, 65_535));
        }
    }

    private record Task(Operation operation, Request request) {}

    private static final class PendingRead {
        final int limit;
        boolean done;
        byte[] bytes;

        PendingRead(int limit) {
            this.limit = limit;
        }
    }

    private static final class Request {
        final String id;
        final List<String> argv;
        final ReentrantLock inputLock = new ReentrantLock();
        final ReentrantLock outputLock = new ReentrantLock();
        final Invocation invocation;
        Phase phase = Phase.CLASSIFYING;
        PendingRead pendingRead;
        boolean inputBusy;
        boolean outputBusy;
        boolean eof;
        long sequence;

        Request(Session session, String id, List<String> argv) {
            this.id = id;
            this.argv = argv;
            this.invocation = new Invocation(id, argv, new RequestInput(session, this),
                    new RequestOutput(session, this, "stdout"), new RequestOutput(session, this, "stderr"));
        }
    }

    private static final class Session {
        final Config config;
        final ApplicationFactory factory;
        final Object lock = new Object();
        final Set<String> requestIds = new HashSet<>();
        final CountDownLatch threadsStarted = new CountDownLatch(3);
        final CountDownLatch initialized = new CountDownLatch(1);
        final CountDownLatch executionExited = new CountDownLatch(1);
        final Thread dataThread;
        final Thread controlThread;
        final Thread executionThread;
        Channel data;
        Channel control;
        Application application;
        Request current;
        Task task;
        boolean ready;
        boolean stopped;
        boolean returned;
        String failure;

        Session(Config config, ApplicationFactory factory) {
            this.config = config;
            this.factory = factory;
            dataThread = thread("mdrun-data", this::dataLoop);
            controlThread = thread("mdrun-control", this::controlLoop);
            executionThread = thread("mdrun-execution", this::executionLoop);
        }

        private Thread thread(String name, Runnable body) {
            Thread thread = new Thread(() -> {
                threadsStarted.countDown();
                body.run();
            }, name);
            thread.setDaemon(true);
            return thread;
        }

        void run() throws Exception {
            Thread.UncaughtExceptionHandler previous = Thread.getDefaultUncaughtExceptionHandler();
            boolean launched = false;
            try {
                data = Channel.connect(config.port);
                data.send(hello("worker_data"));
                control = Channel.connect(config.port);
                control.send(hello("worker_control"));
                Thread.setDefaultUncaughtExceptionHandler((thread, error) -> stop("uncaught_exception"));
                dataThread.start();
                controlThread.start();
                executionThread.start();
                launched = true;
                await(initialized);
                synchronized (lock) {
                    if (!stopped) {
                        ready = true;
                    }
                }
                if (!isStopped()) {
                    data.send(frame("kind", "ready", "protocol", 1, "session_id", config.sessionId,
                            "artifact_id", config.artifactId,
                            "capabilities", List.of("pull_stdin", "sequenced_output", "release_ack")));
                }
                synchronized (lock) {
                    while (!stopped) {
                        lock.wait();
                    }
                }
            } catch (Throwable ignored) {
                stop("worker_failed");
            } finally {
                if (!isStopped()) {
                    stop("worker_failed");
                }
                closeChannels();
                if (launched) {
                    await(executionExited);
                    join(dataThread);
                    join(controlThread);
                    join(executionThread);
                }
                Thread.setDefaultUncaughtExceptionHandler(previous);
            }
            synchronized (lock) {
                returned = true;
                if (failure != null) {
                    throw new Failure(failure);
                }
            }
        }

        private Map<String, Object> hello(String role) {
            return frame("kind", "hello", "protocol", 1, "session_id", config.sessionId,
                    "artifact_id", config.artifactId, "token", config.token, "role", role);
        }

        private void await(CountDownLatch latch) {
            boolean interrupted = false;
            while (true) {
                try {
                    latch.await();
                    break;
                } catch (InterruptedException ignored) {
                    interrupted = true;
                    stop("worker_interrupted");
                }
            }
            if (interrupted) {
                Thread.currentThread().interrupt();
            }
        }

        private void join(Thread thread) {
            boolean interrupted = false;
            while (thread.isAlive()) {
                try {
                    thread.join();
                } catch (InterruptedException ignored) {
                    interrupted = true;
                    stop("worker_interrupted");
                }
            }
            if (interrupted) {
                Thread.currentThread().interrupt();
            }
        }

        private boolean isStopped() {
            synchronized (lock) {
                return stopped;
            }
        }

        private void executionLoop() {
            try {
                threadsStarted.await();
                if (isStopped()) {
                    return;
                }
                application = factory.create();
                if (application == null) {
                    stop("initialization_failed");
                    return;
                }
                initialized.countDown();
                while (true) {
                    Task next;
                    synchronized (lock) {
                        while (task == null && !stopped) {
                            lock.wait();
                        }
                        if (stopped) {
                            return;
                        }
                        next = task;
                        task = null;
                    }
                    if (next.operation == Operation.CLASSIFY) {
                        classify(next.request);
                    } else {
                        invoke(next.request);
                    }
                }
            } catch (Throwable ignored) {
                if (!isStopped()) {
                    stop(application == null ? "initialization_failed" : "execution_failed");
                }
            } finally {
                initialized.countDown();
                if (application != null) {
                    try {
                        application.close();
                    } catch (Throwable ignored) {
                        stop("close_failed");
                    }
                }
                executionExited.countDown();
            }
        }

        private void classify(Request request) {
            Classification result;
            try {
                result = application.classify(request.argv);
            } catch (Throwable ignored) {
                stop("classification_failed");
                return;
            }
            try {
                Map<String, Object> response;
                synchronized (lock) {
                    if (stopped) {
                        return;
                    }
                    require(current == request && request.phase == Phase.CLASSIFYING);
                    if (result instanceof Classification.Eligible) {
                        request.phase = Phase.ELIGIBLE;
                        response = frame("kind", "eligible", "request_id", request.id);
                    } else if (result instanceof Classification.Refused refused) {
                        String reason = nonempty(refused.reason());
                        request.phase = Phase.RELEASED;
                        current = null;
                        response = frame("kind", "refused", "request_id", request.id, "reason", reason);
                    } else {
                        throw new WireFailure();
                    }
                }
                data.send(response);
            } catch (LocalClosure ignored) {
                if (!isStopped()) {
                    stop("invalid_classification");
                }
            } catch (Throwable ignored) {
                stop("invalid_classification");
            }
        }

        private void invoke(Request request) {
            int status;
            try {
                status = application.invoke(request.invocation);
            } catch (Throwable ignored) {
                stop("invocation_failed");
                return;
            }
            try {
                synchronized (lock) {
                    if (stopped) {
                        return;
                    }
                    request.phase = Phase.RESETTING;
                    if (status < 0 || status > 255) {
                        stop("invalid_exit_code");
                        return;
                    }
                    if (request.inputBusy || request.pendingRead != null) {
                        stop("outstanding_input");
                        return;
                    }
                    while (request.outputBusy && !stopped) {
                        lock.wait();
                    }
                    if (stopped) {
                        return;
                    }
                }
                application.reset(request.invocation);
            } catch (Throwable ignored) {
                stop("reset_failed");
                return;
            }
            try {
                synchronized (lock) {
                    if (stopped) {
                        return;
                    }
                    request.phase = Phase.COMPLETE;
                }
                data.send(frame("kind", "complete", "request_id", request.id, "exit_code", status));
            } catch (Throwable ignored) {
                stop("output_failed");
            }
        }

        private void dataLoop() {
            try {
                while (!isStopped()) {
                    receive(Inbound.parse(data.read()));
                }
            } catch (LocalClosure ignored) {
                if (!isStopped()) {
                    stop("data_lost");
                }
            } catch (ConnectionLost ignored) {
                stop("data_lost");
            } catch (Throwable ignored) {
                stop("protocol_error");
            }
        }

        private void controlLoop() {
            try {
                Map<String, Object> message = control.read();
                fields(message, "kind");
                require(string(message.get("kind")).equals("shutdown"));
                synchronized (lock) {
                    if (current != null) {
                        stop("shutdown_during_invocation");
                        return;
                    }
                }
                stop(null);
            } catch (LocalClosure ignored) {
                if (!isStopped()) {
                    stop("owner_lost");
                }
            } catch (ConnectionLost ignored) {
                stop("owner_lost");
            } catch (Throwable ignored) {
                stop("invalid_control");
            }
        }

        private void receive(Inbound message) throws Exception {
            synchronized (lock) {
                if (stopped) {
                    return;
                }
                require(ready);
            }
            switch (message.kind) {
                case CLASSIFY -> {
                    synchronized (lock) {
                        require(current == null && task == null && requestIds.add(message.id));
                        current = new Request(this, message.id, message.argv);
                        task = new Task(Operation.CLASSIFY, current);
                        lock.notifyAll();
                    }
                }
                case INVOKE -> {
                    Request request;
                    synchronized (lock) {
                        request = matching(message.id, Phase.ELIGIBLE);
                        require(task == null);
                        request.phase = Phase.RUNNING;
                    }
                    data.send(frame("kind", "accepted", "request_id", request.id));
                    synchronized (lock) {
                        if (!stopped) {
                            task = new Task(Operation.INVOKE, request);
                            lock.notifyAll();
                        }
                    }
                }
                case STDIN, STDIN_EOF -> {
                    synchronized (lock) {
                        Request request = matching(message.id, Phase.RUNNING);
                        PendingRead pending = request.pendingRead;
                        require(pending != null && !pending.done);
                        require(message.bytes == null || message.bytes.length <= pending.limit);
                        request.pendingRead = null;
                        request.eof = message.bytes == null;
                        pending.bytes = message.bytes;
                        pending.done = true;
                        lock.notifyAll();
                    }
                }
                case RELEASE -> {
                    Request request;
                    synchronized (lock) {
                        request = matching(message.id, Phase.COMPLETE);
                        request.phase = Phase.RELEASED;
                        current = null;
                    }
                    data.send(frame("kind", "released", "request_id", request.id));
                }
            }
        }

        private Request matching(String id, Phase phase) throws WireFailure {
            require(current != null && current.phase == phase
                    && current.id.equals(id));
            return current;
        }

        private void stop(String reason) {
            Request active;
            boolean first;
            synchronized (lock) {
                if (returned && reason != null) {
                    Runtime.getRuntime().halt(70);
                }
                if (reason != null && failure == null) {
                    failure = reason;
                }
                first = !stopped;
                stopped = true;
                active = current;
                if (active != null) {
                    active.phase = Phase.FAULTED;
                }
                lock.notifyAll();
            }
            if (first) {
                if (reason != null && data != null) {
                    data.abort(frame("kind", "fault", "request_id", active == null ? null : active.id,
                            "reason", reason));
                }
                closeChannels();
            }
            if (reason != null && Thread.currentThread() != executionThread) {
                executionThread.interrupt();
            }
        }

        private void closeChannels() {
            if (control != null) {
                control.close();
            }
            if (data != null) {
                data.close();
            }
        }

        private boolean live(Request request) {
            return !stopped && current == request && request.phase == Phase.RUNNING;
        }

        private IOException violation(String reason) {
            stop(reason);
            return new IOException(reason);
        }
    }

    private static final class RequestInput extends InputStream {
        final Session session;
        final Request request;

        RequestInput(Session session, Request request) {
            this.session = session;
            this.request = request;
        }

        @Override
        public int read() throws IOException {
            byte[] byteValue = new byte[1];
            return read(byteValue, 0, 1) < 0 ? -1 : Byte.toUnsignedInt(byteValue[0]);
        }

        @Override
        public int read(byte[] bytes, int offset, int length) throws IOException {
            Objects.checkFromIndexSize(offset, length, bytes.length);
            if (length == 0) {
                return 0;
            }
            if (!request.inputLock.tryLock()) {
                throw session.violation("concurrent_input");
            }
            PendingRead pending = null;
            try {
                synchronized (session.lock) {
                    if (!session.live(request)) {
                        throw session.violation("late_input");
                    }
                    if (request.eof) {
                        return -1;
                    }
                    pending = new PendingRead(Math.min(length, CHUNK_LIMIT));
                    request.pendingRead = pending;
                    request.inputBusy = true;
                }
                try {
                    session.data.send(frame("kind", "stdin_read", "request_id", request.id,
                            "max_bytes", pending.limit));
                } catch (Throwable ignored) {
                    throw session.violation("input_failed");
                }
                synchronized (session.lock) {
                    while (!pending.done && !session.stopped) {
                        session.lock.wait();
                    }
                    if (session.stopped) {
                        throw new IOException("worker_stopped");
                    }
                    if (pending.bytes == null) {
                        return -1;
                    }
                    System.arraycopy(pending.bytes, 0, bytes, offset, pending.bytes.length);
                    return pending.bytes.length;
                }
            } catch (InterruptedException ignored) {
                Thread.currentThread().interrupt();
                throw session.violation("input_interrupted");
            } finally {
                synchronized (session.lock) {
                    if (pending != null) {
                        request.inputBusy = false;
                    }
                    session.lock.notifyAll();
                }
                request.inputLock.unlock();
            }
        }
    }

    private static final class RequestOutput extends OutputStream {
        final Session session;
        final Request request;
        final String stream;

        RequestOutput(Session session, Request request, String stream) {
            this.session = session;
            this.request = request;
            this.stream = stream;
        }

        @Override
        public void write(int value) throws IOException {
            write(new byte[] {(byte) value}, 0, 1);
        }

        @Override
        public void write(byte[] bytes, int offset, int length) throws IOException {
            Objects.checkFromIndexSize(offset, length, bytes.length);
            if (length == 0) {
                return;
            }
            if (!request.outputLock.tryLock()) {
                throw session.violation("concurrent_output");
            }
            boolean started = false;
            try {
                synchronized (session.lock) {
                    if (!session.live(request)) {
                        throw session.violation("late_output");
                    }
                    request.outputBusy = true;
                    started = true;
                }
                for (int consumed = 0; consumed < length;) {
                    int count = Math.min(CHUNK_LIMIT, length - consumed);
                    long sequence;
                    synchronized (session.lock) {
                        if (session.stopped) {
                            throw new IOException("worker_stopped");
                        }
                        if (request.sequence > MAX_SEQUENCE) {
                            throw session.violation("sequence_limit");
                        }
                        sequence = request.sequence++;
                    }
                    String base64 = Base64.getEncoder().encodeToString(
                            Arrays.copyOfRange(bytes, offset + consumed, offset + consumed + count));
                    try {
                        session.data.send(frame("kind", "output", "request_id", request.id,
                                "sequence", sequence, "stream", stream, "base64", base64));
                    } catch (Throwable ignored) {
                        throw session.violation("output_failed");
                    }
                    consumed += count;
                }
            } finally {
                synchronized (session.lock) {
                    if (started) {
                        request.outputBusy = false;
                    }
                    session.lock.notifyAll();
                }
                request.outputLock.unlock();
            }
        }
    }

    private static final class Channel {
        final SocketChannel socket;
        final Selector reader;
        final Selector writer;
        final ReentrantLock writeLock = new ReentrantLock();
        volatile boolean closed;

        Channel(SocketChannel socket, Selector reader, Selector writer) {
            this.socket = socket;
            this.reader = reader;
            this.writer = writer;
        }

        static Channel connect(int port) throws IOException {
            SocketChannel socket = SocketChannel.open();
            Selector reader = null;
            Selector writer = null;
            try {
                socket.connect(new InetSocketAddress("127.0.0.1", port));
                socket.configureBlocking(false);
                reader = Selector.open();
                writer = Selector.open();
                socket.register(reader, SelectionKey.OP_READ);
                socket.register(writer, SelectionKey.OP_WRITE);
                return new Channel(socket, reader, writer);
            } catch (Throwable error) {
                socket.close();
                if (reader != null) {
                    reader.close();
                }
                if (writer != null) {
                    writer.close();
                }
                throw error;
            }
        }

        Map<String, Object> read() throws Exception {
            long length = Integer.toUnsignedLong(readBytes(4).getInt());
            require(length > 0 && length <= FRAME_LIMIT);
            return parse(readBytes((int) length).array());
        }

        private ByteBuffer readBytes(int length) throws IOException {
            ByteBuffer bytes = ByteBuffer.allocate(length);
            try {
                while (bytes.hasRemaining()) {
                    if (closed) {
                        throw new LocalClosure();
                    }
                    int count = socket.read(bytes);
                    if (count < 0) {
                        throw new ConnectionLost();
                    }
                    if (count == 0) {
                        reader.select();
                        reader.selectedKeys().clear();
                    }
                }
            } catch (ClosedChannelException | ClosedSelectorException ignored) {
                if (closed) {
                    throw new LocalClosure();
                }
                throw new ConnectionLost();
            }
            bytes.flip();
            return bytes;
        }

        void send(Map<String, Object> message) throws Exception {
            ByteBuffer bytes = encode(message);
            writeLock.lockInterruptibly();
            try {
                while (bytes.hasRemaining()) {
                    if (closed) {
                        throw new LocalClosure();
                    }
                    if (socket.write(bytes) == 0) {
                        writer.select();
                        writer.selectedKeys().clear();
                    }
                }
            } catch (ClosedChannelException | ClosedSelectorException ignored) {
                if (closed) {
                    throw new LocalClosure();
                }
                throw new ConnectionLost();
            } finally {
                writeLock.unlock();
            }
        }

        void abort(Map<String, Object> message) {
            if (writeLock.tryLock()) {
                try {
                    ByteBuffer bytes = encode(message);
                    while (bytes.hasRemaining() && !closed && socket.write(bytes) > 0) {
                        // A fault is best effort; owner-loss cancellation must never wait for a reader.
                    }
                } catch (Throwable ignored) {
                    // The host also observes connection loss and the worker's actual process status.
                } finally {
                    writeLock.unlock();
                }
            }
            close();
        }

        void close() {
            closed = true;
            try {
                socket.close();
            } catch (IOException ignored) {
                // Closing both selectors still releases pending SDK waits.
            }
            try {
                reader.close();
            } catch (IOException ignored) {
                // Socket closure is independently authoritative for the peer.
            }
            try {
                writer.close();
            } catch (IOException ignored) {
                // Socket closure is independently authoritative for the peer.
            }
        }
    }

    private static Map<String, Object> parse(byte[] bytes) throws Exception {
        String text = StandardCharsets.UTF_8.newDecoder()
                .onMalformedInput(CodingErrorAction.REPORT)
                .onUnmappableCharacter(CodingErrorAction.REPORT)
                .decode(ByteBuffer.wrap(bytes)).toString();
        try (JsonParser parser = JSON.createParser(text)) {
            require(parser.nextToken() == JsonToken.START_OBJECT);
            Map<String, Object> value = readObject(parser);
            require(parser.nextToken() == null);
            return value;
        }
    }

    private static Map<String, Object> readObject(JsonParser parser) throws Exception {
        Map<String, Object> value = new LinkedHashMap<>();
        while (parser.nextToken() != JsonToken.END_OBJECT) {
            require(parser.currentToken() == JsonToken.FIELD_NAME);
            String name = string(parser.currentName());
            require(parser.nextToken() != null);
            value.put(name, readValue(parser));
        }
        return value;
    }

    private static Object readValue(JsonParser parser) throws Exception {
        return switch (parser.currentToken()) {
            case START_OBJECT -> readObject(parser);
            case START_ARRAY -> {
                List<Object> values = new ArrayList<>();
                while (parser.nextToken() != JsonToken.END_ARRAY) {
                    require(parser.currentToken() != null);
                    values.add(readValue(parser));
                }
                yield values;
            }
            case VALUE_STRING -> string(parser.getText());
            case VALUE_NUMBER_INT -> parser.getBigIntegerValue();
            case VALUE_TRUE -> Boolean.TRUE;
            case VALUE_FALSE -> Boolean.FALSE;
            case VALUE_NULL -> null;
            default -> throw new WireFailure();
        };
    }

    private static ByteBuffer encode(Map<String, Object> message) throws Exception {
        ByteArrayOutputStream output = new ByteArrayOutputStream();
        try (JsonGenerator generator = JSON.createGenerator(output)) {
            writeValue(generator, message);
        }
        byte[] body = output.toByteArray();
        require(body.length > 0 && body.length <= FRAME_LIMIT);
        return ByteBuffer.allocate(4 + body.length).putInt(body.length).put(body).flip();
    }

    private static void writeValue(JsonGenerator generator, Object value) throws IOException {
        if (value == null) {
            generator.writeNull();
        } else if (value instanceof String text) {
            generator.writeString(text);
        } else if (value instanceof Integer number) {
            generator.writeNumber(number);
        } else if (value instanceof Long number) {
            generator.writeNumber(number);
        } else if (value instanceof List<?> values) {
            generator.writeStartArray();
            for (Object item : values) {
                writeValue(generator, item);
            }
            generator.writeEndArray();
        } else if (value instanceof Map<?, ?> fields) {
            generator.writeStartObject();
            for (Map.Entry<?, ?> entry : fields.entrySet()) {
                generator.writeFieldName((String) entry.getKey());
                writeValue(generator, entry.getValue());
            }
            generator.writeEndObject();
        } else {
            throw new IOException("invalid_frame");
        }
    }

    private static Map<String, Object> frame(Object... pairs) {
        Map<String, Object> value = new LinkedHashMap<>();
        for (int index = 0; index < pairs.length; index += 2) {
            value.put((String) pairs[index], pairs[index + 1]);
        }
        return value;
    }

    private static Map<String, Object> object(Object value) throws WireFailure {
        require(value instanceof Map<?, ?>);
        Map<String, Object> result = new LinkedHashMap<>();
        for (Map.Entry<?, ?> entry : ((Map<?, ?>) value).entrySet()) {
            require(entry.getKey() instanceof String);
            result.put((String) entry.getKey(), entry.getValue());
        }
        return result;
    }

    private static void fields(Map<String, Object> value, String... names) throws WireFailure {
        require(value.keySet().equals(Set.of(names)));
    }

    private static String string(Object value) throws WireFailure {
        require(value instanceof String);
        String text = (String) value;
        for (int index = 0; index < text.length(); index++) {
            char character = text.charAt(index);
            if (Character.isHighSurrogate(character)) {
                require(index + 1 < text.length() && Character.isLowSurrogate(text.charAt(++index)));
            } else {
                require(!Character.isLowSurrogate(character));
            }
        }
        return text;
    }

    private static String nonempty(Object value) throws WireFailure {
        String text = string(value);
        require(!text.isEmpty());
        return text;
    }

    private static int integer(Object value, int minimum, int maximum) throws WireFailure {
        require(value instanceof BigInteger);
        BigInteger number = (BigInteger) value;
        require(number.compareTo(BigInteger.valueOf(minimum)) >= 0
                && number.compareTo(BigInteger.valueOf(maximum)) <= 0);
        return number.intValue();
    }

    private static List<String> strings(Object value) throws WireFailure {
        require(value instanceof List<?>);
        List<String> strings = new ArrayList<>();
        for (Object item : (List<?>) value) {
            strings.add(string(item));
        }
        return List.copyOf(strings);
    }

    private static void require(boolean condition) throws WireFailure {
        if (!condition) {
            throw new WireFailure();
        }
    }

    private static final class WireFailure extends Exception {
        private static final long serialVersionUID = 1L;
    }

    private static final class ConnectionLost extends IOException {
        private static final long serialVersionUID = 1L;
    }

    private static final class LocalClosure extends IOException {
        private static final long serialVersionUID = 1L;
    }

    private static final class Failure extends Exception {
        private static final long serialVersionUID = 1L;

        Failure(String reason) {
            super(reason);
        }
    }
}
