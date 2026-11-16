---
date: 2026-11-16
fixed-by: https://github.com/Menteion/Mente/pull/158
area: native
missed-by: The program generator, which passes no argument outside its parameter's type, so no program reached a call counted after its arguments were narrowed.
guard: fn test_a_call_from_outside_is_counted_before_its_arguments_are_narrowed
---
