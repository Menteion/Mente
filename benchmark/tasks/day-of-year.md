---
id: day-of-year
milestone: M1
core: false
set: public
kind: solve
---

# Day of the year

## Task

Return the day of the year of the date `year`-`month`-`day` in the Gregorian calendar, January 1st being day 1. February has 29 days in a leap year: a year divisible by 4, except one divisible by 100 that is not divisible by 400. Callers pass only valid dates. The signature states the ranges of the date's parts and of the result.

## Signature

    day_of_year(year: u32 in 1..=9999, month: u8 in 1..=12, day: u8 in 1..=31) -> u16 in 1..=366

## Examples

    day_of_year(2024, 3, 1) == 61
    day_of_year(2023, 3, 1) == 60
    day_of_year(2023, 1, 31) == 31

## Hidden tests

    day_of_year(2000, 12, 31) == 366
    day_of_year(1900, 12, 31) == 365
    day_of_year(1, 1, 1) == 1
    day_of_year(9999, 12, 31) == 365
    day_of_year(2024, 2, 29) == 60
    day_of_year(1600, 7, 4) == 186
    day_of_year(2023, 12, 31) == 365
    day_of_year(2100, 3, 1) == 60
    day_of_year(2400, 3, 1) == 61
    day_of_year(4, 12, 31) == 366
    day_of_year(1999, 6, 15) == 166
    day_of_year(2024, 12, 31) == 366
    day_of_year(1, 12, 31) == 365
