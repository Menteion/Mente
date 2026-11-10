/* The C equivalent of bsearch.mn. */
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>

static uint64_t bsearch_kernel(uint32_t reps) {
  uint32_t a[4096] = {0};
  uint32_t keys[4096] = {0};
  uint64_t seed = 12345;
  uint32_t value = 0;
  for (size_t i = 0; i < 4096; i++) {
    seed = (seed * 1103515245u + 12345u) % 2147483648u;
    value += (uint32_t)(1 + seed % 8u);
    a[i] = value;
  }
  for (size_t i = 0; i < 4096; i++) {
    seed = (seed * 1103515245u + 12345u) % 2147483648u;
    keys[i] = (uint32_t)(seed % 20480u);
  }
  uint64_t check = 0;
  for (uint32_t r = 0; r < reps; r++) {
    uint64_t total = 0;
    for (size_t q = 0; q < 4096; q++) {
      uint32_t key = keys[q];
      size_t lo = 0;
      size_t hi = 4096;
      while (lo < hi) {
        size_t mid = (lo + hi) / 2;
        if (a[mid] < key) {
          lo = mid + 1;
        } else {
          hi = mid;
        }
      }
      total += lo;
      if (lo < 4096 && a[lo] == key) {
        total += 1000000;
      }
    }
    check = (check * 31u + total) % 1000000007u;
    keys[r % 4096u] = (uint32_t)(check % 20480u);
  }
  return check;
}

int main(int argc, char **argv) {
  if (argc != 2) {
    fprintf(stderr, "usage: bsearch <reps>\n");
    return 2;
  }
  printf("%llu\n", (unsigned long long)bsearch_kernel((uint32_t)strtoul(argv[1], NULL, 10)));
  return 0;
}
