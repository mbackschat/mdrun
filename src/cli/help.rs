pub const ROOT: &str = "Examples:
  Record a command and check its output again:
    mdrun init demo.md 'A repeatable greeting'
    mdrun note demo.md 'This command prints a greeting.'
    mdrun exec demo.md sh 'printf \"hello from mdrun\\n\"'
    mdrun verify demo.md

Inspection:
  list and preview read documents without executing their commands.
  verify executes commands and compares recorded output; it does not undo effects.

For one command's syntax and examples, use mdrun COMMAND --help.
For longer authoring workflows, use mdrun guide documents.
For cooperative warm execution and binding requirements, use mdrun guide sessions.
For exit statuses, files and environment, use mdrun guide reference.";

pub const INIT: &str = "Example:
  mdrun init demo.md 'A repeatable greeting'

Creates Markdown with a title, UTC timestamp and mdrun provenance.
The destination must not exist. Its parent directory must already exist.
Next, use note for prose and exec to record a command with its real output.";

pub const NOTE: &str = "Examples:
  mdrun note demo.md 'Explain what the next command demonstrates.'
  printf '%s\\n' 'A longer explanation.' | mdrun note demo.md

Omit TEXT to read Markdown from stdin. The document must already exist.
To undo the final entry, use mdrun pop demo.md.";

pub const EXEC: &str = "Examples:
  mdrun exec demo.md sh 'printf \"hello\\n\"'
  mdrun exec demo.md python3 'print(2 + 2)'
  mdrun exec demo.md sh 'node app.mjs'

Runs LANG -c CODE. Omit CODE to read the source from stdin.
The executed program reads EOF on stdin unless its code redirects input.
stdout and stderr are captured together, printed, and appended to the document.
Output is recorded even on failure; exec returns the program's exit status.
Use pop to remove an unwanted final entry, then record its replacement.";

pub const IMAGE: &str = "Examples (the image must exist):
  mdrun image demo.md screenshot.png
  mdrun image demo.md '![Application screenshot](screenshot.png)'

Copies PNG, JPG/JPEG, GIF or SVG beside the document with a generated filename.
Image paths resolve from the caller's directory, independently of --workdir.
Omit INPUT to read the path or Markdown image reference from stdin.
Popping the entry removes its Markdown, but leaves the copied image file.";

pub const POP: &str = "Example:
  mdrun pop demo.md

Removes the final note or code/output pair. Image entries are removed together;
their copied files are retained. Earlier entries and unmanaged source remain.";

pub const VERIFY: &str = "Examples:
  mdrun verify demo.md
  mdrun verify demo.md --output candidate.md

Executes recognized code blocks, skips images, and compares recorded output.
Exits 0 when output matches, or 1 on differences/errors. Application exit
statuses are not compared. Blocks without captured output still execute.
The input stays unchanged unless explicitly selected as --output.
An output copy contains updated captures even when differences cause exit 1.
Review the diff and candidate before replacing your original document.

With --binding, verification uses a cooperative managed session.
--require-warm rejects cold routing; --mode cold starts no worker.
For binding preparation and report interpretation, use mdrun guide sessions.";

pub const EXTRACT: &str = "Examples:
  mdrun extract demo.md
  mdrun extract demo.md --filename rebuilt.md > rebuild.sh

Prints shell commands that would recreate the document; it executes nothing.
Captured output is regenerated when you run those commands.
Inspect the generated script before executing it.";

pub const LIST: &str = "Examples:
  mdrun list demo.md
  mdrun list demo.md another.md --filter greeting --json

Searches filename, heading and complete command source with a case-sensitive
literal substring. Includes unnamed commands and commands without output.
JSON contains file, block, line, heading, interpreter and summary fields.
Use a returned block value with preview. Positions change when the file changes.
No command, variable expansion or worker is started.";

pub const PREVIEW: &str = "Workflow:
  mdrun list demo.md --json
  mdrun preview demo.md 2

Use a block value actually returned by list; 2 is illustrative.
Prints the complete original fenced source, preserving spacing and line endings.
BLOCK is a zero-based parser position, not an executable-only ordinal.
The selected command is never executed.";

pub const SESSION: &str = "Example (requires a prepared cooperative binding):
  mdrun session --binding binding.json --report session.json -- sh author.sh

Runs the author with a session-local PATH binding and at most one sequential worker.
Fast mode uses warm execution only after eligibility and context checks.
Cold mode invokes the packaged launcher without starting a worker.
An accepted invocation is never retried cold. Infrastructure or cleanup failure
fails the session even when the author masks a client error.
Children must stay in their assigned process group; monitor mode (set -m),
independent groups and daemonization are unsupported and not reliably detected.
Read mdrun guide sessions when preparing an adapter or interpreting reports.";

pub const VERIFY_MANY: &str = "Example (requires a prepared binding and independent documents):
  mdrun verify-many --binding binding.json --jobs 4 --report batch.json \\\n    -- first.md second.md

Each admitted document has a separate session and at most one sequential worker.
Slots include startup, replay and cleanup. A failure or output mismatch stops admission;
already-started healthy documents finish within their deadlines.
A batch custody failure can stop all sessions.
Any declared resource makes effective concurrency 1 unless --share-resources
permits sharing. The batch holds its exclusions across gaps between documents.
stdin reads EOF. Output is spooled to disk without a byte quota and published in
input order; a slow earlier document delays later output. Labels quote and escape
filenames. Inputs are not updated.
The report includes every requested document, including those not started.
Cooperative process-group limits still apply; see mdrun guide sessions.";
