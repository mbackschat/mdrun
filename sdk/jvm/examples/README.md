# Application output helper

Use [InvocationOutput](src/main/java/io/github/mbackschat/mdrun/example/InvocationOutput.java) when your callable JVM application already accepts `Appendable` output sinks. It lets your adapter pass characters to that API while sending the encoded bytes through the current mdrun invocation.

A normal CLI often writes characters through `System.out` and `System.err`. In a warm worker those process streams are private diagnostics, and a retained writer must not write into a later request. This helper gives each invocation its own sinks and character encoders, preserves the original process-stream charsets and rejects nonempty appends after that invocation's output has closed.

The helper is supplied here as application-side example source, outside the SDK package and release archive. Copy or adapt it into your application build if it suits your callable API. Your adapter still routes application output, stops owned work and checks whether reuse is safe; the helper performs neither global stream routing nor application reset. It uses the frozen [version-1 invocation API](../README.md), including sticky `Invocation.invalidate()`.

## Use it inside invoke

Capture the original `System.out.charset()` and `System.err.charset()` when constructing your application, before replacing any process streams. Store them as `stdoutCharset` and `stderrCharset` in the adapter. Then create one helper for each invocation and close it before `invoke` returns:

```java
try (InvocationOutput output = new InvocationOutput(invocation, stdoutCharset, stderrCharset)) {
    output.stdout().append("hello\n");
    return 0;
}
```

This fragment belongs inside your application's `invoke` method, where `invocation` is the SDK-supplied request. It writes `hello` followed by a newline using the saved stdout charset and returns status `0` after the helper closes. In your actual handler, pass `output.stdout()` and `output.stderr()` to the callable application instead of appending the greeting. A later nonempty append through either retained sink invalidates the whole worker, even if the caller catches the exception or character encoding emits no bytes.

Next, compare the real callable command's output with independent packaged execution, including the charsets and character boundaries it uses. Follow the [JVM adapter guide](../README.md#connect-it-to-a-session) to connect it to a binding; successful output handling still requires application reset and resource checks.

## Character and lifetime contract

The two opaque `Appendable` sinks share one lock with admission close. Nonempty appends check admission before encoding, and accepted operations finish before close returns. Close is idempotent and flushes without closing the private persistent JDK encoders, preserving packaged `System.out`/`System.err` behavior for split pairs and incomplete trailing surrogates. Each invocation needs fresh encoders so a pending character cannot cross requests.

Character sequences are read in fixed-size slices rather than converted wholesale with `toString()`. Keep the sequence unchanged until append returns. Nulls, valid subsequences, `CharBuffer` position/limit and empty appends follow the `Appendable` character contract; invalid ranges fail before output. A sequence failure observed during an admitted operation irreversibly invalidates the worker, because output may already have been emitted. Same-length concurrent mutations that produce no observable failure cannot be detected without stronger caller ownership.

## Verify the helper

Run the [independent cold-JDK and SDK peer guards](../../../tests/runtime/jvm/output_bridge.py) from the repository root. Supply your Java 21 JDK through `JAVA_HOME` and the verified locked Jackson jar at the illustrated prepared path:

```sh
MDRUN_JAVA_HOME="$JAVA_HOME" \
MDRUN_JACKSON_CORE=/prepared/jackson-core-2.22.4.jar \
python3 tests/runtime/jvm/output_bridge.py
```
