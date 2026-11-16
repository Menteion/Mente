# The cases of a record's front matter

How a record's front matter is read, `front_matter` in `compiler/mente-cli/src/records.rs`, is held here as data: one file a case, so that every reader of the form, in any language, is tested against the same cases. `test_front_matter_reads_every_case_of_its_fixtures`, in `compiler/mente-cli/tests/records.rs`, runs them through the Rust reader. A reader in JavaScript is to run them all too.

Each case is a file `<key>.txt`, its name a key by the rule of `records.rs`, made of these lines in this order:

1. `fields` and the names of the front matter's fields, in order, each after one space: `fields date area note`.
2. `expect accepted` or `expect refused`.
3. For an accepted case, one line `value <name> <text>` for each field, in order: the string the field is read as, written after one space to the end of the line. An empty string is the line `value <name>` alone.
4. `input`.

The rest of the file, after the line `input`, is the case's input, byte for byte, but for two spellings. `.gitattributes` makes git keep every text file's line ends LF, and the policy allows no file git does not read as text, so a carriage return cannot be kept in a file here. So:

- `{CR}` stands for a carriage return, U+000D;
- `{TAB}` stands for a tab, U+0009, so that a tab can be seen.

A reader expands both before it reads the input. Nothing else in a case is read: no line may stand between the header's lines, and a field's text holds no tab and no line end.
