# Create and verify a document

Suppose a walkthrough says that a command prints a particular result. You can paste the result into a README, but checking it after every application change means running the command and comparing the output yourself. mdrun does that comparison from the document: keep a command beside its expected output, then run `mdrun verify`.

The document remains ordinary Markdown that you can read, edit and commit alongside your code. Write it by hand, or let `init`, `note` and `exec` capture commands and their real output. These authoring commands are optional; verification supports both approaches. The same workflow fits reproducible demonstrations, coding-agent handoffs and command-line walkthroughs.

## First recording

The first example makes a small document you can verify immediately. Its command/output portion looks like this:

````markdown
```sh
printf "hello from mdrun\n"
```

```output
hello from mdrun
```
````

The `sh` fence contains a shell command; the adjacent `output` fence records what that command should print. To create this document by recording a real execution, run these commands in a new directory with `mdrun` on PATH:

```sh
mdrun init demo.md 'A repeatable greeting'
mdrun note demo.md 'This command prints a greeting.'
mdrun exec demo.md sh 'printf "hello from mdrun\n"'
mdrun verify demo.md
```

`init` creates `demo.md`, `note` adds the explanation, and `exec` runs the shell command, prints its captured output and appends the command/output pair shown above. `verify` runs the recorded command again. Matching output produces no diff and exit status zero, leaving `demo.md` unchanged. Open the file to see the title and explanation around the example.

## Write the Markdown yourself

To use your editor instead of the recording commands, save the fragment above directly as `manual.md` and run:

```sh
mdrun verify manual.md
```

The `sh` label selects the shell that executes the code. Titles and explanations are normal Markdown; verification needs no generated timestamp or document ID. The authoring CLI saves you from copying output by hand, but adds no requirement to use it for later edits. Without an `output` fence, a command still executes during verification, but its output is not compared.

You can keep this format when reusing an application in a warm session. A shell block calls the application's chosen command name, and `verify --binding` supplies the prepared application behind it. See the [paired warm-session example](../examples/warm-python.md#authoring-calls-and-the-resulting-markdown), then the [session guide](sessions.md) to integrate your own application.

## Write longer commands

For a paragraph or multiline command, omit the final text/code argument and supply it on standard input. In this example, the pipeline supplies note text and the here-document supplies shell code to record:

```sh
printf '%s\n' 'A longer explanation.' | mdrun note demo.md
mdrun exec demo.md sh <<'CODE'
printf 'first line\n'
printf 'second line\n'
CODE
```

These calls append the following Markdown:

````markdown
A longer explanation.

```sh
printf 'first line\n'
printf 'second line\n'

```

```output
first line
second line
```
````

Input to the authoring CLI supplies the recording's source; it does not become the executed program's standard input. Executed blocks read EOF from `/dev/null` unless the code supplies input through a file, pipeline or redirection.

mdrun invokes the fence's interpreter as `INTERPRETER -c CODE`. Shells and `python3` accept that form. To run a program with a different command-line interface, put its normal command inside a shell block. For example, `mdrun exec demo.md sh 'node app.mjs'` runs the `app.mjs` file you supply using Node on PATH.

## Check changes without overwriting evidence

After an application change, you may want to see new results while retaining the old document for comparison. Inspect unfamiliar commands with `list` and `preview` before executing them, then ask `verify` to write a separate candidate:

```sh
mdrun list demo.md
mdrun verify demo.md --output candidate.md
```

`verify` executes all recognized code blocks except image blocks and compares captured bytes wherever recorded output is present. If output changed, it prints a diff and exits 1; `candidate.md` contains updated captures. Review that file before replacing the original. Selecting `--output demo.md` explicitly permits an in-place update.

Verification checks recorded output, not application exit statuses, and does not roll back command side effects. Commands without a recorded output block still execute, so success says nothing about their output. Markdown also has representation limits: parsing reconstructs LF line endings, and output without a final newline can fail replay even when the program repeats those bytes. Use this format for textual examples rather than a lossless binary capture.

## Correct an entry or add an image

An expected error can be useful documentation too. `exec` records output even when the program fails and returns that program's status. Keep the entry when it demonstrates the intended failure. To correct the last entry, use `mdrun pop demo.md`, then record it again; `pop` removes a code/output pair together, or the final note.

To include an image, supply your own `screenshot.png` in the current directory:

```sh
mdrun image demo.md '![Application screenshot](screenshot.png)'
```

One execution appended this Markdown; the copied image receives a generated filename that changes on each call:

````markdown
```bash {image}
![Application screenshot](screenshot.png)
```

![Application screenshot](a0db0213-2026-11-16.png)
````

mdrun copies the image beside the document and points the rendered image at that copy. The source path resolves from your current directory. Supported extensions are PNG, JPG/JPEG, GIF and SVG. Popping the Markdown entry leaves the copied image file in place.

## Find, inspect and recreate

Before running a long or unfamiliar walkthrough, use `list` to find its executable blocks and `preview` to read one in full. Neither command executes a block or starts an application worker. `list` includes commands with no recorded output and excludes image and output blocks.

```sh
mdrun list demo.md --filter greeting --json
mdrun extract demo.md --filename rebuilt.md > rebuild.sh
```

The first command searches filename, heading and full command text for the case-sensitive literal substring `greeting`. For any returned `block` value, `mdrun preview demo.md BLOCK` prints the complete original fenced block without execution. These values are parser positions in the current file, not persistent IDs or executable-only ordinals; edits can change them.

`list --json` returns an array in input-file and block order. Each object has these fields:

| Field | Type and meaning |
| --- | --- |
| `file` | String containing the input filename as supplied. |
| `block` | Zero-based integer position in the parsed document; pass this value to `preview`. |
| `line` | One-based integer line number of the opening fence. |
| `heading` | Nearest preceding title or ATX heading as a string, or null when absent; Setext headings are not interpreted. |
| `interpreter` | String naming the executable interpreter from the fence. |
| `summary` | First line of the command as a string; filtering searches the complete command. |

Human-readable inventory escapes control characters to keep each entry on one row. Inventory requires UTF-8 filenames, headings, interpreters and commands; invalid text fails without printing a partial inventory. `preview` preserves the selected source bytes and line endings, including bytes that inventory cannot represent, and rejects missing or non-executable blocks. It performs no shell evaluation, variable expansion or stdin consumption.

The second command writes a different kind of result: `rebuild.sh` contains authoring commands that would recreate the document as `rebuilt.md`. `extract` only writes those commands. Inspect the script before running it; running it executes the recorded programs again and captures fresh output.

## Paths and automation

Keep the document path separate from the program's working directory. Document, image and output paths resolve from the caller's current directory; `--workdir DIR` changes where the program executes. Put `--` before positional values that start with a hyphen.

In an automated check, preserve `mdrun verify`'s exit status and retain its diff so a failure is actionable. Use `list --json` for discovery instead of parsing human-readable rows. Continue with `mdrun guide sessions` when repeated calls need to share prepared application initialization, or `mdrun guide reference` to look up exit statuses, environment and files.
