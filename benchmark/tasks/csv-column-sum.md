---
id: csv-column-sum
milestone: M6
core: true
set: public
kind: solve
---

# Sum a column of a CSV file

## Task

Read the file at `path` with `Fs.read_text`. It holds comma-separated values with no quoting: the first line names the columns, and each later line is a row. Return the sum of the values in the column named `column`, the first such column when several share the name. Lines end with a newline, the last one perhaps without; empty lines are skipped.

Return `Io` with the error when the file cannot be read, `MissingColumn` when no column has that name (or the file is empty), `BadNumber` with the 1-based line number of the first row whose field in that column is missing or is not a decimal integer with an optional leading `-`, and `Overflow` when a value or the running sum leaves `i64`.

## Types

    enum CsvError { Io(IoError), MissingColumn, BadNumber(u64), Overflow }

## Signature

    csv_column_sum(path: string, column: string) -> result<i64, CsvError> uses Fs

## Examples

    given Fs.read_text("data.csv") => ok("name,amount\na,10\nb,-3\n")
    csv_column_sum("data.csv", "amount") == ok(7)

    given Fs.read_text("data.csv") => ok("name,amount\na,10\n")
    csv_column_sum("data.csv", "price") == err(MissingColumn)

    csv_column_sum("missing.csv", "amount") == err(Io(NotFound))

## Hidden tests

    given Fs.read_text("d.csv") => ok("a,b,c\n1,2,3\n4,5,6")
    csv_column_sum("d.csv", "b") == ok(7)

    given Fs.read_text("d.csv") => ok("a,b\n1,2\nx,y\n")
    csv_column_sum("d.csv", "b") == err(BadNumber(3))

    given Fs.read_text("d.csv") => ok("a,b\n1,2\n3\n")
    csv_column_sum("d.csv", "b") == err(BadNumber(3))

    given Fs.read_text("d.csv") => ok("a,b\n1,2\n\n3,4\n")
    csv_column_sum("d.csv", "b") == ok(6)

    given Fs.read_text("d.csv") => ok("v\n9223372036854775807\n1\n")
    csv_column_sum("d.csv", "v") == err(Overflow)

    given Fs.read_text("d.csv") => ok("v\n9223372036854775808\n-1\n")
    csv_column_sum("d.csv", "v") == err(Overflow)

    given Fs.read_text("d.csv") => ok("v\n-9223372036854775808\n")
    csv_column_sum("d.csv", "v") == ok(-9223372036854775808)

    given Fs.read_text("d.csv") => ok("")
    csv_column_sum("d.csv", "v") == err(MissingColumn)

    given Fs.read_text("d.csv") => ok("v\n")
    csv_column_sum("d.csv", "v") == ok(0)

    given Fs.read_text("d.csv") => ok("v\n+5\n")
    csv_column_sum("d.csv", "v") == err(BadNumber(2))

    given Fs.read_text("d.csv") => ok("v\n 5\n")
    csv_column_sum("d.csv", "v") == err(BadNumber(2))

    given Fs.read_text("d.csv") => ok("a,v\nq,1\n\n\nq,two\n")
    csv_column_sum("d.csv", "v") == err(BadNumber(5))

    given Fs.read_text("d.csv") => err(PermissionDenied)
    csv_column_sum("d.csv", "v") == err(Io(PermissionDenied))

    given Fs.read_text("d.csv") => ok("x,y,x\n1,2,3\n")
    csv_column_sum("d.csv", "x") == ok(1)
