/* The C equivalent of sieve.mn. */
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>

static uint64_t sieve(uint32_t reps) {
  bool composite[262144] = {false};
  uint64_t check = 0;
  for (uint32_t r = 0; r < reps; r++) {
    size_t limit = 262144 - r % 8u;
    for (size_t c = 0; c < limit; c++) {
      composite[c] = false;
    }
    for (size_t p = 2; p * p < limit; p++) {
      if (!composite[p]) {
        for (size_t m = p * p; m < limit; m += p) {
          composite[m] = true;
        }
      }
    }
    uint64_t count = 0;
    for (size_t n = 2; n < limit; n++) {
      if (!composite[n]) {
        count += 1;
      }
    }
    check = (check * 31u + count) % 1000000007u;
  }
  return check;
}

int main(int argc, char **argv) {
  if (argc != 2) {
    fprintf(stderr, "usage: sieve <reps>\n");
    return 2;
  }
  printf("%llu\n", (unsigned long long)sieve((uint32_t)strtoul(argv[1], NULL, 10)));
  return 0;
}
