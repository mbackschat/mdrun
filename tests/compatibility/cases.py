"""Finite, source-grounded CLI scenarios; fixtures are authored for mdrun."""

from oracle import Case, Step


LEGACY = b"# Example\n\n*2026-01-02T03:04:05Z by Showboat 0.6.1*\n<!-- showboat-id: legacy-fixed -->\n"
MATCH = LEGACY + b"\n```sh\nprintf 'hello\\n'\n```\n\n```output\nhello\n```\n"
DRIFT = MATCH.replace(b"\nhello\n", b"\nstale\n")
IMAGE_BYTES = b"\x89PNG\r\n\x1a\n\x00binary\xfffixture"


def cases():
    result = []

    def add(name, argv, *, files=None, stdin=b"", **kwargs):
        result.append(Case(name, {"doc.md": LEGACY} if files is None else files,
                           [Step(argv, stdin)], **kwargs))

    for name, argv in [
        ("no-args", []), ("help-long", ["--help"]), ("help-short", ["-h"]),
        ("help-command", ["help"]), ("help-extra", ["help", "exec"]),
        ("version", ["--version"]), ("unknown", ["unknown"]),
        ("unknown-option", ["--unknown"]), ("double-dash", ["--", "--version"]),
        ("version-override", ["unknown", "--version"]),
        ("workdir-missing", ["--workdir"]), ("workdir-consumes-version", ["--workdir", "--version"]),
        ("equals-workdir", ["--workdir=work", "help"]),
    ]:
        add("cli-" + name, argv)
    for command, required in [("init", 2), ("note", 1), ("exec", 2),
                              ("image", 1), ("pop", 1), ("verify", 1), ("extract", 1)]:
        for count in range(required):
            add(f"cli-{command}-arity-{count}", [command, *["doc.md", "sh"][:count]])
        add(f"cli-{command}-help-position", [command, "--help"])

    add("init-new", ["init", "new.md", "Title ✓"], generated_documents=["new.md"])
    add("init-extra", ["init", "new.md", "Title", "ignored"], generated_documents=["new.md"])
    add("init-exists", ["init", "doc.md", "Changed"])
    add("init-missing-parent", ["init", "absent/doc.md", "Title"])
    add("init-workdir", ["--workdir", "work", "init", "new.md", "Title"],
        directories=["work"], generated_documents=["new.md"])

    for name, args, stdin in [
        ("argument", ["note", "doc.md", "Hello ✓\n\nparagraph"], b"ignored"),
        ("stdin", ["note", "doc.md"], b"Hello stdin\n\n"),
        ("empty", ["note", "doc.md"], b""),
        ("extras", ["note", "doc.md", "first", "ignored"], b""),
        ("literal-option", ["note", "doc.md", "--unknown"], b""),
        ("global-in-text", ["note", "doc.md", "--version"], b""),
        ("missing-file", ["note", "missing.md", "text"], b""),
    ]:
        add("note-" + name, args, stdin=stdin)

    for name, code in [
        ("success", "printf 'hello\\n'"), ("combined", "printf 'out\\n'; printf 'err\\n' >&2"),
        ("nonzero", "printf 'failed\\n'; exit 7"), ("signal", "kill -TERM $$"),
        ("no-newline", "printf hello"), ("empty", ":"),
        ("unicode", "printf 'Grüße ✓\\n'"), ("binary", "printf '\\377\\000\\n'"),
        ("fence-output", "printf '\x60\x60\x60\\n\x60\x60\x60\x60\\n'"),
        ("effects", "mkdir created; printf effect > created/result; printf 'done\\n'"),
        ("stdin-at-eof", "if read value; then printf unexpected; else printf 'eof\\n'; fi"),
    ]:
        add("exec-" + name, ["exec", "doc.md", "sh", code])
    add("exec-stdin-code", ["exec", "doc.md", "sh"], stdin=b"printf 'stdin code\\n'\n")
    add("exec-missing-interpreter", ["exec", "doc.md", "mdrun-missing-interpreter", "code"])
    add("exec-missing-file-no-effect", ["exec", "missing.md", "sh", "touch unwanted"])
    add("exec-extra", ["exec", "doc.md", "sh", "printf 'first\\n'", "ignored"])
    add("exec-missing-workdir", ["exec", "doc.md", "sh", ":", "--workdir", "absent"])
    for name, args in [
        ("prefix", ["--workdir", "work", "exec", "doc.md", "sh", "cat marker"]),
        ("middle", ["exec", "--workdir", "work", "doc.md", "sh", "cat marker"]),
        ("suffix", ["exec", "doc.md", "sh", "cat marker", "--workdir", "work"]),
        ("repeated", ["--workdir", "absent", "exec", "doc.md", "sh", "cat marker", "--workdir", "work"]),
    ]:
        add("workdir-" + name, args, files={"doc.md": LEGACY, "work/marker": b"cwd marker\n"})

    for name, source in [("match", MATCH), ("drift", DRIFT), ("empty", b""),
                         ("nonzero-match", MATCH.replace(b"printf 'hello\\n'", b"printf 'hello\\n'; exit 7")),
                         ("unpaired", LEGACY + b"\n```sh\nprintf 'unpaired\\n'\n```\n"),
                         ("image-skipped", LEGACY + b"\n```bash {image}\ntouch unwanted\n```\n\n![alt](missing.png)\n"),
                         ("legacy-no-id", MATCH.replace(b"<!-- showboat-id: legacy-fixed -->\n", b"")),
                         ("embedded-fence", LEGACY + b"\n````sh\nprintf '\x60\x60\x60\\n'\n````\n\n````output\n```\n````\n")]:
        add("verify-" + name, ["verify", "doc.md"], files={"doc.md": source})
    add("verify-copy", ["verify", "doc.md", "--output", "updated.md"], files={"doc.md": DRIFT})
    add("verify-same-output-path", ["verify", "doc.md", "--output", "doc.md"], files={"doc.md": DRIFT})
    add("verify-repeated-output", ["verify", "doc.md", "--output", "ignored.md", "--output", "last.md"], files={"doc.md": MATCH})
    add("verify-output-missing-value", ["verify", "doc.md", "--output"], files={"doc.md": MATCH})
    add("verify-output-before-file", ["verify", "--output", "new.md", "doc.md"])
    add("verify-unknown-extra", ["verify", "doc.md", "--unknown"], files={"doc.md": MATCH})
    add("verify-missing-file", ["verify", "missing.md"])
    add("verify-interpreter-error", ["verify", "doc.md"], files={"doc.md": b"```no-such-runtime\nhi\n```\n"})
    add("verify-multiple", ["verify", "doc.md", "--output", "updated.md"], files={"doc.md": DRIFT + b"\n```sh\nprintf 'second\\n'\n```\n\n```output\nold\n```\n"})

    for name, source in [
        ("title", LEGACY), ("empty", b""), ("note", LEGACY + b"\nSome text\n"),
        ("command", MATCH), ("orphan-output", b"```output\none\n```\n"),
        ("image", LEGACY + b"\n```bash {image}\nsource.png\n```\n\n![alt](kept.png)\n"),
    ]:
        add("pop-" + name, ["pop", "doc.md"], files={"doc.md": source, "kept.png": IMAGE_BYTES})
    add("pop-missing", ["pop", "missing.md"])

    for name, source in [
        ("crlf", MATCH.replace(b"\n", b"\r\n")), ("no-final-newline", LEGACY.rstrip(b"\n")),
        ("extra-blanks", LEGACY + b"\n\ntext  \n\n\n"), ("bom", b"\xef\xbb\xbf" + LEGACY),
        ("unclosed-fence", LEGACY + b"\n```sh\nprintf 'x\\n'"),
        ("long-line", LEGACY + b"\n" + b"x" * 70000),
    ]:
        add("text-" + name, ["note", "doc.md", "Added"], files={"doc.md": source})

    for name, input_ in [("plain", "source.PNG"), ("alt", "![caption ✓](source.PNG)"),
                         ("escaped", r"\![caption](source.PNG)"), ("empty-alt", "![](source.PNG)")]:
        add("image-" + name, ["image", "doc.md", input_],
            files={"doc.md": LEGACY, "source.PNG": IMAGE_BYTES}, generated_images=True)
    for extension in ["jpg", "jpeg", "gif", "svg"]:
        add("image-extension-" + extension, ["image", "doc.md", "source." + extension],
            files={"doc.md": LEGACY, "source." + extension: IMAGE_BYTES}, generated_images=True)
    add("image-stdin", ["image", "doc.md"], stdin=b"source.png\n", files={"doc.md": LEGACY, "source.png": IMAGE_BYTES}, generated_images=True)
    add("image-repeat", ["image", "doc.md", "source.png"], files={"doc.md": LEGACY, "source.png": IMAGE_BYTES}, generated_images=True)
    result[-1].steps.append(Step(["image", "doc.md", "source.png"]))
    add("image-missing", ["image", "doc.md", "absent.png"])
    add("image-directory", ["image", "doc.md", "directory.png"], directories=["directory.png"])
    add("image-extension-error", ["image", "doc.md", "source.txt"], files={"doc.md": LEGACY, "source.txt": IMAGE_BYTES})
    add("image-workdir-ignored", ["image", "doc.md", "source.png", "--workdir", "work"], files={"doc.md": LEGACY, "work/source.png": IMAGE_BYTES})
    add("image-missing-document", ["image", "missing.md", "source.png"], files={"source.png": IMAGE_BYTES})

    for name, argv in [
        ("basic", ["extract", "doc.md"]),
        ("filename", ["extract", "doc.md", "--filename", "a path's file.md"]),
        ("missing-value", ["extract", "doc.md", "--filename"]),
        ("repeated", ["extract", "doc.md", "--filename", "ignored", "--filename", "last.md"]),
        ("missing-file", ["extract", "missing.md"]),
    ]:
        add("extract-" + name, argv, files={"doc.md": MATCH + b"\nText with 'quotes', $dollars\nand lines.\n"})

    for status in [200, 503]:
        result.append(Case(f"remote-workflow-{status}", {"source.png": IMAGE_BYTES}, [
            Step(["init", "doc.md", "Remote title"]), Step(["note", "doc.md", "note ✓"]),
            Step(["exec", "doc.md", "sh", "printf 'failed\\n'; exit 7"]),
            Step(["image", "doc.md", "![caption](source.png)"]), Step(["pop", "doc.md"]),
            Step(["verify", "doc.md"]), Step(["extract", "doc.md"]),
        ], remote_status=status, generated_documents=["doc.md"], generated_images=True))
    add("remote-legacy-no-id", ["note", "doc.md", "text"], files={"doc.md": b"# Old\n\n*2020-01-01*\n"}, remote_status=200)
    add("remote-binary-output", ["exec", "doc.md", "sh", "printf '\\377\\000\\n'"], remote_status=200)
    add("remote-legacy-fixed-id", ["note", "doc.md", "Keep legacy identity"], remote_status=200)
    add("path-spaces", ["exec", "a path/doc.md", "sh", "printf 'space\\n'"], files={"a path/doc.md": LEGACY})
    add("extract-reconstruction", ["extract", "doc.md", "--filename", "rebuilt.md"],
        files={"doc.md": MATCH + b"\nA quote: ' and $literal\nshowboat is literal prose here\n\n```bash {image}\n![caption](source.png)\n```\n\n![caption](recorded.png)\n",
               "source.png": IMAGE_BYTES, "recorded.png": IMAGE_BYTES},
        reconstruct_document="rebuilt.md", reconstruct_files={"source.png": IMAGE_BYTES})
    return result
