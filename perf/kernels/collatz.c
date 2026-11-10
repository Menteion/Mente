/* The C equivalent of collatz.mn. */
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>

static uint64_t collatz(uint64_t limit) {
  uint64_t total = 0;
  uint64_t longest = 0;
  for (uint64_t start = 1; start < limit; start++) {
    uint64_t x = start;
    uint64_t steps = 0;
    while (x != 1) {
      if (x % 2 == 0) {
        x = x / 2;
      } else {
        x = 3 * x + 1;
      }
      steps += 1;
    }
    total += steps;
    if (steps > longest) {
      longest = steps;
    }
  }
  return total * 1000 + longest;
}

int main(int argc, char **argv) {
  if (argc != 2) {
    fprintf(stderr, "usage: collatz <limit>\n");
    return 2;
  }
  printf("%llu\n", (unsigned long long)collatz(strtoull(argv[1], NULL, 10)));
  return 0;
}
