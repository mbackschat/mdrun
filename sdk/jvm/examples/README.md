# Application output helper

[InvocationOutput](src/main/java/io/github/mbackschat/mdrun/example/InvocationOutput.java) is an application-side example for a callable CLI that accepts `Appendable` sinks. It is outside the published SDK source set and is not an automatic JVM output or reset facility. It uses the [experimental invocation API](../README.md), including sticky `Invocation.invalidate()`.

Capture the original `System.out.charset()` and `System.err.charset()` when constructing the application, before replacing any process streams. Create one helper per invocation and close it before `invoke` returns. For example, inside an application's handler:

```java
try (InvocationOutput output = new InvocationOutput(invocation, stdoutCharset, stderrCharset)) {
    output.stdout().append("hello\n");
    return 0;
}
```

The two opaque `Appendable` sinks share one lock with admission close. Nonempty appends check admission before encoding; a retained append invalidates the whole SDK worker even when the encoding emits no bytes or the caller catches the exception. Accepted operations finish before close returns. Close is idempotent and flushes without closing the private persistent JDK encoders, preserving packaged `System.out`/`System.err` behavior for split pairs and incomplete trailing surrogates. Each invocation needs fresh encoders so a pending character cannot cross requests.

Character sequences are read in fixed-size slices rather than converted wholesale with `toString()`. Keep the sequence unchanged until append returns. Nulls, valid subsequences, `CharBuffer` position/limit and empty appends follow the `Appendable` character contract; invalid ranges fail before output. A sequence failure observed during an admitted operation irreversibly invalidates the worker, because output may already have been emitted. Same-length concurrent mutations that produce no observable failure cannot be detected without stronger caller ownership.

Run the [independent cold-JDK and SDK peer guards](../../../tests/runtime/jvm/output_bridge.py) from the repository root:

```sh
MDRUN_JAVA_HOME="$JAVA_HOME" \
MDRUN_JACKSON_CORE=/prepared/jackson-core-2.22.4.jar \
python3 tests/runtime/jvm/output_bridge.py
```
