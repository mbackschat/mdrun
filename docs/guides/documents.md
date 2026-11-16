# Create and verify a document

mdrun records commands and their real output in ordinary Markdown. Use it for reproducible demonstrations, coding-agent handoffs and command-line walkthroughs. Verification reruns the commands and compares recorded output.

## First recording

Run these commands in a new directory with `mdrun` on PATH:

```sh
mdrun init demo.md 'A repeatable greeting'
mdrun note demo.md 'This command prints a greeting.'
mdrun exec demo.md sh 'printf "hello from mdrun\n"'
mdrun verify demo.md
```

`exec` prints the captured output and appends a code/output pair to `demo.md`. Open the document to inspect both. `verify` exits zero without a diff when the recorded output matches; it leaves the document unchanged.

## Write longer commands

Omit the text/code argument to read it from stdin. This input supplies the recording's source, not the executed program's stdin:

```sh
printf '%s\n' 'A longer explanation.' | mdrun note demo.md
mdrun exec demo.md sh <<'CODE'
printf 'first line\n'
printf 'second line\n'
CODE
```

The interpreter is invoked as `INTERPRETER -c CODE`. Shells and `python3` accept that form. For other programs, run their normal command inside a shell block, such as `mdrun exec demo.md sh 'node app.mjs'`. Executed blocks read EOF from `/dev/null` on stdin unless their code supplies input through a file, pipeline or redirection.

## Check changes without overwriting evidence

`verify` executes all recognized code blocks except image blocks. It compares captured bytes wherever recorded output is present. It does not verify application exit statuses or roll back command side effects. Markdown is a textual recording, not a lossless binary capture: parsing reconstructs LF line endings, and output without a final newline can fail replay even when the program repeats those bytes. Inspect unfamiliar commands with `list` and `preview` before running them.

```sh
mdrun list demo.md
mdrun verify demo.md --output candidate.md
```

If output changed, verification prints a diff and exits 1; `candidate.md` contains updated captures. Review that file before replacing your original. Selecting `--output demo.md` explicitly permits an in-place update. Commands without a recorded output block still execute, so a successful verification is not an assertion about those commands' output.

## Correct an entry or add an image

An `exec` failure still records its output and returns the program's status. Keep it when demonstrating a failure; use `mdrun pop demo.md` to remove the most recent entry before recording a correction. `pop` removes a code/output pair together, or the final note.

```sh
mdrun image demo.md '![Application screenshot](screenshot.png)'
```

The image is copied beside the document under a generated filename. The source path resolves from your current directory. Supported extensions are PNG, JPG/JPEG, GIF and SVG. Popping its Markdown entry does not delete the copied image file.

## Find, inspect and recreate

```sh
mdrun list demo.md --filter greeting --json
mdrun extract demo.md --filename rebuilt.md > rebuild.sh
```

`list` searches filename, heading and full command text using a case-sensitive literal substring. For a returned `block` value, `mdrun preview demo.md BLOCK` prints that complete original fenced block without execution. Block values are parser positions in the current file, not persistent IDs or executable-only ordinals.

`extract` writes commands that would recreate a document; it does not execute them. Inspect `rebuild.sh` before running it. Rerunning these commands executes the recorded programs again and regenerates their output.

## Paths and automation

Document, image and output paths resolve from the caller's current directory. `--workdir DIR` changes the executed program's directory, while document paths keep their original basis. Put `--` before positional values that start with a hyphen.

For automated checks, preserve the exit status of `mdrun verify` and retain its diff. Use `list --json` for discovery instead of parsing human-readable rows. For warm execution of qualified applications, read `mdrun guide sessions`; for exit statuses, environment and files, read `mdrun guide reference`.
