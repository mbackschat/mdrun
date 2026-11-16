# Task guides

A command in a README can show how to use a tool, but its recorded result can become stale as the tool changes. mdrun keeps the command and expected output together in ordinary Markdown, then reruns the command to check that they still agree. Start with the document guide to record a short example or check one you wrote by hand.

When repeated commands spend time starting the same application, a managed session can reuse its initialization. That requires an application adapter: code supplied by the application developer that defines which calls can safely share a running process. The session guide explains that boundary and how to check whether a call actually used it.

| Command | Use it for |
| --- | --- |
| `mdrun guide documents` | Record a walkthrough, inspect its commands and check its results after a change |
| `mdrun guide sessions` | Reuse a prepared JVM, Node or Python application and check execution and cleanup |
| `mdrun guide reference` | Look up exit statuses, input/output, paths, files, environment and remote delivery |

All three guides are included in the installed executable and work offline. Use `mdrun COMMAND --help` for a command's syntax and examples. The maintained Markdown sources are [documents](documents.md), [sessions](sessions.md) and [reference](reference.md).
