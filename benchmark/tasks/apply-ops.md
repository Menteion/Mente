---
id: apply-ops
milestone: M2
core: true
set: public
kind: solve
---

# Apply account operations

## Task

Apply the operations in `ops`, in order, to an account that starts with `balance`, and return the final balance. Every amount must be greater than zero, and a withdrawal must not take the balance below zero. When an operation breaks either rule, stop and return the error with that operation's position in `ops`, counting from 0; an amount that is not positive is reported as `NonPositiveAmount` even when it is a withdrawal. `balance` is never negative, and balances stay within `i64` in every test.

## Types

    enum Op { Deposit(i64), Withdraw(i64) }
    enum BankError { NonPositiveAmount(usize), Overdraft(usize) }

## Signature

    apply_ops(balance: i64, ops: slice<Op>) -> result<i64, BankError>

## Examples

    apply_ops(100, [Deposit(50), Withdraw(30)]) == ok(120)
    apply_ops(10, [Withdraw(20)]) == err(Overdraft(0))
    apply_ops(0, [Deposit(0)]) == err(NonPositiveAmount(0))

## Hidden tests

    apply_ops(0, []) == ok(0)
    apply_ops(5, [Withdraw(5)]) == ok(0)
    apply_ops(5, [Withdraw(5), Withdraw(1)]) == err(Overdraft(1))
    apply_ops(0, [Deposit(10), Withdraw(-3)]) == err(NonPositiveAmount(1))
    apply_ops(100, [Deposit(1), Deposit(2), Withdraw(103), Deposit(5)]) == ok(5)
    apply_ops(100, [Withdraw(50), Withdraw(50), Withdraw(1)]) == err(Overdraft(2))
    apply_ops(9223372036854775806, [Deposit(1)]) == ok(9223372036854775807)
    apply_ops(1, [Deposit(-9223372036854775808)]) == err(NonPositiveAmount(0))
    apply_ops(0, [Withdraw(0)]) == err(NonPositiveAmount(0))
    apply_ops(3, [Withdraw(4), Deposit(-1)]) == err(Overdraft(0))
