---
id: count-lines
milestone: M6
core: true
set: public
kind: solve
---

# Count the lines of a file

## Task

Return the number of lines in the text file at `path`, read with `Fs.read_text`. Every line ends with a newline character except perhaps the last, which counts as well; an empty file has no lines. Return the error when the file cannot be read.

## Signature

    count_lines(path: string) -> result<u64, IoError> uses Fs

## Examples

    given Fs.read_text("notes.txt") => ok("a\nb\n")
    count_lines("notes.txt") == ok(2)

    given Fs.read_text("notes.txt") => ok("a\nb")
    count_lines("notes.txt") == ok(2)

    count_lines("missing.txt") == err(NotFound)

## Hidden tests

    given Fs.read_text("e.txt") => ok("")
    count_lines("e.txt") == ok(0)

    given Fs.read_text("n.txt") => ok("\n")
    count_lines("n.txt") == ok(1)

    given Fs.read_text("n.txt") => ok("\n\n\n")
    count_lines("n.txt") == ok(3)

    given Fs.read_text("locked.txt") => err(PermissionDenied)
    count_lines("locked.txt") == err(PermissionDenied)

    given Fs.read_text("one.txt") => ok("single line")
    count_lines("one.txt") == ok(1)

    given Fs.read_text("crlf.txt") => ok("x\r\ny\r\n")
    count_lines("crlf.txt") == ok(2)

    given Fs.read_text("bin.dat") => err(InvalidUtf8)
    count_lines("bin.dat") == err(InvalidUtf8)

    given Fs.read_text("a.txt") => ok("1\n2\n3\n4\n5\n6\n7\n8\n9\n10")
    count_lines("a.txt") == ok(10)
